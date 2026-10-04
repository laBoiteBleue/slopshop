// The Histogram panel's math (ADR 0036): the channel shown, its statistics as Photoshop gives
// them (mean, standard deviation, median, pixel count) and the outline of its curve.

import type { HistogramView } from "./engine";

/** What the panel shows: the three colors over each other, or one channel. */
export type HistogramChannel = "colors" | "luminosity" | "red" | "green" | "blue";

export const HISTOGRAM_CHANNELS: HistogramChannel[] = [
  "colors",
  "luminosity",
  "red",
  "green",
  "blue",
];

/** The counts per value (0–255) behind `channel`; Colors adds the three colors up. */
export function channelCounts(view: HistogramView, channel: HistogramChannel): number[] {
  if (channel !== "colors") return view[channel];
  return view.red.map((r, i) => r + view.green[i] + view.blue[i]);
}

export type HistogramStats = { mean: number; stdDev: number; median: number; pixels: number };

/** `counts`' statistics; `null` when nothing was counted. */
export function histogramStats(counts: number[]): HistogramStats | null {
  const total = counts.reduce((sum, c) => sum + c, 0);
  if (total <= 0) return null;
  const mean = counts.reduce((sum, c, v) => sum + c * v, 0) / total;
  const variance = counts.reduce((sum, c, v) => sum + c * (v - mean) ** 2, 0) / total;
  let below = 0;
  let median = 255;
  for (let v = 0; v < counts.length; v++) {
    below += counts[v];
    if (below >= total / 2) {
      median = v;
      break;
    }
  }
  return { mean, stdDev: Math.sqrt(variance), median, pixels: total };
}

/**
 * The outline of `counts` as an SVG path in a `width × height` box, value 0 on the left, the
 * tallest count reaching the top (`scale`: that count, to draw several channels to one scale).
 */
export function histogramPath(
  counts: number[],
  width: number,
  height: number,
  scale = Math.max(...counts),
): string {
  if (!(scale > 0)) return "";
  const step = width / counts.length;
  const points = counts.map((c, v) => {
    const y = height - (Math.min(c, scale) / scale) * height;
    return `L${(v * step).toFixed(2)},${y.toFixed(2)}L${((v + 1) * step).toFixed(2)},${y.toFixed(2)}`;
  });
  return `M0,${height}${points.join("")}L${width},${height}Z`;
}
