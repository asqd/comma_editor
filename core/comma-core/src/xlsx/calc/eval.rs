//! Computing Excel formulas: values, the way Excel turns one kind into another, operators and
//! the functions CommaEditor knows. A formula with a function it does not know, a defined name
//! or an array constant is not computed at all: its cell keeps the value Excel stored.

use std::cmp::Ordering;

use super::parse::{Area, BinaryOp, Expr, UnaryOp};
use crate::formula::civil_from_days;
use crate::profile::{date_value, days_from_civil};
use crate::view::parse_number;
use crate::xlsx::numfmt::{Decimal, DisplayLocale, NumberFormat, general};

/// Error values, as Excel writes them.
pub(crate) const DIV0: &str = "#DIV/0!";
pub(crate) const VALUE: &str = "#VALUE!";
pub(crate) const REF: &str = "#REF!";
pub(crate) const NUM: &str = "#NUM!";
pub(crate) const NA: &str = "#N/A";

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Number(f64),
    Text(String),
    Bool(bool),
    Error(String),
    Empty,
}

fn error(code: &str) -> Value {
    Value::Error(code.to_owned())
}

/// The formula cannot be computed here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Unsupported;

type Outcome<T> = Result<T, Unsupported>;

/// What formulas read: the cells of the workbook.
pub(crate) trait Cells {
    /// A sheet by its name, without case.
    fn sheet(&self, name: &str) -> Option<usize>;
    fn value(&mut self, sheet: usize, row: u32, column: u32) -> Outcome<Value>;
    /// The last row and column with content, which whole rows and columns stop at.
    fn extent(&self, sheet: usize) -> (u32, u32);
    fn locale(&self) -> &DisplayLocale;
    fn date1904(&self) -> bool;
    /// Today's date and the time as a serial number.
    fn now(&self) -> f64;
    /// The formula of a defined name used on a sheet, if the workbook has the name.
    fn name(&self, _name: &str, _sheet: usize) -> Option<std::sync::Arc<Expr>> {
        None
    }
    /// The cells a structured reference points to from a cell: their sheet and area.
    fn table(&self, _sheet: usize, _row: u32, _column: u32, _reference: &super::parse::TableRef) -> Option<(usize, Area)> {
        None
    }
}

/// An argument as a function gets it: a value, or cells it may go through.
enum Arg {
    Value(Value),
    Range(usize, Area),
    /// Values an expression gives cell by cell, in an array formula.
    Grid(Vec<Vec<Value>>),
}

/// Functions whose arguments are ranges by design, which an array formula may hand values.
const RANGE_FUNCTIONS: &[&str] = &[
    "SUM", "SUMPRODUCT", "AVERAGE", "COUNT", "COUNTA", "COUNTBLANK", "MIN", "MAX", "PRODUCT", "AND", "OR", "INDEX",
    "MATCH", "VLOOKUP", "HLOOKUP", "XLOOKUP", "SUMIF", "SUMIFS", "COUNTIF", "COUNTIFS", "AVERAGEIF", "SUBTOTAL",
];

/// Whether an expression reads more than one cell somewhere in it.
fn reads_many(expr: &Expr) -> bool {
    match expr {
        Expr::Reference(reference) => !reference.area.is_cell(),
        Expr::Array => true,
        Expr::Unary(_, operand) => reads_many(operand),
        Expr::Binary(_, left, right) => reads_many(left) || reads_many(right),
        Expr::Call(_, args) => args.iter().any(reads_many),
        _ => false,
    }
}

/// A value as true or false, or the error to give instead.
fn truth_of(value: Value) -> Result<bool, Value> {
    match value {
        Value::Bool(truth) => Ok(truth),
        Value::Number(number) => Ok(number != 0.0),
        Value::Empty => Ok(false),
        Value::Text(text) if text.eq_ignore_ascii_case("TRUE") => Ok(true),
        Value::Text(text) if text.eq_ignore_ascii_case("FALSE") => Ok(false),
        Value::Text(_) => Err(error(VALUE)),
        error @ Value::Error(_) => Err(error),
    }
}

fn width(grid: &[Vec<Value>]) -> usize {
    grid.iter().map(Vec::len).max().unwrap_or(0)
}

/// A cell of a grid; #N/A outside it, as in Excel.
fn pick(grid: &[Vec<Value>], row: usize, column: usize) -> Value {
    grid.get(row).and_then(|values| values.get(column)).cloned().unwrap_or_else(|| error(NA))
}

/// A cell of a grid combined with a larger one: a single row or column repeats.
fn stretched(grid: &[Vec<Value>], row: usize, column: usize) -> Value {
    let row = if grid.len() == 1 { 0 } else { row };
    let column = if width(grid) == 1 { 0 } else { column };
    pick(grid, row, column)
}

fn map_grid(grid: Vec<Vec<Value>>, f: impl Fn(Value) -> Value) -> Vec<Vec<Value>> {
    grid.into_iter().map(|values| values.into_iter().map(&f).collect()).collect()
}

/// Smallest and largest number of arguments of the functions known.
pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    let many = 255;
    Some(match name {
        "SUM" | "AVERAGE" | "MIN" | "MAX" | "COUNT" | "COUNTA" | "PRODUCT" | "AND" | "OR" | "CONCATENATE" | "CONCAT"
        | "SUMPRODUCT" => (1, many),
        "COUNTBLANK" | "NOT" | "INT" | "ABS" | "SQRT" | "SIGN" | "LEN" | "UPPER" | "LOWER" | "PROPER" | "TRIM" | "VALUE"
        | "YEAR" | "MONTH" | "DAY" | "ISBLANK" | "ISNUMBER" | "ISTEXT" | "ISERROR" | "ISNA" | "ISLOGICAL" => (1, 1),
        "IF" => (2, 3),
        "IFERROR" | "IFNA" | "ROUND" | "ROUNDUP" | "ROUNDDOWN" | "MOD" | "POWER" | "COUNTIF" | "TEXT" | "REPT" | "EXACT"
        | "EDATE" | "EOMONTH" => (2, 2),
        "TRUNC" | "LEFT" | "RIGHT" | "WEEKDAY" => (1, 2),
        "SUMIF" | "AVERAGEIF" | "INDEX" | "MATCH" | "FIND" | "SEARCH" => (2, 3),
        "SUMIFS" => (3, many),
        "SUBTOTAL" => (2, many),
        "COUNTIFS" => (2, many),
        "VLOOKUP" | "HLOOKUP" => (3, 4),
        "XLOOKUP" => (3, 6),
        "MID" | "DATE" => (3, 3),
        "SUBSTITUTE" => (3, 4),
        "TODAY" | "NOW" | "NA" | "PI" => (0, 0),
        _ => return None,
    })
}

pub(crate) struct Evaluator<'a, C: Cells> {
    pub cells: &'a mut C,
    /// Where the formula is: its sheet, row and column.
    pub sheet: usize,
    pub row: u32,
    pub column: u32,
    /// The first row and column of the range an array formula fills; `None` for other
    /// formulas, where a range read as one value gives the cell in the formula's row or column.
    pub array: Option<(u32, u32)>,
    /// Defined names being read into, to stop at a name that refers to itself.
    pub names: u8,
}

impl<C: Cells> Evaluator<'_, C> {
    /// The formula's value, as one cell shows it.
    pub(crate) fn evaluate(&mut self, expr: &Expr) -> Outcome<Value> {
        let Some((top, left)) = self.array else {
            return self.scalar(expr);
        };
        // An array formula works on whole ranges; each cell of its range shows its own value.
        let grid = self.grid_of(expr)?;
        Ok(pick(&grid, self.row.saturating_sub(top) as usize, self.column.saturating_sub(left) as usize))
    }

    /// The values of an expression cell by cell, as an array formula computes them: operators
    /// combine ranges cell by cell, a single row or column stretching over the other side.
    fn grid_of(&mut self, expr: &Expr) -> Outcome<Vec<Vec<Value>>> {
        Ok(match expr {
            Expr::Reference(_) | Expr::Name(_) | Expr::Table(_) => self.grid(expr)?,
            Expr::Unary(op, operand) => {
                let op = *op;
                map_grid(self.grid_of(operand)?, |value| match to_number(&value) {
                    Ok(number) => Value::Number(match op {
                        UnaryOp::Negate => -number,
                        UnaryOp::Plus => number,
                        UnaryOp::Percent => number / 100.0,
                    }),
                    Err(error) => error,
                })
            }
            Expr::Binary(op, left, right) => {
                let (a, b) = (self.grid_of(left)?, self.grid_of(right)?);
                let (rows, columns) = (a.len().max(b.len()), width(&a).max(width(&b)));
                let mut out = Vec::with_capacity(rows);
                for row in 0..rows {
                    let mut values = Vec::with_capacity(columns);
                    for column in 0..columns {
                        values.push(self.combine(*op, stretched(&a, row, column), stretched(&b, row, column)));
                    }
                    out.push(values);
                }
                out
            }
            Expr::Call(name, args) if name.eq_ignore_ascii_case("IF") && (2..=3).contains(&args.len()) => {
                let condition = self.grid_of(&args[0])?;
                let then = self.grid_of(&args[1])?;
                let otherwise = match args.get(2) {
                    Some(expr) => self.grid_of(expr)?,
                    None => vec![vec![Value::Bool(false)]],
                };
                let rows = condition.len().max(then.len()).max(otherwise.len());
                let columns = width(&condition).max(width(&then)).max(width(&otherwise));
                (0..rows)
                    .map(|row| {
                        (0..columns)
                            .map(|column| match truth_of(stretched(&condition, row, column)) {
                                Ok(true) => stretched(&then, row, column),
                                Ok(false) => stretched(&otherwise, row, column),
                                Err(error) => error,
                            })
                            .collect()
                    })
                    .collect()
            }
            // Functions made to go through ranges take the values as they are; any other one
            // would have to run cell by cell, which this engine does not do: Excel computes it.
            Expr::Call(name, args) if !RANGE_FUNCTIONS.contains(&name.to_uppercase().as_str()) && args.iter().any(reads_many) => {
                return Err(Unsupported);
            }
            expr => vec![vec![self.value(expr)?]],
        })
    }

    fn arg(&mut self, expr: &Expr) -> Outcome<Arg> {
        Ok(match expr {
            Expr::Reference(reference) => match &reference.sheet {
                None => Arg::Range(self.sheet, reference.area),
                Some(name) => match self.cells.sheet(name) {
                    Some(sheet) => Arg::Range(sheet, reference.area),
                    None => Arg::Value(error(REF)),
                },
            },
            // A structured reference gives the cells of the table it points to.
            Expr::Table(reference) => {
                let (sheet, area) = self.cells.table(self.sheet, self.row, self.column, reference).ok_or(Unsupported)?;
                Arg::Range(sheet, area)
            }
            // A defined name gives what its formula gives: cells, a range, a value.
            Expr::Name(name) => {
                let defined = self.cells.name(name, self.sheet).filter(|_| self.names < 8).ok_or(Unsupported)?;
                self.names += 1;
                let arg = self.arg(&defined);
                self.names -= 1;
                return arg;
            }
            // In an array formula an expression such as `B2:B9*C2:C9` gives many values.
            expr if self.array.is_some() => match self.grid_of(expr)? {
                grid if grid.len() == 1 && width(&grid) == 1 => Arg::Value(pick(&grid, 0, 0)),
                grid => Arg::Grid(grid),
            },
            expr => Arg::Value(self.value(expr)?),
        })
    }

    /// A value from an expression that is not a reference.
    fn value(&mut self, expr: &Expr) -> Outcome<Value> {
        Ok(match expr {
            Expr::Number(number) => Value::Number(*number),
            Expr::Text(text) => Value::Text(text.clone()),
            Expr::Bool(truth) => Value::Bool(*truth),
            Expr::Error(code) => Value::Error(code.clone()),
            Expr::Missing => Value::Empty,
            Expr::Name(_) | Expr::Table(_) => return self.scalar(expr),
            Expr::Array => return Err(Unsupported),
            Expr::Reference(_) => return self.scalar(expr),
            Expr::Unary(op, operand) => {
                let number = match self.number(operand)? {
                    Ok(number) => number,
                    Err(value) => return Ok(value),
                };
                Value::Number(match op {
                    UnaryOp::Negate => -number,
                    UnaryOp::Plus => number,
                    UnaryOp::Percent => number / 100.0,
                })
            }
            Expr::Binary(op, left, right) => self.binary(*op, left, right)?,
            Expr::Call(name, arguments) => self.call(name, arguments)?,
        })
    }

    /// The value of one cell: a range gives the cell in the formula's row or column, as Excel
    /// did before dynamic arrays.
    fn scalar(&mut self, expr: &Expr) -> Outcome<Value> {
        match self.arg(expr)? {
            Arg::Value(value) => Ok(value),
            Arg::Grid(grid) => Ok(pick(&grid, 0, 0)),
            Arg::Range(sheet, area) => {
                if area.is_cell() {
                    return self.cells.value(sheet, area.top, area.left);
                }
                if area.left == area.right && (area.top..=area.bottom).contains(&self.row) {
                    return self.cells.value(sheet, self.row, area.left);
                }
                if area.top == area.bottom && (area.left..=area.right).contains(&self.column) {
                    return self.cells.value(sheet, area.top, self.column);
                }
                Ok(error(VALUE))
            }
        }
    }

    /// A number, or the error value to return instead.
    fn number(&mut self, expr: &Expr) -> Outcome<Result<f64, Value>> {
        Ok(to_number(&self.scalar(expr)?))
    }

    fn text(&mut self, expr: &Expr) -> Outcome<Result<String, Value>> {
        let value = self.scalar(expr)?;
        Ok(self.to_text(&value))
    }

    fn to_text(&self, value: &Value) -> Result<String, Value> {
        Ok(match value {
            Value::Number(number) => general(*number, self.cells.locale()),
            Value::Text(text) => text.clone(),
            Value::Bool(truth) => (if *truth { "TRUE" } else { "FALSE" }).to_owned(),
            Value::Empty => String::new(),
            Value::Error(_) => return Err(value.clone()),
        })
    }

    fn truth(&mut self, expr: &Expr) -> Outcome<Result<bool, Value>> {
        Ok(truth_of(self.scalar(expr)?))
    }

    fn binary(&mut self, op: BinaryOp, left: &Expr, right: &Expr) -> Outcome<Value> {
        let (a, b) = (self.scalar(left)?, self.scalar(right)?);
        Ok(self.combine(op, a, b))
    }

    /// Two values joined by an operator.
    fn combine(&self, op: BinaryOp, a: Value, b: Value) -> Value {
        if let Value::Error(_) = a {
            return a;
        }
        if let Value::Error(_) = b {
            return b;
        }
        let arithmetic = |f: fn(f64, f64) -> Value| match (to_number(&a), to_number(&b)) {
            (Ok(x), Ok(y)) => f(x, y),
            (Err(error), _) | (_, Err(error)) => error,
        };
        match op {
            BinaryOp::Add => arithmetic(|x, y| Value::Number(x + y)),
            BinaryOp::Subtract => arithmetic(|x, y| Value::Number(x - y)),
            BinaryOp::Multiply => arithmetic(|x, y| Value::Number(x * y)),
            BinaryOp::Divide => arithmetic(|x, y| if y == 0.0 { error(DIV0) } else { Value::Number(x / y) }),
            BinaryOp::Power => arithmetic(|x, y| {
                let result = x.powf(y);
                if (x == 0.0 && y == 0.0) || !result.is_finite() { error(NUM) } else { Value::Number(result) }
            }),
            BinaryOp::Concat => match (self.to_text(&a), self.to_text(&b)) {
                (Ok(x), Ok(y)) => Value::Text(x + &y),
                (Err(error), _) | (_, Err(error)) => error,
            },
            BinaryOp::Equal => Value::Bool(compare(&a, &b) == Ordering::Equal),
            BinaryOp::NotEqual => Value::Bool(compare(&a, &b) != Ordering::Equal),
            BinaryOp::Less => Value::Bool(compare(&a, &b) == Ordering::Less),
            BinaryOp::LessOrEqual => Value::Bool(compare(&a, &b) != Ordering::Greater),
            BinaryOp::Greater => Value::Bool(compare(&a, &b) == Ordering::Greater),
            BinaryOp::GreaterOrEqual => Value::Bool(compare(&a, &b) != Ordering::Less),
        }
    }

    /// Values of the cells of a range, by rows; whole rows and columns stop where the sheet's
    /// content does.
    fn range_values(&mut self, sheet: usize, area: Area) -> Outcome<Vec<Vec<Value>>> {
        let (last_row, last_column) = self.cells.extent(sheet);
        let bottom = area.bottom.min(last_row.max(area.top));
        let right = area.right.min(last_column.max(area.left));
        let mut rows = Vec::with_capacity((bottom - area.top + 1) as usize);
        for row in area.top..=bottom {
            let mut values = Vec::with_capacity((right - area.left + 1) as usize);
            for column in area.left..=right {
                values.push(self.cells.value(sheet, row, column)?);
            }
            rows.push(values);
        }
        Ok(rows)
    }

    /// Every value the arguments hold, and whether each came from a cell: functions such as
    /// SUM skip text and true/false in cells but read them when given directly.
    fn flatten(&mut self, arguments: &[Expr]) -> Outcome<Vec<(Value, bool)>> {
        let mut values = Vec::new();
        for argument in arguments {
            match self.arg(argument)? {
                Arg::Value(value) => values.push((value, false)),
                // Values an array formula computed count as cells do: text and true/false skipped.
                Arg::Grid(grid) => values.extend(grid.into_iter().flatten().map(|value| (value, true))),
                Arg::Range(sheet, area) => {
                    values.extend(self.range_values(sheet, area)?.into_iter().flatten().map(|value| (value, true)));
                }
            }
        }
        Ok(values)
    }

    /// The numbers among the arguments, as SUM and AVERAGE count them; the first error found
    /// instead.
    fn numbers(&mut self, arguments: &[Expr]) -> Outcome<Result<Vec<f64>, Value>> {
        let mut numbers = Vec::new();
        for (value, from_cell) in self.flatten(arguments)? {
            match value {
                Value::Number(number) => numbers.push(number),
                Value::Error(_) => return Ok(Err(value)),
                Value::Bool(truth) if !from_cell => numbers.push(f64::from(u8::from(truth))),
                Value::Text(text) if !from_cell => match parse_number(&text) {
                    Some(number) => numbers.push(number),
                    None => return Ok(Err(error(VALUE))),
                },
                _ => {}
            }
        }
        Ok(Ok(numbers))
    }

    /// The cells of an argument as a grid, a single value as a grid of one.
    fn grid(&mut self, expr: &Expr) -> Outcome<Vec<Vec<Value>>> {
        Ok(match self.arg(expr)? {
            Arg::Value(value) => vec![vec![value]],
            Arg::Grid(grid) => grid,
            Arg::Range(sheet, area) => self.range_values(sheet, area)?,
        })
    }

    fn call(&mut self, name: &str, args: &[Expr]) -> Outcome<Value> {
        macro_rules! number {
            ($expr:expr) => {
                match self.number($expr)? {
                    Ok(number) => number,
                    Err(value) => return Ok(value),
                }
            };
        }
        macro_rules! text {
            ($expr:expr) => {
                match self.text($expr)? {
                    Ok(text) => text,
                    Err(value) => return Ok(value),
                }
            };
        }
        macro_rules! numbers {
            ($args:expr) => {
                match self.numbers($args)? {
                    Ok(numbers) => numbers,
                    Err(value) => return Ok(value),
                }
            };
        }
        let optional = |index: usize| args.get(index).filter(|arg| **arg != Expr::Missing);
        Ok(match name {
            "SUM" => Value::Number(numbers!(args).iter().sum()),
            // Totals rows of tables use it. Rows hidden by a filter count, as rows of the file
            // are all shown here, and so do subtotals inside the ranges, which Excel leaves out.
            "SUBTOTAL" => {
                let code = number!(&args[0]).trunc() as i64;
                let function = match code {
                    1 | 101 => "AVERAGE",
                    2 | 102 => "COUNT",
                    3 | 103 => "COUNTA",
                    4 | 104 => "MAX",
                    5 | 105 => "MIN",
                    6 | 106 => "PRODUCT",
                    9 | 109 => "SUM",
                    // Deviations and variances are left to Excel.
                    7 | 8 | 10 | 11 | 107 | 108 | 110 | 111 => return Err(Unsupported),
                    _ => return Ok(error(VALUE)),
                };
                return self.call(function, &args[1..]);
            }
            "PRODUCT" => Value::Number(numbers!(args).iter().product()),
            "AVERAGE" => {
                let numbers = numbers!(args);
                if numbers.is_empty() { error(DIV0) } else { Value::Number(numbers.iter().sum::<f64>() / numbers.len() as f64) }
            }
            "MIN" => Value::Number(numbers!(args).into_iter().reduce(f64::min).unwrap_or(0.0)),
            "MAX" => Value::Number(numbers!(args).into_iter().reduce(f64::max).unwrap_or(0.0)),
            "COUNT" => Value::Number(
                self.flatten(args)?
                    .iter()
                    .filter(|(value, from_cell)| match value {
                        Value::Number(_) => true,
                        Value::Bool(_) => !from_cell,
                        Value::Text(text) => !from_cell && parse_number(text).is_some(),
                        _ => false,
                    })
                    .count() as f64,
            ),
            "COUNTA" => Value::Number(self.flatten(args)?.iter().filter(|(value, _)| *value != Value::Empty).count() as f64),
            "COUNTBLANK" => Value::Number(
                self.flatten(args)?
                    .iter()
                    .filter(|(value, _)| matches!(value, Value::Empty) || *value == Value::Text(String::new()))
                    .count() as f64,
            ),
            "IF" => match self.truth(&args[0])? {
                Ok(true) => self.scalar(&args[1])?,
                Ok(false) => match optional(2) {
                    Some(otherwise) => self.scalar(otherwise)?,
                    None if args.len() > 2 => Value::Number(0.0),
                    None => Value::Bool(false),
                },
                Err(value) => value,
            },
            "IFERROR" => match self.scalar(&args[0])? {
                Value::Error(_) => self.scalar(&args[1])?,
                value => value,
            },
            "IFNA" => match self.scalar(&args[0])? {
                Value::Error(code) if code == NA => self.scalar(&args[1])?,
                value => value,
            },
            "AND" | "OR" => {
                let mut result = name == "AND";
                for (value, from_cell) in self.flatten(args)? {
                    let truth = match value {
                        Value::Bool(truth) => truth,
                        Value::Number(number) => number != 0.0,
                        Value::Error(_) => return Ok(value),
                        _ if from_cell => continue,
                        _ => return Ok(error(VALUE)),
                    };
                    result = if name == "AND" { result && truth } else { result || truth };
                }
                Value::Bool(result)
            }
            "NOT" => match self.truth(&args[0])? {
                Ok(truth) => Value::Bool(!truth),
                Err(value) => value,
            },
            "ROUND" | "ROUNDUP" | "ROUNDDOWN" => {
                let (number, digits) = (number!(&args[0]), number!(&args[1]).trunc() as i32);
                Value::Number(round(number, digits, name))
            }
            "TRUNC" => {
                let digits = match optional(1) {
                    Some(digits) => number!(digits).trunc() as i32,
                    None => 0,
                };
                Value::Number(round(number!(&args[0]), digits, "ROUNDDOWN"))
            }
            "INT" => Value::Number(number!(&args[0]).floor()),
            "ABS" => Value::Number(number!(&args[0]).abs()),
            "SIGN" => {
                let number = number!(&args[0]);
                Value::Number(if number > 0.0 { 1.0 } else if number < 0.0 { -1.0 } else { 0.0 })
            }
            "PI" => Value::Number(std::f64::consts::PI),
            "MOD" => {
                let (x, y) = (number!(&args[0]), number!(&args[1]));
                if y == 0.0 { error(DIV0) } else { Value::Number(x - y * (x / y).floor()) }
            }
            "POWER" => {
                let result = number!(&args[0]).powf(number!(&args[1]));
                if result.is_finite() { Value::Number(result) } else { error(NUM) }
            }
            "SQRT" => {
                let number = number!(&args[0]);
                if number < 0.0 { error(NUM) } else { Value::Number(number.sqrt()) }
            }
            "SUMIF" | "AVERAGEIF" => {
                let range = self.grid(&args[0])?;
                let criterion = Criterion::new(&self.scalar(&args[1])?);
                let values = match optional(2) {
                    Some(sum_range) => self.grid(sum_range)?,
                    None => range.clone(),
                };
                let mut picked = Vec::new();
                for (row, cells) in range.iter().enumerate() {
                    for (column, cell) in cells.iter().enumerate() {
                        if criterion.matches(cell)
                            && let Some(Value::Number(number)) = values.get(row).and_then(|values| values.get(column))
                        {
                            picked.push(*number);
                        }
                    }
                }
                match name {
                    "SUMIF" => Value::Number(picked.iter().sum()),
                    _ if picked.is_empty() => error(DIV0),
                    _ => Value::Number(picked.iter().sum::<f64>() / picked.len() as f64),
                }
            }
            "COUNTIF" => {
                let criterion = Criterion::new(&self.scalar(&args[1])?);
                Value::Number(self.grid(&args[0])?.iter().flatten().filter(|cell| criterion.matches(cell)).count() as f64)
            }
            "SUMIFS" | "COUNTIFS" => {
                let (values, pairs) = if name == "SUMIFS" { (Some(self.grid(&args[0])?), &args[1..]) } else { (None, args) };
                if pairs.len() % 2 != 0 {
                    return Ok(error(VALUE));
                }
                let mut conditions = Vec::new();
                for pair in pairs.chunks(2) {
                    conditions.push((self.grid(&pair[0])?, Criterion::new(&self.scalar(&pair[1])?)));
                }
                let (rows, columns) = conditions.first().map_or((0, 0), |(grid, _)| (grid.len(), grid.first().map_or(0, Vec::len)));
                let mut total = 0.0;
                for row in 0..rows {
                    for column in 0..columns {
                        let all = conditions.iter().all(|(grid, criterion)| {
                            grid.get(row).and_then(|cells| cells.get(column)).is_some_and(|cell| criterion.matches(cell))
                        });
                        if !all {
                            continue;
                        }
                        total += match &values {
                            Some(values) => match values.get(row).and_then(|cells| cells.get(column)) {
                                Some(Value::Number(number)) => *number,
                                _ => 0.0,
                            },
                            None => 1.0,
                        };
                    }
                }
                Value::Number(total)
            }
            "SUMPRODUCT" => {
                let mut grids = Vec::new();
                for argument in args {
                    grids.push(self.grid(argument)?);
                }
                let shape = |grid: &Vec<Vec<Value>>| (grid.len(), grid.first().map_or(0, Vec::len));
                if grids.iter().any(|grid| shape(grid) != shape(&grids[0])) {
                    return Ok(error(VALUE));
                }
                let (rows, columns) = shape(&grids[0]);
                let mut total = 0.0;
                for row in 0..rows {
                    for column in 0..columns {
                        let mut product = 1.0;
                        for grid in &grids {
                            match &grid[row][column] {
                                Value::Number(number) => product *= number,
                                Value::Error(_) => return Ok(grid[row][column].clone()),
                                _ => product = 0.0,
                            }
                        }
                        total += product;
                    }
                }
                Value::Number(total)
            }
            "VLOOKUP" | "HLOOKUP" => {
                let lookup = self.scalar(&args[0])?;
                let table = self.grid(&args[1])?;
                let index = number!(&args[2]) as usize;
                let approximate = match optional(3) {
                    Some(flag) => match self.truth(flag)? {
                        Ok(flag) => flag,
                        Err(value) => return Ok(value),
                    },
                    None => true,
                };
                let vertical = name == "VLOOKUP";
                let keys: Vec<&Value> = if vertical {
                    table.iter().filter_map(|row| row.first()).collect()
                } else {
                    table.first().map(|row| row.iter().collect()).unwrap_or_default()
                };
                let width = if vertical { table.first().map_or(0, Vec::len) } else { table.len() };
                if index < 1 || index > width {
                    return Ok(error(REF));
                }
                match find(&keys, &lookup, if approximate { 1 } else { 0 }) {
                    Some(found) => {
                        let cell = if vertical { table[found].get(index - 1) } else { table[index - 1].get(found) };
                        cell.cloned().unwrap_or(Value::Empty)
                    }
                    None => error(NA),
                }
            }
            "MATCH" => {
                let lookup = self.scalar(&args[0])?;
                let grid = self.grid(&args[1])?;
                let kind = match optional(2) {
                    Some(kind) => number!(kind) as i32,
                    None => 1,
                };
                let keys: Vec<&Value> = if grid.len() == 1 { grid[0].iter().collect() } else { grid.iter().filter_map(|row| row.first()).collect() };
                match find(&keys, &lookup, kind) {
                    Some(found) => Value::Number(found as f64 + 1.0),
                    None => error(NA),
                }
            }
            "INDEX" => {
                let grid = self.grid(&args[0])?;
                let row = number!(&args[1]) as usize;
                let column = match optional(2) {
                    Some(column) => number!(column) as usize,
                    None => 0,
                };
                let (row, column) = match (row, column) {
                    (row, 0) if grid.len() == 1 => (1, row),
                    (row, 0) => (row, 1),
                    pair => pair,
                };
                grid.get(row.wrapping_sub(1)).and_then(|cells| cells.get(column.wrapping_sub(1))).cloned().unwrap_or_else(|| error(REF))
            }
            "XLOOKUP" => {
                let lookup = self.scalar(&args[0])?;
                let keys = self.grid(&args[1])?;
                let results = self.grid(&args[2])?;
                let vertical = keys.len() > 1 || keys.first().is_some_and(|row| row.len() == 1);
                let flat: Vec<&Value> = keys.iter().flatten().collect();
                match find(&flat, &lookup, 0) {
                    Some(found) => {
                        let cell = if vertical { results.get(found).and_then(|row| row.first()) } else { results.first().and_then(|row| row.get(found)) };
                        cell.cloned().unwrap_or_else(|| error(VALUE))
                    }
                    None => match optional(3) {
                        Some(missing) => self.scalar(missing)?,
                        None => error(NA),
                    },
                }
            }
            "LEN" => Value::Number(text!(&args[0]).chars().count() as f64),
            "LEFT" | "RIGHT" => {
                let text = text!(&args[0]);
                let count = match optional(1) {
                    Some(count) => number!(count),
                    None => 1.0,
                };
                if count < 0.0 {
                    return Ok(error(VALUE));
                }
                let chars: Vec<char> = text.chars().collect();
                let count = (count as usize).min(chars.len());
                let part = if name == "LEFT" { &chars[..count] } else { &chars[chars.len() - count..] };
                Value::Text(part.iter().collect())
            }
            "MID" => {
                let text = text!(&args[0]);
                let (start, count) = (number!(&args[1]), number!(&args[2]));
                if start < 1.0 || count < 0.0 {
                    return Ok(error(VALUE));
                }
                Value::Text(text.chars().skip(start as usize - 1).take(count as usize).collect())
            }
            "UPPER" => Value::Text(text!(&args[0]).to_uppercase()),
            "LOWER" => Value::Text(text!(&args[0]).to_lowercase()),
            "PROPER" => {
                let mut out = String::new();
                let mut after_letter = false;
                for c in text!(&args[0]).chars() {
                    if after_letter { out.extend(c.to_lowercase()) } else { out.extend(c.to_uppercase()) }
                    after_letter = c.is_alphabetic();
                }
                Value::Text(out)
            }
            "TRIM" => Value::Text(text!(&args[0]).split(' ').filter(|word| !word.is_empty()).collect::<Vec<_>>().join(" ")),
            "CONCATENATE" | "CONCAT" => {
                let mut out = String::new();
                for (value, _) in self.flatten(args)? {
                    match self.to_text(&value) {
                        Ok(text) => out.push_str(&text),
                        Err(value) => return Ok(value),
                    }
                }
                Value::Text(out)
            }
            "TEXT" => {
                let value = self.scalar(&args[0])?;
                let code = text!(&args[1]);
                let format = NumberFormat::parse(&code);
                match value {
                    Value::Error(_) => value,
                    Value::Text(text) => match parse_number(&text) {
                        Some(number) => Value::Text(format.format_number(number, self.cells.locale(), self.cells.date1904())),
                        None => Value::Text(format.format_text(&text)),
                    },
                    other => match to_number(&other) {
                        Ok(number) => Value::Text(format.format_number(number, self.cells.locale(), self.cells.date1904())),
                        Err(value) => value,
                    },
                }
            }
            "VALUE" => {
                let text = text!(&args[0]);
                match parse_number(&text).or_else(|| date_value(&text).map(|days| days + 25_569.0)) {
                    Some(number) => Value::Number(number),
                    None => error(VALUE),
                }
            }
            "SUBSTITUTE" => {
                let (text, old, new) = (text!(&args[0]), text!(&args[1]), text!(&args[2]));
                if old.is_empty() {
                    return Ok(Value::Text(text));
                }
                match optional(3) {
                    None => Value::Text(text.replace(&old, &new)),
                    Some(instance) => {
                        let instance = number!(instance) as usize;
                        match text.match_indices(&old).nth(instance.wrapping_sub(1)) {
                            Some((at, _)) => Value::Text(format!("{}{new}{}", &text[..at], &text[at + old.len()..])),
                            None => Value::Text(text),
                        }
                    }
                }
            }
            "FIND" | "SEARCH" => {
                let (needle, haystack) = (text!(&args[0]), text!(&args[1]));
                let start = match optional(2) {
                    Some(start) => number!(start) as usize,
                    None => 1,
                };
                let chars: Vec<char> = haystack.chars().collect();
                if start < 1 || start > chars.len() + 1 {
                    return Ok(error(VALUE));
                }
                let rest: String = chars[start - 1..].iter().collect();
                let found = if name == "FIND" { rest.find(&needle) } else { rest.to_lowercase().find(&needle.to_lowercase()) };
                match found {
                    Some(at) => Value::Number((start + rest[..at].chars().count()) as f64),
                    None => error(VALUE),
                }
            }
            "REPT" => {
                let (text, count) = (text!(&args[0]), number!(&args[1]));
                if count < 0.0 { error(VALUE) } else { Value::Text(text.repeat(count as usize)) }
            }
            "EXACT" => Value::Bool(text!(&args[0]) == text!(&args[1])),
            "TODAY" => Value::Number(self.cells.now().floor()),
            "NOW" => Value::Number(self.cells.now()),
            "DATE" => {
                let (year, month, day) = (number!(&args[0]) as i64, number!(&args[1]) as i64, number!(&args[2]) as i64);
                let year = if (0..1900).contains(&year) { year + 1900 } else { year };
                let months = year * 12 + month - 1;
                let first = days_from_civil(months.div_euclid(12), months.rem_euclid(12) + 1, 1);
                match serial_from_days(first + day - 1, self.cells.date1904()) {
                    Some(serial) => Value::Number(serial),
                    None => error(NUM),
                }
            }
            "YEAR" | "MONTH" | "DAY" => {
                let serial = number!(&args[0]);
                let Some((year, month, day)) = civil(serial, self.cells.date1904()) else { return Ok(error(NUM)) };
                Value::Number(match name {
                    "YEAR" => year,
                    "MONTH" => month,
                    _ => day,
                } as f64)
            }
            "WEEKDAY" => {
                let serial = number!(&args[0]).floor();
                let kind = match optional(1) {
                    Some(kind) => number!(kind) as i64,
                    None => 1,
                };
                // 0 is Sunday.
                let weekday = (days_from_serial(serial, self.cells.date1904()) + 4).rem_euclid(7);
                Value::Number(match kind {
                    2 => (weekday + 6) % 7 + 1,
                    3 => (weekday + 6) % 7,
                    _ => weekday + 1,
                } as f64)
            }
            "EDATE" | "EOMONTH" => {
                let (serial, months) = (number!(&args[0]), number!(&args[1]) as i64);
                let Some((year, month, day)) = civil(serial, self.cells.date1904()) else { return Ok(error(NUM)) };
                let total = year * 12 + month - 1 + months;
                let (year, month) = (total.div_euclid(12), total.rem_euclid(12) + 1);
                let days = if name == "EOMONTH" {
                    let next = year * 12 + month;
                    days_from_civil(next.div_euclid(12), next.rem_euclid(12) + 1, 1) - 1
                } else {
                    let last = days_from_civil((year * 12 + month).div_euclid(12), (year * 12 + month).rem_euclid(12) + 1, 1) - 1;
                    days_from_civil(year, month, day).min(last)
                };
                match serial_from_days(days, self.cells.date1904()) {
                    Some(serial) => Value::Number(serial),
                    None => error(NUM),
                }
            }
            "ISBLANK" => Value::Bool(self.scalar(&args[0])? == Value::Empty),
            "ISNUMBER" => Value::Bool(matches!(self.scalar(&args[0])?, Value::Number(_))),
            "ISTEXT" => Value::Bool(matches!(self.scalar(&args[0])?, Value::Text(_))),
            "ISLOGICAL" => Value::Bool(matches!(self.scalar(&args[0])?, Value::Bool(_))),
            "ISERROR" => Value::Bool(matches!(self.scalar(&args[0])?, Value::Error(_))),
            "ISNA" => Value::Bool(matches!(self.scalar(&args[0])?, Value::Error(code) if code == NA)),
            "NA" => error(NA),
            _ => return Err(Unsupported),
        })
    }
}

/// A number from a value, as arithmetic reads it; the error value to return otherwise.
pub(crate) fn to_number(value: &Value) -> Result<f64, Value> {
    match value {
        Value::Number(number) => Ok(*number),
        Value::Bool(truth) => Ok(f64::from(u8::from(*truth))),
        Value::Empty => Ok(0.0),
        Value::Text(text) => parse_number(text).ok_or_else(|| error(VALUE)),
        Value::Error(_) => Err(value.clone()),
    }
}

/// Excel's order: numbers, then text without case, then false and true. An empty cell is
/// zero against a number and empty text against text.
fn compare(a: &Value, b: &Value) -> Ordering {
    fn rank(value: &Value) -> u8 {
        match value {
            Value::Number(_) | Value::Empty => 0,
            Value::Text(_) => 1,
            Value::Bool(_) => 2,
            Value::Error(_) => 3,
        }
    }
    match (a, b) {
        (Value::Empty, Value::Text(text)) | (Value::Text(text), Value::Empty) if text.is_empty() => Ordering::Equal,
        (Value::Empty, Value::Text(_)) => Ordering::Less,
        (Value::Text(_), Value::Empty) => Ordering::Greater,
        (Value::Empty, Value::Bool(truth)) => false.cmp(truth),
        (Value::Bool(truth), Value::Empty) => truth.cmp(&false),
        (Value::Text(x), Value::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ if rank(a) == 0 && rank(b) == 0 => {
            let (x, y) = (to_number(a).unwrap_or(0.0), to_number(b).unwrap_or(0.0));
            x.partial_cmp(&y).unwrap_or(Ordering::Equal)
        }
        _ => rank(a).cmp(&rank(b)),
    }
}

/// Where `lookup` is among `keys`: exactly (`kind` 0, with `*` and `?` in text), or the last
/// key not above it in keys sorted up (1), or not below it in keys sorted down (-1).
fn find(keys: &[&Value], lookup: &Value, kind: i32) -> Option<usize> {
    if kind == 0 {
        let criterion = Criterion { op: Ordering::Equal, negate: false, value: lookup.clone(), pattern: wildcard(lookup) };
        return keys.iter().position(|key| criterion.matches(key));
    }
    let mut found = None;
    for (index, key) in keys.iter().enumerate() {
        if **key == Value::Empty || std::mem::discriminant(*key) != std::mem::discriminant(lookup) {
            continue;
        }
        match (compare(key, lookup), kind > 0) {
            (Ordering::Equal, _) => return Some(index),
            (Ordering::Less, true) | (Ordering::Greater, false) => found = Some(index),
            _ => break,
        }
    }
    found
}

/// A condition of SUMIF and COUNTIF: `">5"`, `"<>done"`, `"a*"`, or a value to equal.
struct Criterion {
    op: Ordering,
    /// `<>`, `<=` and `>=` are the opposite of `=`, `>` and `<`.
    negate: bool,
    value: Value,
    pattern: Option<regex::Regex>,
}

impl Criterion {
    fn new(criterion: &Value) -> Self {
        let Value::Text(text) = criterion else {
            return Self { op: Ordering::Equal, negate: false, value: criterion.clone(), pattern: None };
        };
        let (op, negate, rest) = [("<=", Ordering::Greater, true), (">=", Ordering::Less, true), ("<>", Ordering::Equal, true), ("<", Ordering::Less, false), (">", Ordering::Greater, false), ("=", Ordering::Equal, false)]
            .into_iter()
            .find_map(|(prefix, op, negate)| text.strip_prefix(prefix).map(|rest| (op, negate, rest)))
            .unwrap_or((Ordering::Equal, false, text.as_str()));
        let value = match parse_number(rest) {
            Some(number) if !rest.trim().is_empty() => Value::Number(number),
            _ if rest.is_empty() => Value::Empty,
            _ => Value::Text(rest.to_owned()),
        };
        let pattern = if op == Ordering::Equal { wildcard(&value) } else { None };
        Self { op, negate, value, pattern }
    }

    fn matches(&self, cell: &Value) -> bool {
        let hit = match (&self.value, cell) {
            (Value::Empty, cell) => matches!(cell, Value::Empty) || *cell == Value::Text(String::new()),
            (Value::Text(_), Value::Text(text)) if self.pattern.is_some() => {
                self.pattern.as_ref().is_some_and(|pattern| pattern.is_match(text))
            }
            (Value::Number(_), Value::Number(_)) | (Value::Text(_), Value::Text(_)) | (Value::Bool(_), Value::Bool(_)) => {
                compare(cell, &self.value) == self.op
            }
            (Value::Number(number), Value::Text(text)) if self.op == Ordering::Equal => parse_number(text) == Some(*number),
            _ => false,
        };
        hit != self.negate
    }
}

/// A pattern for text with `*` and `?`, matched whole and without case; `~` escapes them.
fn wildcard(value: &Value) -> Option<regex::Regex> {
    let Value::Text(text) = value else { return None };
    if !text.contains(['*', '?']) {
        let escaped = regex::escape(text);
        return regex::RegexBuilder::new(&format!("^{escaped}$")).case_insensitive(true).build().ok();
    }
    let mut pattern = String::from("^");
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '~' => pattern.push_str(&regex::escape(&chars.next().map(String::from).unwrap_or_default())),
            '*' => pattern.push_str(".*"),
            '?' => pattern.push('.'),
            c => pattern.push_str(&regex::escape(&c.to_string())),
        }
    }
    pattern.push('$');
    regex::RegexBuilder::new(&pattern).case_insensitive(true).dot_matches_new_line(true).build().ok()
}

/// Rounding as Excel does it: to 15 significant digits first, then half away from zero, or
/// always up or down for ROUNDUP and ROUNDDOWN.
fn round(number: f64, digits: i32, how: &str) -> f64 {
    let scale = 10f64.powi(digits);
    match how {
        "ROUNDUP" | "ROUNDDOWN" => {
            let scaled = (number * scale * 1e9).round() / 1e9;
            let rounded = if how == "ROUNDUP" { scaled.abs().ceil() } else { scaled.abs().floor() };
            rounded.copysign(number) / scale
        }
        _ if digits >= 0 => {
            let (integer, fraction) = Decimal::new(number).fixed(digits as usize);
            let text = format!("{}{}.{}", if number < 0.0 { "-" } else { "" }, if integer.is_empty() { "0" } else { &integer }, fraction);
            text.parse().unwrap_or(number)
        }
        _ => round(number / 10f64.powi(-digits), 0, how) * 10f64.powi(-digits),
    }
}

/// Days since 1970-01-01 of a serial date.
fn days_from_serial(serial: f64, date1904: bool) -> i64 {
    let days = serial.floor() as i64;
    if date1904 {
        days - 24_107
    } else if days < 61 {
        days - 25_568
    } else {
        days - 25_569
    }
}

/// The serial date of a day since 1970-01-01; `None` before the first date Excel has.
fn serial_from_days(days: i64, date1904: bool) -> Option<f64> {
    let serial = if date1904 {
        days + 24_107
    } else if days < -25_508 {
        // Before 1900-03-01, Excel's count is one less: it has a 29 February 1900.
        days + 25_568
    } else {
        days + 25_569
    };
    (serial >= 0).then_some(serial as f64)
}

fn civil(serial: f64, date1904: bool) -> Option<(i64, i64, i64)> {
    if serial < 0.0 {
        return None;
    }
    if !date1904 && serial.floor() == 60.0 {
        return Some((1900, 2, 29));
    }
    Some(civil_from_days(days_from_serial(serial, date1904)))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::xlsx::calc::parse::parse;

    /// Cells of one sheet, by name such as `A1`.
    struct Sheet {
        cells: HashMap<(u32, u32), Value>,
        locale: DisplayLocale,
    }

    impl Cells for Sheet {
        fn sheet(&self, name: &str) -> Option<usize> {
            (name.eq_ignore_ascii_case("Data")).then_some(0)
        }
        fn value(&mut self, _: usize, row: u32, column: u32) -> Outcome<Value> {
            Ok(self.cells.get(&(row, column)).cloned().unwrap_or(Value::Empty))
        }
        fn extent(&self, _: usize) -> (u32, u32) {
            let rows = self.cells.keys().map(|(row, _)| *row).max().unwrap_or(0);
            let columns = self.cells.keys().map(|(_, column)| *column).max().unwrap_or(0);
            (rows, columns)
        }
        fn locale(&self) -> &DisplayLocale {
            &self.locale
        }
        fn date1904(&self) -> bool {
            false
        }
        fn now(&self) -> f64 {
            45_658.75
        }
    }

    fn sheet() -> Sheet {
        let mut cells = HashMap::new();
        let mut put = |name: &str, value: Value| {
            cells.insert(crate::xlsx::refs::parse_cell(name).unwrap(), value);
        };
        // A: items, B: prices, C: quantities.
        for (row, (item, price, quantity)) in [("tea", 10.0, 2.0), ("coffee", 25.5, 1.0), ("sugar", 3.0, 10.0), ("tea", 12.0, 1.0)].iter().enumerate() {
            let row = row + 1;
            put(&format!("A{row}"), Value::Text((*item).to_owned()));
            put(&format!("B{row}"), Value::Number(*price));
            put(&format!("C{row}"), Value::Number(*quantity));
        }
        put("D1", Value::Text("12".to_owned()));
        put("D2", Value::Bool(true));
        put("D3", Value::Error(DIV0.to_owned()));
        Sheet { cells, locale: DisplayLocale::default() }
    }

    fn run(formula: &str) -> Value {
        let mut cells = sheet();
        let expr = parse(formula).unwrap();
        Evaluator { cells: &mut cells, sheet: 0, row: 9, column: 9, array: None, names: 0 }.evaluate(&expr).unwrap()
    }

    fn number(formula: &str) -> f64 {
        match run(formula) {
            Value::Number(number) => number,
            other => panic!("{formula} gave {other:?}"),
        }
    }

    #[test]
    fn arithmetic_and_text() {
        assert_eq!(number("1+2*3"), 7.0);
        assert_eq!(number("-2^2"), 4.0);
        assert_eq!(number("2^3^2"), 64.0);
        assert_eq!(number("50%*B2"), 12.75);
        assert_eq!(number("D1+1"), 13.0);
        assert_eq!(number("D2+1"), 2.0);
        assert_eq!(run("1/0"), error(DIV0));
        assert_eq!(run("D3+1"), error(DIV0));
        assert_eq!(run("A1&\" \"&B2"), Value::Text("tea 25.5".to_owned()));
        assert_eq!(run("A1+1"), error(VALUE));
        assert_eq!(run("A1=\"TEA\""), Value::Bool(true));
        assert_eq!(run("B1>B2"), Value::Bool(false));
        assert_eq!(run("E1=0"), Value::Bool(true));
        assert_eq!(run("Data!B1"), Value::Number(10.0));
        assert_eq!(run("Missing!B1"), error(REF));
    }

    #[test]
    fn aggregates_skip_text_in_cells() {
        assert_eq!(number("SUM(B1:B4)"), 50.5);
        assert_eq!(number("SUM(B:B)"), 50.5);
        assert_eq!(number("SUM(A1:B4)"), 50.5);
        assert_eq!(number("SUM(1,\"2\",TRUE)"), 4.0);
        assert_eq!(number("AVERAGE(B1:B4)"), 12.625);
        assert_eq!(number("COUNT(A1:D4)"), 8.0);
        assert_eq!(number("COUNTA(A1:A10)"), 4.0);
        assert_eq!(number("MAX(B1:B4)"), 25.5);
        assert_eq!(number("MIN(B1:C4)"), 1.0);
        assert_eq!(run("SUM(D1:D3)"), error(DIV0));
        assert_eq!(number("SUMPRODUCT(B1:B4,C1:C4)"), 10.0 * 2.0 + 25.5 + 30.0 + 12.0);
    }

    #[test]
    fn conditions_and_lookups() {
        assert_eq!(number("SUMIF(A1:A4,\"tea\",B1:B4)"), 22.0);
        assert_eq!(number("SUMIF(B1:B4,\">10\")"), 37.5);
        assert_eq!(number("COUNTIF(A1:A4,\"t*\")"), 2.0);
        assert_eq!(number("COUNTIF(A1:A4,\"<>tea\")"), 2.0);
        assert_eq!(number("COUNTIFS(A1:A4,\"tea\",C1:C4,\">1\")"), 1.0);
        assert_eq!(number("SUMIFS(B1:B4,A1:A4,\"tea\",C1:C4,1)"), 12.0);
        assert_eq!(run("IF(B1>5,\"big\",\"small\")"), Value::Text("big".to_owned()));
        assert_eq!(run("IF(B1>50,\"big\")"), Value::Bool(false));
        assert_eq!(run("IFERROR(1/0,\"none\")"), Value::Text("none".to_owned()));
        assert_eq!(run("AND(B1>5,C1>1)"), Value::Bool(true));
        assert_eq!(number("VLOOKUP(\"sugar\",A1:C4,2,FALSE)"), 3.0);
        assert_eq!(run("VLOOKUP(\"salt\",A1:C4,2,0)"), error(NA));
        assert_eq!(number("INDEX(B1:B4,MATCH(\"coffee\",A1:A4,0))"), 25.5);
        assert_eq!(number("_xlfn.XLOOKUP(\"sugar\",A1:A4,C1:C4)"), 10.0);
        assert_eq!(run("_xlfn.XLOOKUP(\"salt\",A1:A4,C1:C4,\"-\")"), Value::Text("-".to_owned()));
        assert_eq!(number("MATCH(11,B1:B1,1)"), 1.0);
    }

    #[test]
    fn rounding_and_numbers() {
        assert_eq!(number("ROUND(2.675,2)"), 2.68);
        assert_eq!(number("ROUND(-2.5,0)"), -3.0);
        assert_eq!(number("ROUND(1234,-2)"), 1200.0);
        assert_eq!(number("ROUNDUP(1.21,1)"), 1.3);
        assert_eq!(number("ROUNDDOWN(-1.29,1)"), -1.2);
        assert_eq!(number("INT(-1.5)"), -2.0);
        assert_eq!(number("MOD(-3,2)"), 1.0);
        assert_eq!(run("SQRT(-1)"), error(NUM));
    }

    #[test]
    fn text_functions() {
        assert_eq!(run("LEFT(\"Привет\",3)"), Value::Text("При".to_owned()));
        assert_eq!(run("MID(\"abcdef\",2,3)"), Value::Text("bcd".to_owned()));
        assert_eq!(number("LEN(\"Привет\")"), 6.0);
        assert_eq!(run("PROPER(\"иван петров\")"), Value::Text("Иван Петров".to_owned()));
        assert_eq!(run("TRIM(\"  a   b \")"), Value::Text("a b".to_owned()));
        assert_eq!(run("SUBSTITUTE(\"a-b-c\",\"-\",\"+\",2)"), Value::Text("a-b+c".to_owned()));
        assert_eq!(number("SEARCH(\"B\",\"abc\")"), 2.0);
        assert_eq!(run("TEXT(0.256,\"0.0%\")"), Value::Text("25.6%".to_owned()));
        assert_eq!(run("TEXT(45658,\"dd.mm.yyyy\")"), Value::Text("01.01.2025".to_owned()));
        assert_eq!(run("CONCATENATE(A1,\"-\",C1)"), Value::Text("tea-2".to_owned()));
        assert_eq!(number("VALUE(\"1 250,5\")"), 1250.5);
    }

    #[test]
    fn dates() {
        assert_eq!(number("DATE(2025,1,1)"), 45658.0);
        assert_eq!(number("DATE(2024,13,1)"), 45658.0);
        assert_eq!(number("DATE(1900,3,1)"), 61.0);
        assert_eq!(number("YEAR(45658)") * 10000.0 + number("MONTH(45658)") * 100.0 + number("DAY(45658)"), 20250101.0);
        assert_eq!(number("WEEKDAY(45658)"), 4.0);
        assert_eq!(number("EOMONTH(45658,1)"), 45716.0);
        assert_eq!(number("EDATE(DATE(2025,1,31),1)"), 45716.0);
        assert_eq!(number("TODAY()"), 45658.0);
    }

    #[test]
    fn unknown_functions_and_names_are_not_computed() {
        let mut cells = sheet();
        for formula in ["NEWFUNCTION(1)", "Rate*2", "SUM({1,2})"] {
            let expr = parse(formula).unwrap();
            assert_eq!(Evaluator { cells: &mut cells, sheet: 0, row: 0, column: 0, array: None, names: 0 }.evaluate(&expr), Err(Unsupported), "{formula}");
        }
    }
}
