# 0025 — AI selection

Status: accepted (2026-10-01, by the maintainer). Builds on the research and
measurements in [ai-selection.md](../research/ai-selection.md) and on selections
([ADR 0024](0024-selections.md)).

## Context

The maintainer wants AI selection in the first selection delivery: Photoshop's Select Subject,
Object Selection and Quick Selection, plus what Photoshop lacks as far as we know: **semantic
selection by text** ("sky", "the red car"), and **edges refined at full resolution** (hair,
fur) on any selection. Images reach hundreds of megapixels; the models work at about 1024².

Measured on the maintainer's machine (RTX 5070 Ti, Core Ultra 7 265KF), warm, through ONNX
Runtime 1.28 (`experiments/ai-bench`):

| Model | Stage | CUDA | DirectML | CPU |
|---|---|---|---|---|
| SAM 2.1 base+ fp16 | image encoder | 30 ms | 40 ms | 1.4 s |
| SAM 2.1 | per click / stroke | 4 ms | 6 ms | 20 ms |
| SAM 3 | image encoder + per text prompt | 224 + 65 ms | fails | 4.9 s + 0.7 s |
| BiRefNet lite / full fp16 | 1024² | 120 / 200 ms | 91 / 144 ms (10–11 GB VRAM) | 2.8 / 5.3 s |
| ViTMatte-S | one 1024² edge tile | 52 ms | — | — |

On a 50 MP photo, a coarse SAM mask refined on its edge tiles by ViTMatte separates hairs and
fur, whatever object is selected (BiRefNet matted the cat when the fence was selected; the
guided filter loses strands).

Maintainer's decisions (2026-10-01): inference in a separate helper process; the best models,
commercially usable ones by default and others optional with their license shown and accepted;
TensorRT-RTX on demand on NVIDIA, else DirectML; models downloaded on first use.

## Decision

1. **A separate `slopshop-ai` executable** (like `slopshop-raw`, ADR 0023) runs every model
   through **ONNX Runtime** (`ort`, loaded dynamically). Started on first use and kept while the
   app runs, it talks to the app over a pipe (binary messages: images and masks never as JSON).
   A crash or a driver error ends the helper, not the editor; quitting it frees the VRAM.
2. **Execution providers**, picked at start and reported to the UI.

   > **Amended 2026-10-02 (maintainer):** **DirectML only on Windows**, for every graphics
   > card: ONNX Runtime 1.24.4 with DirectML, from its official PyPI package (16 MB to
   > download, against about 1 GB for CUDA). Without SAM 3 (see 3), CUDA's only gain is
   > Refine Edge, about 5× faster per window (65 ms against 320–430 ms on an RTX 5070 Ti);
   > SAM 2.1's clicks (16 ms against 7 ms), its encoder (same) and BiRefNet (165 ms against
   > 217 ms) differ little. DirectML is in maintenance at Microsoft (1.24.4 is its last
   > release): its replacement is to be followed (Windows ML, the WebGPU plugin). **macOS on
   > Apple silicon**: Core ML, from ONNX Runtime's official macOS release (42 MB), the models
   > in single precision. **Linux (x64, ARM)**: the processor, from ONNX Runtime's official
   > Linux release (11 MB), the small models (SAM 2.1 tiny, BiRefNet lite); Refine Edge is off
   > by default there (about a second per window). Neither can be tested by the maintainer:
   > CI runs them end to end (macOS on Apple silicon runners). Intel Macs and Windows on ARM
   > have no runtime. The text below records the earlier plan.

   - **NVIDIA**: CUDA, with **Microsoft's official ONNX Runtime build** (ort's prebuilt one has
     no RTX 50 kernels) and NVIDIA's runtime libraries (cuDNN, cuBLAS, CUDA runtime) fetched on
     demand from NVIDIA's packages. **TensorRT-RTX** replaces it once NVIDIA's standalone plugin
     EP is distributed: ort's built-in one does not load TensorRT-RTX 1.6 (measured). *This
     changes the decision of 2026-10-01 (TensorRT-RTX first) on evidence.*
   - **Other GPUs on Windows**: DirectML (SAM 2.1, BiRefNet, ViTMatte; not SAM 3, which fails).
   - **macOS**: CoreML, untested (no Mac). **Everywhere**: the CPU as a fallback, slow but
     usable for clicks (SAM 2.1 tiny: 0.4 s per image, then 15 ms per click).
   - WebGPU stays to try (the official plugin EP), for AMD and Intel GPUs everywhere.
3. **Tools and models** (W slot, Photoshop's order, plus the Select menu):

   | Tool | Interaction | Default model | Optional (license accepted) |
   |---|---|---|---|
   | Object Selection | box, or click; later hover highlight | SAM 2.1 | — |
   | Quick Selection | brush strokes (Alt: remove), brush size | SAM 2.1 | — |
   | Select > Subject | one command | BiRefNet | — |
   | Refine Edge (option of every tool, and a command for any selection) | — | ViTMatte-S | SAM2Matting, ZIM (non-commercial) |

   SAM 2.1 size by hardware: base+ on a GPU, tiny on the CPU.

   > **Amended 2026-10-02 (maintainer):** no semantic selection (Select > Semantic… with SAM
   > 3, and the translation of its text): 3.6 GB of models, most of a 16 GB card, for little
   > use in a graphic designer's workflow. It was built and measured (branches
   > `feat/select-semantic` and `feat/translate`, kept) but not merged. SlopShop keeps
   > Photoshop's tools: Object and Quick Selection, Select > Subject, Refine Edge.
4. **Large images**: a model sees the pyramid level near 1024² (or the region in view or in
   the box). Its mask is upsampled into a coarse selection, shown at once. **Refine Edge**
   then runs the matting model on full-resolution tiles along the outline only, guided by a
   trimap from the coarse mask (sure inside, sure outside, a band to decide), and the selection
   is replaced when it is done (progressive, cancellable). The cost follows the outline, not
   the canvas, as for selections.
5. **An AI result is a selection** (ADR 0024): a mask, undoable like any selection, not a live
   node. Its provenance (model and version, prompts, region) is kept with its history entry so
   that it can be run again on demand; nothing is recomputed silently.
6. **Models are downloaded on first use**, never shipped: a manifest pins each file
   (repository, revision, SHA-256, license); the app shows the size and asks first; files go to
   a per-user cache. A model whose license is not permissive (SAM License, non-commercial)
   shows its license and needs an explicit acceptance; non-commercial ones carry a mark in the
   UI wherever they can be chosen.
7. **Weights policy**: by default only weights usable commercially; others are optional, from
   their official source, never redistributed by SlopShop. Known risk, recorded: most matting
   weights are trained on research-only datasets (Composition-1k, Distinctions-646); their own
   license (Apache-2.0 for ViTMatte) is the one applied, as by the rest of the ecosystem.

## Alternatives

- **In-process inference**: simpler, no IPC, but NVIDIA's libraries and model crashes inside
  the editor, and VRAM held for the session.
- **One model for everything** (SAM 3 alone): the best text selection, but 13 GB of VRAM growth,
  no DirectML, 5 s per image on the CPU.
- **BiRefNet for every refinement**: sharp on salient subjects, wrong when the selection is not
  the salient object.
- **Classical refinement only** (guided filter): no model, but it loses hair strands.
- **A Python sidecar**: the richest ecosystem, but a Python install to ship and maintain.

## Consequences

- New crate `crates/slopshop-ai` (the helper) and its protocol; `slopshop-io`-like model
  manifest and downloader in the app; a model manager in the UI (sizes, licenses, cache).
- Still to measure before or during implementation: the trimap band width (it decides
  ViTMatte's result), seams between overlapping tiles, the cost on 100–300 MP, BiRefNet's VRAM
  (a native-DeformConv export), the ONNX Runtime arena on CUDA.
- First implementation step: the helper with SAM 2.1 and the **Quick Selection** tool, then
  Object Selection, Refine Edge, Select Subject and semantic selection.
