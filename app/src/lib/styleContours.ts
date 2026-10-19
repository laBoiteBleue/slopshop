// Layer style contours in the UI (ADR 0032): Photoshop's contours as the engine lists them
// (`contour_presets`, loaded once), which one a contour is, and its thumbnail's path. The engine
// draws the curves (their samples); these rules only show and pick them.

import { engine, type ContourPreset, type StyleContour } from "./engine";

/** Photoshop's contours, in its picker's order (`ContourPreset::id` in contour.rs). */
export const CONTOUR_PRESET_IDS = [
  "linear",
  "cone",
  "coneInverted",
  "coveDeep",
  "coveShallow",
  "gaussian",
  "halfRound",
  "ring",
  "ringDouble",
  "rollingSlopeDescending",
  "roundedSteps",
  "sawtooth",
] as const;

export type ContourPresetId = (typeof CONTOUR_PRESET_IDS)[number];

let loading: Promise<ContourPreset[]> | null = null;

/** The presets, asked of the engine once; none when it cannot answer (asked again later). */
export function contourPresets(): Promise<ContourPreset[]> {
  loading ??= engine.contourPresets().then(
    (presets) => presets ?? [],
    () => {
      loading = null;
      return [];
    },
  );
  return loading;
}

/** Forgets the presets loaded (tests). */
export function forgetContourPresets() {
  loading = null;
}

/** Whether two contours go through the same points. */
export function sameContour(a: StyleContour, b: StyleContour): boolean {
  return (
    a.points.length === b.points.length &&
    a.points.every((p, i) => p.every((v, j) => v === b.points[i][j]))
  );
}

/**
 * The preset `contour` is among `presets` (none: a custom contour, or the presets not loaded);
 * an absent contour is the effect's default, `absent`.
 */
export function presetOf(
  contour: StyleContour | undefined,
  absent: ContourPresetId,
  presets: ContourPreset[],
): ContourPresetId | null {
  if (!contour) return absent;
  const preset = presets.find((p) => sameContour(p.contour, contour));
  return preset && isPresetId(preset.id) ? preset.id : null;
}

function isPresetId(id: string): id is ContourPresetId {
  return (CONTOUR_PRESET_IDS as readonly string[]).includes(id);
}

/** The samples to draw for `contour` (absent: preset `absent`'s), if known. */
export function samplesOf(
  contour: StyleContour | undefined,
  absent: ContourPresetId,
  presets: ContourPreset[],
): number[] | null {
  const shown = contour ?? presets.find((p) => p.id === absent)?.contour;
  if (!shown) return null;
  if (shown.samples?.length) return shown.samples;
  return presets.find((p) => sameContour(p.contour, shown))?.contour.samples ?? null;
}

/**
 * An SVG path through `samples` (outputs 0–1 at evenly spaced inputs) in a `size` square: the
 * input from left to right, the output from bottom to top.
 */
export function contourPath(samples: number[], size: number): string {
  if (samples.length < 2) return "";
  const last = samples.length - 1;
  return samples
    .map((v, i) => {
      const x = (i / last) * size;
      const y = (1 - Math.min(1, Math.max(0, v))) * size;
      return `${i === 0 ? "M" : "L"}${round(x)} ${round(y)}`;
    })
    .join(" ");
}

function round(v: number): number {
  return Math.round(v * 100) / 100;
}
