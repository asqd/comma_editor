//! The bytes of an open file: read into memory, or mapped from disk when the file is large,
//! so that a file of several gigabytes opens without being read whole.

use std::fs::File;
use std::io::{self, Read};
use std::ops::Deref;
use std::path::Path;

use memmap2::Mmap;

/// Files from this size on are mapped instead of read.
pub(crate) const MAP_THRESHOLD: u64 = 64 << 20;

pub(crate) enum Bytes {
    Owned(Vec<u8>),
    Mapped(Mmap),
}

impl Deref for Bytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Self::Owned(bytes) => bytes,
            Self::Mapped(map) => map,
        }
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self::Owned(bytes)
    }
}

pub(crate) fn read_file(path: &Path) -> io::Result<Bytes> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    if length >= MAP_THRESHOLD {
        // SAFETY: the map is read-only. Another program that truncates the file while it is open
        // makes later reads fault; that is the price of not reading gigabytes into memory, and
        // it is why small files are read instead.
        let map = unsafe { Mmap::map(&file)? };
        return Ok(Bytes::Mapped(map));
    }
    let mut bytes = Vec::with_capacity(length as usize);
    file.read_to_end(&mut bytes)?;
    Ok(Bytes::Owned(bytes))
}
