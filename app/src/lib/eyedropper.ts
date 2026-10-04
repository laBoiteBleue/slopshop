// The eyedropper, wherever the image is sampled (the color picker, Select > Color Range): its
// pointer, and the loupe that magnifies the pixels around it for precision.

/** What a click samples: a color, one more, or one taken away (Color Range's three). */
export type EyedropperKind = "pick" | "add" | "subtract";

/** The dropper drawn as the toolbar's icon, its tip at the bottom left; the sign at the top left. */
const DROPPER = "M14.5 4.5l5 5 M17 2.5l4.5 4.5-3 3-4.5-4.5Z M15.5 8.5L6 18l-2 2";
const SIGNS: Record<EyedropperKind, string> = {
  pick: "",
  add: " M5 3v6 M2 6h6",
  subtract: " M2 6h6",
};
/** The tip, where the sample is taken. */
const HOTSPOT = [4, 20];

/** The CSS cursor of the eyedropper `kind`: white over black, readable on any image. */
export function eyedropperCursor(kind: EyedropperKind): string {
  const d = DROPPER + SIGNS[kind];
  const svg =
    "<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24' fill='none'" +
    " stroke-linecap='round' stroke-linejoin='round'>" +
    `<path d='${d}' stroke='black' stroke-width='3.5'/>` +
    `<path d='${d}' stroke='white' stroke-width='1.5'/></svg>`;
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
/** Space between the pointer and the loupe, past the eyedropper's 24 px. */
const GAP = 24;

/**
 * Where the loupe goes for the pointer at (`x`, `y`) in a `width × height` window: above and to
 * the right, clear of the pointer and of what is left of and below it; on the other side where
 * the window ends.
 */
export function loupePlacement(
  x: number,
  y: number,
  view: { width: number; height: number },
  size = LOUPE_SIZE,
): { left: number; top: number } {
  let left = x + GAP;
  if (left + size > view.width) left = x - GAP - size;
  let top = y - GAP - size;
  if (top < 0) top = y + GAP;
  const clamp = (v: number, max: number) => Math.max(0, Math.min(v, max));
  return { left: clamp(left, view.width - size), top: clamp(top, view.height - size) };
}

/**
 * The sampled pixel's color in a loupe patch (`LOUPE_SIDE²` RGBA, straight alpha) as CSS, or null
 * where nothing is shown there.
 */
export function centerColor(patch: Uint8ClampedArray): string | null {
  const i = (LOUPE_RADIUS * LOUPE_SIDE + LOUPE_RADIUS) * 4;
  if (patch.length < i + 4 || patch[i + 3] === 0) return null;
  return `rgb(${patch[i]} ${patch[i + 1]} ${patch[i + 2]})`;
}
