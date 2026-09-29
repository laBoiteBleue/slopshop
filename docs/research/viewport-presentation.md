# Research: viewport presentation (input for ADR 0002)

Status: **research brief**, no decision taken yet. Produced by a multi-agent research run with
adversarial fact-checking of its claims; local measurements were made with a small standalone
Tauri benchmark (`ipcbench`, not yet in the repository).

*Research as of 2026-09-29. Tauri 2.12.0 (latest stable, 2026-09-26; 3.0 alphas exist), wry 0.57, wgpu 30.0.1, WebView2 154. Sources older than about 18 months (before about 2025-04) are marked **[old]**. All local measurements come from one Windows 11 machine (RTX 5070 Ti, Core Ultra 7 265KF).*

## 1. Summary

- On Windows, today's path (ipc::Response and putImageData) cannot reach 60 fps at 4K. A 33 MB frame took about 282 ms to transfer (around 9 ms/MiB). A custom URI scheme was no faster, and neither is `ipc::Channel`.
- WebView2's SharedBuffer API can be reached from Tauri 2.12 today without forking wry. A 4K frame goes from shared memory to screen in a median of 7–11 ms, about 20–40x faster. This excludes the GPU readback, and the tail (p90) reaches 16–24 ms. macOS and Linux have no equivalent API, and their transfer speed has not been measured.
- A native wgpu surface in an opaque "slot" (a child window on top of the webview, covering only the canvas area) is the one native topology with a 2026 reference on Windows and macOS: photo-imager PR #5. That project is young and its claims are not independently verified. The cost is that no DOM element can draw over the canvas. Tauri offers no supported Linux path.
- The most relevant prior art, Graphite (Rust, wgpu, Svelte), left Tauri for CEF offscreen rendering. No shipping app sends full 4K frames through webview IPC at 60 fps.
- Recommendation: fix the cheap readback waste now, put presentation behind a trait, and measure on all three operating systems. Next, ship SharedBuffer on Windows. Then run a time-boxed spike of the native slot on Windows and macOS. Keep the readback path as the Linux path and as the fallback everywhere.

## 2. Options

Frame size: 3840×2160 RGBA8 is 33,177,600 bytes. At 60 fps that means 16.67 ms per frame and about 2.0 GB/s.

| Option | How it works | Per-frame cost at 4K | Win / macOS / Linux: support and risk | Complexity | Reversibility | Key evidence |
|---|---|---|---|---|---|---|
| **A. Frames over IPC (today)** | GPU render, readback, `tauri::ipc::Response` (ArrayBuffer), then `putImageData` | **Win: ~282 ms measured** (custom protocol 209–282 ms). macOS: about 20–30 ms, extrapolated from one unscientific data point (5 ms per 10 MB, Dec 2024 **[old]**). Linux: unknown. Add readback (estimated 2–4 ms, not measured) and a fresh 33 MB Vec per frame (8.5 ms measured). | Works everywhere. On Windows it is unusable for interaction. If the custom-protocol fetch fails (for example because of CSP), Tauri silently falls back to postMessage and sends the frame as a JSON number array. | Exists | Baseline | Local ipcbench; [tauri #11915](https://github.com/tauri-apps/tauri/discussions/11915) **[old]**; tauri `scripts/ipc-protocol.js`, `ipc/channel.rs` |
| **B. Shared memory / cheaper transport** | **Windows:** WebView2 `CreateSharedBuffer` + `PostSharedBufferToScript` via `with_webview` → `environment()`/`controller()` (webview2-com 0.39). Post once, write each frame into it, signal "frame ready" with an empty invoke. **Linux (theoretical):** WebKitGTK web process extension, loaded through Tauri's `extensions_path`, receives a memfd over WebKitUserMessage and exposes it with `jsc_value_new_array_buffer` (no copy). **macOS:** no public API; best available is a custom protocol or invoke. | **Win, measured:** Rust copy 1.2–1.4 ms, notify 0.6–0.8 ms, end to end 7–11 ms with putImageData and 6.6–10.5 ms with WebGL2 (medians; p90 up to 24 ms). Readback excluded. Estimated full pipeline about 10–18 ms (**unverified**). Linux route: untested. macOS: same as A. | Win: works with no fork. A wrapper PR was declined in Dec 2023 **[old]**; direct COM through `controller()` is the maintainer-endorsed route. macOS: none. Linux: plausible, not proven. The buffer must be kept alive and closed explicitly (lifetime and `Close()`). | Low–medium on Windows (unsafe COM). High on Linux (extension .so). | High: sits behind a transport interface and changes neither UI nor renderer | Local ipcbench; [SharedBuffer spec](https://github.com/MicrosoftEdge/WebView2Feedback/blob/main/specs/SharedBuffer.md); [ICoreWebView2Environment12](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2environment12); [wry #1110](https://github.com/tauri-apps/wry/issues/1110); jsc [`new_array_buffer`](https://webkitgtk.org/reference/jsc-glib/stable/ctor.Value.new_array_buffer.html) |
| **C1. Native wgpu surface in an opaque slot** | A native child sits above the webview and covers only the canvas rectangle: a child HWND (`WS_EX_NOACTIVATE`, `HWND_TOP`) on Windows, an NSView with a CAMetalLayer ordered above the WKWebView on macOS. Svelte sends the slot rectangle in CSS px and Rust repositions the child. wgpu draws through `SurfaceTargetUnsafe::RawHandle` / `CoreAnimationLayer`. Input and pen go to the native view; keyboard focus stays in the webview (`MA_NOACTIVATE`). | No CPU readback and no transfer, only GPU composite and present (composite estimated at 1–3 ms, not measured in SlopShop). nemo PR #1337 (macOS fixture): median frame 16.66 ms (vsync), 0 readbacks. | Win / macOS: one 2026 reference implementation (photo-imager). No `unstable`, no transparency, no private API. Linux: **no Tauri path**; hand-built X11 child or `wl_subsurface` from GTK3 only. Main risks: **DOM cannot draw over the canvas** (menus, tooltips, modals, brush cursor); lag between DOM layout and child position during resize; IME, drag-and-drop and accessibility with a child HWND unverified. | Medium–high per OS (Win32 + AppKit, DPI, input, focus, overlays). Linux high. | Medium: the renderer stays view→target, but UI design changes (overlays) are the sticky part | [photo-imager PR #5](https://github.com/Lonshaus/photo-imager/pull/5); [nemo PR #1337](https://github.com/mysteropodes/nemo/pull/1337); [WM_DPICHANGED_AFTERPARENT](https://learn.microsoft.com/en-us/windows/win32/hidpi/wm-dpichanged-afterparent) |
| **C2. Transparent webview over a native surface** | wgpu surface on the window (or a sibling view below); the webview is transparent and the canvas area is a transparent hole in it | Same as C1 (no readback). On macOS a transparent window costs about 8x GPU power even when the page is static (~620 vs ~75 mW). | Win: works once the parent's `WS_CLIPCHILDREN` is cleared (wry example, libmpv plugin "fully tested"); WebView2 background alpha must be 0 or 255; evidence is 2024–early 2025. macOS: needs `macOSPrivateApi`, which means App Store rejection; black base on 15.7 (wry #1867, open, maintainer cannot reproduce on 15.6.1); power cost from #15471 (open, upstream). Linux: flicker (#9220, closed not planned **[old]**); wry example is X11-only; GtkOverlay + GtkGLArea hack is GL, not wgpu/Vulkan. | High, fragile | Medium | [wry examples/wgpu.rs](https://github.com/tauri-apps/wry/blob/dev/examples/wgpu.rs); [wry #1331](https://github.com/tauri-apps/wry/issues/1331) **[old]**; [wry #1867](https://github.com/tauri-apps/wry/issues/1867); [tauri #15471](https://github.com/tauri-apps/tauri/issues/15471); [tauri #9220](https://github.com/tauri-apps/tauri/issues/9220) **[old]**; [tauri-plugin-libmpv](https://github.com/nini22p/tauri-plugin-libmpv) |
| **D. Hybrid: tiles or dirty rects into a WebGL2/WebGPU canvas** | Keep the last frame as a texture in the webview, shift it on pan and scale it on zoom, fetch only newly exposed strips, send a low-resolution frame during interaction, and send dirty rects while painting. Still uses A or B for transport. | Display side is cheap: a 512×512 rect costs 0.1 ms with putImageData and 0.3–0.6 ms with texSubImage2D. Transfer of a 1 MiB tile: ~9.6 ms over Windows invoke, which is too slow; a small fraction of the ~7–11 ms full-frame cost over SharedBuffer. A full 4K upload costs 5.4–8.7 ms (WebGL2 / WebGPU). | WebGL2 is available in all three webviews. WebGPU: WebView2 yes; WKWebView likely on macOS 26+ (from WebKit source, not tested at runtime); **WebKitGTK no**. | Medium (frontend GPU code, invalidation logic) | Medium. ADR 0001 tension if it grows beyond reprojecting the last frame. | Local ipcbench; WebKit `PlatformEnable.h`, `WebKitFeatures.cmake`; [WebKitGTK 2.54 notes](https://webkitgtk.org/2026/09/16/webkitgtk-2.54-highlights.html) |
| **E1. CEF offscreen UI composited into native wgpu (Graphite)** | The web UI renders offscreen in CEF; shared textures (D3D11 / IOSurface / DMA-BUF) are imported into wgpu; one WGSL pass composites UI, viewport and overlays | No viewport readback. UI changes cost a texture import, with a shared-memory software fallback. | Leaves Tauri's system webview (Tauri's CEF runtime is unreleased). Graphite still disables accelerated UI on Linux by default, and IME, tablet and shortcut issues remain open. Chromium ships with the app. | Very high | Low | [Graphite desktop](https://github.com/GraphiteEditor/Graphite/tree/master/desktop); [LWN](https://lwn.net/Articles/1051242/); [cef-rs #192](https://github.com/tauri-apps/cef-rs/issues/192) |
| **E2. Engine inside the page (wasm + WebGPU/WebGL)** | Figma, Photopea, Penpot and Photoshop web: the canvas renders in the page and nothing is transferred | Transport-free | Contradicts ADR 0001 (Rust engine). No WebGPU on WebKitGTK. | Very high | Low | [Figma WebGPU](https://www.figma.com/blog/figma-rendering-powered-by-webgpu/); [Penpot](https://penpot.app/blog/penpots-new-rendering-system/) |
| **E3. Separate native viewport window** | wgpu in its own top-level window or process, next to the webview window (donder, liminal-hq/jar, Graphite's 2025 interim step) | No readback | Works on Linux, including Wayland, where the others do not. Poor editor UX (docking, focus, z-order). | Medium | High | [donder](https://github.com/EddieRydell/donder); [jar #94](https://github.com/liminal-hq/jar/issues/94) |
| **E4. Fully native UI (GPUI, egui)** | Drops the webview (Zed, Rerun, Cap's GPUI rewrite) | Frame time fully under app control | Rebuilds text input, IME and every widget | Very high | Very low | [Cap GPUI](https://github.com/CapSoftware/Cap/tree/main/apps/desktop-gpui); [Zed](https://zed.dev/blog/videogame) **[old]** |

### Notes on the evidence

- **ADR 0002 alternative 4 is partly out of date.** It says shared memory is "not exposed by webviews in a portable way". That remains true for portability, but WebView2 does expose it and it works from Tauri 2.12.
- **ADR 0002 alternative 2 (custom URI scheme) is measured and brings no gain on Windows.** wry builds the body with `SHCreateMemStream`, which adds another copy.
- **WebView2 TextureStream is ruled out.** It is still prerelease/experimental and NV12-only ([#3591](https://github.com/MicrosoftEdge/WebView2Feedback/issues/3591) open), which is unsuitable for pixel-accurate editing.
- **SharedArrayBuffer does not help.** It shares memory only between JS threads, never with Rust, and it needs COOP/COEP headers.
- **About photo-imager.** The repo was created 2026-09-27, has 0 stars, and the PR was co-authored by an AI. The code matches the described design, but "it works" is the author's claim. Treat it as a reference design, not as proven practice.
- **Cap is prior art for option A in production.** It sends frames over a WebSocket to WebGPU and measured 406.6 MB/s at full resolution, so it defaults to a half-resolution preview. It is now rewriting its desktop app in GPUI with no webview.
- **Electrobun (`<electrobun-wgpu>`) is productised prior art for C1's overlay problem.** A DOM anchor keeps the native view's rectangle in sync, and "mask selectors" cut holes in the native layer where HTML must show on top. On Linux, masks are unavailable in transparent windows.
- **Maintainer position.** FabianLars (Dec 2024 **[old]**) called native embedding "kindaaa possible… layers more so than elements/views". On tauri #15213 (2026-04-14) he said "rust-based wgpu already works" and that a `<electrobun-wgpu>`-like element is "not planned yet". There is no first-party API in 2.12 or in the 3.0 alphas.
- **Linux future.** Tauri's GTK4 migration (#14684) targets 3.0 and is not merged. The 3.0 alphas still depend on GTK3/WebKitGTK 4.1 for the webview. Flutter's wl_subsurface technique is an **open, unmerged** PR ([#189584](https://github.com/flutter/flutter/pull/189584)), not shipped practice.

## 3. What to measure in SlopShop before deciding

1. **Transport on macOS and Linux.** Rerun the ipcbench harness unchanged on macOS 15 and 26, and on Linux with WebKitGTK 2.50–2.54 under both X11 and Wayland. Sizes: 1, 8 and 33 MB. Routes: invoke with Response, and custom protocol. Report p50, p90 and p99. This alone decides whether A is acceptable outside Windows.
2. **wgpu readback of a 4K frame on each OS** in `slopshop-render`, with pooled output and readback buffers. Measure `map_async` plus poll latency, and whether rendering frame N+1 can overlap transferring frame N.
3. **GPU composite time** at 4K for documents of 10, 50 and 100+ MP, at fit, 100% and 400% zoom, with 1 to many layers. This number applies to every option.
4. **Windows SharedBuffer end to end inside SlopShop.** Run a 60 s pan and a 60 s paint session, with 1 and 2 frames in flight. Report p50, p99 and dropped frames. Also check memory after 100 resizes (buffer repost and Close) and threading correctness (COM on the UI thread, writes from another thread).
5. **Input-to-photon latency for a brush dab.** Take one timestamp at the pointer event and one at present. Compare A, B and C1 (C1 via the spike).
6. **True display cost.** Use a trace-based measure of the GPU upload behind putImageData, because the synchronous 2.2–2.6 ms figure likely under-counts it. Compare with WebGL2 texSubImage2D.
7. **Dirty-rect volume.** Measure bytes per frame while painting at 4K with typical brush sizes (to size the transport for D).
8. **Pass/fail criteria for a C1 spike (Windows and macOS):**
   - resize lag between DOM and child: at most one frame, with no visible gap or flicker during live resize;
   - moving the window across monitors with different DPI;
   - pen input (WM_POINTER on Windows, trackpad and tablet on macOS);
   - keyboard shortcuts still working after clicking the canvas;
   - IME, drag-and-drop from the webview onto the canvas, and the file-drop handler;
   - accessibility;
   - GPU power use at idle;
   - how many existing UI elements currently overlap the canvas (context menus, tooltips, brush cursor).
9. **Capabilities.** Does `navigator.gpu` exist in a Tauri WKWebView on macOS 26? Does WebGL2 run on Linux with the target drivers?
10. **Watch the console for the fallback warning.** Tauri logs "IPC custom protocol failed" when it falls back to postMessage, and that fallback silently makes every frame catastrophically slow.

## 4. Recommendation and staged path

**Now (days, low risk, fully reversible)**
- Pool the output and readback buffers in `slopshop-render` (it currently allocates them per frame; see `lib.rs` around line 245), and read back into a persistent buffer instead of `extend_from_slice` into a new Vec. Measured: a fresh 33 MB Vec costs 8.5 ms versus 1.3 ms for a warm buffer.
- Put presentation behind a `Presenter` interface with two kinds of implementation: readback presenters that deliver a frame over a transport, and surface presenters that draw into a native target. The renderer's view→frame API stays as ADR 0002 already intends.
- Run measurements 1–3 above on all three operating systems.

**Next (Windows first, because A is worst there)**
- Implement B (SharedBuffer) as the Windows readback transport: allocate once and repost on resize, keep 2 slots in flight, and send a small "frame ready" invoke.
- Add D-lite in the webview. It is pure presentation, so it stays within ADR 0001:
  - keep the last frame as a WebGL2 texture;
  - shift or scale it immediately on pan and zoom, then replace it when the real frame arrives;
  - send dirty rects while painting.
- This gives a usable interactive viewport on Windows without changing the UI's design rules.

**Then (time-boxed spike, about 1–2 weeks): C1 on Windows and macOS**
- Use photo-imager's topology: opaque slot, no transparency, no `unstable`, no private API.
- Judge it against the pass/fail criteria in item 8.
- If it passes, adopt C1 as the primary presenter on Windows and macOS. Move canvas overlays (brush cursor, selection outlines) into wgpu. Use native menus, or hide/shrink the native view, for popups that must cover the canvas.
- Keep the readback path as the fallback: B on Windows, A on macOS.

**Linux**
- Stay on the readback path (A, plus D-lite).
- If the Linux ipcbench is poor, the next step is the WebKitGTK web process extension (memfd via Tauri's `extensions_path`, untested).
- Do not start an X11-child or wl_subsurface effort until Tauri's GTK4 work (3.0) lands or Linux user demand justifies it.

**Do not pursue now**
- C2: fragile on Linux, private API and 8x power on macOS.
- E1, E2, E4: leave the stack or contradict ADR 0001.
- TextureStream: NV12-only.

**Conditions that would change this**
- If macOS and Linux ipcbench show 4K under about 10 ms p99, A is enough there, and C1 becomes a Windows/macOS latency optimisation rather than a necessity.
- If SharedBuffer tails stay above 16.7 ms, or it leaks or crashes over long sessions, go straight to C1 on Windows.
- If brush input-to-photon latency with readback is judged too high (readback plus transfer adds at least one frame), C1 becomes mandatory on Windows and macOS.
- If the C1 spike fails on resize sync or overlays, stay on B/D, and consider Electrobun-style masks or C2 on Windows only.
- If Tauri ships any of the following, reassess C, D and E1 on Linux:
  - a CEF runtime with shared-texture offscreen rendering;
  - a first-party native-view API;
  - GTK4 with Wayland child embedding (wry PR #1767);
  - WebGPU in WebKitGTK.
- If composite time for 100+ MP documents at 4K exceeds the budget on its own, transport is no longer the bottleneck. Invest in renderer tiling or caching first.

## 5. Open questions

- What are the real invoke and custom-protocol throughputs on WKWebView (macOS 15/26) and WebKitGTK (2.50–2.54)? The only macOS number is second-hand from Dec 2024 **[old]**; Linux has none.
- Does SharedBuffer stay reliable over long sessions? Open points: memory release on repost, READ_ONLY vs READ_WRITE, COM threading. Maintainer hearsay called it "weirdly slow" (Dec 2024 **[old]**), which contradicts the local measurements. Did that report concern per-frame allocation?
- How much does the GPU readback of a 4K frame really cost in our renderer, and can it be pipelined without adding more than one frame of brush latency?
- How far does the DOM lag the native child during live resize (WebView2 and WKWebView)? Would DeferWindowPos or CAMetalLayer `presentsWithTransaction` help? Nothing has measured this.
- What is the best practice for overlays over a native canvas: native menus, wgpu-drawn overlays, or temporarily hiding the view? No documented pattern exists; Electrobun's masks are the only productised approach.
- Does an opaque child HWND above WebView2 break IME, drag-and-drop onto the canvas, accessibility or file drop? bevy #17686 (Feb 2025, about 19 months old, still open) shows input and focus pitfalls in a related topology.
- Is the Linux web process extension (memfd plus `jsc_value_new_array_buffer`) practical under Tauri? The API chain exists, but nobody has tested it end to end. `extensions_path` applies per WebContext and only before the first load.
- Does `navigator.gpu` actually exist in a Tauri WKWebView on macOS 26, and do entitlements or lockdown mode matter?
- What exactly was Graphite's "insurmountable technical incompatibility" with Tauri? There is no post-mortem; tauri #9220 (Linux flicker) is the circumstantial suspect.
- When will Tauri 3.0 ship GTK4 in the webview stack, and when will a CEF runtime ship? Both are unreleased and have no dates.
- Is photo-imager's approach robust beyond its author's machines? It is a very young, single-source reference.