//! File > Open Recent: the files and folders opened in new tabs, and the documents saved,
//! newest first, kept across launches in the app's config folder (`recent.txt`, one path per line). The UI gets
//! the list on request and after every change (the `recent-files` event).
//!
//! The welcome page shows them as thumbnails. A thumbnail is rendered when its file is opened
//! or saved, from the document then in memory, and kept next to the list
//! (`recent-thumbnails/`): showing the page never decodes a file again.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use slopshop_core::{Document, Size};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager};

use crate::{AppState, emit};

/// Photoshop shows 20 recent files by default; a shorter menu is easier to scan.
const MAX_RECENT: usize = 10;
const FILE_NAME: &str = "recent.txt";
const THUMBNAILS: &str = "recent-thumbnails";
/// Longer side of the welcome page's thumbnails, pixels (sharp at twice their CSS size).
const THUMBNAIL_SIDE: u32 = 256;
pub const EVENT_RECENT_FILES: &str = "recent-files";

/// The list, read from its file on first use.
#[derive(Default)]
pub struct RecentFiles {
    list: Mutex<Option<Vec<PathBuf>>>,
}

/// `list` with `opened` put first (the first of `opened` first), without duplicates, at most
/// [`MAX_RECENT`] entries.
fn pushed(list: &[PathBuf], opened: &[PathBuf]) -> Vec<PathBuf> {
    let mut next: Vec<PathBuf> = Vec::with_capacity(MAX_RECENT);
    for path in opened.iter().chain(list) {
        if next.len() == MAX_RECENT {
            break;
        }
        if !next.iter().any(|p| same_path(p, path)) {
            next.push(path.clone());
        }
    }
    next
}

/// Windows paths ignore case.
fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
    } else {
        a == b
    }
}

fn read(file: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(file)
        .map(|text| {
            text.lines()
                .filter(|line| !line.is_empty())
                .map(PathBuf::from)
                .take(MAX_RECENT)
                .collect()
        })
        .unwrap_or_default()
}

fn write(file: &Path, list: &[PathBuf]) -> std::io::Result<()> {
    if let Some(folder) = file.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let mut text = String::new();
    // A path with a line break could not be read back: left out.
    for path in list.iter().filter_map(|p| p.to_str()) {
        if !path.contains('\n') {
            text.push_str(path);
            text.push('\n');
        }
    }
    std::fs::write(file, text)
}

fn file_of(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|folder| folder.join(FILE_NAME))
}

/// Change the list with `change`, save it and tell the UI. Best effort: the list is a
/// convenience, a failure to save it is only logged.
fn update(app: &AppHandle, change: impl FnOnce(&[PathBuf]) -> Vec<PathBuf>) {
    let Some(file) = file_of(app) else {
        return;
    };
    let state = app.state::<AppState>();
    let Ok(mut list) = state.recent.list.lock() else {
        return;
    };
    let current = list.get_or_insert_with(|| read(&file));
    let next = change(current);
    if next == *current {
        return;
    }
    // The thumbnails of the entries leaving the list go with them.
    for gone in current
        .iter()
        .filter(|p| !next.iter().any(|n| same_path(n, p)))
    {
        if let Some(thumbnail) = thumbnail_file(app, gone) {
            let _ = std::fs::remove_file(thumbnail);
        }
    }
    *current = next;
    if let Err(e) = write(&file, current) {
        eprintln!("cannot save the recent files: {e}");
    }
    let shown = existing(current);
    drop(list);
    emit(app, EVENT_RECENT_FILES, &shown);
}

/// Put `opened` (files or folders, absolute) first in the list.
pub fn record(app: &AppHandle, opened: &[PathBuf]) {
    if !opened.is_empty() {
        update(app, |list| pushed(list, opened));
    }
}

/// 64-bit FNV-1a: stable across builds, unlike the standard library's hasher.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Where the thumbnail of `path` is kept: named by a hash of the path (lowercase on Windows,
/// where paths ignore case).
fn thumbnail_file(app: &AppHandle, path: &Path) -> Option<PathBuf> {
    let key = path.to_string_lossy();
    let key = if cfg!(windows) {
        key.to_lowercase()
    } else {
        key.into_owned()
    };
    let folder = app.path().app_config_dir().ok()?.join(THUMBNAILS);
    Some(folder.join(format!("{:016x}.rgba", fnv1a(key.as_bytes()))))
}

/// The size of a thumbnail of an image of `size`: at most [`THUMBNAIL_SIDE`] on its longer
/// side, never enlarged; and the document pixels per thumbnail pixel.
fn thumbnail_size(size: Size) -> (Size, f64) {
    let scale = (f64::from(size.width.max(size.height)) / f64::from(THUMBNAIL_SIDE)).max(1.0);
    let side = |v: u32| (f64::from(v) / scale).round().max(1.0) as u32;
    (Size::new(side(size.width), side(size.height)), scale)
}

/// Keep a thumbnail of `doc` (as displayed) for the welcome page's entry of `path`, before the
/// path is recorded. Best effort: without one, the page shows an icon.
pub fn remember_thumbnail(app: &AppHandle, path: &Path, doc: &Document) {
    let Some(file) = thumbnail_file(app, path) else {
        return;
    };
    let rendered = (|| -> Result<(), String> {
        let (output, scale) = thumbnail_size(doc.size());
        let view = slopshop_core::view::ViewTransform {
            origin: [0.0, 0.0],
            scale,
        };
        // The same layout as layer thumbnails: width, height (`u32` little-endian), RGBA8.
        let mut bytes = Vec::with_capacity(8 + 4 * (output.width * output.height) as usize);
        bytes.extend_from_slice(&output.width.to_le_bytes());
        bytes.extend_from_slice(&output.height.to_le_bytes());
        let frame = app
            .state::<AppState>()
            .renderer()?
            .render_view(doc, view, output)
            .map_err(|e| e.to_string())?;
        bytes.extend_from_slice(&frame.data);
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        }
        std::fs::write(&file, bytes).map_err(|e| e.to_string())
    })();
    if let Err(e) = rendered {
        eprintln!("cannot keep the thumbnail of {}: {e}", path.display());
    }
}

/// [`remember_thumbnail`] of open document `document_id`, opened from `path`.
pub fn remember_document(app: &AppHandle, path: &Path, document_id: u64) {
    // A snapshot: the documents stay unlocked while rendering.
    let doc = app
        .state::<AppState>()
        .documents()
        .ok()
        .and_then(|mut documents| {
            let document = documents.get_mut(document_id).ok()?;
            Some(document.session.document().clone())
        });
    if let Some(doc) = doc {
        remember_thumbnail(app, path, &doc);
    }
}

/// The welcome page's thumbnail of a recent entry: raw binary, width and height (`u32`
/// little-endian), then RGBA8 sRGB pixels. An error when there is none (folders, archives, or
/// it could not be made): the page shows an icon.
#[tauri::command]
pub async fn recent_thumbnail(app: AppHandle, path: PathBuf) -> Result<Response, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let file = thumbnail_file(&app, &path).ok_or("no config folder")?;
        std::fs::read(file)
            .map(Response::new)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The entries that still exist, as the menu shows them.
fn existing(list: &[PathBuf]) -> Vec<String> {
    list.iter()
        .filter(|path| path.exists())
        .map(|path| path.to_string_lossy().into_owned())
        .collect()
}

/// File > Open Recent's entries, newest first: those that still exist.
#[tauri::command]
pub async fn recent_files(app: AppHandle) -> Result<Vec<String>, String> {
    // Reading the file and checking each entry touches the disk: off the UI thread.
    tauri::async_runtime::spawn_blocking(move || {
        let file = file_of(&app).ok_or("no config folder")?;
        let state = app.state::<AppState>();
        let mut list = state.recent.list.lock().map_err(|e| e.to_string())?;
        Ok(existing(list.get_or_insert_with(|| read(&file))))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// File > Open Recent > Clear Recent File List.
#[tauri::command]
pub async fn clear_recent_files(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || update(&app, |_| Vec::new()))
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn thumbnails_fit_their_side_and_are_never_enlarged() {
        assert_eq!(
            thumbnail_size(Size::new(4000, 3000)),
            (Size::new(256, 192), 4000.0 / 256.0)
        );
        assert_eq!(
            thumbnail_size(Size::new(100, 50)),
            (Size::new(100, 50), 1.0)
        );
        assert_eq!(thumbnail_size(Size::new(100_000, 10)).0, Size::new(256, 1));
    }

    #[test]
    fn thumbnail_names_are_stable() {
        // FNV-1a's published test vectors: names must not change between builds.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn opened_files_come_first_once() {
        let list = paths(&["/a", "/b", "/c"]);
        assert_eq!(
            pushed(&list, &paths(&["/c", "/d"])),
            paths(&["/c", "/d", "/a", "/b"])
        );
        assert_eq!(pushed(&[], &paths(&["/a", "/a"])), paths(&["/a"]));
    }

    #[test]
    fn the_list_keeps_the_newest_entries() {
        let old: Vec<PathBuf> = (0..MAX_RECENT)
            .map(|i| PathBuf::from(format!("/{i}")))
            .collect();
        let next = pushed(&old, &paths(&["/new"]));
        assert_eq!(next.len(), MAX_RECENT);
        assert_eq!(next[0], PathBuf::from("/new"));
        assert_eq!(
            next[MAX_RECENT - 1],
            PathBuf::from(format!("/{}", MAX_RECENT - 2))
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_ignore_case() {
        let next = pushed(&paths(&[r"C:\Photos\A.jpg"]), &paths(&[r"c:\photos\a.JPG"]));
        assert_eq!(next, paths(&[r"c:\photos\a.JPG"]));
    }

    #[test]
    fn the_list_survives_a_restart_and_missing_entries_are_hidden() {
        let folder = std::env::temp_dir().join(format!("slopshop-recent-{}", std::process::id()));
        let file = folder.join(FILE_NAME);
        let present = folder.join("present.png");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(&present, b"").unwrap();
        let list = vec![folder.join("gone.png"), present.clone()];
        write(&file, &list).unwrap();
        assert_eq!(read(&file), list);
        assert_eq!(existing(&list), [present.to_string_lossy().into_owned()]);
        // A missing file is an empty list.
        std::fs::remove_file(&file).unwrap();
        assert!(read(&file).is_empty());
        std::fs::remove_dir_all(&folder).ok();
    }
}
