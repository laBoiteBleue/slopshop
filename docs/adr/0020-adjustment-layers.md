# 0020 — Adjustment layers

Status: accepted (2026-09-30).

## Context

Color and tone corrections (exposure, levels, hue/saturation, curves…) are the most used edits
after transforms. In a non-destructive editor they must not rewrite pixels: Photoshop's answer,
the adjustment layer, is what its users expect, and it fits the layer tree (ADR 0015) and the
node model the document is heading to (ADR 0009).

## Decision

1. **A layer kind**: `LayerContent::Adjustment { adjustment }`, with the usual opacity,
   visibility, mask (ADR 0014), clipping (ADR 0016) and place in groups. It has no pixels and no
   bounds: it changes what is composited below it.
2. **Semantics** (Photoshop's): the adjustment is applied to the accumulator at its place in the
   stack, the result mixed with it by opacity × mask: `out = mix(below, f(below), coverage)`,
   alpha unchanged. In a pass-through group it changes everything below, outside the group too;
   in an isolated group only the group's content; clipped, only its clipping group.
3. **Where the math runs**: on straight (unpremultiplied) color in the document's blend space
   (ADR 0012): perceptual documents adjust sRGB-encoded values, as Photoshop does, linear ones
   linear values. Exposure is defined in linear light and always runs there.
4. **First adjustments**, parameters as in Photoshop:
   - Exposure: exposure (stops), offset, gamma correction;
   - Hue/Saturation (master): hue (−180…180°), saturation and lightness (−100…100);
   - Levels (RGB): input black and white, gamma, output black and white; from 2026-10-03 also
     per channel (red, green, blue), applied before the composite settings, as Curves.
   Then Brightness/Contrast (black and white kept: a gamma and an S-curve, as Photoshop's
   current mode behaves; Adobe's exact curves are not published), Vibrance, Invert, Posterize
   and Threshold; then Black & White (a gray from each color's hue family, optionally
   tinted), Color Balance (GIMP's model: shifts weighted by lightness ranges, HSL lightness
   optionally kept), Photo Filter (a multiplication in linear light, luminance optionally
   kept) and Channel Mixer. Up to 16 parameters each (flags as 0 or 1): on the GPU they use
   layer fields an adjustment has no other use for. Then Curves: up to 16 points per curve on
   Photoshop's 0–255 scale (composite, red, green, blue; each channel's curve, then the
   composite), a natural cubic spline through them, applied through 1024-entry lookup tables
   that both compositors interpolate the same way (on the GPU, appended to the tile table,
   so no new binding). Then Gradient Map (2026-10-03): up to 16 color stops (location 0–4096,
   sRGB 8-bit colors), interpolated linearly, the luminance (Rec. 601 weights) mapped through
   three lookup tables like Curves'; it always runs on sRGB-encoded values, whatever the blend
   space, so that the gradient's colors come out as chosen (Photoshop's smoothness is not
   reproduced). Then Selective Color (2026-10-03): for each of Photoshop's nine ranges, cyan,
   magenta, yellow and black (−100…100 %), relative or absolute; Adobe does not publish its
   math, which follows Clément Bœsch's measured model ("Understanding selective coloring in
   Adobe Photoshop", 2017: each range's weight from the largest, middle and smallest
   components, each component moved by `((−1 − ink)·black − ink)·m`, the ranges added up).
   Its 37 parameters (`PARAM_COUNT` grew to 37) go to the GPU in the tile table; it runs on
   sRGB-encoded values, as Gradient Map. Others follow on the same model.
5. **Blend mode**: normal only for now; the others (luminosity, color…) come later.
6. **Both compositors**: a step of the shared step list (`Step::Adjust`), computed identically
   by the CPU reference and the GPU (tested against each other).
7. **`.slop`**: a new node type, `slopshop.adjustment`, with the adjustment's kind and
   parameters; older readers refuse it as coming from a newer SlopShop.

## Alternatives

- **Filters that rewrite pixels** (Image > Adjustments in Photoshop): destructive; they can come
  later as an explicit, opt-in command.
- **Adjustments as layer properties** (a stack of effects per layer): less familiar, and an
  adjustment over several layers would need a group anyway.

## Consequences

- Everything that walks layers must handle a layer without pixels: bounds, pick (never
  picked), snapping (not a target), thumbnails (an icon), export defaults.
- PSD adjustment layers can be imported later onto this model.

## Amendment: Color Lookup (2026-10-19)

Status: accepted (2026-10-19, under the maintainer's delegation; to review).

Image > Adjustments > Color Lookup and its adjustment layer, as Photoshop's: a table loaded
from a file (3DLUT File, Load 3D LUT), the file's name shown.

1. **Files**: `.cube` (Adobe's specification, 3D or 1D, `DOMAIN_MIN`/`DOMAIN_MAX`, Resolve's
   `LUT_*_INPUT_RANGE`) and `.3dl` (Lustre and Flame: a mesh of input codes, then the output
   codes, bit depths inferred as other readers do). The parsers are in `slopshop-io`
   (`lut`), bounded and refusing damaged files. A 3D table has at most 129 points a side
   (`.cube` allows 256: 200 MB), a 1D one 65 536. Resolve's shaper (a 1D and a 3D table in one
   file) and the video-range keywords are refused rather than misread. Photoshop's Abstract
   and Device Link (ICC profiles) and its `.look` and `.csp` tables are left out.
2. **Where it applies**: on sRGB-encoded values (sRGB primaries and transfer) whatever the
   blend space, as Gradient Map: `.cube` and `.3dl` tables are made for display-referred
   values, and SlopShop documents have no other display encoding yet. A table made for
   another encoding (a log curve, Rec. 709) is applied to those values as they are, as when
   Photoshop applies it to an sRGB document; nothing is converted. Colors outside the table's
   domain are clamped to it, as grading software does (HDR values above white with them).
3. **Interpolation**: tetrahedral for 3D tables (smoother than trilinear on neutral colors:
   a gray reads only the cube's diagonal; the usual choice of grading software), linear for 1D
   ones; the same tetrahedra on the CPU and the GPU, tested against each other.
4. **The table is data of the document** (non-destructive, as Gradient Map's stops): shared
   by reference (`Arc`, immutable), so `Adjustment` is no longer `Copy`. In `.slop` (0.46) it
   is an image of its `f32` values, as Liquify's displacement: content-addressed records, so
   a table saved again is not written again, and a table used by several layers is stored
   once; the manifest keeps its name, dimension, size and domain.
5. **GPU**: the tables go into the tile table, once a dispatch, after every fill's own words
   (a layer's word says where its table starts): no further storage buffer (downlevel devices
   bind four per stage) and no texture.
6. **Dither** is left out: SlopShop composites in floating point, without the 8-bit rounding
   at this step that Photoshop's option hides. A PSD's Dither is reported as approximated.
7. **PSD** (`clrL`): imported when the embedded file is a `.cube` or `.3dl`; exported with the
   table written as a `.cube` file (lookup type 3D LUT, Dither off).
