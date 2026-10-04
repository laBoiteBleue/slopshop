// The width of the panels column on the right of the window (Layers, Properties): dragged by
// its left edge, part of the saved layout (layout.ts).

export const DEFAULT_PANEL_WIDTH = 260;
export const MIN_PANEL_WIDTH = 200;
/** The image keeps at least this much room beside the panels. */
const MIN_CANVAS_WIDTH = 240;
/** The toolbar's width, left of the image. */
const TOOLBAR_WIDTH = 40;

/** `width` kept between the minimum and what leaves the image room in a `windowWidth` window. */
export function clampPanelWidth(width: number, windowWidth: number): number {
  const max = Math.max(MIN_PANEL_WIDTH, windowWidth - TOOLBAR_WIDTH - MIN_CANVAS_WIDTH);
  return Math.round(Math.min(Math.max(width, MIN_PANEL_WIDTH), max));
}
