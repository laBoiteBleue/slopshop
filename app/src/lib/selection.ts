// Selection tools (ADR 0024): what the UI needs to know; the masks are the engine's.
import type { SelectionMode } from "./engine";

/** The largest feather radius the engine accepts, in pixels (`selection::MAX_FEATHER`). */
export const MAX_FEATHER = 250;

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
