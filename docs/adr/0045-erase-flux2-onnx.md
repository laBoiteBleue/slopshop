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

### 8-bit weights (asked by the maintainer, 2026-10-08)

`--weights` selects how the transformer stores its projections.

- **8-bit blocks:**
  - symmetric quantization when the model loads, the LoRA merged first;
  - one half-precision scale per block of inputs of each output;
  - run by ONNX Runtime's `com.microsoft.MatMulNBits`, which DirectML executes.
- **Per channel:** one scale per output, then `DequantizeLinear` and `MatMul`.

All variants pass the three vectors. Latent error, then PSNR inside the selection, for vectors
01 / 02 / 03:

| Storage | Weights | Latent error | PSNR | Step 01 / 03 | Total 01 / 03 | Peak VRAM 01 / 03 |
|---|---|---|---|---|---|---|
| half (above) | 8.48 GB | 1.29 / 1.52 / 1.54 % | 50.8 / 45.8 / 48.2 dB | 1.79 / 2.85 s | 8.3 / 12.4 s | 13.7 / 15.3 GB |
| 8-bit, blocks of 32 | 5.03 GB | 1.40 / 1.61 / 1.63 % | 50.4 / 45.5 / 48.0 dB | 2.05 / 3.08 s | 9.5 / 13.5 s | 10.4 / 12.1 GB |
| 8-bit, blocks of 64 | 4.91 GB | 1.41 / 1.63 / 1.65 % | 50.1 / 45.4 / 47.7 dB | 2.05 / 3.08 s | 9.5 / 13.5 s | 10.3 / 12.0 GB |
| 8-bit, blocks of 128 | 4.86 GB | 1.45 / 1.66 / 1.73 % | 49.0 / 44.9 / 47.3 dB | 2.07 / 3.08 s | 9.5 / 13.5 s | 10.2 / 11.9 GB |
| 8-bit per channel | 4.80 GB | 1.59 / 1.87 / 1.92 % | 47.6 / 44.1 / 45.6 dB | 1.88 / 2.95 s | 8.9 / 13.1 s | 12.8 / 14.5 GB |
| 8-bit, blocks of 64, modulations too | **4.37 GB** | 1.41 / 1.64 / 1.66 % | 49.9 / 45.2 / 47.7 dB | 2.05 / 3.08 s | 9.5 / 13.6 s | **9.8 / 11.4 GB** |

How to read these numbers:

- **Quality.** The quantization costs about 0.1 point of latent error and under 1 dB of PSNR
  against the bfloat16 reference, far within the protocol's thresholds. It is small next to the
  gap between the half-precision port and the reference.
- **Speed.** MatMulNBits is about 15 % slower per step than half precision.
- **Per channel** is the fastest of the 8-bit variants, but each weight is dequantized whole
  before its product. That transient memory brings its peak back near half precision's, and
  its quality is the lowest of the variants.
- **The last row** also stores the modulations and the prompt's embedder in 8 bits: 1.6 to
  3.0 GB less at the peak than half precision.
- **Peak VRAM** is the whole card as `nvidia-smi` reports it, about 1 GB of it taken by the
  desktop and other applications. So the tool itself peaks at about 8.8 GB at 1024×768 and
  10.4 GB at 1168×880. Of that, the activations of the steps (cached by DirectML's allocator)
  account for about 6 GB at the larger size.

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
- **VRAM:**
  - Half precision takes 14 to 15 GB of a 16 GB card.
  - 8-bit weights (blocks of 64, modulations included) bring the tool to about 9 GB at
    1024×768 and 10.4 GB at 1168×880.
  - A 12 GB card should run up to 1024×768, but is tight at the 1 Mpx maximum; reducing the
    steps' activations (about 6 GB) is the next lever.
  - Proposed: the precision is a hardware choice made automatically, not a user setting. Half
    precision from 16 GB of VRAM, 8 bits below.
  - Nothing was measured on a smaller card.
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
4. **Smaller GPUs:** 8-bit weights evaluated (above): conforming, 15 % slower, 3 GB less.
   Choose automatically, with half precision from 16 GB and 8 bits below? Or 8 bits everywhere,
   for one code path at the cost of 15 %?
