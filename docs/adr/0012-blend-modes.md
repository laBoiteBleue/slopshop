# 0012 — Blend modes and blend space

Status: accepted (2026-09-30).

## Context

Layers only had an opacity and a visibility, composited with premultiplied "over" in the linear
working space (linear Rec. 2020, unbounded values; ADR 0007). Blend modes (multiply, screen,
overlay…) are the base of non-destructive retouching, and PSD files are full of them. Their
result depends on the space the formulas work in: Photoshop blends 8/16-bit documents on
gamma-encoded values (the look everyone knows, and what a PSD expects), and 32-bit documents on
linear values (physically meaningful: multiply filters light, add sums it), where it disables the
modes that assume values in `[0, 1]`.

## Decision

1. **Every layer has a blend mode**, one of Photoshop's modes except Dissolve (it needs a
   dither pattern; later): normal; darken, multiply, color burn, linear burn, darker color;
   lighten, screen, color dodge, linear dodge (add), lighter color; overlay, soft light, hard
   light, vivid light, linear light, pin light, hard mix; difference, exclusion, subtract,
   divide; hue, saturation, color, luminosity. Identified by stable camelCase ids (`colorBurn`,
   `linearDodge`…) in the IPC and in `.slop` files.
2. **Every document has a blend space** (maintainer decision): **perceptual** (the default for
   new documents: values encoded with the sRGB curve on sRGB primaries, Photoshop's 8/16-bit
   look) or **linear** (the working space itself). The blend space applies to every layer,
   normal mode and opacity included, as in Photoshop. Documents saved before blend modes existed
   (`.slop` schema 0.1) open in linear, which is how they were composited.
3. **Compositing formula** (W3C Compositing and Blending, "source over" with a blend function
   `B`), with colors in the blend space: `αo = αs + αb − αs·αb`,
   `αo·Co = αs·(1 − αb)·Cs + αb·(1 − αs)·Cb + αs·αb·B(Cb, Cs)`, where `s` is the layer (its
   opacity folded into `αs`) and `b` what is below. The result goes back to linear working
   values after each layer, so exact paths stay exact: a normal layer that is opaque, or over
   nothing, gives its own color unchanged in either space (unedited sources still export
   bit-exact). In linear space, normal mode is the former premultiplied "over", unchanged.
4. **Formulas**: the W3C definitions for the modes it has (the non-separable ones with its
   `Lum`/`SetLum`/`SetSat`), and Photoshop's published behavior for the others: soft light is
   Photoshop's formula (`2·b·s + b²·(1 − 2s)` below ½, `2·b·(1 − s) + √b·(2s − 1)` above), not
   the W3C one; darker/lighter color compare the sum of the channels; vivid light is color burn
   / color dodge on `2s` / `2s − 1`; linear light `b + 2s − 1`; pin light; hard mix
   `b + s ≥ 1 ? 1 : 0`; divide `b / s`.
5. **Ranges** (values are unbounded: HDR, out of gamut): formulas are applied as they are, with
   guards so that results stay finite (division by zero, square roots of negative values). In
   perceptual space, linear burn, linear dodge, linear light, subtract and divide are clamped to
   `[0, 1]`, as Photoshop 8/16-bit does. In linear space they are only clamped at 0 (negative
   light has no meaning) and may exceed 1; the non-separable modes clip only below 0. Where a
   formula divides (divide, dodges, burns), values within 1e-6 of 0 or 1 count as 0 or 1:
   color conversions leave channels that should be 0 at about ±1e-9, and noise divided by
   noise is arbitrary (and differs between the f32 GPU and the f64 CPU).
6. **Both compositors** implement the same math: the CPU reference (`core::composite`, `f64`)
   and the GPU shader (`f32`); tests compare them.

## Alternatives

- **Always linear**: consistent with the rest of the engine and HDR, but overlay, soft light and
  the dodges look unlike Photoshop, and imported PSDs would not match their source.
- **Always perceptual**: Photoshop's 8/16-bit look everywhere, but no physically based option
  for HDR work.
- **Blending in the document's source space** (e.g. Adobe RGB for a PSD tagged so): the most
  faithful for PSD, kept for later as a third choice (the blend space can become any color
  space); sRGB covers the common case.

## Consequences

- `Layer` gains `blend_mode`, `Document` gains `blend_space`, with their `Edit`s (undoable).
- The `.slop` schema goes to 0.2: nodes carry `blend_mode` (node types at version 2, so that
  older readers refuse files they would render wrongly), the document carries `blend_space`.
- Export flattens transparency over the matte in the document's blend space, as a background
  layer would be (`ExportSpec::blend_space`, copied from the document).
- Perceptual blending in sRGB of colors outside the sRGB gamut can leave the gamut of a wider
  file slightly: reported as clipping, like any other.
- Perceptual compositing costs two color conversions per blended layer and pixel (GPU: a few
  `pow` per layer); opaque normal layers stay free.
- New documents composite translucent layers differently from before (perceptual instead of
  linear): closer to other editors.
