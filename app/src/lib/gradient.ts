// Gradient Map's gradient, as the editor changes it: stops `[location, r, g, b]`, the location
// 0–4096 (Photoshop's steps), the color sRGB 0–255, locations never decreasing (two stops at
// one place make a hard edge). The engine's slopshop_core::gradient interpolates the same way.
//
// No imports: this module also runs under Node for its tests.

/** `[location 0–4096, r, g, b]`. */
export type Stop = [number, number, number, number];

/** The locations of a gradient: 0 to this. */
export const LOCATIONS = 4096;
/** Stops at most (GRADIENT_STOPS in the engine). */
export const MAX_STOPS = 16;
/** Stops at least. */
export const MIN_STOPS = 2;

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi);

/** The color at `location`: linear between the stops around it, the end stops' outside. */
export function colorAt(stops: readonly Stop[], location: number): [number, number, number] {
  const first = stops[0];
  const last = stops[stops.length - 1];
  if (location <= first[0]) return [first[1], first[2], first[3]];
  for (let i = 1; i < stops.length; i++) {
    const [a, b] = [stops[i - 1], stops[i]];
    if (location <= b[0]) {
      const k = b[0] > a[0] ? (location - a[0]) / (b[0] - a[0]) : 1;
      return [1, 2, 3].map((c) => Math.round(a[c] + (b[c] - a[c]) * k)) as [number, number, number];
    }
  }
  return [last[1], last[2], last[3]];
}

/** The gradient as a CSS `linear-gradient` from left to right. */
export function cssGradient(stops: readonly Stop[]): string {
  const parts = stops.map(
    ([location, r, g, b]) => `rgb(${r}, ${g}, ${b}) ${((location / LOCATIONS) * 100).toFixed(2)}%`,
  );
  return `linear-gradient(to right, ${parts.join(", ")})`;
}

/**
 * A stop added at `location` (rounded, clamped) with the color the gradient has there, and its
 * index; null when the gradient has `MAX_STOPS` already.
 */
export function addStop(
  stops: readonly Stop[],
  location: number,
): { stops: Stop[]; index: number } | null {
  if (stops.length >= MAX_STOPS) return null;
  const at = Math.round(clamp(location, 0, LOCATIONS));
  const stop: Stop = [at, ...colorAt(stops, at)];
  // After the stops at the same place: the new one shows to the right of a hard edge.
  let index = stops.findIndex((s) => s[0] > at);
  if (index < 0) index = stops.length;
  return { stops: [...stops.slice(0, index), stop, ...stops.slice(index)], index };
}

/** Stop `index` moved to `location`, kept between its neighbours (rounded). */
export function moveStop(stops: readonly Stop[], index: number, location: number): Stop[] {
  const lo = index > 0 ? stops[index - 1][0] : 0;
  const hi = index < stops.length - 1 ? stops[index + 1][0] : LOCATIONS;
  return stops.map((s, i) =>
    i === index ? [Math.round(clamp(location, lo, hi)), s[1], s[2], s[3]] : s,
  );
}

/** Stop `index` removed; null when only `MIN_STOPS` are left. */
export function removeStop(stops: readonly Stop[], index: number): Stop[] | null {
  if (stops.length <= MIN_STOPS) return null;
  return stops.filter((_, i) => i !== index);
}

/** Stop `index` with `color` (sRGB 0–255). */
export function recolorStop(
  stops: readonly Stop[],
  index: number,
  [r, g, b]: readonly number[],
): Stop[] {
  return stops.map((s, i) => (i === index ? [s[0], r, g, b] : s));
}
