# 0001 — Rust engine, Tauri shell, Svelte UI

Status: accepted

## Context

SlopShop needs a fast, GPU-first engine that handles very large images, runs headless (tests,
CLI, batch), and a modern cross-platform desktop UI.

## Decision

- **Engine in Rust**, split into crates with a strict dependency direction:
  `slopshop-core` (model, no deps) ← `slopshop-render` (wgpu) ← `slopshop-cli`, `app`.
- **GPU through wgpu** (WebGPU API over Vulkan/Metal/DX12), usable headless.
- **Desktop shell: Tauri 2**; the Rust side of the app is a thin IPC layer.
- **UI: Svelte 5 + TypeScript with Vite**, as a plain SPA (no SvelteKit: an editor has no
  routing or SSR needs, and fewer dependencies is better).
- IPC DTOs live in the app crate, not in core, so core stays serde-free and the internal model
  can change without breaking the UI contract.
- npm as the package manager: always available with Node, no extra tooling.

## Alternatives considered

- **Electron**: heavier runtime, and the engine would still be native.
- **Fully native UI (egui, iced, Slint)**: one language and direct GPU access, but a
  familiar pro-editor UI is much slower to build; may be revisited for the viewport only
  (see ADR 0002).
- **SvelteKit** (Tauri's default template): unnecessary routing/SSR machinery.

## Consequences

- Two languages (Rust, TypeScript) and an IPC boundary to keep narrow.
- The engine is testable and scriptable without the UI from day one (`slopshop-cli`).
- The viewport must get pixels from the engine to the webview efficiently (ADR 0002).
