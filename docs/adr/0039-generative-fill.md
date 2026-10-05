# 0039 — Remove and generative fill: generative layers, local models, native definition

Status: **proposed** (2026-10-05), for the maintainer's review. Points 1–3 follow their
instructions of 2026-10-05; points 4 (runtime) and 7 (format) are the choices to make; point 5
sets the experiments before any large-region generation ships. Amends
[ADR 0034](0034-editable-operations.md) point 6 for Remove.

## Context

The maintainer's order (2026-10-05) for the AI project:

- **Local models**: FLUX.2 [klein] 4B and Qwen-Image 2.1.
- **Remove first**, with a choice of what replaces what is removed: **transparency**, the
  **background color**, or **generated** content.
- **Then outpainting** when the image is cropped larger than the canvas.
- **The work is native definition**: the hard part is not generating, but generating at the
  document's resolution ([research](../research/hd-generative-ai.md)).

The research document sets the success criteria:

- generated content is native, never a generic upscale;
- outside the mask, pixels are identical to the bit;
- the result is coherent, without seams or repetition;
- detail and grain match the surroundings;
- the result is reproducible from model, version, seed and parameters;
- VRAM is bounded whatever the document's size;
- a fast preview comes before the final render.

Facts checked on 2026-10-05 (model cards, licenses, runtime repositories):

| | FLUX.2 [klein] 4B | Qwen-Image 2.1 |
|---|---|---|
| License (weights) | Apache-2.0 | Qwen Research License: non-commercial, "research or evaluation"; Chinese jurisdiction |
| Outputs | commercial use allowed | no clause on outputs: commercial use not granted |
| Size | 4B transformer (7.8 GB bf16, 4.1 GB fp8) + Qwen3 text encoder (8 GB bf16) + VAE 0.2 GB | 7B transformer (14.2 GB bf16) + Qwen3-VL 8B text encoder (17.5 GB) + RGBA VAE 1.35 GB; GGUF Q4 about 4 GB |
| VRAM at 1024² | about 13 GB bf16 (BFL), less in fp8/GGUF | 34 GB peak bf16; 12–16 GB with GGUF Q4 (more with offload) |
| Steps | 4 (distilled) | 40 (no official distilled model) |
| Native resolution | up to 4 MP (2 MP recommended), sides multiple of 16 | 2048² (2K), sides multiple of 32 |
| Masked fill | no trained fill mode; inpainting by latent blending (a community diffusers pipeline) | no mask input: a mask is a reference image, "the edit may spill outside the area" |
| ONNX | two 2-week-old community exports (int8/int4), unvalidated on DirectML | none |
| stable-diffusion.cpp (MIT; CUDA, Vulkan, Metal) | supported since 2026-01 | day-0 support (2026-09-20) |

Neither model has a trained inpainting mode. The fill therefore has two parts:

- the sampler conditions on the context: masked latent blending, the known region re-noised
  and replaced at every step;
- strict compositing writes back only the masked pixels.

The current AI helper ([ADR 0025](0025-ai-selection.md)) runs ONNX Runtime with DirectML.
DirectML is in maintenance at Microsoft.

## Decision

### 1. Remove: three ways to fill, all non-destructive

Edit > Remove (and the Remove tool, J, sharing the Healing slot as the toolbar audit decided)
works on the selection, or on a brushed area. It asks what goes there:

- **Transparent**: the active layer's mask hides the area. If the layer has no mask, the mask
  is made from the selection, as Layer > Layer Mask > Hide Selection. Nothing else changes.
- **Background color**: a fill layer of the background color, masked by the area, above the
  active layer.
- **Generated**: a **generative layer** (point 2) above the active layer, masked by the area,
  its content generated from what is visible around it.

All three stay editable and removable. Unlike ADR 0034 point 6, Remove is not baked into the
layer's paint. The generative layer keeps what it generated, so it does not need replaying.

### 2. The generative layer: an AI node, its result cached

A new kind of layer content holds an AI operation as CLAUDE.md requires. It stores:

- the task (fill, expand);
- the model's id and version, and its runtime;
- the prompt (empty for Remove);
- the seed and the parameters;
- the **source**: which visible layers it read (all below it, by default);
- the **region** and its context margin;
- the **mask**: the layer's mask, made from the area;
- the **result**: pixels at the document's resolution over the region, saved in the file,
  since they are expensive to recompute.

Its pixels are shown and composited like a pixel layer's, CPU and GPU alike.

**Stale, never recomputed silently.** When what it read changes (an edit below, within its
region and margin), the layer is marked stale:

- a badge in the Layers panel shows it;
- the Properties panel offers Generate again.

Its old result stays shown until then.

**Variations.** Generate again with another seed gives a variation. The last few variations
are kept, and the Properties panel switches between them, as Photoshop's Generative Fill
does.

Rasterize bakes the layer into plain pixels, like any layer.

### 3. Models: the best non-commercial one, and a commercial alternative

The model-choice rule applies: at most two choices; the best one; and since it is
non-commercial, a commercially usable alternative.

- **FLUX.2 [klein] 4B**, the default.
  - Apache-2.0: weights and outputs usable commercially.
  - 4 steps, so fast.
- **Qwen-Image 2.1**, optional.
  - It is never shipped: it is downloaded from its official repository on first use, as the
    generative model policy (2026-10-01) requires.
  - Its license is shown and must be accepted.
  - A "non-commercial: research and evaluation" mark stays visible on every layer it made.
    The marking is needed because its license grants nothing for outputs.

The Rust-side generative pipeline has a `Model` trait from the start. This is the second real
use the simplicity rule asks for: two models with different resolutions, steps and
conditioning.

The ComfyUI connector stays a later option for users who want other workflows. It runs over a
network API, so it does not link to ComfyUI and the GPL is not an issue.

### 4. Runtime (to choose)

Recommended: **a separate `slopshop-gen` executable built on stable-diffusion.cpp**.

- It is MIT-licensed and uses ggml.
- Its backends:
  - **Vulkan** on Windows and Linux, which covers NVIDIA, AMD and Intel;
  - **Metal** on macOS;
  - CUDA where present, faster on NVIDIA.
- It loads GGUF weights in fp8 or Q8 for the transformer, Q8 or Q4 for the text encoder.

It is a helper like `slopshop-ai` and `slopshop-raw`:

- it starts on first use and talks over a pipe in binary;
- it frees the VRAM when it stops;
- a crash ends the helper, not the editor.

The masked sampler (latent blending, point 5) is implemented there, in C++ around sd.cpp's
API, or in Rust over its C API. That leaves the selection models on ONNX Runtime and DirectML
as they are.

This choice is gated by a measured spike (point 5, step 0). If FLUX.2 [klein] 4B does not reach
a few seconds per 1 MP fill on the maintainer's RTX 4090 Laptop under Vulkan, the alternatives
are reopened.

### 5. Native definition: an experiment bench before large regions

**Architecture, fixed now** (from the research document):

- **Tiles and pyramid levels are first-class.**
  - The working resolution is a property of the model, never a constant.
  - The seed of a tile is derived from the layer's seed and the tile's coordinates.
- **Strict compositing in document space**, in linear light: outside the feathered mask, the
  pixels below are bit-identical.
- **Preview and final are separate**:
  - the preview is the coarse level, shown first;
  - the final render adds the native levels;
  - a VRAM and RAM budget is given to the scheduler, which chooses tile sizes from it.

**Step 0, the runtime spike.**

- It runs FLUX.2 [klein] 4B through stable-diffusion.cpp on Vulkan.
- It measures time, VRAM and fidelity against the reference (PyTorch diffusers):
  - at 1 MP and 2 MP;
  - in fp8 and Q8.

**Step 1, Remove on small regions: native by construction.**

- The region plus its margin is cropped at scale 1:1 (approach A of the research).
- It is generated when it fits the model's working resolution at 1:1: about 2 MP for FLUX.2
  [klein].
- No upscale is involved.
- Most removals are this case: blemishes, objects, passers-by.

**Larger regions.** Until the bench chooses a pipeline, a larger region is generated at the
coarse level only and marked "preview". It is never presented as native.

**Step 2, the bench.**

- It runs on the maintainer's images (24–100 MP: portrait, foliage, architecture, product or
  text, heavy grain) and the four tasks of the research:
  - small removal;
  - large replacement;
  - border outpainting;
  - structure-preserving edit.
- Pipelines compared:
  - B, the baseline;
  - A;
  - C, coarse-to-fine with partial re-noising;
  - C+D, tiles fused at every step;
  - C+E;
  - each with and without F, grain and high-frequency matching.
- Measures:
  - seams;
  - spectrum and grain inside against around;
  - out-of-mask exactness;
  - time and peak VRAM;
  - a blind A/B at 100 %.

The decision gate is an amendment of this ADR recording the chosen pipeline and its evidence.
Large regions and outpainting ship only after it.

### 6. Outpainting: Crop larger

When the crop box goes beyond the canvas, Crop's options offer how to fill the new area, as
Remove does:

- **Transparent**: today's behavior.
- **Background color**.
- **Generated**: a generative layer, its task "expand", its mask the new area.

The image is never resampled; only the canvas grows (ADR 0017). Outpainting is a large-region
task, so it follows step 2.

### 7. File format (to confirm)

The generative layer is a new `.slop` node type, `slopshop.generative` (a compatible
addition):

- its parameters as JSON;
- its result and its kept variations as images;
- a stale flag.

Older readers refuse it as an unknown type. The model weights are never in the file. Opening
it without the model shows the saved result: Generate again asks for the model.

## Alternatives

- **ONNX Runtime with DirectML** for the generative models:
  - It keeps one runtime.
  - FLUX.2 [klein] has only fresh community exports (int8/int4) with no DirectML validation.
    The int4 one is visibly less faithful (cosine 0.95).
  - Qwen-Image 2.1 has no export, and its block-causal attention makes one hard.
  - DirectML itself is frozen.
  - Kept as a fallback if the spike fails.
- **A Python sidecar** (PyTorch and diffusers):
  - It has the reference fidelity and day-0 support of every model.
  - But it costs several gigabytes of runtime to install, CUDA mostly, and is a packaging
    burden on three platforms.
- **ComfyUI only**: no packaging at all, but the user installs and runs another program. It
  remains an option (point 3), not the default.
- **Qwen-Image-Edit 2509/2511 (Apache-2.0)** as the commercial alternative instead of FLUX.2
  [klein]:
  - It would be commercially clean.
  - But it is about 58 GB (a 20B transformer), beyond a 16 GB card without heavy offload.
- **A non-generative remover** (MI-GAN, MIT, 28 MB ONNX; LaMa, Apache-2.0, 208 MB):
  - It would be instant and light.
  - It would be a third model for one function, which the two-choice rule forbids, unless it
    replaced one.
  - To reconsider if the spike shows generation too slow on modest GPUs.
- **Remove baked into the layer's paint** (ADR 0034 point 6 as written):
  - Simpler.
  - But it destroys the original pixels' visibility, keeps no model, seed or region, and
    cannot be generated again: contrary to CLAUDE.md's rule for AI operations.

## Consequences

- **New**:
  - the `slopshop-gen` helper, a native dependency stated in its PR (sd.cpp, ggml), and its
    model download with consent, as the selection models have;
  - a layer content and its `.slop` node;
  - the Remove tool and command;
  - Crop's fill option.
- **The license check grows**: `cargo deny` covers Rust crates; sd.cpp and ggml (MIT, C++) are
  vendored in the helper and listed in its notices.
- **Disk and VRAM**:
  - FLUX.2 [klein] 4B in fp8/Q8 is about 12 GB to download;
  - Qwen-Image 2.1 in Q4/Q8 is 10 to 17 GB;
  - neither is downloaded until a generative feature is first used.
- **The research document gains its results**, and this ADR its amendment, when the bench
  decides.
- **Untested platforms**: macOS (Metal) and Linux (Vulkan) are run by CI where possible; the
  maintainer tests Windows only.
