// The panels below Layers in the right column (maintainer's choice, 2026-10-04): a dock that
// folds down to a row of tab icons, a click on an icon unfolding that panel. Layers stays above
// it, always shown. Which panel is unfolded and how tall the dock is are part of the saved
// layout (layout.ts); the panels are listed in panels/index.ts.

import type { PanelId } from "./panels/registry";

export type DockPanel = PanelId;

/** The panel unfolded (`null`: folded down to the tabs) and the unfolded dock's height. */
export type DockState = { open: DockPanel | null; height: number };

export const DEFAULT_DOCK: DockState = { open: "properties", height: 280 };
/** The unfolded dock's smallest height, CSS pixels. */
export const MIN_DOCK_HEIGHT = 120;
/** Layers keeps at least this much above the dock. */
const MIN_LAYERS_HEIGHT = 160;

/** A click on `panel`'s tab: it unfolds, or, already unfolded, the dock folds. */
export function clickTab(state: DockState, panel: DockPanel): DockState {
  return { ...state, open: state.open === panel ? null : panel };
}

/** `height` kept between the minimum and what leaves Layers room in a `column` tall column. */
export function clampDockHeight(height: number, column: number): number {
  const max = Math.max(MIN_DOCK_HEIGHT, column - MIN_LAYERS_HEIGHT);
  return Math.round(Math.min(Math.max(height, MIN_DOCK_HEIGHT), max));
}

/**
 * Properties replaced `replaced` in the dock by itself (`null`: the dock was folded), to give
 * it back; `null` when it did not, or the user has chosen what the dock shows since.
 */
export type PropertiesFollow = { replaced: DockPanel | null } | null;

/**
 * The dock as the active layer with properties goes from `before` to `now` (its id; `null`:
 * the active layer has none). One just selected unfolds Properties, as in Photoshop; once none
 * is, the dock goes back to what Properties replaced, unless the user chose since (`follow`
 * is then `null`).
 */
export function followProperties(
  dock: DockState,
  follow: PropertiesFollow,
  before: number | null,
  now: number | null,
): { dock: DockState; follow: PropertiesFollow } {
  if (now !== null && now !== before) {
    if (dock.open === "properties") return { dock, follow };
    return { dock: { ...dock, open: "properties" }, follow: { replaced: dock.open } };
  }
  if (now === null && before !== null && follow) {
    const back = dock.open === "properties" ? { ...dock, open: follow.replaced } : dock;
    return { dock: back, follow: null };
  }
  return { dock, follow };
}
