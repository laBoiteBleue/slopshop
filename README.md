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

Details: [`docs/architecture.md`](docs/architecture.md) and the decision records in
[`docs/adr/`](docs/adr/).

## Contributing

The project is too young for outside contributions to be easy, but issues and discussions are
welcome. Development rules (for humans and AI agents alike) are in [`CLAUDE.md`](CLAUDE.md).

## License

[MIT](LICENSE)
