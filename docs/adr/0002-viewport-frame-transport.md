# 0002 — Viewport frame transport

Status: **proposed** — good enough to validate the stack; must be re-evaluated before building
interactive tools.

## Context

The engine renders on the GPU in Rust; the UI lives in a webview. Pixels must reach the screen
with low latency, without ever sending document-sized buffers through IPC.

## Decision (current)

The engine renders a **viewport-sized** frame (device pixels, RGBA8 sRGB) on the GPU, reads it
back, and returns it as raw binary via `tauri::ipc::Response` (an `ArrayBuffer` in JS, no JSON
or base64). The UI draws it into a 2D canvas with `putImageData`. At most one frame is in
flight; newer requests supersede pending ones.

## Alternatives

1. **Native wgpu surface composited with the webview** (the webview is transparent over, or
   next to, a native GPU surface). No GPU→CPU→webview copies, best latency; but
   platform-specific window composition, input routing and overlay issues, and less mature in
   Tauri.
2. **Custom URI scheme** serving frames (`slopshop://frame`): same copies as today, allows
   streaming/HTTP caching semantics; no clear benefit yet.
3. **Tiles to the webview, composited with WebGPU/WebGL in JS**: fewer bytes on pan, but moves
   rendering logic into the frontend, against ADR 0001.
4. **Shared memory between processes**: not exposed by webviews in a portable way today.

## Consequences

- Cost per frame: one GPU readback plus one copy into the webview. Fine for a fitted view and
  occasional updates; likely too slow for 60 fps painting on 4K+ displays.
- Measure before deciding (frame time is shown in the status bar). Option 1 is the likely
  long-term target; the renderer API (view → frame) stays the same either way, only the
  presentation path changes.
