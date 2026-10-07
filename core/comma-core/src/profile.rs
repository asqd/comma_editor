//! Column profiles: the type a column holds, its statistics and the values that do not fit.

use std::collections::HashMap;

use rayon::prelude::*;

use crate::document::Document;
use crate::error::{Error, Result};
use crate::view::parse_number;

/// What a single value looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueKind {
    Empty,
    Integer,
    Decimal,
    Date,
    Boolean,
    Text,
}

/// Something wrong with a value in its column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue {
    /// The value is not of the column's type, e.g. text in a column of numbers.
    Mismatch,
    /// Spaces before or after the value.
    Padding,
}

/// Statistics of one column over the visible rows.
#[derive(Clone, Debug, PartialEq)]
pub struct ColumnProfile {
    /// The type most values have (at least 80% of the non-empty ones), or `Text`.
    pub kind: ValueKind,
    pub total: usize,
    pub empty: usize,
    /// Distinct non-empty values.
    pub unique: usize,
    pub mismatched: usize,
    pub padded: usize,
    /// Smallest and largest value as written, for numbers and dates.
    pub min: Option<String>,
    pub max: Option<String>,
    pub mean: Option<f64>,
    pub median: Option<f64>,
    /// Shortest and longest value in characters.
    pub min_length: usize,
    pub max_length: usize,
    /// The most frequent values with their counts, most frequent first.
    pub top_values: Vec<(String, usize)>,
    /// Counts of numbers or dates in equal-width bins from `min` to `max`.
    pub histogram: Vec<usize>,
}

const TOP_VALUES: usize = 10;
const HISTOGRAM_BINS: usize = 20;
const DOMINANT_SHARE: f64 = 0.8;

impl ValueKind {
    /// Whether a value of this kind belongs in a column of `column` kind.
    pub fn fits(self, column: ValueKind) -> bool {
        match (self, column) {
            (Self::Empty, _) | (_, Self::Text) | (_, Self::Empty) => true,
            (Self::Integer, Self::Decimal) => true,
            (value, column) => value == column,
        }
    }
}

/// The kind of one value. Dates are `2024-01-31` (optionally with a time) or `31.01.2024`;
/// numbers are written as people write them (`1 250,50`); booleans are true/false, yes/no, да/нет.
pub fn classify(value: &str) -> ValueKind {
    let value = value.trim();
    if value.is_empty() {
        return ValueKind::Empty;
    }
    if date_value(value).is_some() {
        return ValueKind::Date;
    }
    if let Some(number) = parse_number(value) {
        let integral = !value.contains(['.', ',', 'e', 'E', '%']) || (number.fract() == 0.0 && is_grouped_integer(value));
        return if integral { ValueKind::Integer } else { ValueKind::Decimal };
    }
    let lower = value.to_lowercase();
    if matches!(lower.as_str(), "true" | "false" | "yes" | "no" | "да" | "нет") {
        return ValueKind::Boolean;
    }
    ValueKind::Text
}

/// `1,234,567`: commas only as thousands separators.
fn is_grouped_integer(value: &str) -> bool {
    let digits = value.trim_start_matches(['-', '+']);
    let mut groups = digits.split(',');
    let first = groups.next().unwrap_or_default();
    !first.is_empty()
        && first.len() <= 3
        && digits.contains(',')
        && groups.all(|group| group.len() == 3 && group.bytes().all(|b| b.is_ascii_digit()))
}

/// Days since 1970-01-01, with the time of day as a fraction.
pub(crate) fn date_value(value: &str) -> Option<f64> {
    let value = value.trim();
    let (date, time) = match value.split_once([' ', 'T']) {
        Some((date, time)) => (date, Some(time.trim().trim_end_matches('Z'))),
        None => (value, None),
    };
    let number = |text: &str| -> Option<i64> {
        (!text.is_empty() && text.len() <= 4 && text.bytes().all(|b| b.is_ascii_digit()))
            .then(|| text.parse().ok())
            .flatten()
    };
    let parts: Vec<&str> = if date.contains('-') {
        date.split('-').collect()
    } else {
        date.split('.').rev().collect()
    };
    let [year, month, day] = parts[..] else { return None };
    if year.len() != 4 {
        return None;
    }
    let (year, month, day) = (number(year)?, number(month)?, number(day)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let mut seconds = 0;
    if let Some(time) = time {
        let mut parts = time.split(':');
        let hour = number(parts.next()?)?;
        let minute = number(parts.next()?)?;
        let second = parts.next().map_or(Some(0), |s| number(s.split('.').next().unwrap_or(s)))?;
        if parts.next().is_some() || hour > 23 || minute > 59 || second > 59 {
            return None;
        }
        seconds = hour * 3600 + minute * 60 + second;
    }
    Some(days_from_civil(year, month, day) as f64 + seconds as f64 / 86_400.0)
}

/// Howard Hinnant's algorithm: days from 1970-01-01 to a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Kinds, values and lengths of a share of a column.
struct Tally {
    kinds: HashMap<ValueKind, usize>,
    counts: HashMap<String, usize>,
    empty: usize,
    padded: usize,
    min_length: usize,
    max_length: usize,
}

impl Default for Tally {
    fn default() -> Self {
        Self {
            kinds: HashMap::new(),
            counts: HashMap::new(),
            empty: 0,
            padded: 0,
            min_length: usize::MAX,
            max_length: 0,
        }
    }
}

impl Tally {
    fn add(&mut self, value: &str) {
        let kind = classify(value);
        *self.kinds.entry(kind).or_default() += 1;
        if kind == ValueKind::Empty {
            self.empty += 1;
            return;
        }
        if has_padding(value) {
            self.padded += 1;
        }
        let length = value.chars().count();
        self.min_length = self.min_length.min(length);
        self.max_length = self.max_length.max(length);
        match self.counts.get_mut(value) {
            Some(count) => *count += 1,
            None => {
                self.counts.insert(value.to_owned(), 1);
            }
        }
    }

    fn merge(mut self, mut other: Self) -> Self {
        // Fold the smaller tally into the larger one.
        if other.counts.len() > self.counts.len() {
            std::mem::swap(&mut self, &mut other);
        }
        for (kind, count) in other.kinds {
            *self.kinds.entry(kind).or_default() += count;
        }
        for (value, count) in other.counts {
            *self.counts.entry(value).or_default() += count;
        }
        self.empty += other.empty;
        self.padded += other.padded;
        self.min_length = self.min_length.min(other.min_length);
        self.max_length = self.max_length.max(other.max_length);
        self
    }
}

fn has_padding(value: &str) -> bool {
    !value.is_empty() && value.trim() != value
}

fn sort_value(value: &str, kind: ValueKind) -> Option<f64> {
    match kind {
        ValueKind::Integer | ValueKind::Decimal => parse_number(value),
        ValueKind::Date => date_value(value),
        _ => None,
    }
}

impl Document {
    /// Profiles `column` over the visible rows.
    pub fn profile_column(&self, column: usize) -> Result<ColumnProfile> {
        if column >= self.column_count() {
            return Err(Error::ColumnOutOfRange {
                column,
                columns: self.column_count(),
            });
        }
        let records: Vec<usize> = (0..self.visible_row_count())
            .filter_map(|row| self.visible_record(row))
            .collect();

        // Each core tallies a share of the rows; the tallies are then merged.
        let Tally { kinds, counts, empty, padded, min_length, max_length } = records
            .par_iter()
            .fold(Tally::default, |mut tally, &record| {
                tally.add(&self.value_at(record, column));
                tally
            })
            .reduce(Tally::default, Tally::merge);

        let filled = records.len() - empty;
        let kind = dominant_kind(&kinds, filled);
        let mismatched = kinds
            .iter()
            .filter(|&(&value_kind, _)| !value_kind.fits(kind))
            .map(|(_, &count)| count)
            .sum();

        let mut top_values: Vec<(String, usize)> = counts.iter().map(|(value, &count)| (value.clone(), count)).collect();
        top_values.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        top_values.truncate(TOP_VALUES);

        let mut profile = ColumnProfile {
            kind,
            total: records.len(),
            empty,
            unique: counts.len(),
            mismatched,
            padded,
            min: None,
            max: None,
            mean: None,
            median: None,
            min_length: if filled == 0 { 0 } else { min_length },
            max_length,
            top_values,
            histogram: Vec::new(),
        };

        // Numbers and dates: range, mean, median and histogram over the values that fit.
        let mut values: Vec<(f64, &str)> = counts
            .iter()
            .filter_map(|(value, &count)| sort_value(value, kind).map(|number| (number, value.as_str(), count)))
            .flat_map(|(number, value, count)| std::iter::repeat_n((number, value), count))
            .collect();
        if values.is_empty() {
            return Ok(profile);
        }
        values.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        let (low, high) = (values[0].0, values[values.len() - 1].0);
        profile.min = Some(values[0].1.trim().to_owned());
        profile.max = Some(values[values.len() - 1].1.trim().to_owned());
        if kind != ValueKind::Date {
            profile.mean = Some(values.iter().map(|v| v.0).sum::<f64>() / values.len() as f64);
            let middle = values.len() / 2;
            profile.median = Some(if values.len().is_multiple_of(2) {
                (values[middle - 1].0 + values[middle].0) / 2.0
            } else {
                values[middle].0
            });
        }
        let bins = if high > low { HISTOGRAM_BINS } else { 1 };
        let mut histogram = vec![0; bins];
        for (number, _) in &values {
            let position = if high > low { (number - low) / (high - low) } else { 0.0 };
            histogram[((position * bins as f64) as usize).min(bins - 1)] += 1;
        }
        profile.histogram = histogram;
        Ok(profile)
    }

    /// The next visible row after `after` (or from the top) whose value in `column` has `issue`,
    /// wrapping around.
    pub fn find_issue(&self, column: usize, issue: Issue, kind: ValueKind, after: Option<usize>) -> Option<usize> {
        let rows = self.visible_row_count();
        if rows == 0 || column >= self.column_count() {
            return None;
        }
        let start = after.map_or(0, |row| row + 1);
        (0..rows).map(|step| (start + step) % rows).find(|&row| {
            let Some(record) = self.visible_record(row) else { return false };
            let value = self.value_at(record, column);
            match issue {
                Issue::Mismatch => !classify(&value).fits(kind),
                Issue::Padding => has_padding(&value),
            }
        })
    }
}

fn dominant_kind(kinds: &HashMap<ValueKind, usize>, filled: usize) -> ValueKind {
    if filled == 0 {
        return ValueKind::Empty;
    }
    let count = |kind| kinds.get(&kind).copied().unwrap_or(0);
    let share = |n: usize| n as f64 / filled as f64;
    let numbers = count(ValueKind::Integer) + count(ValueKind::Decimal);
    if share(numbers) >= DOMINANT_SHARE {
        if count(ValueKind::Decimal) > 0 { ValueKind::Decimal } else { ValueKind::Integer }
    } else if share(count(ValueKind::Date)) >= DOMINANT_SHARE {
        ValueKind::Date
    } else if share(count(ValueKind::Boolean)) >= DOMINANT_SHARE {
        ValueKind::Boolean
    } else {
        ValueKind::Text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_values() {
        assert_eq!(classify("  "), ValueKind::Empty);
        assert_eq!(classify("42"), ValueKind::Integer);
        assert_eq!(classify("007"), ValueKind::Integer);
        assert_eq!(classify("1 250"), ValueKind::Integer);
        assert_eq!(classify("1,234,567"), ValueKind::Integer);
        assert_eq!(classify("1 250,50"), ValueKind::Decimal);
        assert_eq!(classify("-3.5"), ValueKind::Decimal);
        assert_eq!(classify("2024-02-29"), ValueKind::Date);
        assert_eq!(classify("2024-02-29T10:30:00Z"), ValueKind::Date);
        assert_eq!(classify("29.02.2024 10:30"), ValueKind::Date);
        assert_eq!(classify("Да"), ValueKind::Boolean);
        assert_eq!(classify("hello"), ValueKind::Text);
        assert_eq!(classify("2024-13-01"), ValueKind::Text);
    }

    #[test]
    fn dates_are_days_since_1970() {
        assert_eq!(date_value("1970-01-01"), Some(0.0));
        assert_eq!(date_value("02.01.1970 12:00"), Some(1.5));
        assert_eq!(date_value("2000-03-01"), Some(11_017.0));
    }

    #[test]
    fn kinds_fit_columns() {
        assert!(ValueKind::Integer.fits(ValueKind::Decimal));
        assert!(!ValueKind::Decimal.fits(ValueKind::Integer));
        assert!(!ValueKind::Text.fits(ValueKind::Date));
        assert!(ValueKind::Empty.fits(ValueKind::Integer));
        assert!(ValueKind::Date.fits(ValueKind::Text));
    }
}
