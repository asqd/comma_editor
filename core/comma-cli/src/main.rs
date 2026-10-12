//! `comma info FILE...` prints what CommaEditor detects in a file, or the sheets of a workbook.
//! `comma check FILE...` verifies that opening and saving a file reproduces it byte for byte.
//! `comma bench [ROWS...]` times opening, reading, sorting and filtering generated tables.
//! `comma generate ROWS FILE` writes such a table.

mod bench;

use std::path::Path;
use std::process::ExitCode;

use std::time::Instant;

use comma_core::{DisplayLocale, Document, LineEnding, QuoteStyle, SheetKind, SheetVisibility, Workbook, WorkbookFormat};

const USAGE: &str = "usage:
  comma info FILE...   show encoding, dialect and size; for .xlsx and .xls, the sheets
  comma check FILE...  verify that open → save reproduces each file byte for byte
  comma bench [ROWS...] [--runs N]
                       time the first screen, reading, sorting and filtering of generated
                       tables (default 1m 12m 60m rows; the files go to the temp folder)
  comma generate ROWS FILE
                       write a table the bench uses, ROWS rows of 10 columns (e.g. 60m)";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let tool: Option<(&str, fn(&[String]) -> Result<(), String>)> = match args.first().map(String::as_str) {
        Some("bench") => Some(("bench", bench::run)),
        Some("generate") => Some(("generate", bench::generate_file)),
        _ => None,
    };
    if let Some((name, run)) = tool {
        return match run(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("{name}: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let (command, files) = match args.split_first() {
        Some((command, files)) if !files.is_empty() => (command.as_str(), files),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let run: fn(&Path) -> Result<bool, String> = match command {
        "info" => info,
        "check" => check,
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let mut ok = true;
    for file in files {
        match run(Path::new(file)) {
            Ok(passed) => ok &= passed,
            Err(error) => {
                eprintln!("{file}: {error}");
                ok = false;
            }
        }
    }
    if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

fn is_workbook(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| ["xlsx", "xlsm", "xls"].iter().any(|known| extension.eq_ignore_ascii_case(known)))
}

fn info(path: &Path) -> Result<bool, String> {
    if is_workbook(path) {
        return workbook_info(path);
    }
    let document = Document::open(path).map_err(|e| e.to_string())?;
    let info = document.info();
    let delimiter = match info.dialect.delimiter {
        b'\t' => "tab".to_owned(),
        byte => format!("'{}'", byte as char),
    };
    let line_ending = match info.dialect.line_ending {
        LineEnding::Lf => "LF",
        LineEnding::CrLf => "CRLF",
        LineEnding::Cr => "CR",
    };
    let quoting = match info.dialect.quote_style {
        QuoteStyle::Minimal => "when needed",
        QuoteStyle::Always => "always",
    };
    println!("{}", path.display());
    println!(
        "  encoding:   {}{}",
        info.encoding.name(),
        if info.has_bom { " with BOM" } else { "" }
    );
    println!("  delimiter:  {delimiter}");
    println!(
        "  line ends:  {line_ending}{}",
        if info.trailing_newline { "" } else { ", no final line break" }
    );
    println!("  quoting:    {quoting}");
    println!(
        "  size:       {} rows × {} columns{}",
        info.row_count,
        info.column_count,
        if info.has_header { ", first row is a header" } else { "" }
    );
    if info.unterminated_quote {
        println!("  warning:    a quoted value is never closed");
    }
    Ok(true)
}

fn workbook_info(path: &Path) -> Result<bool, String> {
    let started = Instant::now();
    let workbook = Workbook::open(path).map_err(|e| e.to_string())?;
    println!("{}", path.display());
    let format = match workbook.format() {
        WorkbookFormat::Xlsx => "Excel workbook",
        WorkbookFormat::Xls => "Excel 97–2003 workbook",
    };
    println!("  format:     {format}, opened in {:.2} s", started.elapsed().as_secs_f64());
    if workbook.uses_1904_dates() {
        println!("  dates:      from 1904");
    }
    let locale = DisplayLocale::default();
    for (index, entry) in workbook.sheets().iter().enumerate() {
        let state = match entry.visibility {
            SheetVisibility::Visible => "",
            SheetVisibility::Hidden => ", hidden",
            SheetVisibility::VeryHidden => ", very hidden",
        };
        if entry.kind != SheetKind::Worksheet {
            println!("  sheet {}:    “{}”: not a table{state}", index + 1, entry.name);
            continue;
        }
        let started = Instant::now();
        let sheet = workbook.read_sheet(index, &locale).map_err(|e| e.to_string())?;
        let document = &sheet.document;
        let formulas = (0..document.row_count())
            .map(|row| (0..document.column_count()).filter(|&column| document.formula(row, column).is_some()).count())
            .sum::<usize>();
        println!(
            "  sheet {}:    “{}”: {} rows × {} columns, {formulas} formulas, {} merged ranges{}{state}, read in {:.2} s",
            index + 1,
            entry.name,
            document.row_count(),
            document.column_count(),
            sheet.layout.merged.len(),
            if document.has_header() { ", first row is a header" } else { "" },
            started.elapsed().as_secs_f64(),
        );
    }
    Ok(true)
}

fn check(path: &Path) -> Result<bool, String> {
    let original = std::fs::read(path).map_err(|e| e.to_string())?;
    let saved = Document::from_bytes(original.clone())
        .to_bytes()
        .map_err(|e| e.to_string())?;
    if saved == original {
        println!("ok        {}", path.display());
        return Ok(true);
    }
    let offset = original
        .iter()
        .zip(&saved)
        .position(|(a, b)| a != b)
        .unwrap_or(original.len().min(saved.len()));
    println!(
        "MISMATCH  {} (first difference at byte {offset})",
        path.display()
    );
    Ok(false)
}
