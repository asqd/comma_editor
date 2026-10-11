//! Excel formulas: reading them into expressions, and turning what a person types (Russian
//! function names, `;` between arguments, decimal commas) into the text Excel writes in files.

use crate::formula::{FormulaError, FormulaErrorKind};
use crate::xlsx::refs::{MAX_COLUMNS, MAX_ROWS, column_index};

/// Rows `top..=bottom` and columns `left..=right` of a sheet, from 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Area {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
}

impl Area {
    pub(crate) fn cell(row: u32, column: u32) -> Self {
        Self { top: row, left: column, bottom: row, right: column }
    }

    pub(crate) fn is_cell(&self) -> bool {
        self.top == self.bottom && self.left == self.right
    }
}

/// A reference to cells, on the formula's own sheet or on a named one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Reference {
    pub sheet: Option<String>,
    pub area: Area,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Power,
    Concat,
    Equal,
    NotEqual,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnaryOp {
    Negate,
    Plus,
    Percent,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expr {
    Number(f64),
    Text(String),
    Bool(bool),
    /// An error value written in the formula, such as `#N/A`.
    Error(String),
    Reference(Reference),
    Unary(UnaryOp, Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    /// A function by its English name in upper case, without `_xlfn.` and the like.
    Call(String, Vec<Expr>),
    /// An argument left out: `IF(A1,,2)`.
    Missing,
    /// A defined name, which is not computed.
    Name(String),
    /// An array constant `{1,2}`, which is not computed.
    Array,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Text(String),
    Error(String),
    Word(String),
    /// A sheet name in quotes: `'My Sheet'`.
    Quoted(String),
    Bang,
    Colon,
    Op(&'static str),
    Open,
    Close,
    Separator,
    OpenBrace,
    CloseBrace,
}

fn syntax(kind: FormulaErrorKind, position: usize, detail: impl Into<String>) -> FormulaError {
    FormulaError { kind, position, detail: detail.into() }
}

const ERRORS: [&str; 8] = ["#NULL!", "#DIV/0!", "#VALUE!", "#REF!", "#NAME?", "#NUM!", "#N/A", "#GETTING_DATA"];

/// Tokens with their character positions. `separator` is the character between arguments;
/// `decimal` the one in numbers.
fn tokenize(text: &str, separator: char, decimal: char) -> Result<Vec<(Token, usize)>, FormulaError> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        let token = match c {
            ' ' | '\t' | '\n' | '\r' | '\u{a0}' => {
                i += 1;
                continue;
            }
            '"' | '\'' => {
                let mut value = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(syntax(FormulaErrorKind::UnclosedText, start, "")),
                        Some(&next) if next == c && chars.get(i + 1) == Some(&c) => {
                            value.push(c);
                            i += 2;
                        }
                        Some(&next) if next == c => {
                            i += 1;
                            break;
                        }
                        Some(&next) => {
                            value.push(next);
                            i += 1;
                        }
                    }
                }
                tokens.push((if c == '"' { Token::Text(value) } else { Token::Quoted(value) }, start));
                continue;
            }
            '#' => {
                let rest: String = chars[i..].iter().collect::<String>().to_uppercase();
                match ERRORS.iter().find(|error| rest.starts_with(*error)) {
                    Some(error) => {
                        i += error.chars().count();
                        tokens.push((Token::Error((*error).to_owned()), start));
                        continue;
                    }
                    None => return Err(syntax(FormulaErrorKind::UnexpectedText, start, "#")),
                }
            }
            '(' => Token::Open,
            ')' => Token::Close,
            '{' => Token::OpenBrace,
            '}' => Token::CloseBrace,
            '!' => Token::Bang,
            ':' => Token::Colon,
            _ if c == separator => Token::Separator,
            '+' => Token::Op("+"),
            '-' => Token::Op("-"),
            '*' => Token::Op("*"),
            '/' => Token::Op("/"),
            '^' => Token::Op("^"),
            '&' => Token::Op("&"),
            '%' => Token::Op("%"),
            '=' => Token::Op("="),
            '<' | '>' => {
                let pair: String = chars[i..chars.len().min(i + 2)].iter().collect();
                let op = match pair.as_str() {
                    "<=" => "<=",
                    ">=" => ">=",
                    "<>" => "<>",
                    _ if c == '<' => "<",
                    _ => ">",
                };
                i += op.len();
                tokens.push((Token::Op(op), start));
                continue;
            }
            _ if c.is_ascii_digit() || (c == decimal && chars.get(i + 1).is_some_and(char::is_ascii_digit)) => {
                let mut number = String::new();
                while let Some(&next) = chars.get(i) {
                    if next.is_ascii_digit() {
                        number.push(next);
                    } else if next == decimal && !number.contains('.') && !number.contains('e') {
                        number.push('.');
                    } else if (next == 'e' || next == 'E')
                        && !number.contains('e')
                        && chars.get(i + 1).is_some_and(|after| after.is_ascii_digit() || *after == '+' || *after == '-')
                    {
                        number.push('e');
                        if let Some(&sign) = chars.get(i + 1).filter(|after| **after == '+' || **after == '-') {
                            number.push(sign);
                            i += 1;
                        }
                    } else {
                        break;
                    }
                    i += 1;
                }
                // `1A` and the like are words, such as the second half of `A1:1A`; `2:5` is
                // a range of rows, which the parser reads from two numbers.
                if chars.get(i).is_some_and(|next| next.is_alphabetic() || *next == '_') {
                    let mut word = number.clone();
                    while let Some(&next) = chars.get(i).filter(|next| is_word_char(**next)) {
                        word.push(next);
                        i += 1;
                    }
                    tokens.push((Token::Word(word), start));
                    continue;
                }
                let value: f64 = number.parse().map_err(|_| syntax(FormulaErrorKind::UnexpectedText, start, number.clone()))?;
                tokens.push((Token::Number(value), start));
                continue;
            }
            _ if is_word_char(c) => {
                let mut word = String::new();
                while let Some(&next) = chars.get(i).filter(|next| is_word_char(**next)) {
                    word.push(next);
                    i += 1;
                }
                tokens.push((Token::Word(word), start));
                continue;
            }
            _ => return Err(syntax(FormulaErrorKind::UnexpectedText, start, c.to_string())),
        };
        tokens.push((token, start));
        i += 1;
    }
    Ok(tokens)
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '$' | '\\')
}

struct Parser {
    tokens: Vec<(Token, usize)>,
    position: usize,
    length: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position).map(|(token, _)| token)
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.position + offset).map(|(token, _)| token)
    }

    fn at(&self) -> usize {
        self.tokens.get(self.position).map_or(self.length, |(_, at)| *at)
    }

    fn next(&mut self) -> Result<Token, FormulaError> {
        let token = self.peek().cloned().ok_or_else(|| syntax(FormulaErrorKind::UnexpectedEnd, self.length, ""))?;
        self.position += 1;
        Ok(token)
    }

    fn unexpected(&self) -> FormulaError {
        match self.tokens.get(self.position) {
            Some((token, at)) => syntax(FormulaErrorKind::UnexpectedText, *at, describe(token)),
            None => syntax(FormulaErrorKind::UnexpectedEnd, self.length, ""),
        }
    }

    fn expression(&mut self, min: u8) -> Result<Expr, FormulaError> {
        let mut left = self.prefix()?;
        while let Some(Token::Op(op)) = self.peek() {
            let op = *op;
            if op == "%" {
                if 6 < min {
                    break;
                }
                self.position += 1;
                left = Expr::Unary(UnaryOp::Percent, Box::new(left));
                continue;
            }
            let (operator, precedence) = binary(op);
            if precedence < min {
                break;
            }
            self.position += 1;
            // Excel reads every binary operator from the left, `^` too.
            let right = self.expression(precedence + 1)?;
            left = Expr::Binary(operator, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn prefix(&mut self) -> Result<Expr, FormulaError> {
        match self.peek() {
            Some(Token::Op("-")) => {
                self.position += 1;
                Ok(Expr::Unary(UnaryOp::Negate, Box::new(self.expression(7)?)))
            }
            Some(Token::Op("+")) => {
                self.position += 1;
                Ok(Expr::Unary(UnaryOp::Plus, Box::new(self.expression(7)?)))
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Result<Expr, FormulaError> {
        let start = self.at();
        match self.next()? {
            Token::Number(number) => {
                // `2:5`: whole rows.
                if self.peek() == Some(&Token::Colon)
                    && let Some(Token::Number(last)) = self.peek_at(1).cloned()
                    && let Some(area) = row_range(number, last)
                {
                    self.position += 2;
                    return Ok(Expr::Reference(Reference { sheet: None, area }));
                }
                Ok(Expr::Number(number))
            }
            Token::Text(text) => Ok(Expr::Text(text)),
            Token::Error(error) => Ok(Expr::Error(error)),
            Token::Open => {
                let inner = self.expression(0)?;
                match self.next() {
                    Ok(Token::Close) => Ok(inner),
                    _ => Err(syntax(FormulaErrorKind::UnexpectedEnd, start, ")")),
                }
            }
            Token::OpenBrace => {
                while !matches!(self.peek(), Some(Token::CloseBrace) | None) {
                    self.position += 1;
                }
                self.next()?;
                Ok(Expr::Array)
            }
            Token::Quoted(sheet) => {
                if self.next()? != Token::Bang {
                    return Err(syntax(FormulaErrorKind::UnexpectedText, start, format!("'{sheet}'")));
                }
                self.reference(Some(sheet), start)
            }
            Token::Word(word) => {
                if self.peek() == Some(&Token::Open) {
                    return self.call(word, start);
                }
                if self.peek() == Some(&Token::Bang) {
                    self.position += 1;
                    return self.reference(Some(word), start);
                }
                match word.to_uppercase().as_str() {
                    "TRUE" | "ИСТИНА" => return Ok(Expr::Bool(true)),
                    "FALSE" | "ЛОЖЬ" => return Ok(Expr::Bool(false)),
                    _ => {}
                }
                self.position -= 1;
                self.reference(None, start)
            }
            _ => {
                self.position -= 1;
                Err(self.unexpected())
            }
        }
    }

    /// A cell, a range of cells, rows or columns, or a defined name.
    fn reference(&mut self, sheet: Option<String>, start: usize) -> Result<Expr, FormulaError> {
        let first = match self.next()? {
            Token::Word(word) => word,
            Token::Number(number) if sheet.is_some() => {
                // `Sheet1!2:5`
                if self.peek() == Some(&Token::Colon)
                    && let Some(Token::Number(last)) = self.peek_at(1).cloned()
                    && let Some(area) = row_range(number, last)
                {
                    self.position += 2;
                    return Ok(Expr::Reference(Reference { sheet, area }));
                }
                return Err(syntax(FormulaErrorKind::UnexpectedText, start, number.to_string()));
            }
            _ => {
                self.position -= 1;
                return Err(self.unexpected());
            }
        };
        if self.peek() == Some(&Token::Colon) {
            let second = match self.peek_at(1) {
                Some(Token::Word(word)) => word.clone(),
                Some(Token::Number(number)) => format!("{number}"),
                _ => return Err(syntax(FormulaErrorKind::UnexpectedText, start, first)),
            };
            let area = cell_range(&first, &second)
                .or_else(|| column_range(&first, &second))
                .ok_or_else(|| syntax(FormulaErrorKind::UnexpectedText, start, format!("{first}:{second}")))?;
            self.position += 2;
            return Ok(Expr::Reference(Reference { sheet, area }));
        }
        match cell(&first) {
            Some((row, column)) => Ok(Expr::Reference(Reference { sheet, area: Area::cell(row, column) })),
            None if sheet.is_none() => Ok(Expr::Name(first)),
            None => Err(syntax(FormulaErrorKind::UnexpectedText, start, first)),
        }
    }

    fn call(&mut self, word: String, start: usize) -> Result<Expr, FormulaError> {
        self.position += 1;
        let name = canonical_function(&word);
        let mut arguments = Vec::new();
        if self.peek() == Some(&Token::Close) {
            self.position += 1;
        } else {
            loop {
                let argument = match self.peek() {
                    Some(Token::Separator | Token::Close) => Expr::Missing,
                    _ => self.expression(0)?,
                };
                arguments.push(argument);
                match self.next() {
                    Ok(Token::Separator) => continue,
                    Ok(Token::Close) => break,
                    Ok(_) => {
                        self.position -= 1;
                        return Err(self.unexpected());
                    }
                    Err(_) => return Err(syntax(FormulaErrorKind::UnexpectedEnd, start, ")")),
                }
            }
        }
        if let Some((least, most)) = super::eval::arity(&name)
            && (arguments.len() < least || arguments.len() > most)
        {
            return Err(syntax(FormulaErrorKind::WrongArgumentCount, start, name));
        }
        Ok(Expr::Call(name, arguments))
    }
}

fn binary(op: &str) -> (BinaryOp, u8) {
    match op {
        "=" => (BinaryOp::Equal, 1),
        "<>" => (BinaryOp::NotEqual, 1),
        "<" => (BinaryOp::Less, 1),
        "<=" => (BinaryOp::LessOrEqual, 1),
        ">" => (BinaryOp::Greater, 1),
        ">=" => (BinaryOp::GreaterOrEqual, 1),
        "&" => (BinaryOp::Concat, 2),
        "+" => (BinaryOp::Add, 3),
        "-" => (BinaryOp::Subtract, 3),
        "*" => (BinaryOp::Multiply, 4),
        "/" => (BinaryOp::Divide, 4),
        _ => (BinaryOp::Power, 5),
    }
}

fn describe(token: &Token) -> String {
    match token {
        Token::Number(number) => number.to_string(),
        Token::Text(text) => format!("\"{text}\""),
        Token::Error(error) | Token::Word(error) => error.clone(),
        Token::Quoted(sheet) => format!("'{sheet}'"),
        Token::Bang => "!".to_owned(),
        Token::Colon => ":".to_owned(),
        Token::Op(op) => (*op).to_owned(),
        Token::Open => "(".to_owned(),
        Token::Close => ")".to_owned(),
        Token::Separator => ",".to_owned(),
        Token::OpenBrace => "{".to_owned(),
        Token::CloseBrace => "}".to_owned(),
    }
}

/// `B3` or `$B$3`.
fn cell(word: &str) -> Option<(u32, u32)> {
    crate::xlsx::refs::parse_cell(word)
}

fn cell_range(first: &str, second: &str) -> Option<Area> {
    let (a, b) = (cell(first)?, cell(second)?);
    Some(Area { top: a.0.min(b.0), left: a.1.min(b.1), bottom: a.0.max(b.0), right: a.1.max(b.1) })
}

fn column_range(first: &str, second: &str) -> Option<Area> {
    let column = |text: &str| column_index(text.trim_start_matches('$'));
    let (a, b) = (column(first)?, column(second)?);
    Some(Area { top: 0, left: a.min(b), bottom: MAX_ROWS - 1, right: a.max(b) })
}

fn row_range(first: f64, last: f64) -> Option<Area> {
    let row = |number: f64| (number.fract() == 0.0 && number >= 1.0 && number <= f64::from(MAX_ROWS)).then(|| number as u32 - 1);
    let (a, b) = (row(first)?, row(last)?);
    Some(Area { top: a.min(b), left: 0, bottom: a.max(b), right: MAX_COLUMNS - 1 })
}

/// A function's English name in upper case, from its English or Russian name, with the
/// prefixes newer functions have in files (`_xlfn.XLOOKUP`) taken off.
fn canonical_function(word: &str) -> String {
    let upper = word.to_uppercase();
    let bare = upper.trim_start_matches("_XLFN.").trim_start_matches("_XLWS.").to_owned();
    RUSSIAN.iter().find(|(russian, _)| *russian == bare).map_or(bare, |(_, english)| (*english).to_owned())
}

/// Russian names of the functions Excel shows in Russian.
const RUSSIAN: [(&str, &str); 53] = [
    ("СУММ", "SUM"),
    ("СРЗНАЧ", "AVERAGE"),
    ("МИН", "MIN"),
    ("МАКС", "MAX"),
    ("СЧЁТ", "COUNT"),
    ("СЧЕТ", "COUNT"),
    ("СЧЁТЗ", "COUNTA"),
    ("СЧЕТЗ", "COUNTA"),
    ("СЧИТАТЬПУСТОТЫ", "COUNTBLANK"),
    ("ЕСЛИ", "IF"),
    ("ЕСЛИОШИБКА", "IFERROR"),
    ("И", "AND"),
    ("ИЛИ", "OR"),
    ("НЕ", "NOT"),
    ("ОКРУГЛ", "ROUND"),
    ("ОКРУГЛВВЕРХ", "ROUNDUP"),
    ("ОКРУГЛВНИЗ", "ROUNDDOWN"),
    ("ЦЕЛОЕ", "INT"),
    ("ABS", "ABS"),
    ("ОСТАТ", "MOD"),
    ("СТЕПЕНЬ", "POWER"),
    ("КОРЕНЬ", "SQRT"),
    ("СУММЕСЛИ", "SUMIF"),
    ("СЧЁТЕСЛИ", "COUNTIF"),
    ("СЧЕТЕСЛИ", "COUNTIF"),
    ("СРЗНАЧЕСЛИ", "AVERAGEIF"),
    ("СУММПРОИЗВ", "SUMPRODUCT"),
    ("ВПР", "VLOOKUP"),
    ("ГПР", "HLOOKUP"),
    ("ИНДЕКС", "INDEX"),
    ("ПОИСКПОЗ", "MATCH"),
    ("ПРОСМОТРX", "XLOOKUP"),
    ("ДЛСТР", "LEN"),
    ("ЛЕВСИМВ", "LEFT"),
    ("ПРАВСИМВ", "RIGHT"),
    ("ПСТР", "MID"),
    ("ПРОПИСН", "UPPER"),
    ("СТРОЧН", "LOWER"),
    ("ПРОПНАЧ", "PROPER"),
    ("СЖПРОБЕЛЫ", "TRIM"),
    ("СЦЕПИТЬ", "CONCATENATE"),
    ("СЦЕП", "CONCAT"),
    ("ТЕКСТ", "TEXT"),
    ("ЗНАЧЕН", "VALUE"),
    ("ПОДСТАВИТЬ", "SUBSTITUTE"),
    ("СЕГОДНЯ", "TODAY"),
    ("ТДАТА", "NOW"),
    ("ДАТА", "DATE"),
    ("ГОД", "YEAR"),
    ("МЕСЯЦ", "MONTH"),
    ("ДЕНЬ", "DAY"),
    ("ЕПУСТО", "ISBLANK"),
    ("ЕЧИСЛО", "ISNUMBER"),
];

/// Reads a formula as written in a file: English names, `,` between arguments, `.` in numbers.
pub(crate) fn parse(formula: &str) -> Result<Expr, FormulaError> {
    parse_with(formula, ',', '.')
}

fn parse_with(formula: &str, separator: char, decimal: char) -> Result<Expr, FormulaError> {
    let tokens = tokenize(formula, separator, decimal)?;
    let mut parser = Parser { tokens, position: 0, length: formula.chars().count() };
    let expression = parser.expression(0)?;
    if parser.position < parser.tokens.len() {
        return Err(parser.unexpected());
    }
    Ok(expression)
}

/// A formula as a person typed it, after the `=`, written the way files keep it. Arguments
/// may be separated by `;`, as in Russian Excel, and then numbers may have a decimal comma;
/// functions may have their Russian names.
pub(crate) fn normalize(input: &str) -> Result<String, FormulaError> {
    // Russian Excel separates arguments with `;` and writes `1,5`; a formula with `;` or with
    // Russian function names is read so, and so is one that reads only so (`=B2*1,5`).
    let russian_tokens = tokenize(input, ';', ',');
    let russian = russian_tokens.as_ref().is_ok_and(|tokens| {
        tokens.iter().enumerate().any(|(index, (token, _))| {
            *token == Token::Separator
                || matches!(token, Token::Word(word) if word.chars().any(|c| matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё'))
                    && tokens.get(index + 1).is_some_and(|(next, _)| *next == Token::Open))
        })
    });
    let english_fails = !russian && parse_with(input, ',', '.').is_err();
    let (separator, decimal) = if russian || (english_fails && parse_with(input, ';', ',').is_ok()) { (';', ',') } else { (',', '.') };
    // Fails here with the position in what was typed.
    parse_with(input, separator, decimal)?;
    let tokens = tokenize(input, separator, decimal)?;
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::new();
    for (index, (token, start)) in tokens.iter().enumerate() {
        let end = tokens.get(index + 1).map_or(chars.len(), |(_, next)| *next);
        let original: String = chars[*start..end].iter().collect();
        let spaces = &original[original.trim_end().len()..];
        let followed_by_open = tokens.get(index + 1).is_some_and(|(next, _)| *next == Token::Open);
        match token {
            Token::Separator => out.push(','),
            Token::Number(number) => out.push_str(&format_number(*number)),
            Token::Word(word) if followed_by_open => out.push_str(&written_function(word)),
            Token::Word(word) if matches!(word.to_uppercase().as_str(), "ИСТИНА" | "TRUE") => out.push_str("TRUE"),
            Token::Word(word) if matches!(word.to_uppercase().as_str(), "ЛОЖЬ" | "FALSE") => out.push_str("FALSE"),
            Token::Word(word) if cell(word).is_some() => out.push_str(&word.to_uppercase()),
            _ => out.push_str(original.trim_end()),
        }
        out.push_str(spaces);
    }
    Ok(out)
}

/// A function name as files keep it: English, upper case, with `_xlfn.` for the functions
/// newer than Excel 2007 that need it.
fn written_function(word: &str) -> String {
    let name = canonical_function(word);
    match name.as_str() {
        "XLOOKUP" | "CONCAT" | "IFS" | "TEXTJOIN" | "MAXIFS" | "MINIFS" | "SWITCH" => format!("_xlfn.{name}"),
        _ => name,
    }
}

fn format_number(number: f64) -> String {
    if number.fract() == 0.0 && number.abs() < 1e15 { format!("{}", number as i64) } else { format!("{number}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(sheet: Option<&str>, top: u32, left: u32, bottom: u32, right: u32) -> Expr {
        Expr::Reference(Reference { sheet: sheet.map(str::to_owned), area: Area { top, left, bottom, right } })
    }

    #[test]
    fn reads_references() {
        assert_eq!(parse("A1").unwrap(), reference(None, 0, 0, 0, 0));
        assert_eq!(parse("$B$3:C$10").unwrap(), reference(None, 2, 1, 9, 2));
        assert_eq!(parse("Sheet2!A1:A3").unwrap(), reference(Some("Sheet2"), 0, 0, 2, 0));
        assert_eq!(parse("'My Sheet'!B2").unwrap(), reference(Some("My Sheet"), 1, 1, 1, 1));
        assert_eq!(parse("A:B").unwrap(), reference(None, 0, 0, MAX_ROWS - 1, 1));
        assert_eq!(parse("2:5").unwrap(), reference(None, 1, 0, 4, MAX_COLUMNS - 1));
        assert_eq!(parse("Rate").unwrap(), Expr::Name("Rate".to_owned()));
    }

    #[test]
    fn reads_operators_as_excel_does() {
        // Negation before the power: -2^2 is 4.
        assert_eq!(
            parse("-2^2").unwrap(),
            Expr::Binary(BinaryOp::Power, Box::new(Expr::Unary(UnaryOp::Negate, Box::new(Expr::Number(2.0)))), Box::new(Expr::Number(2.0)))
        );
        assert_eq!(
            parse("1+2*3").unwrap(),
            Expr::Binary(
                BinaryOp::Add,
                Box::new(Expr::Number(1.0)),
                Box::new(Expr::Binary(BinaryOp::Multiply, Box::new(Expr::Number(2.0)), Box::new(Expr::Number(3.0))))
            )
        );
        assert!(matches!(parse("50%").unwrap(), Expr::Unary(UnaryOp::Percent, _)));
        assert!(matches!(parse("A1&\" \"&B1").unwrap(), Expr::Binary(BinaryOp::Concat, ..)));
        assert!(matches!(parse("A1<>\"x\"").unwrap(), Expr::Binary(BinaryOp::NotEqual, ..)));
    }

    #[test]
    fn reads_calls() {
        assert_eq!(
            parse("_xlfn.XLOOKUP(A1,B:B,C:C)").unwrap(),
            Expr::Call(
                "XLOOKUP".to_owned(),
                vec![reference(None, 0, 0, 0, 0), reference(None, 0, 1, MAX_ROWS - 1, 1), reference(None, 0, 2, MAX_ROWS - 1, 2)]
            )
        );
        assert_eq!(parse("IF(A1,,2)").unwrap(), Expr::Call("IF".to_owned(), vec![reference(None, 0, 0, 0, 0), Expr::Missing, Expr::Number(2.0)]));
        assert_eq!(parse("TODAY()").unwrap(), Expr::Call("TODAY".to_owned(), vec![]));
        assert_eq!(parse("MYFUNC(1)").unwrap(), Expr::Call("MYFUNC".to_owned(), vec![Expr::Number(1.0)]));
    }

    #[test]
    fn reports_mistakes_where_they_are() {
        assert_eq!(parse("SUM(A1").unwrap_err().kind, FormulaErrorKind::UnexpectedEnd);
        assert_eq!(parse("1+").unwrap_err().kind, FormulaErrorKind::UnexpectedEnd);
        assert_eq!(parse("\"open").unwrap_err().kind, FormulaErrorKind::UnclosedText);
        let error = parse("1 2").unwrap_err();
        assert_eq!((error.kind, error.position), (FormulaErrorKind::UnexpectedText, 2));
        assert_eq!(parse("ROUND(1)").unwrap_err().kind, FormulaErrorKind::WrongArgumentCount);
    }

    #[test]
    fn writes_typed_formulas_the_way_files_keep_them() {
        assert_eq!(normalize("СУММ(a1:a3;1,5)").unwrap(), "SUM(A1:A3,1.5)");
        assert_eq!(normalize("ЕСЛИ(B2>0;\"да; нет\";ЛОЖЬ)").unwrap(), "IF(B2>0,\"да; нет\",FALSE)");
        assert_eq!(normalize("sum(A1:A3, 2)").unwrap(), "SUM(A1:A3, 2)");
        assert_eq!(normalize("ПРОСМОТРX(A1;B:B;C:C)").unwrap(), "_xlfn.XLOOKUP(A1,B:B,C:C)");
        assert_eq!(normalize("'Мой лист'!A1*2").unwrap(), "'Мой лист'!A1*2");
        assert!(normalize("СУММ(A1").is_err());
        // A decimal comma alone, and Russian names, read the Russian way.
        assert_eq!(normalize("B2*1,5").unwrap(), "B2*1.5");
        assert_eq!(normalize("СУММ(1,5)").unwrap(), "SUM(1.5)");
        assert_eq!(normalize("SUM(1,5)").unwrap(), "SUM(1,5)");
        assert!(normalize("1+").is_err());
    }
}
