//! The canvas on macOS (ADR 0002): an AppKit view under the webview, which the engine draws into
//! through a Metal layer, the page transparent over the canvas area as on Windows.
//!
//! The only part of the app that talks to AppKit directly, hence its `unsafe` blocks: a raw
//! pointer from Tauri turned into the view it is, and a surface made from raw handles.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::mpsc;

use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSView, NSWindowOrderingMode};
use slopshop_render::present::Presenter;
use tauri::{AppHandle, Manager, WebviewWindow};
use wgpu::rwh::{AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle};

use crate::AppState;

/// A presenter drawing into a new view under `window`'s webview, filling the window's content
/// and following its size. AppKit is used on the main thread only: the view and its surface are
/// made there, and this, called off it, waits for them.
pub(crate) fn create_presenter(
    app: &AppHandle,
    window: &WebviewWindow,
) -> Result<Presenter, String> {
    let (sender, receiver) = mpsc::channel();
    let app = app.clone();
    window
        .with_webview(move |webview| {
            // The receiver waits below; if it is gone, nobody wants the answer.
            let _ = sender.send(present_under(&app, webview.inner()));
        })
        .map_err(|e| e.to_string())?;
    receiver
        .recv()
        .map_err(|_| "the main thread gave no window surface".to_owned())?
}

/// On the main thread: the view, put under the WKWebView `webview` in its parent (the window's
/// content view, made by wry), and a presenter for it.
fn present_under(app: &AppHandle, webview: *mut c_void) -> Result<Presenter, String> {
    let mtm = MainThreadMarker::new().ok_or("AppKit used off the main thread")?;
    let webview = NonNull::new(webview.cast::<NSView>()).ok_or("no webview")?;
    // SAFETY: Tauri's `PlatformWebview::inner` is the window's WKWebView, an NSView subclass,
    // alive while its window is, and the window outlives this call; it is only used here, on the
    // main thread (checked above).
    let webview: &NSView = unsafe { webview.as_ref() };
    // SAFETY: the bindings mark `superview` unsafe because a view's parent may change; it is
    // read once, here on the main thread, and the parent is retained for as long as it is used.
    let parent = unsafe { webview.superview() }.ok_or("the webview is not in a view")?;
    let view = NSView::initWithFrame(NSView::alloc(mtm), parent.bounds());
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    view.setWantsLayer(true);
    // Under the webview: the page stays in front, receiving every event, as on Windows.
    parent.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, Some(webview));
    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(RawDisplayHandle::AppKit(AppKitDisplayHandle::new())),
        raw_window_handle: RawWindowHandle::AppKit(AppKitWindowHandle::new(
            NonNull::from(&*view).cast(),
        )),
    };
    let state = app.state::<AppState>();
    let renderer = state.renderer()?;
    // SAFETY: the handles describe `view`, a valid NSView, on the main thread as wgpu needs to
    // attach its Metal layer; the view outlives the surface: its parent keeps it (it is never
    // removed) for as long as the main window exists, which is the app's whole run.
    let surface =
        unsafe { renderer.instance().create_surface_unsafe(target) }.map_err(|e| e.to_string())?;
    renderer.presenter_for(surface).map_err(|e| e.to_string())
}
