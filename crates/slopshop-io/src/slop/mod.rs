//! `.slop` document files (ADR 0009; byte layout in `docs/file-format.md`).
//!
//! One file, append-only inside: pixels are stored as content-addressed tiles (BLAKE3 of the
//! raw tile bytes, compressed with zstd after a byte-plane filter), described by a JSON
//! manifest of nodes, and committed atomically through two checksummed slots in the header.
//!
//! - [`SlopFile::create`] writes a new, compact file (a temporary file renamed over the path).
//! - [`SlopFile::open`] reads a document back, bit-exact, verifying every blob's hash.
//! - [`SlopFile::save`] saves in place: only the tiles the file does not hold yet are appended,
//!   then the manifest, the index and a commit record; the new generation becomes visible when
//!   its slot is written, after the data is synced. A crash at any point leaves the previous
//!   generation readable. When more than half of the file is dead data (older manifests,
//!   removed layers), the save rewrites a compact file instead.
//! - [`SlopFile::save_as`] writes a compact copy to another path and continues with it.
//!
//! Everything is blocking and CPU/IO-heavy (hashing, compression and decompression run on all
//! cores): front ends call it off their UI thread. Documents must fit in memory (open is eager
//! in v0); what is written never does more than a few tiles at a time beyond the document.

mod format;
mod manifest;
mod read;
mod write;

use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value};
use slopshop_core::Document;
use slopshop_core::document::RestoreError;
use slopshop_core::raster::{ImageId, RasterError};

use self::format::{Hash, IndexEntry, MAGIC};

/// File extension of documents.
pub const EXTENSION: &str = "slop";

/// Why a document could not be read or written. [`FileError::code`] gives a stable id for UIs.
#[derive(Debug)]
pub enum FileError {
    Io(io::Error),
    /// The file does not start with the `.slop` magic.
    NotASlopFile,
    /// Made by a newer SlopShop, with a format this version cannot read.
    NewerVersion {
        major: u16,
        minor: u16,
    },
    /// The file needs features this version does not know (incompatible feature flags).
    UnsupportedFeatures(u64),
    /// A node type this version does not know (e.g. `slopshop.curves@1`): made by a newer
    /// SlopShop.
    UnknownNodeType(String),
    /// The file is damaged: a checksum or hash does not match, a record is truncated…
    Corrupt(String),
    /// The file was saved by someone else since it was opened: saving would lose their work.
    Conflict,
    /// The file has read-only features this version does not know: it can be read, not saved.
    ReadOnly,
    /// The document read is invalid.
    Document(RestoreError),
    /// An image read is invalid.
    Image(RasterError),
}

impl FileError {
    /// Stable identifier, translated by UIs.
    pub fn code(&self) -> &'static str {
        match self {
            FileError::Io(_) => "io",
            FileError::NotASlopFile => "notASlopFile",
            FileError::NewerVersion { .. } | FileError::UnknownNodeType(_) => "newerVersion",
            FileError::UnsupportedFeatures(_) => "unsupportedFeatures",
            FileError::Corrupt(_) | FileError::Document(_) | FileError::Image(_) => "corrupt",
            FileError::Conflict => "conflict",
            FileError::ReadOnly => "readOnly",
        }
    }
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileError::Io(e) => write!(f, "{e}"),
            FileError::NotASlopFile => write!(f, "not a SlopShop document"),
            FileError::NewerVersion { major, minor } => {
                write!(f, "made by a newer SlopShop (format {major}.{minor})")
            }
            FileError::UnsupportedFeatures(flags) => {
                write!(f, "needs unknown features (flags {flags:#x})")
            }
            FileError::UnknownNodeType(kind) => {
                write!(f, "made by a newer SlopShop (node type {kind})")
            }
            FileError::Corrupt(what) => write!(f, "damaged file: {what}"),
            FileError::Conflict => write!(f, "the file was changed since it was opened"),
            FileError::ReadOnly => write!(f, "this file can only be read by this version"),
            FileError::Document(e) => write!(f, "invalid document: {e}"),
            FileError::Image(e) => write!(f, "invalid image: {e}"),
        }
    }
}

impl std::error::Error for FileError {}

impl From<io::Error> for FileError {
    fn from(e: io::Error) -> Self {
        FileError::Io(e)
    }
}

/// Whether `path` starts with the `.slop` magic (whatever its extension).
pub fn is_slop_file(path: &Path) -> io::Result<bool> {
    let mut magic = [0u8; 8];
    let mut file = File::open(path)?;
    match file.read_exact(&mut magic) {
        Ok(()) => Ok(magic == MAGIC),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
    }
}

/// What a save did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveReport {
    /// Bytes added to the file (or the whole file when it was rewritten).
    pub bytes_written: u64,
    /// The file was rewritten compact instead of appended to.
    pub compacted: bool,
}

/// The hashes of an image already written: saves skip hashing its tiles again. Valid for the
/// life of the image, which is immutable.
#[derive(Debug)]
pub(crate) struct ImageRecord {
    /// Image key: identifies the image across saves (and later, AI caches).
    pub key: Hash,
    /// Per level, finest first: the tile table's hash and the tiles' hashes (row-major).
    pub levels: Vec<(Hash, Vec<Hash>)>,
}

/// Parts of a manifest this version does not use, written back unchanged.
#[derive(Debug, Clone, Default)]
pub(crate) struct Residue {
    pub manifest: Map<String, Value>,
    pub sections: Map<String, Value>,
    pub document: Map<String, Value>,
    pub nodes: HashMap<u64, Map<String, Value>>,
    pub images: HashMap<Hash, Map<String, Value>>,
}

/// An open document file: where it is, which generation was read or last saved, and what it
/// already holds.
#[derive(Debug)]
pub struct SlopFile {
    path: PathBuf,
    generation: u64,
    committed_len: u64,
    /// Every tile and table in the file.
    index: HashMap<Hash, IndexEntry>,
    images: HashMap<ImageId, Arc<ImageRecord>>,
    residue: Residue,
    /// Bytes of the committed file that the current generation does not use.
    dead_bytes: u64,
    read_only: bool,
}

impl SlopFile {
    /// Write `document` to a new compact file at `path` (replacing any file there).
    pub fn create(path: &Path, document: &Document) -> Result<Self, FileError> {
        write::create(path, document, HashMap::new(), Residue::default())
    }

    /// Read the document at `path`.
    pub fn open(path: &Path) -> Result<(Document, Self), FileError> {
        read::open(path)
    }

    /// Save `document` in place (see the module documentation).
    pub fn save(&mut self, document: &Document) -> Result<SaveReport, FileError> {
        if self.read_only {
            return Err(FileError::ReadOnly);
        }
        let report = write::append(self, document)?;
        if self.dead_bytes > self.committed_len / 2 {
            let path = self.path.clone();
            return self.save_as(&path, document);
        }
        Ok(report)
    }

    /// Write a compact copy of `document` to `path` (replacing any file there) and continue
    /// with that file.
    pub fn save_as(&mut self, path: &Path, document: &Document) -> Result<SaveReport, FileError> {
        // Cheap clones (`Arc`s and small maps): on failure, this file stays as it was.
        *self = write::create(path, document, self.images.clone(), self.residue.clone())?;
        Ok(SaveReport {
            bytes_written: self.committed_len,
            compacted: true,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Size of the committed file, in bytes.
    pub fn len(&self) -> u64 {
        self.committed_len
    }

    /// Always false: a file has at least its header.
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Bytes of the file that the current document does not use (reclaimed by compaction).
    pub fn dead_bytes(&self) -> u64 {
        self.dead_bytes
    }

    /// Number of saves since the file was created.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
mod tests;
