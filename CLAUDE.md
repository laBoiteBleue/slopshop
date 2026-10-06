# CLAUDE.md

Guidance for every development session on SlopShop. Read it fully before changing code.
SlopShop is an open-source, GPU-first, non-destructive image editor with optional local AI.
The name is a joke; the engineering is not.

## Repository map

| Path                    | Role                                                                                   |
| ----------------------- | -------------------------------------------------------------------------------------- |
| `crates/slopshop-core`  | Document model, edits + undo/redo, geometry/tiling, color & pixel formats. No GPU, no UI. |
| `crates/slopshop-render`| GPU (wgpu) rendering. Headless-capable. Depends on core only.                          |
| `crates/slopshop-io`    | File formats: decoding images into core rasters. Depends on core only.                 |
| `crates/slopshop-cli`   | Headless binary. Proves the engine runs without the UI.                                |
| `crates/slopshop-raw`   | Camera RAW helper: separate executable (rawler, LGPL-2.1, only here; ADR 0023).          |
| `crates/slopshop-ai`    | AI helper: protocol + client (library), and the executable running the models through ONNX Runtime loaded at run time (ADR 0025). |
| `app/src-tauri`         | Tauri shell: thin IPC layer (DTOs + commands) over core/render. No image logic.        |
| `app/src`               | Svelte 5 + TypeScript UI. Presentation and input only.                                 |
| `docs/`                 | `architecture.md`, `roadmap.md`, `cli.md` (CLI reference, kept in sync by a test), `formats.md` (format support and plan), `ergonomics.md` (UX principles and ideas to validate), `file-format.md` (`.slop` spec), `releasing.md` (publishing a version; the version is written once, in the root `Cargo.toml`), `feature-map.md` + `.csv` (what SlopShop does with each feature of professional editors), ADRs in `docs/adr/`, research in `docs/research/`. |

Dependency direction is strict: `core` ← (`render`, `io`) ← (`cli`, `app`). Never the reverse.

## Commands

```sh
cargo fmt --all                                  # format Rust
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check                                 # licenses (no GPL/AGPL), advisories, sources
cd app && npm install                             # once
cd app && npm run format && npm run check        # format + type-check the UI (incl. i18n catalogs)
cd app && npm test                               # UI tests (Vitest: modules, components in jsdom)
cd app && npm run tauri dev                      # run the desktop app
cargo run -p slopshop-cli -- --help              # headless CLI
# Dev builds of the app open $SLOPSHOP_OPEN, else out/default.slop, else out/default.jpg at startup.
```

**Before finishing any task:** format, lint and test (all commands above except `tauri dev`) and
make sure they pass. Say so explicitly if something could not be run. Then follow the Workflow
below, which depends on who drives the session.

## Workflow

For everyone:

- **One branch and one pull request per feature or fix**, one at a time: branch from an
  up-to-date `main` (`feat/…`, `fix/…`) before the first change. CI runs on every pull
  request (Linux, macOS, Windows).
- Each pull request description says which tests cover the change (Rust and UI), or why a part
  has none.
- **Stop and ask before** decisions that are hard to reverse (file format changes beyond a
  compatible addition, architecture, structuring ADRs, significant or native dependencies),
  product and ergonomics choices, `unsafe` code, and anything risky for the user's data or the
  repository.
- Pull requests are merged with "Rebase and merge", so `main` stays linear and keeps the
  individual commits.
- Commits are small and logical, with clear messages (conventional-commit style prefixes).
- To check the running app visually, capture only its window (e.g. Win32 `PrintWindow`), never
  a region of the screen: other applications with private data may be in front.

**Maintainer sessions** (the git and GitHub user is `laBoiteBleue`):

- **Autonomy (maintainer decision, 2026-09-30).** For work covered by automated tests and
  following established conventions (engine, formats, CLI, GPU rendering checked against the
  CPU, UI that follows existing patterns, bug fixes with a regression test, docs): implement,
  run the checks below, commit, push, open the pull request (`gh pr create`) and merge it
  once its CI is green, then move on to the next roadmap item. Keep a cumulative "to test"
  list in the pull requests and replies, which the maintainer goes through when they choose;
  problems they find are fixed first. "Stop and ask" above means asking the maintainer.
- Push nothing else without being asked.
- The maintainer tests on Windows only: macOS/Linux-specific work cannot be validated by them;
  keep it deferred or rely on CI, and say explicitly what is untested on those platforms.
- Outside pull requests are reviewed like any change: the rules below apply to them, and a
  contribution without tests is not merged.

**Contributor sessions** (anyone else). AI-assisted contributions are encouraged, provided
they come with their guardrails; follow [CONTRIBUTING.md](CONTRIBUTING.md):

- Work on a branch of your fork and open a pull request; never merge it, the maintainer
  reviews and merges. For anything beyond a small fix, check that an issue agreed on the
  approach first.
- **Sign off every commit** (`git commit -s`): the sign-off certifies the Developer
  Certificate of Origin ([DCO](DCO)) for the person driving the session, who must have read
  the change. A check on each pull request verifies it.
- The tests are the proof: unit tests for the logic, integration tests through the public API
  or the CLI, round trips for formats, damaged inputs for decoders, CPU references for GPU
  code, component tests for UI changes, a regression test for every fix. A change without
  tests is not done.
- "Stop and ask" above means asking the person driving the session, and raising the question
  in the issue or pull request. Say in the pull request that an AI agent helped, and how the
  change was tested.

## Languages

- **Repository and code are in English**: identifiers, comments, docs, commit messages.
- **The application is multilingual**: English and French from day one, and adding a language
  must stay trivial.
  - Every user-visible string goes through i18n (`t("key")` from `app/src/lib/i18n`). Never
    hard-code UI text.
  - `en.ts` is the reference catalog; other catalogs are typed against it, so a missing key
    fails `npm run check`. When adding or changing a string, update **all** catalogs.
  - Adding a language = one catalog file + one entry in the `locales` registry.
  - The engine and IPC send identifiers and structured data (ids, codes, numbers), not display
    text; the UI translates. Engine error messages are not localized yet: user-facing errors
    will need error codes rather than strings.

## Engineering rules

**Priorities**
- Performance is a primary requirement: when options differ in speed, prefer the most
  performant one unless it compromises correctness or the architecture (maintainer decision).
- Import must eventually be universal (every image format, incl. JPEG 2000, WebP, DICOM, camera
  RAW), and export too. Never design an import path that assumes 8-bit RGB.

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
- The UI must feel like a native desktop app, not a web page: no native webview context menu,
  text selection, browser shortcuts or zoom in production builds. Dev builds may keep them for
  debugging (`import.meta.env.DEV`).
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
- UI logic (geometry, key handling, state rules) lives in plain `.ts` modules under
  `app/src/lib`, tested in `app/tests/*.test.ts`; components keep presentation and wiring, and
  their behavior (what a click or a key sends, what is shown) is tested in
  `app/tests/components/` with Testing Library, the engine's IPC mocked (`mockIPC`). Tests of
  the whole app go in `app/tests/components/app/`, one file per area (menus, panels…), sharing
  `harness.ts`; a file adds its own IPC answers with `respond`, rather than growing one shared
  file that every branch appends to.
- **UI changes come with their tests, as engine changes do (maintainer decision, 2026-10-03).**
  Write them while developing, in the same commits as the code: new or changed logic in
  `app/src/lib` gets module tests; a new or changed control, dialog, panel, shortcut or gesture
  gets component tests of what the user does and what is sent or shown; a UI bug fix gets a
  regression test that fails without the fix. What jsdom cannot run (GPU frames, canvas
  drawing) has its logic moved into a module and tested there. Only pure presentation (styles,
  layout, icons, wording) goes without tests, and the pull request says so.
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
