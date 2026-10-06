//! Quote-aware scanning of CSV bytes.
//!
//! Works on ASCII-compatible bytes: delimiter, quote and line breaks are single ASCII bytes and
//! never occur inside multi-byte characters. A quote opens a quoted field only at the start of a
//! field; anywhere else it is a literal character, as in Excel. After a closing quote, the rest of
//! the field up to the next delimiter is kept literally.

use std::borrow::Cow;

use memchr::{memchr, memchr3};

/// Line break that ends a record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Terminator {
    /// The last record of a file without a trailing line break.
    None,
    Lf,
    CrLf,
    Cr,
}

impl Terminator {
    pub fn bytes(self) -> &'static [u8] {
        match self {
            Self::None => b"",
            Self::Lf => b"\n",
            Self::CrLf => b"\r\n",
            Self::Cr => b"\r",
        }
    }
}

/// Byte range of one record, without its line break.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RecordSpan {
    pub start: usize,
    pub end: usize,
    pub term: Terminator,
    pub fields: u32,
}

pub(crate) struct Scan {
    pub records: Vec<RecordSpan>,
    pub max_fields: usize,
    pub unterminated_quote: bool,
}

/// Splits `buf[start..]` into records. Stops after `limit` records when given.
pub(crate) fn scan(
    buf: &[u8],
    start: usize,
    delimiter: u8,
    quote: u8,
    limit: Option<usize>,
) -> Scan {
    let n = buf.len();
    let mut records = Vec::new();
    let mut max_fields = 0usize;
    let mut unterminated_quote = false;
    let mut i = start;

    while i < n && limit.is_none_or(|limit| records.len() < limit) {
        let record_start = i;
        let mut fields: u32 = 1;
        let (end, term) = loop {
            if i < n && buf[i] == quote {
                let (next, closed) = skip_quoted(buf, i + 1, quote);
                unterminated_quote |= !closed;
                i = next;
            }
            match memchr3(delimiter, b'\n', b'\r', &buf[i..]) {
                None => {
                    i = n;
                    break (n, Terminator::None);
                }
                Some(offset) => {
                    let p = i + offset;
                    match buf[p] {
                        b if b == delimiter => {
                            fields += 1;
                            i = p + 1;
                        }
                        b'\n' => {
                            i = p + 1;
                            break (p, Terminator::Lf);
                        }
                        _ if buf.get(p + 1) == Some(&b'\n') => {
                            i = p + 2;
                            break (p, Terminator::CrLf);
                        }
                        _ => {
                            i = p + 1;
                            break (p, Terminator::Cr);
                        }
                    }
                }
            }
        };
        max_fields = max_fields.max(fields as usize);
        records.push(RecordSpan {
            start: record_start,
            end,
            term,
            fields,
        });
    }

    Scan {
        records,
        max_fields,
        unterminated_quote,
    }
}

/// Skips a quoted field body; `i` points just past the opening quote. Returns the position right
/// after the closing quote and `true`, or `buf.len()` and `false` if the quote never closes.
fn skip_quoted(buf: &[u8], mut i: usize, quote: u8) -> (usize, bool) {
    loop {
        match memchr(quote, &buf[i..]) {
            None => return (buf.len(), false),
            Some(offset) => {
                i += offset + 1;
                if buf.get(i) == Some(&quote) {
                    i += 1;
                } else {
                    return (i, true);
                }
            }
        }
    }
}

/// Splits one record (without its line break) into raw field byte ranges, relative to `record`.
pub(crate) fn split_fields(record: &[u8], delimiter: u8, quote: u8) -> Vec<(usize, usize)> {
    let n = record.len();
    let mut fields = Vec::new();
    let mut field_start = 0;
    let mut i = 0;
    loop {
        if i < n && record[i] == quote {
            i = skip_quoted(record, i + 1, quote).0;
        }
        match memchr(delimiter, &record[i..]) {
            None => {
                fields.push((field_start, n));
                return fields;
            }
            Some(offset) => {
                let p = i + offset;
                fields.push((field_start, p));
                field_start = p + 1;
                i = p + 1;
            }
        }
    }
}

/// Raw byte range of field `n` of one record, relative to `record`; `None` if the record is
/// shorter. Unlike `split_fields`, stops at the field and allocates nothing.
pub(crate) fn nth_field(record: &[u8], delimiter: u8, quote: u8, n: usize) -> Option<(usize, usize)> {
    let len = record.len();
    let mut index = 0;
    let mut field_start = 0;
    let mut i = 0;
    loop {
        if i < len && record[i] == quote {
            i = skip_quoted(record, i + 1, quote).0;
        }
        match memchr(delimiter, &record[i..]) {
            None => return (index == n).then_some((field_start, len)),
            Some(offset) => {
                let p = i + offset;
                if index == n {
                    return Some((field_start, p));
                }
                index += 1;
                field_start = p + 1;
                i = p + 1;
            }
        }
    }
}

/// Value of a raw field: surrounding quotes removed and doubled quotes collapsed.
pub(crate) fn unquote(raw: &[u8], quote: u8) -> Cow<'_, [u8]> {
    if raw.first() != Some(&quote) {
        return Cow::Borrowed(raw);
    }
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 1;
    while i < raw.len() {
        let byte = raw[i];
        if byte == quote {
            if raw.get(i + 1) == Some(&quote) {
                out.push(quote);
                i += 2;
                continue;
            }
            out.extend_from_slice(&raw[i + 1..]);
            break;
        }
        out.push(byte);
        i += 1;
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(input: &[u8]) -> Vec<(String, Terminator, u32)> {
        scan(input, 0, b',', b'"', None)
            .records
            .iter()
            .map(|r| {
                let text = String::from_utf8(input[r.start..r.end].to_vec()).unwrap();
                (text, r.term, r.fields)
            })
            .collect()
    }

    #[test]
    fn splits_records_with_all_line_endings() {
        assert_eq!(
            records(b"a,b\nc\r\nd,e,f\rg"),
            vec![
                ("a,b".into(), Terminator::Lf, 2),
                ("c".into(), Terminator::CrLf, 1),
                ("d,e,f".into(), Terminator::Cr, 3),
                ("g".into(), Terminator::None, 1),
            ]
        );
    }

    #[test]
    fn keeps_line_breaks_inside_quotes() {
        assert_eq!(
            records(b"\"multi\nline\",x\n\"a\"\"b\",y\n"),
            vec![
                ("\"multi\nline\",x".into(), Terminator::Lf, 2),
                ("\"a\"\"b\",y".into(), Terminator::Lf, 2),
            ]
        );
    }

    #[test]
    fn quote_inside_unquoted_field_is_literal() {
        assert_eq!(
            records(b"5\" disk,ok\nnext,row\n"),
            vec![
                ("5\" disk,ok".into(), Terminator::Lf, 2),
                ("next,row".into(), Terminator::Lf, 2),
            ]
        );
    }

    #[test]
    fn reports_unterminated_quote() {
        let scan = scan(b"a,\"open\nb,c\n", 0, b',', b'"', None);
        assert!(scan.unterminated_quote);
        assert_eq!(scan.records.len(), 1);

        let closed = super::scan(b"a,\"closed\"", 0, b',', b'"', None);
        assert!(!closed.unterminated_quote);
    }

    #[test]
    fn splits_and_unquotes_fields() {
        let record = b"plain,\"quoted, with comma\",\"say \"\"hi\"\"\",,\"x\"tail";
        let fields: Vec<String> = split_fields(record, b',', b'"')
            .into_iter()
            .map(|(s, e)| String::from_utf8(unquote(&record[s..e], b'"').into_owned()).unwrap())
            .collect();
        assert_eq!(
            fields,
            vec!["plain", "quoted, with comma", "say \"hi\"", "", "xtail"]
        );
        for (n, span) in split_fields(record, b',', b'"').into_iter().enumerate() {
            assert_eq!(nth_field(record, b',', b'"', n), Some(span));
        }
        assert_eq!(nth_field(record, b',', b'"', 5), None);
    }
}
