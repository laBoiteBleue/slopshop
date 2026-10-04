# 0007 — Color management and working space

Status: accepted (2026-09-29)

## Context

Universal import brings wide-gamut, HDR and high-bit-depth sources (RAW, EXR, P3/Adobe
RGB/ProPhoto photos, PQ/HLG). The engine composited in linear sRGB and assumed 8-bit sRGB
sources. Rules: never assume sRGB 8-bit, no silent or lossy conversions.

## Decision

1. **Working space: linear Rec.2020 (D65), unbounded, float.** Layers are composited in it with
   premultiplied alpha. Values below 0 or above 1 are legal (HDR, out-of-gamut) and survive
   until the display transform.
2. **Source pixels are never converted at import.** Each raster keeps its native samples and a
   color description: RGB primaries + white point, transfer function (linear, sRGB, gamma,
   parametric, PQ, HLG…), or "assumed sRGB" when the file says nothing.
3. **Conversion to the working space happens at render time, on the GPU**: decode the transfer
   function, then one 3×3 matrix (primaries + Bradford chromatic adaptation to D65), in f32,
   unbounded. This covers sRGB, Display P3, Adobe RGB, ProPhoto, Rec.2020 and matrix/TRC ICC
   profiles.
4. **ICC profiles**: matrix/TRC profiles are read by an in-house parser in `slopshop-io` (pure
   Rust) into the description above; tone curves given as tables are fitted to a parametric
   curve and a warning is shown if the fit is not exact. v2 profiles without `chad` are
   un-adapted from their media white (`wtpt`). LUT-based profiles (Lab, device links,
   A2B-only RGB) are not supported yet: the pixels are imported unconverted, read as sRGB
   (linear for float data), with a visible warning. CMYK pixels are refused. Both will go
   through lcms2 (MIT) baking a float 3D LUT, never its GPL plugins.
   Other declared color is honored the same way: PNG cICP > iCCP > sRGB > gAMA/cHRM (PNG
   3rd edition order), the QOI linear flag, OpenEXR chromaticities (primaries of linear data);
   what cannot be represented gives a warning.
5. **Display transform**: working space → display space (sRGB for now), clipping only at that
   last step. HDR display comes with the native surface presentation (ADR 0002): the webview
   canvas is 8-bit sRGB.
6. **UI colors** (color pickers, swatches) are sRGB-encoded and converted explicitly to and
   from the working space; out-of-gamut working colors are clipped for swatches only.

## Alternatives

- **ACEScg**: even wider gamut, the VFX standard, but less familiar for photography and print.
- **Unbounded linear sRGB** (previous behavior): least disruptive, but wide-gamut sources become
  negative values everywhere, which complicates tools and masks.
- **Convert at import** (bake into the working space): simpler rendering, but destroys the
  source encoding and makes precision depend on storage format.

## Consequences

- Every raster carries its color description; the renderer applies a per-layer conversion.
- Fill colors and other document colors are stored in the working space.
- Gray sources are treated as D65-neutral luminance.
- Soft proofing, CMYK and HDR output are future work built on the same model.
- Soft proofing and a gamut warning (View menu) need, in this order: the display's own
  profile (read from the system) as the display transform's target instead of sRGB; LUT-based
  profiles and CMYK through lcms2; CMYK documents or at least CMYK output profiles in the
  engine; then a proof transform (working space → output profile → display) and a gamut test
  in the display shader. No menu entry exists before they do (noted 2026-10-04).
