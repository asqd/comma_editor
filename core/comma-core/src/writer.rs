use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use memchr::memchr;

use crate::dialect::QuoteStyle;
use crate::document::{Document, Field, Record};
use crate::encoding::{self, TextEncoding};
use crate::error::{Error, Result};
use crate::parser::Terminator;

impl Document {
    /// Writes the document to `path`. Nothing is written if a value cannot be represented in the
    /// file's encoding.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.check_encodable()?;
        let mut out = BufWriter::with_capacity(1 << 20, File::create(path)?);
        self.write_unchecked(&mut out)?;
        out.flush()?;
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.check_encodable()?;
        let mut out = Vec::with_capacity(self.buf.len());
        self.write_unchecked(&mut out)?;
        Ok(out)
    }

    fn write_unchecked<W: Write>(&self, out: &mut W) -> Result<()> {
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
                    if !self.needs_quotes(text) {
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

    fn needs_quotes(&self, text: &str) -> bool {
        let (delimiter, quote) = (self.dialect.delimiter, self.dialect.quote);
        self.dialect.quote_style == QuoteStyle::Always
            || text
                .bytes()
                .any(|b| b == delimiter || b == quote || b == b'\n' || b == b'\r')
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
}
