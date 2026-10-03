// The math of File > New (NewDocumentDialog.svelte): sizes are whole pixels, typed in pixels
// or a length at the resolution (ADR 0028).

import { MAX_PPI, MIN_PPI, toPixels, type LengthUnit } from "./units";
import { MAX_SIDE } from "./sizeDialog";

type Size = { width: number; height: number };

/** The preset of `size`, either way round, or "custom". */
export function matchingPreset(presets: (Size & { id: string })[], size: Size): string {
  return (
    presets.find(
      (p) =>
        (p.width === size.width && p.height === size.height) ||
        (p.width === size.height && p.height === size.width),
    )?.id ?? "custom"
  );
}

/**
 * The size in pixels once the resolution is `next` pixels per inch: in pixels it stays
 * (`size`); in a length unit the length shown (`lengths`) stays and its pixels follow
 * (Photoshop). From the lengths rather than the pixels: typing a resolution digit by digit
 * (1, 15, 150) would otherwise round the pixels at each step.
 */
export function atResolution(size: Size, lengths: Size, unit: LengthUnit, next: number): Size {
  if (unit === "px" || !Number.isFinite(lengths.width) || !Number.isFinite(lengths.height)) {
    return size;
  }
  return {
    width: Math.round(toPixels(lengths.width, unit, next)),
    height: Math.round(toPixels(lengths.height, unit, next)),
  };
}

/** `size` turned to portrait or landscape (a square stays). */
export function oriented(size: Size, portrait: boolean): Size {
  return size.height > size.width === portrait || size.width === size.height
    ? size
    : { width: size.height, height: size.width };
}

export function isValidNew(size: Size, ppi: number): boolean {
  return (
    [size.width, size.height].every(
      (side) => Number.isInteger(side) && side >= 1 && side <= MAX_SIDE,
    ) &&
    ppi >= MIN_PPI &&
    ppi <= MAX_PPI
  );
}
