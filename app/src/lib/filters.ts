// The Filter menu's filters (ADR 0034): their settings, which layer they apply to, and the
// edits that apply them and edit them again. Filters are entries of the active layer's stack.
// Image > Adjustments > Shadows/Highlights is one too (it reads neighbours), on every pixel
// layer shown as the other adjustments are.

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
  /** The heading it is under, Photoshop's (Shadows/Highlights' Shadows, Highlights…). */
  group?: MessageKey;
  /** Shown only with Show More Options (for a filter that has it, `FilterSpec.more`). */
  more?: boolean;
};

/**
 * One setting of a filter's dialog: a number; one of `options` (their index); a check box (0 or
 * 1); a seed, not shown, drawn anew each time the filter is applied (`withNewSeeds`); or a
 * coordinate of a center in percent of the frame (0 to 100), picked in a square: the `x` one
 * shows the square for itself and the `y` one that follows it.
 */
export type FilterParam =
  | NumberParam
  | { kind: "choice"; key: string; label: MessageKey; options: MessageKey[] }
  | { kind: "check"; key: string; label: MessageKey }
  | { kind: "seed"; key: string }
  | { kind: "center"; key: string; label: MessageKey; axis: "x" | "y" };

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

/** A radius in whole pixels from `min` to `max` (a square of pixels around each), Photoshop's. */
const squareRadius = (label: MessageKey, max: number, min = 1): NumberParam => ({
  kind: "number",
  key: "radius",
  label,
  unit: "filter.pixels",
  min,
  max,
  decimals: 0,
  scale: "log",
});

/** An amount from -100 to 100 %, Photoshop's (Pinch, Spherize): below 0, the other way. */
const percent = (label: MessageKey): NumberParam => ({
  kind: "number",
  key: "amount",
  label,
  unit: "filter.percent",
  min: -100,
  max: 100,
  decimals: 0,
  scale: "linear",
});

/** A shift in whole pixels either way, Offset's (Photoshop's -30000 to 30000). */
const shift = (key: string, label: MessageKey): NumberParam => ({
  kind: "number",
  key,
  label,
  unit: "filter.pixels",
  min: -30000,
  max: 30000,
  decimals: 0,
  scale: "linear",
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

/** A Shadows/Highlights setting, under heading `group`: `more` for Show More Options only. */
const shadowsParam = (
  key: string,
  group: MessageKey,
  label: MessageKey,
  range: [number, number],
  unit: MessageKey | undefined,
  more: boolean,
): NumberParam => ({
  kind: "number",
  key,
  label,
  unit,
  min: range[0],
  max: range[1],
  decimals: 0,
  scale: "linear",
  group,
  more,
});

/**
 * A filter's settings (the engine's `Filter::params` order), Photoshop's ranges and defaults;
 * `more`: its dialog has Show More Options, the settings marked `more` shown with it only.
 */
export type FilterSpec = { params: FilterParam[]; defaults: number[]; more?: boolean };

/** Each filter's settings (the engine's `Filter::params` order), Photoshop's ranges and defaults. */
export const FILTERS: Record<FilterId, FilterSpec> = {
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
  findEdges: { params: [], defaults: [] },
  solarize: { params: [], defaults: [] },
  emboss: {
    params: [
      {
        kind: "number",
        key: "angle",
        label: "filter.emboss.angle",
        unit: "filter.degrees",
        min: -180,
        max: 180,
        decimals: 0,
        scale: "linear",
      },
      {
        kind: "number",
        key: "height",
        label: "filter.emboss.height",
        unit: "filter.pixels",
        min: 1,
        max: 10,
        decimals: 0,
        scale: "linear",
      },
      {
        kind: "number",
        key: "amount",
        label: "filter.emboss.amount",
        unit: "filter.percent",
        min: 1,
        max: 500,
        decimals: 0,
        scale: "linear",
      },
    ],
    defaults: [135, 3, 100],
  },
  mosaic: { params: [squareRadius("filter.mosaic.cell", 200, 2)], defaults: [10] },
  twirl: {
    params: [
      {
        kind: "number",
        key: "angle",
        label: "filter.twirl.angle",
        unit: "filter.degrees",
        min: -999,
        max: 999,
        decimals: 0,
        scale: "linear",
      },
    ],
    defaults: [50],
  },
  radialBlur: {
    params: [
      {
        kind: "number",
        key: "amount",
        label: "filter.radialBlur.amount",
        min: 1,
        max: 100,
        decimals: 0,
        scale: "linear",
      },
      {
        kind: "choice",
        key: "method",
        label: "filter.radialBlur.method",
        options: ["filter.radialBlur.spin", "filter.radialBlur.zoom"],
      },
      {
        kind: "choice",
        key: "quality",
        label: "filter.radialBlur.quality",
        options: ["filter.radialBlur.draft", "filter.radialBlur.good", "filter.radialBlur.best"],
      },
      { kind: "center", key: "centerX", label: "filter.radialBlur.center", axis: "x" },
      { kind: "center", key: "centerY", label: "filter.radialBlur.center", axis: "y" },
    ],
    defaults: [10, 0, 1, 50, 50],
  },
  surfaceBlur: {
    params: [
      squareRadius("filter.surfaceBlur.radius", 100),
      {
        kind: "number",
        key: "threshold",
        label: "filter.surfaceBlur.threshold",
        unit: "filter.levels",
        min: 2,
        max: 255,
        decimals: 0,
        scale: "linear",
      },
    ],
    defaults: [5, 15],
  },
  smartBlur: {
    params: [
      {
        kind: "number",
        key: "radius",
        label: "filter.smartBlur.radius",
        unit: "filter.pixels",
        min: 0.1,
        max: 100,
        decimals: 1,
        scale: "log",
      },
      {
        kind: "number",
        key: "threshold",
        label: "filter.smartBlur.threshold",
        min: 0.1,
        max: 100,
        decimals: 1,
        scale: "linear",
      },
      {
        kind: "choice",
        key: "quality",
        label: "filter.smartBlur.quality",
        options: ["filter.smartBlur.low", "filter.smartBlur.medium", "filter.smartBlur.high"],
      },
      {
        kind: "choice",
        key: "mode",
        label: "filter.smartBlur.mode",
        options: [
          "filter.smartBlur.normal",
          "filter.smartBlur.edgeOnly",
          "filter.smartBlur.overlayEdge",
        ],
      },
    ],
    defaults: [3, 25, 0, 0],
  },
  average: { params: [], defaults: [] },
  pinch: { params: [percent("filter.pinch.amount")], defaults: [50] },
  spherize: { params: [percent("filter.spherize.amount")], defaults: [100] },
  polarCoordinates: {
    params: [
      {
        kind: "choice",
        key: "conversion",
        label: "filter.polarCoordinates.conversion",
        options: ["filter.polarCoordinates.polarToRect", "filter.polarCoordinates.rectToPolar"],
      },
    ],
    defaults: [1],
  },
  offset: {
    params: [
      shift("horizontal", "filter.offset.horizontal"),
      shift("vertical", "filter.offset.vertical"),
      {
        kind: "choice",
        key: "edge",
        label: "filter.offset.edge",
        options: ["filter.offset.transparent", "filter.offset.repeat", "filter.offset.wrap"],
      },
    ],
    defaults: [0, 0, 2],
  },
  shadowsHighlights: {
    params: [
      shadowsParam(
        "shadowsAmount",
        "filter.shadowsHighlights.shadows",
        "filter.shadowsHighlights.amount",
        [0, 100],
        "filter.percent",
        false,
      ),
      shadowsParam(
        "shadowsTone",
        "filter.shadowsHighlights.shadows",
        "filter.shadowsHighlights.tone",
        [0, 100],
        "filter.percent",
        true,
      ),
      shadowsParam(
        "shadowsRadius",
        "filter.shadowsHighlights.shadows",
        "filter.shadowsHighlights.radius",
        [0, 2500],
        "filter.pixels",
        true,
      ),
      shadowsParam(
        "highlightsAmount",
        "filter.shadowsHighlights.highlights",
        "filter.shadowsHighlights.amount",
        [0, 100],
        "filter.percent",
        false,
      ),
      shadowsParam(
        "highlightsTone",
        "filter.shadowsHighlights.highlights",
        "filter.shadowsHighlights.tone",
        [0, 100],
        "filter.percent",
        true,
      ),
      shadowsParam(
        "highlightsRadius",
        "filter.shadowsHighlights.highlights",
        "filter.shadowsHighlights.radius",
        [0, 2500],
        "filter.pixels",
        true,
      ),
      shadowsParam(
        "color",
        "filter.shadowsHighlights.adjustments",
        "filter.shadowsHighlights.color",
        [-100, 100],
        undefined,
        true,
      ),
      shadowsParam(
        "midtone",
        "filter.shadowsHighlights.adjustments",
        "filter.shadowsHighlights.midtone",
        [-100, 100],
        undefined,
        true,
      ),
    ],
    defaults: [35, 50, 30, 0, 50, 30, 20, 0],
    more: true,
  },
};

/** The filters of Image > Adjustments (on every pixel layer shown, not repeated by Ctrl+F). */
export const ADJUSTMENT_FILTERS: FilterId[] = ["shadowsHighlights"];

/** Whether `filter` is one of Image > Adjustments' (see `ADJUSTMENT_FILTERS`). */
export function isAdjustmentFilter(filter: FilterId): boolean {
  return ADJUSTMENT_FILTERS.includes(filter);
}

/** The settings a dialog shows: all of them with Show More Options, else those not `more`. */
export function shownParams(filter: FilterId, more: boolean): number[] {
  return FILTERS[filter].params.flatMap((param, i) =>
    more || !(param.kind === "number" && param.more) ? [i] : [],
  );
}

/** Whether `filter` has settings: those without (Average, Find Edges, Solarize) apply at once,
 * as in Photoshop, with no dialog and nothing to edit again. */
export function hasSettings(filter: FilterId): boolean {
  return FILTERS[filter].params.length > 0;
}

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
  {
    label: "menu.filter.blur",
    filters: [
      "average",
      "boxBlur",
      "gaussianBlur",
      "motionBlur",
      "radialBlur",
      "smartBlur",
      "surfaceBlur",
    ],
  },
  { label: "menu.filter.distort", filters: ["pinch", "polarCoordinates", "spherize", "twirl"] },
  { label: "menu.filter.noise", filters: ["addNoise", "dustAndScratches", "median"] },
  { label: "menu.filter.pixelate", filters: ["mosaic"] },
  { label: "menu.filter.sharpen", filters: ["unsharpMask", "clarityTexture"] },
  { label: "menu.filter.stylize", filters: ["emboss", "findEdges", "solarize"] },
  { label: "menu.filter.other", filters: ["highPass", "maximum", "minimum", "offset"] },
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
    case "center":
      return Number.isFinite(v) && v >= 0 && v <= 100;
  }
}

/** A center in percent of the frame (`[x, y]`, whole numbers). */
export type Center = [number, number];

/**
 * The center picked at client point (`x`, `y`) in a square shown at `box` (its client
 * rectangle): in percent of it, whole, within it.
 */
export function centerAt(
  box: { left: number; top: number; width: number; height: number },
  x: number,
  y: number,
): Center {
  const along = (v: number, start: number, size: number) =>
    size > 0 ? Math.round(Math.min(Math.max((v - start) / size, 0), 1) * 100) : 50;
  return [along(x, box.left, box.width), along(y, box.top, box.height)];
}

/**
 * The center moved by an arrow key (`key` as `KeyboardEvent.key`), by 1 % or 10 % with Shift;
 * `null` for another key.
 */
export function nudgedCenter(center: Center, key: string, shift: boolean): Center | null {
  const step = shift ? 10 : 1;
  const move: Record<string, Center> = {
    ArrowLeft: [-step, 0],
    ArrowRight: [step, 0],
    ArrowUp: [0, -step],
    ArrowDown: [0, step],
  };
  const d = move[key];
  if (!d) return null;
  const clamp = (v: number) => Math.min(Math.max(v, 0), 100);
  return [clamp(center[0] + d[0]), clamp(center[1] + d[1])];
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

/**
 * `settings` applied to each layer of `ids` (an adjustment filter on every pixel layer shown):
 * one edit, a batch when there are several.
 */
export function applyFiltersEdit(ids: number[], settings: FilterSettings): EditRequest {
  const edits = ids.map((id) => applyFilterEdit(id, settings));
  return edits.length === 1 ? edits[0] : { kind: "batch", edits };
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
