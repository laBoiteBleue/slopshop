# 0042 — Patterns: image sources repeated across a plane

Status: proposed (2026-10-07, drafted for the maintainer, who approved patterns as a track the
same day; the questions at the end are theirs).

## Context

Photoshop uses patterns in six places: Edit > Define Pattern (a rectangle of the image becomes a
pattern), the Patterns panel (the user's library), Layer > New Fill Layer > Pattern, Edit > Fill
> Pattern, the Pattern Overlay layer style and the Pattern Stamp tool (and Stroke's pattern
fill, View > Pattern Preview for designing seamless ones). The feature map has them `open`,
waiting for a pattern model.

SlopShop already has what a pattern needs to be: an image kept once and referenced
([ADR 0040](0040-sources.md)), fill layers placed by their transform ([ADR 0017](0017-non-destructive-transforms.md)),
layer styles drawn as effect layers ([ADR 0032](0032-layer-styles.md); Color and Gradient
Overlay already are fill layers within the shape), paint in a layer's stack
([ADR 0029](0029-layer-stack.md)) and resampling shared by the CPU and the GPU
([ADR 0018](0018-resampling.md)).

## Decision (proposed)

1. **A pattern is an image source** (ADR 0040) used as a tile: its pixels repeat across the
   plane, its origin at the layer's origin. A document keeps the patterns its layers use as
   sources, shown in the Sources panel like the others; nothing is copied when several layers
   use one.
2. **The library** (the Patterns panel and the pickers) lives outside documents, in the app's
   data folder, as images with a name (one `.slop`-like file per pattern, or a folder of PNG
   files with a small index; *question 1*). Choosing a pattern from the library for a fill makes
   a source of it in the document; a document opened elsewhere keeps its patterns.
3. **Define Pattern** (Edit menu): the rectangle of the selection (the whole canvas without
   one), of what the image shows (all visible layers merged, as Photoshop) or of the active
   layer (*question 2*), named in a dialog, added to the library.
4. **Uses, in this order**:
   - **Pattern fill layers** (Layer > New Fill Layer > Pattern): a new fill content holding the
     pattern source, a scale (1–1000 %), an angle and "Link with Layer" (the transform moves the
     pattern), edited in the Properties panel as gradient fills are.
   - **Pattern Overlay** (layer style): a pattern fill effect layer within the shape, as Color
     and Gradient Overlay; scale, angle, opacity, blend mode, Snap to Origin.
   - **Edit > Fill > Pattern**: paint in the layer's stack (the pattern's pixels baked into the
     paint, as every Fill is), Script patterns left out.
   - **Pattern Stamp**: the Clone Stamp's paint with the pattern as its source (Aligned or
     not, as Photoshop).
5. **Rendering**: a pattern sampled with wrapping through the layer's map (scale and angle
   composed with its transform), filtered by the same resampling as transformed layers
   (EWA from the pyramid level matching the scale), CPU and GPU tested against each other.
   Edge pixels wrap, so a seamless pattern stays seamless at any scale.
6. **`.slop`**: a `slopshop.patternFill` node naming its source (`document.sources`), with
   scale, angle and link; Pattern Overlay in `params.style` at a new node version, as Gradient
   Overlay was. PSD: pattern fill layers and Pattern Overlay read and written when the pattern
   is embedded (`Patt` resources), the stored pixels kept otherwise.

## Alternatives

- **Patterns only in documents** (no library): simple, but Define Pattern would have nowhere
  to put a pattern for the next document.
- **Patterns only in the library, documents naming them**: smaller files, but a document opened
  on another computer would lose its patterns.
- **A pattern fill drawn into pixels when made** (Photoshop's Fill): fine for Edit > Fill, but a
  fill layer should stay editable (scale, angle) as gradient fills do.

## Consequences

- A new fill content beside the solid color and the gradient; the compositor and the shader
  gain a wrapped, resampled image fill.
- The Patterns panel (a dock panel, ADR 0036) and a pattern picker shared by the fill dialog,
  the layer style dialog, the Properties panel and the Pattern Stamp's options.
- Photoshop's `.pat` files could be imported into the library later (their format is
  documented); not in the first version.

## Questions for the maintainer

1. The library's storage: a folder of PNG files with an index (easy to share, other apps can
   read them), or one file per pattern keeping 16-bit and color spaces exactly?
2. Define Pattern from what the image shows (Photoshop's) or from the active layer?
3. Built-in patterns: none, or a few generated ones (checkers, stripes, dots, noise) so that the
   library is not empty at first?
4. A Patterns panel in the dock, or only the pickers (one less panel)?
5. Pattern Preview (View menu, editing a pattern seamlessly across its edges): with the first
   version or later?
6. Order of work as above (fill layers, Overlay, Fill, Stamp), or another?
