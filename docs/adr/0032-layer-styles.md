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
   *Done (2026-10-04)*: the layer's coverage is computed once for all its effects (with room
   for a size to grow), each effect's mask kept by its geometry while the shape is the same: a
   color, mode, opacity or Fill change only recolors, an effect turned on again comes back, a
   layer moved by whole pixels moves its masks, and renaming, hiding, a new opacity or mode
   keep them. The display never waits for effects: computed in the background (one
   computation per layer at a time, the newest asked for next), what the layer's style drew
   last shows meanwhile and the frame asks to be shown again (as stacks do); exports and
   exact renders wait. The frames of a paint stroke under way share one preview document, so
   its effects are computed once per change, not per frame. Coarser levels and the GPU are
   still to come.
5. **Files**: `.slop` nodes gain `params.style` (a new node version); PSD import reads `lfx2`
   into the effects SlopShop draws (reporting the others), PSD export writes them back.
   *Done (2026-10-04)*: Drop Shadow, Outer Glow, Inner Shadow, Inner Glow (from the edge),
   Color Overlay and Stroke, the dialog, the panel, Fill, `.slop` (node v8, v9 for the glows
   and Inner Shadow). An effect is drawn as a fill of its color masked by its coverage (no
   color image): about 95 ms for a 12 MP layer. Then PSD import and export (`lfx2`/`lmfx`
   through a descriptor parser, Fill as `iOpa`; effects not drawn are reported) and styles on
   groups: a group's shape is what it holds, composited isolated, its effects drawn again when
   a layer inside changes (`.slop` schema 0.19).
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

## Amendment (2026-10-15): commands on styles

Status: accepted (2026-10-15, under the maintainer's delegation; to review).

- **The effects' master switch**: `LayerStyle::effects_visible` (Photoshop's eye on the
  "Effects" row, `masterFXSwitch` in PSD): hidden, no effect is drawn and each keeps its own
  `enabled`; Fill Opacity still applies. Layer > Layer Style > Hide All Effects and Show All
  Effects set it on every styled layer (those under Lock All left as they are), one undo entry;
  the menu offers Show once every layer's effects are hidden, as Photoshop's single command.
  `.slop` 0.36 writes `"effects_visible": false` only when hidden (an older reader shows the
  effects, losing nothing). PSD import keeps each effect's own setting under the switch instead
  of turning them all off.
- **Copy and Paste Layer Style**: the style (effects, Fill Opacity, the switch) is kept by the
  app, not by a document, so that it pastes into any open tab, a Pattern Overlay's source with
  it. Paste gives it to every selected layer but adjustment layers, one undo entry. Unlike
  Photoshop, the copied layer's opacity and blend mode are not pasted: they are the layer's,
  not its style's, in SlopShop's Blending Options.
- **Scale Effects** (1 to 1000 %): every size and distance of the selected styles scaled to
  whole pixels within its range (a Stroke at least 1 pixel), the Gradient and Pattern Overlays'
  scale with them; angles, spread and choke unchanged. Live, one undo entry.
- The fx mark's and the "Effects" row's right-click menu is Layer > Layer Style's.
- **Global Light**: `Document::global_light` (angle and altitude, Photoshop's 120° and 30° by
  default) and `use_global_light` on Drop Shadow, Inner Shadow and Bevel and Emboss (on for
  new effects, as in Photoshop). An effect using it keeps the light's angle (and a bevel its
  altitude) in its own fields, as PSD does: drawing does not depend on the document, and the
  engine keeps them equal. `Edit::SetGlobalLight` lights every such effect (whatever the
  layers' locks: a setting of the document), and a style set or a layer inserted (pasted from
  another document) takes the document's light. In the Layer Style dialog, moving the angle of
  an effect using it moves the light for all, as in Photoshop. `.slop` 0.37 writes
  `document.global_light` and `use_global_light`; PSD reads and writes resources 1037 and
  1049 and `uglg`. Alternative rejected: resolving the light while drawing, which would tie
  each style's cached effects to a document and break where layers move between documents
  (paint previews, scratch documents).

## Amendment (2026-10-19): contours

Status: accepted (2026-10-19, under the maintainer's delegation; to review).

- **A contour** (`slopshop_core::contour::Contour`) is a curve on 0–255 through 2 to 16 points,
  inputs strictly increasing, each point smooth or a corner (where the curve breaks); between
  corners, a natural cubic spline as the Curves adjustment's. A plain value (`Copy`), held by
  the effects that have one in Photoshop: Drop and Inner Shadow, Outer and Inner Glow (with
  their Range, 1 to 100 %), Satin, and Bevel and Emboss (its Gloss Contour, and its Contour
  element with its checkbox and Range). Each has its Anti-aliased: off, the mask's input is
  stepped by 1 %.
- **Where it applies**: an effect's mask (its fade, 0 far from the shape, 1 at it) goes through
  the contour (divided by the Range first) before it is colored; outward, a contour not
  starting at 0 fades out near 0 so that it does not fill the canvas. Satin's blurred shape
  goes through it before the difference; a bevel's height through its Contour, how much its
  surface faces the light through its Gloss Contour. The contour is part of the mask's cache
  key; a linear contour over the whole mask makes no pass.
- **Presets**: Photoshop's twelve (Linear, Cone, Cone - Inverted, Cove - Deep, Cove - Shallow,
  Gaussian, Half Round, Ring, Ring - Double, Rolling Slope - Descending, Rounded Steps,
  Sawtooth 1) are the engine's, which the UI lists (`contour_presets`, each curve sampled for
  its thumbnail); their points are drawn after Photoshop's shapes, not taken from it. A
  document from Photoshop brings its contours' own points: any curve of up to 16 points is
  kept as it is (shown as Custom when it is no preset) and written back; one SlopShop cannot
  hold is read as linear and reported as approximated.
- **Defaults**, Photoshop's: linear for all but Satin (Gaussian, anti-aliased); a glow's Range
  50 %; the bevel's Contour element off.
- `.slop` 0.39 writes each only when it differs from what its absence reads (linear, not
  anti-aliased, a glow's Range 100 %, the Contour element off: as effects were drawn before);
  no new node version, so an older reader draws linear contours.
- Not yet: a contour editor (Photoshop's Contour Editor, its own curves saved as presets).
