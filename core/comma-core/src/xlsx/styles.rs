//! Shared strings and the number formats of cell styles.

use std::collections::HashMap;
use std::sync::Arc;

use quick_xml::events::Event;

use super::numfmt::{DisplayLocale, NumberFormat, builtin_code};
use super::xml::{self, attribute};
use crate::error::Result;

/// The strings cells of type `s` point to by index.
pub(crate) fn read_shared_strings(text: &str, part: &str) -> Result<Vec<String>> {
    let mut reader = xml::reader(text);
    let mut strings = Vec::new();
    loop {
        match reader.read_event().map_err(|error| xml::error(part, &reader, error))? {
            Event::Start(element) => match element.local_name().as_ref() {
                "sst" => {
                    let count = attribute(&element, "uniqueCount").and_then(|count| count.parse::<usize>().ok());
                    strings.reserve(count.unwrap_or(0).min(1 << 24));
                }
                "si" => strings.push(xml::read_string(&mut reader, part, "si")?),
                _ => {}
            },
            Event::Empty(element) if element.local_name().as_ref() == "si" => strings.push(String::new()),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(strings)
}

/// The number format of each cell style, by the style's index (a cell's `s`).
#[derive(Clone, Debug, Default)]
pub(crate) struct Styles {
    /// Format id of each style in `cellXfs`.
    style_formats: Vec<u32>,
    /// Format codes the file defines, by id.
    codes: HashMap<u32, String>,
}

impl Styles {
    pub(crate) fn read(text: &str, part: &str) -> Result<Self> {
        let mut reader = xml::reader(text);
        let mut styles = Self::default();
        let mut in_cell_styles = false;
        loop {
            match reader.read_event().map_err(|error| xml::error(part, &reader, error))? {
                Event::Start(element) | Event::Empty(element) => match element.local_name().as_ref() {
                    "numFmt" => {
                        if let (Some(id), Some(code)) = (
                            attribute(&element, "numFmtId").and_then(|id| id.parse().ok()),
                            attribute(&element, "formatCode"),
                        ) {
                            styles.codes.insert(id, code.into_owned());
                        }
                    }
                    "cellXfs" => in_cell_styles = true,
                    "xf" if in_cell_styles => styles
                        .style_formats
                        .push(attribute(&element, "numFmtId").and_then(|id| id.parse().ok()).unwrap_or(0)),
                    _ => {}
                },
                Event::End(element) if element.local_name().as_ref() == "cellXfs" => in_cell_styles = false,
                Event::Eof => break,
                _ => {}
            }
        }
        Ok(styles)
    }

    /// Compiled formats by style index; a style without a known format shows General.
    pub(crate) fn formats(&self, locale: &DisplayLocale) -> Vec<Arc<NumberFormat>> {
        let general = Arc::new(NumberFormat::general());
        let mut by_id: HashMap<u32, Arc<NumberFormat>> = HashMap::new();
        let mut formats: Vec<Arc<NumberFormat>> = self
            .style_formats
            .iter()
            .map(|&id| {
                Arc::clone(by_id.entry(id).or_insert_with(|| {
                    match self.codes.get(&id).cloned().or_else(|| builtin_code(id, locale)) {
                        Some(code) => Arc::new(NumberFormat::parse(&code)),
                        None => Arc::clone(&general),
                    }
                }))
            })
            .collect();
        if formats.is_empty() {
            formats.push(general);
        }
        formats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_formats_of_cell_styles_only() {
        let text = r#"<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
            <numFmts count="1"><numFmt numFmtId="164" formatCode="&quot;₽&quot;#,##0.00"/></numFmts>
            <cellStyleXfs count="1"><xf numFmtId="10"/></cellStyleXfs>
            <cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="164" applyNumberFormat="1"/><xf numFmtId="14"/></cellXfs>
        </styleSheet>"#;
        let styles = Styles::read(text, "styles.xml").unwrap();
        let locale = DisplayLocale::default();
        let formats = styles.formats(&locale);
        assert_eq!(formats.len(), 3);
        assert_eq!(formats[0].format_number(1.5, &locale, false), "1.5");
        assert_eq!(formats[1].format_number(1234.5, &locale, false), "₽1,234.50");
        assert_eq!(formats[2].format_number(45658.0, &locale, false), "1/1/2025");
    }

    #[test]
    fn reads_shared_strings() {
        let text = r#"<sst uniqueCount="3"><si><t>plain</t></si><si/><si><r><t>a</t></r><r><t>b_x000A_c</t></r></si></sst>"#;
        assert_eq!(read_shared_strings(text, "sharedStrings.xml").unwrap(), ["plain", "", "ab\nc"]);
    }
}
