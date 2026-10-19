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
 * 1); a seed, drawn anew each time the filter is applied (`withNewSeeds`), not shown or, with
 * `randomize`, a button drawing another (Wave's Randomize); a coordinate of a center in
 * percent of the frame (0 to 100), picked in a square: the `x` one shows the square for itself
 * and the `y` one that follows it; the foreground or background color, not shown, taken each
 * time the filter is applied (`withColors`) and kept by its entry (Clouds'); or one of Custom's
 * 5 × 5 weights (a whole number from -999 to 999), the first (`cell` 0) showing the grid of all
 * of them, row by row.
 */
export type FilterParam =
  | NumberParam
  | { kind: "choice"; key: string; label: MessageKey; options: MessageKey[] }
  | { kind: "check"; key: string; label: MessageKey }
  | { kind: "seed"; key: string; randomize?: MessageKey }
  | { kind: "center"; key: string; label: MessageKey; axis: "x" | "y" }
  | { kind: "color"; key: string; color: "foreground" | "background" }
  | { kind: "weight"; key: string; cell: number };

/** Custom's weights: whole numbers from -999 to 999 (Photoshop's), 5 × 5 of them. */
export const MAX_WEIGHT = 999;
export const KERNEL_SIDE = 5;

/** The seeds a filter takes: whole numbers below 2^24 (the engine's `NOISE_SEEDS`). */
export const SEEDS = 2 ** 24;

/** The colors a filter keeps: sRGB `0xRRGGBB`, below 2^24 (the engine's `PACKED_COLORS`). */
export const PACKED_COLORS = 2 ** 24;

/** The drawing colors a filter takes (Clouds', Fibers'), foreground then background. */
const drawingColors: FilterParam[] = [
  { kind: "color", key: "foreground", color: "foreground" },
  { kind: "color", key: "background", color: "background" },
];

/** An Oil Paint setting from `min` to 10, a decimal (Photoshop's), under heading `group`. */
const oilSetting = (
  key: string,
  label: MessageKey,
  min: number,
  group?: MessageKey,
): NumberParam => ({
  kind: "number",
  key,
  label,
  min,
  max: 10,
  decimals: 1,
  scale: "linear",
  group,
});

/** A Fibers setting from 1 to 64 (Photoshop's). */
const fibersSetting = (key: string, label: MessageKey): NumberParam => ({
  kind: "number",
  key,
  label,
  min: 1,
  max: 64,
  decimals: 0,
  scale: "linear",
});

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

/** Maximum's and Minimum's Preserve: Squareness or Roundness (the square or the disk around). */
const preserve: FilterParam = {
  kind: "choice",
  key: "preserve",
  label: "filter.preserve",
  options: ["filter.preserve.squareness", "filter.preserve.roundness"],
};

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

/** A whole number from `min` to `max`, evenly on its slider (Wave's, ZigZag's settings). */
const whole = (
  key: string,
  label: MessageKey,
  [min, max]: [number, number],
  unit?: MessageKey,
  group?: MessageKey,
): NumberParam => ({
  kind: "number",
  key,
  label,
  unit,
  min,
  max,
  decimals: 0,
  scale: "linear",
  group,
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
  custom: {
    params: [
      ...Array.from({ length: KERNEL_SIDE * KERNEL_SIDE }, (_, cell): FilterParam => ({
        kind: "weight",
        key: `weight${cell}`,
        cell,
      })),
      {
        kind: "number",
        key: "scale",
        label: "filter.custom.scale",
        min: 1,
        max: 9999,
        decimals: 0,
        scale: "log",
      },
      {
        kind: "number",
        key: "offset",
        label: "filter.custom.offset",
        min: -9999,
        max: 9999,
        decimals: 0,
        scale: "linear",
      },
    ],
    // Photoshop's: a sharpening kernel, the center 5 and its four neighbours -1.
    defaults: [
      ...[0, 0, 0, 0, 0],
      ...[0, 0, -1, 0, 0],
      ...[0, -1, 5, -1, 0],
      ...[0, 0, -1, 0, 0],
      ...[0, 0, 0, 0, 0],
      1,
      0,
    ],
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
        key: "order",
        label: "filter.hsbHsl.order",
        options: ["filter.hsbHsl.rgb", "filter.hsbHsl.hsb", "filter.hsbHsl.hsl"],
      },
    ],
    defaults: [0, 1],
  },
  despeckle: { params: [], defaults: [] },
  reduceNoise: {
    params: [
      whole("strength", "filter.reduceNoise.strength", [0, 10]),
      whole("preserve", "filter.reduceNoise.preserve", [0, 100], "filter.percent"),
      whole("color", "filter.reduceNoise.color", [0, 100], "filter.percent"),
      whole("sharpen", "filter.reduceNoise.sharpen", [0, 100], "filter.percent"),
    ],
    defaults: [6, 60, 45, 25],
  },
  smartSharpen: {
    params: [
      whole("amount", "filter.smartSharpen.amount", [1, 500], "filter.percent"),
      {
        kind: "number",
        key: "radius",
        label: "filter.smartSharpen.radius",
        unit: "filter.pixels",
        min: 0.1,
        max: 64,
        decimals: 1,
        scale: "log",
      },
      whole("noise", "filter.smartSharpen.noise", [0, 100], "filter.percent"),
      {
        kind: "choice",
        key: "remove",
        label: "filter.smartSharpen.remove",
        options: [
          "filter.smartSharpen.gaussianBlur",
          "filter.smartSharpen.lensBlur",
          "filter.smartSharpen.motionBlur",
        ],
      },
      whole("angle", "filter.smartSharpen.angle", [-180, 180], "filter.degrees"),
    ],
    defaults: [200, 1, 10, 1, 0],
  },
  boxBlur: { params: [squareRadius("filter.boxBlur.radius", 2000)], defaults: [10] },
  median: { params: [squareRadius("filter.median.radius", 500)], defaults: [1] },
  maximum: { params: [squareRadius("filter.maximum.radius", 500), preserve], defaults: [1, 0] },
  minimum: { params: [squareRadius("filter.minimum.radius", 500), preserve], defaults: [1, 0] },
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
  colorHalftone: {
    params: [
      whole("radius", "filter.colorHalftone.radius", [4, 127], "filter.pixels"),
      ...([1, 2, 3] as const).map((n) =>
        whole(
          `angle${n}`,
          `filter.colorHalftone.channel${n}`,
          [-360, 360],
          "filter.degrees",
          "filter.colorHalftone.angles",
        ),
      ),
    ],
    defaults: [8, 108, 162, 90],
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
  // Photoshop's default background color, white, until applied with the current one.
  pointillize: {
    params: [
      squareRadius("filter.pointillize.cell", 300, 3),
      { kind: "color", key: "background", color: "background" },
      { kind: "seed", key: "seed" },
    ],
    defaults: [5, 0xffffff, 0],
  },
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
  // Photoshop's default colors, black and white, until applied with the current ones.
  clouds: { params: [...drawingColors, { kind: "seed", key: "seed" }], defaults: [0, 0xffffff, 0] },
  differenceClouds: {
    params: [...drawingColors, { kind: "seed", key: "seed" }],
    defaults: [0, 0xffffff, 0],
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
  tiles: {
    params: [
      whole("tiles", "filter.tiles.number", [1, 99]),
      whole("offset", "filter.tiles.offset", [1, 99], "filter.percent"),
      {
        kind: "choice",
        key: "fill",
        label: "filter.tiles.fill",
        options: [
          "filter.tiles.background",
          "filter.tiles.foreground",
          "filter.tiles.inverse",
          "filter.tiles.unaltered",
        ],
      },
      ...drawingColors,
      { kind: "seed", key: "seed" },
    ],
    defaults: [10, 10, 0, 0, 0xffffff, 0],
  },
  traceContour: {
    params: [
      whole("level", "filter.traceContour.level", [0, 255]),
      {
        kind: "choice",
        key: "edge",
        label: "filter.traceContour.edge",
        options: ["filter.traceContour.lower", "filter.traceContour.upper"],
      },
    ],
    defaults: [128, 1],
  },
  oilPaint: {
    params: [
      oilSetting("stylization", "filter.oilPaint.stylization", 0.1, "filter.oilPaint.brush"),
      oilSetting("cleanliness", "filter.oilPaint.cleanliness", 0, "filter.oilPaint.brush"),
      oilSetting("scale", "filter.oilPaint.scale", 0.1, "filter.oilPaint.brush"),
      oilSetting("bristle", "filter.oilPaint.bristle", 0, "filter.oilPaint.brush"),
      { kind: "check", key: "lighting", label: "filter.oilPaint.lighting" },
      {
        kind: "number",
        key: "angle",
        label: "filter.oilPaint.angle",
        unit: "filter.degrees",
        min: -180,
        max: 180,
        decimals: 0,
        scale: "linear",
      },
      oilSetting("shine", "filter.oilPaint.shine", 0),
    ],
    defaults: [4, 5, 1, 2, 1, -60, 1],
  },
  fibers: {
    params: [
      fibersSetting("variance", "filter.fibers.variance"),
      fibersSetting("strength", "filter.fibers.strength"),
      ...drawingColors,
      { kind: "seed", key: "seed", randomize: "filter.fibers.randomize" },
    ],
    defaults: [16, 4, 0, 0xffffff, 0],
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
          "filter.spherize.horizontalOnly",
          "filter.spherize.verticalOnly",
        ],
      },
    ],
    defaults: [100, 0],
  },
  ripple: {
    params: [
      whole("amount", "filter.ripple.amount", [-999, 999], "filter.percent"),
      {
        kind: "choice",
        key: "size",
        label: "filter.ripple.size",
        options: ["filter.ripple.small", "filter.ripple.medium", "filter.ripple.large"],
      },
    ],
    defaults: [100, 1],
  },
  wave: {
    params: [
      whole("generators", "filter.wave.generators", [1, 999]),
      whole("wavelengthMin", "filter.wave.min", [1, 999], undefined, "filter.wave.wavelength"),
      whole("wavelengthMax", "filter.wave.max", [1, 999], undefined, "filter.wave.wavelength"),
      whole("amplitudeMin", "filter.wave.min", [1, 999], undefined, "filter.wave.amplitude"),
      whole("amplitudeMax", "filter.wave.max", [1, 999], undefined, "filter.wave.amplitude"),
      whole("scaleH", "filter.wave.horizontal", [1, 100], "filter.percent", "filter.wave.scale"),
      whole("scaleV", "filter.wave.vertical", [1, 100], "filter.percent", "filter.wave.scale"),
      {
        kind: "choice",
        key: "type",
        label: "filter.wave.type",
        options: ["filter.wave.sine", "filter.wave.triangle", "filter.wave.square"],
      },
      {
        kind: "choice",
        key: "edge",
        label: "filter.offset.edge",
        options: ["filter.offset.wrap", "filter.offset.repeat"],
      },
      { kind: "seed", key: "seed", randomize: "filter.wave.randomize" },
    ],
    defaults: [5, 10, 120, 5, 35, 100, 100, 0, 1, 0],
  },
  zigZag: {
    params: [
      whole("amount", "filter.zigZag.amount", [-100, 100]),
      whole("ridges", "filter.zigZag.ridges", [1, 20]),
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

/** Whether `filter` has settings to show: those without (Average, Find Edges, Clouds) apply at
 * once, as in Photoshop, with no dialog and nothing to edit again. */
export function hasSettings(filter: FilterId): boolean {
  return FILTERS[filter].params.some(
    (param) => param.kind !== "color" && (param.kind !== "seed" || param.randomize),
  );
}

/** `values` of `filter` with its colors the current drawing colors (`#rrggbb`): the filter
 * applied anew, with the colors of the moment, as in Photoshop. */
export function withColors(
  filter: FilterId,
  values: number[],
  colors: { foreground: string; background: string },
): number[] {
  return values.map((v, i) => {
    const param = FILTERS[filter].params[i];
    return param?.kind === "color" ? packColor(colors[param.color]) : v;
  });
}

/** `#rrggbb` as a filter keeps it (see `PACKED_COLORS`). */
export function packColor(hex: string): number {
  return parseInt(hex.slice(1, 7), 16);
}

/** `values` of `filter` with each seed drawn anew: the filter applied again, another grain. */
export function withNewSeeds(
  filter: FilterId,
  values: number[],
  random: () => number = Math.random,
): number[] {
  return values.map((v, i) => (FILTERS[filter].params[i]?.kind === "seed" ? newSeed(random) : v));
}

/** Another seed (Wave's Randomize): a whole number below `SEEDS`. */
export function newSeed(random: () => number = Math.random): number {
  return Math.floor(random() * SEEDS);
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
  {
    label: "menu.filter.distort",
    filters: ["pinch", "polarCoordinates", "ripple", "spherize", "twirl", "wave", "zigZag"],
  },
  {
    label: "menu.filter.noise",
    filters: ["addNoise", "despeckle", "dustAndScratches", "median", "reduceNoise"],
  },
  {
    label: "menu.filter.pixelate",
    filters: [
      "colorHalftone",
      "crystallize",
      "facet",
      "fragment",
      "mezzotint",
      "mosaic",
      "pointillize",
    ],
  },
  { label: "menu.filter.render", filters: ["clouds", "differenceClouds", "fibers"] },
  { label: "menu.filter.sharpen", filters: ["smartSharpen", "unsharpMask", "clarityTexture"] },
  {
    label: "menu.filter.stylize",
    filters: [
      "diffuse",
      "emboss",
      "findEdges",
      "oilPaint",
      "solarize",
      "tiles",
      "traceContour",
      "wind",
    ],
  },
  {
    label: "menu.filter.other",
    filters: ["custom", "highPass", "hsbHsl", "maximum", "minimum", "offset"],
  },
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
    case "color":
      return Number.isInteger(v) && v >= 0 && v < PACKED_COLORS;
    case "weight":
      return Number.isInteger(v) && Math.abs(v) <= MAX_WEIGHT;
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
