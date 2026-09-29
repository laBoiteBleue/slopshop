# 0003 — Document model, edits and history

Status: accepted (the stack-to-DAG evolution remains open)

## Context

Every document change must be undoable, the user sees a layer stack, and the internal model
must be able to become a DAG of operations (AI nodes with dependencies, shared inputs, etc.).

## Decision

- The `Document` is read-only from outside `slopshop-core`. The only mutation path is
  `Edit::apply`, which validates first and returns the **exact inverse edit**.
- History (`Session`) stores inverses: undo applies the top inverse and pushes *its* inverse on
  the redo stack. Linear history; a new edit clears redo.
- Layers are identified by **stable, never-reused `LayerId`s**. Positions (indices) are only
  used to describe placement in the stack, never identity.
- `Edit::Batch` groups edits atomically (all or nothing). Continuous interactions are
  *gestures*: edits applied live through `Session::perform_in_gesture`, recorded as a single
  history entry by `end_gesture` (or implicitly before any other edit/undo/redo).
- `Document::revision` increments on every applied edit (including undo/redo) so caches and
  views can detect change cheaply.

## Alternatives

- **Snapshots** (store full document states): simple but memory-hungry with pixel data, unless
  everything is persistent/COW — which we will likely need anyway for tiles. Inverse edits keep
  history small and explicit; snapshots may complement them for large pixel edits.
- **Command objects with `do/undo` methods (trait objects)**: more boilerplate and less
  inspectable than a data enum.
- **CRDT/operation log**: relevant for collaboration, premature now.

## Consequences

- Each new edit variant must provide an exact inverse and tests (`apply` then inverse restores
  the state).
- Pixel edits (brush strokes) will need inverses that reference tile data (old tiles kept by
  reference), not copies of whole layers.
- History memory limits are future work.
- Moving to a DAG means `LayerContent` becomes node references; ids already make that possible.
