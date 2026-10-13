// The Type tool's logic (T, ADR 0041): its options (font, style, size in points, color,
// alignment), the text a click starts, how options change a text, and where the caret stands.
// The engine lays text out; the UI keeps the string being typed and the caret's place in it.
import { hexToSrgb, srgbToHex } from "./color";
import type { TextAlign, TextDto } from "./engine";

/** Photoshop's defaults: 12 pt, left aligned. */
export const DEFAULT_SIZE_PT = 12;
/** Photoshop's range for type sizes, in points. */
export const MIN_SIZE_PT = 0.01;
export const MAX_SIZE_PT = 1296;
/** The engine's largest size, in pixels. */
const MAX_SIZE_PX = 10_000;

/** What the next text is set in, and what changes the text being edited. */
export type TypeOptions = {
  /** Empty: the default family (the bundled one). */
  family: string;
  weight: number;
  italic: boolean;
  /** Points (1/72 inch), turned into pixels by the document's resolution (ADR 0028). */
  sizePt: number;
  /** `#rrggbb`. */
  color: string;
  align: TextAlign;
};

export function defaultTypeOptions(color: string): TypeOptions {
  return {
    family: "",
    weight: 400,
    italic: false,
    sizePt: DEFAULT_SIZE_PT,
    color,
    align: "left",
  };
}

/** Points to pixels at `ppi`. */
export function pointsToPixels(pt: number, ppi: number): number {
  return (pt * ppi) / 72;
}

/** Pixels to points at `ppi`. */
export function pixelsToPoints(px: number, ppi: number): number {
  return (px * 72) / ppi;
}

/** A size in points kept in range, and within what the engine draws at `ppi`. */
export function clampSizePt(pt: number, ppi: number): number {
  if (!Number.isFinite(pt)) return DEFAULT_SIZE_PT;
  const max = Math.min(MAX_SIZE_PT, pixelsToPoints(MAX_SIZE_PX, ppi));
  return Math.min(Math.max(pt, MIN_SIZE_PT), max);
}

/** The text a click at `position` (document pixels) starts: empty, set in `options`. */
export function newText(
  options: TypeOptions,
  position: [number, number],
  ppi: number,
  defaultFamily: string,
): TextDto {
  return withOptions(
    {
      content: "",
      family: defaultFamily,
      weight: 400,
      italic: false,
      size: 1,
      color: [0, 0, 0, 1],
      tracking: 0,
      leading: null,
      align: "left",
      position,
    },
    options,
    ppi,
    defaultFamily,
  );
}

/** `text` set in `options` (the options bar changed while a text layer is active). */
export function withOptions(
  text: TextDto,
  options: TypeOptions,
  ppi: number,
  defaultFamily: string,
): TextDto {
  return {
    ...text,
    family: options.family || defaultFamily,
    weight: options.weight,
    italic: options.italic,
    size: pointsToPixels(clampSizePt(options.sizePt, ppi), ppi),
    color: [...hexToSrgb(options.color), 1],
    align: options.align,
  };
}

/** The options a text layer shows in the options bar. */
export function optionsOf(text: TextDto, ppi: number): TypeOptions {
  return {
    family: text.family,
    weight: text.weight,
    italic: text.italic,
    // Hundredths of a point, as Photoshop shows them.
    sizePt: Math.round(pixelsToPoints(text.size, ppi) * 100) / 100,
    color: srgbToHex(text.color),
    align: text.align,
  };
}

/** The weights' names, Photoshop's, by hundreds. */
const WEIGHT_NAMES = [
  "thin",
  "extraLight",
  "light",
  "regular",
  "medium",
  "semibold",
  "bold",
  "extraBold",
  "black",
] as const;
export type WeightName = (typeof WEIGHT_NAMES)[number];

/** The name of the nearest named weight. */
export function weightName(weight: number): WeightName {
  const index = Math.round(weight / 100) - 1;
  return WEIGHT_NAMES[Math.min(Math.max(index, 0), WEIGHT_NAMES.length - 1)];
}

/** A style's value in the options bar's list: `weight` or `weight italic`. */
export function styleKey(style: { weight: number; italic: boolean }): string {
  return style.italic ? `${style.weight} italic` : `${style.weight}`;
}

/** The style a list value names. */
export function styleOfKey(key: string): { weight: number; italic: boolean } {
  const [weight, italic] = key.split(" ");
  return { weight: Number(weight), italic: italic === "italic" };
}

/** A caret as the engine gives it: top and bottom, in document pixels. */
export type Caret = [number, number, number, number];

/**
 * The caret at `index` (UTF-16 units, as the text field counts them), or the last one when the
 * layout is behind the text being typed.
 */
export function caretAt(carets: readonly Caret[], index: number): Caret | null {
  if (carets.length === 0) return null;
  return carets[Math.min(Math.max(index, 0), carets.length - 1)];
}

/**
 * What a key does while a text is typed (the rest goes to the text): Escape, Ctrl+Enter
 * (Cmd+Enter on macOS) and the numeric keypad's Enter commit it, as in Photoshop; Enter alone
 * is a new line.
 */
export function commitsText(
  e: { key: string; code: string; ctrlKey: boolean; metaKey: boolean },
  mac: boolean,
): boolean {
  if (e.key === "Escape") return true;
  if (e.key !== "Enter") return false;
  return e.code === "NumpadEnter" || (mac ? e.metaKey : e.ctrlKey);
}
