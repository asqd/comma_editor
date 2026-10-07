//! Repair of broken rows: rows with another number of fields than the rest of the file, and
//! quotes that swallowed line breaks. Finds them, suggests fixes with a preview, and applies
//! fixes as one undoable step.
//!
//! The expected number of fields is the most common one. Problems are reported by row of the
//! file, the header row included.

use std::sync::Arc;
use std::borrow::Cow;
use std::collections::HashMap;

use memchr::memchr_iter;

use crate::document::{Document, Field, Record};
use crate::error::Result;
use crate::history::Splice;
use crate::parser::{self, QuotedSpan, Terminator};
use crate::profile::{ValueKind, classify};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProblemKind {
    /// A quote opens a value and never closes: the rest of the file became one value.
    UnclosedQuote,
    /// A quoted value runs over line breaks and swallowed what look like whole rows.
    StrayQuote,
    /// Fewer fields than most rows, blank lines included.
    MissingFields,
    /// More fields than most rows.
    ExtraFields,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub row: usize,
    pub kind: ProblemKind,
    pub fields: usize,
    /// Lines of the file the row takes.
    pub lines: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepairReport {
    /// The number of fields most rows have.
    pub expected_fields: usize,
    /// Problems in file order, at most the number asked for.
    pub problems: Vec<Problem>,
    /// Number of problems of each kind, all of them, in the order of `ProblemKind`.
    pub counts: Vec<(ProblemKind, usize)>,
    /// Values in the first rows that look like text read in the wrong encoding.
    pub garbled_sample: usize,
}

impl RepairReport {
    pub fn total(&self) -> usize {
        self.counts.iter().map(|(_, count)| count).sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fix {
    /// Keep the quote as an ordinary character; the swallowed lines become rows again.
    QuoteAsText,
    /// Remove the quote; the swallowed lines become rows again.
    RemoveQuote,
    /// Add empty fields at the end.
    PadFields,
    /// Join with the next row: a line break inside a value split the row in two.
    JoinNext,
    /// Merge the extra fields into the value of `column`, with the delimiter between them.
    MergeFields { column: usize },
    /// Remove the empty fields past the expected number.
    DropEmptyFields,
    DeleteRow,
}

/// A fix with the rows it would produce.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepairOption {
    pub fix: Fix,
    /// The first rows the fix produces.
    pub rows: Vec<Vec<String>>,
    /// How many rows the fix produces in all.
    pub row_count: usize,
}

const PREVIEW_ROWS: usize = 5;
const MERGE_OPTIONS: usize = 3;
const KIND_SAMPLE: usize = 2_000;

/// Field counts and suspicious quotes of every record.
struct Survey {
    fields: Vec<u32>,
    quoted: HashMap<usize, QuotedSpan>,
    expected: usize,
}

impl Document {
    /// Finds broken rows; lists at most `limit` of them and counts all.
    pub fn find_problems(&self, limit: usize) -> RepairReport {
        let survey = self.survey();
        let mut counts: HashMap<ProblemKind, usize> = HashMap::new();
        let mut problems = Vec::new();
        for row in 0..self.records.len() {
            if let Some(kind) = self.problem_kind(&survey, row) {
                *counts.entry(kind).or_default() += 1;
                if problems.len() < limit {
                    problems.push(Problem {
                        row,
                        kind,
                        fields: survey.fields[row] as usize,
                        lines: self.line_count(row),
                    });
                }
            }
        }
        let counts = [
            ProblemKind::UnclosedQuote,
            ProblemKind::StrayQuote,
            ProblemKind::MissingFields,
            ProblemKind::ExtraFields,
        ]
        .into_iter()
        .filter_map(|kind| counts.get(&kind).map(|&count| (kind, count)))
        .collect();
        RepairReport {
            expected_fields: survey.expected,
            problems,
            counts,
            // When the file's own encoding is a guess, odd text means a wrong guess, which
            // reopening fixes; reading values in yet another encoding would only make it worse.
            garbled_sample: if self.info().detection_confident { self.garbled_in_sample() } else { 0 },
        }
    }

    /// Fixes for the problem in `row`, the recommended one first, each with a preview.
    /// Empty if the row is fine.
    pub fn repair_options(&self, row: usize) -> Result<Vec<RepairOption>> {
        self.check_row(row)?;
        let survey = self.survey();
        let fixes = self.fixes(&survey, row, &mut None);
        Ok(fixes
            .into_iter()
            .map(|fix| {
                let (_, records, _) = self.fixed_records(&survey, row, fix);
                RepairOption {
                    fix,
                    rows: records
                        .iter()
                        .take(PREVIEW_ROWS)
                        .map(|record| self.record_values(record).into_iter().map(Cow::into_owned).collect())
                        .collect(),
                    row_count: records.len(),
                }
            })
            .collect())
    }

    /// Applies one fix to the problem in `row`. Returns `false` if the fix does not apply.
    pub fn apply_fix(&mut self, row: usize, fix: Fix) -> Result<bool> {
        self.check_row(row)?;
        let survey = self.survey();
        if !self.fixes(&survey, row, &mut None).contains(&fix) {
            return Ok(false);
        }
        Ok(self.apply_fixes(&survey, vec![(row, fix)]) > 0)
    }

    /// Applies the recommended fix to every problem of one kind as one step.
    /// Returns the number of rows fixed.
    pub fn fix_all(&mut self, kind: ProblemKind) -> usize {
        let survey = self.survey();
        let mut kinds = None;
        let fixes = (0..self.records.len())
            .filter(|&row| self.problem_kind(&survey, row) == Some(kind))
            .filter_map(|row| self.fixes(&survey, row, &mut kinds).first().map(|&fix| (row, fix)))
            .collect();
        self.apply_fixes(&survey, fixes)
    }

    /// The row as it is written in the file, cut to `max_chars` characters.
    pub fn raw_text(&self, row: usize, max_chars: usize) -> Result<String> {
        self.check_row(row)?;
        let text = match &self.records[row] {
            Record::Original { start, end, .. } => {
                // Enough bytes for `max_chars` characters in any supported encoding.
                let end = (*end).min(start + max_chars.saturating_mul(4).saturating_add(4));
                self.buffer_encoding().decode(&self.buf[*start..end]).into_owned()
            }
            Record::Modified(_) => self.values(row).join(&(self.dialect.delimiter as char).to_string()),
        };
        Ok(match text.char_indices().nth(max_chars) {
            Some((index, _)) => format!("{}…", &text[..index]),
            None => text,
        })
    }

    // MARK: Finding problems

    fn survey(&self) -> Survey {
        let (delimiter, quote) = (self.dialect.delimiter, self.dialect.quote);
        let mut fields = Vec::with_capacity(self.records.len());
        let mut quoted = HashMap::new();
        let mut histogram: HashMap<u32, usize> = HashMap::new();
        for (row, record) in self.records.iter().enumerate() {
            let count = match record {
                Record::Original { start, end, .. } => {
                    let (count, span) = parser::shape(&self.buf[*start..*end], delimiter, quote);
                    if let Some(span) = span {
                        quoted.insert(row, span);
                    }
                    count
                }
                Record::Modified(modified) => modified.fields.len(),
            } as u32;
            fields.push(count);
            if !record.is_blank() {
                *histogram.entry(count).or_default() += 1;
            }
        }
        let expected = histogram
            .into_iter()
            .max_by_key(|&(count, rows)| (rows, count))
            .map_or(0, |(count, _)| count as usize);
        Survey { fields, quoted, expected }
    }

    fn problem_kind(&self, survey: &Survey, row: usize) -> Option<ProblemKind> {
        if let Some((kind, _)) = self.quote_problem(survey, row) {
            return Some(kind);
        }
        let fields = survey.fields[row] as usize;
        if self.records[row].is_blank() {
            return (survey.expected > 1).then_some(ProblemKind::MissingFields);
        }
        match fields.cmp(&survey.expected) {
            std::cmp::Ordering::Less => Some(ProblemKind::MissingFields),
            std::cmp::Ordering::Greater => Some(ProblemKind::ExtraFields),
            std::cmp::Ordering::Equal => None,
        }
    }

    /// A quote that broke the row, and its position in the file. A quote that never closes is
    /// always one. Any other quote that opens a value is a stray one when reading it as an
    /// ordinary character gives rows of the expected width: a value over several lines that
    /// swallowed whole rows, or fields of one row merged into a single value.
    fn quote_problem(&self, survey: &Survey, row: usize) -> Option<(ProblemKind, usize)> {
        const CANDIDATES: usize = 8;
        let Record::Original { start, end, .. } = self.records[row] else {
            return None;
        };
        let span = survey.quoted.get(&row);
        if let Some(span) = span.filter(|span| !span.closed) {
            return Some((ProblemKind::UnclosedQuote, start + span.at));
        }
        let expected = survey.expected;
        if expected < 2 {
            return None;
        }
        let fits = |at: usize| {
            parser::scan_literal(&self.buf[..end], start, self.dialect.delimiter, self.dialect.quote, at)
                .iter()
                .all(|span| span.fields as usize == expected)
        };
        if let Some(span) = span
            && fits(start + span.at)
        {
            return Some((ProblemKind::StrayQuote, start + span.at));
        }
        if survey.fields[row] as usize == expected {
            return None;
        }
        let quote = self.dialect.quote;
        parser::split_fields(&self.buf[start..end], self.dialect.delimiter, quote)
            .into_iter()
            .filter(|&(s, _)| self.buf[start + s] == quote)
            .take(CANDIDATES)
            .map(|(s, _)| start + s)
            .find(|&at| fits(at))
            .map(|at| (ProblemKind::StrayQuote, at))
    }

    fn line_count(&self, row: usize) -> usize {
        match self.records[row] {
            Record::Original { start, end, .. } => {
                // A record that ran to the end of the file may end with a line break.
                let bytes = self.buf[start..end].trim_ascii_end();
                let lf = memchr_iter(b'\n', bytes).count();
                1 + if lf > 0 { lf } else { memchr_iter(b'\r', bytes).count() }
            }
            Record::Modified(_) => 1,
        }
    }

    // MARK: Fixes

    /// Fixes that apply to the problem in `row`, the recommended one first. `kinds` caches the
    /// kinds of values in each column, which rank merges.
    fn fixes(&self, survey: &Survey, row: usize, kinds: &mut Option<Vec<KindShares>>) -> Vec<Fix> {
        let Some(kind) = self.problem_kind(survey, row) else {
            return Vec::new();
        };
        let expected = survey.expected;
        let fields = survey.fields[row] as usize;
        match kind {
            ProblemKind::UnclosedQuote | ProblemKind::StrayQuote => vec![Fix::QuoteAsText, Fix::RemoveQuote],
            ProblemKind::MissingFields if self.records[row].is_blank() => vec![Fix::DeleteRow, Fix::PadFields],
            ProblemKind::MissingFields => {
                let joined = survey
                    .fields
                    .get(row + 1)
                    .map(|&next| next as usize)
                    .filter(|&next| next < expected && !self.records[row + 1].is_blank())
                    .map(|next| fields + next - 1)
                    .filter(|&joined| joined <= expected);
                match joined {
                    Some(joined) if joined == expected => vec![Fix::JoinNext, Fix::PadFields, Fix::DeleteRow],
                    Some(_) => vec![Fix::PadFields, Fix::JoinNext, Fix::DeleteRow],
                    None => vec![Fix::PadFields, Fix::DeleteRow],
                }
            }
            ProblemKind::ExtraFields => {
                let values = self.values(row);
                let extra_empty = values[expected..].iter().all(|value| value.is_empty());
                let kinds = kinds.get_or_insert_with(|| self.column_kinds(survey));
                let delimiter = (self.dialect.delimiter as char).to_string();
                let mut merges: Vec<(usize, f64)> = (0..expected)
                    .map(|column| (column, merge_score(&values, column, fields - expected, kinds, &delimiter)))
                    .collect();
                merges.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
                let mut fixes = Vec::new();
                if extra_empty {
                    fixes.push(Fix::DropEmptyFields);
                }
                fixes.extend(
                    merges
                        .into_iter()
                        .take(MERGE_OPTIONS)
                        .map(|(column, _)| Fix::MergeFields { column }),
                );
                fixes.push(Fix::DeleteRow);
                fixes
            }
        }
    }

    /// Share of each kind among the filled values of every column, from a sample of rows that
    /// have the expected number of fields.
    fn column_kinds(&self, survey: &Survey) -> Vec<KindShares> {
        let expected = survey.expected;
        let mut counts: Vec<HashMap<ValueKind, usize>> = vec![HashMap::new(); expected];
        let rows = (self.header_offset()..self.records.len())
            .filter(|&row| survey.fields[row] as usize == expected && !survey.quoted.contains_key(&row))
            .take(KIND_SAMPLE);
        for row in rows {
            for (column, value) in self.values(row).iter().enumerate().take(expected) {
                let kind = classify(value);
                if kind != ValueKind::Empty {
                    *counts[column].entry(kind).or_default() += 1;
                }
            }
        }
        counts
            .into_iter()
            .map(|counts| {
                let filled: usize = counts.values().sum();
                counts
                    .into_iter()
                    .map(|(kind, count)| (kind, count as f64 / filled as f64))
                    .collect()
            })
            .collect()
    }

    /// What the fix replaces: the number of records from `row` on, the records that take their
    /// place, and whether the file then ends with a line break.
    fn fixed_records(&self, survey: &Survey, row: usize, fix: Fix) -> (usize, Vec<Record>, bool) {
        let record = &self.records[row];
        let term = record.term();
        let trailing = self.trailing_newline;
        let modified = |fields| (1, vec![Record::modified(fields, term)], trailing);
        let expected = survey.expected;
        match fix {
            Fix::QuoteAsText | Fix::RemoveQuote => self.split_at_quote(survey, row, fix == Fix::RemoveQuote),
            Fix::PadFields => {
                let mut fields = self.fields(record);
                fields.resize(expected.max(fields.len()), Field::Text(String::new()));
                modified(fields)
            }
            Fix::JoinNext => {
                let next = &self.records[row + 1];
                let mut fields = self.fields(record);
                let mut rest = self.fields(next).into_iter();
                let last = fields.pop().map(|field| self.field_value(&field).into_owned()).unwrap_or_default();
                let first = rest.next().map(|field| self.field_value(&field).into_owned()).unwrap_or_default();
                let line_break = match term {
                    Terminator::None => self.dialect.line_ending.terminator(),
                    term => term,
                };
                let line_break = std::str::from_utf8(line_break.bytes()).unwrap_or("\n");
                fields.push(Field::Text(format!("{last}{line_break}{first}")));
                fields.extend(rest);
                (2, vec![Record::modified(fields, next.term())], trailing)
            }
            Fix::MergeFields { column } => {
                let mut fields = self.fields(record);
                let extra = fields.len().saturating_sub(expected);
                let delimiter = (self.dialect.delimiter as char).to_string();
                let merged: Vec<String> = fields
                    .drain(column..=column + extra)
                    .map(|field| self.field_value(&field).into_owned())
                    .collect();
                fields.insert(column, Field::Text(merged.join(&delimiter)));
                modified(fields)
            }
            Fix::DropEmptyFields => {
                let mut fields = self.fields(record);
                while fields.len() > expected && fields.last().is_some_and(|field| self.field_value(field).is_empty()) {
                    fields.pop();
                }
                modified(fields)
            }
            Fix::DeleteRow => (1, Vec::new(), trailing),
        }
    }

    /// Re-reads the lines a quote swallowed with that quote as an ordinary character, or
    /// without it. Rows without the quote stay untouched bytes of the file.
    fn split_at_quote(&self, survey: &Survey, row: usize, remove: bool) -> (usize, Vec<Record>, bool) {
        let (Record::Original { start, end, term }, Some((_, at))) = (&self.records[row], self.quote_problem(survey, row))
        else {
            return (1, vec![self.records[row].clone()], self.trailing_newline);
        };
        let (start, end, term) = (*start, *end, *term);
        let (delimiter, quote) = (self.dialect.delimiter, self.dialect.quote);
        let spans = parser::scan_literal(&self.buf[..end], start, delimiter, quote, at);
        let mut trailing = self.trailing_newline;
        let last = spans.len() - 1;
        let records = spans
            .iter()
            .enumerate()
            .map(|(index, span)| {
                // The last line ends where the record ended. A record that ran to the end of the
                // file may end with a line break of its own: the file then ends with one.
                let span_term = if index == last {
                    if span.term == Terminator::None {
                        term
                    } else {
                        trailing = true;
                        span.term
                    }
                } else {
                    span.term
                };
                if !(span.start..=span.end).contains(&at) {
                    return Record::Original {
                        start: span.start,
                        end: span.end,
                        term: span_term,
                    };
                }
                let bytes = &self.buf[span.start..span.end];
                let fields = parser::split_fields_with(bytes, delimiter, quote, Some(at - span.start))
                    .into_iter()
                    .map(|(s, e)| {
                        let (s, e) = (span.start + s, span.start + e);
                        if !(s..e).contains(&at) {
                            return Field::Raw { start: s, end: e };
                        }
                        let encoding = self.buffer_encoding();
                        let text = if remove {
                            let mut text = encoding.decode(&self.buf[s..at]).into_owned();
                            text.push_str(&encoding.decode(&self.buf[at + 1..e]));
                            text
                        } else {
                            encoding.decode(&self.buf[s..e]).into_owned()
                        };
                        Field::Text(text)
                    })
                    .collect();
                Record::modified(fields, span_term)
            })
            .collect();
        (1, records, trailing)
    }

    /// Applies fixes that do not overlap as one undoable step; returns how many were applied.
    fn apply_fixes(&mut self, survey: &Survey, mut fixes: Vec<(usize, Fix)>) -> usize {
        fixes.sort_by_key(|&(row, _)| row);
        let mut planned = Vec::new();
        let mut next_free = 0;
        let mut trailing = self.trailing_newline;
        for (row, fix) in fixes {
            if row < next_free {
                continue;
            }
            let (remove, records, ends_with_break) = self.fixed_records(survey, row, fix);
            if row + remove == self.records.len() {
                trailing = ends_with_break;
            }
            next_free = row + remove;
            planned.push((row, remove, records));
        }
        if planned.is_empty() {
            return 0;
        }
        let applied = planned.len();
        // From the end, so earlier rows keep their indices.
        let splices: Vec<Splice> = planned
            .into_iter()
            .rev()
            .map(|(row, remove, records)| Splice::apply(Arc::make_mut(&mut self.records), row, remove, records))
            .collect();
        let columns_before = self.column_count();
        let columns = self.widest_record().max(1);
        self.commit_with_trailing(splices, columns, true, trailing);
        if columns == columns_before {
            self.refresh_view();
        } else {
            self.reset_view();
        }
        applied
    }

    fn widest_record(&self) -> usize {
        let (delimiter, quote) = (self.dialect.delimiter, self.dialect.quote);
        self.records
            .iter()
            .map(|record| match record {
                Record::Original { start, end, .. } => parser::shape(&self.buf[*start..*end], delimiter, quote).0,
                Record::Modified(modified) => modified.fields.len(),
            })
            .max()
            .unwrap_or(0)
    }
}

type KindShares = HashMap<ValueKind, f64>;

/// How typical the values of the row would be for their columns if the extra fields were
/// merged into `column`, summed over the columns. A merged number or date counts as one value;
/// merged text counts by its parts, so "Seoul,12" in a column of city names scores lower than
/// "Smith, John" in a column of names.
fn merge_score(values: &[Cow<'_, str>], column: usize, extra: usize, kinds: &[KindShares], delimiter: &str) -> f64 {
    let parts = &values[column..=column + extra];
    let merged = parts.join(delimiter);
    kinds
        .iter()
        .enumerate()
        .map(|(index, shares)| match index.cmp(&column) {
            std::cmp::Ordering::Less => typicality(&values[index], shares),
            std::cmp::Ordering::Greater => typicality(&values[index + extra], shares),
            std::cmp::Ordering::Equal if classify(&merged) != ValueKind::Text => typicality(&merged, shares),
            std::cmp::Ordering::Equal => {
                parts.iter().map(|part| typicality(part, shares)).sum::<f64>() / parts.len() as f64
            }
        })
        .sum()
}

/// Share of the value's kind among the values of its column. An empty value or a column without
/// a sample counts as typical; a whole number fits any column of numbers, a fraction only a
/// column that has fractions.
fn typicality(value: &str, shares: &KindShares) -> f64 {
    let share = |kind| shares.get(&kind).copied().unwrap_or(0.0);
    match classify(value) {
        ValueKind::Empty => 1.0,
        _ if shares.is_empty() => 1.0,
        ValueKind::Integer => share(ValueKind::Integer) + share(ValueKind::Decimal),
        ValueKind::Decimal if share(ValueKind::Decimal) > 0.0 => share(ValueKind::Integer) + share(ValueKind::Decimal),
        kind => share(kind),
    }
}
