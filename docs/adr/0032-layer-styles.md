# 0032 — Layer styles: effects drawn from a layer's shape

Status: accepted (2026-10-04). Decided by the maintainer in the Layer menu audit (layer styles
as in Photoshop, an editable list per layer computed after its own stack and mask, sharing
the stack's primitives; not entries of the stack), then the four choices below ("je prends").

## Context

Photoshop's layer styles (Drop Shadow, Inner Shadow, Outer and Inner Glow, Stroke, Color,
Gradient and Pattern Overlay, Satin, Bevel & Emboss) are drawn from a layer's shape (its
alpha) and stay editable: a shadow follows the layer when it is painted, moved or
transformed. SlopShop has none yet: PSD import drops them (`lfx2`) and reports it.

They cannot be entries of a layer's stack (ADR 0029): stack entries are applied to the
layer's own pixels before its mask, inside its bounds, and are never edited, while effects
read the final shape (after the stack, and the mask unless asked otherwise), draw around it
(a shadow beyond the layer's pixels) and are edited all the time. Nor are they adjustment
layers: they belong to one layer and move with it.

What they need already exists for selections, on gray coverage images
(`slopshop_core::selection`): Feather (a Gaussian blur), Expand and Contract at an exact
distance, and Edit > Stroke's band inside, centered on or outside an outline (Photoshop's
Stroke effect positions).

## Decision

1. **A layer holds a style**: `Layer::style: Option<LayerStyle>`, a fixed list of effects in
   Photoshop's order, each with `enabled` and its parameters (color, blend mode, opacity,
   distance and angle, spread or choke, size, position for Stroke…), and the style's own
   settings: **Fill Opacity** (the content's opacity, effects untouched, as Photoshop's
   "Fill"), "Layer Mask Hides Effects" (off by default, as in Photoshop). Pixel and fill
   layers and groups may have one; adjustment layers not, as in Photoshop. One edit,
   `SetLayerStyle`, undoable; live while a dialog's slider moves (a gesture).
2. **Effects are images drawn from the shape**: the layer's coverage (its evaluated pixels'
   alpha in document space, through its transform, and its mask when it hides effects) goes
   through the selection's coverage operations: a shadow is the coverage contracted by
   Spread, feathered by Size, offset by Distance and Angle, colored; a glow is it expanded and
   feathered; Stroke is the band at its position; an overlay is the color over the shape;
   inner ones are inverted coverage clipped to the shape. One engine for selections, Edit >
   Stroke and effects, not a second implementation.
3. **Composited with the existing steps**: a styled layer is composited as an isolated group
   would be: the effects below it (shadows, outer glow), the content at Fill Opacity, the
   effects above it (overlays, inner shadow and glow, stroke), clipped to the content where
   Photoshop clips them; then the whole blended with the layer's mode and opacity. No new
   kind of step: effect images are composited as raster layers with their blend mode.
4. **Evaluation is a cache**, as a stack's (ADR 0029 point 6): an effect's image is computed
   per tile, on every core, where it reaches (the shape's bounds grown by its reach), and kept
   while the shape and the style are the same; editing a parameter recomputes that effect only.
   Zoomed out, effects are computed on the coarser level with distances scaled (close, as
   adjustment views are). The GPU computes them later, as an optimization behind the same
   model.
5. **Files**: `.slop` nodes gain `params.style` (a new node version); PSD import reads `lfx2`
   into the effects SlopShop draws (reporting the others), PSD export writes them back.
6. **Order of work**: the model, Drop Shadow, Stroke and Color Overlay with their tests
   (the CPU compositor first); the Layer Style dialog; Outer and Inner Glow, Inner Shadow;
   PSD import and export; then Gradient Overlay (with a gradient engine, shared with the
   Gradient tool and gradient fill layers), Pattern Overlay (with patterns), Satin and Bevel
   & Emboss.

## Alternatives

- **Effects as entries of the stack**: they would be applied before the mask, inside the
  layer's bounds, and never edited (ADR 0029): wrong for a shadow.
- **Effects as layers of their own** (Photoshop's "Create Layers"): the panel fills up and the
  effect no longer follows the layer.
- **A filter engine of its own for effects**: a second implementation of what selections
  already do.

## Consequences

- A styled layer costs a composite of its effects when it or its style changes, not per frame
  (cached).
- Painting a styled layer updates its shadow as the stroke goes, at the cost of recomputing
  the effects of the tiles it reaches.

## The maintainer's choices (2026-10-04)

1. **The layers panel**: an "fx" mark on a styled layer, and its effects listed below it, each
   with an eye, in the list that already shows a stack's entries (folded at first).
2. **Editing**: Photoshop's Layer Style dialog: the effects listed on the left with their
   checkboxes, the selected one's settings on the right, the canvas previewing live; opened by
   a double-click on the layer, the fx button or Layer > Layer Style.
3. **Fill Opacity**: a second field next to Opacity in the layers panel, as Photoshop's "Fill".
4. **First effects**: Drop Shadow, Stroke and Color Overlay, then the glows and Inner Shadow.
