// Quick Mask (Q, ADR 0024): painting edits the selection. White selects and black unselects, as
// in Photoshop, with a pair of colors of its own so that the drawing colors come back on leaving
// it; the options bar shows that pair as Add / Remove, so nobody needs the convention. The
// overlay's opacity is an app preference.

/** The foreground and background colors, `#rrggbb` sRGB. */
export type ColorPair = { foreground: string; background: string };

/** Photoshop's default colors (D): black paints, so the brush removes from the selection. */
export const QUICK_MASK_COLORS: ColorPair = { foreground: "#000000", background: "#ffffff" };

/** What the Brush does to the selection with `colors`: white adds, black removes, a gray does
 * some of either (neither button is on). */
export function quickMaskAction(colors: ColorPair): "add" | "remove" | null {
  const foreground = colors.foreground.toLowerCase();
  if (foreground === "#ffffff") return "add";
  if (foreground === "#000000") return "remove";
  return null;
}

/** The pair that makes the Brush `action` (the other color behind it, for X). */
export function quickMaskColors(action: "add" | "remove"): ColorPair {
  return action === "add"
    ? { foreground: "#ffffff", background: "#000000" }
    : { foreground: "#000000", background: "#ffffff" };
}

const OPACITY_KEY = "slopshop.quickMaskOpacity";
/** Half opaque, as Photoshop. */
export const DEFAULT_QUICK_MASK_OPACITY = 50;

type Store = Pick<Storage, "getItem" | "setItem">;

/** The overlay's opacity in percent, as saved last (storage may be unavailable). */
export function loadQuickMaskOpacity(store?: Store): number {
  try {
    const text = (store ?? localStorage).getItem(OPACITY_KEY);
    const saved = Number(text);
    if (text !== null && Number.isFinite(saved)) {
      return Math.min(Math.max(Math.round(saved), 0), 100);
    }
  } catch {
    // Storage blocked: the default.
  }
  return DEFAULT_QUICK_MASK_OPACITY;
}

export function saveQuickMaskOpacity(opacity: number, store?: Store) {
  try {
    (store ?? localStorage).setItem(OPACITY_KEY, String(Math.round(opacity)));
  } catch {
    // Storage blocked: kept for this session only.
  }
}
