# CLAUDE.md

Guidance for every development session on SlopShop. Read it fully before changing code.
SlopShop is an open-source, GPU-first, non-destructive image editor with optional local AI.
The name is a joke; the engineering is not.

## Repository map

| Path                    | Role                                                                                   |
| ----------------------- | -------------------------------------------------------------------------------------- |
| `crates/slopshop-core`  | Document model, edits + undo/redo, geometry/tiling, color & pixel formats. No GPU, no UI. |
| `crates/slopshop-render`| GPU (wgpu) rendering. Headless-capable. Depends on core only.                          |
| `crates/slopshop-cli`   | Headless binary. Proves the engine runs without the UI.                                |
| `app/src-tauri`         | Tauri shell: thin IPC layer (DTOs + commands) over core/render. No image logic.        |
| `app/src`               | Svelte 5 + TypeScript UI. Presentation and input only.                                 |
| `docs/`                 | `architecture.md`, ADRs in `docs/adr/`, research in `docs/research/`.                  |

Dependency direction is strict: `core` ← `render` ← (`cli`, `app`). Never the reverse.

## Commands

```sh
cargo fmt --all                                  # format Rust
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd app && npm run format && npm run check        # format + type-check the UI
cd app && npm run tauri dev                      # run the desktop app
cargo run -p slopshop-cli -- --help              # headless CLI
```

**Before finishing any task:** format, lint and test (all commands above except `tauri dev`) and
make sure they pass. Say so explicitly if something could not be run.

## Engineering rules

**Simplicity**
- Prefer simple, readable code. Modules have one clear responsibility.
- No premature abstraction: no trait/generic/plugin system until a second real use exists.
- Do not over-engineer, but never take a shortcut that compromises the architecture below.
  If a hack is unavoidable, isolate it, mark it `// HACK(reason):` and open a follow-up.

**Architecture**
- Image/document logic lives in Rust crates, never in the frontend and never in `app/src-tauri`.
- The engine must stay usable without the UI (tests, CLI, batch, headless render).
- Large pixel buffers never cross the IPC unless strictly needed; only viewport-sized display
  frames go to the UI, as raw binary (`tauri::ipc::Response`), never JSON/base64.
- No heavy work on the UI thread: Tauri commands touching the engine are `async` and push
  CPU/GPU work to `spawn_blocking` or worker threads. The Svelte side never blocks either.
- The user sees a layer stack; the model must remain able to evolve into a DAG. Refer to
  things by stable IDs, not indices.

**Non-destructive editing**
- Non-destructive by default: original pixels stay intact; operations are layers/nodes with
  parameters. Destructive operations must be explicit and opt-in.
- Every document mutation goes through a reversible `Edit` and is undoable/redoable. No
  direct mutation of document state from outside the edit path.
- AI operations are nodes like any other: parameters, prompt, model + version, seed, mask,
  source region, dependencies, cached result. Upstream changes *invalidate*; they do not
  silently recompute expensive work.

**Pixels, color and memory**
- Never assume sRGB 8-bit. Pixel format and color space are always explicit in types.
- No silent or lossy conversions (bit depth, color space, alpha, clipping). Conversions are
  explicit, named, and documented. Display transforms are views, not document changes.
- Never assume an image fits in RAM or VRAM. Design for tiles, regions of interest, caches,
  mip levels and partial recomputation. Work on the region actually needed.
- Avoid needless copies of pixel data: borrow, slice, reuse buffers, and justify each copy.

**Rust**
- `unsafe` is denied workspace-wide. If truly required, isolate it in a small module,
  `#[allow(unsafe_code)]` locally, and document every block with `// SAFETY:`.
- No `unwrap()`/`expect()` on fallible runtime paths (tests and proven invariants excepted,
  with a message stating the invariant). Errors are typed in libraries.
- Dependencies are limited and justified (why, and why not std/an existing dep). Mention new
  dependencies in the commit message. `slopshop-core` stays dependency-free unless an ADR says otherwise.

**Testing**
- Important logic is tested (edits and their inverses, history, geometry, color math, render
  correctness). Bug fixes come with a regression test.
- GPU tests skip when no adapter is available, except when `SLOPSHOP_REQUIRE_GPU=1` (CI).

## Skills and documentation

- Before any specialized or complex task, check whether a relevant skill exists. If it does:
  1. fetch/read it, 2. follow its instructions, 3. use its tools/references, 4. only then implement.
- For libraries and APIs whose current behavior matters (wgpu, Tauri, Svelte, Vite, …),
  consult the official documentation or the upstream examples for the pinned version instead
  of relying on memory. These APIs change frequently.

## Decisions

- Document significant architectural decisions as ADRs in `docs/adr/` (short: context,
  decision, alternatives, consequences). Update `docs/architecture.md` when structure changes.
- When an important decision is uncertain or hard to reverse, present the alternatives with
  trade-offs to the maintainer before locking it in.
- High-definition generative AI is strategic and still open: see
  `docs/research/hd-generative-ai.md`. Do not lock an approach without experiments.

## Honesty

- Never claim in docs, README or UI that a feature exists if it is not implemented.
- Commits are small and logical, with clear messages (conventional-commit style prefixes).
