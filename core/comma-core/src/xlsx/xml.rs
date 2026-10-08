//! Reading the XML parts of a workbook.

use std::borrow::Cow;
use std::io::BufRead;

use quick_xml::{Reader, XmlVersion};
use quick_xml::events::{BytesRef, BytesStart, Event};

use crate::error::Error;

/// Text of an XML part. Parts are UTF-8, or UTF-16 with a byte order mark.
pub(crate) fn decode(bytes: Vec<u8>) -> String {
    match bytes.as_slice() {
        [0xFF, 0xFE, ..] => encoding_rs::UTF_16LE.decode_with_bom_removal(&bytes).0.into_owned(),
        [0xFE, 0xFF, ..] => encoding_rs::UTF_16BE.decode_with_bom_removal(&bytes).0.into_owned(),
        [0xEF, 0xBB, 0xBF, ..] => match String::from_utf8(bytes) {
            Ok(mut text) => {
                text.drain(..3);
                text
            }
            Err(error) => String::from_utf8_lossy(&error.into_bytes()[3..]).into_owned(),
        },
        _ => String::from_utf8(bytes).unwrap_or_else(|error| String::from_utf8_lossy(error.as_bytes()).into_owned()),
    }
}

pub(crate) fn reader(text: &str) -> Reader<&[u8]> {
    let mut reader = Reader::from_str(text);
    configure(&mut reader);
    reader
}

fn configure<R>(reader: &mut Reader<R>) {
    let config = reader.config_mut();
    config.trim_text(false);
    config.check_end_names = false;
}

pub(crate) fn error<R>(part: &str, reader: &Reader<R>, error: quick_xml::Error) -> Error {
    Error::Workbook(format!("{part} is damaged at byte {}: {error}", reader.error_position()))
}

/// Events of an XML part, read from text in memory or from a stream.
pub(crate) trait EventSource {
    /// The next event; it lives until the next call.
    fn next_event(&mut self, part: &str) -> Result<Event<'_>, Error>;
    /// Bytes of the part read so far.
    fn position(&self) -> u64;
}

impl EventSource for Reader<&[u8]> {
    fn next_event(&mut self, part: &str) -> Result<Event<'_>, Error> {
        self.read_event().map_err(|e| error(part, self, e))
    }

    fn position(&self) -> u64 {
        self.buffer_position()
    }
}

/// Events of a part read as it streams in.
pub(crate) struct Stream<R> {
    reader: Reader<R>,
    buffer: Vec<u8>,
}

impl<R: BufRead> Stream<R> {
    pub(crate) fn new(input: R) -> Self {
        let mut reader = Reader::from_reader(input);
        configure(&mut reader);
        Self { reader, buffer: Vec::with_capacity(1024) }
    }
}

impl<R: BufRead> EventSource for Stream<R> {
    fn next_event(&mut self, part: &str) -> Result<Event<'_>, Error> {
        self.buffer.clear();
        let reader = &mut self.reader;
        match reader.read_event_into(&mut self.buffer) {
            Ok(event) => Ok(event),
            Err(e) => Err(error(part, reader, e)),
        }
    }

    fn position(&self) -> u64 {
        self.reader.buffer_position()
    }
}

/// An attribute by its name without a namespace prefix: `id` finds `r:id`.
pub(crate) fn attribute<'a>(element: &'a BytesStart<'_>, name: &str) -> Option<Cow<'a, str>> {
    element.attributes().with_checks(false).flatten().find_map(|attribute| {
        if attribute.key.local_name().as_ref() != name {
            return None;
        }
        Some(match attribute.normalized_value(XmlVersion::Implicit1_0) {
            Ok(value) => value,
            Err(_) => attribute.value,
        })
    })
}

/// Whether an attribute says yes: `1` or `true`.
pub(crate) fn flag(element: &BytesStart<'_>, name: &str) -> bool {
    attribute(element, name).is_some_and(|value| value == "1" || value == "true")
}

/// Adds the text of a text, CDATA or entity event to `out`; other events add nothing.
pub(crate) fn push_text(out: &mut String, event: &Event<'_>) {
    match event {
        Event::Text(text) => out.push_str(&text.xml10_content()),
        Event::CData(data) => out.push_str(&data.xml10_content()),
        Event::GeneralRef(reference) => push_reference(out, reference),
        _ => {}
    }
}

fn push_reference(out: &mut String, reference: &BytesRef<'_>) {
    if let Ok(Some(c)) = reference.resolve_char_ref() {
        out.push(c);
        return;
    }
    match &**reference {
        "amp" => out.push('&'),
        "lt" => out.push('<'),
        "gt" => out.push('>'),
        "quot" => out.push('"'),
        "apos" => out.push('\''),
        other => {
            out.push('&');
            out.push_str(other);
            out.push(';');
        }
    }
}

/// Characters XML cannot hold are written as `_x000D_`; `_x005F_` escapes an underscore that
/// would otherwise start one.
pub(crate) fn unescape_characters(text: String) -> String {
    if !text.contains("_x") {
        return text;
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(start) = rest.find("_x") {
        out.push_str(&rest[..start]);
        let candidate = &rest[start..];
        let code = candidate
            .get(2..6)
            .filter(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()) && candidate.as_bytes().get(6) == Some(&b'_'))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .and_then(char::from_u32);
        match code {
            Some(c) => {
                out.push(c);
                rest = &candidate[7..];
            }
            None => {
                out.push_str("_x");
                rest = &candidate[2..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Text of a shared or inline string: `<t>` alone or the runs of formatted text, without
/// phonetic hints. Reads up to the end of the element that holds it.
pub(crate) fn read_string(source: &mut impl EventSource, part: &str, end: &str) -> Result<String, Error> {
    let mut out = String::new();
    let (mut in_text, mut in_phonetic) = (false, false);
    loop {
        let event = source.next_event(part)?;
        match &event {
            Event::Start(element) => match element.local_name().as_ref() {
                "t" => in_text = true,
                "rPh" => in_phonetic = true,
                _ => {}
            },
            Event::End(element) => match element.local_name().as_ref() {
                "t" => in_text = false,
                "rPh" => in_phonetic = false,
                name if name == end => break,
                _ => {}
            },
            Event::Eof => break,
            event if in_text && !in_phonetic => push_text(&mut out, event),
            _ => {}
        }
    }
    Ok(unescape_characters(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes_characters() {
        assert_eq!(unescape_characters("a_x000D_b".to_owned()), "a\rb");
        assert_eq!(unescape_characters("_x005F_x0041_".to_owned()), "_x0041_");
        assert_eq!(unescape_characters("my_xls_file".to_owned()), "my_xls_file");
        assert_eq!(unescape_characters("_x00".to_owned()), "_x00");
    }

    #[test]
    fn reads_rich_text_without_phonetics() {
        let text = "<si><r><rPr><b/></rPr><t>Bold </t></r><r><t xml:space=\"preserve\">&amp; plain &#x41;</t></r><rPh><t>ふりがな</t></rPh></si>";
        let mut reader = reader(text);
        reader.read_event().unwrap();
        assert_eq!(read_string(&mut reader, "test", "si").unwrap(), "Bold & plain A");
    }
}
