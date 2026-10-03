// The panels below Layers in the right column (maintainer's choice, 2026-10-04): a dock that
// folds down to a row of tab icons, a click on an icon unfolding that panel. Layers stays above
// it, always shown. Which panel is unfolded and how tall the dock is are remembered on this
// machine.

/** The dock's panels, in the order of their tabs (a future Window menu lists the same). */
export const DOCK_PANELS = ["properties", "selections"] as const;
export type DockPanel = (typeof DOCK_PANELS)[number];

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

const STORAGE_KEY = "slopshop.dock";
type Store = Pick<Storage, "getItem" | "setItem">;

/** The state saved last, or the default (nothing saved, storage unavailable or corrupt). */
export function loadDock(store?: Store): DockState {
  try {
    const saved = JSON.parse((store ?? localStorage).getItem(STORAGE_KEY) ?? "null");
    const open =
      saved?.open === null || DOCK_PANELS.includes(saved?.open) ? saved.open : DEFAULT_DOCK.open;
    const height =
      Number.isFinite(saved?.height) && saved.height >= MIN_DOCK_HEIGHT
        ? saved.height
        : DEFAULT_DOCK.height;
    return saved ? { open, height } : { ...DEFAULT_DOCK };
  } catch {
    return { ...DEFAULT_DOCK };
  }
}

export function saveDock(state: DockState, store?: Store) {
  try {
    (store ?? localStorage).setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    // Not remembered: it lasts for the session.
  }
}
