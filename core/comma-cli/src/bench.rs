//! `comma bench [ROWS...]` generates tables of the given sizes and times what the app does with
//! a large file: the first screen, reading the rest, sorting by a number column and filtering.

use std::fs::{self, File};
use std::io::{BufWriter, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use comma_core::{Document, Filter, FilterOp, OpenOptions, SortKey, ViewSpec};

/// Bytes indexed per background step, as `CSVDocument.indexingStep` in the app.
const INDEXING_STEP: usize = 64 << 20;
const DEFAULT_ROWS: [usize; 3] = [1_000_000, 12_000_000, 60_000_000];
const BALANCE: usize = 5;
const CITY: usize = 3;
const CITIES: u64 = 500;
const FILTERED_CITY: u64 = 42;

pub fn run(args: &[String]) -> Result<(), String> {
    let mut sizes = Vec::new();
    let mut runs = None;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--runs" {
            let value = args.next().ok_or("--runs needs a number")?;
            runs = Some(value.parse::<usize>().ok().filter(|&n| n > 0).ok_or(format!("not a number of runs: {value}"))?);
        } else {
            sizes.push(parse_rows(arg).ok_or(format!("not a number of rows: {arg}"))?);
        }
    }
    if sizes.is_empty() {
        sizes.extend(DEFAULT_ROWS);
    }

    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    println!("seconds, best of several runs; {threads} threads; the file is in the system cache");
    println!(
        "{:>12}  {:>9}  {:>12}  {:>9}  {:>7}  {:>7}",
        "rows", "size", "first screen", "full read", "sort", "filter"
    );
    for rows in sizes {
        let file = TempFile(std::env::temp_dir().join(format!("comma-bench-{rows}.csv")));
        let terminal = std::io::stderr().is_terminal();
        if terminal {
            eprint!("generating {} rows in {}…\r", group(rows), file.0.display());
        }
        let expected = generate(rows, &file.0).map_err(|e| e.to_string())?;
        if terminal {
            eprint!("\x1b[2K");
        }
        let size = fs::metadata(&file.0).map_err(|e| e.to_string())?.len();
        let result = measure(&file.0, runs.unwrap_or(if rows > 20_000_000 { 3 } else { 5 }), expected)?;
        println!(
            "{:>12}  {:>9}  {:>12.3}  {:>9.3}  {:>7.3}  {:>7.3}",
            group(rows),
            megabytes(size),
            result.first,
            result.full,
            result.sort,
            result.filter
        );
    }
    Ok(())
}

/// `1m`, `500k`, `12000000` or `12_000_000`.
fn parse_rows(text: &str) -> Option<usize> {
    let text = text.to_ascii_lowercase().replace('_', "");
    let (digits, scale) = match text.strip_suffix('m') {
        Some(digits) => (digits, 1_000_000),
        None => match text.strip_suffix('k') {
            Some(digits) => (digits, 1_000),
            None => (text.as_str(), 1),
        },
    };
    digits.parse::<usize>().ok().filter(|&n| n > 0).map(|n| n * scale)
}

/// Removes the generated file however the run ends.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Writes a header and `rows` rows of 10 columns, about 88 bytes a row; returns how many rows
/// the filter should keep.
fn generate(rows: usize, path: &Path) -> std::io::Result<usize> {
    let mut out = BufWriter::with_capacity(1 << 20, File::create(path)?);
    writeln!(out, "id,name,email,city,date,balance,note,a,b,c")?;
    let mut matches = 0;
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    for i in 0..rows {
        // xorshift: the same file on every run, with sums in no particular order.
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let city = x % CITIES;
        matches += usize::from(city == FILTERED_CITY);
        writeln!(
            out,
            "{i},Name {i},user{i}@example.com,City {city},2024-{:02}-{:02},{}.{:02},\"note, {}\",x,y,z",
            x % 12 + 1,
            x % 28 + 1,
            x % 100_000,
            x / 100_000 % 100,
            x % 1000
        )?;
    }
    out.flush()?;
    Ok(matches)
}

struct Timings {
    first: f64,
    full: f64,
    sort: f64,
    filter: f64,
}

fn measure(path: &Path, runs: usize, expected: usize) -> Result<Timings, String> {
    let first = best(runs, || timed(|| open_first_part(path)))?;
    let full = best(runs, || timed(|| open_whole(path)))?;
    let mut document = open_whole(path)?;

    let sort = ViewSpec { sort: vec![SortKey { column: BALANCE, ascending: true }], filters: vec![] };
    let sort_time = best(runs, || view(&mut document, &sort))?;
    let balance = |document: &Document, row: usize| -> Result<f64, String> {
        let values = document.visible_row(row).map_err(|e| e.to_string())?;
        values[BALANCE].parse().map_err(|_| format!("not a number: {}", values[BALANCE]))
    };
    let last = document.visible_row_count() - 1;
    if balance(&document, 0)? > balance(&document, last)? {
        return Err("sorting left the rows out of order".into());
    }

    let filter = ViewSpec {
        sort: vec![],
        filters: vec![Filter {
            column: Some(CITY),
            op: FilterOp::Equals,
            value: format!("City {FILTERED_CITY}"),
            case_sensitive: false,
        }],
    };
    let filter_time = best(runs, || view(&mut document, &filter))?;
    if document.visible_row_count() != expected {
        return Err(format!("the filter kept {} rows instead of {expected}", document.visible_row_count()));
    }
    Ok(Timings { first, full, sort: sort_time, filter: filter_time })
}

/// The shortest of `runs` runs, each returning its seconds.
fn best(runs: usize, mut run: impl FnMut() -> Result<f64, String>) -> Result<f64, String> {
    let mut best = f64::MAX;
    for _ in 0..runs {
        best = best.min(run()?);
    }
    Ok(best)
}

/// Seconds `work` took; freeing what it made is not counted.
fn timed<T>(work: impl FnOnce() -> Result<T, String>) -> Result<f64, String> {
    let started = Instant::now();
    let made = work()?;
    let seconds = started.elapsed().as_secs_f64();
    drop(made);
    Ok(seconds)
}

/// What the app does before it shows a large file.
fn open_first_part(path: &Path) -> Result<Document, String> {
    Document::open_partially(path, &OpenOptions::default(), Document::FIRST_PART)
        .map(|(document, _)| document)
        .map_err(|e| e.to_string())
}

/// The first part, then the rest in steps, as the app reads it in the background.
fn open_whole(path: &Path) -> Result<Document, String> {
    let (mut document, indexer) =
        Document::open_partially(path, &OpenOptions::default(), Document::FIRST_PART).map_err(|e| e.to_string())?;
    if let Some(mut indexer) = indexer {
        loop {
            let chunk = indexer.next_chunk(INDEXING_STEP);
            let done = chunk.is_done();
            document.append_index(chunk);
            if done {
                break;
            }
        }
    }
    Ok(document)
}

/// Sorts or filters as the app does for a large table: on a snapshot, then shown. The view
/// is reset first, untimed, so every run does the whole work; returns the seconds.
fn view(document: &mut Document, spec: &ViewSpec) -> Result<f64, String> {
    document.set_view(ViewSpec::default()).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut task = document.begin_view(spec.clone()).map_err(|e| e.to_string())?;
    let finished = task.run(&AtomicBool::new(false)) && document.finish_view(task);
    let seconds = started.elapsed().as_secs_f64();
    if finished { Ok(seconds) } else { Err("the view was not computed".into()) }
}

fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(digit);
    }
    out
}

fn megabytes(bytes: u64) -> String {
    let mb = bytes as f64 / 1e6;
    if mb >= 1000.0 { format!("{:.1} GB", mb / 1000.0) } else { format!("{mb:.0} MB") }
}
