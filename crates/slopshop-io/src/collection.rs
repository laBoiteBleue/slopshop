//! Folders and zip archives opened like several files at once: the files SlopShop can open in
//! them (images and `.slop` documents), in natural order, and how many others were skipped.
//!
//! A folder is listed without its subfolders. A zip archive is extracted to a temporary folder
//! (its images may be too large to hold twice in memory), which is deleted when the
//! [`ExtractedArchive`] is dropped. Archives are untrusted: entries whose path would leave the
//! folder are skipped, and no entry may inflate beyond the size it declares.

use std::cmp::Ordering;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

/// Extensions of the files the importer reads, and of documents.
const OPENABLE: [&str; 27] = [
    "png", "jpg", "jpeg", "jpe", "jfif", "tif", "tiff", "webp", "gif", "bmp", "tga", "ico", "pnm",
    "pbm", "pgm", "ppm", "pfm", "pam", "qoi", "ff", "exr", "hdr", "dds", "jxl", "psd", "psb",
    "slop",
];

/// Largest entry extracted from an archive, uncompressed: well beyond any image the engine can
/// hold, well below what would fill a disk by mistake.
const MAX_ENTRY_BYTES: u64 = 64 << 30;

/// Whether a file of this name is one SlopShop opens (by extension, any case).
pub fn is_openable(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| OPENABLE.contains(&e.to_ascii_lowercase().as_str()))
}

/// Whether a file is a zip archive (by extension, any case).
pub fn is_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
}

/// The openable files of a folder (not of its subfolders), in natural order, and the number of
/// other files skipped.
pub fn folder_files(dir: &Path) -> io::Result<(Vec<PathBuf>, usize)> {
    let mut files = Vec::new();
    let mut skipped = 0;
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        if is_openable(&path) {
            files.push(path);
        } else {
            skipped += 1;
        }
    }
    files.sort_by(|a, b| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    Ok((files, skipped))
}

#[derive(Debug)]
pub enum ArchiveError {
    Io(io::Error),
    /// Not a zip archive, damaged, or using a compression method not supported (deflate and
    /// stored are).
    Invalid(String),
    /// An entry inflates beyond the size it declares, or beyond [`MAX_ENTRY_BYTES`].
    TooLarge(String),
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArchiveError::Io(e) => write!(f, "{e}"),
            ArchiveError::Invalid(e) => write!(f, "{e}"),
            ArchiveError::TooLarge(name) => write!(f, "{name} is larger than it declares"),
        }
    }
}

impl std::error::Error for ArchiveError {}

impl From<io::Error> for ArchiveError {
    fn from(e: io::Error) -> Self {
        ArchiveError::Io(e)
    }
}

impl From<zip::result::ZipError> for ArchiveError {
    fn from(e: zip::result::ZipError) -> Self {
        match e {
            zip::result::ZipError::Io(e) => ArchiveError::Io(e),
            other => ArchiveError::Invalid(other.to_string()),
        }
    }
}

/// The openable files of an archive, extracted to a temporary folder deleted on drop.
#[derive(Debug)]
pub struct ExtractedArchive {
    dir: PathBuf,
    /// In natural order of their paths in the archive.
    pub files: Vec<PathBuf>,
    /// Entries skipped: other files, and entries whose path would leave the folder.
    pub skipped: usize,
}

impl Drop for ExtractedArchive {
    fn drop(&mut self) {
        // Best effort: a leftover temporary folder is not worth failing for.
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Numbers the temporary folders of this process.
static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// Extract the openable files of the zip archive at `path`. Each file keeps its name (layers
/// and tabs are named after it), in a folder of its own so that names never collide.
pub fn extract_archive(path: &Path) -> Result<ExtractedArchive, ArchiveError> {
    let mut archive = zip::ZipArchive::new(File::open(path)?)?;
    let n = NEXT_DIR.fetch_add(1, AtomicOrdering::Relaxed);
    let dir = std::env::temp_dir().join(format!("slopshop-zip-{}-{n}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let mut extracted = ExtractedArchive {
        dir,
        files: Vec::new(),
        skipped: 0,
    };

    // Entries to extract, in natural order of their paths.
    let mut wanted: Vec<(usize, PathBuf)> = Vec::new();
    for index in 0..archive.len() {
        let entry = archive.by_index_raw(index)?;
        if entry.is_dir() {
            continue;
        }
        match entry.enclosed_name() {
            Some(name) if is_openable(&name) => wanted.push((index, name)),
            _ => extracted.skipped += 1,
        }
    }
    wanted.sort_by(|(_, a), (_, b)| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));

    for (slot, (index, name)) in wanted.into_iter().enumerate() {
        let Some(file_name) = name.file_name() else {
            extracted.skipped += 1;
            continue;
        };
        let folder = extracted.dir.join(slot.to_string());
        fs::create_dir_all(&folder)?;
        let destination = folder.join(file_name);
        let mut entry = archive.by_index(index)?;
        let declared = entry.size();
        let label = name.display().to_string();
        if declared > MAX_ENTRY_BYTES {
            return Err(ArchiveError::TooLarge(label));
        }
        let mut out = File::create(&destination)?;
        // One byte more than declared tells a lying entry (a zip bomb) from an honest one.
        let copied = io::copy(&mut (&mut entry).take(declared + 1), &mut out)?;
        if copied > declared {
            return Err(ArchiveError::TooLarge(label));
        }
        extracted.files.push(destination);
    }
    Ok(extracted)
}

/// Case-insensitive order where runs of digits compare as numbers: `img2` before `img10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let number = |it: &mut std::iter::Peekable<std::str::Chars<'_>>| {
                    let mut digits = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        digits.push(c);
                        it.next();
                    }
                    digits
                };
                let (m, n) = (number(&mut a), number(&mut b));
                // Compare as numbers without overflow: leading zeros aside, longer is larger.
                let (m_trim, n_trim) = (m.trim_start_matches('0'), n.trim_start_matches('0'));
                let order = m_trim
                    .len()
                    .cmp(&n_trim.len())
                    .then_with(|| m_trim.cmp(n_trim))
                    .then_with(|| m.len().cmp(&n.len()));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                let order = x.to_lowercase().cmp(y.to_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a.next();
                b.next();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("slopshop-collection-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn natural_order_compares_numbers_as_numbers() {
        let mut names = [
            "img10.png",
            "IMG2.png",
            "img1.png",
            "img02.png",
            "a.png",
            "img100.png",
        ];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            [
                "a.png",
                "img1.png",
                "IMG2.png",
                "img02.png",
                "img10.png",
                "img100.png"
            ]
        );
        let huge = "x99999999999999999999999999999.png";
        assert_eq!(natural_cmp(huge, "x1.png"), Ordering::Greater);
    }

    #[test]
    fn folders_list_their_openable_files_in_natural_order() {
        let dir = temp_dir("folder");
        for name in ["b10.JPG", "b2.png", "notes.txt", "doc.slop", "a.pdf"] {
            fs::write(dir.join(name), b"x").unwrap();
        }
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("sub").join("inside.png"), b"x").unwrap();
        let (files, skipped) = folder_files(&dir).unwrap();
        let names: Vec<_> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["b2.png", "b10.JPG", "doc.slop"]);
        assert_eq!(skipped, 2, "notes.txt and a.pdf; subfolders are not listed");
        fs::remove_dir_all(&dir).ok();
    }

    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn archives_extract_their_openable_files_and_clean_up() {
        let dir = temp_dir("zip");
        let path = dir.join("photos.zip");
        write_zip(
            &path,
            &[
                ("album/p10.png", b"ten"),
                ("album/p9.png", b"nine"),
                ("readme.txt", b"hello"),
                ("other/p9.png", b"another nine"),
                ("../evil.png", b"escape"),
            ],
        );
        let extracted = extract_archive(&path).unwrap();
        let contents: Vec<Vec<u8>> = extracted
            .files
            .iter()
            .map(|f| fs::read(f).unwrap())
            .collect();
        assert_eq!(
            contents,
            [b"nine".to_vec(), b"ten".to_vec(), b"another nine".to_vec()]
        );
        // Names are kept (two p9.png, in folders of their own).
        assert!(extracted.files.iter().all(|f| f.file_name().is_some()));
        assert_eq!(extracted.skipped, 2, "readme.txt and the escaping entry");
        let temp = extracted.dir.clone();
        assert!(temp.is_dir());
        drop(extracted);
        assert!(!temp.exists(), "the temporary folder is deleted");

        fs::write(&path, b"not a zip at all").unwrap();
        assert!(matches!(
            extract_archive(&path),
            Err(ArchiveError::Invalid(_))
        ));
        assert!(is_archive(Path::new("A.ZIP")) && !is_archive(&dir.join("a.png")));
        fs::remove_dir_all(&dir).ok();
    }
}
