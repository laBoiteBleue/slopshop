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

/** A wavelength or an amplitude of Wave's, 1 to 999 pixels (Photoshop's). */
const waveSize = (key: string, label: MessageKey): NumberParam => ({
  kind: "number",
  key,
  label,
  unit: "filter.pixels",
  min: 1,
  max: 999,
  decimals: 0,
  scale: "log",
});

/** How much of Wave's move goes one way, 1 to 100 % (Photoshop's Scale). */
const waveScale = (key: string, label: MessageKey): NumberParam => ({
  kind: "number",
  key,
  label,
  unit: "filter.percent",
  min: 1,
  max: 100,
  decimals: 0,
  scale: "linear",
});

/** A screen's angle of Color Halftone's, -360 to 360 degrees (Photoshop's). */
const screenAngle = (key: string, label: MessageKey): NumberParam => ({
  kind: "number",
  key,
  label,
  unit: "filter.degrees",
  min: -360,
  max: 360,
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
  pinch: { params: [percent("filter.pinch.amount")], defaults: [50] },
  spherize: {
    params: [
      percent("filter.spherize.amount"),
      {
        kind: "choice",
        key: "mode",
        label: "filter.spherize.mode",
        options: [
          "filter.spherize.normal",
          "filter.spherize.horizontal",
          "filter.spherize.vertical",
        ],
      },
    ],
    defaults: [100, 0],
  },
  ripple: {
    params: [
      {
        kind: "number",
        key: "amount",
        label: "filter.ripple.amount",
        unit: "filter.percent",
        min: -999,
        max: 999,
        decimals: 0,
        scale: "linear",
      },
      {
        kind: "choice",
        key: "size",
        label: "filter.ripple.size",
        options: ["filter.ripple.small", "filter.ripple.medium", "filter.ripple.large"],
      },
    ],
    defaults: [100, 1],
  },
  zigZag: {
    params: [
      percent("filter.zigZag.amount"),
      {
        kind: "number",
        key: "ridges",
        label: "filter.zigZag.ridges",
        min: 1,
        max: 20,
        decimals: 0,
        scale: "linear",
      },
      {
        kind: "choice",
        key: "style",
        label: "filter.zigZag.style",
        options: [
          "filter.zigZag.aroundCenter",
          "filter.zigZag.outFromCenter",
          "filter.zigZag.pondRipples",
        ],
      },
    ],
    defaults: [10, 5, 2],
  },
  wind: {
    params: [
      {
        kind: "choice",
        key: "method",
        label: "filter.wind.method",
        options: ["filter.wind.wind", "filter.wind.blast", "filter.wind.stagger"],
      },
      {
        kind: "choice",
        key: "direction",
        label: "filter.wind.direction",
        options: ["filter.wind.fromRight", "filter.wind.fromLeft"],
      },
      { kind: "seed", key: "seed" },
    ],
    defaults: [0, 0, 0],
  },
  diffuse: {
    params: [
      {
        kind: "choice",
        key: "mode",
        label: "filter.diffuse.mode",
        options: [
          "filter.diffuse.normal",
          "filter.diffuse.darkenOnly",
          "filter.diffuse.lightenOnly",
          "filter.diffuse.anisotropic",
        ],
      },
      { kind: "seed", key: "seed" },
    ],
    defaults: [0, 0],
  },
  traceContour: {
    params: [
      {
        kind: "number",
        key: "level",
        label: "filter.traceContour.level",
        min: 0,
        max: 255,
        decimals: 0,
        scale: "linear",
      },
      {
        kind: "choice",
        key: "edge",
        label: "filter.traceContour.edge",
        options: ["filter.traceContour.lower", "filter.traceContour.upper"],
      },
    ],
    defaults: [128, 1],
  },
  crystallize: {
    params: [squareRadius("filter.crystallize.cell", 300, 3), { kind: "seed", key: "seed" }],
    defaults: [10, 0],
  },
  facet: { params: [], defaults: [] },
  fragment: { params: [], defaults: [] },
  mezzotint: {
    params: [
      {
        kind: "choice",
        key: "type",
        label: "filter.mezzotint.type",
        options: [
          "filter.mezzotint.fineDots",
          "filter.mezzotint.mediumDots",
          "filter.mezzotint.grainyDots",
          "filter.mezzotint.coarseDots",
          "filter.mezzotint.shortLines",
          "filter.mezzotint.mediumLines",
          "filter.mezzotint.longLines",
          "filter.mezzotint.shortStrokes",
          "filter.mezzotint.mediumStrokes",
          "filter.mezzotint.longStrokes",
        ],
      },
      { kind: "seed", key: "seed" },
    ],
    defaults: [0, 0],
  },
  colorHalftone: {
    params: [
      {
        kind: "number",
        key: "radius",
        label: "filter.colorHalftone.radius",
        unit: "filter.pixels",
        min: 4,
        max: 127,
        decimals: 0,
        scale: "log",
      },
      screenAngle("channel1", "filter.colorHalftone.channel1"),
      screenAngle("channel2", "filter.colorHalftone.channel2"),
      screenAngle("channel3", "filter.colorHalftone.channel3"),
      screenAngle("channel4", "filter.colorHalftone.channel4"),
    ],
    defaults: [8, 108, 162, 90, 45],
  },
  hsbHsl: {
    params: [
      {
        kind: "choice",
        key: "input",
        label: "filter.hsbHsl.input",
        options: ["filter.hsbHsl.rgb", "filter.hsbHsl.hsb", "filter.hsbHsl.hsl"],
      },
      {
        kind: "choice",
        key: "output",
        label: "filter.hsbHsl.output",
        options: ["filter.hsbHsl.rgb", "filter.hsbHsl.hsb", "filter.hsbHsl.hsl"],
      },
    ],
    defaults: [0, 1],
  },
  wave: {
    params: [
      {
        kind: "number",
        key: "generators",
        label: "filter.wave.generators",
        min: 1,
        max: 999,
        decimals: 0,
        scale: "log",
      },
      waveSize("shortest", "filter.wave.wavelengthMin"),
      waveSize("longest", "filter.wave.wavelengthMax"),
      waveSize("lowest", "filter.wave.amplitudeMin"),
      waveSize("highest", "filter.wave.amplitudeMax"),
      waveScale("horizontal", "filter.wave.scaleHorizontal"),
      waveScale("vertical", "filter.wave.scaleVertical"),
      {
        kind: "choice",
        key: "type",
        label: "filter.wave.type",
        options: ["filter.wave.sine", "filter.wave.triangle", "filter.wave.square"],
      },
      {
        kind: "choice",
        key: "edge",
        label: "filter.wave.edge",
        options: ["filter.offset.wrap", "filter.offset.repeat"],
      },
      { kind: "seed", key: "seed" },
    ],
    defaults: [5, 10, 120, 5, 35, 100, 100, 0, 1, 0],
  },
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
};

/** Whether `filter` has settings: those without (Find Edges, Solarize) apply at once, as in
 * Photoshop, with no dialog and nothing to edit again. */
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
  { label: "menu.filter.blur", filters: ["boxBlur", "gaussianBlur", "motionBlur"] },
  {
    label: "menu.filter.distort",
    filters: ["pinch", "polarCoordinates", "ripple", "spherize", "twirl", "wave", "zigZag"],
  },
  { label: "menu.filter.noise", filters: ["addNoise", "dustAndScratches", "median"] },
  {
    label: "menu.filter.pixelate",
    filters: ["colorHalftone", "crystallize", "facet", "fragment", "mezzotint", "mosaic"],
  },
  { label: "menu.filter.sharpen", filters: ["unsharpMask", "clarityTexture"] },
  {
    label: "menu.filter.stylize",
    filters: ["diffuse", "emboss", "findEdges", "solarize", "traceContour", "wind"],
  },
  { label: "menu.filter.other", filters: ["highPass", "hsbHsl", "maximum", "minimum", "offset"] },
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
