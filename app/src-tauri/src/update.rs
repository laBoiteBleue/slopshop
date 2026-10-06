//! In-app updates (ADR 0039): checks the release manifest, downloads the new version's package
//! with progress, and installs it, through the official updater plugin driven from Rust (the
//! webview is granted none of its commands). Only release bundles configure the updater
//! (`tauri.bundle.conf.json`); other builds offer no update.

use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::async_runtime::JoinHandle;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_updater::{Update, UpdaterExt};

/// The name of the plugin's configuration (`plugins.updater`).
const PLUGIN: &str = "updater";

/// The update found by the last check, and the download under way.
#[derive(Default)]
pub(crate) struct UpdateState {
    found: Mutex<Option<Update>>,
    download: Mutex<Option<JoinHandle<()>>>,
}

/// Whether this build can update itself: a release bundle (the updater is configured) and, on
/// Linux, an AppImage (packages are updated by the system's package manager).
pub(crate) fn supported<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.config().plugins.0.contains_key(PLUGIN) && installable(std::env::var_os("APPIMAGE"))
}

/// Whether the running installation is one the updater can replace (see [`supported`]).
fn installable(appimage: Option<std::ffi::OsString>) -> bool {
    !cfg!(target_os = "linux") || appimage.is_some()
}

/// Registers the updater plugin when this build can use it (it fails without its
/// configuration).
pub(crate) fn register<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if supported(app) {
        app.plugin(tauri_plugin_updater::Builder::new().build())?;
    }
    Ok(())
}

/// A newer version, as the Update dialog shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateInfo {
    version: String,
    current_version: String,
    /// The release notes, as published (Markdown).
    notes: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct UpdateProgress {
    done: u64,
    /// 0 when the server did not say.
    total: u64,
}

/// Why an update failed: `code` is translated by the UI (`update.error.<code>`).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct UpdateFailure {
    code: &'static str,
    detail: String,
}

impl UpdateFailure {
    fn new(code: &'static str, detail: impl ToString) -> Self {
        Self {
            code,
            detail: detail.to_string(),
        }
    }
}

fn poisoned<T>(_: T) -> UpdateFailure {
    UpdateFailure::new("failed", "poisoned")
}

impl From<tauri_plugin_updater::Error> for UpdateFailure {
    fn from(e: tauri_plugin_updater::Error) -> Self {
        use tauri_plugin_updater::Error;
        let code = match &e {
            Error::Reqwest(_) | Error::Network(_) | Error::ReleaseNotFound => "network",
            Error::Minisign(_)
            | Error::Base64(_)
            | Error::SignatureUtf8(_)
            | Error::SignedVersionMismatch { .. }
            | Error::MissingSignedVersion => "signature",
            _ => "failed",
        };
        Self::new(code, e)
    }
}

/// Whether this build offers updates (the UI hides the check, its menu entry and preference
/// otherwise).
#[tauri::command]
pub(crate) fn update_supported(app: AppHandle) -> bool {
    supported(&app)
}

/// Asks the release manifest for a newer version; `None` when this one is the latest.
#[tauri::command]
pub(crate) async fn update_check(
    app: AppHandle,
    state: State<'_, crate::AppState>,
) -> Result<Option<UpdateInfo>, UpdateFailure> {
    if !supported(&app) {
        return Err(UpdateFailure::new("unsupported", ""));
    }
    let update = app.updater()?.check().await?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
    });
    *state.update.found.lock().map_err(poisoned)? = update;
    Ok(info)
}

/// Downloads the update found by the last check and installs it, then restarts. The UI has
/// already offered to save the documents; exports must have ended. On Windows the installer
/// takes over and the application exits during the install.
#[tauri::command]
pub(crate) async fn update_install(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    progress: Channel<UpdateProgress>,
) -> Result<(), UpdateFailure> {
    let update = state
        .update
        .found
        .lock()
        .map_err(poisoned)?
        .clone()
        .ok_or_else(|| UpdateFailure::new("failed", "no update was found"))?;
    if !state.exports.is_idle() {
        return Err(UpdateFailure::new("exporting", ""));
    }
    let bytes = {
        let mut download = state.update.download.lock().map_err(poisoned)?;
        if download.is_some() {
            return Err(UpdateFailure::new("busy", ""));
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        *download = Some(tauri::async_runtime::spawn(async move {
            let mut done = 0;
            let mut sent = Instant::now() - Duration::from_secs(1);
            let result = update
                .download(
                    |chunk, total| {
                        done += chunk as u64;
                        // A message every 100 ms is enough for a progress bar.
                        if sent.elapsed() >= Duration::from_millis(100) {
                            sent = Instant::now();
                            let total = total.unwrap_or(0);
                            let _ = progress.send(UpdateProgress { done, total });
                        }
                    },
                    || {},
                )
                .await;
            let _ = sender.send(result.map(|bytes| (update, bytes)));
        }));
        receiver
    };
    // Waiting off the async runtime's threads; a cancelled download drops the sender.
    let result = tauri::async_runtime::spawn_blocking(move || bytes.recv())
        .await
        .map_err(|e| UpdateFailure::new("failed", e));
    if let Ok(mut download) = state.update.download.lock() {
        *download = None;
    }
    let (update, bytes) = match result? {
        Ok(downloaded) => downloaded?,
        Err(_) => return Err(UpdateFailure::new("cancelled", "")),
    };
    if !state.exports.is_idle() {
        return Err(UpdateFailure::new("exporting", ""));
    }
    // The installer replaces the helpers' executables: none may be running.
    let worker = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::segment::stop(&worker.state::<crate::AppState>());
    })
    .await
    .map_err(|e| UpdateFailure::new("failed", e))?;
    state.quit_confirmed.store(true, Ordering::Relaxed);
    if let Err(e) = update.install(bytes) {
        state.quit_confirmed.store(false, Ordering::Relaxed);
        return Err(e.into());
    }
    app.restart()
}

/// Stops the download under way, if any.
#[tauri::command]
pub(crate) fn update_cancel(state: State<'_, crate::AppState>) {
    if let Some(download) = state
        .update
        .download
        .lock()
        .ok()
        .and_then(|mut download| download.take())
    {
        download.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appimages_only_on_linux() {
        assert!(installable(Some("/tmp/SlopShop.AppImage".into())));
        assert_eq!(installable(None), !cfg!(target_os = "linux"));
    }

    #[test]
    fn failures_have_codes() {
        let e = UpdateFailure::from(tauri_plugin_updater::Error::Network("offline".into()));
        assert_eq!(e.code, "network");
        let e = UpdateFailure::from(tauri_plugin_updater::Error::MissingSignedVersion);
        assert_eq!(e.code, "signature");
        let e = UpdateFailure::from(tauri_plugin_updater::Error::EmptyEndpoints);
        assert_eq!(e.code, "failed");
    }

    #[test]
    fn the_bundle_configures_the_updater() {
        // Release bundles update themselves; the development configuration does not.
        let read = |name: &str| -> serde_json::Value {
            let path = format!("{}/{name}", env!("CARGO_MANIFEST_DIR"));
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
        };
        let bundle = read("tauri.bundle.conf.json");
        let updater = &bundle["plugins"][PLUGIN];
        assert!(updater["pubkey"].as_str().is_some_and(|k| !k.is_empty()));
        let endpoint = updater["endpoints"][0].as_str().unwrap();
        assert_eq!(
            endpoint,
            format!(
                "{}/releases/download/updates/latest.json",
                env!("CARGO_PKG_REPOSITORY")
            )
        );
        assert_eq!(bundle["bundle"]["createUpdaterArtifacts"], true);
        assert!(read("tauri.conf.json")["plugins"].get(PLUGIN).is_none());
    }
}
