//! Export of the visible rows to JSON, Markdown, HTML, SQL and Excel.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::ops::Range;
use std::path::Path;

use rayon::prelude::*;
use rust_xlsxwriter::{Format, Workbook};

use crate::document::Document;
use crate::error::{Error, Result};
use crate::range::CellRange;
use crate::view::{parse_number, strip_currency};
use crate::xlsx::DisplayLocale;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    /// An array of objects keyed by column name.
    Json,
    /// A GitHub-flavoured Markdown table.
    Markdown,
    Html,
    /// `CREATE TABLE` and one `INSERT` per row.
    Sql,
    /// An Excel workbook with one sheet.
    Xlsx,
}

/// Excel's row limit, the header row included.
const XLSX_MAX_ROWS: usize = 1_048_576;
/// Rows built at once when exporting to a file; each batch is written before the next.
const EXPORT_BATCH: usize = 1 << 20;
/// Rows one core builds into one piece of text.
const EXPORT_BLOCK: usize = 1 << 14;

struct Columns {
    indices: Range<usize>,
    names: Vec<String>,
    /// Per column: the values as numbers, when every non-empty value is one.
    numeric: Vec<bool>,
}

impl Document {
    /// The visible rows of `range` (every visible cell when `None`), with column names from the
    /// header row or column letters. `name` titles the HTML page, SQL table and sheet. A
    /// `fragment` is for the clipboard: HTML without the page around the table, SQL without
    /// `CREATE TABLE`.
    ///
    /// Numbers are exported as numbers only in columns where every value is one, written
    /// plainly (no leading zeros, at most 15 digits); Excel also takes `1 250,50`-style numbers.
    pub fn export(&self, format: ExportFormat, range: Option<CellRange>, name: &str, fragment: bool) -> Result<Vec<u8>> {
        let (columns, rows) = self.export_scope(format, range)?;
        if format == ExportFormat::Xlsx {
            return self.export_xlsx(&columns, rows, name);
        }
        let text = TextExport::new(self, format, columns, rows, name, fragment);
        let mut out = String::new();
        text.head(&mut out);
        for block in text.render(0..text.rows.len()) {
            out.push_str(&block);
        }
        text.tail(&mut out);
        Ok(out.into_bytes())
    }

    /// Writes the export of the visible rows to `path`. Rows are built on all cores a batch at
    /// a time and written as they are ready, so a large table is never held in memory whole.
    pub fn export_to_file(&self, path: impl AsRef<Path>, format: ExportFormat, name: &str) -> Result<()> {
        let (columns, rows) = self.export_scope(format, None)?;
        let mut out = BufWriter::with_capacity(1 << 20, File::create(path)?);
        if format == ExportFormat::Xlsx {
            out.write_all(&self.export_xlsx(&columns, rows, name)?)?;
        } else {
            let text = TextExport::new(self, format, columns, rows, name, false);
            let mut part = String::new();
            text.head(&mut part);
            out.write_all(part.as_bytes())?;
            let count = text.rows.len();
            for batch in (0..count).step_by(EXPORT_BATCH) {
                for block in text.render(batch..(batch + EXPORT_BATCH).min(count)) {
                    out.write_all(block.as_bytes())?;
                }
            }
            part.clear();
            text.tail(&mut part);
            out.write_all(part.as_bytes())?;
        }
        out.flush()?;
        Ok(())
    }

    fn export_scope(&self, format: ExportFormat, range: Option<CellRange>) -> Result<(Columns, Range<usize>)> {
        let range = match range {
            Some(range) => {
                self.check_range(&range)?;
                range
            }
            None => CellRange {
                top: 0,
                left: 0,
                bottom: self.visible_row_count().saturating_sub(1),
                right: self.column_count().saturating_sub(1),
            },
        };
        let rows = if self.visible_row_count() == 0 { 0..0 } else { range.rows() };
        let columns = self.export_columns(range.columns(), rows.clone(), format == ExportFormat::Xlsx);
        Ok((columns, rows))
    }

    fn export_columns(&self, indices: Range<usize>, rows: Range<usize>, lenient: bool) -> Columns {
        let header = (self.header_offset() == 1).then_some(0);
        let mut seen = HashSet::new();
        let names = indices
            .clone()
            .map(|column| {
                let name = header
                    .map(|row| self.value_at(row, column).trim().to_owned())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| column_letter(column));
                // Keys must be unique in JSON and SQL.
                let mut unique = name.clone();
                let mut n = 2;
                while !seen.insert(unique.clone()) {
                    unique = format!("{name}_{n}");
                    n += 1;
                }
                unique
            })
            .collect();
        // Per column: whether every filled value is a number, and whether any value is filled.
        let numeric = indices
            .clone()
            .map(|column| {
                let (all, any) = rows
                    .clone()
                    .into_par_iter()
                    .filter_map(|row| self.visible_record(row))
                    .map(|record| {
                        let value = self.value_at(record, column);
                        if value.trim().is_empty() {
                            (true, false)
                        } else {
                            (number(&value, lenient).is_some(), true)
                        }
                    })
                    .reduce(|| (true, false), |a, b| (a.0 && b.0, a.1 || b.1));
                all && any
            })
            .collect();
        Columns { indices, names, numeric }
    }

    /// Values of a visible row in the exported columns.
    fn export_row(&self, row: usize, columns: &Columns) -> Vec<String> {
        let record = self.visible_record(row).expect("visible row");
        columns
            .indices
            .clone()
            .map(|column| self.value_at(record, column).into_owned())
            .collect()
    }

    fn export_xlsx(&self, columns: &Columns, rows: Range<usize>, name: &str) -> Result<Vec<u8>> {
        // Column names make a first row only when the table has a header row: a sheet shown
        // as Excel shows it, or a file without one, starts with its own first row.
        let header = u32::from(self.header_offset() == 1);
        if rows.len() + header as usize > XLSX_MAX_ROWS {
            return Err(Error::TooManyRowsForExcel { rows: rows.len(), limit: XLSX_MAX_ROWS - header as usize });
        }
        let xlsx = |error: rust_xlsxwriter::XlsxError| Error::Export(error.to_string());
        let mut workbook = Workbook::new();
        let sheet = workbook.add_worksheet();
        let sheet_name: String = name
            .chars()
            .filter(|c| !"[]:*?/\\".contains(*c))
            .take(31)
            .collect();
        if !sheet_name.trim().is_empty() {
            sheet.set_name(sheet_name.trim()).map_err(xlsx)?;
        }
        if header == 1 {
            let bold = Format::new().set_bold();
            for (column, name) in columns.names.iter().enumerate() {
                sheet.write_string_with_format(0, column as u16, name, &bold).map_err(xlsx)?;
            }
        }
        // A sheet of a workbook: each cell read back from how it is shown, with a format that
        // shows it the same way. A delimited file: numbers only in all-number columns.
        let locale = self.sheet.as_ref().map(|sheet| &sheet.locale);
        let mut formats: HashMap<String, Format> = HashMap::new();
        // Widest value shown per column: the autofit of the writer measures bare numbers, not
        // `1 120,00 ₽` or a date.
        let mut widths: Vec<usize> = columns.names.iter().map(|name| if header == 1 { name.chars().count() } else { 0 }).collect();
        let row_count = rows.len();
        for (index, row) in rows.enumerate() {
            let excel_row = index as u32 + header;
            for (column, value) in self.export_row(row, columns).iter().enumerate() {
                if value.is_empty() {
                    continue;
                }
                let column_number = column as u16;
                widths[column] = widths[column].max(value.chars().count());
                let cell = match locale {
                    Some(locale) => sheet_cell(value, locale),
                    None => match columns.numeric[column].then(|| number(value, true)).flatten() {
                        Some(number) => SheetCell::Number(number, None),
                        None => SheetCell::Text,
                    },
                };
                match cell {
                    SheetCell::Number(number, None) => sheet.write_number(excel_row, column_number, number).map_err(xlsx)?,
                    SheetCell::Number(number, Some(code)) => {
                        let format = formats.entry(code).or_insert_with_key(|code| Format::new().set_num_format(code));
                        sheet.write_number_with_format(excel_row, column_number, number, format).map_err(xlsx)?
                    }
                    SheetCell::Bool(truth) => sheet.write_boolean(excel_row, column_number, truth).map_err(xlsx)?,
                    SheetCell::Text => sheet.write_string(excel_row, column_number, value).map_err(xlsx)?,
                };
            }
        }
        if header == 1 {
            sheet.set_freeze_panes(1, 0).map_err(xlsx)?;
        }
        if locale.is_some() {
            for (column, width) in widths.iter().enumerate() {
                sheet.set_column_width(column as u16, (*width as f64 * 1.1 + 2.0).min(60.0)).map_err(xlsx)?;
            }
        } else if row_count <= 20_000 {
            sheet.autofit();
        }
        workbook.save_to_buffer().map_err(xlsx)
    }
}

/// A text format with its beginning, one piece per row and its end, so rows can be built on
/// all cores and joined in order.
struct TextExport<'a> {
    document: &'a Document,
    format: ExportFormat,
    columns: Columns,
    rows: Range<usize>,
    name: &'a str,
    fragment: bool,
    /// SQL: the quoted table name and column list.
    table: String,
    column_list: String,
}

impl<'a> TextExport<'a> {
    fn new(document: &'a Document, format: ExportFormat, columns: Columns, rows: Range<usize>, name: &'a str, fragment: bool) -> Self {
        let table = sql_identifier(if name.trim().is_empty() { "data" } else { name.trim() });
        let column_list = columns.names.iter().map(|name| sql_identifier(name)).collect::<Vec<_>>().join(", ");
        Self { document, format, columns, rows, name, fragment, table, column_list }
    }

    /// Rows `part` of the export (counted from its first row), built on all cores in blocks
    /// and returned in order.
    fn render(&self, part: Range<usize>) -> Vec<String> {
        let blocks: Vec<usize> = part.clone().step_by(EXPORT_BLOCK).collect();
        blocks
            .into_par_iter()
            .map(|block| {
                let mut out = String::new();
                for index in block..(block + EXPORT_BLOCK).min(part.end) {
                    self.row(&mut out, index);
                }
                out
            })
            .collect()
    }

    fn head(&self, out: &mut String) {
        let columns = &self.columns;
        match self.format {
            ExportFormat::Json => out.push_str("[\n"),
            ExportFormat::Markdown => {
                markdown_line(out, columns.names.iter().map(|name| markdown_cell(name)));
                markdown_line(out, columns.numeric.iter().map(|&n| if n { "---:" } else { "---" }.to_owned()));
            }
            ExportFormat::Html => {
                if !self.fragment {
                    out.push_str("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>");
                    out.push_str(&html_escape(self.name));
                    out.push_str(
                        "</title>\n<style>\n\
                         table { border-collapse: collapse; font: 14px -apple-system, \"Segoe UI\", sans-serif; }\n\
                         th, td { border: 1px solid #d0d0d0; padding: 4px 8px; text-align: left; vertical-align: top; }\n\
                         th { background: #f3f3f3; }\n\
                         td.number { text-align: right; }\n\
                         </style>\n</head>\n<body>\n",
                    );
                }
                out.push_str("<table>\n<thead>\n<tr>");
                for name in &columns.names {
                    out.push_str("<th>");
                    out.push_str(&html_escape(name));
                    out.push_str("</th>");
                }
                out.push_str("</tr>\n</thead>\n<tbody>\n");
            }
            ExportFormat::Sql => {
                if !self.fragment {
                    out.push_str(&format!("CREATE TABLE {} (\n", self.table));
                    let count = columns.names.len();
                    for (i, name) in columns.names.iter().enumerate() {
                        let kind = if columns.numeric[i] { "NUMERIC" } else { "TEXT" };
                        let comma = if i + 1 < count { "," } else { "" };
                        out.push_str(&format!("  {} {kind}{comma}\n", sql_identifier(name)));
                    }
                    out.push_str(");\n\n");
                }
            }
            ExportFormat::Xlsx => unreachable!("Excel is not text"),
        }
    }

    /// Row `index` of the export.
    fn row(&self, out: &mut String, index: usize) {
        let columns = &self.columns;
        let values = self.document.export_row(self.rows.start + index, columns);
        match self.format {
            ExportFormat::Json => {
                out.push_str("  {");
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    json_string(out, &columns.names[i]);
                    out.push_str(": ");
                    if !columns.numeric[i] {
                        json_string(out, value);
                    } else if value.trim().is_empty() {
                        out.push_str("null");
                    } else {
                        out.push_str(value.trim());
                    }
                }
                out.push_str(if index + 1 < self.rows.len() { "},\n" } else { "}\n" });
            }
            ExportFormat::Markdown => markdown_line(out, values.iter().map(|value| markdown_cell(value))),
            ExportFormat::Html => {
                out.push_str("<tr>");
                for (i, value) in values.iter().enumerate() {
                    out.push_str(if columns.numeric[i] { "<td class=\"number\">" } else { "<td>" });
                    out.push_str(&html_escape(value).replace("\r\n", "<br>").replace(['\n', '\r'], "<br>"));
                    out.push_str("</td>");
                }
                out.push_str("</tr>\n");
            }
            ExportFormat::Sql => {
                let values: Vec<String> = values
                    .iter()
                    .enumerate()
                    .map(|(i, value)| match (columns.numeric[i], value.trim().is_empty()) {
                        (true, true) => "NULL".to_owned(),
                        (true, false) => value.trim().to_owned(),
                        (false, _) => format!("'{}'", value.replace('\'', "''")),
                    })
                    .collect();
                out.push_str(&format!("INSERT INTO {} ({}) VALUES ({});\n", self.table, self.column_list, values.join(", ")));
            }
            ExportFormat::Xlsx => unreachable!("Excel is not text"),
        }
    }

    fn tail(&self, out: &mut String) {
        match self.format {
            ExportFormat::Json => out.push_str("]\n"),
            ExportFormat::Html => {
                out.push_str("</tbody>\n</table>\n");
                if !self.fragment {
                    out.push_str("</body>\n</html>\n");
                }
            }
            ExportFormat::Markdown | ExportFormat::Sql | ExportFormat::Xlsx => {}
        }
    }
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace("\r\n", "<br>").replace(['\n', '\r'], "<br>")
}

fn markdown_line(out: &mut String, values: impl Iterator<Item = String>) {
    out.push('|');
    for value in values {
        out.push(' ');
        out.push_str(&value);
        out.push_str(" |");
    }
    out.push('\n');
}

/// A number written plainly (`-12`, `3.5`), or with `lenient` also as people write it
/// (`1 250,50`). Leading zeros (`007`) and more than 15 digits stay text: they are codes.
fn number(value: &str, lenient: bool) -> Option<f64> {
    let value = value.trim();
    let digits = value.bytes().filter(u8::is_ascii_digit).count();
    let unsigned = value.trim_start_matches(['-', '+']);
    let leading_zero = unsigned.len() > 1 && unsigned.starts_with('0') && !unsigned[1..].starts_with(['.', ',']);
    if digits == 0 || digits > 15 || leading_zero {
        return None;
    }
    if lenient {
        return parse_number(value).filter(|n| n.is_finite());
    }
    let plain = value.strip_prefix('-').unwrap_or(value);
    let (integer, fraction) = plain.split_once('.').unwrap_or((plain, ""));
    let digits_only = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let valid = !integer.is_empty() && digits_only(integer) && digits_only(fraction) && !(plain.contains('.') && fraction.is_empty());
    valid.then(|| value.parse().ok()).flatten()
}

/// A cell of a workbook sheet as an exported workbook keeps it.
#[derive(Debug, PartialEq)]
enum SheetCell {
    /// A number, with the format that shows it as the sheet did.
    Number(f64, Option<String>),
    Bool(bool),
    Text,
}

/// What a sheet shows, read back: amounts with or without a currency, percentages, dates and
/// times, true and false; anything else, codes with leading zeros included, stays text.
fn sheet_cell(text: &str, locale: &DisplayLocale) -> SheetCell {
    let value = text.trim();
    if value == locale.true_text || value == locale.false_text {
        return SheetCell::Bool(value == locale.true_text);
    }
    if let Some(cell) = date_or_time(value) {
        return cell;
    }
    if let Some(number_part) = value.strip_suffix('%') {
        return match number(number_part, true) {
            Some(number) => SheetCell::Number(number / 100.0, Some(format!("{}%", number_code(number_part, locale, false)))),
            None => SheetCell::Text,
        };
    }
    if let Some(bare) = strip_currency(value) {
        // The sign before the number, or after it.
        let currency = value.strip_suffix(bare).unwrap_or(&value[bare.len()..]);
        let Some(number) = number(bare, true) else { return SheetCell::Text };
        let code = number_code(bare.trim(), locale, true);
        let currency = currency.trim();
        let code = if value.starts_with(currency) {
            let space = if bare.starts_with(char::is_whitespace) { " " } else { "" };
            format!("\"{currency}\"{space}{code}")
        } else {
            let space = if bare.ends_with(char::is_whitespace) { " " } else { "" };
            format!("{code}{space}\"{currency}\"")
        };
        return SheetCell::Number(number, Some(code));
    }
    match number(value, true) {
        Some(number) => {
            let code = number_code(value, locale, false);
            SheetCell::Number(number, (code != "0").then_some(code))
        }
        None => SheetCell::Text,
    }
}

/// The format code of a number as shown: thousands grouped or not, as many decimals as shown;
/// an amount always gets its group.
fn number_code(shown: &str, locale: &DisplayLocale, amount: bool) -> String {
    let grouped = amount || shown.chars().any(|c| matches!(c, ' ' | '\u{a0}' | '\u{202f}') || c == locale.group_separator);
    let decimals = shown.rsplit_once(locale.decimal_separator).map_or(0, |(_, fraction)| fraction.chars().take_while(char::is_ascii_digit).count());
    let mut code = if grouped { "#,##0".to_owned() } else { "0".to_owned() };
    if decimals > 0 {
        code.push('.');
        code.push_str(&"0".repeat(decimals));
    }
    code
}

/// `31.12.2026`, `2026-12-31`, either with a time, or a time alone (`09:30`, `36:00`), as an
/// Excel serial number with its format.
fn date_or_time(value: &str) -> Option<SheetCell> {
    let digits = |text: &str| -> Option<i64> { (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().ok())? };
    let (date, time) = match value.split_once(' ') {
        Some((date, time)) => (date, Some(time)),
        None if value.contains(':') => ("", Some(value)),
        None => (value, None),
    };
    let mut serial = 0.0;
    let mut code = String::new();
    if !date.is_empty() {
        let parts: Vec<&str> = date.split(['.', '-']).collect();
        let [first, second, third] = parts.as_slice() else { return None };
        let (year, month, day, layout) = if date.contains('-') {
            (digits(first)?, digits(second)?, digits(third)?, "yyyy-mm-dd")
        } else {
            (digits(third)?, digits(second)?, digits(first)?, "dd.mm.yyyy")
        };
        if !(1900..=9999).contains(&year) || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        serial = (crate::profile::days_from_civil(year, month, day) + 25_569) as f64;
        code.push_str(layout);
    }
    if let Some(time) = time {
        let parts: Vec<&str> = time.split(':').collect();
        let (hours, minutes, seconds) = match parts.as_slice() {
            [hours, minutes] => (digits(hours)?, digits(minutes)?, 0),
            [hours, minutes, seconds] => (digits(hours)?, digits(minutes)?, digits(seconds)?),
            _ => return None,
        };
        if minutes > 59 || seconds > 59 || (!date.is_empty() && hours > 23) {
            return None;
        }
        serial += (hours * 3600 + minutes * 60 + seconds) as f64 / 86_400.0;
        if !code.is_empty() {
            code.push(' ');
        }
        code.push_str(if hours > 23 { "[h]:mm" } else { "hh:mm" });
        if parts.len() == 3 {
            code.push_str(":ss");
        }
    }
    Some(SheetCell::Number(serial, Some(code)))
}

pub(crate) fn column_letter(index: usize) -> String {
    let mut number = index + 1;
    let mut letters = Vec::new();
    while number > 0 {
        letters.push(b'A' + ((number - 1) % 26) as u8);
        number = (number - 1) / 26;
    }
    letters.reverse();
    String::from_utf8(letters).expect("ASCII")
}

fn json_string(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn sql_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_cells_read_back_as_shown() {
        let locale = DisplayLocale { decimal_separator: ',', group_separator: '\u{a0}', ..DisplayLocale::default() };
        let number = |value: f64, code: &str| SheetCell::Number(value, Some(code.to_owned()));
        assert_eq!(sheet_cell("1\u{a0}120,00 ₽", &locale), number(1120.0, "#,##0.00 \"₽\""));
        assert_eq!(sheet_cell("$5", &locale), number(5.0, "\"$\"#,##0"));
        assert_eq!(sheet_cell("10,0%", &locale), number(0.1, "0.0%"));
        assert_eq!(sheet_cell("01.09.2026", &locale), number(46266.0, "dd.mm.yyyy"));
        assert_eq!(sheet_cell("09:30", &locale), number(0.395_833_333_333_333_3, "hh:mm"));
        assert_eq!(sheet_cell("36:00", &locale), number(1.5, "[h]:mm"));
        assert_eq!(sheet_cell("1\u{a0}234\u{a0}568", &locale), number(1_234_568.0, "#,##0"));
        assert_eq!(sheet_cell("4", &locale), SheetCell::Number(4.0, None));
        assert_eq!(sheet_cell(&locale.true_text.clone(), &locale), SheetCell::Bool(true));
        assert_eq!(sheet_cell("007", &locale), SheetCell::Text);
        assert_eq!(sheet_cell("Итого: 220,00", &locale), SheetCell::Text);
        assert_eq!(sheet_cell("+7 900 001-01-01", &locale), SheetCell::Text);
    }

    #[test]
    fn numbers_and_codes() {
        assert_eq!(number("42", false), Some(42.0));
        assert_eq!(number("-3.5", false), Some(-3.5));
        assert_eq!(number("0.25", false), Some(0.25));
        assert_eq!(number("007", false), None);
        assert_eq!(number("1 250,50", false), None);
        assert_eq!(number("1 250,50", true), Some(1250.5));
        assert_eq!(number("1234567890123456", true), None);
        assert_eq!(number("3.", false), None);
        assert_eq!(column_letter(0), "A");
        assert_eq!(column_letter(27), "AB");
    }
}
