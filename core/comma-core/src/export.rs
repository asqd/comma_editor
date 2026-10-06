//! Export of the visible rows to JSON, Markdown, HTML, SQL and Excel.

use std::collections::HashSet;
use std::ops::Range;

use rust_xlsxwriter::{Format, Workbook};

use crate::document::Document;
use crate::error::{Error, Result};
use crate::range::CellRange;
use crate::view::parse_number;

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
        let text = match format {
            ExportFormat::Json => self.export_json(&columns, rows),
            ExportFormat::Markdown => self.export_markdown(&columns, rows),
            ExportFormat::Html => self.export_html(&columns, rows, name, fragment),
            ExportFormat::Sql => self.export_sql(&columns, rows, name, fragment),
            ExportFormat::Xlsx => return self.export_xlsx(&columns, rows, name),
        };
        Ok(text.into_bytes())
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
        let numeric = indices
            .clone()
            .map(|column| {
                let mut any = false;
                let all = rows.clone().filter_map(|row| self.visible_record(row)).all(|record| {
                    let value = self.value_at(record, column);
                    if value.trim().is_empty() {
                        return true;
                    }
                    any = true;
                    number(&value, lenient).is_some()
                });
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

    fn export_json(&self, columns: &Columns, rows: Range<usize>) -> String {
        let mut out = String::from("[\n");
        let count = rows.len();
        for (index, row) in rows.enumerate() {
            out.push_str("  {");
            for (i, value) in self.export_row(row, columns).iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                json_string(&mut out, &columns.names[i]);
                out.push_str(": ");
                if !columns.numeric[i] {
                    json_string(&mut out, value);
                } else if value.trim().is_empty() {
                    out.push_str("null");
                } else {
                    out.push_str(value.trim());
                }
            }
            out.push_str(if index + 1 < count { "},\n" } else { "}\n" });
        }
        out.push_str("]\n");
        out
    }

    fn export_markdown(&self, columns: &Columns, rows: Range<usize>) -> String {
        let cell = |value: &str| value.replace('|', "\\|").replace("\r\n", "<br>").replace(['\n', '\r'], "<br>");
        let mut out = String::new();
        let line = |out: &mut String, values: &mut dyn Iterator<Item = String>| {
            out.push('|');
            for value in values {
                out.push(' ');
                out.push_str(&value);
                out.push_str(" |");
            }
            out.push('\n');
        };
        line(&mut out, &mut columns.names.iter().map(|name| cell(name)));
        line(&mut out, &mut columns.numeric.iter().map(|&n| if n { "---:" } else { "---" }.to_owned()));
        for row in rows {
            line(&mut out, &mut self.export_row(row, columns).iter().map(|value| cell(value)));
        }
        out
    }

    fn export_html(&self, columns: &Columns, rows: Range<usize>, name: &str, fragment: bool) -> String {
        let mut out = String::new();
        if !fragment {
            out.push_str("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<title>");
            out.push_str(&html_escape(name));
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
        for row in rows {
            out.push_str("<tr>");
            for (i, value) in self.export_row(row, columns).iter().enumerate() {
                out.push_str(if columns.numeric[i] { "<td class=\"number\">" } else { "<td>" });
                out.push_str(&html_escape(value).replace("\r\n", "<br>").replace(['\n', '\r'], "<br>"));
                out.push_str("</td>");
            }
            out.push_str("</tr>\n");
        }
        out.push_str("</tbody>\n</table>\n");
        if !fragment {
            out.push_str("</body>\n</html>\n");
        }
        out
    }

    fn export_sql(&self, columns: &Columns, rows: Range<usize>, name: &str, fragment: bool) -> String {
        let table = sql_identifier(if name.trim().is_empty() { "data" } else { name.trim() });
        let names: Vec<String> = columns.names.iter().map(|name| sql_identifier(name)).collect();
        let mut out = String::new();
        if !fragment {
            out.push_str(&format!("CREATE TABLE {table} (\n"));
            for (i, name) in names.iter().enumerate() {
                let kind = if columns.numeric[i] { "NUMERIC" } else { "TEXT" };
                let comma = if i + 1 < names.len() { "," } else { "" };
                out.push_str(&format!("  {name} {kind}{comma}\n"));
            }
            out.push_str(");\n\n");
        }
        let column_list = names.join(", ");
        for row in rows {
            let values: Vec<String> = self
                .export_row(row, columns)
                .iter()
                .enumerate()
                .map(|(i, value)| match (columns.numeric[i], value.trim().is_empty()) {
                    (true, true) => "NULL".to_owned(),
                    (true, false) => value.trim().to_owned(),
                    (false, _) => format!("'{}'", value.replace('\'', "''")),
                })
                .collect();
            out.push_str(&format!("INSERT INTO {table} ({column_list}) VALUES ({});\n", values.join(", ")));
        }
        out
    }

    fn export_xlsx(&self, columns: &Columns, rows: Range<usize>, name: &str) -> Result<Vec<u8>> {
        if rows.len() + 1 > XLSX_MAX_ROWS {
            return Err(Error::Export(format!(
                "Excel sheets hold at most {} rows; this table has {}",
                XLSX_MAX_ROWS - 1,
                rows.len()
            )));
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
        let bold = Format::new().set_bold();
        for (column, name) in columns.names.iter().enumerate() {
            sheet.write_string_with_format(0, column as u16, name, &bold).map_err(xlsx)?;
        }
        let row_count = rows.len();
        for (index, row) in rows.enumerate() {
            let excel_row = index as u32 + 1;
            for (column, value) in self.export_row(row, columns).iter().enumerate() {
                if value.is_empty() {
                    continue;
                }
                let column_number = column as u16;
                match columns.numeric[column].then(|| number(value, true)).flatten() {
                    Some(number) => sheet.write_number(excel_row, column_number, number).map_err(xlsx)?,
                    None => sheet.write_string(excel_row, column_number, value).map_err(xlsx)?,
                };
            }
        }
        sheet.set_freeze_panes(1, 0).map_err(xlsx)?;
        if row_count <= 20_000 {
            sheet.autofit();
        }
        workbook.save_to_buffer().map_err(xlsx)
    }
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
