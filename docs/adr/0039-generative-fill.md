# 0039 — Remove and generative fill: generation entries, local editing models, native definition

Status: **proposed** (2026-10-05), for the maintainer's review. Points 1–3 follow the
maintainer's instructions of 2026-10-05. Two revisions were made the same day:

- Remove is an entry of the layer's stack;
- the models are used as the editing models they are.

Points 4 (runtime) and 7 (format) are the choices left to make. Point 5 sets the experiments to
run before any large-region generation ships. FLUX.2 [klein] 4B comes first.

Amends [ADR 0034](0034-editable-operations.md) point 6 for Remove.

## Context

The maintainer's order (2026-10-05) for the AI project:

- **Local models**: FLUX.2 [klein] 4B and Qwen-Image 2.1.
- **Remove first**, with a choice of what replaces what is removed: **transparency**, the
  **background color**, or **generated** content.
- **Then outpainting**, when the image is cropped larger than the canvas.
- **The work is native definition**. The hard part is not generating but generating at the
  document's resolution ([research](../research/hd-generative-ai.md)).

The research document sets the success criteria:

- generated content is native, never a generic upscale;
- outside the mask, pixels are identical to the bit;
- the result is coherent, without seams or repetition;
- detail and grain match the surroundings;
- a result can be reproduced from the model, its version, the seed and the parameters;
- VRAM is bounded whatever the document's size;
- a fast preview comes before the final render.

Facts checked on 2026-10-05 (model cards, licenses, runtime repositories):

| | FLUX.2 [klein] 4B | Qwen-Image 2.1 |
|---|---|---|
| License (weights) | Apache-2.0 | Qwen Research License: non-commercial ("research or evaluation"), under Chinese jurisdiction |
| Outputs | commercial use allowed | the license says nothing about outputs, so commercial use is not granted |
| Transformer | 4B: 7.8 GB in bf16, 4.1 GB in fp8 | 7B: 14.2 GB in bf16; about 4 GB in GGUF Q4 |
| Text encoder | Qwen3, 8 GB in bf16 | Qwen3-VL 8B, 17.5 GB |
| VAE | 0.2 GB | RGBA, 1.35 GB |
| VRAM at 1024² | about 13 GB in bf16 (BFL's figure), less in fp8 or GGUF | 34 GB peak in bf16; 12–16 GB in GGUF Q4, more with offload |
| Steps | 4 (distilled) | 40 (no official distilled model) |
| Native resolution | up to 4 MP (2 MP recommended), sides multiples of 16 | 2048² (2K), sides multiples of 32 |
| Editing | from reference images plus an instruction | from up to 10 references plus an instruction; guided by marks drawn on the reference ("circles, painted annotations") |
| Masked fill | no trained fill mode; latent blending in a community diffusers pipeline | no mask input: a mask is just another reference ("the edit may spill outside the area") |
| ONNX | two community exports (int8, int4), two weeks old, not validated on DirectML | none |
| stable-diffusion.cpp (MIT; CUDA, Vulkan, Metal) | supported since 2026-01 | supported from release day (2026-09-20) |

**Neither model is an inpainter.** Both are editing models: given reference images and an
instruction, they return the edited image.

- Qwen-Image 2.1 documents edits guided by marks drawn on the reference ("remove what is
  circled").
- FLUX.2 [klein] documents reference editing with an instruction. Guidance by drawn marks is
  not documented for it and has to be measured.
- Masked latent blending is the classic alternative: the known region is noised again and
  replaced at every step.

The current AI helper ([ADR 0025](0025-ai-selection.md)) runs ONNX Runtime with DirectML, and
DirectML is in maintenance at Microsoft.

## Decision

### 1. Remove: three ways to fill, all non-destructive

Edit > Remove, and the Remove tool (J, sharing the Healing Brush's slot as the toolbar audit
decided), work on the selection or on a brushed area. They ask what goes in its place:

- **Transparent**: the active layer's mask hides the area. Without a mask, one is made from the
  selection, as Layer > Layer Mask > Hide Selection does. Nothing else changes.
- **Background color**: a fill layer of the background color, masked by the area, above the
  active layer.
- **Generated**: a **generation entry** on top of the active layer's stack (point 2), holding
  what the model gave within the area.

All three stay editable and removable.

Unlike ADR 0034 point 6, Remove is not baked into the layer's paint as anonymous pixels. The
entry keeps what it generated and how it did. It can be shown, hidden, generated again or
removed like the stack's other entries, and it follows the layer when the layer moves or is
transformed.

**How the model is asked** (an editing model, not an inpainter):

1. **The crop.** The region and its context margin are cropped at scale 1:1 from what the
   entry reads. By default that is the layer, as the entries below the new one make it. With
   Sample All Layers, it is every visible layer.
2. **The request.** The crop is the reference image. The area is marked on it when the model
   is guided by marks. The instruction is the task's ("remove what is marked") or the user's
   prompt.
3. **The answer.** The model returns the whole crop, edited.
4. **What is kept.** Only the area:
   - dilated a little, because the shadows and reflections of what is removed lie just
     outside a tight selection;
   - feathered;
   - blended across its edge (Laplacian or gradient-domain blending, the research's approach
     F), so that the slight color shift an editing model makes elsewhere does not show.

   Outside the area, the layer's pixels are untouched.

The step-0 spike (point 5) measures three ways to ask FLUX.2 [klein]:

- the instruction with a drawn mark;
- the instruction alone, the area cropped tight;
- masked latent blending.

The best one becomes the default.

### 2. The generation: an AI node with its result cached

Remove's result is a new kind of **stack entry**, as Liquify's is ([ADR 0037](0037-liquify.md)).

Crop's outpainting (point 6) makes a **generative layer** instead, since no layer holds the new
area. A generative layer is a pixel layer whose stack starts with such an entry.

Either one holds an AI operation, as CLAUDE.md requires. It stores:

- the task (fill, expand);
- the model's id and version, and its runtime;
- the prompt (empty for Remove);
- the seed and the parameters;
- the **source**, what it read: the layer as the entries below make it (by default), or all
  visible layers (Sample All Layers);
- the **region** and its context margin;
- the **area**, a coverage in the layer's pixels (the dilated, feathered selection);
- the **result**: pixels at the layer's resolution over the region. They are saved in the
  file, since they are expensive to compute again.

It is evaluated like a paint entry: its result is laid over what is below it, within the area,
on the CPU and the GPU alike.

**Stale, never computed again silently.** The entry becomes stale when what it read changes
within its region and margin: an entry below it, or, with Sample All Layers, another layer
(tracked by its revision over the region). Then:

- a badge on the entry, and on the layer, shows it;
- the entry's properties offer Generate Again.

Its old result stays shown until then.

**Variations.** Generate Again with another seed gives a variation. The last few variations
are kept, and the entry's properties switch between them, as in Photoshop's Generative Fill.

Rasterize bakes it into plain pixels, as for any layer.

### 3. Models: the best non-commercial one, and a commercial alternative

The model-choice rule applies: at most two choices, the best one first. Since the best one is
non-commercial, a commercially usable alternative is offered too.

- **FLUX.2 [klein] 4B**, the default and the first one built:
  - Apache-2.0: its weights and its outputs can be used commercially;
  - 4 steps, so it is fast.
- **Qwen-Image 2.1**, optional:
  - It is never shipped. It is downloaded from its official repository on first use, and its
    license is shown and must be accepted (the generative model policy of 2026-10-01).
  - Its license grants nothing for outputs. A "non-commercial: research and evaluation" mark
    therefore stays visible on every entry it made.

The pipeline gets a `Model` trait in Rust from the start. Two real uses exist, as the simplicity
rule asks: two models with different resolutions, steps and ways of being asked.

A ComfyUI connector remains a later option, for users who want other workflows. It works over a
network API, so it does not link SlopShop to ComfyUI and the GPL is not involved.

### 4. Runtime (to choose)

Recommended: **a separate `slopshop-gen` executable built on stable-diffusion.cpp**.

- stable-diffusion.cpp is MIT-licensed and built on ggml.
- Backends:
  - Vulkan on Windows and Linux, for NVIDIA, AMD and Intel cards;
  - Metal on macOS;
  - CUDA where present, faster on NVIDIA.
- It loads GGUF weights: fp8 or Q8 for the transformer, Q8 or Q4 for the text encoder.

It is a helper process, like `slopshop-ai` and `slopshop-raw`:

- it starts on first use and talks to the app over a pipe, in binary;
- quitting it frees the VRAM;
- a crash ends the helper, not the editor.

The ways of asking the model (point 1) live there. They are written in C++ around sd.cpp's
API, or in Rust over its C API.

The selection models stay on ONNX Runtime with DirectML, as they are.

**Gated by a measured spike** (point 5, step 0): FLUX.2 [klein] 4B must take a few seconds per
1 MP edit on the maintainer's RTX 5070 Ti (16 GB) under Vulkan. Otherwise the alternatives are
opened again.

### 5. Native definition: an experiment bench before large regions

**Fixed now**, from the research document:

- **Tiles and pyramid levels are first-class.** The working resolution is a property of the
  model, never a constant.
- **Seeds are derived per tile**, from the entry's seed and the tile's coordinates.
- **Compositing is strict**, in document space and in linear light. Outside the feathered
  area, the pixels below stay identical to the bit.
- **Preview and final render are separate.** The preview is the coarse level, shown first. The
  final render adds the native levels.
- **Budgets.** The scheduler receives a VRAM and RAM budget and chooses tile sizes from it.

**Step 0, the runtime spike.** It runs FLUX.2 [klein] 4B through stable-diffusion.cpp on
Vulkan, at 1 MP and 2 MP, in fp8 and Q8. It measures:

- time, VRAM, and fidelity against the PyTorch diffusers reference;
- the three ways of asking the model (point 1).

**Step 1, Remove on small regions, native by construction.** The region and its margin are
cropped at scale 1:1 (the research's approach A). When they fit the model's working resolution
at 1:1 (about 2 MP for FLUX.2 [klein]), they are generated directly, with no upscale. Most
removals are this case: blemishes, objects, passers-by.

**Larger regions, meanwhile.** Until the bench chooses a pipeline, a larger region is generated
at the coarse level only and marked "preview". It is never presented as native.

**Step 2, the bench.** It runs on the maintainer's images: 24 to 100 MP, a portrait, foliage,
architecture, a product or text, and heavy grain. It covers the research's four tasks:

- small removal;
- large replacement;
- border outpainting;
- an edit that keeps the structure.

It compares these pipelines, each with and without F (matching grain and high frequencies):

- B, the baseline;
- A;
- C, coarse to fine, with partial re-noising;
- C+D, with tiles fused at every step;
- C+E.

It measures:

- seams;
- spectrum and grain inside the area against around it;
- exactness outside the mask;
- time and peak VRAM;
- a blind A/B comparison at 100 %.

**The decision gate** is an amendment to this ADR that records the chosen pipeline and its
evidence. Large regions and outpainting ship only after it.

### 6. Outpainting: Crop larger

When the crop box goes beyond the canvas, Crop's options offer how to fill the new area, as
Remove does:

- **Transparent**: today's behavior.
- **Background color**.
- **Generated**: a generative layer (point 2), with the task "expand" and the new area as its
  area.

The image is never resampled: only the canvas grows (ADR 0017). Outpainting is a large-region
task, so it comes after step 2.

### 7. File format (to confirm)

The generation becomes a new kind of stack entry in `.slop`, `{"generation": {…}}`. It is a
compatible addition, at a new node version, as Liquify's entry was. It holds:

- its parameters, as JSON;
- its area, its result and its kept variations, as images;
- a stale flag.

A generative layer (outpainting) is a raster node whose stack starts with this entry.

Older readers refuse the file rather than misread it. The model's weights are never in the
file. Opened without the model, the file shows the saved result, and Generate Again asks for
the model.

## Alternatives

- **ONNX Runtime with DirectML** for the generative models.
  - It would keep a single runtime.
  - But FLUX.2 [klein] has only fresh community exports (int8 and int4), not validated on
    DirectML; the int4 one is visibly less faithful (cosine 0.95).
  - Qwen-Image 2.1 has no export, and its block-causal attention makes one hard.
  - DirectML itself is frozen.
  - Kept as a fallback if the spike fails.
- **A Python sidecar** (PyTorch and diffusers).
  - It gives the reference fidelity and supports every model from release day.
  - But it means several gigabytes of runtime to install, mostly CUDA, and a packaging burden
    on three platforms.
- **ComfyUI only.**
  - Nothing to package.
  - But the user must install and run another program. It stays an option (point 3), not the
    default.
- **Qwen-Image-Edit 2509 or 2511** (Apache-2.0) as the commercial alternative, instead of
  FLUX.2 [klein].
  - It would be commercially clean.
  - But it weighs about 58 GB (a 20B transformer), beyond a 16 GB card without heavy offload.
- **A non-generative remover** (MI-GAN: MIT, a 28 MB ONNX; LaMa: Apache-2.0, 208 MB).
  - It would be instant and light.
  - But it would be a third model for one function, which the two-choice rule forbids unless
    it replaces one.
  - To reconsider if the spike shows generation too slow on modest cards.
- **Remove baked into the layer's paint** (ADR 0034 point 6, as written).
  - Simpler: the result is a paint entry's pixels.
  - But it keeps no model, seed, prompt or region. It cannot be generated again, give
    variations, become stale, or be redone at native definition once the bench decides. That
    goes against CLAUDE.md's rule for AI operations.
- **Remove as a separate generative layer**, as Photoshop's Generative Fill does.
  - It shows clearly in the Layers panel.
  - But a removal is an edit of the photo it belongs to. An entry follows that layer when it
    moves or is transformed, and does not clutter the panel.
  - Kept for outpainting, where no layer holds the area.

## Consequences

- **New pieces**:
  - the `slopshop-gen` helper, a native dependency stated in its PR (stable-diffusion.cpp,
    ggml);
  - its model download, with consent, as the selection models have;
  - a stack entry and its `.slop` encoding;
  - the Remove tool and command;
  - Crop's fill option.
- **License checks grow.** `cargo deny` covers the Rust crates. stable-diffusion.cpp and ggml
  (MIT, C++) are vendored in the helper and listed in its notices.
- **Disk and VRAM**:
  - FLUX.2 [klein] 4B, in fp8 or Q8, is about 12 GB to download;
  - Qwen-Image 2.1, in Q4 or Q8, is 10 to 17 GB;
  - neither is downloaded before a generative feature is first used.
- **The research document gets its results**, and this ADR its amendment, once the bench
  decides.
- **Untested platforms**: CI runs macOS (Metal) and Linux (Vulkan) where it can. The maintainer
  tests Windows only.
