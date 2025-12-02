use anyhow::{Context, Error};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use clap::{arg, Command};
use pdf::content::*;
use pdf::file::File as PdfFile;
use regex::Regex;
use serde_json;
use std::collections::HashMap;
use std::io;
use std::process::exit;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::{fs, vec};

/// Transaction row representation.
#[derive(Debug, Clone)]
pub struct Transaction {
    pub date: NaiveDateTime,
    pub description: String,
    pub points: i32,
    pub amount: f32,
}

impl Default for Transaction {
    fn default() -> Self {
        Transaction {
            date: NaiveDateTime::new(
                NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ),
            description: String::new(),
            points: 0,
            amount: 0.0,
        }
    }
}

impl Transaction {
    /// Send this transaction to the CSV writer channel.
    fn emit(&self, sender: &Sender<Vec<String>>) -> Result<(), Error> {
        sender
            .send(vec![
                self.date.to_string(),
                self.description.clone(),
                self.points.to_string(),
                self.amount.to_string(),
            ])
            .context("Failed to send transaction to writer")
    }
}

// ============================================================================
// Date/Amount/Points Parsing Helpers
// ============================================================================

/// Date formats used in HDFC statements.
const DATE_FORMATS: &[&str] = &[
    "%d/%m/%Y| %H:%M",  // Domestic: "19/10/2025| 00:57"
    "%d/%m/%Y | %H:%M", // International: "26/09/2025 | 13:33"
    "%d/%m/%Y %H:%M:%S", // Old format with seconds
];

/// Try to parse a date string from various formats used in HDFC statements.
fn parse_transaction_date(s: &str) -> Option<NaiveDateTime> {
    // Try datetime formats first
    for format in DATE_FORMATS {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, format) {
            return Some(dt);
        }
    }
    // Try date-only format
    if let Ok(d) = NaiveDate::parse_from_str(s, "%d/%m/%Y") {
        return Some(NaiveDateTime::new(
            d,
            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        ));
    }
    None
}

/// Try to parse an amount string, handling currency symbols and credit markers.
fn parse_amount(s: &str, is_credit: bool) -> Option<f32> {
    let clean = s
        .replace('₹', "")
        .replace('\u{20b9}', "")
        .replace(',', "")
        .trim()
        .to_string();

    let (is_credit, num_str) = if clean.starts_with('+') {
        (true, clean.trim_start_matches('+').trim())
    } else {
        (is_credit, clean.as_str())
    };

    num_str
        .parse::<f32>()
        .ok()
        .map(|amt| if is_credit { amt } else { -amt })
}

/// Try to parse reward points from various formats.
fn parse_points(s: &str) -> Option<i32> {
    s.replace("+ ", "+")
        .replace("- ", "-")
        .trim()
        .parse::<i32>()
        .ok()
}

// ============================================================================
// Text Classification Helpers
// ============================================================================

/// Section terminators that end a transaction block.
const SECTION_TERMINATORS: &[&str] = &[
    "Eligible for EMI",
    "Eligible for",
    "TRANSACTIONS",
    "Past Dues",
    "GST Summary",
    "Rewards Program Points Summary",
    "Offers on your card",
    "TOTAL AMOUNT",
    "CONVERT TO EMI",
];

/// Foreign currency prefixes (amounts in foreign currency are part of description).
const FOREIGN_CURRENCY_PREFIXES: &[&str] =
    &["USD ", "JPY ", "MYR ", "EUR ", "GBP ", "SGD ", "AUD ", "THB "];

/// Check if text is a section terminator.
fn is_section_terminator(text: &str) -> bool {
    SECTION_TERMINATORS.contains(&text) || text.starts_with("*Transaction time")
}

/// Check if text is a page header (appears on continuation pages).
fn is_page_header(text: &str) -> bool {
    text == "Infinia Credit Card Statement"
        || text.starts_with("HSN Code:")
        || text.starts_with("HDFC Bank Credit Cards GSTIN:")
        || text.contains("GSTIN: 33AAACH")
}

/// Check if text is a page number like "Page 1 of 7".
fn is_page_number(text: &str) -> bool {
    text.starts_with("Page ") && text.contains(" of ")
}

/// Check if text is a foreign currency amount.
fn is_foreign_currency(text: &str) -> bool {
    FOREIGN_CURRENCY_PREFIXES
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

/// Check if text should be skipped (standalone symbols/markers).
fn is_skippable_symbol(text: &str) -> bool {
    matches!(text, "+" | "C" | "₹" | "l" | "●" | "•" | "Cr")
}

// ============================================================================
// Summary Feature
// ============================================================================

/// Summary of transactions for reporting.
#[derive(Default)]
struct Summary {
    total_spent: f32,
    bill_payment: f32,
    total_points: i32,
    category_totals: HashMap<String, f32>,
    uncategorized: f32,
    transaction_count: usize,
}

/// Load category patterns from a JSON file.
/// Format: {"Category Name": ["PATTERN1", "PATTERN2"], ...}
fn load_categories(path: &str) -> Result<HashMap<String, Vec<String>>, Error> {
    let content = fs::read_to_string(path)
        .context(format!("Failed to read categories file: {}", path))?;
    let categories: HashMap<String, Vec<String>> = serde_json::from_str(&content)
        .context("Failed to parse categories JSON")?;
    Ok(categories)
}

/// Check if a transaction is a credit card bill payment.
fn is_bill_payment(description: &str) -> bool {
    description.to_uppercase().contains("CREDIT CARD PAYMENT")
}

/// Find the category for a transaction based on description patterns.
fn categorize(description: &str, categories: &HashMap<String, Vec<String>>) -> Option<String> {
    let desc_upper = description.to_uppercase();
    for (category, patterns) in categories {
        for pattern in patterns {
            if desc_upper.contains(&pattern.to_uppercase()) {
                return Some(category.clone());
            }
        }
    }
    None
}

/// Parse a CSV record back into transaction fields for summary calculation.
fn parse_record(record: &[String]) -> (String, i32, f32) {
    let description = record.get(1).cloned().unwrap_or_default();
    let points = record.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let amount = record.get(3).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    (description, points, amount)
}

/// Calculate summary from transaction records.
fn calculate_summary(
    records: &[Vec<String>],
    categories: &Option<HashMap<String, Vec<String>>>,
) -> Summary {
    let mut summary = Summary::default();

    for record in records {
        let (description, points, amount) = parse_record(record);
        summary.transaction_count += 1;
        summary.total_points += points;

        if is_bill_payment(&description) {
            summary.bill_payment += amount;
        } else if amount < 0.0 {
            // Debit transaction (spending)
            let spent = amount.abs();
            summary.total_spent += spent;

            // Categorize if categories provided
            if let Some(cats) = categories {
                if let Some(category) = categorize(&description, cats) {
                    *summary.category_totals.entry(category).or_insert(0.0) += spent;
                } else {
                    summary.uncategorized += spent;
                }
            }
        }
    }

    summary
}

/// Print summary to stdout.
fn print_summary(summary: &Summary, has_categories: bool) {
    println!();
    println!("═══════════════════════════════════════════");
    println!("              SUMMARY");
    println!("═══════════════════════════════════════════");
    println!("Total Spent:          ₹ {:>12.2}", summary.total_spent);
    println!("Bill Payment:         ₹ {:>12.2}", summary.bill_payment);
    println!("Points Earned:        {:>15}", summary.total_points);
    println!("Transactions:         {:>15}", summary.transaction_count);

    if has_categories && !summary.category_totals.is_empty() {
        println!();
        println!("───────────────────────────────────────────");
        println!("         CATEGORY BREAKDOWN");
        println!("───────────────────────────────────────────");

        // Sort categories by amount (descending)
        let mut sorted: Vec<_> = summary.category_totals.iter().collect();
        sorted.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());

        for (category, amount) in sorted {
            let percentage = if summary.total_spent > 0.0 {
                (amount / summary.total_spent) * 100.0
            } else {
                0.0
            };
            println!("{:<20}  ₹ {:>10.2}  ({:>5.1}%)", category, amount, percentage);
        }

        if summary.uncategorized > 0.0 {
            let percentage = if summary.total_spent > 0.0 {
                (summary.uncategorized / summary.total_spent) * 100.0
            } else {
                0.0
            };
            println!(
                "{:<20}  ₹ {:>10.2}  ({:>5.1}%)",
                "Uncategorized", summary.uncategorized, percentage
            );
        }

        println!("───────────────────────────────────────────");
    }
    println!();
}

// ============================================================================
// PDF Text Extraction
// ============================================================================

/// Extract all non-empty text elements from a PDF page.
fn extract_page_texts(ops: &[Op]) -> Vec<String> {
    ops.iter()
        .filter_map(|op| {
            if let Op::TextDraw { ref text } = op {
                std::str::from_utf8(text.as_bytes())
                    .ok()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect()
}

// ============================================================================
// Parser State Machine
// ============================================================================

/// State for parsing transactions from a page.
struct ParserState {
    in_transactions: bool,
    past_header: bool,
    skip_next_non_date: bool,
    in_row: bool,
    has_amount: bool,
    is_credit: bool,
    transaction: Transaction,
    desc_parts: Vec<String>,
    debug: bool,
}

impl ParserState {
    fn new(debug: bool) -> Self {
        ParserState {
            in_transactions: false,
            past_header: false,
            skip_next_non_date: false,
            in_row: false,
            has_amount: false,
            is_credit: false,
            transaction: Transaction::default(),
            desc_parts: Vec::new(),
            debug,
        }
    }

    /// Finalize and emit the current transaction if valid.
    fn flush_transaction(&mut self, sender: &Sender<Vec<String>>, reason: &str) -> Result<(), Error> {
        if self.in_row && !self.transaction.description.is_empty() {
            if !self.desc_parts.is_empty() {
                self.transaction.description = self.desc_parts.join(" ");
            }
            self.transaction.emit(sender)?;
            if self.debug {
                eprintln!("=== EMIT ({}): {:?} ===", reason, self.transaction);
            }
        }
        Ok(())
    }

    /// Reset state for a new transaction.
    fn start_new_transaction(&mut self, date: NaiveDateTime) {
        self.transaction = Transaction::default();
        self.transaction.date = date;
        self.in_row = true;
        self.has_amount = false;
        self.is_credit = false;
        self.desc_parts.clear();
    }

    /// Reset state when exiting a transaction section.
    fn exit_section(&mut self) {
        self.in_transactions = false;
        self.transaction = Transaction::default();
        self.in_row = false;
        self.has_amount = false;
        self.desc_parts.clear();
    }
}

// ============================================================================
// Main Parser
// ============================================================================

/// Parse a PDF file and send transactions to the channel.
pub fn parse(
    path: String,
    cardholder_name: String,
    password: String,
    sender: &Sender<Vec<String>>,
) -> Result<(), Error> {
    let file = PdfFile::<Vec<u8>>::open_password(path.clone(), password.as_bytes())
        .context(format!("failed to open file {}", path))?;

    let debug = std::env::var("DEBUG").is_ok();

    for page in file.pages() {
        let page = match page {
            Ok(p) => p,
            Err(_) => continue,
        };

        let content = match &page.contents {
            Some(c) => c,
            None => continue,
        };

        let ops = match content.operations(&file) {
            Ok(o) => o,
            Err(_) => continue,
        };

        let texts = extract_page_texts(&ops);
        let mut state = ParserState::new(debug);

        for (i, text) in texts.iter().enumerate() {
            if debug {
                eprintln!("{}: {:?}", i, text);
            }

            // Start of transaction section
            if text == "Domestic Transactions" || text == "International Transactions" {
                state.in_transactions = true;
                state.past_header = false;
                continue;
            }

            if !state.in_transactions {
                continue;
            }

            // Skip header row until we see cardholder name or PI column
            if !state.past_header {
                if text == &cardholder_name || text == "PI" {
                    state.past_header = true;
                    state.skip_next_non_date = true;
                    if debug {
                        eprintln!("=== PAST HEADER (trigger: {}) ===", text);
                    }
                }
                continue;
            }

            // Skip cardholder name that appears right after header
            if state.skip_next_non_date {
                state.skip_next_non_date = false;
                if parse_transaction_date(text).is_none() {
                    if debug {
                        eprintln!("=== SKIP CARDHOLDER NAME: {} ===", text);
                    }
                    continue;
                }
            }

            // Check for section end
            if is_section_terminator(text) {
                state.flush_transaction(sender, "section end")?;
                state.exit_section();
                continue;
            }

            // Try to parse as a date (starts a new transaction)
            if let Some(dt) = parse_transaction_date(text) {
                state.flush_transaction(sender, "new date")?;
                state.start_new_transaction(dt);
                continue;
            }

            if !state.in_row {
                continue;
            }

            // Skip various non-data elements
            if is_skippable_symbol(text) {
                if text == "+" {
                    state.is_credit = true;
                } else if text == "Cr" {
                    state.transaction.amount = state.transaction.amount.abs();
                }
                continue;
            }

            if is_page_number(text) || is_page_header(text) {
                continue;
            }

            // Foreign currency amounts are part of the description
            if is_foreign_currency(text) {
                state.desc_parts.push(text.clone());
                continue;
            }

            // Try to parse as amount
            if text.contains('.') {
                if let Some(amt) = parse_amount(text, state.is_credit) {
                    state.transaction.amount = amt;
                    state.has_amount = true;
                    state.is_credit = false;
                    continue;
                }
            }

            // Try to parse as points
            if let Some(p) = parse_points(text) {
                state.transaction.points = p;
                continue;
            }

            // After amount, any remaining text is likely a section header (cardholder name)
            if state.has_amount {
                if debug {
                    eprintln!("=== SKIP POST-AMOUNT TEXT: {} ===", text);
                }
                continue;
            }

            // Must be part of description
            state.desc_parts.push(text.clone());
            if state.transaction.description.is_empty() {
                state.transaction.description = text.clone();
            }
        }

        // Flush last transaction on page
        state.flush_transaction(sender, "page end")?;
    }

    Ok(())
}

// ============================================================================
// File Sorting Utilities
// ============================================================================

/// Convert a date format string to a regex pattern.
fn date_format_to_regex(date_format: &str) -> Regex {
    let regex_str = date_format
        .replace("%Y", r"\d{4}")
        .replace("%m", r"\d{2}")
        .replace("%d", r"\d{2}")
        .replace("%H", r"\d{2}")
        .replace("%M", r"\d{2}")
        .replace("%S", r"\d{2}")
        .replace("%z", r"[\+\-]\d{4}")
        .replace("%Z", r"[A-Z]{3}");

    Regex::new(&regex_str).unwrap()
}

/// Extract date from filename using the given format.
fn extract_date_from_filename(filename: &str, format: &str, regex: &Regex) -> NaiveDate {
    regex
        .find(filename)
        .and_then(|m| NaiveDate::parse_from_str(m.as_str(), format).ok())
        .unwrap_or_else(|| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
}

// ============================================================================
// CLI and Main
// ============================================================================

/// Collect PDF files from a directory.
fn collect_pdf_files(dir_path: &str) -> Vec<String> {
    let entries = match fs::read_dir(dir_path) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("Error opening statements directory: {}", err);
            exit(1);
        }
    };

    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .map_or(false, |ext| ext.eq_ignore_ascii_case("pdf"))
        })
        .map(|path| path.to_string_lossy().to_string())
        .collect()
}

/// Sort PDF files by date extracted from filename.
fn sort_files_by_date(files: &mut [String], date_format: &str) {
    let regex = date_format_to_regex(date_format);
    files.sort_by(|a, b| {
        let a_date = extract_date_from_filename(a, date_format, &regex);
        let b_date = extract_date_from_filename(b, date_format, &regex);
        a_date.cmp(&b_date)
    });
}

fn main() -> Result<(), Error> {
    let matches = Command::new("HDFC credit card statement parser")
        .arg(
            arg!(--dir <path_to_directory>)
                .required_unless_present("file")
                .conflicts_with("file"),
        )
        .arg(
            arg!(--file <path_to_file>)
                .required_unless_present("dir")
                .conflicts_with("dir"),
        )
        .arg(arg!(--name <name>).required(true))
        .arg(arg!(--password <password>).required(false))
        .arg(arg!(--sortformat <date_format>).required(false))
        .arg(arg!(--addheaders).required(false))
        .arg(arg!(--summary).required(false))
        .arg(arg!(--categories <categories_file>).required(false))
        .get_matches();

    let dir_path = matches.get_one::<String>("dir");
    let file_path = matches.get_one::<String>("file");
    let name = matches.get_one::<String>("name").cloned().unwrap_or_default();
    let password = matches.get_one::<String>("password").cloned().unwrap_or_default();
    let add_headers = matches.get_flag("addheaders");
    let show_summary = matches.get_flag("summary");
    let categories_path = matches.get_one::<String>("categories");

    // Collect PDF files
    let mut pdf_files = if let Some(dir) = dir_path {
        collect_pdf_files(dir)
    } else if let Some(file) = file_path {
        match fs::metadata(file) {
            Ok(_) => vec![file.to_string()],
            Err(err) => {
                eprintln!("Error opening statement file: {}", err);
                exit(1);
            }
        }
    } else {
        Vec::new()
    };

    // Sort files by date if format specified
    if let Some(sort_format) = matches.get_one::<String>("sortformat") {
        sort_files_by_date(&mut pdf_files, sort_format);
    }

    // Load categories if provided
    let categories: Option<HashMap<String, Vec<String>>> = if let Some(path) = categories_path {
        Some(load_categories(path)?)
    } else {
        None
    };

    // Set up channel for CSV writing
    let (tx, rx) = mpsc::channel();

    let writer_thread = thread::spawn(move || -> Result<(), Error> {
        if show_summary {
            // Summary mode: collect records and show summary only (no CSV)
            let records: Vec<Vec<String>> = rx.into_iter().collect();
            let summary = calculate_summary(&records, &categories);
            print_summary(&summary, categories.is_some());
        } else {
            // Normal mode: write CSV to stdout
            let mut wtr = csv::Writer::from_writer(io::stdout());

            if add_headers {
                wtr.write_record(["Date", "Description", "Points", "Amount"])
                    .context("Failed to write headers")?;
            }

            for record in rx {
                wtr.write_record(&record).context("Failed to write row")?;
            }

            wtr.flush().context("Error flushing to stdout")?;
        }

        Ok(())
    });

    // Parse all PDF files
    for file in pdf_files {
        parse(file, name.clone(), password.clone(), &tx).context("Failed to parse statement")?;
    }

    drop(tx);

    // Wait for writer thread
    match writer_thread.join() {
        Ok(Ok(_)) => (),
        Ok(Err(e)) => return Err(e),
        Err(e) => return Err(anyhow::anyhow!("Writer thread panicked: {:?}", e)),
    }

    Ok(())
}
