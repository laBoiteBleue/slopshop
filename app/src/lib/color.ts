// Conversions between `<input type="color">` values and the sRGB-encoded components the engine
// takes (in [0, 1]); the engine converts them to its working space explicitly.

/** `#rrggbb` → sRGB-encoded [r, g, b] in [0, 1]. */
export function hexToSrgb(hex: string): [number, number, number] {
  const n = parseInt(hex.slice(1), 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}

/** sRGB-encoded [r, g, b] in [0, 1] → `#rrggbb`. */
export function srgbToHex(rgb: readonly number[]): string {
  const byte = (c: number) =>
    Math.round(Math.min(Math.max(c, 0), 1) * 255)
      .toString(16)
      .padStart(2, "0");
  return `#${rgb.slice(0, 3).map(byte).join("")}`;
}
