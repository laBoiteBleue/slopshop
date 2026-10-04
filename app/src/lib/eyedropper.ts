// The eyedropper, wherever the image is sampled (the color picker, Select > Color Range): its
// pointer, and the loupe that magnifies the pixels around it for precision.

/** What a click samples: a color, one more, or one taken away (Color Range's three). */
export type EyedropperKind = "pick" | "add" | "subtract";

/**
 * A small cross centered on the sampled pixel, open in its middle so that pixel stays visible;
 * the sign at the top right.
 */
const CROSS = "M8 1v5 M8 11v5 M1 8h5 M11 8h5";
const SIGNS: Record<EyedropperKind, string> = {
  pick: "",
  add: " M18 2v7 M14.5 5.5h7",
  subtract: " M14.5 5.5h7",
};
/** The cross's middle, where the sample is taken. */
const HOTSPOT = [8, 8];

/** The CSS cursor of the eyedropper `kind`: white over black, readable on any image. */
export function eyedropperCursor(kind: EyedropperKind): string {
  const d = CROSS + SIGNS[kind];
  const svg =
    "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24' fill='none'>" +
    `<path d='${d}' stroke='black' stroke-width='3'/>` +
    `<path d='${d}' stroke='white' stroke-width='1'/></svg>`;
  return `url("data:image/svg+xml,${encodeURIComponent(svg)}") ${HOTSPOT[0]} ${HOTSPOT[1]}, crosshair`;
}

/** Color Range's eyedropper at a click: Shift adds, Alt takes away, else the one chosen. */
export function eyedropperFromKeys(
  chosen: EyedropperKind,
  keys: { shiftKey: boolean; altKey: boolean },
): EyedropperKind {
  return keys.shiftKey ? "add" : keys.altKey ? "subtract" : chosen;
}

/** Document pixels shown on each side of the sampled one. */
export const LOUPE_RADIUS = 6;
/** Document pixels a side. */
export const LOUPE_SIDE = 2 * LOUPE_RADIUS + 1;
/** Screen pixels per document pixel. */
export const LOUPE_CELL = 10;
/** The loupe's side, screen pixels. */
export const LOUPE_SIZE = LOUPE_SIDE * LOUPE_CELL;
/** The ring around the pixels: the new color over the current one (Photoshop's sampling ring). */
export const LOUPE_RING = 10;
/** The gray ring around it and its outline, drawn outside it. */
export const LOUPE_HALO = 6;
/** The loupe's side with its rings, screen pixels. */
export const LOUPE_OUTER = LOUPE_SIZE + 2 * (LOUPE_RING + LOUPE_HALO);
/** The new color's value under the loupe: its height with the space above it. */
export const LOUPE_TAG = 28;
/** Space between the pointer and the loupe, past the cross's 24 px. */
const GAP = 16;

/**
 * Where the loupe (its value under it) goes for the pointer at (`x`, `y`) in a `width × height`
 * window: above and to the right, clear of the pointer and of what is left of and below it; on
 * the other side where the window ends.
 */
export function loupePlacement(
  x: number,
  y: number,
  view: { width: number; height: number },
): { left: number; top: number } {
  const [width, height] = [LOUPE_OUTER, LOUPE_OUTER + LOUPE_TAG];
  let left = x + GAP;
  if (left + width > view.width) left = x - GAP - width;
  let top = y - GAP - height;
  if (top < 0) top = y + GAP;
  const clamp = (v: number, max: number) => Math.max(0, Math.min(v, max));
  return { left: clamp(left, view.width - width), top: clamp(top, view.height - height) };
}

/** Where the loupe takes its pixels from. */
export type LoupeSource = {
  /** The document point under a window point; null off the canvas. */
  point: (clientX: number, clientY: number) => [number, number] | null;
  /**
   * The pixels shown around document point (`x`, `y`), `radius` on each side: `(2 × radius + 1)²`
   * RGBA (8-bit sRGB, straight alpha), the one under it in the middle; null if there are none.
   */
  pixels: (x: number, y: number, radius: number) => Promise<Uint8ClampedArray<ArrayBuffer> | null>;
  /** What the pixels show (the document and its revision): a change drops those kept. */
  version: unknown;
};

/**
 * The sampled pixel's color in a loupe patch (`LOUPE_SIDE²` RGBA, straight alpha) as `#rrggbb`, or
 * null where nothing is shown there.
 */
export function centerHex(patch: Uint8ClampedArray): string | null {
  const i = (LOUPE_RADIUS * LOUPE_SIDE + LOUPE_RADIUS) * 4;
  if (patch.length < i + 4 || patch[i + 3] === 0) return null;
  const byte = (v: number) => v.toString(16).padStart(2, "0");
  return `#${byte(patch[i])}${byte(patch[i + 1])}${byte(patch[i + 2])}`;
}
