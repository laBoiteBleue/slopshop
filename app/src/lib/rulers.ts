// The rulers' graduations (View > Rulers): where a ruler's ticks go for the part of the document
// it shows, in document pixels, a label on every major tick, as Photoshop's.

/** A tick: `at` CSS pixels from the ruler's start, its document coordinate, labelled if major. */
export type Tick = { at: number; value: number; major: boolean };

/** Labelled ticks are at least this far apart, CSS pixels: room for a label. */
export const MAJOR_MIN_CSS = 60;
/** Ticks are at least this far apart, CSS pixels. */
export const MINOR_MIN_CSS = 6;

/**
 * The distance between labelled ticks, in document pixels: the smallest 1, 2 or 5 × 10ⁿ (n ≥ 0,
 * never a fraction of a pixel) at least [`MAJOR_MIN_CSS`] apart on screen.
 */
export function majorStep(docPerCss: number): number {
  if (!(docPerCss > 0) || !Number.isFinite(docPerCss)) return 1;
  const min = MAJOR_MIN_CSS * docPerCss;
  for (let power = 1; ; power *= 10) {
    for (const m of [1, 2, 5]) if (m * power >= min) return m * power;
  }
}

/**
 * The ticks of a ruler `length` CSS pixels long whose start shows document coordinate `start`,
 * at `docPerCss` document pixels per CSS pixel: each major step divided in 10, 5 or 2 (the
 * finest that keeps ticks [`MINOR_MIN_CSS`] apart), or not at all.
 */
export function rulerTicks(start: number, docPerCss: number, length: number): Tick[] {
  const major = majorStep(docPerCss);
  if (!(docPerCss > 0) || !Number.isFinite(start) || !(length > 0)) return [];
  const divisions = [10, 5, 2].find((d) => major / d / docPerCss >= MINOR_MIN_CSS) ?? 1;
  const minor = major / divisions;
  const end = start + length * docPerCss;
  const ticks: Tick[] = [];
  for (let i = Math.ceil(start / minor); i * minor <= end; i++) {
    const value = i * minor;
    ticks.push({ at: (value - start) / docPerCss, value, major: i % divisions === 0 });
  }
  return ticks;
}
