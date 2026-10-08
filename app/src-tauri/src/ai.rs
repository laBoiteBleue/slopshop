//! The AI components (ADR 0025): which ones this machine needs, whether they are installed, their
//! download and their removal. The UI asks for consent first (sizes, licenses); nothing is
//! downloaded otherwise. They live in a per-user folder, `<local app data>/ai`.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::Serialize;
use slopshop_ai::EraseModel;
use slopshop_ai::install::{self, COMPONENTS, Component, InstallError};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_opener::OpenerExt;

/// The ONNX Runtime build this machine runs, and the models made for it (ADR 0025).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Runtime {
    /// Windows x64: DirectML, on any DirectX 12 graphics card; the models in half precision.
    DirectMl,
    /// macOS on Apple silicon: Core ML (GPU and Neural Engine); the models in single precision.
    CoreMl,
    /// Linux (x64 or ARM): the processor; the small models.
    Cpu,
}

impl Runtime {
    /// This machine's. `None` where AI is not offered (no runtime in the manifest: Intel Macs,
    /// Windows on ARM…).
    pub(crate) fn detect() -> Option<Self> {
        if cfg!(all(windows, target_arch = "x86_64")) {
            Some(Self::DirectMl)
        } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            Some(Self::CoreMl)
        } else if cfg!(all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )) {
            Some(Self::Cpu)
        } else {
            None
        }
    }

    /// The helper's `--provider`.
    pub(crate) fn provider(self) -> &'static str {
        match self {
            Self::DirectMl => "directml",
            Self::CoreMl => "coreml",
            Self::Cpu => "cpu",
        }
    }

    /// The components a feature needs on this runtime (the runtime first); none where the
    /// feature is not offered (the Erase tool runs on DirectML only, ADR 0045).
    pub(crate) fn components(self, feature: Feature) -> Vec<&'static str> {
        // Every feature includes ViTMatte: selections are refined at full resolution.
        let (runtime, sam, birefnet) = match self {
            Self::DirectMl => ("runtime-directml", "sam2.1-base-plus", "birefnet"),
            Self::CoreMl => (
                "runtime-coreml-macos-arm64",
                "sam2.1-base-plus-fp32",
                "birefnet-fp32",
            ),
            Self::Cpu if cfg!(target_arch = "aarch64") => {
                ("runtime-cpu-linux-arm64", "sam2.1-tiny", "birefnet-lite")
            }
            Self::Cpu => ("runtime-cpu-linux-x64", "sam2.1-tiny", "birefnet-lite"),
        };
        let model = match feature {
            Feature::Segmentation => sam,
            Feature::Subject => birefnet,
            Feature::Erase if self == Self::DirectMl => {
                return erase_components(runtime, erase_model());
            }
            Feature::Erase => return Vec::new(),
        };
        // Refine Edge: ViTMatte-B on a GPU; the small model on the CPU, where the base one
        // takes about 2 s a window.
        let matte = match self {
            Self::Cpu => "vitmatte-small",
            _ => "vitmatte-base",
        };
        vec![runtime, model, matte]
    }
}

/// The components the Erase tool's `model` needs, after `runtime`.
fn erase_components(runtime: &'static str, model: EraseModel) -> Vec<&'static str> {
    match model {
        EraseModel::Turbo => vec![runtime, "flux2-vae", "flux2-klein-4b", "erase-v1"],
        // Its prompt embeddings are not a component until they are published: the helper
        // reads them from `models/slopshop/erase-base`, put there by hand.
        EraseModel::Base => vec![
            runtime,
            "flux2-vae",
            "flux2-klein-base-4b",
            "fal-object-remove",
        ],
    }
}

/// The Erase tool's model (ADR 0045): the turbo one, or the base one with
/// `SLOPSHOP_ERASE_MODEL=base` while it is evaluated (its prompt embeddings are not published
/// yet).
pub(crate) fn erase_model() -> EraseModel {
    match std::env::var("SLOPSHOP_ERASE_MODEL").as_deref() {
        Ok("base") => EraseModel::Base,
        _ => EraseModel::Turbo,
    }
}

/// The helper's provider on this machine (`directml`, `coreml` or `cpu`), or `None` where AI
/// is not offered: the UI picks its defaults from it (on the CPU, Refine Edges is slow).
#[tauri::command]
pub(crate) fn ai_runtime() -> Option<&'static str> {
    Runtime::detect().map(Runtime::provider)
}

/// What the user asks AI for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Feature {
    /// Objects from clicks, boxes and strokes (SAM 2.1): Quick and Object Selection.
    Segmentation,
    /// The main subject of the image (BiRefNet): Select > Subject.
    Subject,
    /// Generative fill of a selection (FLUX.2 [klein] with `erase_v1`): Delete's choice.
    Erase,
}

const FEATURES: [Feature; 3] = [Feature::Segmentation, Feature::Subject, Feature::Erase];

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
            Some(feature) => match runtime.components(feature) {
                ids if ids.is_empty() => return Ok(None),
                ids => ids,
            },
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
        for runtime in [Runtime::DirectMl, Runtime::CoreMl, Runtime::Cpu] {
            for feature in FEATURES {
                for id in runtime.components(feature) {
                    assert!(install::component(id).is_some(), "{id}");
                }
            }
        }
    }

    /// Both Erase variants, whatever `SLOPSHOP_ERASE_MODEL` says: the base one named a
    /// component missing from the manifest, and failed before starting the helper.
    #[test]
    fn both_erase_models_need_components_of_the_manifest() {
        for model in [EraseModel::Turbo, EraseModel::Base] {
            for id in erase_components("runtime-directml", model) {
                assert!(install::component(id).is_some(), "{model:?}: {id}");
            }
        }
    }
}
