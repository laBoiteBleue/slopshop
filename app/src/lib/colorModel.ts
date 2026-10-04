// Color math of the color picker (ADR 0027): HSB, sRGB and CIE Lab as Photoshop shows them.
// Display and input only: what the engine receives is the sRGB-encoded color, which it
// converts to its working space explicitly. Lab is relative to D50, as Photoshop's.

/** sRGB-encoded components in [0, 1]. */
export type Rgb = [number, number, number];
/** Hue in degrees [0, 360), saturation and brightness in [0, 1]. */
export type Hsb = [number, number, number];
/** L in [0, 100], a and b in about [-128, 127]. */
export type Lab = [number, number, number];

export function hsbToRgb([h, s, v]: Hsb): Rgb {
  const f = (n: number) => {
    const k = (n + h / 60) % 6;
    return v - v * s * Math.max(0, Math.min(k, 4 - k, 1));
  };
  return [f(5), f(3), f(1)];
}

/** `hue` is kept when the color has none (grays), so that a slider does not jump. */
export function rgbToHsb([r, g, b]: Rgb, hue = 0): Hsb {
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  let h = hue;
  if (d > 0) {
    if (max === r) h = 60 * (((g - b) / d) % 6);
    else if (max === g) h = 60 * ((b - r) / d + 2);
    else h = 60 * ((r - g) / d + 4);
    if (h < 0) h += 360;
  }
  return [h, max === 0 ? 0 : d / max, max];
}

function decode(c: number): number {
  return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
}

function encode(c: number): number {
  return c <= 0.0031308 ? c * 12.92 : 1.055 * Math.pow(c, 1 / 2.4) - 0.055;
}

// Linear sRGB → XYZ, adapted from D65 to D50 (Bradford): the matrix of the ICC sRGB profile.
const TO_XYZ_D50 = [
  [0.4360747, 0.3850649, 0.1430804],
  [0.2225045, 0.7168786, 0.0606169],
  [0.0139322, 0.0971045, 0.7141733],
];
const FROM_XYZ_D50 = [
  [3.1338561, -1.6168667, -0.4906146],
  [-0.9787684, 1.9161415, 0.033454],
  [0.0719453, -0.2289914, 1.4052427],
];
const WHITE_D50 = [0.9642, 1, 0.8249];
const EPSILON = 216 / 24389;
const KAPPA = 24389 / 27;

function mul(m: number[][], v: number[]): [number, number, number] {
  return [0, 1, 2].map((i) => m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2]) as [
    number,
    number,
    number,
  ];
}

export function rgbToLab(rgb: Rgb): Lab {
  const xyz = mul(TO_XYZ_D50, rgb.map(decode));
  const f = xyz.map((v, i) => {
    const t = v / WHITE_D50[i];
    return t > EPSILON ? Math.cbrt(t) : (KAPPA * t + 16) / 116;
  });
  return [116 * f[1] - 16, 500 * (f[0] - f[1]), 200 * (f[1] - f[2])];
}

/** sRGB of `lab`, clipped to the sRGB gamut (and whether it had to be). */
export function labToRgb([l, a, b]: Lab): { rgb: Rgb; clipped: boolean } {
  const fy = (l + 16) / 116;
  const fx = fy + a / 500;
  const fz = fy - b / 200;
  const inverse = (f: number) => (f ** 3 > EPSILON ? f ** 3 : (116 * f - 16) / KAPPA);
  const xyz = [inverse(fx), l > KAPPA * EPSILON ? fy ** 3 : l / KAPPA, inverse(fz)].map(
    (v, i) => v * WHITE_D50[i],
  );
  const linear = mul(FROM_XYZ_D50, xyz);
  const clipped = linear.some((c) => c < -1e-4 || c > 1 + 1e-4);
  return {
    rgb: linear.map((c) => encode(Math.min(Math.max(c, 0), 1))) as Rgb,
    clipped,
  };
}

/**
 * The gray a color paints in a mask (Quick Mask, a layer's mask), in [0, 1]: the sRGB encoding
 * of its luminance, as the engine's `gray_of_srgb`.
 */
export function grayOf([r, g, b]: Rgb): number {
  return encode(0.2126 * decode(r) + 0.7152 * decode(g) + 0.0722 * decode(b));
}

export function rgbToHex(rgb: Rgb): string {
  return `#${rgb
    .map((c) =>
      Math.round(Math.min(Math.max(c, 0), 1) * 255)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

/** `#rrggbb` or `rrggbb` (3-digit forms too), or null. */
export function hexToRgb(hex: string): Rgb | null {
  let digits = hex.trim().replace(/^#/, "");
  if (/^[0-9a-f]{3}$/i.test(digits)) digits = [...digits].map((d) => d + d).join("");
  if (!/^[0-9a-f]{6}$/i.test(digits)) return null;
  const n = parseInt(digits, 16);
  return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255];
}
