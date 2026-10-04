# 0034 — Editable operations: the stack's entries, filters and filter layers

Status: accepted (2026-10-04, the maintainer's answers to the Filter menu audit).
Revises [ADR 0029](0029-layer-stack.md), points 3 to 5.

## Context

ADR 0029 keeps what Photoshop does destructively (paint, Image > Adjustments) as a stack inside
a raster layer, whose entries are deleted but never edited: editing one would recompute what is
above it, and paint above it was made looking at the old result. The Filter menu is next, and
Photoshop's answer for editable filters (convert to a Smart Object, then Smart Filters, with no
paint possible between them) is what SlopShop wants to avoid.

The maintainer wants operations kept with their parameters and edited again, in a layer's
stack (chronological: paint after an operation is not affected by it retroactively) and, for
filters, also as layers acting on what is below, as adjustment layers do (ADR 0020).

The engine already shares most of what this needs: `Adjustment` is both the adjustment layer's
content and the applied effect's (same parameters, math on the CPU and the GPU, `.slop`
encoding, Properties fields); paint is a delta `P + k·B` that follows what is below it; a stack
edit is one `SetLayerStack` whose inverse keeps the previous stack by reference, and a slider
drag is one gesture. What it lacks: any operation reading neighbouring pixels (the compositor
and the shader's stack evaluation are pointwise, in one pass per tile).

## Decision

1. **Entries are editable.** Editing replaces the entry by one with the new parameters: one
   `SetLayerStack`, a drag of a slider one undo entry. What is above it is evaluated again where
   the old and the new entry reach (widened by the margin of the spatial entries above it);
   what is below is not.
2. **Alike neighbours still merge.** An operation applied over an entry of its kind joins it
   (×n), as deleting what separated two of them does; each of its steps stays editable on its
   own (the dialog chooses the step). Two Inverts that meet cancel and the entry disappears.
   Editing never merges nor splits entries.
3. **Each entry has an eye**: a hidden entry is kept and skipped by the evaluation (saved).
4. **Targets.** Image > Adjustments (and Auto Tone, Contrast, Color) keep applying to every
   visible raster layer, one entry on top of each stack, each edited on its own layer (no
   link between layers). Filter > … applies to the active layer only: grayed when it is not a
   raster layer, when its mask is the target (for now) and in Quick Mask, which keeps its
   features to the minimum.
5. **The selection** an operation is applied with stays its implicit mask (ADR 0029); it can
   be loaded as the selection (Ctrl+click on the entry) and replaced by the current one (the
   entry's dialog). A new adjustment, fill or filter layer made while there is a selection gets
   a layer mask from it, as in Photoshop (not a new empty pixel layer).
6. **Tools that read what is below replay their gesture.** Paint (Brush, Eraser, Fill,
   Stroke, Delete, the Restore Eraser) is a delta and follows an edit below it by construction.
   Tools that take pixels from below (moved pixels today; Clone Stamp, Healing, Smudge, Mixer
   later) keep their result baked as in ADR 0029 and also record their gesture (moved pixels:
   the selection, the offset, copy or not; strokes: the path, pressure, settings, source); when
   an entry below them in the same stack changes, the gesture is computed again on the new
   result. A tool that sampled other layers (Sample All Layers) stays baked: other layers
   changing do not replay it. Accepted: editing an old entry changes the render of what was
   painted above it.
7. **Operations, one model, two evaluations.** Adjustments and filters are one kind of value
   (kind, parameters, validation, `.slop` encoding, dialog, Properties fields, Repeat), each
   describing its reach: pointwise, a neighbourhood of a radius, or the whole image (and
   whether it depends on a frame: a center, a grid). Which places accept it derives from that
   description, not from flags. Pointwise operations evaluate as today (one pass, in the shader
   for display). A spatial entry is a materialization point: its result is a cache (bounded,
   never saved, as the stack's pixels), the display evaluates the entries above from it, and a
   change below re-evaluates it over the tiles reached widened by its margin. CPU reference and
   GPU agree, tested against each other. Parameters are in the layer's own pixels: the stack is
   before the layer's transform, which stays a layer property (ADR 0017), not an entry.
8. **Filter layers.** The user sees two concepts, Adjustment Layer and Filter Layer (Layer > New
   Filter Layer, at the top level beside New Adjustment Layer, a flat list by category); the
   engine has one content for both. A filter layer takes the accumulator at its place, as an
   adjustment layer (pass-through: everything below; isolated group: the group's content;
   clipped: its clipping group), and gives `mix(below, mode(below, f(below)), opacity × mask)`:
   its opacity is the effect's strength, alpha may change (a blur spreads transparency), the
   canvas edge is repeated, hidden means absent. It needs a multi-pass compositor (an ADR of its
   own) and comes after filters in the stack.
9. **Filter menu.** No dead entries: Repeat Last Filter (Ctrl+F: a new entry with the same
   parameters on the active layer, within the current selection, in the same place as last time;
   Alt+Ctrl+F reopens its dialog), then Blur > Gaussian Blur first, then Blur, Sharpen, Noise,
   Distort, Pixelate, Stylize as each works. Not reproduced: Convert for Smart Filters, Filter
   Gallery, Camera Raw Filter (its tools as native operations), Neural Filters (AI features go
   where their intention is), render generators (fill layers instead).
10. **Liquify** is a stack entry holding a displacement field, edited again in its own
    workspace; later, with no menu entry before it works. Not a filter layer.
11. **`.slop`**: effect steps gain the filter kinds and their parameters, entries an `enabled`
    flag, replayable entries their gesture; filter layers a node type, `slopshop.filter`. A
    new schema; files with stacks of earlier schemas read unchanged.

## Alternatives

- **Entries not editable (ADR 0029 as is)**: no recomputation above an entry, but a filter or
  an adjustment cannot be changed without deleting what was applied since.
- **Smart Objects and Smart Filters**: editable, but no paint between filters, and a conversion
  forced before filtering.
- **Baking the tools that read pixels, marked stale when something below changes**: cheaper,
  but the maintainer prefers the result to follow (replay).
- **Linking the entries one Image > Adjustments made on several layers**: one edit for all, but
  the stack is a layer's own; declined.
- **One "Effect Layer" concept for the user**: fewer menus, but "Adjustment Layer" is what
  Photoshop users know, and "Effects" already names layer styles.
- **The transform as a chronological entry**: paint after it would land in document space, but
  every transform would resample; it stays a lossless layer property.

## Consequences

- Editing below a long stack costs evaluation time (replays included), not frame time; a
  spatial entry costs memory for its cached result.
- 8-bit layers are rounded after each entry: stacks edited at will make converting a layer to
  a deeper format more pressing.
- `Footprint` gains a margin; `stack::Effect` grows into the shared operation type.
- Order of work: entries editable, their eye and the dialog's step choice (adjustments, the
  engine and the app); moved pixels replayed; Gaussian Blur in the stack with Repeat and the
  Filter menu; the multi-pass compositor's ADR, then filter layers; the other filters; Liquify.
