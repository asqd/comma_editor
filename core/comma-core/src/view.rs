//! Sorting and filtering. A view only decides which rows are shown and in what order;
//! the file itself does not change.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};

use rayon::prelude::*;
use regex::{Regex, RegexBuilder};

use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SortKey {
    pub column: usize,
    pub ascending: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterOp {
    Contains,
    NotContains,
    Equals,
    NotEquals,
    StartsWith,
    EndsWith,
    /// Regular expression.
    Matches,
    IsEmpty,
    IsNotEmpty,
    GreaterThan,
    LessThan,
}

/// One condition. With `column: None` it applies to any column: a row passes if some value
/// satisfies the condition (for negated operators: if no value satisfies the positive one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub column: Option<usize>,
    pub op: FilterOp,
    pub value: String,
    pub case_sensitive: bool,
}

/// Rows pass when they satisfy every filter; then they are ordered by the sort keys in turn.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewSpec {
    pub sort: Vec<SortKey>,
    pub filters: Vec<Filter>,
}

impl ViewSpec {
    pub fn is_empty(&self) -> bool {
        self.sort.is_empty() && self.filters.is_empty()
    }
}

#[derive(Clone)]
pub(crate) struct CompiledFilter {
    filter: Filter,
    needle: String,
    regex: Option<Regex>,
}

impl CompiledFilter {
    pub fn new(filter: &Filter) -> Result<Self> {
        let regex = match filter.op {
            FilterOp::Matches => Some(
                RegexBuilder::new(&filter.value)
                    .case_insensitive(!filter.case_sensitive)
                    .build()
                    .map_err(|error| Error::InvalidPattern(error.to_string()))?,
            ),
            _ => None,
        };
        Ok(Self {
            needle: fold(&filter.value, filter.case_sensitive).into_owned(),
            filter: filter.clone(),
            regex,
        })
    }

    pub fn column(&self) -> Option<usize> {
        self.filter.column
    }

    /// The row passes when `test` holds for the column (or some column) XOR this is true.
    pub fn negated(&self) -> bool {
        matches!(
            self.filter.op,
            FilterOp::NotContains | FilterOp::NotEquals | FilterOp::IsNotEmpty
        )
    }

    /// Whether one value satisfies the positive form of the condition.
    pub fn test(&self, value: &str) -> bool {
        use FilterOp::*;
        match self.filter.op {
            IsEmpty | IsNotEmpty => value.trim().is_empty(),
            Matches => self.regex.as_ref().is_some_and(|regex| regex.is_match(value)),
            GreaterThan | LessThan => {
                if value.trim().is_empty() {
                    return false;
                }
                let wanted = if self.filter.op == GreaterThan {
                    Ordering::Greater
                } else {
                    Ordering::Less
                };
                compare_values(value, &self.filter.value) == wanted
            }
            Contains | NotContains | Equals | NotEquals | StartsWith | EndsWith => {
                let value = fold(value, self.filter.case_sensitive);
                let needle = self.needle.as_str();
                match self.filter.op {
                    Contains | NotContains => value.contains(needle),
                    Equals | NotEquals => value == needle,
                    StartsWith => value.starts_with(needle),
                    _ => value.ends_with(needle),
                }
            }
        }
    }
}

fn fold(value: &str, case_sensitive: bool) -> Cow<'_, str> {
    if case_sensitive {
        Cow::Borrowed(value)
    } else {
        Cow::Owned(value.to_lowercase())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Number,
    Date,
    Text,
}

enum Key {
    Empty,
    Number(f64),
    Date(i64),
    Text(String),
}

/// Stable sort of `rows` (record indices) by `keys`. Each column is compared as numbers if all
/// its values are numbers, as dates if all are dates, and as text otherwise. Empty values go last
/// in both directions. Returns `false`, leaving `rows` as they were, when `cancel` is set
/// while the values are read. Values are read and sorted on all cores.
pub(crate) fn sort_rows<'a>(
    rows: &mut Vec<usize>,
    keys: &[SortKey],
    value: impl Fn(usize, usize) -> Cow<'a, str> + Sync,
    cancel: &AtomicBool,
) -> bool {
    let value = &value;
    let mut columns: Vec<(Vec<Key>, bool)> = Vec::with_capacity(keys.len());
    for key in keys {
        let values: Vec<Cow<str>> = rows
            .par_chunks(CANCEL_CHECK)
            .flat_map_iter(|chunk| {
                let cancelled = cancel.load(AtomicOrdering::Relaxed);
                chunk.iter().map(move |&row| if cancelled { Cow::Borrowed("") } else { value(row, key.column) })
            })
            .collect();
        if cancel.load(AtomicOrdering::Relaxed) {
            return false;
        }
        let kind = infer_kind(&values);
        let keys = values.par_iter().map(|v| make_key(v, kind)).collect();
        columns.push((keys, key.ascending));
    }
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.par_sort_by(|&a, &b| {
        columns
            .iter()
            .map(|(keys, ascending)| compare_keys(&keys[a], &keys[b], *ascending))
            .find(|ordering| ordering.is_ne())
            .unwrap_or(Ordering::Equal)
    });
    *rows = order.into_iter().map(|index| rows[index]).collect();
    true
}

/// How many rows long work goes between looks at its cancel flag.
pub(crate) const CANCEL_CHECK: usize = 1 << 16;

fn infer_kind(values: &[Cow<'_, str>]) -> Kind {
    let filled = |v: &&Cow<'_, str>| !v.trim().is_empty();
    if !values.par_iter().any(|v| filled(&v)) {
        return Kind::Text;
    }
    if values.par_iter().filter(filled).all(|v| parse_amount(v).is_some()) {
        Kind::Number
    } else if values.par_iter().filter(filled).all(|v| parse_date(v).is_some()) {
        Kind::Date
    } else {
        Kind::Text
    }
}

fn make_key(value: &str, kind: Kind) -> Key {
    if value.trim().is_empty() {
        return Key::Empty;
    }
    match kind {
        Kind::Number => parse_amount(value).map_or(Key::Empty, Key::Number),
        Kind::Date => parse_date(value).map_or(Key::Empty, Key::Date),
        Kind::Text => Key::Text(text_key(value)),
    }
}

fn compare_keys(a: &Key, b: &Key, ascending: bool) -> Ordering {
    let ordering = match (a, b) {
        (Key::Empty, Key::Empty) => return Ordering::Equal,
        (Key::Empty, _) => return Ordering::Greater,
        (_, Key::Empty) => return Ordering::Less,
        (Key::Number(a), Key::Number(b)) => a.total_cmp(b),
        (Key::Date(a), Key::Date(b)) => a.cmp(b),
        (Key::Text(a), Key::Text(b)) => natural_cmp(a, b),
        _ => Ordering::Equal,
    };
    if ascending { ordering } else { ordering.reverse() }
}

/// Compares two values the way sorting would: as numbers, then dates, then text.
pub(crate) fn compare_values(a: &str, b: &str) -> Ordering {
    if let (Some(a), Some(b)) = (parse_amount(a), parse_amount(b)) {
        return a.total_cmp(&b);
    }
    if let (Some(a), Some(b)) = (parse_date(a), parse_date(b)) {
        return a.cmp(&b);
    }
    natural_cmp(&text_key(a), &text_key(b))
}

/// Case-insensitive text with ё sorted as е.
fn text_key(value: &str) -> String {
    value.trim().to_lowercase().replace('ё', "е")
}

/// Numbers as people write them: `1250.50`, `1 250,50`, `1,250.50`, `-3`, `12%`, `1e5`.
pub(crate) fn parse_number(value: &str) -> Option<f64> {
    let value = value.trim();
    if !value.bytes().any(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut number: String = value
        .chars()
        .filter(|c| !matches!(c, ' ' | '\u{a0}' | '\u{202f}' | '_' | '\''))
        .collect();
    if number.ends_with('%') {
        number.pop();
    }
    let commas = number.matches(',').count();
    if commas > 0 {
        number = match (number.rfind('.'), number.rfind(',')) {
            // Both separators: the last one is the decimal point.
            (Some(dot), Some(comma)) if comma > dot => number.replace('.', "").replace(',', "."),
            (Some(_), _) => number.replace(',', ""),
            _ if commas == 1 => number.replace(',', "."),
            _ => number.replace(',', ""),
        };
    }
    let plain = number
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+' | b'e' | b'E'));
    if !plain {
        return None;
    }
    number.parse().ok()
}

/// Amounts as sorting and comparing read them: numbers, also with a currency sign or word
/// before or after them (`1 250,50 ₽`, `$1,250.50`, `1250 руб.`), negative in parentheses as
/// accounting formats show them, and `-` standing for zero there (`- ₽`).
pub(crate) fn parse_amount(value: &str) -> Option<f64> {
    if let Some(number) = parse_number(value) {
        return Some(number);
    }
    let mut text = value.trim();
    let negative = match text.strip_prefix('(').and_then(|inner| inner.strip_suffix(')')) {
        Some(inner) => {
            text = inner.trim();
            true
        }
        None => false,
    };
    let bare = strip_currency(text).unwrap_or(text).trim();
    let number = match bare {
        "-" if bare.len() < text.len() => 0.0,
        _ => parse_number(bare)?,
    };
    Some(if negative { -number } else { number })
}

/// Currency signs and words, lower case, longer first.
const CURRENCIES: &[&str] = &[
    "руб.", "руб", "грн.", "грн", "тг.", "тг", "р.", "rub", "usd", "eur", "₽", "$", "€", "£", "¥", "₴", "₸", "₺", "₹",
];

/// The text without a currency sign or word at its start or end.
fn strip_currency(text: &str) -> Option<&str> {
    for currency in CURRENCIES {
        if let Some(start) = text.len().checked_sub(currency.len())
            && text.is_char_boundary(start)
            && text[start..].to_lowercase() == *currency
        {
            return Some(&text[..start]);
        }
        if text.len() >= currency.len() && text.is_char_boundary(currency.len()) && text[..currency.len()].to_lowercase() == *currency {
            return Some(&text[currency.len()..]);
        }
    }
    None
}

/// `31.12.2024` or `31.12.2024 23:59[:59]`, as a sortable number. ISO dates already sort
/// correctly as text.
pub(crate) fn parse_date(value: &str) -> Option<i64> {
    let value = value.trim();
    let (date, time) = match value.split_once(' ') {
        Some((date, time)) => (date, Some(time.trim())),
        None => (value, None),
    };
    let mut parts = date.split('.');
    let (day, month, year) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || year.len() != 4 {
        return None;
    }
    let number = |text: &str| -> Option<i64> {
        (!text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())).then(|| text.parse().ok())?
    };
    let (day, month, year) = (number(day)?, number(month)?, number(year)?);
    if !(1..=31).contains(&day) || !(1..=12).contains(&month) {
        return None;
    }
    let (mut hour, mut minute, mut second) = (0, 0, 0);
    if let Some(time) = time {
        let mut parts = time.split(':');
        hour = number(parts.next()?)?;
        minute = number(parts.next()?)?;
        second = parts.next().map_or(Some(0), number)?;
        if parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
            return None;
        }
    }
    Some((((year * 100 + month) * 100 + day) * 100 + hour) * 10_000 + minute * 100 + second)
}

/// Compares runs of digits by numeric value, so `file2` < `file10`.
pub(crate) fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a, b);
    loop {
        match (a.chars().next(), b.chars().next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let digits = |s: &str| s.len() - s.trim_start_matches(|c: char| c.is_ascii_digit()).len();
                let (run_a, rest_a) = a.split_at(digits(a));
                let (run_b, rest_b) = b.split_at(digits(b));
                let (value_a, value_b) = (run_a.trim_start_matches('0'), run_b.trim_start_matches('0'));
                let ordering = value_a
                    .len()
                    .cmp(&value_b.len())
                    .then_with(|| value_a.cmp(value_b))
                    .then_with(|| run_a.len().cmp(&run_b.len()));
                if ordering.is_ne() {
                    return ordering;
                }
                (a, b) = (rest_a, rest_b);
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                (a, b) = (&a[x.len_utf8()..], &b[y.len_utf8()..]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_numbers_as_written() {
        assert_eq!(parse_amount("1\u{a0}120,00 ₽"), Some(1120.0));
        assert_eq!(parse_amount("12\u{202f}345,00 руб."), Some(12345.0));
        assert_eq!(parse_amount("$1,250.50"), Some(1250.5));
        assert_eq!(parse_amount("(1 500,00 ₽)"), Some(-1500.0));
        assert_eq!(parse_amount("- ₽"), Some(0.0));
        assert_eq!(parse_amount("-"), None);
        assert_eq!(parse_amount("₽"), None);
        assert_eq!(parse_amount("рубль"), None);
        assert_eq!(parse_number("1250.50"), Some(1250.5));
        assert_eq!(parse_number("1 250,50"), Some(1250.5));
        assert_eq!(parse_number("1,250.50"), Some(1250.5));
        assert_eq!(parse_number("1.250,50"), Some(1250.5));
        assert_eq!(parse_number("1,234,567"), Some(1234567.0));
        assert_eq!(parse_number("-3"), Some(-3.0));
        assert_eq!(parse_number("12%"), Some(12.0));
        assert_eq!(parse_number("2024-01-15"), None);
        assert_eq!(parse_number("inf"), None);
        assert_eq!(parse_number("Nan"), None);
        assert_eq!(parse_number("+7 999 123-45-67"), None);
    }

    #[test]
    fn parses_day_first_dates() {
        assert!(parse_date("01.02.2024") < parse_date("31.01.2025"));
        assert!(parse_date("1.2.2024 09:30") < parse_date("1.2.2024 10:00:01"));
        assert_eq!(parse_date("32.01.2024"), None);
        assert_eq!(parse_date("1.2.24"), None);
    }

    #[test]
    fn natural_order() {
        let mut values = vec!["file10", "file2", "File1", "file02", "apple"];
        values.sort_by(|a, b| natural_cmp(&text_key(a), &text_key(b)));
        assert_eq!(values, vec!["apple", "File1", "file2", "file02", "file10"]);
        assert_eq!(natural_cmp(&text_key("Ёлка"), &text_key("яблоко")), Ordering::Less);
    }

    fn sorted(values: &[&str], ascending: bool) -> Vec<String> {
        let mut rows: Vec<usize> = (0..values.len()).collect();
        sort_rows(
            &mut rows,
            &[SortKey { column: 0, ascending }],
            |row, _| Cow::Borrowed(values[row]),
            &AtomicBool::new(false),
        );
        rows.into_iter().map(|row| values[row].to_owned()).collect()
    }

    #[test]
    fn sorts_by_inferred_type_with_empty_values_last() {
        assert_eq!(sorted(&["10", "", "9", "1 000,5"], true), vec!["9", "10", "1 000,5", ""]);
        assert_eq!(sorted(&["10", "", "9"], false), vec!["10", "9", ""]);
        assert_eq!(
            sorted(&["02.01.2024", "31.12.2023", "15.06.2024"], true),
            vec!["31.12.2023", "02.01.2024", "15.06.2024"]
        );
        assert_eq!(sorted(&["b", "a10", "a9", "A1"], true), vec!["A1", "a9", "a10", "b"]);
    }

    #[test]
    fn sorts_amounts_with_currency_and_group_separators() {
        let amounts = ["987,60 ₽", "1\u{a0}120,00 ₽", "54,00 ₽", "12\u{202f}345,00 ₽", "- ₽"];
        assert_eq!(sorted(&amounts, false), vec!["12\u{202f}345,00 ₽", "1\u{a0}120,00 ₽", "987,60 ₽", "54,00 ₽", "- ₽"]);
    }

    #[test]
    fn filters() {
        let filter = |op, value: &str| {
            CompiledFilter::new(&Filter {
                column: Some(0),
                op,
                value: value.to_owned(),
                case_sensitive: false,
            })
            .unwrap()
        };
        assert!(filter(FilterOp::Contains, "моск").test("Москва"));
        assert!(filter(FilterOp::Equals, "LONDON").test("london"));
        assert!(filter(FilterOp::StartsWith, "new").test("New York"));
        assert!(filter(FilterOp::Matches, r"^\d{3}$").test("042"));
        assert!(filter(FilterOp::GreaterThan, "100").test("1 250,50"));
        assert!(filter(FilterOp::GreaterThan, "1000").test("1\u{a0}120,00 ₽"));
        assert!(!filter(FilterOp::GreaterThan, "100").test(""));
        assert!(filter(FilterOp::LessThan, "01.01.2025").test("31.12.2024"));
        assert!(filter(FilterOp::IsEmpty, "").test("  "));
        assert!(filter(FilterOp::NotContains, "x").negated());
        assert!(CompiledFilter::new(&Filter {
            column: None,
            op: FilterOp::Matches,
            value: "(".into(),
            case_sensitive: false
        })
        .is_err());
    }
}
