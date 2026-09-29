# AGENTS.md

Instructions for AI coding agents working on SlopShop.

**The canonical rules live in [`CLAUDE.md`](CLAUDE.md). Read and follow it entirely**, whatever
agent you are. This file only exists so that tools looking for `AGENTS.md` find the same rules;
do not duplicate or fork guidance here — update `CLAUDE.md` instead.

Quick reminders:

- Engine logic in Rust crates (`crates/`), never in the UI or the Tauri shell.
- Non-destructive by default; every document change is an undoable `Edit`.
- Never assume sRGB 8-bit, never assume an image fits in RAM/VRAM, no silent conversions.
- Before finishing: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`, and `npm run format && npm run check` in `app/`.
- Significant decisions go in `docs/adr/`.
