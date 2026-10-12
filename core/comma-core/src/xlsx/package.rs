//! The zip archive of a workbook and the relationships that tie its parts together.

use std::collections::{HashMap, HashSet};
use std::io::{self, BufWriter, Cursor, Read, Seek, Write};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, sync_channel};

use quick_xml::events::Event;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use super::xml::{self, attribute};
use crate::bytes::Bytes;
use crate::error::{Error, Result, WorkbookError};

/// The file's bytes, shared by the copies of the archive that read parts.
#[derive(Clone)]
pub(crate) struct Shared(pub(crate) Arc<Bytes>);

impl AsRef<[u8]> for Shared {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

pub(crate) struct Package {
    archive: ZipArchive<Cursor<Shared>>,
    /// Part names in lower case, as names of parts are compared without case.
    names: HashMap<String, usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct Relationship {
    pub id: String,
    /// The last part of the type URI: `worksheet`, `sharedStrings`, `styles`. Transitional and
    /// Strict files use different URIs that end the same way.
    pub kind: String,
    /// Part name from the root of the package, without a leading `/`.
    pub target: String,
    pub external: bool,
}

impl Package {
    pub(crate) fn new(bytes: Bytes) -> Result<Self> {
        let archive = ZipArchive::new(Cursor::new(Shared(Arc::new(bytes))))
            .map_err(|_| WorkbookError::NotAWorkbook)?;
        let names = (0..archive.len())
            .filter_map(|index| Some((archive.name_for_index(index)?.trim_start_matches('/').to_lowercase(), index)))
            .collect();
        Ok(Self { archive, names })
    }

    /// Names of the parts, in lower case.
    pub(crate) fn part_names(&self) -> impl Iterator<Item = &str> {
        self.names.keys().map(String::as_str)
    }

    pub(crate) fn has_part(&self, name: &str) -> bool {
        self.names.contains_key(&name.to_lowercase())
    }

    /// The bytes of a part, `None` if there is no such part.
    pub(crate) fn read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let Some(&index) = self.names.get(&name.to_lowercase()) else {
            return Ok(None);
        };
        let mut archive = self.archive.clone();
        let mut file = archive.by_index(index).map_err(|error| part_error(name, error))?;
        let mut bytes = Vec::with_capacity(file.size().min(1 << 30) as usize);
        file.read_to_end(&mut bytes).map_err(|error| part_error(name, error))?;
        Ok(Some(bytes))
    }

    /// A part decompressed on another thread while it is read, so that a large part is never
    /// whole in memory; with its decompressed size. `None` if there is no such part.
    pub(crate) fn stream(&self, name: &str) -> Result<Option<(PartStream, u64)>> {
        let Some(&index) = self.names.get(&name.to_lowercase()) else {
            return Ok(None);
        };
        let mut archive = self.archive.clone();
        let size = archive.by_index(index).map_err(|error| part_error(name, error))?.size();
        let (sender, receiver) = sync_channel(STREAM_CHUNKS);
        std::thread::spawn(move || {
            let mut file = match archive.by_index(index) {
                Ok(file) => file,
                Err(error) => {
                    let _ = sender.send(Err(io::Error::other(error)));
                    return;
                }
            };
            loop {
                let chunk = read_chunk(&mut file);
                let last = !matches!(&chunk, Ok(bytes) if !bytes.is_empty());
                // A failed send means the reader is gone: it stopped early.
                if sender.send(chunk).is_err() || last {
                    break;
                }
            }
        });
        Ok(Some((PartStream { receiver, chunk: Vec::new(), position: 0 }, size)))
    }

    /// A part as text; XML parts are UTF-8 or UTF-16.
    pub(crate) fn read_text(&self, name: &str) -> Result<Option<String>> {
        Ok(self.read(name)?.map(xml::decode))
    }

    /// The archive again, with some parts replaced (by lower-case name) and some left out. The
    /// other parts are copied compressed, as they are, in their order.
    pub(crate) fn write_with<W: Write + Seek>(
        &self,
        out: W,
        replaced: &HashMap<String, Vec<u8>>,
        compressed: &mut HashMap<String, CompressedPart>,
        removed: &HashSet<String>,
    ) -> Result<W> {
        let mut archive = self.archive.clone();
        let mut writer = ZipWriter::new(out);
        for index in 0..archive.len() {
            let file = archive.by_index_raw(index).map_err(zip_error)?;
            let name = file.name().trim_start_matches('/').to_lowercase();
            if removed.contains(&name) {
                continue;
            }
            if let Some(part) = compressed.get_mut(&name) {
                let original = file.name().to_owned();
                drop(file);
                let made = part.0.by_index_raw(0).map_err(zip_error)?;
                writer.raw_copy_file_rename(made, original).map_err(zip_error)?;
                continue;
            }
            match replaced.get(&name) {
                Some(bytes) => {
                    let original = file.name().to_owned();
                    drop(file);
                    writer.start_file(original, deflated()).map_err(zip_error)?;
                    io::Write::write_all(&mut writer, bytes)?;
                }
                None => writer.raw_copy_file(file).map_err(zip_error)?,
            }
        }
        writer.finish().map_err(zip_error)
    }

    /// The relationships of a part (`""` for the package itself).
    pub(crate) fn relationships(&self, part: &str) -> Result<Vec<Relationship>> {
        let path = relationships_part(part);
        let folder = part.rsplit_once('/').map_or("", |(folder, _)| folder);
        let Some(text) = self.read_text(&path)? else {
            return Ok(Vec::new());
        };
        let mut reader = xml::reader(&text);
        let mut relationships = Vec::new();
        loop {
            match reader.read_event().map_err(|error| xml::error(&path, &reader, error))? {
                Event::Start(element) | Event::Empty(element) if element.local_name().as_ref() == "Relationship" => {
                    let external = attribute(&element, "TargetMode").is_some_and(|mode| mode == "External");
                    let target = attribute(&element, "Target").unwrap_or_default();
                    relationships.push(Relationship {
                        id: attribute(&element, "Id").unwrap_or_default().into_owned(),
                        kind: attribute(&element, "Type")
                            .map(|kind| kind.rsplit('/').next().unwrap_or_default().to_owned())
                            .unwrap_or_default(),
                        target: if external { target.into_owned() } else { resolve(folder, &target) },
                        external,
                    });
                }
                Event::Eof => break,
                _ => {}
            }
        }
        Ok(relationships)
    }
}

fn zip_error(error: zip::result::ZipError) -> Error {
    Error::Io(io::Error::other(error))
}

fn deflated() -> SimpleFileOptions {
    SimpleFileOptions::default().compression_method(CompressionMethod::Deflated)
}

/// A part made anew, compressed while it was written, to go into the package as it is.
pub(crate) struct CompressedPart(ZipArchive<Cursor<Vec<u8>>>);

/// Writes a part compressed as its text comes, so that a large sheet is never whole in memory.
pub(crate) struct PartWriter {
    writer: BufWriter<ZipWriter<Cursor<Vec<u8>>>>,
    error: Option<io::Error>,
}

impl PartWriter {
    pub(crate) fn new() -> Result<Self> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file("part", deflated()).map_err(zip_error)?;
        Ok(Self { writer: BufWriter::with_capacity(1 << 16, writer), error: None })
    }

    /// Adds text; an error comes out of `finish`.
    pub(crate) fn push_str(&mut self, text: &str) {
        if self.error.is_none()
            && let Err(error) = self.writer.write_all(text.as_bytes())
        {
            self.error = Some(error);
        }
    }

    pub(crate) fn finish(self) -> Result<CompressedPart> {
        if let Some(error) = self.error {
            return Err(error.into());
        }
        let writer = self.writer.into_inner().map_err(|error| Error::Io(error.into_error()))?;
        let bytes = writer.finish().map_err(zip_error)?;
        Ok(CompressedPart(ZipArchive::new(bytes).map_err(zip_error)?))
    }
}

/// The part with the relationships of `part`: `xl/_rels/workbook.xml.rels`.
pub(crate) fn relationships_part(part: &str) -> String {
    let (folder, file) = part.rsplit_once('/').unwrap_or(("", part));
    if folder.is_empty() { format!("_rels/{file}.rels") } else { format!("{folder}/_rels/{file}.rels") }
}

/// A target relative to `folder`, or from the root when it starts with `/`, as a part name.
pub(crate) fn resolve(folder: &str, target: &str) -> String {
    let mut parts: Vec<&str> = match target.strip_prefix('/') {
        Some(_) => Vec::new(),
        None => folder.split('/').filter(|part| !part.is_empty()).collect(),
    };
    for part in target.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// Decompressed bytes passed from the thread that reads the archive at a time, and how many
/// may wait to be read.
const STREAM_CHUNK: usize = 1 << 20;
const STREAM_CHUNKS: usize = 4;

/// Up to `STREAM_CHUNK` bytes, fewer only at the end of the part.
fn read_chunk(file: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut chunk = vec![0; STREAM_CHUNK];
    let mut filled = 0;
    while filled < chunk.len() {
        match file.read(&mut chunk[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    chunk.truncate(filled);
    Ok(chunk)
}

pub(crate) struct PartStream {
    receiver: Receiver<io::Result<Vec<u8>>>,
    chunk: Vec<u8>,
    position: usize,
}

impl Read for PartStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.position == self.chunk.len() {
            match self.receiver.recv() {
                Ok(chunk) => {
                    self.chunk = chunk?;
                    self.position = 0;
                }
                // The thread finished.
                Err(_) => return Ok(0),
            }
        }
        let count = out.len().min(self.chunk.len() - self.position);
        out[..count].copy_from_slice(&self.chunk[self.position..self.position + count]);
        self.position += count;
        Ok(count)
    }
}

fn part_error(name: &str, error: impl std::fmt::Display) -> Error {
    WorkbookError::Damaged { part: name.to_owned(), position: None, detail: error.to_string() }.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_targets() {
        assert_eq!(resolve("xl", "worksheets/sheet1.xml"), "xl/worksheets/sheet1.xml");
        assert_eq!(resolve("xl", "/xl/worksheets/sheet1.xml"), "xl/worksheets/sheet1.xml");
        assert_eq!(resolve("xl/worksheets", "../sharedStrings.xml"), "xl/sharedStrings.xml");
        assert_eq!(resolve("", "xl/workbook.xml"), "xl/workbook.xml");
        assert_eq!(resolve("xl", "./styles.xml"), "xl/styles.xml");
    }
}
