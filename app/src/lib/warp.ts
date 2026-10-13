// Edit > Transform > Warp (ADR 0044): the options bar's settings, the request they send, and the
// grid's lines as the overlay draws them. The engine makes the grid (slopshop_core::warp).

import type { EditRequest, WarpGrid, WarpStyle } from "./engine";

/** Warp's styles, in the options bar's order (Photoshop's, those SlopShop has). */
export const WARP_STYLES: readonly WarpStyle[] = ["arc", "bulge", "flag", "wave", "twist"];

/** Photoshop's bend when a style is first chosen, in %. */
export const DEFAULT_BEND = 50;

/** What the options bar shows: the style (null: None), the bend in % and the orientation. */
export type WarpSettings = { style: WarpStyle | null; bend: number; vertical: boolean };

/** The settings a layer's grid was made from (a layer without a warp: None, bend 50 %). */
export function settingsOf(grid: WarpGrid): WarpSettings {
  if (!grid.style) return { style: null, bend: DEFAULT_BEND, vertical: false };
  return { style: grid.style, bend: Math.round(grid.bend * 1000) / 10, vertical: grid.vertical };
}

/** A bend in %, within -100 to 100 (not a number: 0). */
export function clampBend(bend: number): number {
  if (!Number.isFinite(bend)) return 0;
  return Math.min(100, Math.max(-100, bend));
}

/** The request that warps layer `id` with `settings` (None takes its warp away). */
export function warpRequest(id: number, settings: WarpSettings): EditRequest {
  if (!settings.style) return { kind: "warpLayer", id };
  return {
    kind: "warpLayer",
    id,
    style: settings.style,
    bend: clampBend(settings.bend) / 100,
    vertical: settings.vertical,
  };
}

/** Whether two settings warp alike (None is None whatever its bend). */
export function sameWarp(a: WarpSettings, b: WarpSettings): boolean {
  if (!a.style || !b.style) return a.style === b.style;
  return a.style === b.style && a.bend === b.bend && a.vertical === b.vertical;
}

/** An SVG path through a line's points (document pixels), mapped to the viewport. */
export function linePath(
  line: [number, number][],
  toViewport: (x: number, y: number) => [number, number],
): string {
  return line
    .map(([x, y], i) => {
      const [vx, vy] = toViewport(x, y);
      return `${i === 0 ? "M" : "L"}${vx.toFixed(2)} ${vy.toFixed(2)}`;
    })
    .join(" ");
}
