//! A1 references: `B3`, `$B$3`, `A:C`, `2:5`, and moving the relative ones in a formula, as
//! Excel does when one formula is shared by a range of cells.

/// Rows and columns a sheet can have.
pub const MAX_ROWS: u32 = 1 << 20;
pub const MAX_COLUMNS: u32 = 1 << 14;

/// Column index from letters: `A` → 0, `Z` → 25, `AA` → 26. `None` past `XFD`.
pub fn column_index(letters: &str) -> Option<u32> {
    if letters.is_empty() || letters.len() > 3 {
        return None;
    }
    let mut index: u32 = 0;
    for byte in letters.bytes() {
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a',
            _ => return None,
        };
        index = index * 26 + u32::from(digit) + 1;
    }
    (index <= MAX_COLUMNS).then(|| index - 1)
}

/// Letters of a column: 0 → `A`, 26 → `AA`.
pub fn column_letters(mut index: u32) -> String {
    let mut letters = Vec::new();
    loop {
        letters.push(b'A' + (index % 26) as u8);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    letters.reverse();
    String::from_utf8(letters).expect("ASCII")
}

/// Zero-based row and column of a cell reference such as `B3` or `$B$3`.
pub fn parse_cell(text: &str) -> Option<(u32, u32)> {
    let cell = CellRef::parse(text)?;
    Some((cell.row, cell.column))
}

/// `A1` for zero-based row and column.
pub fn cell_name(row: u32, column: u32) -> String {
    format!("{}{}", column_letters(column), row + 1)
}

/// The first and last cells of a range such as `A1:C10`, or of a single cell.
pub fn parse_range(text: &str) -> Option<((u32, u32), (u32, u32))> {
    match text.split_once(':') {
        Some((first, last)) => Some((parse_cell(first)?, parse_cell(last)?)),
        None => {
            let cell = parse_cell(text)?;
            Some((cell, cell))
        }
    }
}

struct CellRef {
    column: u32,
    column_fixed: bool,
    row: u32,
    row_fixed: bool,
}

impl CellRef {
    fn parse(text: &str) -> Option<Self> {
        let (column_fixed, rest) = strip_dollar(text);
        let split = rest.find(|c: char| !c.is_ascii_alphabetic())?;
        let column = column_index(&rest[..split])?;
        let (row_fixed, digits) = strip_dollar(&rest[split..]);
        let row = parse_row(digits)?;
        Some(Self { column, column_fixed, row, row_fixed })
    }
}

fn strip_dollar(text: &str) -> (bool, &str) {
    match text.strip_prefix('$') {
        Some(rest) => (true, rest),
        None => (false, text),
    }
}

/// Zero-based row from its 1-based number.
fn parse_row(digits: &str) -> Option<u32> {
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number: u32 = digits.parse().ok()?;
    (1..=MAX_ROWS).contains(&number).then(|| number - 1)
}

fn moved(index: u32, by: i64, limit: u32) -> Option<u32> {
    let index = i64::from(index) + by;
    (0..i64::from(limit)).contains(&index).then_some(index as u32)
}

fn dollar(fixed: bool) -> &'static str {
    if fixed { "$" } else { "" }
}

/// Moves the relative references of a formula by `rows` and `columns`, as for a formula
/// copied that far. A reference moved off the sheet becomes `#REF!`. Text in quotes, names,
/// functions and structured references (`Table1[Price]`) stay as they are.
pub fn shift_formula(formula: &str, rows: i64, columns: i64) -> String {
    if rows == 0 && columns == 0 {
        return formula.to_owned();
    }
    let bytes = formula.as_bytes();
    let mut out = String::with_capacity(formula.len() + 8);
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        match byte {
            b'"' => {
                let end = skip_quoted(bytes, i, b'"');
                out.push_str(&formula[i..end]);
                i = end;
            }
            // A quoted sheet name: copied up to its `!`; the reference after it moves.
            b'\'' => {
                let end = skip_quoted(bytes, i, b'\'');
                out.push_str(&formula[i..end]);
                i = end;
            }
            b'[' => {
                let end = skip_brackets(bytes, i);
                out.push_str(&formula[i..end]);
                i = end;
            }
            b'#' => {
                // Errors such as `#REF!` and `#N/A`.
                let end = i + 1 + formula[i + 1..]
                    .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '/' | '!' | '?' | '_')))
                    .unwrap_or(formula.len() - i - 1);
                out.push_str(&formula[i..end]);
                i = end;
            }
            _ if is_word_byte(byte) => {
                let end = i + formula[i..].find(|c: char| !is_word_char(c)).unwrap_or(formula.len() - i);
                let word = &formula[i..end];
                let next = bytes.get(end).copied();
                // A function or a sheet name. Numbers such as `1E5` are words too, but never
                // parse as references.
                if next == Some(b'(') || next == Some(b'!') {
                    out.push_str(word);
                    i = end;
                    continue;
                }
                // A range of whole rows or columns: `2:5`, `$A:$C`.
                if next == Some(b':') {
                    let second_end = end + 1 + formula[end + 1..].find(|c: char| !is_word_char(c)).unwrap_or(formula.len() - end - 1);
                    let second = &formula[end + 1..second_end];
                    if let Some(range) = shift_line_range(word, second, rows, columns) {
                        out.push_str(&range);
                        i = second_end;
                        continue;
                    }
                }
                match CellRef::parse(word) {
                    Some(cell) if !word.contains('.') => {
                        let row = if cell.row_fixed { Some(cell.row) } else { moved(cell.row, rows, MAX_ROWS) };
                        let column =
                            if cell.column_fixed { Some(cell.column) } else { moved(cell.column, columns, MAX_COLUMNS) };
                        match (row, column) {
                            (Some(row), Some(column)) => {
                                out.push_str(dollar(cell.column_fixed));
                                out.push_str(&column_letters(column));
                                out.push_str(dollar(cell.row_fixed));
                                out.push_str(&(row + 1).to_string());
                            }
                            _ => out.push_str("#REF!"),
                        }
                    }
                    _ => out.push_str(word),
                }
                i = end;
            }
            _ => {
                let end = i + formula[i..].chars().next().map_or(1, char::len_utf8);
                out.push_str(&formula[i..end]);
                i = end;
            }
        }
    }
    out
}

/// `2:5` or `A:C` moved; `None` if the two parts are not both rows or both columns.
fn shift_line_range(first: &str, second: &str, rows: i64, columns: i64) -> Option<String> {
    let parse_rows = |text: &str| {
        let (fixed, digits) = strip_dollar(text);
        Some((parse_row(digits)?, fixed))
    };
    let parse_columns = |text: &str| {
        let (fixed, letters) = strip_dollar(text);
        Some((column_index(letters)?, fixed))
    };
    let (parts, by, limit, name): (_, _, _, fn(u32) -> String) =
        if let (Some(a), Some(b)) = (parse_rows(first), parse_rows(second)) {
            ([a, b], rows, MAX_ROWS, |row| (row + 1).to_string())
        } else if let (Some(a), Some(b)) = (parse_columns(first), parse_columns(second)) {
            ([a, b], columns, MAX_COLUMNS, column_letters)
        } else {
            return None;
        };
    let mut out = String::new();
    for (index, (value, fixed)) in parts.into_iter().enumerate() {
        if index > 0 {
            out.push(':');
        }
        match if fixed { Some(value) } else { moved(value, by, limit) } {
            Some(value) => {
                out.push_str(dollar(fixed));
                out.push_str(&name(value));
            }
            None => return Some("#REF!".to_owned()),
        }
    }
    Some(out)
}

fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'$' | b'_' | b'\\' | b'.') || byte >= 0x80
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '$' | '_' | '\\' | '.')
}

/// End of a quoted part starting at `start`; the quote doubles inside.
fn skip_quoted(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == quote {
            if bytes.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// End of a bracketed part, brackets nesting as in `Table1[[#This Row],[Price]]`.
fn skip_brackets(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0;
    for (i, &byte) in bytes.iter().enumerate().skip(start) {
        match byte {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_letters_round_trip() {
        for (letters, index) in [("A", 0), ("Z", 25), ("AA", 26), ("AZ", 51), ("BA", 52), ("ZZ", 701), ("AAA", 702), ("XFD", 16383)] {
            assert_eq!(column_index(letters), Some(index), "{letters}");
            assert_eq!(column_letters(index), letters);
        }
        assert_eq!(column_index("XFE"), None);
        assert_eq!(column_index("ABCD"), None);
        assert_eq!(parse_cell("$B$3"), Some((2, 1)));
        assert_eq!(parse_cell("B0"), None);
        assert_eq!(parse_cell("B1048577"), None);
        assert_eq!(parse_range("A1:C10"), Some(((0, 0), (9, 2))));
    }

    #[test]
    fn shifts_relative_references_only() {
        assert_eq!(shift_formula("A1*2", 1, 0), "A2*2");
        assert_eq!(shift_formula("SUM(A1:B2)+$C$1+C$1+$C1", 2, 1), "SUM(B3:C4)+$C$1+D$1+$C3");
        assert_eq!(shift_formula("Sheet2!A1+'My Sheet'!B2", 1, 1), "Sheet2!B2+'My Sheet'!C3");
        assert_eq!(shift_formula("SUM(A:A)+SUM(2:3)+SUM($A:B)", 1, 1), "SUM(B:B)+SUM(3:4)+SUM($A:C)");
    }

    #[test]
    fn leaves_text_names_and_functions_alone() {
        assert_eq!(shift_formula("\"A1\"&A1", 1, 0), "\"A1\"&A2");
        assert_eq!(shift_formula("LOG10(A1)+ATAN2(B1,1)", 1, 0), "LOG10(A2)+ATAN2(B2,1)");
        assert_eq!(shift_formula("Table1[Price]*A1", 1, 0), "Table1[Price]*A2");
        assert_eq!(shift_formula("_xlfn.XLOOKUP(A1,B:B,C:C)", 1, 0), "_xlfn.XLOOKUP(A2,B:B,C:C)");
        assert_eq!(shift_formula("1E5+A1+1.5E3", 1, 0), "1E5+A2+1.5E3");
        assert_eq!(shift_formula("SALES2024+TAX_RATE", 1, 0), "SALES2024+TAX_RATE");
        assert_eq!(shift_formula("IF(A1=\"x\",#N/A,A1)", 1, 0), "IF(A2=\"x\",#N/A,A2)");
    }

    #[test]
    fn references_off_the_sheet_become_ref_errors() {
        assert_eq!(shift_formula("A1", -1, 0), "#REF!");
        assert_eq!(shift_formula("XFD1+B2", 0, 1), "#REF!+C2");
    }
}
