//! The AI components' installer (ADR 0025): ONNX Runtime and the models, downloaded from their
//! publishers with the user's consent, never shipped with the editor. Every file is pinned by
//! its size and SHA-256 (see `manifest`). A file inside an archive is fetched alone, as an HTTP
//! range over its compressed bytes, then inflated. An interrupted download resumes, and a file
//! appears under its name only once verified, so an installed file is one that exists.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

pub use crate::manifest::COMPONENTS;

/// A license a component's files come under, shown before the download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct License {
    pub name: &'static str,
    pub url: &'static str,
    /// Whether it allows commercial use.
    pub commercial: bool,
    /// Whether the user must accept it explicitly before the download: anything but a
    /// permissive open-source license (ADR 0025).
    pub accept: bool,
}

/// Where a file's bytes are at its URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The whole resource.
    File,
    /// Stored as is in a zip archive, from `offset`.
    Stored { offset: u64 },
    /// Deflated in a zip archive: `compressed` bytes from `offset`.
    Deflated { offset: u64, compressed: u64 },
}

/// One installed file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Download {
    /// Where it is installed, relative to the AI folder (`/`-separated).
    pub path: &'static str,
    pub url: &'static str,
    pub source: Source,
    /// Its size once installed.
    pub size: u64,
    /// Its SHA-256 once installed, in lowercase hexadecimal.
    pub sha256: &'static str,
}

impl Download {
    /// The bytes fetched for it: its range at the URL, `(start, length)`.
    fn range(&self) -> (u64, u64) {
        match self.source {
            Source::File => (0, self.size),
            Source::Stored { offset } => (offset, self.size),
            Source::Deflated { offset, compressed } => (offset, compressed),
        }
    }

    fn target(&self, root: &Path) -> PathBuf {
        root.join(self.path)
    }
}

/// Something installed as a whole: a runtime, or a model's files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    /// Stable id; the interface translates it.
    pub id: &'static str,
    pub licenses: &'static [License],
    pub files: &'static [Download],
}

impl Component {
    /// The bytes to download.
    pub fn download_size(&self) -> u64 {
        self.files.iter().map(|f| f.range().1).sum()
    }

    /// The bytes on disk once installed.
    pub fn installed_size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// Whether every file is in `root` (the AI folder). Files are verified when installed.
    pub fn is_installed(&self, root: &Path) -> bool {
        self.files
            .iter()
            .all(|f| fs::metadata(f.target(root)).is_ok_and(|m| m.is_file() && m.len() == f.size))
    }

    /// Removes its files from `root`, and their partial downloads.
    pub fn remove(&self, root: &Path) -> io::Result<()> {
        for file in self.files {
            let target = file.target(root);
            for path in [target.clone(), partial(&target), inflating(&target)] {
                match fs::remove_file(&path) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

/// The component with this id.
pub fn component(id: &str) -> Option<&'static Component> {
    COMPONENTS.iter().find(|c| c.id == id)
}

/// Why an install stopped.
#[derive(Debug)]
pub enum InstallError {
    /// The download failed (network, server); the part already fetched is kept.
    Network(String),
    Io(io::Error),
    /// A file did not match its pin: the download is discarded.
    Corrupt(&'static str),
    Cancelled,
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(e) => write!(f, "download failed: {e}"),
            Self::Io(e) => write!(f, "{e}"),
            Self::Corrupt(path) => write!(f, "{path} does not match its checksum"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for InstallError {}

impl From<io::Error> for InstallError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// How far an install is, in downloaded bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub done: u64,
    pub total: u64,
}

/// Reads `length` bytes from `start` at a URL.
pub type Fetch<'a> = dyn FnMut(&str, u64, u64) -> Result<Box<dyn Read>, InstallError> + 'a;

/// Installs `component` into `root` (the AI folder) over HTTPS. `progress` is called as bytes
/// arrive; returning `false` cancels (what was fetched is kept for the next attempt).
pub fn install(
    component: &Component,
    root: &Path,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<(), InstallError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent(concat!("SlopShop/", env!("CARGO_PKG_VERSION")))
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .build()
        .into();
    install_with(
        component,
        root,
        &mut |url, start, length| http(&agent, url, start, length),
        progress,
    )
}

/// A range of a resource over HTTP. The server must honor it (`206`), except for a whole file.
fn http(
    agent: &ureq::Agent,
    url: &str,
    start: u64,
    length: u64,
) -> Result<Box<dyn Read>, InstallError> {
    let end = start + length - 1;
    let response = agent
        .get(url)
        .header("Range", format!("bytes={start}-{end}"))
        .call()
        .map_err(|e| InstallError::Network(format!("{url}: {e}")))?;
    if response.status() != 206 {
        return Err(InstallError::Network(format!(
            "{url}: the server ignored the range (status {})",
            response.status()
        )));
    }
    Ok(Box::new(response.into_body().into_reader()))
}

/// `install` with any source of bytes (tests use memory).
pub fn install_with(
    component: &Component,
    root: &Path,
    fetch: &mut Fetch<'_>,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<(), InstallError> {
    let mut state = Progress {
        done: 0,
        total: component.download_size(),
    };
    for file in component.files {
        let target = file.target(root);
        let (start, length) = file.range();
        if fs::metadata(&target).is_ok_and(|m| m.len() == file.size) {
            state.done += length;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let part = partial(&target);
        fetch_part(&part, file.url, start, length, fetch, &mut state, progress)?;
        verify_into_place(file, &part, &target)?;
    }
    progress(state);
    Ok(())
}

/// Fetches `length` bytes from `start` into `part`, resuming after what it already holds.
fn fetch_part(
    part: &Path,
    url: &str,
    start: u64,
    length: u64,
    fetch: &mut Fetch<'_>,
    state: &mut Progress,
    progress: &mut dyn FnMut(Progress) -> bool,
) -> Result<(), InstallError> {
    let mut have = fs::metadata(part).map_or(0, |m| m.len());
    if have > length {
        fs::remove_file(part)?;
        have = 0;
    }
    state.done += have;
    if have == length {
        return Ok(());
    }
    let mut out = BufWriter::with_capacity(
        1 << 20,
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(part)?,
    );
    let mut input = fetch(url, start + have, length - have)?.take(length - have);
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        if !progress(*state) {
            out.flush()?;
            return Err(InstallError::Cancelled);
        }
        let n = match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                out.flush()?;
                return Err(InstallError::Network(format!("{url}: {e}")));
            }
        };
        out.write_all(&buffer[..n])?;
        have += n as u64;
        state.done += n as u64;
    }
    out.flush()?;
    if have < length {
        return Err(InstallError::Network(format!(
            "{url}: the transfer ended early"
        )));
    }
    Ok(())
}

/// Checks the fetched bytes against the pin (inflating them first if deflated) and moves the
/// file into place; discards them if they do not match.
fn verify_into_place(file: &Download, part: &Path, target: &Path) -> Result<(), InstallError> {
    let input = BufReader::with_capacity(1 << 20, File::open(part)?);
    let (written, digest, from) = match file.source {
        Source::Deflated { .. } => {
            let tmp = inflating(target);
            let mut out = BufWriter::with_capacity(1 << 20, File::create(&tmp)?);
            let result = copy_hashing(
                flate2::read::DeflateDecoder::new(input),
                &mut out,
                file.size,
            );
            let flushed = out.flush();
            drop(out);
            match (result, flushed) {
                (Ok((written, digest)), Ok(())) => (written, digest, Some(tmp)),
                (Err(e), _) | (_, Err(e)) => {
                    let _ = fs::remove_file(&tmp);
                    // Undecodable bytes are corrupt bytes: fetch them again next time.
                    if e.kind() == io::ErrorKind::InvalidInput
                        || e.kind() == io::ErrorKind::InvalidData
                    {
                        let _ = fs::remove_file(part);
                        return Err(InstallError::Corrupt(file.path));
                    }
                    return Err(e.into());
                }
            }
        }
        Source::File | Source::Stored { .. } => {
            let (written, digest) = copy_hashing(input, &mut io::sink(), file.size)?;
            (written, digest, None)
        }
    };
    if written != file.size || digest != file.sha256 {
        if let Some(tmp) = &from {
            let _ = fs::remove_file(tmp);
        }
        let _ = fs::remove_file(part);
        return Err(InstallError::Corrupt(file.path));
    }
    match from {
        Some(tmp) => {
            fs::rename(&tmp, target)?;
            fs::remove_file(part)?;
        }
        None => fs::rename(part, target)?,
    }
    Ok(())
}

/// Copies at most `limit` + 1 bytes, hashing them: `(bytes, SHA-256 in hexadecimal)`.
fn copy_hashing(input: impl Read, out: &mut impl Write, limit: u64) -> io::Result<(u64, String)> {
    let mut input = input.take(limit + 1);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    let mut written = 0u64;
    loop {
        let n = match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(&buffer[..n]);
        out.write_all(&buffer[..n])?;
        written += n as u64;
    }
    let digest = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((written, digest))
}

fn partial(target: &Path) -> PathBuf {
    suffixed(target, ".part")
}

fn inflating(target: &Path) -> PathBuf {
    suffixed(target, ".inflating")
}

fn suffixed(target: &Path, suffix: &str) -> PathBuf {
    let mut name = target.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn leak<T>(value: T) -> &'static T {
        Box::leak(Box::new(value))
    }

    /// An "archive": a prefix, a deflated payload, a stored payload; and a plain file.
    struct Server {
        archive: Vec<u8>,
        plain: Vec<u8>,
        component: &'static Component,
        payloads: [Vec<u8>; 3],
        /// Bytes served before failing (simulates a broken connection), if any.
        cut: Option<usize>,
        served: usize,
    }

    fn server() -> Server {
        // Pseudo-random, so that it stays large once deflated.
        let mut seed = 1u32;
        let deflated_payload: Vec<u8> = (0..300_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 24) as u8
            })
            .collect();
        let stored_payload: Vec<u8> = (0..5_000u32).map(|i| (i * 7 % 256) as u8).collect();
        let plain: Vec<u8> = (0..70_000u32).map(|i| (i % 13) as u8).collect();
        let mut encoder =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&deflated_payload).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut archive = vec![0xAAu8; 100];
        let deflated_at = archive.len() as u64;
        archive.extend_from_slice(&compressed);
        let stored_at = archive.len() as u64;
        archive.extend_from_slice(&stored_payload);
        archive.extend_from_slice(&[0xBB; 50]);
        let files: &'static [Download] = Box::leak(Box::new([
            Download {
                path: "runtime/test/a.dll",
                url: "archive",
                source: Source::Deflated {
                    offset: deflated_at,
                    compressed: compressed.len() as u64,
                },
                size: deflated_payload.len() as u64,
                sha256: Box::leak(sha(&deflated_payload).into_boxed_str()),
            },
            Download {
                path: "runtime/test/b.dll",
                url: "archive",
                source: Source::Stored { offset: stored_at },
                size: stored_payload.len() as u64,
                sha256: Box::leak(sha(&stored_payload).into_boxed_str()),
            },
            Download {
                path: "models/test/model.onnx",
                url: "plain",
                source: Source::File,
                size: plain.len() as u64,
                sha256: Box::leak(sha(&plain).into_boxed_str()),
            },
        ]));
        let component = leak(Component {
            id: "test",
            licenses: &[],
            files,
        });
        Server {
            archive,
            plain: plain.clone(),
            component,
            payloads: [deflated_payload, stored_payload, plain],
            cut: None,
            served: 0,
        }
    }

    impl Server {
        fn fetch(
            &mut self,
            url: &str,
            start: u64,
            length: u64,
        ) -> Result<Box<dyn Read>, InstallError> {
            let data = if url == "archive" {
                &self.archive
            } else {
                &self.plain
            };
            let range = &data[start as usize..(start + length) as usize];
            let mut bytes = range.to_vec();
            if let Some(cut) = self.cut {
                let left = cut.saturating_sub(self.served);
                if left < bytes.len() {
                    bytes.truncate(left);
                    self.served += left;
                    return Ok(Box::new(Cursor::new(bytes).chain(Failing)));
                }
            }
            self.served += bytes.len();
            Ok(Box::new(Cursor::new(bytes)))
        }
    }

    struct Failing;
    impl Read for Failing {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset"))
        }
    }

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("slopshop-ai-install-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn installs_ranges_and_files_and_verifies_them() {
        let mut server = server();
        let component = server.component;
        let root = folder("ok");
        assert!(!component.is_installed(&root));
        let mut last = None;
        install_with(
            component,
            &root,
            &mut |url, start, length| server.fetch(url, start, length),
            &mut |p| {
                last = Some(p);
                true
            },
        )
        .unwrap();
        assert!(component.is_installed(&root));
        for (file, payload) in component.files.iter().zip(&server.payloads) {
            assert_eq!(&fs::read(root.join(file.path)).unwrap(), payload);
        }
        let last = last.unwrap();
        assert_eq!(last.done, last.total);
        assert_eq!(last.total, component.download_size());
        // Installed: nothing is fetched again.
        install_with(
            component,
            &root,
            &mut |_, _, _| panic!("fetched again"),
            &mut |_| true,
        )
        .unwrap();
        component.remove(&root).unwrap();
        assert!(!component.is_installed(&root));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_broken_download_resumes_where_it_stopped() {
        let mut server = server();
        let component = server.component;
        let root = folder("resume");
        server.cut = Some(10_000);
        let error = install_with(
            component,
            &root,
            &mut |url, start, length| server.fetch(url, start, length),
            &mut |_| true,
        )
        .unwrap_err();
        assert!(matches!(error, InstallError::Network(_)), "{error}");
        server.cut = None;
        let mut first = Vec::new();
        install_with(
            component,
            &root,
            &mut |url, start, length| {
                first.push(start);
                server.fetch(url, start, length)
            },
            &mut |_| true,
        )
        .unwrap();
        // The first file resumed after the 10 000 bytes already there.
        let Source::Deflated { offset, .. } = component.files[0].source else {
            unreachable!()
        };
        assert_eq!(first[0], offset + 10_000);
        assert!(component.is_installed(&root));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn corrupt_bytes_are_discarded() {
        let mut server = server();
        let component = server.component;
        let root = folder("corrupt");
        let at = server.archive.len() - 60;
        server.archive[at] ^= 0xff; // In the stored payload.
        let error = install_with(
            component,
            &root,
            &mut |url, start, length| server.fetch(url, start, length),
            &mut |_| true,
        )
        .unwrap_err();
        assert!(
            matches!(error, InstallError::Corrupt("runtime/test/b.dll")),
            "{error}"
        );
        let b = root.join(component.files[1].path);
        assert!(!b.exists() && !partial(&b).exists());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cancelling_keeps_what_was_fetched() {
        let mut server = server();
        let component = server.component;
        let root = folder("cancel");
        let mut calls = 0;
        let error = install_with(
            component,
            &root,
            &mut |url, start, length| server.fetch(url, start, length),
            &mut |_| {
                calls += 1;
                calls < 2
            },
        )
        .unwrap_err();
        assert!(matches!(error, InstallError::Cancelled));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_manifest_is_consistent() {
        let mut ids = std::collections::HashSet::new();
        let mut paths = std::collections::HashSet::new();
        for component in COMPONENTS {
            assert!(ids.insert(component.id), "{} twice", component.id);
            assert!(
                !component.licenses.is_empty(),
                "{}: no license",
                component.id
            );
            for file in component.files {
                assert!(paths.insert(file.path), "{} twice", file.path);
                assert!(file.url.starts_with("https://"), "{}", file.url);
                assert!(
                    file.sha256.len() == 64 && file.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
                    "{}",
                    file.path
                );
                assert!(!file.path.contains(".."), "{}", file.path);
            }
        }
        assert!(component("runtime-directml").is_some());
    }
}
