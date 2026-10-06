# Contributing to SlopShop

Thanks for your interest! SlopShop is a young image editor that can already do real work:
layers and groups, selections (AI-assisted included), painting and retouching tools,
adjustment layers, filters, layer styles, transforms in perspective, and dozens of file
formats including layered PSD. Much is still missing (text, shapes and paths, generative AI,
a plugin API), and contributions are welcome.

![SlopShop with a layered document](docs/images/screenshot-main.jpg)

This page explains how to help without losing your time.

## Ways to help

- **Try it and report what breaks.** Download a build from the
  [releases](https://github.com/laBoiteBleue/slopshop/releases) and
  [open an issue](https://github.com/laBoiteBleue/slopshop/issues/new/choose).
- **macOS and Linux.** The maintainer tests on Windows only. Nobody has used the app on macOS
  or Linux yet, although continuous integration builds and tests it there: running it,
  reporting what breaks, and fixing it is the most valuable help right now. The native
  viewport on macOS (a Metal view presenting the engine's frames, as Windows does) needs
  someone with a Mac.
- **Issues labeled [`good first issue`](https://github.com/laBoiteBleue/slopshop/labels/good%20first%20issue)
  or [`help wanted`](https://github.com/laBoiteBleue/slopshop/labels/help%20wanted).**
- **Features.** The [feature map](docs/feature-map.md) lists the features of professional
  editors (taking Photoshop's menus and tools as the reference, since many users know them)
  and says, for each one, whether SlopShop has it, plans it, leaves it open to contributions
  or leaves it out, and why. The [roadmap](docs/roadmap.md) has sections open to
  contributors, such as the advanced transforms (Warp, Puppet Warp, Perspective Warp).
- **Image formats.** [docs/formats.md](docs/formats.md) lists every format, what SlopShop
  supports, the priorities and a step-by-step guide. Krita, GIMP and OpenRaster files, and
  the older formats, are small and self-contained projects.
- **A new interface language.** One catalog file in `app/src/lib/i18n/` and one entry in the
  `locales` registry ([ADR 0004](docs/adr/0004-ui-internationalization.md)).
- **Guardrails.** A fuzz target for a decoder, a test that pins a documented behavior, a CI
  check that catches a class of mistakes.

## Before you start

- **Small fixes** (a typo, a clear bug with a regression test, a missing translation): open a
  pull request directly.
- **Anything larger**: open an issue first, or a
  [discussion](https://github.com/laBoiteBleue/slopshop/discussions), and describe what you
  want to do and how. SlopShop has strict architectural rules (below), and some features wait
  for an engine decision; it is better to agree on the approach before you write the code.
- **Decisions that are hard to reverse** (the `.slop` document format beyond a compatible
  addition, the architecture, significant dependencies, `unsafe` code, product and ergonomics
  choices) are made by the maintainer, usually as an [ADR](docs/adr/). Proposals are welcome.

## Setting up

Follow [Building from source](README.md#building-from-source) in the README: Rust (the
toolchain is pinned), Node.js 24+, Tauri's system dependencies, and on Windows Visual Studio's
C++ ATL component. Then:

```sh
cd app
npm install
npm run tauri dev
```

Development builds open `$SLOPSHOP_OPEN`, else `out/default.slop`, else `out/default.jpg` at
startup, which is handy for testing. The engine also runs without the app:
`cargo run -p slopshop-cli -- --help` ([docs/cli.md](docs/cli.md)).

## How the code is organized

| Path                     | Role                                                                                  |
| ------------------------ | ------------------------------------------------------------------------------------- |
| `crates/slopshop-core`   | Document model, edits and undo, selections, painting, geometry, color. No GPU, no UI. |
| `crates/slopshop-render` | GPU rendering (wgpu), headless-capable, checked against a CPU reference.              |
| `crates/slopshop-io`     | File formats: decoding and encoding images and `.slop` documents.                     |
| `crates/slopshop-cli`    | The headless `slopshop` command.                                                      |
| `crates/slopshop-raw`    | Camera RAW, a separate executable (rawler is LGPL; ADR 0023).                         |
| `crates/slopshop-ai`     | The local AI helper: ONNX Runtime loaded at run time (ADR 0025).                      |
| `app/src-tauri`          | The Tauri shell: a thin IPC layer over the engine. No image logic.                    |
| `app/src`                | The Svelte 5 + TypeScript interface: presentation and input only.                     |

Dependencies go one way: `core` ← (`render`, `io`) ← (`cli`, `app`). The
[architecture](docs/architecture.md) and the [decision records](docs/adr/) explain why things
are the way they are; read the ADR of the area you touch.

## The rules, in short

The full rules are in [CLAUDE.md](CLAUDE.md) (written for AI agents, but they apply to
everyone). The ones that matter most:

- **Image and document logic lives in the Rust crates**, never in the Svelte UI or the Tauri
  shell. The engine works without the UI.
- **Non-destructive by default.** Every document change is a reversible `Edit`, undoable.
  Original pixels stay intact; destructive operations are explicit and opt-in.
- **Explicit pixels.** Never assume sRGB 8-bit or that an image fits in memory; no silent or
  lossy conversion.
- **Performance matters.** When two approaches differ in speed, the faster one wins unless it
  costs correctness or the architecture. GPU code is checked against a CPU reference.
- **Every user-visible string goes through i18n** (`t("key")`), in every catalog. If you do
  not speak one of the languages, add your best translation and say so in the pull request.
- **No `unsafe`, no `unwrap()` on fallible paths**, typed errors in libraries.
- **Dependencies are justified**, and their licenses must pass `cargo deny check`: permissive
  licenses only, LGPL only in a separate process, never GPL or AGPL
  ([ADR 0006](docs/adr/0006-universal-import-and-licensing.md)).
- **Tests** for important logic, UI tests for UI changes, a regression test for every fix.
- **Honesty**: docs, README and UI never claim a feature that is not implemented.

## Before opening a pull request

Run the checks (CI runs them too, on Linux, macOS and Windows):

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
cd app && npm run format && npm run check && npm test
```

Then:

- **One topic per pull request**, rebased on an up-to-date `main`. Pull requests are merged
  with "Rebase and merge", so keep the commits small and meaningful, with
  [conventional](https://www.conventionalcommits.org) prefixes (`feat(io): …`, `fix(app): …`,
  `docs: …`).
- **Sign off every commit** (below).
- **Update the docs** your change touches: the README feature list (in both languages),
  `docs/formats.md`, `docs/cli.md` (a test checks it against the CLI),
  `docs/feature-map.csv`, the roadmap.
- **Say how you tested it**, and on which platform.

## Sign-off: the Developer Certificate of Origin

SlopShop is licensed under the [GNU GPL version 3 only](LICENSE), and so are contributions:
there is no contributor license agreement to sign. Instead, each commit carries a sign-off
certifying the [Developer Certificate of Origin](DCO): that you wrote the change, or have the
right to submit it under the project's license.

```sh
git commit -s -m "fix(io): …"      # adds "Signed-off-by: Your Name <you@example.com>"
git rebase --signoff origin/main   # signs off the commits already on your branch
```

The name and email must match the commit's author. A check on each pull request verifies it.

## AI-assisted contributions

Encouraged. SlopShop itself is developed with AI agents, and they are very productive when
they work inside guardrails. The guardrails are what make the difference between a useful
contribution and slop:

- **Tests are the proof.** Unit tests for the logic (an edit and its inverse, color math,
  geometry), integration tests through the public API or the CLI, round trips for formats
  (write, read back, compare), damaged and truncated inputs for decoders, GPU results compared
  with the CPU reference, component tests for UI controls, and a regression test for every
  bug fix. A change without tests is not done, whoever wrote it.
- **The checks pass locally** before the pull request: formatting, Clippy with warnings as
  errors, the tests, `cargo deny`, and the UI type-check (which also catches missing
  translations).
- **Point your agent to the rules.** Agents find them in [AGENTS.md](AGENTS.md) and
  [CLAUDE.md](CLAUDE.md); have it read them before it writes code, and keep the change to one
  topic.
- **You stay the author.** Read the diff, understand it, be able to answer review questions.
  Your sign-off certifies the change, whoever typed it. Say in the pull request that an AI tool
  helped and how the change was tested.

Pull requests without tests, or clearly not reviewed by their author, will be closed.

## Reporting bugs

Open an issue with the bug report template: what you did, what happened, your OS and GPU, and
a file that reproduces the problem when you can share one. Security problems (for example a
file that crashes a decoder in a way that might be exploitable) go through
[SECURITY.md](SECURITY.md), not public issues.

## Code of conduct

Everyone taking part follows the [code of conduct](CODE_OF_CONDUCT.md).
