// Selection tools (ADR 0024): what the UI needs to know; the masks are the engine's.
import type { SelectionMode } from "./engine";

/** The largest feather radius the engine accepts, in pixels (`selection::MAX_FEATHER`). */
export const MAX_FEATHER = 250;

/** The largest amount of Select > Modify's other changes (`selection::MAX_MODIFY`). */
export const MAX_MODIFY = 500;

/** The sign by the pointer of a selection tool: how the next shape combines. */
export const MODE_BADGES: Record<SelectionMode, string> = {
  replace: "",
  add: "+",
  subtract: "−",
  intersect: "×",
};

/**
 * The mode a press asks for with its keys, as in Photoshop: Shift adds, Alt subtracts, both
 * intersect; `null` without them (the options bar's mode applies).
 */
export function modeFromKeys(e: { shiftKey: boolean; altKey: boolean }): SelectionMode | null {
  if (e.shiftKey && e.altKey) return "intersect";
  if (e.shiftKey) return "add";
  if (e.altKey) return "subtract";
  return null;
}

/** The largest brush (Quick Selection), document pixels, as Photoshop's. */
export const MAX_BRUSH = 5000;

/**
 * The next brush size with `[` (smaller) or `]` (larger), in Photoshop-like steps. Going down
 * takes the step of the range below, so that `[` undoes `]`.
 */
export function stepBrush(size: number, larger: boolean): number {
  const from = larger ? size : size - 1;
  const step = from < 10 ? 1 : from < 50 ? 5 : from < 100 ? 10 : from < 500 ? 25 : 100;
  const next = larger ? size + step : size - step;
  return Math.min(Math.max(Math.round(next / step) * step, 1), MAX_BRUSH);
}

/** The largest Refine Edge radius, document pixels (the engine's `MAX_REFINE_RADIUS`). */
export const MAX_REFINE = 256;
