//! Replacing a file atomically: write a temporary file next to the destination, sync it, then
//! rename it over the destination. Readers see the old file or the new one, never a partial
//! one, and a failure (or a crash) leaves the destination untouched. Shared by export and by
//! document saves (ADR 0008, 0009).
//!
//! The temporary file is `.<name>.<pid>-<n>.slopshop-tmp` in the destination's directory
//! (renames only replace atomically within one file system), created exclusively so that
//! concurrent writers to one destination never share a file: the last to finish wins, whole.
//! On Unix the directory is synced after the rename, so that the new name survives a power
//! loss; Windows has no such operation (NTFS journals the rename).

use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Suffix of the temporary files written next to the destination.
pub(crate) const TEMP_SUFFIX: &str = ".slopshop-tmp";

/// Characters of the destination's name kept in a temporary file's name: enough to recognize
/// it, few enough to stay within file-name length limits.
const TEMP_NAME_CHARS: usize = 64;

/// Existing files skipped before giving up on creating a temporary file.
const TEMP_ATTEMPTS: u32 = 1000;

/// Numbers the temporary files of this process.
pub(crate) static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temporary file next to the destination, owned by one writer: deleted unless it was
/// renamed over the destination.
#[derive(Debug)]
pub(crate) struct TempFile {
    path: PathBuf,
    persisted: bool,
}

impl TempFile {
    /// Create the temporary file for `destination`, empty, for reading and writing. It is
    /// created exclusively: an existing file (another writer's, or one a crash left behind) is
    /// skipped, never truncated, renamed or deleted. `InvalidInput` if `destination` has no
    /// file name.
    pub(crate) fn create(destination: &Path) -> io::Result<(Self, File)> {
        let name = destination.file_name().ok_or_else(|| {
            io::Error::new(
                ErrorKind::InvalidInput,
                format!("{} is not a file path", destination.display()),
            )
        })?;
        // Only a hint for whoever finds the file: a lossy name is fine.
        let name: String = name
            .to_string_lossy()
            .chars()
            .take(TEMP_NAME_CHARS)
            .collect();
        let pid = std::process::id();
        let mut skipped = 0;
        loop {
            let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = destination.with_file_name(format!(".{name}.{pid}-{n}{TEMP_SUFFIX}"));
            let created = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path);
            match created {
                Ok(file) => {
                    let temp = Self {
                        path,
                        persisted: false,
                    };
                    return Ok((temp, file));
                }
                Err(e) if e.kind() == ErrorKind::AlreadyExists && skipped < TEMP_ATTEMPTS => {
                    skipped += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Replace `destination` with the temporary file. The caller synced its content and closed
    /// every handle on it (Windows cannot rename an open file).
    pub(crate) fn persist(mut self, destination: &Path) -> io::Result<()> {
        fs::rename(&self.path, destination)?;
        self.persisted = true;
        sync_parent(destination)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.persisted {
            // Best effort: the writer's own error matters more than the clean-up's.
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Make a rename in `path`'s directory durable (Unix). A no-op elsewhere.
#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => File::open(dir)?.sync_all(),
        _ => File::open(".")?.sync_all(),
    }
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// The temporary files for `destination` that still exist (tests: none may be left).
#[cfg(test)]
pub(crate) fn temp_files(destination: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (destination.parent(), destination.file_name()) else {
        return Vec::new();
    };
    let prefix = format!(".{}.", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .is_some_and(|n| n.starts_with(&prefix) && n.ends_with(TEMP_SUFFIX))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("slopshop-atomic-{}-{name}", std::process::id()))
    }

    #[test]
    fn persisting_replaces_the_destination_whole() {
        let destination = temp_path("replace.bin");
        fs::write(&destination, b"old").unwrap();
        let (temp, mut file) = TempFile::create(&destination).unwrap();
        file.write_all(b"new content").unwrap();
        file.sync_all().unwrap();
        drop(file);
        assert_eq!(fs::read(&destination).unwrap(), b"old");
        temp.persist(&destination).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"new content");
        assert!(temp_files(&destination).is_empty());
        fs::remove_file(&destination).ok();
    }

    #[test]
    fn dropping_without_persisting_leaves_the_destination_untouched() {
        let destination = temp_path("abandon.bin");
        fs::write(&destination, b"old").unwrap();
        let (temp, mut file) = TempFile::create(&destination).unwrap();
        file.write_all(b"half").unwrap();
        drop(file);
        assert_eq!(temp_files(&destination).len(), 1);
        drop(temp);
        assert!(temp_files(&destination).is_empty());
        assert_eq!(fs::read(&destination).unwrap(), b"old");
        fs::remove_file(&destination).ok();
    }

    #[test]
    fn concurrent_writers_get_their_own_files() {
        let destination = temp_path("concurrent.bin");
        let (a, _file_a) = TempFile::create(&destination).unwrap();
        let (b, _file_b) = TempFile::create(&destination).unwrap();
        assert_ne!(a.path, b.path);
        assert_eq!(temp_files(&destination).len(), 2);
    }

    #[test]
    fn a_path_without_a_file_name_is_invalid_input() {
        let error = TempFile::create(Path::new("/")).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }
}
