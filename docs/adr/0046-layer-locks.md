# 0046 — Layer locks

Status: accepted (2026-10-13, under the maintainer's delegation; to review).

## Context

Photoshop's Layers panel has a "Lock:" row: Lock Transparent Pixels, Lock Image Pixels, Lock
Position and Lock All (and Prevent Auto-Nesting, for artboards), Layer > Lock Layers (Ctrl+/)
sets them on several layers at once, `/` toggles Lock Transparent Pixels, and a locked row shows
a padlock (full for Lock All, hollow for the others). A group's locks apply to its layers. Users
rely on them to protect a layer from a stray stroke or move, and Lock Transparent Pixels is how
Photoshop recolors a shape without a clipping mask. PSD files carry them (`lspf`).

The rule of the project is that the engine enforces the document's rules, not the UI
([ADR 0003](0003-document-model-edits-history.md)): every mutation is an `Edit`.

## Decision

1. **Locks are layer state**: `Layer::locks` holds four flags (transparency, pixels, position,
   all), set by a reversible `Edit::SetLayerLocks` (one undo entry, "Layer Locks"). A group's
   locks apply to every layer inside it (`Document::effective_locks`). Duplicates keep them.
2. **Edits refuse what a lock forbids** (`EditError::Locked { id, lock }`), checked as each edit
   applies, so that a batch refused by a lock changes nothing:
   - Lock Image Pixels (or All): the layer's stack (paint, fills, filters, adjustments applied
     to its pixels, Liquify, moved pixels, Delete Paint), a vector layer's shape, a fill layer's
     color, gradient or pattern.
   - Lock Position (or All): the layer's transform when it changes, and that of a group holding
     a position-locked layer (it would carry it along). A transform set to what it already is
     (a stroke growing a layer) is no move.
   - Lock All: also its opacity, blend mode, style, mask (added, removed, enabled, painted),
     clipping, a group's pass-through and an adjustment's settings. As in Photoshop, a fully
     locked layer can still be shown or hidden, renamed, moved in the stack, duplicated and
     deleted; locks never stop these.
   - Lock Image Pixels leaves the layer's mask editable (the mask is not the layer's pixels;
     Lock All locks it).
3. **Whole-image changes move locked layers with the image**, as Photoshop does: Image Size,
   Canvas Size, Crop, Trim, Reveal All and Image Rotation are wrapped in `Edit::Image`, which
   applies its edits without checking locks (its inverse too).
4. **Lock Transparent Pixels makes painting keep the layer's alpha.** A stroke on such a layer
   lays its color premultiplied by the alpha the layer showed when the stroke started, its
   keep factor as usual: the paint stays an exact `P + k·B` entry of the stack
   ([ADR 0029](0029-layer-stack.md)), evaluated by the CPU and the GPU as any other, and the
   result keeps the alpha it had. This holds for the Brush, Pencil, Clone Stamp and the tools
   built on it, Edit > Fill, Edit > Stroke and the Gradient tool. The Eraser and the Restore
   Eraser, which would lower alpha, are refused (Photoshop's Eraser paints the background color
   there: not done). Such a layer does not grow when painted beyond its bounds (nothing to
   paint there), nor does a position-locked one (growing moves its origin).
5. **The UI** shows the four buttons above the layers (set on every selected layer, or cleared
   when all have it; "mixed" when some have it), a padlock on locked rows (full or partial,
   dimmed when it comes from a group), grays the blend mode and opacity under Lock All, offers
   Layer > Lock Layers (Ctrl+/, Photoshop's dialog) and `/`. Tools are refused at their first
   dab by the engine; its message ("… is locked: pixels") is recognized by the UI and said in
   the interface's language.
6. **Files**: `.slop` schema 0.33 writes `params.locks`, the identifiers of the locks set, on
   any node; a reader that does not know them opens the layer unlocked (nothing it shows is
   lost), and unknown identifiers are ignored. PSD import and export map them to `lspf` (bit 0
   transparency, 1 image pixels, 2 position, 31 all).

## Alternatives

- **Locks enforced by the UI only**: simpler, but the CLI, scripts and any future entry point
  would bypass them, and every tool would need its own check.
- **Lock Transparent Pixels as a selection** (the layer's alpha limiting the stroke): changes
  semi-transparent pixels' alpha, which is not what the lock promises.
- **A "keeps alpha" flag on paint entries**, evaluated as `P·α + k·B`: exact even when what is
  below the paint changes later, but a new evaluation path on the CPU and the GPU and a file
  format change for little gain; Photoshop's own lock is destructive anyway.

## Consequences

- Every new edit that changes a layer must be placed in `Edit::check_locks`; a new whole-image
  operation must go through `Edit::Image`.
- History stays consistent: locks change only through edits, so undoing an edit always finds
  the locks it was made under.
- Not done yet: the Eraser painting the background color under Lock Transparent Pixels, Lock
  All Layers in Group, Prevent Auto-Nesting (no artboards).
