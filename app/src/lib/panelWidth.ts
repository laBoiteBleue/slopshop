// The width of the panels column on the right of the window (Layers, Properties): dragged by
// its left edge, remembered on this machine.

export const DEFAULT_PANEL_WIDTH = 260;
export const MIN_PANEL_WIDTH = 200;
/** The image keeps at least this much room beside the panels. */
const MIN_CANVAS_WIDTH = 240;
/** The toolbar's width, left of the image. */
const TOOLBAR_WIDTH = 40;

const STORAGE_KEY = "slopshop.panelWidth";

/** `width` kept between the minimum and what leaves the image room in a `windowWidth` window. */
export function clampPanelWidth(width: number, windowWidth: number): number {
  const max = Math.max(MIN_PANEL_WIDTH, windowWidth - TOOLBAR_WIDTH - MIN_CANVAS_WIDTH);
  return Math.round(Math.min(Math.max(width, MIN_PANEL_WIDTH), max));
}

/** The width saved last, or the default (nothing saved, storage unavailable or corrupt). */
export function loadPanelWidth(): number {
  try {
    const saved = Number(localStorage.getItem(STORAGE_KEY));
    if (Number.isFinite(saved) && saved >= MIN_PANEL_WIDTH) return saved;
  } catch {
    // Storage unavailable: the default.
  }
  return DEFAULT_PANEL_WIDTH;
}

export function savePanelWidth(width: number) {
  try {
    localStorage.setItem(STORAGE_KEY, String(width));
  } catch {
    // Not remembered; the width lasts for the session.
  }
}
