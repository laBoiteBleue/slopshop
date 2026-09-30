# 0015 — Layer groups

Status: accepted (2026-09-30).

## Context

Photoshop users organize documents in groups ("folders"): to fold, move, hide or fade many layers
at once, and to blend a set of layers as one. PSD files are full of them, and a `.slop` or PSD
imported into a document should arrive as one. The document model must stay able to become a DAG
of operations (ADR 0003), and rendering must stay a single pass over the layers without
intermediate buffers (performance, VRAM).

## Decision

1. **A tree.** A group is a layer whose content is its children:
   `LayerContent::Group { children, pass_through }`, children bottom to top. Groups nest (at most
   `MAX_GROUP_DEPTH` = 16 levels, Photoshop allows 10). Every layer of the tree keeps a stable
   id, unique across the whole tree. `Document::layers()` is the top level;
   `Document::all_layers()` walks the tree. A group is the first node with inputs: the DAG
   generalizes it (a node referencing its inputs by id).
2. **Semantics, as in Photoshop.** A hidden group hides its subtree.
   - *Pass-through* (the default for new groups): the children blend directly onto what is below;
     the group's opacity and mask then fade that result against what was below:
     `lerp(below, children over below, opacity × mask)`. At full opacity without a mask the group
     changes nothing to the result.
   - *Isolated* (any blend mode): the children are composited on transparency, then that result is
     blended like one layer with the group's mode, opacity and mask.

   Pass-through is a group option, not a 28th blend mode: blend functions never see it, and a
   group keeps its blend mode while it passes through.
3. **Edits.** `InsertLayer` and `MoveLayer` take a `parent` (`None`: the top level) and an index
   within it; `RemoveLayer` removes a subtree and its inverse restores it whole;
   `SetGroupPassThrough`. Moving a group into itself or one of its descendants, too deep a
   nesting, or a parent that is not a group are errors. Grouping and ungrouping are batches of
   these edits (one undo entry).
4. **Rendering: one program for both compositors.** `composite::steps` turns the visible tree into
   a flat list of steps (layer, group begin, group end). The CPU reference and the GPU shader run
   it with a stack of accumulators (16 on the GPU, in registers): no intermediate texture, still
   one pass. A pass-through group at full opacity without an enabled mask is left out of the
   program (its children inline): exactly the same result, at no cost. Empty and hidden groups
   produce nothing.
5. **`.slop` schema 0.4**: a `slopshop.group` node whose `inputs` are its children (bottom to
   top), with `blend_mode`, `pass_through` and `mask` parameters. A compatible addition: older
   versions refuse such files as using features they do not support.

## Alternatives

- **A flat list with parent ids** (or PSD-like begin/end markers): keeps simple loops over
  layers, but every edit must maintain the invariant that a group's children follow it; a tree
  makes invalid structures unrepresentable.
- **An offscreen texture per isolated group**: the classic approach, needed later by nodes that
  read neighboring pixels (a blur of a group). Costs VRAM and passes per group; the stack is exact
  for everything a group does today.
- **Pass-through as a blend mode**: one field fewer, but every blend function, table and shader
  constant would have to know a mode that only groups may use.

## Consequences

- Every traversal of layers decides whether it wants the top level or the whole tree; tools that
  collect images (rendering plans, saving, thumbnails) walk the tree.
- Clipping masks and PSD groups become possible on the same structure; the layers panel shows
  groups as folders (a separate change).
- A future node needing its inputs as a texture (filters) will add intermediate buffers for that
  node only.
