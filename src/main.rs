use anyhow::{Context, Error};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use csv::Writer;
use pdf::content::*;
use pdf::file::File as pdfFile;
use pdf_tools::ops_with_text_state;
use regex::Regex;
use std::env::args;
use std::fs;
use std::fs::File;
use std::str::FromStr;

#[derive(Debug)]
pub struct TransactionList {
    pub members: Vec<Transaction>,
}

#[derive(Debug, Clone)]
pub struct Transaction {
    pub date: NaiveDateTime,
    pub tx: String,
    pub amount: f32,
}

impl Default for Transaction {
    fn default() -> Self {
        Transaction {
            date: NaiveDateTime::new(
                NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
                NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
            ),
            tx: "".to_owned(),
            amount: 0.0,
        }
    }
}

pub fn parse(path: String, _password: String) -> Result<TransactionList, Error> {
    let file = pdfFile::<Vec<u8>>::open_password(path.clone(), _password.as_bytes())
        .context(format!("failed to open file {}", path))?;

    let mut members = Vec::new();

    for page in file.pages() {
        if let Ok(page) = page {
            let mut flag = false;
            let mut intl_flag = false;
            let mut skip_header = 11;
            let mut column_ct: i32 = 4;
            let mut transaction = Transaction::default();
            for (op, _text_state) in ops_with_text_state(&page, &file) {
                match op {
                    Op::TextDraw { ref text } => {
                        if flag && skip_header > 0 {
                            skip_header -= 1;
                            continue;
                        }

                        let data = text.as_bytes();
                        if let Ok(s) = std::str::from_utf8(data) {
                            if flag {
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

                                if let Ok(d) = parsed_datetime {
                                    // Date field found, count for the next 4 columns
                                    transaction.date = d;
                                    column_ct = 4;
                                } else {
                                    match column_ct {
                                        4 => transaction.tx = String::from_str(s.trim()).unwrap(),
                                        3 => {}
                                        2 => {
                                            if s.trim() == "" {
                                                continue;
                                            }
                                            transaction.amount =
                                                s.trim().replace(",", "").parse::<f32>().unwrap()
                                        }
                                        1 => {
                                            if intl_flag {
                                                transaction.amount = s
                                                    .trim()
                                                    .replace(",", "")
                                                    .parse::<f32>()
                                                    .unwrap()
                                            }
                                            if !intl_flag && s.trim() != "Cr" {
                                                transaction.amount *= -1.0;
                                            }
                                            members.push(transaction.clone());
                                        }
                                        0 => {}
                                        _ => {}
                                    }
                                    column_ct -= 1;
                                }
                            }

                            match s.trim() {
                                "Domestic Transactions" => {
                                    intl_flag = false;
                                    flag = true;
                                }
                                "International Transactions" => {
                                    flag = true;
                                    intl_flag = true;
                                }
                                _ => {}
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(TransactionList { members })
}

fn main() -> Result<(), Error> {
    let path = args().nth(1).expect("no dir given");
    let _password = args().nth(2).expect("no password given");
    let output = args().nth(3).expect("no output file given");

    let entries = fs::read_dir(path).unwrap();

    // Filter pdf files, sort the statement files based on dates in the file names.
    let re = Regex::new(r"(\d{1,2}-\d{2}-\d{4})").unwrap();
    let mut pdf_files: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .map_or(false, |ext| ext == "pdf" || ext == "PDF")
        })
        .map(|path| path.to_string_lossy().to_string())
        .collect();
    pdf_files.sort_by(|a, b| {
        let a_date = NaiveDate::parse_from_str(&re.captures(a).unwrap()[1], "%d-%m-%Y").unwrap();
        let b_date = NaiveDate::parse_from_str(&re.captures(b).unwrap()[1], "%d-%m-%Y").unwrap();
        a_date.cmp(&b_date)
    });

    // Parse all the statement files.
    let mut members = Vec::new();
    for file in pdf_files {
        members.extend(
            parse(file, _password.clone())
                .context("Failed to parse statement")?
                .members,
        )
    }

    let w = File::create(output).context("Unable to create output file")?;
    let mut csv_writer = Writer::from_writer(w);

    for member in members {
        let row = &[
            member.date.to_string(),
            member.tx.clone(),
            member.amount.to_string(),
        ];

        csv_writer
            .write_record(row)
            .context("Failed to write row")?
    }

    Ok(())
}
