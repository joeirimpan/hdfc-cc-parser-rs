# HDFC CC bill parser

This tool parse and extract information from HDFC Bank credit card statements in .csv format. The extracted information can be used for personal finance management or analytics purposes.

## Features

* Extracts transaction details such as date, description, points, amount
* Multiple pdfs can be parsed and collated into 1 CSV.

## Requirements

* Rust
* HDFC credit card statements

## Usage
* Clone this repository: `git clone https://github.com/joeirimpan/hdfc-cc-parser-rs.git`
* Navigate to the repository directory: cd hdfc-cc-parser-rs
* Build the project: `cargo build --release`
* Run the binary: `./target/release/hdfc-cc-parser-rs --dir <optional statements directory> --file <optional file path> --password <optional password> --sortformat="optional format eg., %d-%m-%Y"`

## Why?

A similar python implementation which uses tabula-py took 70s+ to generate a csv with 8 pdfs. With this implementation, it took only 0.02s to generate the same.

## Analytics

Assuming `clickhouse-local` is installed

* Get the points accumulated
```bash
cat output.csv | clickhouse-local --structure "tx_date Datetime, tx String, points Int32, amount Float32" --query "SELECT SUM(points) FROM table" --input-format CSV
```

* Get the debits
```bash
cat output.csv | clickhouse-local --structure "tx_date Datetime, tx String, points Int32, amount Float32" --query "SELECT SUM(amount) FROM table WHERE amount < 0" --input-format CSV
```