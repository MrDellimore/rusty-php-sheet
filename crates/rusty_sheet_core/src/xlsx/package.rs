//! The zip container (OPC package) behind an XLSX file.
//!
//! The central directory is read once. After that, every part is opened as an
//! **owned** stream (its own file handle + decompressor) instead of borrowing a
//! `ZipArchive`. That lets a sheet stream outlive the call that created it,
//! which the PHP `Reader` needs, since it keeps a stream open between `nextRows()` calls.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use flate2::bufread::DeflateDecoder;
use zip::{CompressionMethod, ZipArchive};

use crate::{Error, Result};

/// A readable, owned part stream.
pub type PartReader = Box<dyn Read + Send>;

/// Read buffer for the compressed bytes of a part.
const RAW_BUF: usize = 64 * 1024;

#[derive(Clone)]
enum Source {
    File(PathBuf),
    Bytes(Arc<[u8]>),
}

impl Source {
    fn open(&self) -> io::Result<Box<dyn ReadSeek>> {
        Ok(match self {
            Source::File(path) => Box::new(File::open(path)?),
            Source::Bytes(bytes) => Box::new(Cursor::new(Arc::clone(bytes))),
        })
    }
}

trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

#[derive(Debug, Clone)]
struct Entry {
    data_start: u64,
    compressed_size: u64,
    method: CompressionMethod,
    encrypted: bool,
}

pub struct Package {
    source: Source,
    /// Keyed by normalised part name: no leading `/`, ASCII-lowercased
    /// (OPC part names are case-insensitive).
    entries: HashMap<String, Entry>,
}

impl Package {
    pub fn open(path: &Path) -> Result<Self> {
        Self::new(Source::File(path.to_path_buf()))
    }

    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>) -> Result<Self> {
        Self::new(Source::Bytes(bytes.into()))
    }

    fn new(source: Source) -> Result<Self> {
        let mut zip = ZipArchive::new(source.open()?)?;
        let mut entries = HashMap::with_capacity(zip.len());
        for i in 0..zip.len() {
            let file = zip.by_index_raw(i)?;
            if file.is_dir() {
                continue;
            }
            let name = file.name()?.into_owned();
            let data_start = file
                .data_start()
                .ok_or_else(|| Error::MissingPart(format!("data offset for {name}")))?;
            entries.insert(
                normalise(&name),
                Entry {
                    data_start,
                    compressed_size: file.compressed_size(),
                    method: file.compression(),
                    encrypted: file.encrypted(),
                },
            );
        }
        Ok(Package { source, entries })
    }

    pub fn contains(&self, part: &str) -> bool {
        self.entries.contains_key(&normalise(part))
    }

    /// Open a part as an owned, decompressed stream.
    pub fn open_part(&self, part: &str) -> Result<PartReader> {
        let entry = self
            .entries
            .get(&normalise(part))
            .ok_or_else(|| Error::MissingPart(part.to_string()))?;
        if entry.encrypted {
            return Err(Error::UnsupportedFormat(format!(
                "encrypted zip entry {part}"
            )));
        }

        let mut file = self.source.open()?;
        file.seek(SeekFrom::Start(entry.data_start))?;
        let raw = BufReader::with_capacity(RAW_BUF, file.take(entry.compressed_size));

        match entry.method {
            CompressionMethod::Stored => Ok(Box::new(raw)),
            CompressionMethod::Deflated => Ok(Box::new(DeflateDecoder::new(raw))),
            other => Err(Error::UnsupportedFormat(format!(
                "zip compression method {other:?} in {part}"
            ))),
        }
    }

    /// Read a whole (small) part into a byte vector.
    pub fn read_part(&self, part: &str) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        self.open_part(part)?.read_to_end(&mut out)?;
        Ok(out)
    }
}

fn normalise(part: &str) -> String {
    part.trim_start_matches('/').to_ascii_lowercase()
}

/// Resolve a relationship target against the directory of the part that owns
/// the relationship. Absolute targets (`/xl/...`) are taken from the package root.
pub fn resolve_target(base_part: &str, target: &str) -> String {
    let mut segments: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        let dir = base_part.rsplit_once('/').map_or("", |(dir, _)| dir);
        dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    for seg in target.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            s => segments.push(s),
        }
    }
    segments.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relationship_targets() {
        assert_eq!(
            resolve_target("xl/workbook.xml", "worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            resolve_target("xl/workbook.xml", "/xl/worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(
            resolve_target("xl/workbook.xml", "../customXml/item1.xml"),
            "customXml/item1.xml"
        );
        assert_eq!(resolve_target("", "xl/workbook.xml"), "xl/workbook.xml");
        assert_eq!(
            resolve_target("xl/workbook.xml", "./styles.xml"),
            "xl/styles.xml"
        );
    }

    #[test]
    fn part_names_are_case_insensitive() {
        assert_eq!(normalise("/XL/Workbook.xml"), "xl/workbook.xml");
    }
}
