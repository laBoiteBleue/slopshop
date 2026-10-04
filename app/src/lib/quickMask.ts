// Masks are painted in grays (Quick Mask, ADR 0024, and a layer's mask): white selects or shows,
// black leaves out or hides, as in Photoshop. They have a pair of gray colors of their own, so
// that the drawing colors come back afterwards. Quick Mask's overlay opacity is an app
// preference.

/** The foreground and background colors, `#rrggbb` sRGB. */
export type ColorPair = { foreground: string; background: string };

/** Photoshop's default colors (D): black paints, so the Brush masks or hides. */
export const MASK_COLORS: ColorPair = { foreground: "#000000", background: "#ffffff" };

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
