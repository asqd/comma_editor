//! Formulas for computed columns: `{price} * {qty}`, `{first} & " " & {last}`,
//! `IF({total} > 1000, "big", "small")`.
//!
//! Columns are named in braces: by header name, by letter (`{B}`) or by number (`{#2}`).
//! Values are text; arithmetic reads numbers as people write them (`1 250,50`) and an empty
//! value counts as zero. A CSV file cannot keep a formula, so a computed column is written
//! as values, in one undoable step.

use std::cmp::Ordering;
use std::fmt;

use crate::document::{Document, Field, Record};
use crate::edit::{Change, Edit, EditOutcome, Prepared, Progress, map_rows};
use crate::error::{Error, Result};
use crate::profile::date_value;
use crate::view::{natural_cmp, parse_number};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormulaErrorKind {
    /// The formula ends where more is expected.
    UnexpectedEnd,
    /// `detail` is the text that does not fit.
    UnexpectedText,
    UnclosedText,
    UnclosedColumn,
    /// `detail` is the name.
    UnknownColumn,
    UnknownFunction,
    /// `detail` is the function name.
    WrongArgumentCount,
    /// `detail` is the value.
    NotANumber,
    NotADate,
    DivisionByZero,
}

/// What is wrong with a formula, or with its result in one row. `position` is the character
/// offset in the formula for syntax errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormulaError {
    pub kind: FormulaErrorKind,
    pub position: usize,
    pub detail: String,
}

impl fmt::Display for FormulaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let detail = &self.detail;
        match self.kind {
            FormulaErrorKind::UnexpectedEnd => write!(f, "the formula is incomplete"),
            FormulaErrorKind::UnexpectedText => write!(f, "unexpected “{detail}” at {}", self.position + 1),
            FormulaErrorKind::UnclosedText => write!(f, "a quoted text is not closed"),
            FormulaErrorKind::UnclosedColumn => write!(f, "a column name is not closed with }}"),
            FormulaErrorKind::UnknownColumn => write!(f, "there is no column “{detail}”"),
            FormulaErrorKind::UnknownFunction => write!(f, "there is no function {detail}"),
            FormulaErrorKind::WrongArgumentCount => write!(f, "wrong number of arguments for {detail}"),
            FormulaErrorKind::NotANumber => write!(f, "“{detail}” is not a number"),
            FormulaErrorKind::NotADate => write!(f, "“{detail}” is not a date"),
            FormulaErrorKind::DivisionByZero => write!(f, "division by zero"),
        }
    }
}

/// Result of writing a computed column.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FormulaReport {
    /// Rows written.
    pub rows: usize,
    /// Rows whose formula failed; they were left empty.
    pub errors: usize,
    /// The first failing row (a row of the file) and why.
    pub first_error: Option<(usize, FormulaError)>,
}

impl FormulaReport {
    fn add(&mut self, row: usize, error: Option<FormulaError>) {
        self.rows += 1;
        if let Some(error) = error {
            self.errors += 1;
            self.first_error.get_or_insert((row, error));
        }
    }
}

/// Functions a formula can call, with the number of arguments each takes.
pub const FUNCTIONS: &[(&str, usize, Option<usize>)] = &[
    ("IF", 3, Some(3)),
    ("ROUND", 1, Some(2)),
    ("ABS", 1, Some(1)),
    ("MIN", 1, None),
    ("MAX", 1, None),
    ("SUM", 1, None),
    ("LEN", 1, Some(1)),
    ("UPPER", 1, Some(1)),
    ("LOWER", 1, Some(1)),
    ("TRIM", 1, Some(1)),
    ("LEFT", 2, Some(2)),
    ("RIGHT", 2, Some(2)),
    ("MID", 3, Some(3)),
    ("REPLACE", 3, Some(3)),
    ("CONTAINS", 2, Some(2)),
    ("CONCAT", 1, None),
    ("COALESCE", 1, None),
    ("ISEMPTY", 1, Some(1)),
    ("YEAR", 1, Some(1)),
    ("MONTH", 1, Some(1)),
    ("DAY", 1, Some(1)),
    ("DAYS", 2, Some(2)),
];

/// A compiled formula with its columns resolved.
#[derive(Clone, Debug)]
pub struct Formula {
    expr: Expr,
}

#[derive(Clone, Debug)]
enum Expr {
    Number(f64),
    Text(String),
    Column(usize),
    Negate(Box<Expr>),
    Not(Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    Call(&'static str, Vec<Expr>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Concat,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

impl BinaryOp {
    /// Binding strength: higher binds tighter.
    fn precedence(self) -> u8 {
        match self {
            Self::Or => 1,
            Self::And => 2,
            Self::Eq | Self::Ne | Self::Lt | Self::Le | Self::Gt | Self::Ge => 4,
            Self::Concat => 5,
            Self::Add | Self::Sub => 6,
            Self::Mul | Self::Div | Self::Rem => 7,
        }
    }
}

const NOT_PRECEDENCE: u8 = 3;

// MARK: Tokens

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Text(String),
    Column(String),
    Name(String),
    Op(BinaryOp),
    Minus,
    Not,
    Open,
    Close,
    Comma,
}

fn syntax(kind: FormulaErrorKind, position: usize, detail: impl Into<String>) -> FormulaError {
    FormulaError { kind, position, detail: detail.into() }
}

fn tokenize(text: &str) -> std::result::Result<Vec<(Token, usize)>, FormulaError> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let token = match c {
            '{' => {
                let end = chars[i + 1..]
                    .iter()
                    .position(|&c| c == '}')
                    .ok_or_else(|| syntax(FormulaErrorKind::UnclosedColumn, start, ""))?;
                let name: String = chars[i + 1..i + 1 + end].iter().collect();
                i += end + 2;
                Token::Column(name)
            }
            '"' => {
                let mut value = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(syntax(FormulaErrorKind::UnclosedText, start, "")),
                        // A doubled quote stands for one, as in CSV.
                        Some('"') if chars.get(i + 1) == Some(&'"') => {
                            value.push('"');
                            i += 2;
                        }
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        Some(&c) => {
                            value.push(c);
                            i += 1;
                        }
                    }
                }
                Token::Text(value)
            }
            '0'..='9' | '.' => {
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let literal: String = chars[start..i].iter().collect();
                let number = literal
                    .parse()
                    .map_err(|_| syntax(FormulaErrorKind::UnexpectedText, start, literal.clone()))?;
                Token::Number(number)
            }
            c if c.is_alphabetic() || c == '_' => {
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                match word.to_uppercase().as_str() {
                    "AND" => Token::Op(BinaryOp::And),
                    "OR" => Token::Op(BinaryOp::Or),
                    "NOT" => Token::Not,
                    _ => Token::Name(word),
                }
            }
            _ => {
                let next = chars.get(i + 1).copied();
                let (token, width) = match (c, next) {
                    ('<', Some('>')) | ('!', Some('=')) => (Token::Op(BinaryOp::Ne), 2),
                    ('<', Some('=')) => (Token::Op(BinaryOp::Le), 2),
                    ('>', Some('=')) => (Token::Op(BinaryOp::Ge), 2),
                    ('=', Some('=')) => (Token::Op(BinaryOp::Eq), 2),
                    ('<', _) => (Token::Op(BinaryOp::Lt), 1),
                    ('>', _) => (Token::Op(BinaryOp::Gt), 1),
                    ('=', _) => (Token::Op(BinaryOp::Eq), 1),
                    ('&', _) => (Token::Op(BinaryOp::Concat), 1),
                    ('+', _) => (Token::Op(BinaryOp::Add), 1),
                    ('-', _) => (Token::Minus, 1),
                    ('*', _) => (Token::Op(BinaryOp::Mul), 1),
                    ('/', _) => (Token::Op(BinaryOp::Div), 1),
                    ('%', _) => (Token::Op(BinaryOp::Rem), 1),
                    ('(', _) => (Token::Open, 1),
                    (')', _) => (Token::Close, 1),
                    (',' | ';', _) => (Token::Comma, 1),
                    _ => return Err(syntax(FormulaErrorKind::UnexpectedText, start, c.to_string())),
                };
                i += width;
                token
            }
        };
        tokens.push((token, start));
    }
    Ok(tokens)
}

// MARK: Parsing

struct Parser<'a> {
    tokens: Vec<(Token, usize)>,
    next: usize,
    end: usize,
    columns: &'a dyn Fn(&str) -> Option<usize>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.next).map(|(token, _)| token)
    }

    fn position(&self) -> usize {
        self.tokens.get(self.next).map_or(self.end, |&(_, position)| position)
    }

    fn advance(&mut self) -> std::result::Result<(Token, usize), FormulaError> {
        let token = self
            .tokens
            .get(self.next)
            .cloned()
            .ok_or_else(|| syntax(FormulaErrorKind::UnexpectedEnd, self.end, ""))?;
        self.next += 1;
        Ok(token)
    }

    fn unexpected(&self, text: &str) -> FormulaError {
        syntax(FormulaErrorKind::UnexpectedText, self.position(), text)
    }

    /// Precedence climbing: parses operators that bind at least as tight as `min`.
    fn expression(&mut self, min: u8) -> std::result::Result<Expr, FormulaError> {
        let mut left = self.prefix()?;
        loop {
            let op = match self.peek() {
                Some(Token::Op(op)) => *op,
                Some(Token::Minus) => BinaryOp::Sub,
                _ => break,
            };
            if op.precedence() < min {
                break;
            }
            self.advance()?;
            // Left-associative: the right side binds tighter.
            let right = self.expression(op.precedence() + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn prefix(&mut self) -> std::result::Result<Expr, FormulaError> {
        let (token, position) = self.advance()?;
        match token {
            Token::Number(number) => Ok(Expr::Number(number)),
            Token::Text(text) => Ok(Expr::Text(text)),
            Token::Column(name) => (self.columns)(&name)
                .map(Expr::Column)
                .ok_or_else(|| syntax(FormulaErrorKind::UnknownColumn, position, name)),
            Token::Minus => Ok(Expr::Negate(Box::new(self.expression(BinaryOp::Mul.precedence() + 1)?))),
            Token::Not => Ok(Expr::Not(Box::new(self.expression(NOT_PRECEDENCE)?))),
            Token::Open => {
                let inner = self.expression(0)?;
                match self.advance()? {
                    (Token::Close, _) => Ok(inner),
                    _ => Err(syntax(FormulaErrorKind::UnexpectedText, self.tokens[self.next - 1].1, "")),
                }
            }
            Token::Name(name) => self.call(name, position),
            Token::Close => Err(syntax(FormulaErrorKind::UnexpectedText, position, ")")),
            Token::Comma => Err(syntax(FormulaErrorKind::UnexpectedText, position, ",")),
            Token::Op(_) => Err(syntax(FormulaErrorKind::UnexpectedText, position, "operator")),
        }
    }

    fn call(&mut self, name: String, position: usize) -> std::result::Result<Expr, FormulaError> {
        let upper = name.to_uppercase();
        let Some(&(function, min, max)) = FUNCTIONS.iter().find(|(function, ..)| *function == upper) else {
            return Err(syntax(FormulaErrorKind::UnknownFunction, position, name));
        };
        if self.peek() != Some(&Token::Open) {
            return Err(self.unexpected(&name));
        }
        self.advance()?;
        let mut arguments = Vec::new();
        if self.peek() == Some(&Token::Close) {
            self.advance()?;
        } else {
            loop {
                arguments.push(self.expression(0)?);
                match self.advance()? {
                    (Token::Comma, _) => continue,
                    (Token::Close, _) => break,
                    (_, at) => return Err(syntax(FormulaErrorKind::UnexpectedText, at, "")),
                }
            }
        }
        if arguments.len() < min || max.is_some_and(|max| arguments.len() > max) {
            return Err(syntax(FormulaErrorKind::WrongArgumentCount, position, function));
        }
        Ok(Expr::Call(function, arguments))
    }
}

impl Formula {
    /// Compiles `text`; `columns` resolves a name in braces to a column index.
    pub(crate) fn compile(text: &str, columns: &dyn Fn(&str) -> Option<usize>) -> std::result::Result<Self, FormulaError> {
        let tokens = tokenize(text)?;
        let mut parser = Parser { tokens, next: 0, end: text.chars().count(), columns };
        let expr = parser.expression(0)?;
        if let Some((token, position)) = parser.tokens.get(parser.next) {
            let text = match token {
                Token::Close => ")".to_owned(),
                Token::Comma => ",".to_owned(),
                _ => String::new(),
            };
            return Err(syntax(FormulaErrorKind::UnexpectedText, *position, text));
        }
        Ok(Self { expr })
    }

    /// The value of the formula for a row whose cells `cell` returns.
    pub(crate) fn evaluate(&self, cell: &dyn Fn(usize) -> String) -> std::result::Result<String, FormulaError> {
        let mut context = Context { cell, decimal_comma: false };
        let value = context.eval(&self.expr)?;
        Ok(context.text(&value))
    }
}

// MARK: Evaluation

#[derive(Clone, Debug)]
enum Value {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
}

fn runtime(kind: FormulaErrorKind, detail: impl Into<String>) -> FormulaError {
    FormulaError { kind, position: 0, detail: detail.into() }
}

struct Context<'a> {
    cell: &'a dyn Fn(usize) -> String,
    /// A number read from the data had a decimal comma, so results are written with one too.
    decimal_comma: bool,
}

type Eval<T> = std::result::Result<T, FormulaError>;

impl Context<'_> {
    fn eval(&mut self, expr: &Expr) -> Eval<Value> {
        Ok(match expr {
            Expr::Number(number) => Value::Number(*number),
            Expr::Text(text) => Value::Text(text.clone()),
            Expr::Column(column) => {
                let value = (self.cell)(*column);
                if value.is_empty() { Value::Empty } else { Value::Text(value) }
            }
            Expr::Negate(inner) => {
                let value = self.eval(inner)?;
                Value::Number(-self.number(&value)?)
            }
            Expr::Not(inner) => {
                let value = self.eval(inner)?;
                Value::Bool(!self.truth(&value))
            }
            Expr::Binary(op, left, right) => self.binary(*op, left, right)?,
            Expr::Call(function, arguments) => self.call(function, arguments)?,
        })
    }

    fn binary(&mut self, op: BinaryOp, left: &Expr, right: &Expr) -> Eval<Value> {
        // AND and OR evaluate the right side only when needed.
        match op {
            BinaryOp::And => {
                let left = self.eval(left)?;
                return Ok(Value::Bool(self.truth(&left) && {
                    let right = self.eval(right)?;
                    self.truth(&right)
                }));
            }
            BinaryOp::Or => {
                let left = self.eval(left)?;
                return Ok(Value::Bool(self.truth(&left) || {
                    let right = self.eval(right)?;
                    self.truth(&right)
                }));
            }
            _ => {}
        }
        let a = self.eval(left)?;
        let b = self.eval(right)?;
        Ok(match op {
            BinaryOp::Concat => Value::Text(self.text(&a) + &self.text(&b)),
            BinaryOp::Add => Value::Number(self.number(&a)? + self.number(&b)?),
            BinaryOp::Sub => Value::Number(self.number(&a)? - self.number(&b)?),
            BinaryOp::Mul => Value::Number(self.number(&a)? * self.number(&b)?),
            BinaryOp::Div | BinaryOp::Rem => {
                let (x, y) = (self.number(&a)?, self.number(&b)?);
                if y == 0.0 {
                    return Err(runtime(FormulaErrorKind::DivisionByZero, ""));
                }
                Value::Number(if op == BinaryOp::Div { x / y } else { x % y })
            }
            BinaryOp::Eq => Value::Bool(self.compare(&a, &b) == Ordering::Equal),
            BinaryOp::Ne => Value::Bool(self.compare(&a, &b) != Ordering::Equal),
            BinaryOp::Lt => Value::Bool(self.compare(&a, &b) == Ordering::Less),
            BinaryOp::Le => Value::Bool(self.compare(&a, &b) != Ordering::Greater),
            BinaryOp::Gt => Value::Bool(self.compare(&a, &b) == Ordering::Greater),
            BinaryOp::Ge => Value::Bool(self.compare(&a, &b) != Ordering::Less),
            BinaryOp::And | BinaryOp::Or => unreachable!("handled above"),
        })
    }

    fn call(&mut self, function: &str, arguments: &[Expr]) -> Eval<Value> {
        if function == "IF" {
            let condition = self.eval(&arguments[0])?;
            let branch = if self.truth(&condition) { &arguments[1] } else { &arguments[2] };
            return self.eval(branch);
        }
        let values = arguments.iter().map(|argument| self.eval(argument)).collect::<Eval<Vec<_>>>()?;
        let text = |context: &Self, index: usize| context.text(&values[index]);
        Ok(match function {
            "ROUND" => {
                let digits = if values.len() > 1 { self.number(&values[1])? } else { 0.0 };
                let scale = 10f64.powi(digits.clamp(-15.0, 15.0) as i32);
                Value::Number((self.number(&values[0])? * scale).round() / scale)
            }
            "ABS" => Value::Number(self.number(&values[0])?.abs()),
            "MIN" | "MAX" | "SUM" => {
                let numbers = values.iter().map(|value| self.number(value)).collect::<Eval<Vec<f64>>>()?;
                Value::Number(match function {
                    "MIN" => numbers.iter().copied().fold(f64::INFINITY, f64::min),
                    "MAX" => numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    _ => numbers.iter().sum(),
                })
            }
            "LEN" => Value::Number(text(self, 0).chars().count() as f64),
            "UPPER" => Value::Text(text(self, 0).to_uppercase()),
            "LOWER" => Value::Text(text(self, 0).to_lowercase()),
            "TRIM" => Value::Text(text(self, 0).trim().to_owned()),
            "LEFT" | "RIGHT" => {
                let value = text(self, 0);
                let count = self.number(&values[1])?.max(0.0) as usize;
                let length = value.chars().count();
                Value::Text(if function == "LEFT" {
                    value.chars().take(count).collect()
                } else {
                    value.chars().skip(length.saturating_sub(count)).collect()
                })
            }
            "MID" => {
                let start = (self.number(&values[1])?.max(1.0) as usize) - 1;
                let count = self.number(&values[2])?.max(0.0) as usize;
                Value::Text(text(self, 0).chars().skip(start).take(count).collect())
            }
            "REPLACE" => {
                let find = text(self, 1);
                let value = text(self, 0);
                Value::Text(if find.is_empty() { value } else { value.replace(&find, &text(self, 2)) })
            }
            "CONTAINS" => Value::Bool(text(self, 0).to_lowercase().contains(&text(self, 1).to_lowercase())),
            "CONCAT" => Value::Text(values.iter().map(|value| self.text(value)).collect()),
            "COALESCE" => values
                .iter()
                .find(|value| !matches!(value, Value::Empty) && !self.text(value).is_empty())
                .cloned()
                .unwrap_or(Value::Empty),
            "ISEMPTY" => Value::Bool(text(self, 0).trim().is_empty()),
            "YEAR" | "MONTH" | "DAY" => {
                let (year, month, day) = civil_from_days(self.date(&values[0])?.floor() as i64);
                Value::Number(match function {
                    "YEAR" => year,
                    "MONTH" => month,
                    _ => day,
                } as f64)
            }
            "DAYS" => Value::Number((self.date(&values[0])? - self.date(&values[1])?).floor()),
            _ => unreachable!("unknown functions fail to compile"),
        })
    }

    fn number(&mut self, value: &Value) -> Eval<f64> {
        match value {
            Value::Empty => Ok(0.0),
            Value::Number(number) => Ok(*number),
            Value::Bool(flag) => Ok(f64::from(u8::from(*flag))),
            Value::Text(text) => {
                let number = parse_number(text).ok_or_else(|| runtime(FormulaErrorKind::NotANumber, text.clone()))?;
                if has_decimal_comma(text) {
                    self.decimal_comma = true;
                }
                Ok(number)
            }
        }
    }

    fn date(&self, value: &Value) -> Eval<f64> {
        let text = self.text(value);
        date_value(&text).ok_or_else(|| runtime(FormulaErrorKind::NotADate, text))
    }

    fn truth(&self, value: &Value) -> bool {
        match value {
            Value::Empty => false,
            Value::Bool(flag) => *flag,
            Value::Number(number) => *number != 0.0,
            Value::Text(text) => !matches!(
                text.trim().to_lowercase().as_str(),
                "" | "0" | "false" | "no" | "нет"
            ),
        }
    }

    /// Numbers compare as numbers, dates as dates, anything else as text without regard to case.
    fn compare(&self, a: &Value, b: &Value) -> Ordering {
        let numeric = |value: &Value| match value {
            Value::Number(number) => Some(*number),
            Value::Bool(flag) => Some(f64::from(u8::from(*flag))),
            Value::Text(text) => parse_number(text),
            Value::Empty => None,
        };
        if let (Some(x), Some(y)) = (numeric(a), numeric(b)) {
            return x.total_cmp(&y);
        }
        let (x, y) = (self.text(a), self.text(b));
        if let (Some(x), Some(y)) = (date_value(&x), date_value(&y)) {
            return x.total_cmp(&y);
        }
        if x.to_lowercase() == y.to_lowercase() {
            return Ordering::Equal;
        }
        natural_cmp(&x, &y)
    }

    fn text(&self, value: &Value) -> String {
        match value {
            Value::Empty => String::new(),
            Value::Text(text) => text.clone(),
            Value::Bool(flag) => flag.to_string(),
            Value::Number(number) => format_number(*number, self.decimal_comma),
        }
    }
}

/// `12,5`: one comma and no point, so the comma is the decimal separator.
fn has_decimal_comma(text: &str) -> bool {
    match (text.rfind('.'), text.rfind(',')) {
        (Some(dot), Some(comma)) => comma > dot,
        (None, Some(_)) => text.matches(',').count() == 1,
        _ => false,
    }
}

/// Up to ten decimals, without trailing zeros: `0.1 + 0.2` gives `0.3`.
fn format_number(number: f64, decimal_comma: bool) -> String {
    let rounded = (number * 1e10).round() / 1e10;
    let text = if rounded.fract() == 0.0 && rounded.abs() < 1e15 {
        format!("{}", rounded as i64)
    } else {
        format!("{rounded}")
    };
    if decimal_comma { text.replace('.', ",") } else { text }
}

/// Year, month and day of a day count since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

// MARK: Python

/// The formula as a Python expression over a pandas row `r`, using the helpers `num`, `text`,
/// `truth`, `compare` and `round_half` that `Recipe::to_pandas` defines. `None` when it uses a
/// function without a close Python equivalent, or does not compile.
pub(crate) fn python_expression(text: &str) -> Option<String> {
    let names = std::cell::RefCell::new(Vec::<String>::new());
    let resolve = |name: &str| -> Option<usize> {
        let mut names = names.borrow_mut();
        names.push(name.trim().to_owned());
        Some(names.len() - 1)
    };
    let formula = Formula::compile(text, &resolve).ok()?;
    python(&formula.expr, &names.borrow())
}

fn python_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn python(expr: &Expr, names: &[String]) -> Option<String> {
    let go = |expr: &Expr| python(expr, names);
    let all = |arguments: &[Expr]| arguments.iter().map(go).collect::<Option<Vec<String>>>();
    Some(match expr {
        Expr::Number(number) => format!("{number}"),
        Expr::Text(text) => python_string(text),
        Expr::Column(index) => {
            let name = &names[*index];
            match name.strip_prefix('#').and_then(|n| n.parse::<usize>().ok()).filter(|&n| n > 0) {
                Some(n) => format!("r.iloc[{}]", n - 1),
                None => format!("r[{}]", python_string(name)),
            }
        }
        Expr::Negate(inner) => format!("(-num({}))", go(inner)?),
        Expr::Not(inner) => format!("(not truth({}))", go(inner)?),
        Expr::Binary(op, left, right) => {
            let (a, b) = (go(left)?, go(right)?);
            match op {
                BinaryOp::Or => format!("(truth({a}) or truth({b}))"),
                BinaryOp::And => format!("(truth({a}) and truth({b}))"),
                BinaryOp::Eq => format!("(compare({a}, {b}) == 0)"),
                BinaryOp::Ne => format!("(compare({a}, {b}) != 0)"),
                BinaryOp::Lt => format!("(compare({a}, {b}) < 0)"),
                BinaryOp::Le => format!("(compare({a}, {b}) <= 0)"),
                BinaryOp::Gt => format!("(compare({a}, {b}) > 0)"),
                BinaryOp::Ge => format!("(compare({a}, {b}) >= 0)"),
                BinaryOp::Concat => format!("(text({a}) + text({b}))"),
                BinaryOp::Add => format!("(num({a}) + num({b}))"),
                BinaryOp::Sub => format!("(num({a}) - num({b}))"),
                BinaryOp::Mul => format!("(num({a}) * num({b}))"),
                BinaryOp::Div => format!("(num({a}) / num({b}))"),
                BinaryOp::Rem => format!("(num({a}) % num({b}))"),
            }
        }
        Expr::Call(function, arguments) => {
            let a = all(arguments)?;
            let numbers = || a.iter().map(|x| format!("num({x})")).collect::<Vec<_>>().join(", ");
            match *function {
                "IF" => format!("({} if truth({}) else {})", a[1], a[0], a[2]),
                "ROUND" => format!("round_half(num({}), {})", a[0], a.get(1).map_or("0".to_owned(), |d| format!("num({d})"))),
                "ABS" => format!("abs(num({}))", a[0]),
                "MIN" => format!("min([{}])", numbers()),
                "MAX" => format!("max([{}])", numbers()),
                "SUM" => format!("sum([{}])", numbers()),
                "LEN" => format!("float(len(text({})))", a[0]),
                "UPPER" => format!("text({}).upper()", a[0]),
                "LOWER" => format!("text({}).lower()", a[0]),
                "TRIM" => format!("text({}).strip()", a[0]),
                "LEFT" => format!("text({})[:max(int(num({})), 0)]", a[0], a[1]),
                "RIGHT" => format!("(text({0})[-int(num({1})):] if int(num({1})) > 0 else \"\")", a[0], a[1]),
                "MID" => format!(
                    "text({0})[max(int(num({1})), 1) - 1:max(int(num({1})), 1) - 1 + max(int(num({2})), 0)]",
                    a[0], a[1], a[2]
                ),
                "REPLACE" => format!("(text({0}).replace(text({1}), text({2})) if text({1}) else text({0}))", a[0], a[1], a[2]),
                "CONTAINS" => format!("(text({1}).lower() in text({0}).lower())", a[0], a[1]),
                "CONCAT" => format!("\"\".join([{}])", a.iter().map(|x| format!("text({x})")).collect::<Vec<_>>().join(", ")),
                "COALESCE" => format!("next((v for v in [{}] if text(v) != \"\"), \"\")", a.join(", ")),
                "ISEMPTY" => format!("(text({}).strip() == \"\")", a[0]),
                _ => return None,
            }
        }
    })
}

// MARK: Document

impl Document {
    /// Compiles a formula against the columns of this document.
    pub fn compile_formula(&self, text: &str) -> Result<Formula> {
        let header = if self.has_header() { self.row(0).unwrap_or_default() } else { Vec::new() };
        let count = self.column_count();
        let resolve = |name: &str| -> Option<usize> {
            let name = name.trim();
            header
                .iter()
                .position(|title| title == name)
                .or_else(|| header.iter().position(|title| title.trim().to_lowercase() == name.to_lowercase()))
                .or_else(|| column_from_letters(name))
                .or_else(|| name.strip_prefix('#').and_then(|n| n.trim().parse::<usize>().ok()).and_then(|n| n.checked_sub(1)))
                .filter(|&column| column < count)
        };
        Formula::compile(text, &resolve).map_err(Error::Formula)
    }

    /// Values of the formula for the given visible rows, or why each failed.
    pub fn preview_formula(
        &self,
        text: &str,
        visible_rows: std::ops::Range<usize>,
    ) -> Result<Vec<std::result::Result<String, FormulaError>>> {
        let formula = self.compile_formula(text)?;
        Ok(visible_rows
            .filter_map(|row| self.visible_record(row))
            .map(|record| formula.evaluate(&|column| self.value_at(record, column).into_owned()))
            .collect())
    }

    /// Inserts a column named `name` before column `at`, with the formula's value for every
    /// data row, as one undoable step. Rows where the formula fails are left empty. Resets the view.
    pub fn add_formula_column(&mut self, text: &str, name: &str, at: usize) -> Result<FormulaReport> {
        let edit = Edit::AddFormulaColumn { formula: text.to_owned(), name: name.to_owned(), at };
        Ok(self.apply_edit(&edit)?.report.unwrap_or_default())
    }

    pub(crate) fn prepare_formula_column(
        &self,
        text: &str,
        name: &str,
        at: usize,
        progress: &Progress,
    ) -> Result<Option<Prepared>> {
        let formula = self.compile_formula(text)?;
        if at > self.column_count() {
            return Err(Error::ColumnOutOfRange { column: at, columns: self.column_count() });
        }
        let header = self.header_offset();
        let rows = 0..self.records.len();
        progress.expect(rows.len());
        // Rows are computed on all cores; the report is gathered in order afterwards.
        let Some(computed) = map_rows(rows, progress, |row| {
            let record = &self.records[row];
            let (value, error) = if row < header {
                (name.to_owned(), None)
            } else {
                self.evaluate(&formula, row)
            };
            let mut fields = self.fields(record);
            if fields.len() < at {
                fields.resize(at, Field::Text(String::new()));
            }
            fields.insert(at, Field::Text(value));
            Some((Record::modified(fields, record.term()), error))
        }) else {
            return Ok(None);
        };
        let mut report = FormulaReport::default();
        let mut records = Vec::with_capacity(computed.len());
        for (row, (record, error)) in computed.into_iter().enumerate() {
            if row >= header {
                report.add(row, error);
            }
            records.push(record);
        }
        Ok(Some(Prepared {
            change: Change::All { records, columns_after: self.column_count() + 1 },
            outcome: EditOutcome { changed: false, count: report.rows, report: Some(report) },
        }))
    }

    /// Writes the formula's value into `column` of every visible row, as one undoable step.
    /// The formula may use the column itself: `{price} * 1.2`.
    pub fn fill_column_with_formula(&mut self, text: &str, column: usize) -> Result<FormulaReport> {
        let edit = Edit::FillWithFormula { formula: text.to_owned(), column };
        Ok(self.apply_edit(&edit)?.report.unwrap_or_default())
    }

    pub(crate) fn prepare_formula_fill(&self, text: &str, column: usize, progress: &Progress) -> Result<Option<Prepared>> {
        let formula = self.compile_formula(text)?;
        if column >= self.column_count() {
            return Err(Error::ColumnOutOfRange { column, columns: self.column_count() });
        }
        // Values come from the rows as they were, so the formula may use the column itself.
        let rows = 0..self.visible_row_count();
        progress.expect(rows.len());
        let Some(computed) = map_rows(rows, progress, |visible| {
            let record = self.visible_record(visible)?;
            let (value, error) = self.evaluate(&formula, record);
            let mut value = Some(value);
            let update = self.updated_record(record, std::iter::once(column), |_, _| value.take());
            Some((record, update, error))
        }) else {
            return Ok(None);
        };
        let mut report = FormulaReport::default();
        let mut updates = Vec::new();
        for (record, update, error) in computed {
            report.add(record, error);
            updates.extend(update);
        }
        Ok(Some(Prepared::rows(updates, self.column_count(), Some(report))))
    }

    /// The formula's value for a row of the file; empty, with the reason, where it fails.
    fn evaluate(&self, formula: &Formula, row: usize) -> (String, Option<FormulaError>) {
        match formula.evaluate(&|column| self.value_at(row, column).into_owned()) {
            Ok(value) => (value, None),
            Err(error) => (String::new(), Some(error)),
        }
    }
}

/// `A` is column 0, `Z` 25, `AA` 26.
fn column_from_letters(name: &str) -> Option<usize> {
    if name.is_empty() || name.len() > 3 || !name.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    Some(name.bytes().fold(0, |n, b| n * 26 + usize::from(b - b'A') + 1) - 1)
}
