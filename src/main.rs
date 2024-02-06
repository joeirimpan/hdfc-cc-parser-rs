use anyhow::{Context, Error};
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use clap::{arg, Command};
use pdf::content::*;
use pdf::file::File as pdfFile;
use regex::Regex;
use std::io;
use std::process::exit;
use std::str::FromStr;
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::{fs, vec};

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
pub fn parse(
    path: String,
    name: String,
    _password: String,
    sender: &Sender<Vec<String>>,
) -> Result<(), Error> {
    let file = pdfFile::<Vec<u8>>::open_password(path.clone(), _password.as_bytes())
        .context(format!("failed to open file {}", path))?;

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

                                        // read till name. (that is the header columns)
                                        match d {
                                            x if x == name => {
                                                header_assigned = true;
                                                // +1 considering 'Cr' (credit/debit)
                                                header_column_ct += 1;
                                                continue;
                                            }
                                            "" | _ => continue,
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
                                if !header_assigned {
                                    match prev_value {
                                        "" => continue,
                                        "Domestic Transactions" | "International Transactions" => {
                                            continue
                                        }
                                        _ => header_column_ct += 1,
                                    }
                                }
                            }

                            Op::EndText => {
                                match column_ct {
                                    // ignore 0 column_ct
                                    0 => continue,

                                    x if x == header_column_ct && found_row => {
                                        // write to stdout
                                        sender
                                            .send(vec![
                                                transaction.date.to_string(),
                                                transaction.tx.clone(),
                                                transaction.points.to_string(),
                                                transaction.amount.to_string(),
                                            ])
                                            .context("Failed to write row")?;

                                        // reset found flag
                                        found_row = false;
                                        transaction = Transaction::default();
                                        column_ct = 0;
                                    }

                                    _ => continue,
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    Ok(())
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
        .get_matches();

    let dir_path = matches.get_one::<String>("dir");
    let file_path = matches.get_one::<String>("file");
    let name = matches.get_one::<String>("name");
    let _password = matches.get_one::<String>("password");
    let add_headers = matches.get_flag("addheaders");

    let mut pdf_files = Vec::new();

    // path is directory?
    if let Some(dir_path) = dir_path {
        let entries = match fs::read_dir(dir_path) {
            Ok(file) => file,
            Err(err) => {
                eprintln!("Error opening statements directory: {}", err);
                exit(1);
            }
        };

        // Filter pdf files, sort the statement files based on dates in the file names.
        pdf_files = entries
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
    }

    // path is file?
    if let Some(file_path) = file_path {
        match fs::metadata(file_path) {
            Ok(_) => pdf_files.push(file_path.to_string()),
            Err(err) => {
                eprintln!("Error opening statement file: {}", err);
                exit(1);
            }
        };
    }

    let (tx, rx) = mpsc::channel();

    let writer_thread = thread::spawn(move || -> Result<(), Error> {
        let mut wtr = csv::Writer::from_writer(io::stdout());

        if add_headers {
            //  writes the header rows to CSV if user passes --addheaders param
            wtr.write_record(&["Date", "Description", "Points", "Amount"])
                .context("Failed to write headers")?;
        }

        for record in rx {
            wtr.write_record(&record).context("Failed to write row")?;
        }

        wtr.flush().context("Error flushing to stdout")?;
        Ok(())
    });

    let pass: String = match _password {
        Some(s) => s.clone(),
        None => "".to_string(),
    };

    let n: String = match name {
        Some(s) => s.clone(),
        None => "".to_string(),
    };

    for file in pdf_files {
        parse(file, n.clone(), pass.clone(), &tx).context("Failed to parse statement")?;
    }

    drop(tx);

    match writer_thread.join() {
        Ok(Ok(_)) => (),
        Ok(Err(e)) => return Err(e.into()),
        Err(e) => return Err(anyhow::anyhow!("Thread panicked: {:?}", e)),
    }

    Ok(())
}
