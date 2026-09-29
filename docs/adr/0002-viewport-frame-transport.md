# 0002 — Viewport presentation

Status: **accepted direction** (2026-09-29): performance first. The native GPU surface is the
target; frames over IPC remain the fallback. The target still has to pass a time-boxed spike.

## Context

The engine renders on the GPU in Rust; the UI lives in a webview. Pixels must reach the screen
with low latency, without ever sending document-sized buffers through IPC. The maintainer's
requirement is the most performant option.

Research ([viewport-presentation.md](../research/viewport-presentation.md)) and measurements in
the app show that the transport, not the rendering, dominates: e.g. 3.4 ms engine render vs
29.6 ms request-to-screen for a ~1 MP viewport on Windows, and ~282 ms to move a 4K frame
through Tauri IPC.

## Decision

1. **Target: a native wgpu surface in an opaque slot** over the canvas area (child window on
   Windows, `CAMetalLayer` view on macOS): no GPU readback, no transfer, presentation at display
   rate. Canvas overlays (brush cursor, selection outlines) are drawn by the engine; popups that
   must cover the canvas are handled explicitly (native menus or shrinking the slot).
2. **Fallback: frames over IPC** (today's path), with WebView2 `SharedBuffer` on Windows as the
   first improvement, used where the native surface is unavailable (Linux today) or fails.
3. Presentation sits behind one interface (`Presenter`): the renderer keeps producing a view of
   the document; only where the pixels go changes.

## Current state

The frame path is implemented: viewport-sized RGBA8 frames with a header (size, revision,
zoom, origin, render time) over `tauri::ipc::Response`, drawn with `putImageData`, and the last
frame is reprojected with a CSS transform during pan/zoom so navigation stays immediate.

## Spike pass/fail criteria (native surface, Windows then macOS)

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

- **Transparent webview over a native surface**: fragile on Linux, needs private API and costs
  ~8× GPU power on macOS. Rejected.
- **Tiles to a WebGL/WebGPU canvas in the webview**: keeps rendering partly in the frontend
  (against ADR 0001) and still pays the transfer; only as a presentation aid.
- **Leaving Tauri (CEF offscreen, or a fully native UI)**: much larger rewrite. Rejected for now.

## Consequences

- Linux keeps the frame path until Tauri/GTK4 offers a sound embedding.
- UI overlays on top of the canvas become an engine concern (drawn in wgpu) on the native path.
- The view API (document view → pixels) does not change, so rendering work is not wasted.
