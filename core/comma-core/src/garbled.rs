//! Text that was read in the wrong encoding and saved that way: "РџСЂРёРІРµС‚" or "ÐŸÑ€Ð¸Ð²ÐµÑ‚"
//! for "Привет", "Ã©tÃ©" for "été", "Ïðèâåò" for "Привет" in Windows-1251 shown as
//! Windows-1252. The text is turned back into the bytes it came from and read correctly;
//! a fix is kept only when the result reads more naturally.

use encoding_rs::{Encoding, KOI8_R, WINDOWS_1251, WINDOWS_1252};

use crate::document::Document;
use crate::edit::{Edit, Prepared, Progress, map_rows};
use crate::error::Result;
use crate::range::CellRange;

enum Source {
    Utf8,
    SingleByte(&'static Encoding),
}

/// How text gets garbled: the encoding it was wrongly shown in, and the one it really was in.
const MISREADINGS: [(&Encoding, Source); 5] = [
    (WINDOWS_1251, Source::Utf8),
    (WINDOWS_1252, Source::Utf8),
    (WINDOWS_1252, Source::SingleByte(WINDOWS_1251)),
    (WINDOWS_1251, Source::SingleByte(KOI8_R)),
    (KOI8_R, Source::SingleByte(WINDOWS_1251)),
];

/// The text read correctly, if it looks garbled and one reading makes it clearly better.
pub fn fix_garbled(text: &str) -> Option<String> {
    if text.is_ascii() {
        return None;
    }
    let current = naturalness(text);
    let mut best: Option<(i64, String)> = None;
    for (shown_as, source) in &MISREADINGS {
        let (bytes, _, unmappable) = shown_as.encode(text);
        if unmappable {
            continue;
        }
        let fixed = match source {
            // Text that decodes as UTF-8 only by accident is very rare.
            Source::Utf8 => match String::from_utf8(bytes.into_owned()) {
                Ok(fixed) => fixed,
                Err(_) => continue,
            },
            Source::SingleByte(encoding) => encoding.decode_without_bom_handling(&bytes).0.into_owned(),
        };
        let score = naturalness(&fixed);
        // A single-byte reading always succeeds, so it must be clearly better.
        let margin = if matches!(source, Source::Utf8) { 1 } else { 3 };
        if fixed != text && score >= current + margin && best.as_ref().is_none_or(|(best, _)| score > *best) {
            best = Some((score, fixed));
        }
    }
    best.map(|(_, fixed)| fixed)
}

/// How much the text looks like ordinary writing: letters in words of one alphabet with an
/// ordinary use of capitals count for it, other letters and stray symbols against it.
fn naturalness(text: &str) -> i64 {
    let mut score = 0i64;
    let chars: Vec<char> = text.chars().collect();
    for (index, &c) in chars.iter().enumerate() {
        if is_junk(c) {
            score -= 4;
        }
        // The lead byte of a UTF-8 character shown as Latin-1, followed by its continuation
        // byte: "Ã©" for "é", "Ð¸" for "и".
        if matches!(c, 'Ã' | 'Â' | 'Ð' | 'Ñ') && chars.get(index + 1).is_some_and(|next| ('\u{80}'..='\u{BF}').contains(next)) {
            score -= 4;
        }
        // The same for Russian shown as Windows-1251: "Р§" for "Ч", "С‘" for "ё". Short words
        // have too few junk symbols to give themselves away otherwise.
        if matches!(c, 'Р' | 'С') && chars.get(index + 1).is_some_and(|&next| is_continuation_in_1251(next)) {
            score -= 4;
        }
    }
    for word in text.split(|c: char| !c.is_alphabetic()).filter(|word| !word.is_empty()) {
        score += if natural_word(word) {
            word.chars().map(letter_weight).sum()
        } else {
            -(word.chars().count() as i64)
        };
    }
    score
}

/// Common Russian letters count more than rare ones, so a misreading that turns Russian into
/// other Russian-looking letters loses: "Новосибирск" beats "Лмамъуюуоъи".
fn letter_weight(c: char) -> i64 {
    match c.to_lowercase().next().unwrap_or(c) {
        'о' | 'е' | 'а' | 'и' | 'н' | 'т' | 'с' | 'р' | 'в' | 'л' | 'к' | 'м' | 'д' | 'п' | 'у' | 'я' | 'ы' | 'ь'
        | 'г' | 'з' => 2,
        'ъ' => -2,
        '\u{400}'..='\u{4FF}' => 1,
        _ => 2,
    }
}

fn natural_word(word: &str) -> bool {
    let (mut latin, mut accented, mut cyrillic, mut other) = (0, 0, 0, 0);
    for c in word.chars() {
        match c {
            'a'..='z' | 'A'..='Z' => latin += 1,
            '\u{C0}'..='\u{24F}' if c != '\u{D7}' && c != '\u{F7}' => {
                latin += 1;
                accented += 1;
            }
            '\u{400}'..='\u{4FF}' => cyrillic += 1,
            _ => other += 1,
        }
    }
    let one_alphabet = other == 0 && (latin == 0 || cyrillic == 0);
    // Western words have an accent here and there ("été"), not on almost every letter.
    let plausible_accents = accented * 2 <= latin + 1;
    one_alphabet && plausible_accents && ordinary_case(word)
}

/// lower, Capitalized or UPPER.
fn ordinary_case(word: &str) -> bool {
    let mut chars = word.chars();
    let first = chars.next().is_none_or(char::is_uppercase);
    let rest: Vec<bool> = chars.map(char::is_uppercase).collect();
    rest.iter().all(|&upper| !upper) || (first && rest.iter().all(|&upper| upper))
}

/// Symbols that show up when bytes are read in the wrong encoding and seldom otherwise.
fn is_junk(c: char) -> bool {
    matches!(c,
        '\u{FFFD}' | '\u{80}'..='\u{9F}'
        | '‚' | '„' | '†' | '‡' | '‰' | '‹' | '›' | '˜' | 'ˆ'
        | '¤' | '¦' | '¨' | '¯' | '´' | '¸' | '¹' | '²' | '³' | '¼' | '½' | '¾' | '¿' | '¡' | '¬' | '¶' | 'µ'
        // Box drawing and other graphics of KOI8-R.
        | '\u{2500}'..='\u{25FF}' | '⌠' | '⌡' | '√' | '≈' | '≤' | '≥' | '∙' | '÷')
}

/// A symbol Windows-1251 shows for a UTF-8 continuation byte (0x80–0xBF) other than a letter:
/// `§`, `°`, `№`, `‘`, `Ђ`…
fn is_continuation_in_1251(c: char) -> bool {
    static SYMBOLS: std::sync::OnceLock<Vec<char>> = std::sync::OnceLock::new();
    let symbols = SYMBOLS.get_or_init(|| {
        let bytes: Vec<u8> = (0x80..=0xBF).collect();
        WINDOWS_1251
            .decode_without_bom_handling(&bytes)
            .0
            .chars()
            .filter(|c| !matches!(c, 'А'..='я' | 'Ё' | 'ё'))
            .collect()
    });
    symbols.contains(&c)
}

const SAMPLE_ROWS: usize = 2000;

impl Document {
    /// Values in the first rows that look garbled.
    pub(crate) fn garbled_in_sample(&self) -> usize {
        (0..self.records.len().min(SAMPLE_ROWS))
            .map(|row| self.values(row).iter().filter(|value| fix_garbled(value).is_some()).count())
            .sum()
    }

    /// Fixes garbled values in a range of visible cells, or in the whole file, header
    /// included, as one undoable step. Returns the number of fixed cells.
    pub fn fix_garbled_text(&mut self, range: Option<CellRange>) -> Result<usize> {
        Ok(self.apply_edit(&Edit::FixGarbled { range })?.count)
    }

    pub(crate) fn prepare_fix_garbled(&self, range: Option<CellRange>, progress: &Progress) -> Result<Option<Prepared>> {
        if let Some(range) = range {
            self.check_range(&range)?;
            let columns: Vec<usize> = range.columns().collect();
            return Ok(self.cell_updates(range.rows(), &columns, progress, |_, _, value| fix_garbled(value)));
        }
        let rows = 0..self.records.len();
        progress.expect(rows.len());
        let columns = 0..self.column_count();
        Ok(map_rows(rows, progress, |row| {
            self.updated_record(row, columns.clone(), |_, value| fix_garbled(value))
        })
        .map(|updates| Prepared::rows(updates, self.column_count(), None)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn garble(text: &str, real: &'static Encoding, shown_as: &'static Encoding) -> String {
        let (bytes, _, _) = real.encode(text);
        shown_as.decode_without_bom_handling(&bytes).0.into_owned()
    }

    fn utf8_shown_as(text: &str, shown_as: &'static Encoding) -> String {
        shown_as.decode_without_bom_handling(text.as_bytes()).0.into_owned()
    }

    #[test]
    fn fixes_utf8_shown_in_single_byte_encodings() {
        for text in ["Привет, мир", "Москва", "ООО «Ромашка»", "Ёлка", "Чай", "Мёд", "Сыр", "Рис"] {
            let garbled = utf8_shown_as(text, WINDOWS_1251);
            assert_eq!(fix_garbled(&garbled).as_deref(), Some(text), "{garbled}");
            let garbled = utf8_shown_as(text, WINDOWS_1252);
            assert_eq!(fix_garbled(&garbled).as_deref(), Some(text), "{garbled}");
        }
        let french = utf8_shown_as("été à Paris", WINDOWS_1252);
        assert_eq!(french, "Ã©tÃ© Ã\u{A0} Paris");
        assert_eq!(fix_garbled(&french).as_deref(), Some("été à Paris"));
    }

    #[test]
    fn fixes_single_byte_misreadings() {
        assert_eq!(fix_garbled(&garble("Привет", WINDOWS_1251, WINDOWS_1252)).as_deref(), Some("Привет"));
        assert_eq!(fix_garbled(&garble("Новосибирск", KOI8_R, WINDOWS_1251)).as_deref(), Some("Новосибирск"));
        assert_eq!(fix_garbled(&garble("Новосибирск", WINDOWS_1251, KOI8_R)).as_deref(), Some("Новосибирск"));
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        for text in [
            "Привет", "ООО", "ООО РОМАШКА", "Россия", "СССР", "Р-1", "С. Петров", "Р. №5", "Café", "Straße", "São Paulo", "naïve", "Їжак і ґанок",
            "1 250,50 ₽", "50°C", "© 2024", "Ångström", "Ελλάδα", "東京", "e-mail: a@b.c", "Zürich",
        ] {
            assert_eq!(fix_garbled(text), None, "{text}");
        }
    }
}
