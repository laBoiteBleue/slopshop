# SlopShop

> A modern, open-source image editor for creating professional-grade slop — non-destructive,
> GPU-accelerated, and AI-native.

The name is a joke. The engineering is not.

> [!WARNING]
> **SlopShop is at a very early stage.** It is not usable for editing images yet. The repository
> currently contains the project foundations and a minimal desktop shell that validates the
> stack and the architecture. Everything below marked as a *goal* is **not implemented**.

## Vision

SlopShop aims to become a modern alternative to professional editors such as Photoshop:

- **Open source and cross-platform** (Windows, macOS, Linux).
- **Fast and GPU-first**: rendering and processing run on the GPU whenever it makes sense.
- **Non-destructive by design**: original pixels stay intact; edits are layers/nodes with
  parameters that can be changed, reordered, disabled or undone at any time.
- **Very large images**: documents larger than RAM or VRAM, through tiles, caches, mip levels
  and partial recomputation.
- **Local AI, native but optional**: AI tools integrated as non-destructive layers, running
  locally, never required to use the editor.
- **Extensible architecture**, and an interface familiar to users of professional editors.

Users will mostly see a classic layer stack; internally, the document model is designed to be
able to evolve into a **DAG** (directed acyclic graph) of operations.

## High-definition AI: the strategic bet

Generative models usually work at around 1–2 megapixels. Professional images are often 8K, 12K,
50 or 100 megapixels. Today, using a generative tool on such an image usually means accepting a
lower-resolution result, or upscaling it afterwards.

**Goal:** apply generative tools to very high-resolution images **without permanently reducing
their resolution** — producing results that are globally coherent *and* detailed at the
document's native resolution, while strictly protecting the pixels outside the edited area.
A plain AI upscale of a low-resolution result is explicitly *not* the goal.

The strategy is deliberately **not decided yet**. Candidate approaches (region-of-interest
processing, multi-resolution generation, image pyramids, overlapping tiles with shared context,
iterative refinement, re-injection of the original high frequencies, strict masking, caching of
high-resolution results…) are documented and will be compared experimentally before an
architecture is chosen. See [`docs/research/hd-generative-ai.md`](docs/research/hd-generative-ai.md).

AI operations will be non-destructive nodes storing their parameters, prompt, model and
version, seed, mask, source region, dependencies and cached result. An upstream change can
*invalidate* a generation without recomputing it automatically, and expensive generations can be
previewed first and rendered at full definition later.

## Stack

| Layer          | Technology                                      |
| -------------- | ----------------------------------------------- |
| Engine         | Rust (workspace of crates, runs headless)       |
| GPU            | [wgpu](https://wgpu.rs) (WebGPU API, native backends: Vulkan, Metal, DX12) |
| Desktop shell  | [Tauri 2](https://tauri.app)                    |
| User interface | [Svelte 5](https://svelte.dev) + TypeScript (Vite) |

## Architectural principles

- **The engine owns the document.** Image and document logic live in Rust crates, never in the
  frontend. The UI displays views and sends intents.
- **Headless first.** The engine works without the UI: tests, CLI, batch processing, headless
  rendering.
- **No big buffers over IPC.** Only viewport-sized display frames go to the UI, as raw binary.
- **Every document change is an undoable edit.** No hidden mutation.
- **Explicit pixels.** Never assume sRGB 8-bit; formats and color spaces are explicit, and
  conversions are never silent.
- **Never assume an image fits in memory.** Tiles, regions of interest, caches and mip levels
  are part of the design from day one.

Details: [`docs/architecture.md`](docs/architecture.md), the decision records in
[`docs/adr/`](docs/adr/) and the [roadmap](docs/roadmap.md).

## What exists today

- A Rust engine with a minimal document model (procedural fill layers), reversible edits with
  undo/redo, tiling geometry and explicit color/pixel formats.
- A headless wgpu renderer that composites a view of the document in linear light.
- A `slopshop` CLI (GPU info, headless render to PNG, export).
- Export to PNG (8/16-bit), TIFF (8/16-bit, 32-bit float), OpenEXR (32/16-bit float), JPEG
  and WebP (lossy or lossless) at full resolution, streamed in bands (WebP excepted: it holds
  one frame, at most 16383 px per side), with the color space always tagged (sRGB/cICP/ICC, EXR
  chromaticities), transparency flattened over a chosen background when the file has no alpha,
  and every lossy conversion reported ([ADR 0008](docs/adr/0008-export.md),
  [ADR 0010](docs/adr/0010-jpeg-webp-export.md)).
- Opening images of hundreds of megapixels in their native precision (8/16-bit, 16/32-bit
  float, HDR): PNG, JPEG, TIFF, WebP, GIF, BMP, TGA, ICO, PNM/PFM, QOI, farbfeld, EXR, HDR, DDS.
  Embedded ICC profiles (matrix/TRC) are applied; layers are composited in linear Rec.2020.
  JPEG XL, JPEG 2000, AVIF, DICOM, camera RAW, PSD and more are planned
  ([ADR 0006](docs/adr/0006-universal-import-and-licensing.md)); HEIC is not supported.
- A desktop app validating the stack: document tabs (reorder, rename, drop a tab on the canvas
  to copy its layers), smooth zoom and pan presented natively on Windows, layers panel (add
  fill, visibility, live opacity, rename, drag to reorder, delete), images dropped on the canvas
  become layers, export with progress and cancel, undo/redo, English and French interface.

No document file format (save/reopen with layers), painting, selections, filters or AI yet.

## Getting started

Prerequisites:

- [Rust](https://rustup.rs) (stable; the exact toolchain is pinned by `rust-toolchain.toml`)
- [Node.js](https://nodejs.org) 24+ with npm
- Tauri system dependencies for your OS: see the
  [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) (WebView2 on Windows,
  Xcode Command Line Tools on macOS, WebKitGTK and friends on Linux)
- A GPU with Vulkan, Metal or DirectX 12 support

Run the desktop app:

```sh
cd app
npm install
npm run tauri dev
```

Headless CLI:

```sh
cargo run -p slopshop-cli -- gpu
cargo run -p slopshop-cli -- render --size 1024x768 --out out/demo.png
cargo run -p slopshop-cli -- export photo.jpg photo.png
```

Every command and option is documented in [docs/cli.md](docs/cli.md).

Checks (also run by CI):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd app && npm run format:check && npm run check && npm run build
```

## Languages

The code and documentation are in English. The application is available in **English and
French**; adding a language means adding one translation catalog (see
[ADR 0004](docs/adr/0004-ui-internationalization.md)).

## Contributing

The project is too young for outside contributions to be easy, but issues and discussions are
welcome. Development rules (for humans and AI agents alike) are in [`CLAUDE.md`](CLAUDE.md).

## License

[MIT](LICENSE)
