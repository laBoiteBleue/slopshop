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
   - Levels (RGB): input black and white, gamma, output black and white.
   Curves (with an editor) and others follow on the same model.
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
