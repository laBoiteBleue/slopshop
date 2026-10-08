# 0045 — Erase: FLUX.2 [klein] on ONNX graphs written by SlopShop

Status: proposed (2026-10-08, spike measured; awaiting the maintainer's answers below).

## Context

The maintainer supplied an integration package for the first generative tool, **Erase**: the
user selects an object, and the tool replaces the selection by the background. No pixel outside
the selection changes. It runs FLUX.2 [klein] 4B **turbo** (Apache-2.0) with a LoRA,
`erase_v1` (Apache-2.0, distilled from fal's object-remove LoRA), in 4 transformer passes
without CFG. No text encoder is needed: the prompt's embedding is precomputed.

The package holds a protocol (`PROTOCOL.md` v1.0.0), an executable Python reference, and three
test vectors. A port conforms when, with the given noise:

- the sigmas match within 1e-6;
- the final latent is within 3 % relative error;
- the PSNR inside the selection is above 40 dB;
- the result outside the selection is bit-identical to the input.

The AI helper (ADR 0025) runs ONNX Runtime with DirectML on Windows. BFL publishes no ONNX
version of klein, DirectML has no bfloat16, and the maintainer excludes Python. The maintainer
chose (2026-10-08) to **write the graphs ourselves**, then to measure a spike before any UI.

## Decision

1. **SlopShop writes the ONNX graphs** of the transformer and the VAE (`slopshop-ai::flux2`,
   with a minimal protobuf writer in `slopshop-ai::onnx`, hand-encoded, no dependency). The
   graphs are built from the official diffusers weights at a pinned revision, read in place
   (`safetensors`). The weights reach ONNX Runtime from memory as external data: there is no
   second copy on disk. The LoRA is merged into the 100 layers it targets (`W + B·A`) when the
   model loads, which takes about 12 s of CPU for 8.5 GB.
2. **Precision.**
   - The transformer's residual streams, LayerNorms, RMSNorms, rotary embedding and
     modulations stay in single precision.
   - Its large projections and the attention run in half precision. The bfloat16 weights
     convert exactly.
   - The VAE runs in single precision.
3. **One graph, one session.** The VAE encoder, the transformer and the VAE decoder share one
   ONNX Runtime session, and therefore one DirectML allocator: the decoder reuses what the
   transformer's steps freed. Run in separate sessions, the decoder took 5 to 7 s instead of
   0.4 s, because VRAM overflowed into shared memory. ONNX Runtime still runs the whole graph
   at each call, so the parts not asked for get tiny inputs; their cost is negligible.
4. **Memory and speed in the graph.**
   - Fused projections are cut by their weights (q, k, v, the MLP's halves), not by their
     outputs. The original form cost 21 s per step against 1.7 s.
   - The attention runs over chunks of queries, 16 by default.
   - The sequence is padded to a multiple of 8. An odd length (12 557 tokens for test vector
     03) leaves DirectML's fast half-precision path: 5.7 s per step against 2.85 s.
   - The padding is masked exactly: each head gets 8 more dimensions, 1 in the queries, and
     −4096 per dimension in the padding's keys. A padding score is then about −32 768, and its
     weight after the softmax is zero.
5. **The protocol's steps around the model** live in `slopshop-ai::erase`:
   - working size and sigma schedule;
   - the timestep as the model rounds it;
   - rotary tables, latent packing, Euler steps in bfloat16;
   - the strict composite;
   - Pillow's Lanczos and nearest-neighbour resampling, reproduced exactly.

   Each is unit-tested, against `case.json` where the vectors give values.

## Measurements (RTX 5070 Ti 16 GB, ONNX Runtime 1.24.4 DirectML)

`cargo run --release -p slopshop-ai --features helper --example erase_spike` runs the three test
vectors.

| Vector | Working size | Latent error (< 3 %) | PSNR in selection (> 40 dB) | Outside | Step | Total |
|---|---|---|---|---|---|---|
| 01 dog, tight selection | 1024×768 | 1.29 % | 50.8 dB | identical | 1.79 s | 8.4 s |
| 02 car, coarse selection | 1024×768 | 1.52 % | 45.8 dB | identical | 1.79 s | 8.0 s |
| 03 bike, 2048×1536 | 1168×880 | 1.54 % | 48.2 dB | identical | 2.85 s | 12.4 s |

For comparison, the integration measured diffusers against the reference at 0.9 to 1.0 % and
46.8 to 50.9 dB.

- "Total" covers encoding the photo and the mask, 4 steps, decoding and compositing.
- Loading takes about 25 s once: the graph and the LoRA merge on the CPU, then the session.
- VRAM after a run: 13.6 to 15.3 GB of 16 GB (weights 8.5 GB).

Tried and dropped:

- ONNX Runtime's fused `MultiHeadAttention`: in one kernel the GPU stopped responding (device
  hung). In 16 chunks it was 8 % faster on vector 01 (before padding), but in 4 chunks it failed
  on 03, and it does not take the padding mask as written.
- 32 query chunks: slower than 16, and no memory saved.

## Alternatives

Reviewed with the maintainer on 2026-10-08, before the spike:

- cgb's community int8 ONNX graph: third-party, and its int8 weights prevent a plain LoRA merge.
- stable-diffusion.cpp: a native C++ dependency, a patch needed for the precomputed embedding
  and the reference ids, and LoRAs had no effect there in an earlier spike.
- Our own wgpu inference: far more work, without tensor cores.

## Consequences

- **Download:** the transformer (7.75 GB) and the VAE (0.17 GB) from BFL's repository, pinned
  by revision and SHA-256, plus the LoRA (0.19 GB) and the embedding (8 MB).
- **VRAM:** about 14 to 15 GB used on a 16 GB card. Cards with 12 GB or less do not fit as is.
  DirectML runs ONNX Runtime's 8-bit and 4-bit weight-only `MatMulNBits` (used for LLMs), which
  would halve the weights. Its quality with the LoRA is untested, and the protocol lists FP8 +
  LoRA as an open point too.
- **Platforms:** Windows only for now. Core ML (macOS) and the CPU (Linux) are not candidates at
  this size without further work. Nothing here was tested on them.
- **Large photos:** the tool works at about 1 Mpx then upscales the result, so on a 24 Mpx photo
  the erased area is soft. The protocol recommends working on a crop around the selection; the
  steps are the same.
- **Other tools** (inpaint, outpaint, tiled upscale) reuse the same graph with another LoRA, an
  embedding, and possibly other reference ids.

## Open questions for the maintainer

1. **Hosting** of `erase_v1` (LoRA, embedding): a Hugging Face repository of the project (for
   example `laBoiteBleue/slopshop-erase`), pinned by SHA-256 in the AI manifest like the other
   models?
2. **Product:** Erase as a non-destructive entry in the layer stack, as decided on 2026-10-05
   (model, seed, selection, cached result, marked stale and never recomputed silently)? Should
   the selection be dilated by about 2 % automatically, as the protocol recommends, or should
   the dilation be a visible option?
3. **Large photos:** work on a crop around the selection (for example its box enlarged ×2, at
   most 1 Mpx at the model)? This departs from the test vectors only for photos above 1 Mpx.
4. **Smaller GPUs:** require 16 GB for the first version, or evaluate 8-bit weights first?
