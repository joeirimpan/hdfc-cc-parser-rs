# HDFC CC bill parser

This tool parse and extract information from HDFC Bank credit card statements in .csv format. The extracted information can be used for personal finance management or analytics purposes.

## Features

* Extracts transaction details such as date, description, amount
* Multiple pdfs can be parsed and collated into 1 CSV.

## Requirements

* Rust
* HDFC credit card statements

## Usage
* Clone this repository: `git clone https://github.com/joeirimpan/hdfc-cc-parser-rs.git`
* Navigate to the repository directory: cd hdfc-cc-parser-rs
* Build the project: `cargo build --release`
* Run the binary: `./target/release/hdfc-cc-parser-rs </path/to/statements> <password> <output.csv>`

## Why?

A similar python implementation which uses tabula-py took 70s+ to generate a csv with 8 pdfs. With this implementation, it took only 0.02s to generate the same.