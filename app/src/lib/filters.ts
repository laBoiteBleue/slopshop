// The Filter menu's filters (ADR 0034): their settings, which layer they apply to, and the
// edits that apply them and edit them again. Filters are entries of the active layer's stack.

import type { EditRequest, FilterId, LayerView, StackEntryView } from "./engine";

/** A filter's settings as edits send them (`Filter::params` order in the engine). */
export type FilterSettings = { filter: FilterId; values: number[] };

/** One number setting of a filter's dialog. */
export type FilterParam = {
  /** Its label is `filter.<id>.<key>`. */
  key: "radius";
  min: number;
  max: number;
  /** Decimals the field keeps. */
  decimals: number;
};

/** Each filter's settings, and their values the first time its dialog opens. */
export const FILTERS: Record<FilterId, { params: FilterParam[]; defaults: number[] }> = {
  // Photoshop's: the radius is the standard deviation, 0.1 to 1000 pixels.
  gaussianBlur: { params: [{ key: "radius", min: 0.1, max: 1000, decimals: 1 }], defaults: [1] },
};

/** Whether `values` are settings `filter` accepts. */
export function validValues(filter: FilterId, values: number[]): boolean {
  const params = FILTERS[filter].params;
  return (
    values.length === params.length &&
    values.every((v, i) => Number.isFinite(v) && v >= params[i].min && v <= params[i].max)
  );
}

/**
 * Whether Filter > … applies: to the active layer when it is a pixel layer shown, its pixels
 * the target (not its mask), outside Quick Mask (the maintainer keeps it to the minimum).
 */
export function filterable(
  layer: LayerView | null,
  paintsMask: boolean,
  quickMask: boolean,
): layer is LayerView {
  return layer !== null && layer.kind === "raster" && layer.visible && !paintsMask && !quickMask;
}

/** Filter > …: `settings` applied to layer `id`, an entry on top of its stack. */
export function applyFilterEdit(id: number, settings: FilterSettings): EditRequest {
  return { kind: "applyFilter", id, filter: settings.filter, values: settings.values };
}

/** The settings of each step of a filter entry, bottom to top. */
export function filterSteps(entry: StackEntryView): FilterSettings[] {
  return entry.filterSteps.map((step) => ({ filter: step.id, values: [...step.values] }));
}

/** The edit that sets filter entry `index` of layer `id`'s stack: its steps, its eye. */
export function filterEntryEdit(
  id: number,
  index: number,
  settings: FilterSettings[],
  hidden: boolean,
): EditRequest {
  return { kind: "setStackEntry", id, index, hidden, filters: settings };
}
