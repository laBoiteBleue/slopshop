// The Filter menu's filters (ADR 0034): their settings, which layer they apply to, and the
// edits that apply them and edit them again. Filters are entries of the active layer's stack.

import type { EditRequest, FilterId, LayerView, StackEntryView } from "./engine";
import type { MessageKey } from "./i18n/en";

/** A filter's settings as edits send them (`Filter::params` order in the engine). */
export type FilterSettings = { filter: FilterId; values: number[] };

/** One number setting of a filter's dialog. */
export type FilterParam = {
  key: string;
  label: MessageKey;
  /** What its value counts, after the field. */
  unit: MessageKey;
  min: number;
  max: number;
  /** Decimals the field keeps. */
  decimals: number;
  /** How its slider moves: evenly, or by ratios (a radius from 0.1 to 1000 pixels). */
  scale: "linear" | "log";
};

/** A radius in pixels, Photoshop's: a Gaussian's standard deviation, 0.1 to 1000. */
const radius = (label: MessageKey): FilterParam => ({
  key: "radius",
  label,
  unit: "filter.pixels",
  min: 0.1,
  max: 1000,
  decimals: 1,
  scale: "log",
});

/** Each filter's settings (the engine's `Filter::params` order), Photoshop's ranges and defaults. */
export const FILTERS: Record<FilterId, { params: FilterParam[]; defaults: number[] }> = {
  gaussianBlur: { params: [radius("filter.gaussianBlur.radius")], defaults: [1] },
  unsharpMask: {
    params: [
      {
        key: "amount",
        label: "filter.unsharpMask.amount",
        unit: "filter.percent",
        min: 1,
        max: 500,
        decimals: 0,
        scale: "linear",
      },
      radius("filter.unsharpMask.radius"),
      {
        key: "threshold",
        label: "filter.unsharpMask.threshold",
        unit: "filter.levels",
        min: 0,
        max: 255,
        decimals: 0,
        scale: "linear",
      },
    ],
    defaults: [100, 1, 0],
  },
  highPass: { params: [radius("filter.highPass.radius")], defaults: [10] },
};

/** A slider position (0–1000) for value `v` of `param`. */
export function sliderPosition(param: FilterParam, v: number): number {
  const { min, max } = param;
  const clamped = Math.min(Math.max(v, min), max);
  return param.scale === "log"
    ? (Math.log(clamped / min) / Math.log(max / min)) * 1000
    : ((clamped - min) / (max - min)) * 1000;
}

/** The value of `param` at slider position `p` (0–1000), rounded to its decimals. */
export function sliderValue(param: FilterParam, p: number): number {
  const { min, max } = param;
  const v =
    param.scale === "log" ? min * Math.pow(max / min, p / 1000) : min + ((max - min) * p) / 1000;
  return Number(v.toFixed(param.decimals));
}

/** The Filter menu's submenus, Photoshop's, and the filters in each. */
export const FILTER_MENU: { label: MessageKey; filters: FilterId[] }[] = [
  { label: "menu.filter.blur", filters: ["gaussianBlur"] },
  { label: "menu.filter.sharpen", filters: ["unsharpMask"] },
  { label: "menu.filter.other", filters: ["highPass"] },
];

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
