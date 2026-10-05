// The selection of the Layers panel: UI state, not document state (it is not undoable).
// Several layers can be selected, as in Photoshop: a click selects one, Ctrl+click adds or
// removes one, Shift+click selects a range from the anchor. The active layer (always a selected
// one, when any is) is the one rename, the Layer menu and the options show; actions apply to
// every selected layer. Each rule returns the next selection, or the same object when nothing
// changes.

export type LayerSelection = {
  /** Selected layers, in the order they were selected. */
  ids: number[];
  active: number | null;
  /** Where a Shift+click range starts. */
  anchor: number | null;
};

export const NO_LAYERS: LayerSelection = { ids: [], active: null, anchor: null };

/** `ids` selected, `active` the active layer and the anchor. */
export function selectionOf(ids: number[], active: number | null): LayerSelection {
  return { ids, active, anchor: active };
}

/** The topmost of `ids` in `order` (every layer, bottom to top). */
function topmostIn(order: number[], ids: number[]): number | null {
  const set = new Set(ids);
  return order.findLast((id) => set.has(id)) ?? null;
}

/**
 * The selection once the document's layers are `order` (every layer, bottom to top) and were
 * `known` before (empty for a new panel). New layers become the selection (several for a
 * layered import); deleted ones leave it, and when none is left, the top layer is selected (a
 * deliberate "Deselect Layers" keeps the selection empty). A new panel selects the top layer.
 */
export function afterLayersChange(
  selection: LayerSelection,
  known: ReadonlySet<number>,
  order: number[],
): LayerSelection {
  const first = known.size === 0;
  const created = order.filter((id) => !known.has(id));
  if (created.length > 0 && !first) return selectionOf(created, created[created.length - 1]);
  const present = new Set(order);
  const kept = selection.ids.filter((id) => present.has(id));
  if (first || (kept.length === 0 && selection.ids.length > 0)) {
    const top = order.at(-1) ?? null;
    return selectionOf(top === null ? [] : [top], top);
  }
  if (kept.length === selection.ids.length) return selection;
  const active =
    selection.active !== null && present.has(selection.active)
      ? selection.active
      : topmostIn(order, kept);
  const anchor =
    selection.anchor !== null && present.has(selection.anchor) ? selection.anchor : active;
  return { ids: kept, active, anchor };
}

/** Ctrl+click: `id` joins the selection and becomes active, or leaves it. */
export function toggled(selection: LayerSelection, id: number, order: number[]): LayerSelection {
  if (!selection.ids.includes(id)) return selectionOf([...selection.ids, id], id);
  const ids = selection.ids.filter((s) => s !== id);
  const active = selection.active === id ? topmostIn(order, ids) : selection.active;
  return selectionOf(ids, active);
}

/**
 * Shift+click: the rows from the anchor to `id` (inclusive), in `displayed` order (the rows,
 * top to bottom); the anchor stays.
 */
export function ranged(selection: LayerSelection, id: number, displayed: number[]): LayerSelection {
  const from = displayed.indexOf(selection.anchor ?? id);
  const to = displayed.indexOf(id);
  if (from < 0 || to < 0) return selectionOf([id], id);
  const ids = displayed.slice(Math.min(from, to), Math.max(from, to) + 1);
  return { ids, active: id, anchor: selection.anchor };
}

/** Select > All Layers: every layer, the active one kept (else the top one). */
export function allSelected(selection: LayerSelection, order: number[]): LayerSelection {
  return selectionOf([...order], selection.active ?? order.at(-1) ?? null);
}

/**
 * A press on a layer: alone, it becomes the selection. Within a selection of several, it
 * becomes the active layer and the selection stays, so that the press can drag them all; a
 * release without a drag then selects it alone (`collapse`).
 */
export function pressed(
  selection: LayerSelection,
  id: number,
): { selection: LayerSelection; collapse: boolean } {
  if (selection.ids.length > 1 && selection.ids.includes(id)) {
    return { selection: { ids: selection.ids, active: id, anchor: id }, collapse: true };
  }
  return { selection: selectionOf([id], id), collapse: false };
}

/**
 * A press on the image with the Move tool picking layer `hit` (null: no layer shows there), the
 * Layers panel's selection and rules: alone, as a press on its row (`pressed`); with Shift
 * (`add`), it joins the selection or leaves it, as Ctrl+click on a row (Ctrl stays Photoshop's
 * Auto-Select switch on the image). `moves`: whether a drag that follows moves the selection
 * (not after taking a layer out of it).
 */
export function pickedInImage(
  selection: LayerSelection,
  hit: number | null,
  add: boolean,
  order: number[],
): { selection: LayerSelection; collapse: boolean; moves: boolean } {
  if (hit === null) return { selection, collapse: false, moves: !add };
  if (add) {
    const next = toggled(selection, hit, order);
    return { selection: next, collapse: false, moves: next.ids.includes(hit) };
  }
  return { ...pressed(selection, hit), moves: true };
}
