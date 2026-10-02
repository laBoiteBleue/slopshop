//! The AI components (ADR 0025): which ones this machine needs, whether they are installed, their
//! download and their removal. The UI asks for consent first (sizes, licenses); nothing is
//! downloaded otherwise. They live in a per-user folder, `<local app data>/ai`.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use slopshop_ai::install::{self, COMPONENTS, Component, InstallError};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

/// The ONNX Runtime build this machine runs, and the models made for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Runtime {
    /// NVIDIA GPUs: ONNX Runtime's CUDA build and NVIDIA's libraries.
    Cuda,
    /// The processor: slower, but everywhere.
    Cpu,
}

impl Runtime {
    /// This machine's: CUDA when NVIDIA's driver is installed. `None` where AI is not offered
    /// yet (outside Windows x64: no runtime in the manifest).
    pub(crate) fn detect() -> Option<Self> {
        if !cfg!(all(windows, target_arch = "x86_64")) {
            return None;
        }
        // The driver installs the CUDA driver library; the rest comes with the download.
        let nvidia = std::env::var_os("SystemRoot")
            .is_some_and(|root| PathBuf::from(root).join("System32/nvcuda.dll").is_file());
        Some(if nvidia { Self::Cuda } else { Self::Cpu })
    }

    /// The helper's `--provider`.
    pub(crate) fn provider(self) -> &'static str {
        match self {
            Self::Cuda => "cuda",
            Self::Cpu => "cpu",
        }
    }

    /// The components a feature needs on this runtime.
    /// The components a feature needs on this runtime (the runtime first).
    pub(crate) fn components(self, feature: Feature) -> [&'static str; 3] {
        // Every feature includes ViTMatte: selections are refined at full resolution.
        let runtime = match self {
            Self::Cuda => "runtime-cuda",
            Self::Cpu => "runtime-cpu",
        };
        let model = match (self, feature) {
            (Self::Cuda, Feature::Segmentation) => "sam2.1-base-plus",
            (Self::Cpu, Feature::Segmentation) => "sam2.1-tiny",
            (Self::Cuda, Feature::Subject) => "birefnet",
            (Self::Cpu, Feature::Subject) => "birefnet-lite",
        };
        [runtime, model, "vitmatte-small"]
    }
}

/// What the user asks AI for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Feature {
    /// Objects from clicks, boxes and strokes (SAM 2.1): Quick and Object Selection.
    Segmentation,
    /// The main subject of the image (BiRefNet): Select > Subject.
    Subject,
}

const FEATURES: [Feature; 2] = [Feature::Segmentation, Feature::Subject];

/// Installs running, one at a time, and their cancellation.
#[derive(Default)]
pub(crate) struct AiState {
    installing: Mutex<bool>,
    cancel: AtomicBool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LicenseDto {
    name: &'static str,
    url: &'static str,
    commercial: bool,
    /// Must be accepted explicitly before the download.
    accept: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComponentDto {
    /// Stable id, translated by the UI (`ai.component.<id>`).
    id: &'static str,
    download_size: u64,
    installed_size: u64,
    installed: bool,
    licenses: Vec<LicenseDto>,
}

/// Why an install or a removal failed: `code` is translated by the UI (`ai.error.<code>`).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AiFailure {
    code: &'static str,
    detail: String,
}

impl AiFailure {
    pub(crate) fn new(code: &'static str, detail: impl ToString) -> Self {
        Self {
            code,
            detail: detail.to_string(),
        }
    }
}

impl From<InstallError> for AiFailure {
    fn from(e: InstallError) -> Self {
        let code = match &e {
            InstallError::Network(_) => "network",
            InstallError::Io(_) => "disk",
            InstallError::Corrupt(_) => "corrupt",
            InstallError::Cancelled => "cancelled",
        };
        Self::new(code, e)
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct InstallProgress {
    done: u64,
    total: u64,
}

pub(crate) fn folder(app: &AppHandle) -> Result<PathBuf, AiFailure> {
    app.path()
        .app_local_data_dir()
        .map(|dir| dir.join("ai"))
        .map_err(|e| AiFailure::new("disk", e))
}

fn dto(component: &Component, root: &std::path::Path) -> ComponentDto {
    ComponentDto {
        id: component.id,
        download_size: component.download_size(),
        installed_size: component.installed_size(),
        installed: component.is_installed(root),
        licenses: component
            .licenses
            .iter()
            .map(|l| LicenseDto {
                name: l.name,
                url: l.url,
                commercial: l.commercial,
                accept: l.accept,
            })
            .collect(),
    }
}

/// The components `feature` needs on this machine, or, without a feature, every component this
/// machine can use and every other one still installed (to remove it). `None`: AI is not
/// offered on this platform yet.
#[tauri::command]
pub(crate) async fn ai_components(
    app: AppHandle,
    feature: Option<Feature>,
) -> Result<Option<Vec<ComponentDto>>, AiFailure> {
    let root = folder(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(runtime) = Runtime::detect() else {
            return Ok(None);
        };
        let wanted: Vec<&str> = match feature {
            Some(feature) => runtime.components(feature).to_vec(),
            None => {
                let mut all: Vec<&str> = FEATURES
                    .iter()
                    .flat_map(|&f| runtime.components(f))
                    .collect();
                all.sort_unstable();
                all.dedup();
                all
            }
        };
        let list = COMPONENTS
            .iter()
            .filter(|c| wanted.contains(&c.id) || (feature.is_none() && c.is_installed(&root)))
            .map(|c| dto(c, &root))
            .collect();
        Ok(Some(list))
    })
    .await
    .map_err(|e| AiFailure::new("internal", e))?
}

/// Downloads `ids` in turn, reporting progress over all of them; one install at a time.
#[tauri::command]
pub(crate) async fn ai_install(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    ids: Vec<String>,
    progress: Channel<InstallProgress>,
) -> Result<(), AiFailure> {
    let components = ids
        .iter()
        .map(|id| install::component(id).ok_or_else(|| AiFailure::new("internal", id)))
        .collect::<Result<Vec<_>, _>>()?;
    let root = folder(&app)?;
    {
        let mut installing = state
            .ai
            .installing
            .lock()
            .map_err(|_| AiFailure::new("internal", "poisoned"))?;
        if *installing {
            return Err(AiFailure::new("busy", ""));
        }
        *installing = true;
    }
    state.ai.cancel.store(false, Ordering::Relaxed);
    let worker = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let state = worker.state::<crate::AppState>();
        let total: u64 = components.iter().map(|c| c.download_size()).sum();
        let mut before = 0;
        let mut sent = Instant::now() - Duration::from_secs(1);
        for component in components {
            install::install(component, &root, &mut |p| {
                // A message every 100 ms is enough for a progress bar.
                if sent.elapsed() >= Duration::from_millis(100) || p.done == p.total {
                    sent = Instant::now();
                    let _ = progress.send(InstallProgress {
                        done: before + p.done,
                        total,
                    });
                }
                !state.ai.cancel.load(Ordering::Relaxed)
            })?;
            before += component.download_size();
        }
        Ok::<(), AiFailure>(())
    })
    .await
    .map_err(|e| AiFailure::new("internal", e));
    if let Ok(mut installing) = state.ai.installing.lock() {
        *installing = false;
    }
    result?
}

/// Stops the install running, if any; what it fetched is kept for the next attempt.
#[tauri::command]
pub(crate) fn ai_cancel_install(state: State<'_, crate::AppState>) {
    state.ai.cancel.store(true, Ordering::Relaxed);
}

/// Removes a component's files.
#[tauri::command]
pub(crate) async fn ai_remove(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    id: String,
) -> Result<(), AiFailure> {
    let component = install::component(&id).ok_or_else(|| AiFailure::new("internal", &id))?;
    if state.ai.installing.lock().map(|i| *i).unwrap_or(true) {
        return Err(AiFailure::new("busy", ""));
    }
    // The helper may hold the runtime's libraries open.
    crate::segment::stop(&state);
    let root = folder(&app)?;
    tauri::async_runtime::spawn_blocking(move || component.remove(&root))
        .await
        .map_err(|e| AiFailure::new("internal", e))?
        .map_err(|e| AiFailure::new("disk", e))
}

/// Opens a component's license in the browser. Only the manifest's licenses: the UI never
/// opens an arbitrary address.
#[tauri::command]
pub(crate) async fn ai_open_license(app: AppHandle, url: String) -> Result<(), AiFailure> {
    let known = COMPONENTS
        .iter()
        .flat_map(|c| c.licenses)
        .any(|l| l.url == url);
    if !known {
        return Err(AiFailure::new("internal", url));
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AiFailure::new("internal", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feature_needs_components_of_the_manifest() {
        for runtime in [Runtime::Cuda, Runtime::Cpu] {
            for feature in FEATURES {
                for id in runtime.components(feature) {
                    assert!(install::component(id).is_some(), "{id}");
                }
            }
        }
    }
}
