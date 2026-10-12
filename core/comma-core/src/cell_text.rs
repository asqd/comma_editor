//! Text of a cell. A sheet holds millions of short values, so short ones are kept in the cell
//! itself, without an allocation each.

use std::borrow::Borrow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;

/// Bytes of UTF-8 kept inline.
const INLINE: usize = 22;

#[derive(Clone)]
pub(crate) enum CellText {
    Inline { length: u8, bytes: [u8; INLINE] },
    Heap(Box<str>),
}

impl CellText {
    pub(crate) const fn new() -> Self {
        Self::Inline { length: 0, bytes: [0; INLINE] }
    }

    pub(crate) fn as_str(&self) -> &str {
        match self {
            // Only whole UTF-8 strings are put inline.
            Self::Inline { length, bytes } => unsafe { std::str::from_utf8_unchecked(&bytes[..*length as usize]) },
            Self::Heap(text) => text,
        }
    }

    /// Heap memory the text takes, for the size of the history.
    pub(crate) fn capacity(&self) -> usize {
        match self {
            Self::Inline { .. } => 0,
            Self::Heap(text) => text.len(),
        }
    }
}

impl Default for CellText {
    fn default() -> Self {
        Self::new()
    }
}

impl From<&str> for CellText {
    fn from(text: &str) -> Self {
        if text.len() <= INLINE {
            let mut bytes = [0; INLINE];
            bytes[..text.len()].copy_from_slice(text.as_bytes());
            Self::Inline { length: text.len() as u8, bytes }
        } else {
            Self::Heap(text.into())
        }
    }
}

impl From<String> for CellText {
    fn from(text: String) -> Self {
        if text.len() <= INLINE { Self::from(text.as_str()) } else { Self::Heap(text.into_boxed_str()) }
    }
}

impl From<&String> for CellText {
    fn from(text: &String) -> Self {
        Self::from(text.as_str())
    }
}

impl From<CellText> for String {
    fn from(text: CellText) -> Self {
        match text {
            CellText::Heap(text) => text.into_string(),
            inline => inline.as_str().to_owned(),
        }
    }
}

impl Deref for CellText {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for CellText {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for CellText {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq for CellText {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for CellText {}

impl PartialEq<str> for CellText {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for CellText {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for CellText {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<CellText> for String {
    fn eq(&self, other: &CellText) -> bool {
        self == other.as_str()
    }
}

impl Hash for CellText {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl fmt::Debug for CellText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), formatter)
    }
}

impl fmt::Display for CellText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_stays_inline_and_long_text_goes_to_the_heap() {
        assert_eq!(std::mem::size_of::<CellText>(), 24);
        for text in ["", "42", "Строка 1234", "ровно двадцать два б", "длинная строка, которая не помещается"] {
            let cell = CellText::from(text);
            assert_eq!(cell.as_str(), text);
            assert_eq!(matches!(cell, CellText::Inline { .. }), text.len() <= INLINE, "{text}");
            assert_eq!(String::from(cell.clone()), text);
        }
    }
}
