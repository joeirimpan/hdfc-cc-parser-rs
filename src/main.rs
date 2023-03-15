mod pdf_tools;

use anyhow::{Context, Error};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use clap::{arg, Command};
use csv::Writer;
use pdf::content::*;
use pdf::file::File as pdfFile;
use pdf_tools::ops_with_text_state;
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
            // For the pdf operations, skip till domestic/internation transactions and then skip till the first occurence of date
            // This guesses the transactions rows.
            let state = ops_with_text_state(&page, &file)
                .skip_while(|(op, _text_state)| match op {
                    Op::TextDraw { ref text } => {
                        let data = text.as_bytes();
                        if let Ok(s) = std::str::from_utf8(data) {
                            return s.trim() != "Domestic Transactions"
                                && s.trim() != "International Transactions";
                        }
                        return true;
                    }
                    _ => return true,
                })
                .skip_while(|(op, _text_state)| match op {
                    Op::TextDraw { ref text } => {
                        let data = text.as_bytes();
                        if let Ok(s) = std::str::from_utf8(data) {
                            let parsed_datetime =
                                NaiveDateTime::parse_from_str(s.trim(), "%d/%m/%Y %H:%M:%S")
                                    .or_else(|_| {
                                        NaiveDate::parse_from_str(s.trim(), "%d/%m/%Y").map(
                                            |date| {
                                                NaiveDateTime::new(
                                                    date,
                                                    NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
                                                )
                                            },
                                        )
                                    });
                            match parsed_datetime {
                                Ok(_) => return false,
                                Err(_) => return true,
                            }
                        }
                        return true;
                    }
                    _ => return true,
                });

            let mut amt_assigned = false;
            let mut col = 0;
            let mut found_row = false;
            let mut transaction = Transaction::default();
            for (op, _text_state) in state {
                match op {
                    Op::TextDraw { ref text } => {
                        let data = text.as_bytes();
                        if let Ok(s) = std::str::from_utf8(data) {
                            let d = s.trim();
                            if d == "" {
                                continue;
                            }

                            // try parsing %d/%m/%Y %H:%M:%S / %d/%m/%Y formats
                            match NaiveDateTime::parse_from_str(d, "%d/%m/%Y %H:%M:%S") {
                                Ok(dt) => {
                                    // we have transaction here, clone it
                                    if col > 0 {
                                        members.push(transaction.clone());
                                        transaction = Transaction::default();
                                    }

                                    transaction.date = dt;
                                    found_row = true;

                                    // reset col
                                    col = 0;
                                }
                                Err(_) => match NaiveDate::parse_from_str(d, "%d/%m/%Y") {
                                    Ok(dt) => {
                                        // we have transaction here, clone it
                                        if col > 0 {
                                            members.push(transaction.clone());
                                            transaction = Transaction::default();
                                        }

                                        transaction.date = NaiveDateTime::new(
                                            dt,
                                            NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
                                        );
                                        found_row = true;

                                        // reset col
                                        col = 0;
                                    }

                                    Err(_) => {
                                        // Check for the descriptio, amount in the same row where the date was found.
                                        if found_row {
                                            // page end. push the transaction to the list and continue.
                                            if amt_assigned {
                                                if col > 3 {
                                                    if let Ok(tx) = String::from_str(s.trim()) {
                                                        if tx == "Cr" {
                                                            transaction.amount *= -1.0;
                                                        }
                                                    }

                                                    members.push(transaction.clone());
                                                    found_row = false;
                                                    transaction = Transaction::default();
                                                    continue;
                                                }
                                            }

                                            col += 1;

                                            // Must be amount?
                                            if col > 1 && d.contains(".") {
                                                if let Ok(amt) = d.replace(",", "").parse::<f32>() {
                                                    amt_assigned = true;
                                                    transaction.amount = amt * -1.0;
                                                    continue;
                                                }
                                            }

                                            // Must be description or debit/credit representation or reward points
                                            if let Ok(tx) = String::from_str(s.trim()) {
                                                // skip empty string
                                                if tx == "" {
                                                    continue;
                                                }

                                                // skip reward points
                                                if let Ok(p) = tx.replace("- ", "-").parse::<i32>()
                                                {
                                                    transaction.points = p;
                                                    continue;
                                                }

                                                // mark it as credit
                                                if col > 2 && tx == "Cr" {
                                                    transaction.amount *= -1.0;
                                                    continue;
                                                }

                                                // assume transaction description to be next to date
                                                if col == 1 {
                                                    transaction.tx = tx;
                                                }
                                            }
                                        }
                                    }
                                },
                            }
                        }
                    }
                    _ => {}
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

    let entries = fs::read_dir(path.unwrap()).unwrap();

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
