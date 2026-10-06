//! `comma info FILE...` prints what CommaEditor detects in a file.
//! `comma check FILE...` verifies that opening and saving a file reproduces it byte for byte.

use std::path::Path;
use std::process::ExitCode;

use comma_core::{Document, LineEnding, QuoteStyle};

const USAGE: &str = "usage:
  comma info FILE...   show encoding, dialect and size
  comma check FILE...  verify that open → save reproduces each file byte for byte";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
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

fn info(path: &Path) -> Result<bool, String> {
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
