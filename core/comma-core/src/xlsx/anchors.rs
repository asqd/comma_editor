//! Comments, notes and drawings of a sheet follow rows and columns that moved when it is
//! saved. They name cells by row and column numbers, not by formulas: a comment by its cell,
//! a note's box and a picture or shape by the cells under its corners.

use super::refs::{cell_name, parse_cell};
use super::save::{AxisMap, attribute_value, find_element, set_attribute};

/// Comments of `commentsN.xml` or `threadedCommentN.xml` on the cells their cells went to;
/// those whose cells were deleted go.
pub(super) fn remap_comments(xml: &str, rows: &AxisMap, columns: &AxisMap) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut at = 0;
    while let Some((start, tag_end, end, _)) = next_element(xml, at, &["comment", "threadedComment"]) {
        out.push_str(&xml[at..start]);
        let tag = &xml[start..tag_end];
        let moved = attribute_value(tag, "ref")
            .and_then(|cell| parse_cell(&cell))
            .map(|(row, column)| rows.index(row).zip(columns.index(column)));
        match moved {
            Some(Some((row, column))) => {
                out.push_str(&set_attribute(tag, "ref", Some(&cell_name(row, column))));
                out.push_str(&xml[tag_end..end]);
            }
            Some(None) => {}
            None => out.push_str(&xml[start..end]),
        }
        at = end;
    }
    out.push_str(&xml[at..]);
    out
}

/// A VML drawing of notes and form controls: a note goes with its cell, its box moved as far;
/// a note whose cell was deleted goes. Other shapes keep to the cells under their corners.
pub(super) fn remap_vml(xml: &str, rows: &AxisMap, columns: &AxisMap, cells: (&AxisMap, &AxisMap)) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut at = 0;
    while let Some((start, _, end, _)) = next_element(xml, at, &["shape"]) {
        out.push_str(&xml[at..start]);
        let shape = &xml[start..end];
        let number = |name: &str| text_of(shape, name).and_then(|text| text.trim().parse::<u32>().ok());
        let note = shape.contains("ObjectType=\"Note\"");
        match (number("Row"), number("Column")) {
            (Some(row), Some(column)) if note => {
                let (cell_rows, cell_columns) = cells;
                let Some((new_row, new_column)) = cell_rows.index(row).zip(cell_columns.index(column)) else {
                    at = end;
                    continue;
                };
                let (down, right) = (i64::from(new_row) - i64::from(row), i64::from(new_column) - i64::from(column));
                let shape = replace_text(shape, "Row", &new_row.to_string());
                let shape = replace_text(&shape, "Column", &new_column.to_string());
                let shape = map_anchor(&shape, |value, is_row| {
                    let moved = i64::from(value) + if is_row { down } else { right };
                    u32::try_from(moved.max(0)).unwrap_or(0)
                });
                out.push_str(&shape);
            }
            _ if text_of(shape, "Anchor").is_some() => {
                out.push_str(&map_anchor(shape, |value, is_row| if is_row { rows.point(value) } else { columns.point(value) }));
            }
            _ => out.push_str(shape),
        }
        at = end;
    }
    out.push_str(&xml[at..]);
    out
}

/// A drawing of pictures, shapes and charts with their corners on the cells those went to;
/// those placed absolutely stay where they are. A corner on a deleted row or column goes to
/// the start of the next one left; one sized only from its first corner keeps its size.
pub(super) fn remap_drawing(xml: &str, rows: &AxisMap, columns: &AxisMap) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut at = 0;
    while let Some((start, tag_end, end, local)) = next_element(xml, at, &["twoCellAnchor", "oneCellAnchor"]) {
        out.push_str(&xml[at..start]);
        let anchor = &xml[start..end];
        let edit_as = attribute_value(&xml[start..tag_end], "editAs");
        if edit_as.as_deref() == Some("absolute") {
            out.push_str(anchor);
            at = end;
            continue;
        }
        let Some(from) = corner(anchor, "from") else {
            out.push_str(anchor);
            at = end;
            continue;
        };
        let new_from = place(from, rows, columns);
        let mut anchor = set_corner(anchor, "from", new_from);
        if local == "twoCellAnchor"
            && let Some(to) = corner(&anchor, "to")
        {
            let new_to = if edit_as.as_deref() == Some("oneCell") {
                let row = new_from.row + to.row.saturating_sub(from.row);
                let column = new_from.column + to.column.saturating_sub(from.column);
                Corner { row, column, ..to }
            } else {
                let mut new_to = place(to, rows, columns);
                if (new_to.row, new_to.row_offset) < (new_from.row, new_from.row_offset) {
                    (new_to.row, new_to.row_offset) = (new_from.row, new_from.row_offset);
                }
                if (new_to.column, new_to.column_offset) < (new_from.column, new_from.column_offset) {
                    (new_to.column, new_to.column_offset) = (new_from.column, new_from.column_offset);
                }
                new_to
            };
            anchor = set_corner(&anchor, "to", new_to);
        }
        out.push_str(&anchor);
        at = end;
    }
    out.push_str(&xml[at..]);
    out
}

#[derive(Clone, Copy)]
struct Corner<'a> {
    row: u32,
    row_offset: &'a str,
    column: u32,
    column_offset: &'a str,
}

/// Where a corner went: on the same place of its cell, or the start of the next one left.
fn place<'a>(corner: Corner<'a>, rows: &AxisMap, columns: &AxisMap) -> Corner<'a> {
    let (row, row_offset) = match rows.index(corner.row) {
        Some(row) => (row, corner.row_offset),
        None => (rows.point(corner.row), "0"),
    };
    let (column, column_offset) = match columns.index(corner.column) {
        Some(column) => (column, corner.column_offset),
        None => (columns.point(corner.column), "0"),
    };
    Corner { row, row_offset, column, column_offset }
}

fn corner<'a>(anchor: &'a str, name: &str) -> Option<Corner<'a>> {
    let (_, inner_start, inner_end, _) = next_element(anchor, 0, &[name])?;
    let inner = &anchor[inner_start..inner_end];
    let number = |name: &str| text_of(inner, name).and_then(|text| text.trim().parse::<u32>().ok());
    Some(Corner {
        row: number("row")?,
        row_offset: text_of(inner, "rowOff").unwrap_or("0"),
        column: number("col")?,
        column_offset: text_of(inner, "colOff").unwrap_or("0"),
    })
}

fn set_corner(anchor: &str, name: &str, corner: Corner) -> String {
    let Some((_, inner_start, inner_end, _)) = next_element(anchor, 0, &[name]) else {
        return anchor.to_owned();
    };
    let inner = &anchor[inner_start..inner_end];
    let inner = replace_text(inner, "row", &corner.row.to_string());
    let inner = replace_text(&inner, "rowOff", corner.row_offset);
    let inner = replace_text(&inner, "col", &corner.column.to_string());
    let inner = replace_text(&inner, "colOff", corner.column_offset);
    format!("{}{inner}{}", &anchor[..inner_start], &anchor[inner_end..])
}

/// `<x:Anchor>` of a VML shape with its rows and columns mapped: left column, offset, top row,
/// offset, right column, offset, bottom row, offset.
fn map_anchor(shape: &str, map: impl Fn(u32, bool) -> u32) -> String {
    let Some(text) = text_of(shape, "Anchor") else {
        return shape.to_owned();
    };
    let values: Vec<&str> = text.split(',').map(str::trim).collect();
    if values.len() != 8 {
        return shape.to_owned();
    }
    let mapped: Vec<String> = values
        .iter()
        .enumerate()
        .map(|(index, value)| match (index % 2, value.parse::<u32>()) {
            (0, Ok(number)) => map(number, index % 4 == 2).to_string(),
            _ => (*value).to_owned(),
        })
        .collect();
    replace_text(shape, "Anchor", &mapped.join(", "))
}

/// The first element with one of these local names from `from`: where it starts, where its
/// start tag ends, where it ends, and its name. The content of an empty one is nothing.
fn next_element<'a>(xml: &str, from: usize, names: &[&'a str]) -> Option<(usize, usize, usize, &'a str)> {
    let mut search = from;
    loop {
        let start = search + names.iter().filter_map(|name| find_element(&xml[search..], name)).min()?;
        let tag_length = xml[start..].find('>')?;
        let tag_end = start + tag_length + 1;
        let name: &str = xml[start + 1..].split(|c: char| c.is_whitespace() || c == '>' || c == '/').next()?;
        let local = name.rsplit(':').next()?;
        let Some(&found) = names.iter().find(|candidate| **candidate == local) else {
            search = start + 1;
            continue;
        };
        if xml[start..tag_end].ends_with("/>") {
            return Some((start, tag_end, tag_end, found));
        }
        let closing = format!("</{name}>");
        let close = tag_end + xml[tag_end..].find(&closing)?;
        // The content's span for corners; the whole element for the others.
        if matches!(found, "from" | "to") {
            return Some((start, tag_end, close, found));
        }
        return Some((start, tag_end, close + closing.len(), found));
    }
}

/// The text of the first element with this local name.
fn text_of<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let (_, tag_end) = text_span(xml, name)?;
    let close = xml[tag_end..].find("</")?;
    Some(&xml[tag_end..tag_end + close])
}

/// Where the first element with this local name starts and where its start tag ends.
fn text_span(xml: &str, name: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    loop {
        let start = from + find_element(&xml[from..], name)?;
        let tag_end = start + xml[start..].find('>')? + 1;
        let tag_name = xml[start + 1..].split(|c: char| c.is_whitespace() || c == '>' || c == '/').next()?;
        if tag_name.rsplit(':').next() == Some(name) && !xml[start..tag_end].ends_with("/>") {
            return Some((start, tag_end));
        }
        from = start + 1;
    }
}

/// XML with the text of the first element with this local name replaced.
fn replace_text(xml: &str, name: &str, text: &str) -> String {
    let Some((_, tag_end)) = text_span(xml, name) else {
        return xml.to_owned();
    };
    let Some(close) = xml[tag_end..].find("</") else {
        return xml.to_owned();
    };
    format!("{}{text}{}", &xml[..tag_end], &xml[tag_end + close..])
}
