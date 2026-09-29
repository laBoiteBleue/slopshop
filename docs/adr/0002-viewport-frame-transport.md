# 0002 — Viewport presentation

Status: **accepted** (2026-09-29), amended 2026-09-30 after the Windows spike: performance
first. The engine presents natively where the platform allows it; frames over IPC remain the
fallback. Windows presents natively; macOS and Linux still use frames.

## Context

The engine renders on the GPU in Rust; the UI lives in a webview. Pixels must reach the screen
with low latency, without ever sending document-sized buffers through IPC. The maintainer's
requirement is the most performant option.

Research ([viewport-presentation.md](../research/viewport-presentation.md)) and measurements in
the app show that the transport, not the rendering, dominates: e.g. 3.4 ms engine render vs
29.6 ms request-to-screen for a ~1 MP viewport on Windows, and ~282 ms to move a 4K frame
through Tauri IPC.

## Decision

1. **Windows: the engine presents to the main window's surface, under a transparent webview.**
   wgpu (DX12, flip-model swapchain) draws on the Tauri window itself; the webview's default
   background is transparent and the page leaves the canvas area see-through, so the
   swapchain shows there while every other panel stays opaque above it. No GPU readback, no
   transfer, presentation at display rate. The UI sends the canvas rectangle with each present
   request; input, file drops, shortcuts, IME, accessibility and DOM overlays (drop hint,
   error messages, future menus) keep working unchanged because the webview still receives
   everything.
2. **macOS (planned): a native `CAMetalLayer` view in an opaque slot** over the canvas area.
   A transparent WKWebView over Metal costs about 8× the GPU power, so the Windows layout is not
   reused there. Canvas overlays are then drawn by the engine, and popups that must cover the
   canvas are handled explicitly. Needs a Mac to develop and test.
3. **Fallback: frames over IPC**, used on Linux (no sound embedding in WebKitGTK yet), when
   `SLOPSHOP_PRESENTER=frames` is set, and automatically when the native surface cannot be
   created at startup.
4. Presentation sits behind one interface (`slopshop_render::present::Presenter`): the renderer
   produces a view of the document; only where the pixels go changes.

## Current state

- **Native (Windows, default):** `Renderer::present_view` composites the view and copies it into
  the swapchain back buffer (rows padded to 256 bytes, no shader change), clears the rest of the
  surface to the pasteboard color, and presents (vsync, one frame of latency at most). The UI
  requests a present on every document or view change; there is no CSS reprojection.
- **Frames (fallback):** viewport-sized RGBA8 frames with a header (size, revision, zoom, origin,
  render time) over `tauri::ipc::Response`, drawn with `putImageData`; the last frame is
  reprojected with a CSS transform during pan/zoom so navigation stays immediate.
- `SLOPSHOP_PRESENTER=frames|window` overrides the platform default (`window` elsewhere than on
  Windows is for experiments only).

## Windows spike results (2026-09-30)

Measured in the app on the maintainer's machine (RTX 5070 Ti, Windows 11), canvas area about
1020 × 720 physical pixels, 233 MP document:

| Path | Engine time | Request to screen |
| --- | --- | --- |
| Frames over IPC | ~6 ms (1440p) | 30 ms and more; ~282 ms for a 4K frame over IPC alone |
| Native surface | ~0.6 ms | ~1.8 ms |

Also: the engine's compute pass is unchanged and all GPU tests pass on DX12, which is now the
Windows backend (`WGPU_BACKEND` overrides); DX12 reconfigures a swapchain in 1–2.5 ms against
3.5–18 ms for Vulkan on the same machine.

Validated visually by the maintainer: correct placement and colors, smooth pan and zoom, UI
panels drawn above the surface. Still to check systematically: resizing (one stretched frame
at most is expected while the page relayouts), two monitors with different DPI,
semi-transparent DOM overlays over the canvas (wry#1331 reports grey instead of blending on
some systems), idle GPU power, and behavior after WebView2 runtime updates (transparency has
regressed before). If any of these fails, the fallback is `frames`, and the next option is an
opaque native child window (below).

The layout was chosen over an opaque native child window (the original target) because it
passes the criteria by construction: file drops, shortcuts, IME and overlays need no native
code, and it needs no `unsafe` code or new dependency. The child window stays in reserve: it
would allow independent flip / hardware overlay planes (DWM must compose the webview over the
swapchain in the current layout), at the cost of native input, OLE drag-and-drop and focus
handling written in `unsafe` Win32 code.

## Spike pass/fail criteria (native surface; kept for macOS and later re-checks)

- Resize: at most one frame of lag between the DOM layout and the native slot, no flicker.
- Monitors with different DPI; window moves between them.
- Input: mouse, pen (WM_POINTER / tablet), wheel/pinch; keyboard shortcuts still reach the app
  after clicking the canvas.
- **File drag-and-drop onto the canvas** (drops land on the native child, not the webview) and
  the tab bar drop target.
- IME, accessibility, idle GPU power.
- Which UI elements overlap the canvas today (menus, tooltips, drop hints) and how each is
  handled.

## Alternatives

- **Transparent webview over a native surface**: chosen on Windows (see above). Rejected on
  Linux (fragile) and macOS (private API, ~8× GPU power).
- **Opaque native child window over the canvas on Windows** (the original target): best
  presentation path (hardware overlay planes), but input, file drops and focus must be
  reimplemented natively, and DOM overlays cannot cover it. Kept in reserve.
- **Rendering a margin around the viewport / a low-resolution backdrop** (frames path) to hide
  the edges revealed during navigation: costs more transfer per frame; superseded by native
  presentation on Windows, still an option for the fallback.
- **Tiles to a WebGL/WebGPU canvas in the webview**: keeps rendering partly in the frontend
  (against ADR 0001) and still pays the transfer; only as a presentation aid.
- **Leaving Tauri (CEF offscreen, or a fully native UI)**: much larger rewrite. Rejected for now.

## Consequences

- Linux keeps the frame path until Tauri/GTK4 offers a sound embedding; macOS keeps it until
  the Metal view is built and tested on a Mac.
- On Windows, UI overlays stay in the DOM. On macOS (opaque view), overlays on top of the canvas
  become an engine concern (drawn in wgpu).
- Two presentation paths must be maintained and tested (CI runs the frames path; the native path
  needs a Windows desktop session).
- The view API (document view → pixels) does not change, so rendering work is not wasted.
