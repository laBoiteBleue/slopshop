# 0016 — Clipping masks

Status: accepted (2026-09-30).

## Context

A clipping mask shows layers only where the layer below them has pixels: texture inside text,
a color on a shape, a shadow limited to a figure. Photoshop documents use it everywhere, and PSD
import reported it as ignored. Groups (ADR 0015) gave the compositor the stack it needs.

## Decision

1. **A flag on layers**: `Layer::clipped`. A clipped layer is clipped to the nearest layer
   below it, among its siblings, that is not clipped: the *base*. The base and the clipped
   layers above it form a *clipping group*. A clipped layer without a base (at the bottom of its
   level) is drawn as usual. Any layer may be a base or clipped, groups included.
2. **Semantics, as Photoshop with "Blend Clipped Layers as Group"** (its default): the base is
   drawn alone at full opacity in normal mode (its mask applied), each clipped layer is blended
   *atop* it (with its own mode, opacity and mask; the result keeps the base's coverage: W3C
   source-atop with blending), and the clipping group's result is then blended like one layer
   with the base's blend mode and opacity. A hidden base hides its clipping group. A group as a
   base or as a clipped layer is composited as a unit (a pass-through one is isolated there).
3. **Rendering**: `composite::steps` resolves this into the existing program (ADR 0015); steps
   now carry the mode and opacity to apply, and an `atop` flag, instead of reading them from the
   layer. Both compositors apply atop blending with the same formula.
4. **Edit** `SetLayerClipped`. **`.slop`**: a node with `params.clipped` is written at version 4
   (schema 0.5), so that versions that do not know clipping refuse it instead of drawing it
   unclipped; other nodes stay at version 3.

## Alternatives

- **Clipping as a mask made from the base's alpha**: a copy of pixels, not live, and wrong for
  blend modes (Photoshop blends the clipped layers against the base, not against what is below).
- **Clipping groups as a model structure** (a node holding its base and clipped layers): cleaner
  in a DAG, but every Photoshop user thinks of it as a per-layer flag (Alt+click between layers),
  and PSD stores it so; the flag maps directly and the compositor derives the structure.

## Consequences

- PSD clipping imports as is; its warning goes away.
- The layers panel shows clipped layers indented with an arrow, the base's name underlined, and
  Layer > Create/Release Clipping Mask (Alt+Ctrl+G) toggles the flag.
