// The viewport's geometry: between document pixels and the viewport's CSS pixels, given the
// view the engine computed (document = origin + output / zoom, output in device pixels), the
// reprojection of a frame drawn for an older view, and the smooth zoom's steps.

export type View = { zoom: number; origin: [number, number] };

/** Before the engine's first answer. */
export const NO_VIEW: View = { zoom: 1, origin: [0, 0] };

/** The document point at (`x`, `y`), CSS pixels from the viewport's top-left corner. */
export function toDocument(view: View, dpr: number, x: number, y: number): [number, number] {
  return [view.origin[0] + (x * dpr) / view.zoom, view.origin[1] + (y * dpr) / view.zoom];
}

/** Where a document point is in the viewport, CSS pixels from its top-left corner. */
export function toViewport(view: View, dpr: number, x: number, y: number): [number, number] {
  return [((x - view.origin[0]) * view.zoom) / dpr, ((y - view.origin[1]) * view.zoom) / dpr];
}

/**
 * The CSS transform (translate by `tx`, `ty`, then scale by `k`) that puts a frame drawn for
 * `shown` where `target` shows it; null when it is already there. A point drawn at `output` in
 * the shown frame belongs at (origin_s - origin_t) * zoom_t + output * zoom_t / zoom_s.
 */
export function reprojection(
  shown: View,
  target: View,
  dpr: number,
): { tx: number; ty: number; k: number } | null {
  const k = target.zoom / shown.zoom;
  const tx = ((shown.origin[0] - target.origin[0]) * target.zoom) / dpr;
  const ty = ((shown.origin[1] - target.origin[1]) * target.zoom) / dpr;
  const identity = Math.abs(k - 1) < 1e-9 && Math.abs(tx) < 1e-3 && Math.abs(ty) < 1e-3;
  return identity ? null : { tx, ty, k };
}

/**
 * The part of the zoom still to apply (`remaining`, in log space) that an exponential ease of
 * time constant `tau` applies over `dt` ms; all of it once what is left is negligible.
 */
export function easeStep(remaining: number, dt: number, tau: number): number {
  const step = remaining * (1 - Math.exp(-dt / tau));
  return Math.abs(remaining - step) < 1e-3 ? remaining : step;
}

/** `WheelEvent.deltaMode` values (Node has no `WheelEvent`). */
const DOM_DELTA_LINE = 1;
const DOM_DELTA_PAGE = 2;

/** A wheel delta in CSS pixels, whatever the device reports (pixels, lines or pages). */
export function wheelPixels(delta: number, deltaMode: number, pageHeight: number): number {
  const unit = deltaMode === DOM_DELTA_LINE ? 16 : deltaMode === DOM_DELTA_PAGE ? pageHeight : 1;
  return delta * unit;
}

/** Zoom from which the pixel grid shows by itself (800%): pixels big enough to tell apart. */
export const PIXEL_GRID_ZOOM = 8;

/**
 * Where the pixel grid is drawn, in the viewport's CSS pixels: the part of the canvas
 * (`width` × `height` document pixels) inside the viewport (`viewWidth` × `viewHeight`), its
 * cells `cell` wide, a line at (`offsetX`, `offsetY`) from the area's corner (within one cell
 * before it); null below [`PIXEL_GRID_ZOOM`] or with nothing in sight. Only the visible part:
 * a whole canvas at 800% would be a huge element. The offset is that of the line nearest the
 * corner, not of the canvas's edge: the webview places backgrounds in fixed point (1/64 px),
 * and a fractional cell repeated from an edge thousands of cells away drifts off the pixels.
 */
export function pixelGrid(
  view: View,
  dpr: number,
  canvas: { width: number; height: number },
  viewWidth: number,
  viewHeight: number,
): {
  left: number;
  top: number;
  width: number;
  height: number;
  cell: number;
  offsetX: number;
  offsetY: number;
} | null {
  if (view.zoom < PIXEL_GRID_ZOOM) return null;
  const [x0, y0] = toViewport(view, dpr, 0, 0);
  const [x1, y1] = toViewport(view, dpr, canvas.width, canvas.height);
  const left = Math.max(x0, 0);
  const top = Math.max(y0, 0);
  const right = Math.min(x1, viewWidth);
  const bottom = Math.min(y1, viewHeight);
  if (right <= left || bottom <= top) return null;
  const cell = view.zoom / dpr;
  // The canvas's edge is at or before the corner: keep the line within one cell of it.
  const phase = (edge: number) => (edge % cell === 0 ? 0 : edge % cell);
  return {
    left,
    top,
    width: right - left,
    height: bottom - top,
    cell,
    offsetX: phase(x0 - left),
    offsetY: phase(y0 - top),
  };
}
