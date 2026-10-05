# 0035 — Filters on the GPU

Status: accepted (2026-10-04, the maintainer's go for a branch of its own after Gaussian Blur).

## Context

A filter entry (ADR 0034) is computed on the CPU: the whole layer when its pixels are needed,
and for the display, the look at the part shown (about a screen of pixels): 0.1 to 0.2 s on 32
threads at 100 %, every time a setting changes. The maintainer wants filters fast enough to be
dragged at any zoom, and the other filters to come on the same machinery. The renderer already
holds a wgpu device; the core must stay free of it (dependency direction).

## Decision

1. **The renderer computes looks on the GPU.** A look's work (`stack::LookJob`: the tiles of
   what is below the filter around the part shown, at the level it is seen at, the filter's
   steps scaled to that level) is handed to the renderer's GPU filter (a function the display
   passes to `Pixels::look_for`, so that the core does not depend on wgpu). Compute passes blur
   the region along rows then columns with the exact Gaussian kernel, on a thread of its own
   (wgpu's device and queue are shared), and the result comes back as the look's image.
2. **What the GPU takes, and what stays on the CPU.** The GPU takes the common case first:
   8-bit RGBA sRGB layers in a perceptual document (their stored values are the blend values,
   decoded and encoded in the shader), and 8-bit RGB ones (a JPEG's: stored as RGBA with an
   opaque alpha, which every filter keeps opaque; 2026-10-05), steps without a selection, a
   reach of at most 1024 pixels at the look's level. Anything else (other formats, a
   selection, a huge reach) is computed by the CPU as before, and so is the whole layer for
   export, tools and the clipboard: the CPU stays the reference.
3. **Exactness.** Up to a radius of 8 both use the exact kernel and agree but for rounding;
   beyond, the CPU's three boxes approximate the Gaussian the GPU computes exactly: within a
   few levels of 8 bits. Tested against each other (skipped without an adapter, as the other
   GPU tests).
4. **Filters made from the blur** (Unsharp Mask, High Pass, 2026-10-04) run in the same two
   passes: the column pass makes each pixel from the input it blurred and its blur, as the
   CPU's `Filter::finish` does.
   Motion Blur (2026-10-04) is a pass of its own (`line_main`): the line's samples, read
   bilinearly, as the CPU's `Line`; beyond 256 pixels at the look's level the CPU samples the
   layer reduced, softening a few pixels across the line.
   Add Noise (2026-10-04) is a pass of its own (`noise_main`): the same 32-bit hash of the
   document pixel, the seed and the channel as the CPU's `filter::noise`.
   Dust & Scratches (`median_main`, up to a radius of 8 at the look's level) takes each
   channel's exact median by bisection on the value; Clarity and Texture run two separable
   passes, the first one's blur kept in the second half of the rows' buffer (devices may bind
   no more than four storage buffers).
5. **Later**: looks kept on the GPU and sampled by the display without a readback, other
   formats in the shader, the whole layer on the GPU, layer styles' blurs on the same passes.

## Alternatives

- **Only the blur on the GPU, decoding and encoding on the CPU**: simpler, but reading and
  writing the pixels on the CPU and moving floats both ways cost about as much as the blur
  (measured: barely 1.7× faster).
- **Everything on the GPU at once, looks never read back**: the fastest, but a new kind of
  raster in the display's tile caches and its shader; kept for later, once this measures.

## Consequences

- `slopshop-render` gains a filter shader and pipelines; the core a `LookJob` and the hook it
  is handed to.
- A machine without an adapter keeps the CPU path unchanged.
