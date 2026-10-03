// The edits of the Image menu and the Crop tool, from what the dialogs and the frame give:
// none when nothing would change (OK without a change leaves nothing to undo, as in Photoshop).

import type { AutoCorrection, Bounds, EditRequest } from "./engine";

type Size = { width: number; height: number };

/** The whole canvas of a document `size` pixels large. */
export function canvasBounds(size: Size): Bounds {
  return { left: 0, top: 0, right: size.width, bottom: size.height };
}

/** Document point (`x`, `y`) is outside a canvas of `size`. */
export function outsideCanvas(size: Size, x: number, y: number): boolean {
  return x < 0 || y < 0 || x >= size.width || y >= size.height;
}

/**
 * Image Size (the image resampled, at `resolution` pixels per inch) or Canvas Size (around
 * `anchor`) for `doc`, and whether its size changes; null when nothing changes.
 */
export function sizeEdit(
  doc: Size & { resolution: number },
  mode: "image" | "canvas",
  next: Size,
  anchor: [number, number],
  resolution: number,
): { request: EditRequest; resized: boolean } | null {
  const resized = next.width !== doc.width || next.height !== doc.height;
  if (!resized && (mode === "canvas" || resolution === doc.resolution)) return null;
  const { width, height } = next;
  return {
    request:
      mode === "image"
        ? { kind: "resizeImage", width, height, resolution }
        : { kind: "canvasSize", width, height, anchor },
    resized,
  };
}

/** The crop to `frame` of a canvas of `size`; null for the whole canvas. */
export function cropEdit(size: Size, frame: Bounds): EditRequest | null {
  const whole =
    frame.left === 0 &&
    frame.top === 0 &&
    frame.right === size.width &&
    frame.bottom === size.height;
  if (whole) return null;
  return {
    kind: "crop",
    x: frame.left,
    y: frame.top,
    width: frame.right - frame.left,
    height: frame.bottom - frame.top,
  };
}

/**
 * Image > Auto Tone, Auto Contrast or Auto Color on the layers `ids` (every pixel layer shown):
 * the engine analyzes the visible image; null without a layer to apply it to.
 */
export function autoLevelsEdit(ids: number[], correction: AutoCorrection): EditRequest | null {
  return ids.length > 0 ? { kind: "autoLevels", ids, correction } : null;
}
