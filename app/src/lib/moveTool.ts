// The Move tool's decisions (ADR 0027), as in Photoshop: what a drag or an arrow moves (the
// layers, the selected pixels, or with a selection tool the outline alone), and where a drag
// lands (snapped, whole pixels).

import type { Bounds, LayerView } from "./engine";
import { snapMove, type SmartGuide, type SnapTarget } from "./snap";
import { isSelectionTool, type ToolId } from "./tools";

/** What moving selected pixels takes: the active layer's pixels, or its mask when targeted. */
export type PixelTarget = { target: "layer" | "mask"; layerId: number };

/**
 * The pixels of `layer` (its mask when `paintsMask`), or why they cannot move: a mask, or a
 * pixel layer, is needed, and it must be visible.
 */
export function pixelTarget(
  layer: LayerView | null,
  paintsMask: boolean,
): PixelTarget | { error: "move.needRaster" | "move.hidden" } {
  const target = paintsMask ? "mask" : "layer";
  if (!layer || (target === "layer" && layer.kind !== "raster")) {
    return { error: "move.needRaster" };
  }
  if (!layer.visible) return { error: "move.hidden" };
  return { target, layerId: layer.id };
}

/**
 * What an arrow moves: with a selection (not in Quick Mask, which hides the outline), a
 * selection tool moves the outline alone and the Move tool the selected pixels; else the layers.
 */
export function nudged(tool: ToolId, selection: boolean): "outline" | "pixels" | "layers" {
  if (!selection) return "layers";
  if (isSelectionTool(tool)) return "outline";
  return tool === "move" ? "pixels" : "layers";
}

/**
 * Where a drag by `raw` (document pixels, since it began) puts what moves: snapped to the
 * canvas and `others` when `moving` (its bounds) is known and snapping is on, then whole pixels.
 */
export function landing(
  raw: { x: number; y: number },
  moving: Bounds | null,
  targets: SnapTarget[],
  threshold: number,
): { x: number; y: number; guides: SmartGuide[] } {
  let { x, y } = raw;
  let guides: SmartGuide[] = [];
  if (moving) {
    const snapped = snapMove(moving, x, y, targets, threshold);
    ({ x, y } = snapped);
    guides = snapped.guides;
  }
  return { x: Math.round(x), y: Math.round(y), guides };
}

/** A drag constrained to one axis (Shift): along the one it went furthest. */
export function alongAxis(raw: { x: number; y: number }): { x: number; y: number } {
  return Math.abs(raw.x) >= Math.abs(raw.y) ? { x: raw.x, y: 0 } : { x: 0, y: raw.y };
}
