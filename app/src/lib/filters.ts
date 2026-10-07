// The Filter menu's filters (ADR 0034): their settings, which layer they apply to, and the
// edits that apply them and edit them again. Filters are entries of the active layer's stack.

import type { EditRequest, FilterId, LayerView, StackEntryView } from "./engine";
import type { MessageKey } from "./i18n/en";

/** A filter's settings as edits send them (`Filter::params` order in the engine). */
export type FilterSettings = { filter: FilterId; values: number[] };

/** A number setting of a filter's dialog: a field, its unit, a slider below. */
export type NumberParam = {
  kind: "number";
  key: string;
  label: MessageKey;
  /** What its value counts, after the field (nothing for a strength from -100 to 100). */
  unit?: MessageKey;
  min: number;
  max: number;
  /** Decimals the field keeps. */
  decimals: number;
  /** How its slider moves: evenly, or by ratios (a radius from 0.1 to 1000 pixels). */
  scale: "linear" | "log";
};

/**
 * One setting of a filter's dialog: a number; one of `options` (their index); a check box (0 or
 * 1); or a seed, not shown, drawn anew each time the filter is applied (`withNewSeeds`).
 */
export type FilterParam =
  | NumberParam
  | { kind: "choice"; key: string; label: MessageKey; options: MessageKey[] }
  | { kind: "check"; key: string; label: MessageKey }
  | { kind: "seed"; key: string };

/** The seeds a filter takes: whole numbers below 2^24 (the engine's `NOISE_SEEDS`). */
export const SEEDS = 2 ** 24;

/** A radius in pixels, Photoshop's: a Gaussian's standard deviation, 0.1 to 1000. */
const radius = (label: MessageKey): NumberParam => ({
  kind: "number",
  key: "radius",
  label,
  unit: "filter.pixels",
  min: 0.1,
  max: 1000,
  decimals: 1,
  scale: "log",
});

/** A radius in whole pixels from 1 to `max` (a square of pixels around each), Photoshop's. */
const squareRadius = (label: MessageKey, max: number): NumberParam => ({
  kind: "number",
  key: "radius",
  label,
  unit: "filter.pixels",
  min: 1,
  max,
  decimals: 0,
  scale: "log",
});

/** A strength from -100 to 100, Lightroom's: 0 does nothing, below it the opposite. */
const strength = (key: string, label: MessageKey): NumberParam => ({
  kind: "number",
  key,
  label,
  min: -100,
  max: 100,
  decimals: 0,
  scale: "linear",
});

/** Each filter's settings (the engine's `Filter::params` order), Photoshop's ranges and defaults. */
export const FILTERS: Record<FilterId, { params: FilterParam[]; defaults: number[] }> = {
  gaussianBlur: { params: [radius("filter.gaussianBlur.radius")], defaults: [1] },
  motionBlur: {
    params: [
      {
        kind: "number",
        key: "angle",
        label: "filter.motionBlur.angle",
        unit: "filter.degrees",
        min: -90,
        max: 90,
        decimals: 0,
        scale: "linear",
      },
      {
        kind: "number",
        key: "distance",
        label: "filter.motionBlur.distance",
        unit: "filter.pixels",
        min: 1,
        max: 2000,
        decimals: 0,
        scale: "log",
      },
    ],
    defaults: [0, 10],
  },
  unsharpMask: {
    params: [
      {
        kind: "number",
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
        kind: "number",
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
  addNoise: {
    params: [
      {
        kind: "number",
        key: "amount",
        label: "filter.addNoise.amount",
        unit: "filter.percent",
        min: 0.1,
        max: 400,
        decimals: 1,
        scale: "log",
      },
      {
        kind: "choice",
        key: "distribution",
        label: "filter.addNoise.distribution",
        options: ["filter.addNoise.uniform", "filter.addNoise.gaussian"],
      },
      { kind: "check", key: "monochromatic", label: "filter.addNoise.monochromatic" },
      { kind: "seed", key: "seed" },
    ],
    defaults: [12.5, 0, 0, 0],
  },
  dustAndScratches: {
    params: [
      {
        kind: "number",
        key: "radius",
        label: "filter.dustAndScratches.radius",
        unit: "filter.pixels",
        min: 1,
        max: 500,
        decimals: 0,
        scale: "log",
      },
      {
        kind: "number",
        key: "threshold",
        label: "filter.dustAndScratches.threshold",
        unit: "filter.levels",
        min: 0,
        max: 255,
        decimals: 0,
        scale: "linear",
      },
    ],
    defaults: [1, 0],
  },
  clarityTexture: {
    params: [
      strength("texture", "filter.clarityTexture.texture"),
      strength("clarity", "filter.clarityTexture.clarity"),
    ],
    defaults: [0, 0],
  },
  highPass: { params: [radius("filter.highPass.radius")], defaults: [10] },
  boxBlur: { params: [squareRadius("filter.boxBlur.radius", 2000)], defaults: [10] },
  median: { params: [squareRadius("filter.median.radius", 500)], defaults: [1] },
  maximum: { params: [squareRadius("filter.maximum.radius", 500)], defaults: [1] },
  minimum: { params: [squareRadius("filter.minimum.radius", 500)], defaults: [1] },
};

/** `values` of `filter` with each seed drawn anew: the filter applied again, another grain. */
export function withNewSeeds(
  filter: FilterId,
  values: number[],
  random: () => number = Math.random,
): number[] {
  return values.map((v, i) =>
    FILTERS[filter].params[i]?.kind === "seed" ? Math.floor(random() * SEEDS) : v,
  );
}

/** A slider position (0–1000) for value `v` of `param`. */
export function sliderPosition(param: NumberParam, v: number): number {
  const { min, max } = param;
  const clamped = Math.min(Math.max(v, min), max);
  return param.scale === "log"
    ? (Math.log(clamped / min) / Math.log(max / min)) * 1000
    : ((clamped - min) / (max - min)) * 1000;
}

/** The value of `param` at slider position `p` (0–1000), rounded to its decimals. */
export function sliderValue(param: NumberParam, p: number): number {
  const { min, max } = param;
  const v =
    param.scale === "log" ? min * Math.pow(max / min, p / 1000) : min + ((max - min) * p) / 1000;
  return Number(v.toFixed(param.decimals));
}

/** The Filter menu's submenus, Photoshop's, and the filters in each. */
export const FILTER_MENU: { label: MessageKey; filters: FilterId[] }[] = [
  { label: "menu.filter.blur", filters: ["boxBlur", "gaussianBlur", "motionBlur"] },
  { label: "menu.filter.noise", filters: ["addNoise", "dustAndScratches", "median"] },
  { label: "menu.filter.sharpen", filters: ["unsharpMask", "clarityTexture"] },
  { label: "menu.filter.other", filters: ["highPass", "maximum", "minimum"] },
];

/** Whether `values` are settings `filter` accepts. */
export function validValues(filter: FilterId, values: number[]): boolean {
  const params = FILTERS[filter].params;
  return values.length === params.length && values.every((v, i) => validValue(params[i], v));
}

function validValue(param: FilterParam, v: number): boolean {
  switch (param.kind) {
    case "number":
      // Without decimals, a whole number (Dust & Scratches' radius, a square of pixels).
      return (
        Number.isFinite(v) &&
        v >= param.min &&
        v <= param.max &&
        (param.decimals > 0 || Number.isInteger(v))
      );
    case "choice":
      return Number.isInteger(v) && v >= 0 && v < param.options.length;
    case "check":
      return v === 0 || v === 1;
    case "seed":
      return Number.isInteger(v) && v >= 0 && v < SEEDS;
  }
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
