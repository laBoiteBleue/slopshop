// Filter > Liquify's workspace (ADR 0037): its tools and keys, the brush's settings, and the
// geometry of its view (navigated as the rest of the app: middle button pans, the wheel zooms).
// The engine edits the displacement field; the UI only sends strokes and draws the frames.

import type { LiquifyBrush, LiquifyToolId, LiquifyViewRequest } from "./engine";
import type { MessageKey } from "./i18n/en";

/** The tools of the workspace's toolbar, Photoshop's order, with their keys. */
export const LIQUIFY_TOOLS: {
  id: LiquifyToolId;
  key: string;
  label: MessageKey;
  icon:
    | "liquifyForward"
    | "liquifyReconstruct"
    | "liquifySmooth"
    | "liquifyTwirl"
    | "liquifyPucker"
    | "liquifyBloat"
    | "liquifyPushLeft"
    | "liquifyFreeze"
    | "liquifyThaw";
}[] = [
  { id: "forwardWarp", key: "w", label: "liquify.tool.forwardWarp", icon: "liquifyForward" },
  { id: "reconstruct", key: "r", label: "liquify.tool.reconstruct", icon: "liquifyReconstruct" },
  { id: "smooth", key: "e", label: "liquify.tool.smooth", icon: "liquifySmooth" },
  { id: "twirlClockwise", key: "c", label: "liquify.tool.twirl", icon: "liquifyTwirl" },
  { id: "pucker", key: "s", label: "liquify.tool.pucker", icon: "liquifyPucker" },
  { id: "bloat", key: "b", label: "liquify.tool.bloat", icon: "liquifyBloat" },
  { id: "pushLeft", key: "o", label: "liquify.tool.pushLeft", icon: "liquifyPushLeft" },
  { id: "freeze", key: "f", label: "liquify.tool.freeze", icon: "liquifyFreeze" },
  { id: "thaw", key: "d", label: "liquify.tool.thaw", icon: "liquifyThaw" },
];

/** The tool a key chooses (a letter, any case), or null. */
export function toolForKey(key: string): LiquifyToolId | null {
  const lower = key.toLowerCase();
  return LIQUIFY_TOOLS.find((tool) => tool.key === lower)?.id ?? null;
}

/**
 * The tool at work while Alt is held: Twirl turns the other way, Pucker bloats, Freeze thaws
 * (and back), as in Photoshop. The others do not change.
 */
export function effectiveTool(tool: LiquifyToolId, alt: boolean): LiquifyToolId {
  if (!alt) return tool;
  switch (tool) {
    case "twirlClockwise":
      return "twirlCounterclockwise";
    case "twirlCounterclockwise":
      return "twirlClockwise";
    case "pucker":
      return "bloat";
    case "bloat":
      return "pucker";
    case "freeze":
      return "thaw";
    case "thaw":
      return "freeze";
    default:
      return tool;
  }
}

/** The toolbar button a tool belongs to (the twirls share one). */
export function toolbarTool(tool: LiquifyToolId): LiquifyToolId {
  return tool === "twirlCounterclockwise" ? "twirlClockwise" : tool;
}

/** Photoshop's defaults. */
export const DEFAULT_BRUSH: LiquifyBrush = { size: 100, density: 50, pressure: 100, rate: 80 };

/** The ranges of the brush's settings (the engine's `Brush::is_valid`). */
export const BRUSH_LIMITS = {
  size: { min: 1, max: 15000 },
  density: { min: 0, max: 100 },
  pressure: { min: 1, max: 100 },
  rate: { min: 0, max: 100 },
} as const;

function within(value: number, min: number, max: number): number {
  return Number.isFinite(value) ? Math.min(Math.max(value, min), max) : min;
}

/** `brush` with every setting in its range. */
export function clampBrush(brush: LiquifyBrush): LiquifyBrush {
  const l = BRUSH_LIMITS;
  return {
    size: within(brush.size, l.size.min, l.size.max),
    density: within(brush.density, l.density.min, l.density.max),
    pressure: within(brush.pressure, l.pressure.min, l.pressure.max),
    rate: within(brush.rate, l.rate.min, l.rate.max),
  };
}

/** The brush's size after `[` (`-1`) or `]` (`1`): a tenth of it more or less, at least 1 pixel. */
export function steppedSize(size: number, direction: 1 | -1): number {
  const step = Math.max(1, Math.round(size / 10));
  return within(Math.round(size + direction * step), BRUSH_LIMITS.size.min, BRUSH_LIMITS.size.max);
}

// --- The view ---------------------------------------------------------------------------------

/** A layer point at the canvas's top left corner, and CSS pixels a layer pixel takes. */
export type View = { x: number; y: number; zoom: number };

export const MIN_ZOOM = 0.01;
export const MAX_ZOOM = 32;

/** What the frames hold at most (device pixels): a view larger than this is drawn smaller. */
export const MAX_FRAME_PIXELS = 4_000_000;

type Size = { width: number; height: number };

/** The whole layer in the canvas, centered, at most at 100%, with `margin` pixels around. */
export function fitView(layer: Size, canvas: Size, margin = 16): View {
  const room = {
    width: Math.max(canvas.width - 2 * margin, 1),
    height: Math.max(canvas.height - 2 * margin, 1),
  };
  const zoom = within(
    Math.min(room.width / layer.width, room.height / layer.height, 1),
    MIN_ZOOM,
    MAX_ZOOM,
  );
  return {
    zoom,
    x: layer.width / 2 - canvas.width / 2 / zoom,
    y: layer.height / 2 - canvas.height / 2 / zoom,
  };
}

/** The layer point under canvas point (`px`, `py`), CSS pixels from its top left. */
export function toLayer(view: View, px: number, py: number): [number, number] {
  return [view.x + px / view.zoom, view.y + py / view.zoom];
}

/** Where layer point (`x`, `y`) is in the canvas, CSS pixels from its top left. */
export function toCanvas(view: View, x: number, y: number): [number, number] {
  return [(x - view.x) * view.zoom, (y - view.y) * view.zoom];
}

/** The view zoomed by `factor` about canvas point (`px`, `py`), which keeps its layer point. */
export function zoomAt(view: View, factor: number, px: number, py: number): View {
  const zoom = within(view.zoom * factor, MIN_ZOOM, MAX_ZOOM);
  const [x, y] = toLayer(view, px, py);
  return { zoom, x: x - px / zoom, y: y - py / zoom };
}

/** The view dragged by (`dx`, `dy`) canvas pixels: the layer follows the pointer. */
export function panned(view: View, dx: number, dy: number): View {
  return { ...view, x: view.x - dx / view.zoom, y: view.y - dy / view.zoom };
}

/** How much a wheel delta (in pixels, positive down) zooms: a notch of 100 pixels is 22%. */
export function wheelZoom(deltaPixels: number): number {
  return Math.exp(-deltaPixels * 0.002);
}

/** The device pixels per CSS pixel the frames are drawn at: the screen's, within the cap. */
export function frameScale(canvas: Size, dpr: number): number {
  const pixels = canvas.width * canvas.height;
  if (pixels <= 0) return dpr;
  return Math.min(dpr, Math.sqrt(MAX_FRAME_PIXELS / pixels));
}

/** What the engine is asked to draw for `view` in a canvas of `canvas` CSS pixels. */
export function frameRequest(
  view: View,
  canvas: Size,
  dpr: number,
  overlay: boolean,
): LiquifyViewRequest {
  const scale = frameScale(canvas, dpr);
  return {
    x: view.x,
    y: view.y,
    zoom: view.zoom * scale,
    width: Math.max(1, Math.round(canvas.width * scale)),
    height: Math.max(1, Math.round(canvas.height * scale)),
    overlay,
  };
}

/** The zoom shown as a percentage. */
export function zoomPercent(view: View): string {
  const percent = view.zoom * 100;
  return `${percent >= 10 ? Math.round(percent) : Number(percent.toFixed(1))}%`;
}
