# 0040 — Sources: what layers show, kept once and referenced

Status: proposed (2026-10-07, from the maintainer's proposal: sources in a bin as in a video
editor, layers referencing them; answers so far: Duplicate shares the source, sources are
read-only and go away when nothing uses them, the bin is a panel, `.slop` may break while the
model is designed, painting on a vector layer goes on a pixel layer above it). Points marked
*to confirm* are the author's proposals, not yet answered.

## Context

Photoshop's Smart Objects answer three needs with one heavy concept: non-destructive
transforms and filters, several layers showing the same content (edit once, every copy
follows), and content that is not pixels (an embedded document, a linked file, vector art).
They forbid painting on them, which is why users rasterize them.

SlopShop already covers the first need on every layer: transforms are layer properties
(ADR 0017), filters and adjustments are editable entries of the layer's stack (ADR 0034),
and paint sits in that stack above the original, which stays intact (ADR 0029). The pixels
are immutable tiles shared by reference (ADR 0005), and `.slop` stores a tile once whatever
uses it (ADR 0009): duplicating a layer already costs no memory and no file size.

What is missing is identity. The shared pixels are anonymous: nothing says that two layers
show the same image, so nothing can change it for both, list it, replace it, or let the
content be something other than pixels. Text, shapes and paths (vector content, next ADR),
embedded documents, linked files, stack modes and panoramas, and later AI results, all need
a layer to show *something kept elsewhere*, by reference.

The model also has to choose between staying a tree of layers and becoming a graph (roadmap,
Phase 2: "Stack-to-DAG evolution"). References are where the graph comes from.

## Decision

1. **A document holds sources, and layers reference them.** A source is immutable content
   with a stable `SourceId` (as `LayerId`), a name and a kind:
   - **Image**: tiled pixels and their pyramid, as a raster layer's original is today.
   - Later, each in its own ADR: **vector** (text, shapes, paths), **document** (layers of
     their own, the embedded Smart Object), **linked file**, **stack** (several sources
     combined: median, mean… and panoramas), **AI result** (its parameters and inputs).
   A pixel layer becomes **a source reference + its transform + its own stack** (ADR 0029,
   0034): paint, adjustments, filters and Liquify stay per layer, as effects stay per clip in a
   video editor. A new empty layer has no source, only its stack (*to confirm*: every pixel
   layer is a reference, one kind of content rather than two).
2. **Sources are read-only.** Nothing writes into a source. Changing what a source shows
   makes a new source and points layers at it: every layer that referenced the old one
   (Replace Contents, a text corrected, a document edited), or only the active layer (Make
   Unique). One edit, `Edit::Repoint` (layers, old source, new source); its inverse points them
   back. Undo never re-creates content, it only moves references.
3. **Duplicate shares the source.** Duplicate Layer (and every command that duplicates layers)
   references the same source; the layer's stack is shared too until one of them
   changes it (copy on write, as today). **Make Unique** gives the active layer its own
   reference; with read-only sources it copies nothing until something changes.
4. **A source lives while something uses it**: a layer, the undo or redo history, the
   clipboard, another source. Deleting the last layer of a source keeps it while Ctrl+Z can
   bring it back; once nothing uses it, it is dropped. There is no "unused" state to purge,
   and a saved file holds only the sources its layers use (the history is not saved).
5. **No cycles, by construction.** A new source can only be built from sources that already
   exist (a document source referencing images, a stack source referencing its frames), and
   sources never change: the references form a graph without cycles, with no check needed at
   edit time. **The layer tree stays the user's model**; sources are the graph's shared nodes
   outside it. This settles the stack-or-DAG question for now: a tree of layers, plus
   references to read-only sources; no node editor.
6. **Rasterize and merges make a hard copy.** Layer > Bake to Pixels (ADR 0031) evaluates
   what the layer shows into a new image source, which the layer then references with an
   empty stack. The source it referenced before stays untouched for the other layers using
   it.
7. **The Sources panel shows the bin**: every source the document uses, with a thumbnail, its
   name, its kind and how many layers use it. From it: select the layers using a source,
   drag a source onto the canvas (a new layer referencing it), rename, Replace Contents.
   Sources nested in a document source show under it. A pixel layer with no source (a new
   empty layer) adds nothing to the panel.
8. **Painting needs a pixel layer.** A layer whose source has no pixel grid (vector, later)
   has no paint in its stack: the Brush and the other painting tools on such a layer ask
   "Paint on a new layer above?", which creates a pixel layer clipped to it (ADR 0016), so the
   paint stays within the shape and the vector stays sharp at any scale. Filters on such a
   layer: through a filter layer clipped to it once filter layers exist (ADR 0034, point 8),
   *to confirm*.
9. **Rendering and caches.** An image source is drawn as a raster original is today. A source
   whose content is computed (document, stack, vector) is evaluated into tiles keyed by
   (source, region, pyramid level), in a bounded cache as the stack's pixels (ADR 0029); every
   layer referencing it reads the same tiles. A source never changes, so its tiles never go
   stale: a new source has new keys.
10. **`.slop`**: the document gains a `sources` table (id, name, kind, content); raster nodes
    reference a source id instead of holding their original image. Breaking compatibility is
    accepted while the model is designed (maintainer, 2026-10-07). Before it is released, a
    reader of the earlier schema turns each layer's original into a source, so that files
    saved with v0.1 open (*to confirm*). PSD export writes each layer evaluated; PSD Smart
    Objects come with document sources.

## Alternatives

- **Photoshop's Smart Objects**: a layer kind of its own, transforms and filters only there,
  no painting on it. SlopShop already gives every layer what Smart Objects give, minus the
  sharing; copying the concept would add a second kind of layer and the rasterize step users
  avoid.
- **Editable sources** (writing into the shared content, every reference following): the
  edit's inverse must restore content instead of a reference, and editing a source from one
  layer silently changes others. Read-only sources with repointing do the same for the user,
  with an explicit choice (all, or Make Unique).
- **Duplicate as an independent copy** (Photoshop's text layers): simpler to explain, but
  "edit once, every copy follows" then needs a special command, and the memory is already
  shared anyway. Make Unique covers the independent copy.
- **A bin keeping unused sources until purged** (video editors): lets content wait in the
  bin, but grows files with what no layer shows and adds a purge command; declined by the
  maintainer.
- **A node graph exposed to the user**: the most general, but far from the layer stack users
  expect; references give the graph where it is needed without showing it.

## Consequences

- `LayerContent::Raster` holds a source reference instead of its original; `LayerStack`
  starts from the source's pixels. `Document` holds the sources, kept alive by reference
  counting (`Arc`), which already covers the history and the clipboard.
- New edits: `Repoint`, Make Unique; Replace Contents and the Sources panel in the app
  (commands, i18n in every catalog, component tests).
- Vector content (Type, Pen, Shapes, vector masks) is the next ADR, as a source kind; it
  relies on points 1, 2 and 8.
- Document sources give Edit Contents (the source opened in a tab, a new source on return),
  Convert to Source (layers into a document source, Photoshop's Convert to Smart Object) and
  PSD Smart Objects; linked files give Place Linked and "Update Modified Content" (a new
  source read from the file); stack sources give the stack modes and the panoramas; AI
  results give a node whose inputs are known, to mark it stale when they change (CLAUDE.md).
  Each in its own ADR when it comes.
- Order of work: the core model (sources, references, `Repoint`, lifetime) with tests; image
  sources replacing raster originals, `.slop` and the earlier schema's reader; Duplicate and
  Make Unique; the Sources panel; then vector content.
