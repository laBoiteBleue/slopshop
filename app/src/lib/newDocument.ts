// The math of File > New (NewDocumentDialog.svelte): sizes are whole pixels, typed in pixels
// or a length at the resolution (ADR 0028).

import { MAX_PPI, MIN_PPI, type LengthUnit } from "./units.ts";
import { MAX_SIDE } from "./sizeDialog.ts";

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
 * The size once the resolution goes from `ppi` to `next`: a size typed in a length unit keeps
 * its length, so its pixels follow (Photoshop); in pixels it stays.
 */
export function atResolution(size: Size, unit: LengthUnit, ppi: number, next: number): Size {
  if (unit === "px") return size;
  return {
    width: Math.round((size.width * next) / ppi),
    height: Math.round((size.height * next) / ppi),
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
