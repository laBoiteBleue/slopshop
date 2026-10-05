# 0018 — Resampling transformed layers

Status: accepted (2026-09-30). Point 4 amended by [ADR 0038](0038-projective-transforms.md): a
projective layer takes its level and ellipse from the Jacobian at each pixel.

## Context

ADR 0017 stores a 2D affine per layer and left the resampling filter to the step that allows
transforms other than whole-pixel moves. The filter decides how a scaled or rotated layer looks
in the viewport and in exported files. The target is the quality of Photoshop's best classical
(non-AI) resampling: sharp enlargements without halos or jaggies, reductions without moiré.
Adobe does not publish its "Preserve Details" algorithm. Transforms are arbitrary affines
(rotation, shear, uneven scales), so a filter that only works along the axes does not fit. The
same result is needed on the CPU (reference, export fallback) and on the GPU.

## Decision

1. **Elliptical weighted average (EWA) with a Jinc-windowed Jinc ("EWA Lanczos sharp")**, the
   kernel ImageMagick's distort and mpv use as their best classical filter: radius 3.2383 (the
   third zero of Jinc), blur 0.98125 (Robidoux's sharpening). The same filter for every
   transform: it is rotation-invariant, and the ellipse follows the transform. It is round
   (radius 1 texel) when enlarging, and stretched to the output pixel's footprint when reducing,
   so there is no aliasing. Texels outside the image are transparent: edges are antialiased.
2. **Anti-ringing**: the result is clamped, channel by channel, to the range of the texels
   nearest to the sample point (those within √2 of it in the ellipse's metric). This removes
   the halos of the kernel's negative lobes at hard edges and keeps its sharpness elsewhere
   (mpv's approach, at full strength).
3. **Pyramid levels bound the cost**: the level is the coarsest one whose texels are still
   no larger than the output pixel's minor axis (2×2 box levels in linear light, ADR 0005). It
   is coarsened further if the ellipse's major radius would exceed 8 texels (strong
   anisotropy). A sample reads at most 17×17 texels, and 7×7 when enlarging.
4. **Everything is computed per layer on the host**: the level, the map from document pixels to
   the level's texels, the ellipse's quadratic form and its bounding box (the transform is
   affine, so they are the same for every pixel). The kernel is a 1024-entry table over r²,
   built once in `slopshop_core::resample` and shared by the CPU and the GPU.
5. **Exact cases stay exact**: whole-pixel moves (already), and 90° rotations and flips with
   whole-pixel placement copy texels (no filtering) in export and when zoomed in.
6. **The viewport shows document pixels**: zoomed in, a transformed layer is sampled at the
   center of each document pixel with the export's filter, so what is seen is what is exported
   (square pixels, like the other layers). Zoomed out, it is sampled at each output pixel with
   the ellipse of that pixel's footprint.
7. **Resampling happens in linear light** (the working space), with premultiplied alpha: no
   dark fringes at transparent edges, and reductions keep their brightness.
8. **Transforms must be finite and invertible** (|determinant| ≥ 1e-9, magnitudes < 1e9);
   edits, restore and `.slop` reading refuse others.

## Alternatives

- **Separable Lanczos-3 (tensor)**: slightly sharper, cheaper for plain scaling, but not
  rotation-invariant: rotated layers alias, and a second path would be needed for them.
- **Bicubic (Photoshop's default family)**: cheap, softer, visible stair-stepping on enlarged
  diagonals.
- **FSR 1 EASU + RCAS**: edge-adaptive and fast, but designed for 1×–2× enlargements of
  rendered frames, not for reductions or rotations, and hard to match exactly on the CPU.
- **AI upscaling (Photoshop's Preserve Details 2.0, Super Resolution)**: out of scope here; it
  belongs to AI nodes (see `docs/research/hd-generative-ai.md`).
- **Resampling in the blend space (sRGB-like)**: closer to what Photoshop does, but reductions
  darken thin light details and the pyramid is linear. It can be revisited with comparisons.

## Consequences

- A per-layer choice of filter (nearest, bicubic, …) can be added later as a layer parameter,
  without changing the default.
- Sampling a transformed layer costs 33 to 289 texel reads per pixel instead of one: fine on
  the GPU. The CPU fallback is slower for export.
- The kernel table and the exact cases are tested; the GPU is checked against the CPU on scaled,
  rotated and masked layers.
