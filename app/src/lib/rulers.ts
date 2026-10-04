// The rulers' graduations (View > Rulers): where a ruler's ticks go for the part of the document
// it shows, in the ruler's unit (pixels, or a length at the document's resolution), a label on
// every major tick, as Photoshop's.

/** A tick: `at` CSS pixels from the ruler's start, its coordinate, labelled if major. */
export type Tick = { at: number; value: number; major: boolean };

/** Labelled ticks are at least this far apart, CSS pixels: room for a label. */
export const MAJOR_MIN_CSS = 60;
/** Ticks are at least this far apart, CSS pixels. */
export const MINOR_MIN_CSS = 6;

/**
 * The distance between labelled ticks, in the ruler's unit: the smallest 1, 2 or 5 × 10ⁿ at
 * least [`MAJOR_MIN_CSS`] apart on screen, `perCss` units being one CSS pixel. `whole` (pixels):
 * never a fraction.
 */
export function majorStep(perCss: number, whole = true): number {
  if (!(perCss > 0) || !Number.isFinite(perCss)) return 1;
  const min = MAJOR_MIN_CSS * perCss;
  let power = whole ? 1 : 10 ** Math.floor(Math.log10(min));
  for (;;) {
    for (const m of [1, 2, 5]) if (m * power >= min * (1 - 1e-12)) return m * power;
    power *= 10;
  }
}

/** The decimals a label needs at `step` between labels: none for whole steps. */
export function labelDigits(step: number): number {
  return step >= 1 ? 0 : Math.ceil(-Math.log10(step) - 1e-9);
}

/**
 * The ticks of a ruler `length` CSS pixels long whose start shows coordinate `start`, at
 * `perCss` units per CSS pixel: each major step divided in 10, 5 or 2 (the finest that keeps
 * ticks [`MINOR_MIN_CSS`] apart), or not at all. `whole`: pixels, never fractions of labels.
 */
export function rulerTicks(start: number, perCss: number, length: number, whole = true): Tick[] {
  const major = majorStep(perCss, whole);
  if (!(perCss > 0) || !Number.isFinite(start) || !(length > 0)) return [];
  const divisions = [10, 5, 2].find((d) => major / d / perCss >= MINOR_MIN_CSS) ?? 1;
  const minor = major / divisions;
  const end = start + length * perCss;
  const ticks: Tick[] = [];
  for (let i = Math.ceil(start / minor); i * minor <= end; i++) {
    // Whole multiples of the major step are exact, whatever the floating point says.
    const isMajor = i % divisions === 0;
    const value = isMajor ? (i / divisions) * major : i * minor;
    ticks.push({ at: (value - start) / perCss, value, major: isMajor });
  }
  return ticks;
}
