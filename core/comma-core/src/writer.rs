use std::io::{self, BufWriter, Write};
use std::path::Path;

use memchr::memchr;

use crate::dialect::{LineEnding, QuoteStyle};
use crate::document::{Document, Field, Record};
use crate::encoding::{self, TextEncoding};
use crate::error::{Error, Result};
use crate::parser::Terminator;

/// How a file is written: text encoding, byte order mark, delimiter and line endings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SaveFormat {
    pub encoding: TextEncoding,
    /// Byte order mark; only UTF-8 and UTF-16 have one.
    pub bom: bool,
    pub delimiter: u8,
    pub line_ending: LineEnding,
}

impl SaveFormat {
    pub(crate) fn validate(&self, quote: u8) -> Result<()> {
        let delimiter = self.delimiter;
        if !delimiter.is_ascii() || delimiter == quote || delimiter == b'\r' || delimiter == b'\n' {
            return Err(Error::InvalidDelimiter(delimiter as char));
        }
        Ok(())
    }

    fn has_bom(&self) -> bool {
        self.bom && matches!(self.encoding, TextEncoding::Utf8 | TextEncoding::Utf16Le | TextEncoding::Utf16Be)
    }
}

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

impl Document {
    /// Writes the document to `path` in its save format. Nothing is written if a value cannot be
    /// represented in the file's encoding.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.save_with(path, &self.save_format())
    }

    /// Writes the document to `path` in `format`, leaving the document's own format unchanged.
    pub fn save_with(&self, path: impl AsRef<Path>, format: &SaveFormat) -> Result<()> {
        format.validate(self.dialect.quote)?;
        if *format == self.source_format() {
            self.check_encodable()?;
            let mut out = BufWriter::with_capacity(1 << 20, std::fs::File::create(path)?);
            self.write_unchanged_format(&mut out)?;
            out.flush()?;
        } else {
            std::fs::write(path, self.convert(format)?)?;
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.to_bytes_with(&self.save_format())
    }

    pub fn to_bytes_with(&self, format: &SaveFormat) -> Result<Vec<u8>> {
        format.validate(self.dialect.quote)?;
        if *format != self.source_format() {
            return self.convert(format);
        }
        self.check_encodable()?;
        let mut out = Vec::with_capacity(self.buf.len());
        self.write_unchanged_format(&mut out)?;
        Ok(out)
    }

    // MARK: Original format

    fn write_unchanged_format<W: Write>(&self, out: &mut W) -> Result<()> {
        if self.encoding.is_utf16() {
            let mut utf8 = Vec::with_capacity(self.buf.len());
            self.write_records(&mut utf8)?;
            let text = String::from_utf8(utf8)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            let little_endian = self.encoding == TextEncoding::Utf16Le;
            out.write_all(&encoding::encode_utf16(&text, little_endian, self.has_bom))?;
        } else {
            out.write_all(&self.buf[..self.content_start])?;
            self.write_records(out)?;
        }
        Ok(())
    }

    /// Untouched records and fields are copied byte for byte. Every record but the last gets a
    /// line break (its own, or the file's usual one for new records); the last gets one only if
    /// the original file ended with a line break.
    fn write_records<W: Write>(&self, out: &mut W) -> Result<()> {
        let default_term = self.dialect.line_ending.terminator();
        let last = self.records.len().saturating_sub(1);
        for (index, record) in self.records.iter().enumerate() {
            match record {
                Record::Original { start, end, .. } => out.write_all(&self.buf[*start..*end])?,
                Record::Modified(modified) => self.write_fields(out, &modified.fields)?,
            }
            if index < last || self.trailing_newline {
                let term = match record.term() {
                    Terminator::None => default_term,
                    term => term,
                };
                out.write_all(term.bytes())?;
            }
        }
        Ok(())
    }

    fn write_fields<W: Write>(&self, out: &mut W, fields: &[Field]) -> Result<()> {
        let encoding = self.buffer_encoding();
        let quote = self.dialect.quote;
        for (index, field) in fields.iter().enumerate() {
            if index > 0 {
                out.write_all(&[self.dialect.delimiter])?;
            }
            match field {
                Field::Raw { start, end } => out.write_all(&self.buf[*start..*end])?,
                Field::Text(text) => {
                    let bytes = encoding
                        .encode(text)
                        .expect("checked by check_encodable");
                    if !self.needs_quotes(text, self.dialect.delimiter) {
                        out.write_all(&bytes)?;
                        continue;
                    }
                    out.write_all(&[quote])?;
                    let mut rest: &[u8] = &bytes;
                    while let Some(position) = memchr(quote, rest) {
                        out.write_all(&rest[..=position])?;
                        out.write_all(&[quote])?;
                        rest = &rest[position + 1..];
                    }
                    out.write_all(rest)?;
                    out.write_all(&[quote])?;
                }
            }
        }
        Ok(())
    }

    fn check_encodable(&self) -> Result<()> {
        let encoding = self.buffer_encoding();
        if encoding == TextEncoding::Utf8 {
            return Ok(());
        }
        for (row, record) in self.records.iter().enumerate() {
            let Record::Modified(modified) = record else {
                continue;
            };
            for (column, field) in modified.fields.iter().enumerate() {
                if let Field::Text(text) = field
                    && encoding.encode(text).is_none()
                {
                    return Err(Error::Unmappable {
                        row,
                        column,
                        encoding: self.encoding.name(),
                    });
                }
            }
        }
        Ok(())
    }

    // MARK: Another format

    /// Writes the document in another format, changing as little as possible: with the same
    /// delimiter each record keeps its quoting and only its encoding or line break changes; with
    /// a new delimiter records are rebuilt from their values.
    fn convert(&self, format: &SaveFormat) -> Result<Vec<u8>> {
        let same_delimiter = format.delimiter == self.dialect.delimiter;
        let same_line_endings = format.line_ending == self.dialect.line_ending;
        let last = self.records.len().saturating_sub(1);
        let mut text = String::with_capacity(self.buf.len() + self.buf.len() / 8);
        let mut out = Vec::with_capacity(self.buf.len() + 4);
        if format.has_bom() && format.encoding == TextEncoding::Utf8 {
            out.extend_from_slice(&UTF8_BOM);
        }

        let mut record_text = String::new();
        for (row, record) in self.records.iter().enumerate() {
            record_text.clear();
            if same_delimiter {
                self.record_text(record, &mut record_text);
            } else {
                for (column, value) in self.values(row).iter().enumerate() {
                    if column > 0 {
                        record_text.push(format.delimiter as char);
                    }
                    self.push_quoted(&mut record_text, value, format.delimiter);
                }
            }
            if row < last || self.trailing_newline {
                let term = match record.term() {
                    _ if !same_line_endings => format.line_ending.terminator(),
                    Terminator::None => self.dialect.line_ending.terminator(),
                    term => term,
                };
                record_text.push_str(std::str::from_utf8(term.bytes()).expect("ASCII"));
            }
            match format.encoding {
                TextEncoding::Utf8 => out.extend_from_slice(record_text.as_bytes()),
                TextEncoding::Utf16Le | TextEncoding::Utf16Be => text.push_str(&record_text),
                single_byte => match single_byte.encode(&record_text) {
                    Some(bytes) => out.extend_from_slice(&bytes),
                    None => {
                        let column = self
                            .values(row)
                            .iter()
                            .position(|value| single_byte.encode(value).is_none())
                            .unwrap_or(0);
                        return Err(Error::Unmappable {
                            row,
                            column,
                            encoding: single_byte.name(),
                        });
                    }
                },
            }
        }
        if format.encoding.is_utf16() {
            let little_endian = format.encoding == TextEncoding::Utf16Le;
            out = encoding::encode_utf16(&text, little_endian, format.has_bom());
        }
        Ok(out)
    }

    /// A record as text with the original delimiter and quoting.
    fn record_text(&self, record: &Record, out: &mut String) {
        let encoding = self.buffer_encoding();
        match record {
            Record::Original { start, end, .. } => out.push_str(&encoding.decode(&self.buf[*start..*end])),
            Record::Modified(modified) => {
                for (index, field) in modified.fields.iter().enumerate() {
                    if index > 0 {
                        out.push(self.dialect.delimiter as char);
                    }
                    match field {
                        Field::Raw { start, end } => out.push_str(&encoding.decode(&self.buf[*start..*end])),
                        Field::Text(text) => self.push_quoted(out, text, self.dialect.delimiter),
                    }
                }
            }
        }
    }

    fn push_quoted(&self, out: &mut String, value: &str, delimiter: u8) {
        if !self.needs_quotes(value, delimiter) {
            out.push_str(value);
            return;
        }
        let quote = self.dialect.quote as char;
        out.push(quote);
        for c in value.chars() {
            if c == quote {
                out.push(quote);
            }
            out.push(c);
        }
        out.push(quote);
    }

    fn needs_quotes(&self, text: &str, delimiter: u8) -> bool {
        let quote = self.dialect.quote;
        self.dialect.quote_style == QuoteStyle::Always
            || text
                .bytes()
                .any(|b| b == delimiter || b == quote || b == b'\n' || b == b'\r')
    }
}
