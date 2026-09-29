//! Export commands (ADR 0008): settings for the export dialog, and export jobs.
//!
//! An export runs on a worker thread from a snapshot of the document (cheap: raster pixels are
//! shared), so editing can go on meanwhile. Its progress and outcome are events; it can be
//! cancelled by id. The pixels come from the GPU renderer, or from the CPU compositor when there
//! is no GPU ([`slopshop_render::export_source`]).
//!
//! Closing the main window while exports run cancels them and waits, a few seconds at most, for
//! them to end, so that they remove their temporary files ([`ExportJobs::stop_all`]).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use slopshop_core::{CancelToken, ColorSpace, Document, Progress};
use slopshop_io::export::{
    ExportReport, ExportSpec, default_spec, export_image, max_side, supports_gray, supports_space,
};
use slopshop_render::Renderer;
use tauri::{AppHandle, Manager, State};

use crate::ipc::{
    ExportFailed, ExportFinished, ExportFormatId, ExportNoticeView, ExportProgress, ExportSpecDto,
    ExportStarted, NAMED_SPACES,
};
use crate::{AppState, DOCUMENT_CLOSED, emit, file_name, panic_detail};

const EVENT_EXPORT_STARTED: &str = "export-started";
const EVENT_EXPORT_PROGRESS: &str = "export-progress";
const EVENT_EXPORT_FINISHED: &str = "export-finished";
const EVENT_EXPORT_FAILED: &str = "export-failed";

/// Minimum time between two progress events of one export.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Export jobs running, by id, with the token that cancels each of them.
#[derive(Default)]
pub(crate) struct ExportJobs {
    last_id: AtomicU64,
    running: Mutex<HashMap<u64, CancelToken>>,
    /// Notified when the last running job ends.
    idle: Condvar,
    /// Set by [`Self::stop_all`] (the app is quitting): jobs started from then on are
    /// cancelled at once.
    stopping: AtomicBool,
}

impl ExportJobs {
    /// Register a new job: its id (never reused) and its cancellation token.
    fn start(&self) -> (u64, CancelToken) {
        let id = self.last_id.fetch_add(1, Ordering::Relaxed) + 1;
        let token = CancelToken::new();
        if let Ok(mut running) = self.running.lock() {
            // Read under the lock: `stop_all` either sees this job or is seen by it.
            if self.stopping.load(Ordering::SeqCst) {
                token.cancel();
            }
            running.insert(id, token.clone());
        }
        (id, token)
    }

    /// Ask a job to stop. False if it is not running (finished already, or unknown).
    fn cancel(&self, id: u64) -> bool {
        let token = self
            .running
            .lock()
            .ok()
            .and_then(|running| running.get(&id).cloned());
        token.inspect(CancelToken::cancel).is_some()
    }

    fn finish(&self, id: u64) {
        if let Ok(mut running) = self.running.lock() {
            running.remove(&id);
            if running.is_empty() {
                self.idle.notify_all();
            }
        }
    }

    /// No job is running.
    pub(crate) fn is_idle(&self) -> bool {
        self.running.lock().is_ok_and(|running| running.is_empty())
    }

    /// Cancel every running job, and every job started from now on (the app is quitting).
    /// False if this was already done.
    pub(crate) fn stop_all(&self) -> bool {
        if self.stopping.swap(true, Ordering::SeqCst) {
            return false;
        }
        if let Ok(running) = self.running.lock() {
            running.values().for_each(CancelToken::cancel);
        }
        true
    }

    /// Block until no job is running, at most `timeout`. True if none is running. Jobs end after
    /// their temporary file is removed, so a cancelled job that has ended left nothing behind.
    pub(crate) fn wait_idle(&self, timeout: Duration) -> bool {
        let Ok(running) = self.running.lock() else {
            return false;
        };
        self.idle
            .wait_timeout_while(running, timeout, |running| !running.is_empty())
            .is_ok_and(|(_, wait)| !wait.timed_out())
    }
}

/// Unregisters a job when dropped: explicitly before its outcome is announced, or on unwinding.
struct JobGuard<'a> {
    jobs: &'a ExportJobs,
    id: u64,
}

impl Drop for JobGuard<'_> {
    fn drop(&mut self) {
        self.jobs.finish(self.id);
    }
}

/// Lets through one progress update per interval (the first one immediately), so that fast
/// exports do not flood the UI with events.
struct Throttle {
    interval: Duration,
    last: Option<Instant>,
}

impl Throttle {
    fn new(interval: Duration) -> Self {
        Self {
            interval,
            last: None,
        }
    }

    fn ready(&mut self, now: Instant) -> bool {
        if self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) < self.interval)
        {
            return false;
        }
        self.last = Some(now);
        true
    }
}

/// The document's own color space when [`default_spec`] picks it for `format` and it has no
/// name: what the `custom` space id stands for.
fn custom_space(format: ExportFormatId, document: &Document) -> Option<ColorSpace> {
    let space = default_spec(format.kind(), document).space;
    space.id().is_none().then_some(space)
}

/// Snapshot of a document. Cheap: raster pixels are shared, never copied.
fn snapshot(state: &AppState, document_id: u64) -> Result<Document, String> {
    Ok(state
        .documents()?
        .get_mut(document_id)?
        .session
        .document()
        .clone())
}

/// Export `document` to `path`: blocking and heavy, worker threads only. A panic (in the
/// encoders, the renderer, …) becomes a failure with the code `internal`.
fn run_export(
    path: &Path,
    document: &Document,
    spec: &ExportSpec,
    renderer: Option<&Renderer>,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(Progress),
) -> Result<ExportReport, ExportFailed> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let source = slopshop_render::export_source(renderer, document);
        export_image(path, document.size(), spec, source, cancel, progress)
            .map_err(|e| ExportFailed::new(None, &e))
    }))
    .unwrap_or_else(|panic| {
        Err(ExportFailed {
            id: None,
            code: "internal",
            detail: panic_detail(panic.as_ref(), "export panicked"),
        })
    })
}

/// The job itself, on a worker thread: export, then announce the outcome.
fn export_job(
    app: &AppHandle,
    id: u64,
    path: &Path,
    document: &Document,
    spec: &ExportSpec,
    cancel: &CancelToken,
) {
    let state = app.state::<AppState>();
    let guard = JobGuard {
        jobs: &state.exports,
        id,
    };
    // Without a GPU, the CPU compositor gives the same pixels, only slower.
    let renderer = state
        .renderer()
        .inspect_err(|e| eprintln!("exporting without the GPU: {e}"))
        .ok();
    let mut throttle = Throttle::new(PROGRESS_INTERVAL);
    let mut progress = |p: Progress| {
        if throttle.ready(Instant::now()) {
            let event = ExportProgress {
                id,
                done: p.done,
                total: p.total,
            };
            emit(app, EVENT_EXPORT_PROGRESS, &event);
        }
    };
    let result = run_export(path, document, spec, renderer, cancel, &mut progress);

    // Leave the running list before announcing the outcome: cancelling is over.
    drop(guard);
    match result {
        Ok(report) => {
            let finished = ExportFinished {
                id,
                path: path.display().to_string(),
                notices: report
                    .notices
                    .into_iter()
                    .map(ExportNoticeView::new)
                    .collect(),
            };
            emit(app, EVENT_EXPORT_FINISHED, &finished);
        }
        Err(failed) => {
            let failed = ExportFailed {
                id: Some(id),
                ..failed
            };
            emit(app, EVENT_EXPORT_FAILED, &failed);
        }
    }
}

/// The settings an export of a document to `format` starts with.
#[tauri::command]
pub(crate) async fn export_defaults(
    state: State<'_, AppState>,
    document_id: u64,
    format: ExportFormatId,
) -> Result<ExportSpecDto, String> {
    let document = snapshot(&state, document_id)?;
    Ok(ExportSpecDto::new(&default_spec(format.kind(), &document)))
}

/// Identifiers of the named color spaces `format` can store and tag, for color or gray samples.
#[tauri::command]
pub(crate) async fn export_spaces(format: ExportFormatId, gray: bool) -> Vec<&'static str> {
    let kind = format.kind();
    NAMED_SPACES
        .iter()
        .filter(|space| {
            if gray {
                supports_gray(kind, space)
            } else {
                supports_space(kind, space)
            }
        })
        .filter_map(ColorSpace::id)
        .collect()
}

/// The largest width or height `format` can store, if it has a limit.
#[tauri::command]
pub(crate) async fn export_max_side(format: ExportFormatId) -> Option<u32> {
    max_side(format.kind())
}

/// Start exporting a document to `path` (overwritten). Returns the job id at once; progress
/// and outcome arrive as `export-*` events. Edits made after this call are not exported.
#[tauri::command]
pub(crate) async fn export_document(
    app: AppHandle,
    document_id: u64,
    path: PathBuf,
    spec: ExportSpecDto,
) -> Result<u64, ExportFailed> {
    let state = app.state::<AppState>();
    let document = snapshot(&state, document_id).map_err(|e| ExportFailed {
        id: None,
        code: if e == DOCUMENT_CLOSED {
            "documentClosed"
        } else {
            "internal"
        },
        detail: e,
    })?;
    let spec = spec
        .to_spec(custom_space(spec.format, &document))
        .map_err(|e| ExportFailed::new(None, &e))?;
    let (id, cancel) = state.exports.start();
    let started = ExportStarted {
        id,
        document_id,
        path: path.display().to_string(),
        name: file_name(&path),
    };
    emit(&app, EVENT_EXPORT_STARTED, &started);
    let job_app = app.clone();
    // Not awaited: the command answers with the job id, the job reports through events.
    drop(tauri::async_runtime::spawn_blocking(move || {
        export_job(&job_app, id, &path, &document, &spec, &cancel);
    }));
    Ok(id)
}

/// Ask an export to stop. Its `export-failed` event (code `cancelled`) follows, unless it
/// finished first.
#[tauri::command]
pub(crate) async fn cancel_export(state: State<'_, AppState>, job_id: u64) -> Result<(), String> {
    state.exports.cancel(job_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopshop_core::color::{AlphaMode, ChannelLayout, PixelFormat, SampleType};
    use slopshop_core::{Edit, Layer, LayerContent, RasterImage, Size};
    use std::sync::Arc;

    #[test]
    fn jobs_have_unique_ids_and_cancel_only_while_running() {
        let jobs = ExportJobs::default();
        let (a, token_a) = jobs.start();
        let (b, token_b) = jobs.start();
        assert_ne!(a, b);
        assert!(a > 0 && b > 0, "ids start at 1");

        assert!(jobs.cancel(a));
        assert!(token_a.is_cancelled());
        assert!(!token_b.is_cancelled(), "other jobs keep running");

        jobs.finish(b);
        assert!(!jobs.cancel(b), "a finished job cannot be cancelled");
        assert!(!token_b.is_cancelled());
        assert!(!jobs.cancel(999), "unknown job");
        let (c, _) = jobs.start();
        assert!(c > b, "ids are not reused");
    }

    #[test]
    fn a_job_leaves_the_running_list_even_when_it_panics() {
        let jobs = ExportJobs::default();
        let (id, _) = jobs.start();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = JobGuard { jobs: &jobs, id };
            panic!("encoder bug");
        }));
        assert!(result.is_err());
        assert!(!jobs.cancel(id));
    }

    #[test]
    fn waiting_for_idle_returns_when_the_last_job_ends_or_on_timeout() {
        let jobs = ExportJobs::default();
        assert!(jobs.is_idle());
        assert!(jobs.wait_idle(Duration::ZERO), "nothing to wait for");

        let (a, _) = jobs.start();
        let (b, _) = jobs.start();
        assert!(!jobs.is_idle());
        assert!(!jobs.wait_idle(Duration::from_millis(20)), "times out");
        jobs.finish(a);
        assert!(!jobs.wait_idle(Duration::from_millis(20)), "b still runs");

        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(50));
                jobs.finish(b);
            });
            assert!(
                jobs.wait_idle(Duration::from_secs(10)),
                "woken by the last job"
            );
        });
        assert!(jobs.is_idle());
    }

    #[test]
    fn stopping_cancels_running_jobs_and_later_ones() {
        let jobs = ExportJobs::default();
        let (_, running) = jobs.start();
        assert!(jobs.stop_all());
        assert!(running.is_cancelled());
        assert!(!jobs.stop_all(), "already stopping");

        let (late, token) = jobs.start();
        assert!(
            token.is_cancelled(),
            "a job started while quitting stops at once"
        );
        assert!(jobs.cancel(late), "and is still tracked until it ends");
    }

    #[test]
    fn quitting_during_an_export_waits_for_it_to_remove_its_temporary_file() {
        // Regression: closing the app mid-export left `<name>.slopshop-tmp` behind.
        let document = &raster_document(Size::new(16, 1200), PixelFormat::RGBA8_SRGB);
        let spec = &ExportSpecDto::new(&default_spec(ExportFormatId::Png.kind(), document))
            .to_spec(None)
            .unwrap();
        let dir = temp_path("quit");
        std::fs::create_dir_all(&dir).unwrap();
        let path = &dir.join("quit.png");
        let jobs = &ExportJobs::default();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel::<()>();

        let failed = std::thread::scope(|scope| {
            let job = scope.spawn(move || {
                let (id, cancel) = jobs.start();
                let _guard = JobGuard { jobs, id };
                let mut first = true;
                // Hold the job in its first progress update (the temporary file is being
                // written) until the app has asked to quit.
                let mut progress = |_: Progress| {
                    if std::mem::take(&mut first) {
                        started_tx.send(()).unwrap();
                        resume_rx.recv().unwrap();
                    }
                };
                run_export(path, document, spec, None, &cancel, &mut progress)
            });
            started_rx.recv().unwrap();
            let writing = std::fs::read_dir(&dir).unwrap().count();
            assert!(writing > 0, "the temporary file exists while exporting");
            assert!(!jobs.is_idle());
            assert!(jobs.stop_all());
            resume_tx.send(()).unwrap();
            assert!(jobs.wait_idle(Duration::from_secs(30)), "the job ends");
            job.join().unwrap()
        });

        let left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(failed.unwrap_err().code, "cancelled");
        assert!(left.is_empty(), "files left behind: {left:?}");
    }

    #[test]
    fn progress_is_throttled() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut throttle = Throttle::new(Duration::from_millis(100));
        assert!(throttle.ready(at(0)), "the first update goes through");
        assert!(!throttle.ready(at(50)));
        assert!(!throttle.ready(at(99)));
        assert!(throttle.ready(at(100)));
        assert!(!throttle.ready(at(150)));
        assert!(throttle.ready(at(260)));
    }

    /// A document with one raster layer of `format` (all samples zero).
    fn raster_document(size: Size, format: PixelFormat) -> Document {
        let bytes = size.pixel_count() as usize * format.bytes_per_pixel() as usize;
        let image = RasterImage::from_pixels(size, format, &vec![0; bytes]).unwrap();
        let mut document = Document::new(size);
        let id = document.allocate_layer_id();
        Edit::InsertLayer {
            index: 0,
            layer: Layer {
                id,
                name: "image".to_owned(),
                visible: true,
                opacity: 1.0,
                content: LayerContent::Raster {
                    image: Arc::new(image),
                },
            },
        }
        .apply(&mut document)
        .unwrap();
        document
    }

    #[test]
    fn the_custom_space_is_the_documents_own_unnamed_one() {
        use slopshop_core::color::{RgbPrimaries, TransferFunction};
        let unnamed = ColorSpace {
            primaries: RgbPrimaries::ADOBE_RGB,
            transfer: TransferFunction::Srgb,
        };
        let format = |color_space| PixelFormat {
            layout: ChannelLayout::Rgba,
            sample: SampleType::U16,
            color_space,
            alpha: AlphaMode::Straight,
        };
        let size = Size::new(4, 4);
        let document = raster_document(size, format(unnamed));
        assert_eq!(custom_space(ExportFormatId::Tiff, &document), Some(unnamed));
        let defaults = ExportSpecDto::new(&default_spec(ExportFormatId::Tiff.kind(), &document));
        assert_eq!(defaults.space, "custom");
        let spec = defaults
            .to_spec(custom_space(ExportFormatId::Tiff, &document))
            .unwrap();
        assert_eq!(spec.space, unnamed);
        // EXR only stores linear spaces: its default is a named one.
        assert_eq!(custom_space(ExportFormatId::Exr, &document), None);

        let named = raster_document(size, format(ColorSpace::DISPLAY_P3));
        assert_eq!(custom_space(ExportFormatId::Tiff, &named), None);
    }

    /// A fresh path in the system temporary directory.
    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("slopshop-app-{}-{name}", std::process::id()))
    }

    #[test]
    fn jobs_export_with_the_cpu_compositor_and_report_progress() {
        let size = Size::new(40, 300);
        let document = raster_document(size, PixelFormat::RGBA8_SRGB);
        let path = temp_path("export.png");
        let spec = ExportSpecDto::new(&default_spec(ExportFormatId::Png.kind(), &document))
            .to_spec(None)
            .unwrap();
        let mut updates = Vec::new();
        let report = run_export(
            &path,
            &document,
            &spec,
            None,
            &CancelToken::new(),
            &mut |p| updates.push(p),
        )
        .unwrap();
        let written = std::fs::metadata(&path).map(|m| m.len());
        std::fs::remove_file(&path).ok();
        assert!(written.unwrap() > 0);
        assert!(report.notices.is_empty(), "{report:?}");
        assert_eq!(
            updates.last(),
            Some(&Progress {
                done: 300,
                total: 300
            })
        );
    }

    #[test]
    fn cancelled_jobs_fail_with_the_cancelled_code_and_leave_no_file() {
        let document = raster_document(Size::new(8, 8), PixelFormat::RGBA8_SRGB);
        let path = temp_path("cancelled.png");
        let spec = ExportSpecDto::new(&default_spec(ExportFormatId::Png.kind(), &document))
            .to_spec(None)
            .unwrap();
        let cancel = CancelToken::new();
        cancel.cancel();
        let failed = run_export(&path, &document, &spec, None, &cancel, &mut |_| {}).unwrap_err();
        assert_eq!(failed.code, "cancelled");
        assert!(!path.exists());
    }

    #[test]
    fn export_failures_carry_the_engine_code() {
        let document = raster_document(Size::new(8, 8), PixelFormat::RGBA8_SRGB);
        // A directory that does not exist: the file cannot be created.
        let path = temp_path("missing-dir").join("out.png");
        let spec = ExportSpecDto::new(&default_spec(ExportFormatId::Png.kind(), &document))
            .to_spec(None)
            .unwrap();
        let failed = run_export(
            &path,
            &document,
            &spec,
            None,
            &CancelToken::new(),
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(failed.code, "io");
        assert!(!failed.detail.is_empty());
    }
}
