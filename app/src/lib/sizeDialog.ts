// The math of Image > Image Size and Image > Canvas Size (SizeDialog.svelte), as in Photoshop:
// sizes in percent, pixels, inches, centimeters or millimeters at the document's resolution
// (ADR 0028). Image Size resamples (proportions linked by default) and sets the resolution;
// with Resample off, the pixels stay and a printed size sets the resolution. Canvas Size's
// relative sizes add to the current ones. Each change says which fields to rewrite: the others
// keep what is being typed.

import { MAX_PPI, MIN_PPI, fromPixels, rounded, toPixels, type LengthUnit } from "./units";

export type SizeUnit = LengthUnit | "percent";
export type SizeField = "width" | "height" | "resolution";
type Side = "width" | "height";

/** Largest side accepted, in pixels. */
export const MAX_SIDE = 300_000;

export type SizeState = {
  mode: "image" | "canvas";
  /** The document now: size in pixels, resolution in pixels per inch. */
  current: { width: number; height: number; ppi: number };
  unit: SizeUnit;
  /** Canvas Size: the fields are what is added. */
  relative: boolean;
  /** Image Size keeps the proportions. */
  constrain: boolean;
  resample: boolean;
  /** The new size in pixels (not rounded while typing) and resolution, pixels per inch. */
  width: number;
  height: number;
  ppi: number;
};

export function initialSize(mode: SizeState["mode"], current: SizeState["current"]): SizeState {
  return {
    mode,
    current,
    unit: "px",
    relative: false,
    constrain: true,
    resample: true,
    width: current.width,
    height: current.height,
    ppi: current.ppi,
  };
}

const other = (side: Side): Side => (side === "width" ? "height" : "width");

/** What a side's field shows, in the fields' unit. */
export function sideField(state: SizeState, side: Side): number {
  const size = state.current[side];
  const value = state.relative ? state[side] - size : state[side];
  return state.unit === "percent"
    ? rounded((value / size) * 100, "percent")
    : rounded(fromPixels(value, state.unit, state.ppi), state.unit);
}

/** A side's field value as pixels. */
function typedPixels(state: SizeState, side: Side, value: number): number {
  const size = state.current[side];
  const extra =
    state.unit === "percent" ? (size * value) / 100 : toPixels(value, state.unit, state.ppi);
  return state.relative ? size + extra : extra;
}

/**
 * A width or a height typed. Without resampling the pixels stay: the length typed is the print
 * size, which sets the resolution (null when it cannot). Image Size keeps the proportions when
 * asked: the other side follows.
 */
export function typeSide(
  state: SizeState,
  side: Side,
  value: number,
): { state: SizeState; rewrite: SizeField[] } | null {
  if (!Number.isFinite(value)) return null;
  const pixels = typedPixels(state, side, value);
  if (state.mode === "image" && !state.resample) {
    if (pixels <= 0) return null;
    const ppi = (state.ppi * state.current[side]) / pixels;
    return { state: { ...state, ppi }, rewrite: ["resolution", other(side)] };
  }
  const next = { ...state, [side]: pixels };
  if (state.mode !== "image" || !state.constrain) return { state: next, rewrite: [] };
  const follower = other(side);
  next[follower] = (pixels * state.current[follower]) / state.current[side];
  return { state: next, rewrite: [follower] };
}

/**
 * A resolution typed (pixels per inch): with resampling the print size stays and the pixels
 * follow. Pixel fields change with resampling, length fields without; percents follow pixels.
 */
export function typeResolution(
  state: SizeState,
  ppi: number,
): { state: SizeState; rewrite: SizeField[] } | null {
  if (!Number.isFinite(ppi) || ppi <= 0) return null;
  const scale = state.resample ? ppi / state.ppi : 1;
  const next = { ...state, width: state.width * scale, height: state.height * scale, ppi };
  const changed = (state.unit === "px") === state.resample || state.unit === "percent";
  return { state: next, rewrite: changed ? ["width", "height"] : [] };
}

/**
 * Resample on or off. Off: back to the image's pixels, shown as their print size (pixels and
 * percents cannot change any more). Every field is rewritten then.
 */
export function withResample(state: SizeState, resample: boolean): SizeState {
  if (resample) return { ...state, resample };
  const unit = state.unit === "px" || state.unit === "percent" ? "cm" : state.unit;
  return { ...state, resample, unit, width: state.current.width, height: state.current.height };
}

/** Without resampling, the pixels are fixed: they cannot be typed in pixels or percent. */
export function isLocked(state: SizeState): boolean {
  return (
    state.mode === "image" && !state.resample && (state.unit === "px" || state.unit === "percent")
  );
}

/** The size applied, whole pixels. */
export function newSize(state: SizeState): { width: number; height: number } {
  return { width: Math.round(state.width), height: Math.round(state.height) };
}

export function isValid(state: SizeState): boolean {
  const { width, height } = newSize(state);
  return (
    [width, height].every((side) => Number.isFinite(side) && side >= 1 && side <= MAX_SIDE) &&
    Number.isFinite(state.ppi) &&
    state.ppi >= MIN_PPI &&
    state.ppi <= MAX_PPI
  );
}
