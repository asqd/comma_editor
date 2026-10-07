use std::borrow::Cow;

use encoding_rs::{Encoding, KOI8_R, WINDOWS_1251, WINDOWS_1252};

/// Text encoding of a file on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Windows1251,
    Koi8R,
    Windows1252,
}

impl TextEncoding {
    pub fn name(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf16Le => "UTF-16LE",
            Self::Utf16Be => "UTF-16BE",
            Self::Windows1251 => "Windows-1251",
            Self::Koi8R => "KOI8-R",
            Self::Windows1252 => "Windows-1252",
        }
    }

    pub fn is_utf16(self) -> bool {
        matches!(self, Self::Utf16Le | Self::Utf16Be)
    }

    fn single_byte(self) -> Option<&'static Encoding> {
        match self {
            Self::Windows1251 => Some(WINDOWS_1251),
            Self::Koi8R => Some(KOI8_R),
            Self::Windows1252 => Some(WINDOWS_1252),
            Self::Utf8 | Self::Utf16Le | Self::Utf16Be => None,
        }
    }

    /// Decodes bytes of the in-memory buffer. UTF-16 files are held as UTF-8, so this is
    /// only ever called with ASCII-compatible encodings.
    pub(crate) fn decode(self, bytes: &[u8]) -> Cow<'_, str> {
        match self.single_byte() {
            Some(encoding) => encoding.decode_without_bom_handling(bytes).0,
            None => String::from_utf8_lossy(bytes),
        }
    }

    /// Encodes text for the in-memory buffer. `None` means a character has no
    /// representation in this encoding.
    pub(crate) fn encode(self, text: &str) -> Option<Cow<'_, [u8]>> {
        match self.single_byte() {
            Some(encoding) => {
                let (bytes, _, had_errors) = encoding.encode(text);
                (!had_errors).then_some(bytes)
            }
            None => Some(Cow::Borrowed(text.as_bytes())),
        }
    }
}

/// File bytes prepared for parsing.
///
/// ASCII-compatible files keep their original bytes, BOM included (`content_start` skips it).
/// UTF-16 files are transcoded to UTF-8 and transcoded back on save, which is lossless for
/// valid UTF-16; invalid UTF-16 falls back to raw single-byte handling so nothing is lost.
pub(crate) struct Prepared {
    pub encoding: TextEncoding,
    pub has_bom: bool,
    pub buffer: Vec<u8>,
    pub content_start: usize,
    /// The encoding is certain (a BOM, valid UTF-8, clear letter statistics) or was chosen.
    pub confident: bool,
}

const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
const DETECTION_SAMPLE: usize = 1 << 20;

pub(crate) fn prepare(bytes: Vec<u8>) -> Prepared {
    if bytes.starts_with(&UTF8_BOM) {
        return Prepared {
            encoding: TextEncoding::Utf8,
            has_bom: true,
            buffer: bytes,
            content_start: UTF8_BOM.len(),
            confident: true,
        };
    }

    let utf16 = match bytes.get(..2) {
        Some([0xFF, 0xFE]) => Some((TextEncoding::Utf16Le, true)),
        Some([0xFE, 0xFF]) => Some((TextEncoding::Utf16Be, true)),
        _ => sniff_utf16_without_bom(&bytes).map(|encoding| (encoding, false)),
    };
    if let Some((encoding, has_bom)) = utf16 {
        let body = if has_bom { &bytes[2..] } else { &bytes[..] };
        if let Some(text) = decode_utf16(body, encoding == TextEncoding::Utf16Le) {
            return Prepared {
                encoding,
                has_bom,
                buffer: text.into_bytes(),
                content_start: 0,
                confident: true,
            };
        }
    }

    let (encoding, confident) = guess_ascii_compatible(&bytes);
    Prepared {
        encoding,
        has_bom: false,
        buffer: bytes,
        content_start: 0,
        confident,
    }
}

/// Reads the bytes in `encoding`, chosen by the user. A BOM that matches is kept as one;
/// UTF-16 that does not decode falls back to detection.
pub(crate) fn prepare_as(bytes: Vec<u8>, encoding: TextEncoding) -> Prepared {
    match encoding {
        TextEncoding::Utf16Le | TextEncoding::Utf16Be => {
            let little_endian = encoding == TextEncoding::Utf16Le;
            let bom: &[u8] = if little_endian { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] };
            let has_bom = bytes.starts_with(bom);
            let body = if has_bom { &bytes[2..] } else { &bytes[..] };
            match decode_utf16(body, little_endian) {
                Some(text) => Prepared {
                    encoding,
                    has_bom,
                    buffer: text.into_bytes(),
                    content_start: 0,
                    confident: true,
                },
                None => prepare(bytes),
            }
        }
        _ => {
            let has_bom = encoding == TextEncoding::Utf8 && bytes.starts_with(&UTF8_BOM);
            Prepared {
                encoding,
                has_bom,
                content_start: if has_bom { UTF8_BOM.len() } else { 0 },
                buffer: bytes,
                confident: true,
            }
        }
    }
}

pub(crate) fn encode_utf16(text: &str, little_endian: bool, has_bom: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 2 + 2);
    if has_bom {
        out.extend_from_slice(if little_endian { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] });
    }
    for unit in text.encode_utf16() {
        let pair = if little_endian {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        };
        out.extend_from_slice(&pair);
    }
    out
}

fn decode_utf16(bytes: &[u8], little_endian: bool) -> Option<String> {
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let units = bytes.chunks_exact(2).map(|pair| {
        if little_endian {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        }
    });
    char::decode_utf16(units).collect::<Result<String, _>>().ok()
}

/// UTF-16 without a BOM: ASCII text leaves a zero in every other byte.
fn sniff_utf16_without_bom(bytes: &[u8]) -> Option<TextEncoding> {
    let sample = &bytes[..bytes.len().min(4096) & !1];
    let pairs = sample.len() / 2;
    if pairs < 2 {
        return None;
    }
    let even_zeros = sample.iter().step_by(2).filter(|&&b| b == 0).count();
    let odd_zeros = sample.iter().skip(1).step_by(2).filter(|&&b| b == 0).count();
    let share = |count: usize| count as f64 / pairs as f64;
    if share(odd_zeros) > 0.3 && share(even_zeros) < 0.05 {
        Some(TextEncoding::Utf16Le)
    } else if share(even_zeros) > 0.3 && share(odd_zeros) < 0.05 {
        Some(TextEncoding::Utf16Be)
    } else {
        None
    }
}

/// The encoding, and whether the guess is sure: valid UTF-8 is, UTF-8 with broken bytes
/// is not, a single-byte encoding is when its letter statistics are clear.
fn guess_ascii_compatible(bytes: &[u8]) -> (TextEncoding, bool) {
    let sample = &bytes[..bytes.len().min(DETECTION_SAMPLE)];
    let truncated = sample.len() < bytes.len();
    let (multibyte, invalid) = utf8_stats(sample, truncated);
    if invalid == 0 {
        return (TextEncoding::Utf8, true);
    }
    if multibyte > 0 && invalid * 10 < multibyte {
        return (TextEncoding::Utf8, false);
    }
    guess_single_byte(sample)
}

/// Counts valid multi-byte UTF-8 characters and invalid byte sequences.
fn utf8_stats(mut sample: &[u8], truncated: bool) -> (usize, usize) {
    let mut multibyte = 0;
    let mut invalid = 0;
    loop {
        match std::str::from_utf8(sample) {
            Ok(text) => {
                multibyte += text.chars().filter(|c| !c.is_ascii()).count();
                return (multibyte, invalid);
            }
            Err(error) => {
                let (valid, rest) = sample.split_at(error.valid_up_to());
                // SAFETY: `from_utf8` reported this prefix as valid.
                let valid = unsafe { std::str::from_utf8_unchecked(valid) };
                multibyte += valid.chars().filter(|c| !c.is_ascii()).count();
                match error.error_len() {
                    Some(len) => {
                        invalid += 1;
                        sample = &rest[len..];
                    }
                    // An incomplete character at the very end: cut by the sample or a broken file.
                    None => {
                        if !truncated {
                            invalid += 1;
                        }
                        return (multibyte, invalid);
                    }
                }
            }
        }
    }
}

/// Tells Russian text in Windows-1251 or KOI8-R from Western text in Windows-1252
/// by the most frequent lowercase Russian letters (о е а и н т с р в).
fn guess_single_byte(sample: &[u8]) -> (TextEncoding, bool) {
    let mut high = 0usize;
    let mut ascii_letters = 0usize;
    let mut cp1251 = 0usize;
    let mut koi8 = 0usize;
    for &byte in sample {
        if byte.is_ascii_alphabetic() {
            ascii_letters += 1;
        }
        if byte < 0x80 {
            continue;
        }
        high += 1;
        match byte {
            0xEE | 0xE5 | 0xE0 | 0xE8 | 0xED | 0xF2 | 0xF1 | 0xF0 | 0xE2 => cp1251 += 1,
            0xCF | 0xC5 | 0xC1 | 0xC9 | 0xCE | 0xD4 | 0xD3 | 0xD2 | 0xD7 => koi8 += 1,
            _ => {}
        }
    }
    let high_share = high as f64 / (high + ascii_letters).max(1) as f64;
    let russian_share = (cp1251 + koi8) as f64 / high.max(1) as f64;
    if high_share > 0.3 && russian_share > 0.25 {
        // Sure when the letters are clearly Russian and one encoding clearly wins.
        let sure = russian_share > 0.4 && cp1251.max(koi8) >= 3 * cp1251.min(koi8).max(1);
        let encoding = if cp1251 >= koi8 { TextEncoding::Windows1251 } else { TextEncoding::Koi8R };
        (encoding, sure)
    } else {
        // Western text has a few accented letters among plain ones; many high bytes without
        // Russian statistics may be another language altogether.
        (TextEncoding::Windows1252, high_share < 0.15)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_utf8_bom() {
        let prepared = prepare(b"\xEF\xBB\xBFa,b\n".to_vec());
        assert_eq!(prepared.encoding, TextEncoding::Utf8);
        assert!(prepared.has_bom);
        assert_eq!(prepared.content_start, 3);
    }

    #[test]
    fn detects_windows_1251() {
        let (bytes, _, _) = WINDOWS_1251.encode("Имя;Город\nИван;Москва\nОльга;Новосибирск\n");
        assert_eq!(prepare(bytes.into_owned()).encoding, TextEncoding::Windows1251);
    }

    #[test]
    fn detects_koi8_r() {
        let (bytes, _, _) = KOI8_R.encode("имя,город\nиван,москва\nольга,новосибирск\n");
        assert_eq!(prepare(bytes.into_owned()).encoding, TextEncoding::Koi8R);
    }

    #[test]
    fn detects_windows_1252() {
        let (bytes, _, _) = WINDOWS_1252.encode("name,city\nJosé,São Paulo\nFrançois,Genève\n");
        assert_eq!(prepare(bytes.into_owned()).encoding, TextEncoding::Windows1252);
    }

    #[test]
    fn detects_utf16_without_bom() {
        let bytes = encode_utf16("a,b\nc,d\n", true, false);
        let prepared = prepare(bytes);
        assert_eq!(prepared.encoding, TextEncoding::Utf16Le);
        assert!(!prepared.has_bom);
        assert_eq!(prepared.buffer, b"a,b\nc,d\n");
    }

    #[test]
    fn invalid_utf16_falls_back_to_raw_bytes() {
        // BOM followed by an odd number of bytes.
        let prepared = prepare(vec![0xFF, 0xFE, b'a', 0, b'b']);
        assert!(!prepared.encoding.is_utf16());
        assert_eq!(prepared.buffer.len(), 5);
    }
}
