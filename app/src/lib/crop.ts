// The Crop tool's frame (ADR 0017): its proportions, from the options bar's ratio or size, or
// Shift on a corner. Frames are document pixels; the tool rounds them to whole pixels.

import type { Bounds } from "./engine";

/**
 * The options bar's constraint on the frame: none, a ratio (`width:height`, any unit), or a size
 * in pixels (the frame keeps it: cropping never resamples, so the size is the frame's own).
 */
export type CropAspect =
  | { mode: "free" }
  | { mode: "ratio"; width: number; height: number }
  | { mode: "size"; width: number; height: number };

/** The options bar's presets, Photoshop's. */
export const CROP_RATIOS: readonly [number, number][] = [
  [1, 1],
  [4, 5],
  [5, 7],
  [2, 3],
  [16, 9],
];

/** The width over the height `aspect` keeps, if any. */
export function aspectRatio(aspect: CropAspect): number | null {
  if (aspect.mode === "free" || aspect.width <= 0 || aspect.height <= 0) return null;
  return aspect.width / aspect.height;
}

/** The largest frame of `ratio` (width over height) centered in `b`, in whole pixels. */
export function fitRatio(b: Bounds, ratio: number): Bounds {
  const [width, height] = [b.right - b.left, b.bottom - b.top];
  const [w, h] = width / height > ratio ? [height * ratio, height] : [width, width / ratio];
  return centered(b, Math.max(1, Math.round(w)), Math.max(1, Math.round(h)));
}

/** A `width × height` frame with the center of `b` (whole pixels). */
export function centered(b: Bounds, width: number, height: number): Bounds {
  const left = Math.round((b.left + b.right - width) / 2);
  const top = Math.round((b.top + b.bottom - height) / 2);
  return { left, top, right: left + width, bottom: top + height };
}

/**
 * `b`, the frame as the drag puts it (edges not yet in order), kept at `ratio`. A corner (even
 * `handle`) or drawing (`handle` null): the corner dragged follows the pointer, the opposite one
 * stays, the side going further sets the size. A side handle: the other side follows, centered
 * on the frame as it was (`start`).
 */
export function keepRatio(b: Bounds, start: Bounds, handle: number | null, ratio: number): Bounds {
  const out = { ...b };
  if (handle !== null && handle % 2 === 1) {
    if (handle === 3 || handle === 7) {
      const height = Math.abs(b.right - b.left) / ratio;
      const middle = (start.top + start.bottom) / 2;
      out.top = middle - height / 2;
      out.bottom = middle + height / 2;
    } else {
      const width = Math.abs(b.bottom - b.top) * ratio;
      const middle = (start.left + start.right) / 2;
      out.left = middle - width / 2;
      out.right = middle + width / 2;
    }
    return out;
  }
  // The dragged corner's edges: the right and bottom ones when drawing.
  const drawing = handle === null;
  const xEdge = !drawing && (handle === 0 || handle === 6) ? "left" : "right";
  const yEdge = !drawing && (handle === 0 || handle === 2) ? "top" : "bottom";
  const ax = xEdge === "left" ? b.right : b.left;
  const ay = yEdge === "top" ? b.bottom : b.top;
  const size = Math.max(Math.abs(b[xEdge] - ax), Math.abs(b[yEdge] - ay) * ratio);
  out[xEdge] = ax + Math.sign(b[xEdge] - ax || 1) * size;
  out[yEdge] = ay + Math.sign(b[yEdge] - ay || 1) * (size / ratio);
  return out;
}

/**
 * Straighten: the turn (degrees, clockwise) that makes the line drawn from `from` to `to`
 * (document pixels) level, or upright when it is nearer to vertical; null for a line too short
 * to tell, or already straight.
 */
export function straightenTurn(from: [number, number], to: [number, number]): number | null {
  const [dx, dy] = [to[0] - from[0], to[1] - from[1]];
  if (Math.hypot(dx, dy) < 2) return null;
  // The line's angle from the horizontal, clockwise on screen, within (-90°, 90°].
  let angle = (Math.atan2(dy, dx) * 180) / Math.PI;
  if (angle > 90) angle -= 180;
  if (angle <= -90) angle += 180;
  // Nearer to vertical: made upright instead.
  const turn = Math.abs(angle) > 45 ? Math.sign(angle) * 90 - angle : -angle;
  return Math.abs(turn) < 1e-6 ? null : turn;
}

/**
 * After a `width × height` canvas turned by `degrees` (the canvas grown to `grown` around it,
 * centered): the largest frame of the canvas's proportions inside the turned image, so that no
 * transparent corner is kept; whole pixels, inward.
 */
export function insetAfterTurn(
  width: number,
  height: number,
  degrees: number,
  grown: { width: number; height: number },
): Bounds {
  const radians = (degrees * Math.PI) / 180;
  const [cos, sin] = [Math.abs(Math.cos(radians)), Math.abs(Math.sin(radians))];
  const scale = Math.min(
    width / (width * cos + height * sin),
    height / (width * sin + height * cos),
  );
  const [w, h] = [width * scale, height * scale];
  const [cx, cy] = [grown.width / 2, grown.height / 2];
  return {
    left: Math.ceil(cx - w / 2),
    top: Math.ceil(cy - h / 2),
    right: Math.floor(cx + w / 2),
    bottom: Math.floor(cy + h / 2),
  };
}
