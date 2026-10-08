//! Saving a workbook by changing only what changed. The file opened is the base of every save:
//! parts of the archive that hold no edited sheet are copied as they are, compressed bytes and
//! all; in an edited sheet, rows and cells that did not change are copied byte for byte, and
//! only changed cells are written anew, with the style they had.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use super::numfmt::{DisplayLocale, NumberFormat};
use super::refs::{cell_name, parse_range};
use quick_xml::events::Event;

use super::refs::MAX_ROWS;
use super::sheet::{self, Context, cell_attributes};
use super::{Source, Workbook, xml};
use crate::document::{Computed, Document, Field};
use crate::error::{Result, WorkbookError};
use crate::profile::date_value;
use crate::view::parse_number;
use crate::writer::write_atomically;

/// A sheet saved anew, and whether a formula of it changed or moved.
struct PatchedSheet {
    xml: String,
    formulas_changed: bool,
}

impl Workbook {
    /// Writes the workbook to `path` with the edits of the given sheets, as documents read from
    /// it with `read_sheet` (by sheet index). Sheets not given, or not changed, are kept as they
    /// are in the file that was opened.
    pub fn save(&self, path: impl AsRef<Path>, sheets: &[(usize, &Document)]) -> Result<()> {
        let bytes = self.to_bytes(sheets)?;
        write_atomically(path.as_ref(), |out| Ok(std::io::Write::write_all(out, &bytes)?))
    }

    /// The bytes `save` writes.
    pub fn to_bytes(&self, sheets: &[(usize, &Document)]) -> Result<Vec<u8>> {
        let Source::Xlsx { package, strings, styles } = &self.source else {
            return Err(WorkbookError::ReadOnly.into());
        };
        let mut replaced: HashMap<String, Vec<u8>> = HashMap::new();
        let mut formulas_changed = false;
        for &(index, document) in sheets {
            let entry = self.sheets.get(index).ok_or(WorkbookError::NoSuchSheet { sheet: index + 1 })?;
            let (Some(part), Some(source)) = (entry.part.as_deref(), document.sheet.as_ref()) else {
                continue;
            };
            if document.sheet_unchanged() {
                continue;
            }
            let text = package
                .read_text(part)?
                .ok_or_else(|| WorkbookError::MissingCells { sheet: entry.name.clone() })?;
            let formats = styles.formats(&source.locale);
            let context = Context { strings, formats: &formats, locale: &source.locale, date1904: self.date1904 };
            let Some(spans) = scan_rows(&text) else {
                return Err(WorkbookError::MissingCells { sheet: entry.name.clone() }.into());
            };
            if let Some(patched) = patch_sheet(&text, &spans, part, document, &context)? {
                formulas_changed |= patched.formulas_changed;
                replaced.insert(part.to_lowercase(), patched.xml.into_bytes());
            }
        }
        if replaced.is_empty() {
            return package.write_with(&HashMap::new(), &HashSet::new());
        }

        // Excel recomputes formulas when it opens the file, as values they use may have changed.
        let workbook_part = self.workbook_part.as_str();
        let workbook = package.read_text(workbook_part)?.unwrap_or_default();
        replaced.insert(workbook_part.to_lowercase(), compute_on_load(&workbook).into_bytes());

        // The calculation chain lists cells with formulas; one that lost its formula would make
        // Excel repair the file, so the chain goes and Excel builds it again.
        let mut removed = HashSet::new();
        if formulas_changed && let Some(chain) = self.calc_chain.as_deref() {
            removed.insert(chain.to_lowercase());
            let relationships = super::package::relationships_part(workbook_part);
            if let Some(text) = package.read_text(&relationships)? {
                replaced.insert(relationships.to_lowercase(), remove_elements(&text, "Relationship", "calcChain").into_bytes());
            }
            if let Some(text) = package.read_text("[Content_Types].xml")? {
                replaced.insert("[content_types].xml".to_owned(), remove_elements(&text, "Override", "calcChain").into_bytes());
            }
        }
        package.write_with(&replaced, &removed)
    }
}

/// The sheet's XML with the document's cells, or `None` if nothing changed.
fn patch_sheet(xml: &str, spans: &Spans, part: &str, document: &Document, context: &Context) -> Result<Option<PatchedSheet>> {
    let Some((data_start, data_end)) = spans.sheet_data else {
        return Ok(None);
    };
    let read_rows = document.sheet.as_ref().map_or(0, |sheet| sheet.rows);
    let prefix = element_prefix(xml, data_start, spans.sheet_data_empty);
    let rows_by_index: HashMap<u32, &RowSpan> = spans.rows.iter().map(|span| (span.row, span)).collect();
    let row_count = document.row_count();

    // Values the file had in the rows that were edited, read from those rows only.
    let edited: Vec<u32> = (0..row_count)
        .filter(|&row| document.row_edited(row))
        .filter_map(|row| document.row_origin(row))
        .map(|origin| origin as u32)
        .collect();
    let originals = original_rows(xml, &edited, &rows_by_index, part, context)?;

    let mut used: HashSet<u32> = HashSet::new();
    let mut body = String::with_capacity(data_end.saturating_sub(data_start) + 1024);
    let mut changed = false;
    let mut formulas_changed = false;
    let (mut last_row, mut last_column) = (0usize, 0usize);
    for row in 0..row_count {
        let origin = document.row_origin(row);
        // A row added at the end may take the place of a row of the file that held formatting
        // but no values.
        let origin_index = origin.or_else(|| {
            let index = row as u32;
            (row >= read_rows && rows_by_index.contains_key(&index) && !used.contains(&index)).then_some(row)
        });
        let span = origin_index.and_then(|origin| rows_by_index.get(&(origin as u32)).copied());
        if let Some(origin) = origin_index {
            used.insert(origin as u32);
        }
        let current = document.row_fields(row);
        if let Some(column) = current.iter().rposition(|field| !value(field).is_empty()) {
            last_row = row;
            last_column = last_column.max(column);
        }
        let number = (row + 1).to_string();
        if !document.row_edited(row) && origin.is_some() {
            // Not edited: the row of the file, moved here if it moved.
            if let Some(span) = span {
                if origin == Some(row) {
                    body.push_str(&xml[span.start..span.end]);
                } else {
                    changed = true;
                    push_moved_row(&mut body, xml, span, row as u32, &prefix);
                }
            }
            continue;
        }
        let before = origin.and_then(|origin| originals.get(origin)).map(Vec::as_slice).unwrap_or(&[]);
        if origin == Some(row) && same_row(before, &current) {
            if let Some(span) = span {
                body.push_str(&xml[span.start..span.end]);
            }
            continue;
        }
        changed = true;
        let has_values = current.iter().any(|field| !value(field).is_empty());
        if span.is_none() && !has_values {
            continue;
        }
        match span {
            Some(span) => {
                let tag = set_attribute(&xml[span.start..span.tag_end], "r", Some(&number));
                body.push_str(&open_tag(&set_attribute(&tag, "spans", None)));
            }
            None => body.push_str(&format!("<{prefix}row r=\"{number}\">")),
        }
        let cells = span.map(|span| cell_spans(xml, span)).unwrap_or_default();
        let mut columns: BTreeSet<u32> = cells.iter().map(|cell| cell.column).collect();
        columns.extend(current.iter().enumerate().filter(|(_, field)| !value(field).is_empty()).map(|(column, _)| column as u32));
        for column in columns {
            let cell_span = cells.iter().find(|cell| cell.column == column);
            let now = current.get(column as usize);
            let reference = cell_name(row as u32, column);
            if same_cell(before.get(column as usize), now) {
                if let Some(cell) = cell_span {
                    if origin == Some(row) {
                        body.push_str(&xml[cell.start..cell.end]);
                    } else {
                        body.push_str(&set_attribute(&xml[cell.start..cell.tag_end], "r", Some(&reference)));
                        body.push_str(&xml[cell.tag_end..cell.end]);
                    }
                }
                continue;
            }
            let had_formula = cell_span.is_some_and(|cell| has_formula(&xml[cell.start..cell.end]));
            let style = cell_span.and_then(|cell| attribute_value(&xml[cell.start..cell.tag_end], "s"));
            let format = style
                .as_deref()
                .and_then(|style| style.parse::<usize>().ok())
                .and_then(|style| context.formats.get(style))
                .unwrap_or(&context.formats[0]);
            if let Some(Field::Formula(formula)) = now {
                // A formula read keeps its element as the file has it; an entered one is new.
                let element = match cell_span {
                    Some(cell) if formula.from_file => formula_element(&xml[cell.start..cell.end], &prefix),
                    _ => None,
                };
                formulas_changed |= !formula.from_file;
                let element = element.unwrap_or_else(|| format!("<{prefix}f>{}</{prefix}f>", escape_text(&formula.formula)));
                body.push_str(&write_formula_cell(&prefix, &reference, style.as_deref(), &element, &formula.computed));
                continue;
            }
            formulas_changed |= had_formula;
            let text = now.map(value).unwrap_or_default();
            body.push_str(&write_cell(&prefix, &reference, style.as_deref(), text, format, context.locale, context.date1904));
        }
        body.push_str(&format!("</{prefix}row>"));
    }
    // Rows of the file past the rows read hold formatting only; they stay. Other rows of the
    // file no row came from were deleted.
    for span in spans.rows.iter().filter(|span| !used.contains(&span.row)) {
        if span.row as usize >= read_rows {
            body.push_str(&xml[span.start..span.end]);
        } else {
            changed = true;
            formulas_changed |= has_formula(&xml[span.start..span.end]);
        }
    }
    if !changed {
        return Ok(None);
    }

    let mut out = String::with_capacity(xml.len() + body.len() / 8);
    match spans.dimension {
        Some((start, end)) if end <= data_start => {
            out.push_str(&xml[..start]);
            out.push_str(&dimension(&xml[start..end], last_row, last_column));
            out.push_str(&xml[end..data_start]);
        }
        _ => out.push_str(&xml[..data_start]),
    }
    if spans.sheet_data_empty {
        out.push_str(&format!("<{prefix}sheetData>{body}</{prefix}sheetData>"));
    } else {
        out.push_str(&body);
    }
    out.push_str(&xml[data_end..]);
    Ok(Some(PatchedSheet { xml: out, formulas_changed }))
}

/// Where `<sheetData>` and its rows are in the XML of a sheet, found by their tags without
/// reading the cells: saving a large sheet reads the cells of the rows that changed only.
struct Spans {
    /// What `<sheetData>` holds; for an empty `<sheetData/>`, the element itself.
    sheet_data: Option<(usize, usize)>,
    sheet_data_empty: bool,
    /// The `<dimension>` element.
    dimension: Option<(usize, usize)>,
    rows: Vec<RowSpan>,
}

struct RowSpan {
    /// Row of the sheet, from 0.
    row: u32,
    start: usize,
    /// End of the start tag; the whole element when it is empty.
    tag_end: usize,
    end: usize,
}

struct CellSpan {
    column: u32,
    start: usize,
    tag_end: usize,
    end: usize,
}

/// The rows of a sheet's XML. `None` without `<sheetData>`.
fn scan_rows(xml: &str) -> Option<Spans> {
    let data = find_element(xml, "sheetData")?;
    let data_tag_end = data + xml[data..].find('>')? + 1;
    let dimension = find_element(&xml[..data], "dimension").and_then(|start| Some((start, start + xml[start..].find('>')? + 1)));
    if xml[data..data_tag_end].ends_with("/>") {
        return Some(Spans { sheet_data: Some((data, data_tag_end)), sheet_data_empty: true, dimension, rows: Vec::new() });
    }
    let prefix = element_prefix(xml, data, true);
    let closing = format!("</{prefix}sheetData>");
    let data_end = data_tag_end + xml[data_tag_end..].find(&closing)?;
    let row_closing = format!("</{prefix}row>");
    let mut rows = Vec::new();
    let mut at = data_tag_end;
    while let Some(offset) = find_element(&xml[at..data_end], "row") {
        let start = at + offset;
        let tag_end = start + xml[start..data_end].find('>')? + 1;
        let end = if xml[start..tag_end].ends_with("/>") {
            tag_end
        } else {
            tag_end + xml[tag_end..data_end].find(&row_closing)? + row_closing.len()
        };
        let row = attribute_value(&xml[start..tag_end], "r")
            .and_then(|r| r.parse::<u32>().ok())
            .filter(|&r| (1..=MAX_ROWS).contains(&r))
            .map_or_else(|| rows.last().map_or(0, |last: &RowSpan| last.row + 1), |r| r - 1);
        rows.push(RowSpan { row, start, tag_end, end });
        at = end;
    }
    Some(Spans { sheet_data: Some((data_tag_end, data_end)), sheet_data_empty: false, dimension, rows })
}

/// The cells of a row of the XML, read when the row changed or moved.
fn cell_spans(xml: &str, row: &RowSpan) -> Vec<CellSpan> {
    let text = &xml[row.start..row.end];
    let mut reader = xml::reader(text);
    let mut cells: Vec<CellSpan> = Vec::new();
    loop {
        let before = reader.buffer_position() as usize;
        let Ok(event) = reader.read_event() else { break };
        let after = reader.buffer_position() as usize;
        match &event {
            Event::Start(element) | Event::Empty(element) if element.local_name().as_ref() == "c" => {
                let column = cell_attributes(element).0.map_or_else(|| cells.last().map_or(0, |last| last.column + 1), |(_, column)| column);
                cells.push(CellSpan { column, start: row.start + before, tag_end: row.start + after, end: row.start + after });
            }
            Event::End(element) if element.local_name().as_ref() == "c" => {
                if let Some(cell) = cells.last_mut() {
                    cell.end = row.start + after;
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    cells
}

/// The values of some rows of the file, by row, read from their XML alone.
fn original_rows(
    xml: &str,
    rows: &[u32],
    spans: &HashMap<u32, &RowSpan>,
    part: &str,
    context: &Context,
) -> Result<Vec<Vec<Field>>> {
    let mut wanted: Vec<&RowSpan> = rows.iter().filter_map(|row| spans.get(row).copied()).collect();
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    wanted.sort_by_key(|span| span.start);
    wanted.dedup_by_key(|span| span.start);
    let mut fragment = String::from("<sheetData>");
    for span in wanted {
        fragment.push_str(&xml[span.start..span.end]);
    }
    fragment.push_str("</sheetData>");
    let content = sheet::read_sheet(&mut xml::reader(&fragment), fragment.len() as u64, part, context, None, false)?;
    Ok(content.map(|content| content.rows).unwrap_or_default())
}

/// A row of the file, unchanged, written at another row: the row and its cells renumbered.
fn push_moved_row(body: &mut String, xml: &str, span: &RowSpan, row: u32, prefix: &str) {
    let tag = set_attribute(&xml[span.start..span.tag_end], "r", Some(&(row + 1).to_string()));
    let tag = set_attribute(&tag, "spans", None);
    if span.tag_end == span.end {
        body.push_str(&tag);
        return;
    }
    body.push_str(&tag);
    for cell in cell_spans(xml, span) {
        body.push_str(&set_attribute(&xml[cell.start..cell.tag_end], "r", Some(&cell_name(row, cell.column))));
        body.push_str(&xml[cell.tag_end..cell.end]);
    }
    body.push_str(&format!("</{prefix}row>"));
}

/// Whether the XML of a cell or row holds a formula.
fn has_formula(xml: &str) -> bool {
    super::has_formula(xml.as_bytes()).unwrap_or(true)
}

fn value(field: &Field) -> &str {
    match field {
        Field::Text(text) => text,
        Field::Formula(cell) => &cell.value,
        Field::Raw { .. } => "",
    }
}

/// Whether a cell is as the file has it. A formula read from the file is when its result
/// shows the same: its formula may be shared with a cell in another row, which reading the row
/// alone does not see, but edits never change a formula read, only replace it. A formula
/// entered here never is.
fn same_cell(was: Option<&Field>, now: Option<&Field>) -> bool {
    if let Some(Field::Formula(cell)) = now
        && !cell.from_file
    {
        return false;
    }
    was.map(value).unwrap_or_default() == now.map(value).unwrap_or_default()
}

fn same_row(was: &[Field], now: &[Field]) -> bool {
    (0..was.len().max(now.len())).all(|column| same_cell(was.get(column), now.get(column)))
}

/// The namespace prefix of `<sheetData>`, with its colon, for elements written next to it.
fn element_prefix(xml: &str, data_start: usize, empty: bool) -> String {
    let tag_start = if empty { data_start } else { xml[..data_start].rfind('<').unwrap_or(0) };
    let name: String = xml[tag_start + 1..].chars().take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/').collect();
    match name.rsplit_once(':') {
        Some((prefix, _)) => format!("{prefix}:"),
        None => String::new(),
    }
}

/// An empty element's tag made into a start tag: `<row r="3"/>` → `<row r="3">`.
fn open_tag(tag: &str) -> String {
    match tag.strip_suffix("/>") {
        Some(start) => format!("{}>", start.trim_end()),
        None => tag.to_owned(),
    }
}

/// The value of an attribute in a start tag, by its name without a prefix.
fn attribute_value(tag: &str, name: &str) -> Option<String> {
    attributes(tag).into_iter().find(|attribute| attribute.name == name).map(|attribute| attribute.value.to_owned())
}

struct Attribute<'a> {
    name: &'a str,
    value: &'a str,
    /// The attribute with the space before it.
    span: (usize, usize),
}

/// Attributes of a start tag, in order.
fn attributes(tag: &str) -> Vec<Attribute<'_>> {
    let bytes = tag.as_bytes();
    let mut found = Vec::new();
    let mut i = tag.find(|c: char| c.is_whitespace()).unwrap_or(tag.len());
    while i < bytes.len() {
        let space = i;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len() && !matches!(bytes[i], b'=' | b'>' | b'/') && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            break;
        }
        let name = &tag[name_start..i];
        i += 1;
        let Some(&quote) = bytes.get(i).filter(|&&b| b == b'"' || b == b'\'') else { break };
        let value_start = i + 1;
        let Some(length) = tag[value_start..].find(quote as char) else { break };
        i = value_start + length + 1;
        found.push(Attribute {
            name: name.rsplit(':').next().unwrap_or(name),
            value: &tag[value_start..value_start + length],
            span: (space, i),
        });
    }
    found
}

/// A start tag with an attribute set to `value`, added at the end if it is not there, or
/// taken out with `None`.
fn set_attribute(tag: &str, name: &str, value: Option<&str>) -> String {
    let existing = attributes(tag).into_iter().find(|attribute| attribute.name == name);
    match (existing, value) {
        (Some(attribute), Some(value)) => {
            format!("{} {name}=\"{value}\"{}", &tag[..attribute.span.0], &tag[attribute.span.1..])
        }
        (Some(attribute), None) => format!("{}{}", &tag[..attribute.span.0], &tag[attribute.span.1..]),
        (None, Some(value)) => {
            let end = if tag.ends_with("/>") { tag.len() - 2 } else { tag.len() - 1 };
            let head = tag[..end].trim_end();
            format!("{head} {name}=\"{value}\"{}", &tag[end..])
        }
        (None, None) => tag.to_owned(),
    }
}

/// `<dimension ref="A1:H10"/>` for the cells in use, keeping where the range starts.
fn dimension(tag: &str, last_row: usize, last_column: usize) -> String {
    let start = attribute_value(tag, "ref").and_then(|range| parse_range(&range)).map_or((0, 0), |(start, _)| start);
    let end = (last_row as u32, last_column as u32);
    let range = if end.0 <= start.0 && end.1 <= start.1 {
        cell_name(start.0, start.1)
    } else {
        format!("{}:{}", cell_name(start.0, start.1), cell_name(end.0.max(start.0), end.1.max(start.1)))
    };
    set_attribute(tag, "ref", Some(&range))
}

/// What a value typed into a cell is stored as.
#[derive(Debug, PartialEq)]
pub(crate) enum Typed<'a> {
    Number(f64),
    Bool(bool),
    Text(&'a str),
}

/// A value as a cell of the given format holds it: a number when the text reads as one the
/// way the format shows numbers (or as a plain number), a date in a date cell, true or false,
/// otherwise text as it is.
pub(crate) fn typed<'a>(text: &'a str, format: &NumberFormat, locale: &DisplayLocale, date1904: bool) -> Typed<'a> {
    let trimmed = text.trim();
    for (word, truth) in [(locale.true_text.as_str(), true), ("TRUE", true), (locale.false_text.as_str(), false), ("FALSE", false)] {
        if trimmed.eq_ignore_ascii_case(word) || trimmed.to_lowercase() == word.to_lowercase() {
            return Typed::Bool(truth);
        }
    }
    let shows = |number: f64| format.format_number(number, locale, date1904) == text;
    if format.is_date()
        && let Some(days) = date_value(trimmed)
    {
        return Typed::Number(days + if date1904 { 24_107.0 } else { 25_569.0 });
    }
    // As the format shows it: "1 250,50 ₽", "12,5%", "(300)".
    let digits: String = trimmed
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == locale.decimal_separator || matches!(c, '-' | '.' | 'E' | 'e' | '+'))
        .collect();
    let candidates = [
        number_in(&digits, locale),
        number_in(&digits, locale).map(|number| -number),
        number_in(&digits, locale).map(|number| number / 100.0),
        number_in(&digits, locale).map(|number| -number / 100.0),
    ];
    if let Some(number) = candidates.into_iter().flatten().find(|&number| shows(number)) {
        return Typed::Number(number);
    }
    if let Some(number) = plain_number(trimmed, locale) {
        return Typed::Number(number);
    }
    Typed::Text(text)
}

fn number_in(digits: &str, locale: &DisplayLocale) -> Option<f64> {
    let normalized = digits.replace(locale.decimal_separator, ".");
    normalized.parse().ok()
}

/// A number written plainly: `42`, `-3.5`, `1 250,5` with the locale's separators, `1E5`, `15%`.
/// Not codes such as `007`, which stay text.
fn plain_number(text: &str, locale: &DisplayLocale) -> Option<f64> {
    let (body, percent) = match text.strip_suffix('%') {
        Some(body) => (body.trim_end(), true),
        None => (text, false),
    };
    let unsigned = body.strip_prefix(['-', '+']).unwrap_or(body);
    let integer: String = unsigned
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == locale.group_separator || matches!(c, ' ' | '\u{a0}' | '\u{202f}'))
        .filter(char::is_ascii_digit)
        .collect();
    if integer.is_empty() || (integer.len() > 1 && integer.starts_with('0')) || integer.len() > 15 {
        return None;
    }
    if !body.chars().all(|c| c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | ',' | 'e' | 'E' | ' ' | '\u{a0}' | '\u{202f}') || c == locale.group_separator) {
        return None;
    }
    let number = parse_number(body)?;
    Some(if percent { number / 100.0 } else { number })
}

/// A cell written anew: the reference, the style it had, and the value typed.
fn write_cell(
    prefix: &str,
    reference: &str,
    style: Option<&str>,
    text: &str,
    format: &NumberFormat,
    locale: &DisplayLocale,
    date1904: bool,
) -> String {
    let style = style.map(|style| format!(" s=\"{style}\"")).unwrap_or_default();
    if text.is_empty() {
        return if style.is_empty() { String::new() } else { format!("<{prefix}c r=\"{reference}\"{style}/>") };
    }
    match typed(text, format, locale, date1904) {
        Typed::Number(number) => format!("<{prefix}c r=\"{reference}\"{style}><{prefix}v>{}</{prefix}v></{prefix}c>", xml_number(number)),
        Typed::Bool(truth) => format!("<{prefix}c r=\"{reference}\"{style} t=\"b\"><{prefix}v>{}</{prefix}v></{prefix}c>", u8::from(truth)),
        Typed::Text(text) => {
            let space = if text.starts_with(char::is_whitespace) || text.ends_with(char::is_whitespace) || text.contains('\n') {
                " xml:space=\"preserve\""
            } else {
                ""
            };
            format!(
                "<{prefix}c r=\"{reference}\"{style} t=\"inlineStr\"><{prefix}is><{prefix}t{space}>{}</{prefix}t></{prefix}is></{prefix}c>",
                escape_text(text)
            )
        }
    }
}

/// A cell with a formula and its result: `t` says what kind of result it is, and a result not
/// known is left out for Excel to compute.
fn write_formula_cell(prefix: &str, reference: &str, style: Option<&str>, element: &str, computed: &Computed) -> String {
    let style = style.map(|style| format!(" s=\"{style}\"")).unwrap_or_default();
    let (kind, value) = match computed {
        Computed::Number(number) => ("", Some(xml_number(*number))),
        Computed::Text(text) => (" t=\"str\"", Some(escape_text(text))),
        Computed::Bool(truth) => (" t=\"b\"", Some(u8::from(*truth).to_string())),
        Computed::Error(code) => (" t=\"e\"", Some(escape_text(code))),
        Computed::Unknown => ("", None),
    };
    let value = value.map(|value| format!("<{prefix}v>{value}</{prefix}v>")).unwrap_or_default();
    format!("<{prefix}c r=\"{reference}\"{style}{kind}>{element}{value}</{prefix}c>")
}

/// The `<f>` element of a cell's XML, as it is.
fn formula_element(cell: &str, prefix: &str) -> Option<String> {
    let start = find_element(cell, "f")?;
    let tag_end = start + cell[start..].find('>')? + 1;
    if cell[start..tag_end].ends_with("/>") {
        return Some(cell[start..tag_end].to_owned());
    }
    let closing = format!("</{prefix}f>");
    let end = tag_end + cell[tag_end..].find(&closing)? + closing.len();
    Some(cell[start..end].to_owned())
}

/// A number as XML Schema writes a double: the shortest text that reads back the same.
fn xml_number(number: f64) -> String {
    let magnitude = number.abs();
    if number.fract() == 0.0 && magnitude < 1e15 {
        format!("{}", number as i64)
    } else if magnitude != 0.0 && !(1e-7..1e16).contains(&magnitude) {
        format!("{number:E}")
    } else {
        format!("{number}")
    }
}

/// Text for an element: markup characters escaped, and characters XML cannot hold written as
/// `_xHHHH_`, as Excel does; an underscore that would start such a sequence is escaped too.
fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (index, c) in text.char_indices() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '_' if looks_escaped(&text[index..]) => out.push_str("_x005F_"),
            '\t' | '\n' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => out.push_str(&format!("_x{:04X}_", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn looks_escaped(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 7 && bytes[1] == b'x' && bytes[2..6].iter().all(u8::is_ascii_hexdigit) && bytes[6] == b'_'
}

/// `workbook.xml` with `fullCalcOnLoad` set, adding `<calcPr>` where the schema wants it.
fn compute_on_load(workbook: &str) -> String {
    if let Some(start) = find_element(workbook, "calcPr") {
        let end = start + workbook[start..].find('>').map_or(workbook.len() - start, |end| end + 1);
        let tag = set_attribute(&workbook[start..end], "fullCalcOnLoad", Some("1"));
        return format!("{}{tag}{}", &workbook[..start], &workbook[end..]);
    }
    let prefix = find_element(workbook, "workbook")
        .map(|start| workbook[start + 1..].chars().take_while(|c| !c.is_whitespace() && *c != '>').collect::<String>())
        .and_then(|name| name.rsplit_once(':').map(|(prefix, _)| format!("{prefix}:")))
        .unwrap_or_default();
    // After the elements that come before it, the last of which is there.
    let after = ["definedNames", "externalReferences", "functionGroups", "sheets"].iter().find_map(|name| {
        let closing = format!("</{prefix}{name}>");
        workbook.find(&closing).map(|at| at + closing.len()).or_else(|| {
            let empty = find_element(workbook, name)?;
            let end = empty + workbook[empty..].find("/>")? + 2;
            Some(end)
        })
    });
    match after {
        Some(at) => format!("{}<{prefix}calcPr fullCalcOnLoad=\"1\"/>{}", &workbook[..at], &workbook[at..]),
        None => workbook.to_owned(),
    }
}

/// Where an element with this local name starts, `<name` or `<prefix:name`.
fn find_element(xml: &str, name: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(offset) = xml[from..].find('<') {
        let start = from + offset;
        let tag: &str = &xml[start + 1..];
        let end = tag.find(|c: char| c.is_whitespace() || c == '>' || c == '/').unwrap_or(tag.len());
        if tag[..end].rsplit(':').next() == Some(name) {
            return Some(start);
        }
        from = start + 1;
    }
    None
}

/// XML without the empty elements with this local name whose text mentions `needle`.
fn remove_elements(xml: &str, name: &str, needle: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = find_element(rest, name) {
        let Some(length) = rest[start..].find("/>") else { break };
        let end = start + length + 2;
        out.push_str(&rest[..start]);
        if !rest[start..end].contains(needle) {
            out.push_str(&rest[start..end]);
        }
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

impl Document {
    /// The fields of a row as they are stored.
    pub(crate) fn row_fields(&self, row: usize) -> Vec<Field> {
        self.records.get(row).map(|record| self.fields(record)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_attributes() {
        assert_eq!(set_attribute(r#"<c r="B5" s="3" t="s">"#, "r", Some("B7")), r#"<c r="B7" s="3" t="s">"#);
        assert_eq!(set_attribute(r#"<row r="5" spans="1:3" ht="20">"#, "spans", None), r#"<row r="5" ht="20">"#);
        assert_eq!(set_attribute(r#"<c s="1"/>"#, "r", Some("A1")), r#"<c s="1" r="A1"/>"#);
        assert_eq!(attribute_value(r#"<x:c r='C2' x:s="7">"#, "s").as_deref(), Some("7"));
        assert_eq!(open_tag(r#"<row r="3" ht="20" />"#), r#"<row r="3" ht="20">"#);
    }

    #[test]
    fn types_values_as_their_cells_show_them() {
        let locale = DisplayLocale { decimal_separator: ',', group_separator: '\u{a0}', ..DisplayLocale::default() };
        let money = NumberFormat::parse("#,##0.00\\ \"₽\"");
        let general = NumberFormat::general();
        let percent = NumberFormat::parse("0%");
        let date = NumberFormat::parse("dd.mm.yyyy");
        assert_eq!(typed("1\u{a0}250,50 ₽", &money, &locale, false), Typed::Number(1250.5));
        assert_eq!(typed("1250,5", &money, &locale, false), Typed::Number(1250.5));
        assert_eq!(typed("42", &general, &locale, false), Typed::Number(42.0));
        assert_eq!(typed("-3,5", &general, &locale, false), Typed::Number(-3.5));
        assert_eq!(typed("007", &general, &locale, false), Typed::Text("007"));
        assert_eq!(typed("12 шт", &general, &locale, false), Typed::Text("12 шт"));
        assert_eq!(typed("25%", &percent, &locale, false), Typed::Number(0.25));
        assert_eq!(typed("15%", &general, &locale, false), Typed::Number(0.15));
        assert_eq!(typed("05.01.2025", &date, &locale, false), Typed::Number(45662.0));
        assert_eq!(typed("2025-01-05", &date, &locale, false), Typed::Number(45662.0));
        assert_eq!(typed("TRUE", &general, &locale, false), Typed::Bool(true));
        assert_eq!(typed("1234567890123456789", &general, &locale, false), Typed::Text("1234567890123456789"));
    }

    #[test]
    fn escapes_text() {
        assert_eq!(escape_text("a < b & c"), "a &lt; b &amp; c");
        assert_eq!(escape_text("line\r\nnext"), "line_x000D_\nnext");
        assert_eq!(escape_text("_x0041_"), "_x005F_x0041_");
        assert_eq!(xml_number(1250.5), "1250.5");
        assert_eq!(xml_number(3.0), "3");
        assert_eq!(xml_number(1e-9), "1E-9");
    }

    #[test]
    fn asks_excel_to_compute_on_load() {
        assert_eq!(
            compute_on_load(r#"<workbook><sheets><sheet name="A"/></sheets><calcPr calcId="191029"/></workbook>"#),
            r#"<workbook><sheets><sheet name="A"/></sheets><calcPr calcId="191029" fullCalcOnLoad="1"/></workbook>"#
        );
        assert_eq!(
            compute_on_load(r#"<workbook><sheets><sheet name="A"/></sheets><definedNames><definedName name="x">A!$A$1</definedName></definedNames></workbook>"#),
            r#"<workbook><sheets><sheet name="A"/></sheets><definedNames><definedName name="x">A!$A$1</definedName></definedNames><calcPr fullCalcOnLoad="1"/></workbook>"#
        );
        assert_eq!(
            remove_elements(r#"<Types><Override PartName="/xl/calcChain.xml" ContentType="x"/><Override PartName="/a.xml"/></Types>"#, "Override", "calcChain"),
            r#"<Types><Override PartName="/a.xml"/></Types>"#
        );
    }
}
