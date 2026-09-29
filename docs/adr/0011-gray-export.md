# 0011 — Gray export

Status: accepted (2026-09-30). Extends [ADR 0008](0008-export.md) and
[ADR 0010](0010-jpeg-webp-export.md).

## Context

ADR 0008 left gray targets out: the converter wrote RGB and RGBA only. A gray source (scans,
medical or scientific images, black-and-white photos) exported as RGB triples its size and loses
its nature, and some deliveries require gray files. Gray has no primaries: a gray sample is a
luminance encoded with a tone curve. Turning a color composite into gray is a lossy conversion,
which must be explicit and reported.

## Decision

1. **Gray is the luminance.** The converter takes the color to the target primaries, then keeps
   `Y = row 2 of the RGB → XYZ matrix · RGB` (the white has `Y` = 1), before un-premultiplying,
   clipping and encoding as usual. This is how ICC gray profiles define gray (gray ↔ `Y` of the
   connection space). A neutral color keeps its value exactly, so gray sources round-trip
   bit-exact. Pixels whose channels differ from `Y` by more than the rounding noise are counted
   and reported (`colorDiscarded`, in pixels).
2. **An explicit option**, `gray` in `ExportSpec` (`--gray` / `--color` in the CLI, a checkbox in
   the app). It is on by default when the document is gray by construction (every visible layer a
   gray raster or a neutral fill, one raster at least): then nothing is lost.
3. **Formats.** PNG (gray, gray + alpha, 8/16-bit), TIFF (`BlackIsZero`, 8/16-bit and 32-bit
   float, with `ExtraSamples` for alpha) and JPEG (one component). Tagging: the sRGB chunk (PNG,
   sRGB curve) or an ICC v4 gray profile (`kTRC` as an exact `para` curve, plus `chad`). So PQ and
   HLG are not available for gray (no ICC curve, and PNG's cICP describes RGB only).
4. **Not for EXR and WebP.** WebP has no gray samples. EXR could store a `Y` channel, but our
   importer (the `image` crate) does not read luminance-only files back, so it waits.

## Alternatives

- **Average of R, G and B, or L\***: simpler or perceptual, but not what color-managed readers
  expect from a gray profile; a neutral source would still round-trip, colors would not map to
  their luminance.
- **Luminance in the working space** (Rec. 2020), whatever the file's space: gray would not match
  the profile the file declares.
- **A gray document mode** instead of an export option: a larger change of the model, for later
  if gray editing needs it.

## Consequences

- `ConvertError::UnsupportedLayout` is gone: every layout converts. `ConversionReport` gains
  `color_discarded`, `ExportNotice` gains `ColorDiscarded`.
- `supports_gray(kind, space)` and `has_gray(kind)` tell front ends which formats and spaces gray
  files can have; the app lists the spaces for the chosen samples.
- Gray JPEG ignores the subsampling setting (one component).
