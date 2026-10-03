// Lengths and resolutions as Photoshop shows them (ADR 0028): sizes in pixels, inches,
// centimeters or millimeters at the document's resolution, kept in pixels per inch and shown
// in pixels per inch or per centimeter.

export type LengthUnit = "px" | "in" | "cm" | "mm";
export type ResolutionUnit = "ppi" | "ppcm";

export const LENGTH_UNITS: LengthUnit[] = ["px", "in", "cm", "mm"];
export const RESOLUTION_UNITS: ResolutionUnit[] = ["ppi", "ppcm"];

/** Units per inch of each physical unit. */
const PER_INCH: Record<Exclude<LengthUnit, "px">, number> = { in: 1, cm: 2.54, mm: 25.4 };

/** `pixels` in `unit` at `ppi` pixels per inch. */
export function fromPixels(pixels: number, unit: LengthUnit, ppi: number): number {
  return unit === "px" ? pixels : (pixels / ppi) * PER_INCH[unit];
}

/** A length in `unit` as pixels at `ppi` (not rounded). */
export function toPixels(value: number, unit: LengthUnit, ppi: number): number {
  return unit === "px" ? value : (value / PER_INCH[unit]) * ppi;
}

/** A resolution in pixels per inch, shown in `unit`. */
export function fromPpi(ppi: number, unit: ResolutionUnit): number {
  return unit === "ppi" ? ppi : ppi / 2.54;
}

/** A resolution shown in `unit`, as pixels per inch. */
export function toPpi(value: number, unit: ResolutionUnit): number {
  return unit === "ppi" ? value : value * 2.54;
}

/** A field's value, rounded as Photoshop shows it for its unit. */
export function rounded(value: number, unit: LengthUnit | ResolutionUnit | "percent"): number {
  const digits =
    unit === "px" ? 0 : unit === "mm" || unit === "percent" ? 1 : unit === "in" ? 3 : 2;
  const scale = 10 ** digits;
  return Math.round(value * scale) / scale;
}

/** Resolutions accepted, pixels per inch (as the engine's). */
export const MIN_PPI = 1;
export const MAX_PPI = 100_000;
