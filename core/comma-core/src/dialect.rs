use std::collections::{HashMap, HashSet};

use crate::parser::{self, RecordSpan, Terminator};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub fn terminator(self) -> Terminator {
        match self {
            Self::Lf => Terminator::Lf,
            Self::CrLf => Terminator::CrLf,
            Self::Cr => Terminator::Cr,
        }
    }
}

/// How new or edited values are quoted on save. Untouched fields keep their original quoting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteStyle {
    /// Quote only values that contain the delimiter, a quote or a line break.
    Minimal,
    /// Quote every value: the file already does so.
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dialect {
    pub delimiter: u8,
    pub quote: u8,
    pub line_ending: LineEnding,
    pub quote_style: QuoteStyle,
}

impl Default for Dialect {
    fn default() -> Self {
        Self {
            delimiter: b',',
            quote: b'"',
            line_ending: LineEnding::Lf,
            quote_style: QuoteStyle::Minimal,
        }
    }
}

const CANDIDATES: [u8; 4] = [b',', b';', b'\t', b'|'];
const SAMPLE_BYTES: usize = 256 * 1024;
const SAMPLE_RECORDS: usize = 1000;

pub(crate) fn sniff(buf: &[u8], start: usize) -> Dialect {
    let end = buf.len().min(start + SAMPLE_BYTES);
    let sample = &buf[start..end];
    let truncated = end < buf.len();
    let delimiter = sniff_delimiter(sample, truncated);
    let records = sample_records(sample, delimiter, truncated);
    Dialect {
        delimiter,
        quote: b'"',
        line_ending: sniff_line_ending(sample),
        quote_style: sniff_quote_style(sample, &records, delimiter),
    }
}

/// Non-blank records of the sample; the last one is dropped when the sample cut it short.
fn sample_records(sample: &[u8], delimiter: u8, truncated: bool) -> Vec<RecordSpan> {
    let mut records = parser::scan(sample, 0, delimiter, b'"', Some(SAMPLE_RECORDS)).records;
    if truncated && records.len() > 1 {
        records.pop();
    }
    records.retain(|r| r.end > r.start);
    records
}

/// Picks the candidate that splits most records into the same number of fields (at least two);
/// more fields win a tie, then the order of `CANDIDATES`.
fn sniff_delimiter(sample: &[u8], truncated: bool) -> u8 {
    let mut best = (b',', 0.0f64, 0u32);
    for delimiter in CANDIDATES {
        let records = sample_records(sample, delimiter, truncated);
        if records.is_empty() {
            continue;
        }
        let mut counts: HashMap<u32, usize> = HashMap::new();
        for record in &records {
            *counts.entry(record.fields).or_default() += 1;
        }
        let (&mode, &frequency) = counts
            .iter()
            .max_by_key(|&(&fields, &count)| (count, fields))
            .expect("records is not empty");
        if mode < 2 {
            continue;
        }
        let consistency = frequency as f64 / records.len() as f64;
        if consistency > best.1 + 1e-9 || ((consistency - best.1).abs() <= 1e-9 && mode > best.2) {
            best = (delimiter, consistency, mode);
        }
    }
    best.0
}

fn sniff_line_ending(sample: &[u8]) -> LineEnding {
    let (mut lf, mut crlf, mut cr) = (0usize, 0usize, 0usize);
    let mut i = 0;
    while i < sample.len() {
        match sample[i] {
            b'\n' => lf += 1,
            b'\r' if sample.get(i + 1) == Some(&b'\n') => {
                crlf += 1;
                i += 1;
            }
            b'\r' => cr += 1,
            _ => {}
        }
        i += 1;
    }
    if crlf >= lf && crlf >= cr && crlf > 0 {
        LineEnding::CrLf
    } else if cr > lf {
        LineEnding::Cr
    } else {
        LineEnding::Lf
    }
}

fn sniff_quote_style(sample: &[u8], records: &[RecordSpan], delimiter: u8) -> QuoteStyle {
    let (mut total, mut quoted) = (0usize, 0usize);
    for record in records {
        let bytes = &sample[record.start..record.end];
        for (start, end) in parser::split_fields(bytes, delimiter, b'"') {
            total += 1;
            if bytes[start..end].first() == Some(&b'"') {
                quoted += 1;
            }
        }
    }
    if total >= 2 && quoted as f64 / total as f64 >= 0.9 {
        QuoteStyle::Always
    } else {
        QuoteStyle::Minimal
    }
}

/// A first row of distinct, non-empty, non-numeric values is taken for a header.
pub(crate) fn sniff_header(first_row: &[String]) -> bool {
    if first_row.is_empty() {
        return false;
    }
    let mut seen = HashSet::new();
    first_row.iter().all(|value| {
        let value = value.trim();
        !value.is_empty() && !looks_numeric(value) && seen.insert(value)
    })
}

/// Numbers, dates and times: digits with separators only.
fn looks_numeric(value: &str) -> bool {
    value.chars().any(|c| c.is_ascii_digit())
        && value
            .chars()
            .all(|c| c.is_ascii_digit() || " +-.,:/%eE".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delimiter_of(input: &str) -> char {
        sniff(input.as_bytes(), 0).delimiter as char
    }

    #[test]
    fn detects_delimiters() {
        assert_eq!(delimiter_of("a,b,c\n1,2,3\n"), ',');
        assert_eq!(delimiter_of("name;price\nTea;1,50\nCoffee;2,75\n"), ';');
        assert_eq!(delimiter_of("a\tb\n1\t2\n"), '\t');
        assert_eq!(delimiter_of("a|b|c\n1|2|3\n"), '|');
        assert_eq!(delimiter_of("single column\nvalues\n"), ',');
    }

    #[test]
    fn ignores_delimiters_inside_quotes() {
        assert_eq!(delimiter_of("\"a;b\",c\n\"d;e\",f\n\"g;h\",i\n"), ',');
    }

    #[test]
    fn detects_line_endings_and_quote_style() {
        let dialect = sniff(b"\"a\",\"b\"\r\n\"1\",\"2\"\r\n", 0);
        assert_eq!(dialect.line_ending, LineEnding::CrLf);
        assert_eq!(dialect.quote_style, QuoteStyle::Always);
        assert_eq!(sniff(b"a,b\n1,2\n", 0).quote_style, QuoteStyle::Minimal);
    }

    #[test]
    fn detects_header() {
        let row = |values: &[&str]| values.iter().map(|v| v.to_string()).collect::<Vec<_>>();
        assert!(sniff_header(&row(&["id", "name", "created_at"])));
        assert!(!sniff_header(&row(&["1", "Alice", "2024-01-01"])));
        assert!(!sniff_header(&row(&["name", "", "city"])));
        assert!(!sniff_header(&row(&["a", "a"])));
    }
}
