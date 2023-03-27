use anyhow::{Context, Error};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use clap::{arg, Command};
use csv::Writer;
use std::process::exit;
use pdf::content::*;
use pdf::file::File as pdfFile;
use regex::Regex;
use std::fs;
use std::fs::File;
use std::str::FromStr;

// Transaction row representation.
#[derive(Debug, Clone)]
pub struct Transaction {
    pub date: NaiveDateTime,
    pub tx: String,
    pub points: i32,
    pub amount: f32,
}

// default values for new Transaction.
impl Default for Transaction {
    fn default() -> Self {
        Transaction {
            date: NaiveDateTime::new(
                NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ),
            tx: "".to_owned(),
            points: 0,
            amount: 0.0,
        }
    }
}

// Parse the pdf and return a list of transactions.
pub fn parse(path: String, _password: String) -> Result<Vec<Transaction>, Error> {
    let file = pdfFile::<Vec<u8>>::open_password(path.clone(), _password.as_bytes())
        .context(format!("failed to open file {}", path))?;

    let mut members = Vec::new();

    // Iterate through pages
    for page in file.pages() {
        if let Ok(page) = page {
            if let Some(content) = &page.contents {
                if let Ok(ops) = content.operations(&file) {
                    let mut transaction = Transaction::default();

                    let mut found_row = false;
                    let mut column_ct = 0;
                    let mut header_assigned = false;
                    let mut header_column_ct = 0;
                    let mut prev_value = "";

                    for op in ops.iter().skip_while(|op| match op {
                        Op::TextDraw { ref text } => {
                            let data = text.as_bytes();
                            if let Ok(s) = std::str::from_utf8(data) {
                                return s.trim() != "Domestic Transactions"
                                    && s.trim() != "International Transactions";
                            }
                            return true;
                        }
                        _ => return true,
                    }) {
                        match op {
                            Op::TextDraw { ref text } => {
                                let data = text.as_bytes();
                                if let Ok(s) = std::str::from_utf8(data) {
                                    // figure out the header column count from the table header.
                                    // This makes it easier to figure out the end of transaction lines.
                                    let d = s.trim();
                                    if !header_assigned {
                                        // save this value to check in next iteration of Op::BeginText to count header columns.
                                        prev_value = d;
                                        if d == "" {
                                            continue;
                                        }

                                        // XXX: assume the transaction row starts with a date.
                                        let parsed_datetime =
                                            NaiveDateTime::parse_from_str(d, "%d/%m/%Y %H:%M:%S")
                                                .or_else(|_| {
                                                    NaiveDate::parse_from_str(d, "%d/%m/%Y").map(
                                                        |date| {
                                                            NaiveDateTime::new(
                                                                date,
                                                                NaiveTime::from_hms_opt(0, 0, 0)
                                                                    .unwrap(),
                                                            )
                                                        },
                                                    )
                                                });

                                        match parsed_datetime {
                                            Ok(_) => {
                                                header_assigned = true;
                                                // remove card holder name
                                                header_column_ct -= 1;
                                                prev_value = "";
                                            }
                                            Err(_) => continue,
                                        }
                                    }

                                    column_ct += 1;
                                    if d == "" {
                                        if !found_row {
                                            column_ct -= 1;
                                        }

                                        continue;
                                    }

                                    if column_ct == 1 {
                                        if let Ok(tx_date) =
                                            NaiveDateTime::parse_from_str(d, "%d/%m/%Y %H:%M:%S")
                                        {
                                            found_row = true;
                                            transaction.date = tx_date;
                                            continue;
                                        }
                                        if let Ok(tx_date) =
                                            NaiveDate::parse_from_str(d, "%d/%m/%Y")
                                        {
                                            found_row = true;
                                            transaction.date = NaiveDateTime::new(
                                                tx_date,
                                                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
                                            );
                                            continue;
                                        }
                                    }

                                    if column_ct > 2 && d.contains(".") {
                                        if let Ok(amt) = d.replace(",", "").parse::<f32>() {
                                            transaction.amount = amt * -1.0;
                                            continue;
                                        }
                                    }

                                    // Must be description or debit/credit representation or reward points
                                    if let Ok(tx) = String::from_str(d) {
                                        // skip empty string
                                        if tx == "" {
                                            continue;
                                        }

                                        // skip reward points
                                        if let Ok(p) = tx.replace("- ", "-").parse::<i32>() {
                                            transaction.points = p;
                                            continue;
                                        }

                                        // mark it as credit
                                        if column_ct > 3 && tx == "Cr" {
                                            transaction.amount *= -1.0;
                                            continue;
                                        }

                                        // assume transaction description to be next to date
                                        if column_ct == 2 {
                                            transaction.tx = tx;
                                        }
                                    }
                                }
                            }

                            Op::BeginText => {
                                if !header_assigned && prev_value != "" {
                                    header_column_ct += 1;
                                }
                            }

                            Op::EndText => {
                                if found_row && column_ct == header_column_ct {
                                    // push transaction here
                                    members.push(transaction.clone());

                                    // reset found flag
                                    found_row = false;
                                    transaction = Transaction::default();
                                    column_ct = 0;
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    Ok(members)
}

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

fn main() -> Result<(), Error> {
    let matches = Command::new("HDFC credit card statement parser")
        .arg(arg!(--dir <path_to_directory>).required(true))
        .arg(arg!(--password <password>).required(false))
        .arg(arg!(--output <output>).required(true))
        .arg(arg!(--sortformat <date_format>).required(false))
        .get_matches();

    let path = matches.get_one::<String>("dir");
    let _password = matches.get_one::<String>("password");
    let output = matches.get_one::<String>("output").unwrap().to_string();

    // path is directory?
    let entries = match fs::read_dir(path.unwrap()) {
        Ok(file) => file,
        Err(err) => {
            eprintln!("Error opening statements directory: {}", err);
            exit(1);
        }
    };

    // Filter pdf files, sort the statement files based on dates in the file names.
    let mut pdf_files: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .map_or(false, |ext| ext == "pdf" || ext == "PDF")
        })
        .map(|path| path.to_string_lossy().to_string())
        .collect();

    // Sort only if there is a date format specified
    if let Some(sort_format) = matches.get_one::<String>("sortformat") {
        pdf_files.sort_by(|a, b| {
            let re = date_format_to_regex(sort_format);
            let a_date = match re.find(a) {
                Some(date_str) => {
                    NaiveDate::parse_from_str(date_str.as_str(), sort_format).unwrap()
                }
                None => NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            };
            let b_date = match re.find(b) {
                Some(date_str) => {
                    NaiveDate::parse_from_str(date_str.as_str(), sort_format).unwrap()
                }
                None => NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
            };
            a_date.cmp(&b_date)
        })
    }

    // Parse all the statement files.
    let mut members = Vec::new();
    for file in pdf_files {
        members.extend(
            parse(file, _password.unwrap_or(&"".to_string()).to_string())
                .context("Failed to parse statement")?,
        )
    }

    // Create a csv file and write the contents of the transaction list
    let w = File::create(output).context("Unable to create output file")?;
    let mut csv_writer = Writer::from_writer(w);

    for member in members {
        let row = &[
            member.date.to_string(),
            member.tx.clone(),
            member.points.to_string(),
            member.amount.to_string(),
        ];

        csv_writer
            .write_record(row)
            .context("Failed to write row")?
    }

    Ok(())
}
