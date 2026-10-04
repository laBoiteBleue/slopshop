# Research: generative AI at native high definition

Status: **open research** — no approach is chosen. This document frames the problem, lists
candidate approaches, the constraints they impose on the architecture, and the experiments
needed before deciding. Nothing described here is implemented.

## Problem

Generative image models work at a limited *working resolution* (typically ~1–2 megapixels).
Professional documents are often 24–100+ megapixels (8K, 12K, medium format, stitched
panoramas). Today, using a generative tool on such an image usually means:

- generating at working resolution and **upscaling** the result (soft, "plastic", mismatched
  grain; the image is effectively degraded where it was edited), or
- running the model on the full image (impossible or incoherent: repeated objects, broken
  global structure), or
- working on a small crop only (fine for tiny edits, fails for large ones).

**Goal:** generative operations whose result, inside the edited region, is **globally
coherent and detailed at the document's native resolution**, while pixels outside the region
are **strictly preserved**. A plain AI upscale of a low-resolution result is explicitly not the
goal (it may be a *component*, never the whole strategy).

## Success criteria

1. **Native output**: the generated region is produced at document resolution, not upscaled
   from a low-resolution result by a generic upscaler alone.
2. **Strict protection**: outside the (feathered) mask, output pixels are bit-identical to the
   input.
3. **Global coherence**: no visible tile seams, no duplicated content across tiles, consistent
   lighting, perspective and semantics over the whole region.
4. **Detail match**: sharpness, noise/grain and texture statistics inside the region match the
   surrounding original.
5. **Reproducibility**: same inputs + model + version + seed + parameters ⇒ same result, so a
   non-destructive node can be re-rendered.
6. **Bounded resources**: fits a VRAM budget (target to define, e.g. 8–12 GB) by construction,
   whatever the document size.
7. **Preview / final split**: a fast preview is available well before the full-definition
   render.

## Candidate approaches

They are not mutually exclusive; the likely answer is a pipeline combining several.

### A. Region of interest at native scale
Crop the ROI plus a context margin; if it fits the working resolution at scale 1:1, generate
directly at native resolution. Ideal for small edits (object removal, retouching). Breaks down
when the ROI is much larger than the working resolution.

### B. Downscale → generate → upscale (baseline only)
Generate at working resolution over ROI + context, upscale, composite. Serves as the
**baseline** every other approach must beat on criteria 1 and 4.

### C. Coarse-to-fine / multi-resolution (image pyramid)
Generate the global structure at a coarse pyramid level, then repeatedly upsample and
*refine* at finer levels with partial re-noising (SDEdit-style img2img at decreasing
strength), until native resolution. The coarse pass owns composition; fine passes add detail.
Related work: SDEdit, "hires fix" workflows, ScaleCrafter, DemoFusion.
Natural fit for the preview/final split (preview = coarse levels).

### D. Tiled diffusion with overlap and shared context
At levels larger than the working resolution, process overlapping tiles and fuse them *at
every denoising step* (averaging/weighting latents in overlaps) so tiles agree; add global
context through a low-resolution guidance signal or dilated/shifted sampling.
Related work: MultiDiffusion, Mixture of Diffusers, DemoFusion (skip residuals, dilated
sampling). Risk: content repetition and semantic drift without enough global guidance.

### E. Tile-conditioned refinement
Condition each fine tile on the upsampled coarse result (ControlNet-"tile"-like conditioning
or equivalent) so refinement adds detail without changing content.

### F. High-frequency management
- Re-inject the original's high frequencies where the edit should not change structure
  (relighting, color/style edits): low frequencies from the generation, high from the source.
- For newly created content, synthesize or transfer **matched noise/grain** so that the
  region's spectrum matches its surroundings.
- Blend across the mask boundary with Laplacian-pyramid or gradient-domain (Poisson) blending.

### G. Strict masking and compositing (always required)
Whatever the generator does, the final result is composited in document space, in linear
light, with the node's mask; outside the mask the original is untouched (criterion 2).

### H. Reference-based super-resolution as a stage
An SR model conditioned on the original (reference) may serve inside C/E — never as a generic
final upscale.

### I. Native high-resolution models
Future models may support larger resolutions natively. The architecture must not hard-code a
fixed working resolution; it should be a property of the model.

## What the architecture can already assume

Independent of the final choice:

- **AI operations are nodes** storing: parameters, prompt, model id + version, seed, mask, ROI
  (+ context margin), dependencies, and cached results.
- **Regions, tiles and pyramid levels** are first-class (already started in
  `slopshop-core::tile`); the generative pipeline reuses the same scheduler as rendering.
- **Caches keyed by** (node, parameter hash, input revision/hash, pyramid level, tile), so a
  preview (coarse levels) and a final render (all levels) share work, and upstream changes
  mark results as *stale* rather than recomputing them automatically.
- **Deterministic seeding per tile**: derived from the node seed and the tile coordinates.
- **Budgets**: the scheduler receives a VRAM/RAM budget and chooses tile sizes accordingly.

## Experiment plan

1. **Benchmark set** (licensed for the purpose): ~5 images of 24–100 MP covering skin
   (portrait), foliage (landscape), straight lines (architecture), text/product, strong film
   grain.
2. **Tasks**: small-ROI removal, large-ROI replacement (e.g. sky), outpainting of a border,
   structure-preserving edit (relighting/recolor).
3. **Pipelines**: B (baseline), A, C, C+D, C+E, each ± F.
4. **Metrics**: outside-mask bit-exactness; seam score (gradient discontinuities along tile
   borders); spectrum/noise statistics inside vs. around the mask; patch-level no-reference
   quality (and patch-FID against the originals); prompt adherence at coarse level; wall time
   and peak VRAM; blind A/B review at 100% zoom.
5. **Prototyping**: quick prototypes may live in a separate `experiments/` folder (Python
   allowed there, outside the product); results summarized back into this document.
6. **Decision gate**: before the first AI feature ships, an ADR records the chosen pipeline and
   the evidence.

## Related open questions

- **Inference runtime** inside a Rust application: ONNX Runtime, candle, burn, or a local
  sidecar process. Affects packaging, GPU sharing with wgpu, and model availability.
- **Model families and licenses** compatible with a GPL-3.0-licensed, local-first editor.
- **Minimum hardware** and CPU fallback policy (AI stays optional).

## References

- Meng et al., *SDEdit: Guided Image Synthesis and Editing with Stochastic Differential
  Equations*, ICLR 2022.
- Bar-Tal et al., *MultiDiffusion: Fusing Diffusion Paths for Controlled Image Generation*,
  ICML 2023.
- Jiménez, *Mixture of Diffusers for scene composition and high resolution image generation*,
  arXiv 2023.
- He et al., *ScaleCrafter: Tuning-free Higher-Resolution Visual Generation with Diffusion
  Models*, ICLR 2024.
- Du et al., *DemoFusion: Democratising High-Resolution Image Generation With No $$$*, CVPR 2024.
- Zhang et al., *Adding Conditional Control to Text-to-Image Diffusion Models* (ControlNet),
  ICCV 2023.
- Burt & Adelson, *A Multiresolution Spline With Application to Image Mosaics*, ACM ToG 1983.
- Pérez, Gangnet & Blake, *Poisson Image Editing*, SIGGRAPH 2003.
