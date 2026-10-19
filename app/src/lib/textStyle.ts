// Character settings of a text layer (ADR 0041): what the Character settings and the Type
// tool's options bar show for the characters selected, or the whole text, and what a change
// there sends. The engine keeps the spans and applies the styles; this only reads them.
import type { CharStyleDto, TextBaseline, TextCase, TextDto } from "./engine";

/** A character's settings: sizes and leading in pixels (leading null: automatic). */
export type CharSettings = {
  family: string;
  weight: number;
  italic: boolean;
  size: number;
  color: [number, number, number, number];
  tracking: number;
  leading: number | null;
  fauxBold: boolean;
  fauxItalic: boolean;
  case: TextCase;
  baseline: TextBaseline;
  underline: boolean;
  strikethrough: boolean;
  ligatures: boolean;
};

export type CharField = keyof CharSettings;

const FIELDS: CharField[] = [
  "family",
  "weight",
  "italic",
  "size",
  "color",
  "tracking",
  "leading",
  "fauxBold",
  "fauxItalic",
  "case",
  "baseline",
  "underline",
  "strikethrough",
  "ligatures",
];

/** The settings of characters set in `style` (a span's), or of the text's own. */
export function charSettings(text: TextDto, style: CharStyleDto = {}): CharSettings {
  return {
    family: style.family ?? text.family,
    weight: style.weight ?? text.weight,
    italic: style.italic ?? text.italic,
    size: style.size ?? text.size,
    color: style.color ?? text.color,
    tracking: style.tracking ?? text.tracking,
    leading: style.leading !== undefined ? style.leading : text.leading,
    fauxBold: style.fauxBold ?? text.fauxBold ?? false,
    fauxItalic: style.fauxItalic ?? text.fauxItalic ?? false,
    case: style.case ?? text.case ?? "normal",
    baseline: style.baseline ?? text.baseline ?? "normal",
    underline: style.underline ?? text.underline ?? false,
    strikethrough: style.strikethrough ?? text.strikethrough ?? false,
    ligatures: style.ligatures ?? text.ligatures ?? true,
  };
}

function same(field: CharField, a: CharSettings, b: CharSettings): boolean {
  if (field === "color") return a.color.every((v, i) => v === b.color[i]);
  return a[field] === b[field];
}

/**
 * What characters `start` to `end` (UTF-16 units) are set in, as Photoshop shows them: the
 * first one's settings, and the fields that differ among them (`mixed`). At a caret
 * (`start === end`), the character before it (the first one at the text's start), with the
 * style chosen there for the next characters typed (`pending`) over it.
 */
export function styleAt(
  text: TextDto,
  start: number,
  end: number,
  pending: CharStyleDto | null = null,
): { settings: CharSettings; mixed: CharField[] } {
  let from = Math.max(0, Math.min(start, end));
  let to = Math.min(text.content.length, Math.max(start, end));
  const caret = from >= to;
  if (caret) {
    from = Math.max(0, Math.min(from, text.content.length) - 1);
    to = Math.min(from + 1, text.content.length);
  }
  // The pieces of the range: the spans in it, and the text's own settings between them.
  const pieces: CharSettings[] = [];
  let at = from;
  for (const span of text.spans ?? []) {
    if (span.end <= from || span.start >= to) continue;
    if (span.start > at) pieces.push(charSettings(text));
    pieces.push(charSettings(text, span.style));
    at = span.end;
  }
  if (at < to || pieces.length === 0) pieces.push(charSettings(text));
  const first = pieces[0];
  const mixed = FIELDS.filter((f) => pieces.some((p) => !same(f, p, first)));
  if (!caret || !pending) return { settings: first, mixed };
  return { settings: { ...first, ...pending }, mixed };
}

/** Whether `change` sets nothing. */
export function isEmptyStyle(change: CharStyleDto): boolean {
  return Object.keys(change).length === 0;
}

/**
 * `text` with its own settings changed by `change` (a style applied to the whole text), as the
 * engine makes it: shown at once while the text is typed.
 */
export function withOwnStyle(text: TextDto, change: CharStyleDto): TextDto {
  return { ...text, ...charSettings(text, change) };
}

/**
 * A style chosen at a caret with nothing selected (Photoshop's pending style): it waits for the
 * next characters typed there, and is forgotten when the caret moves.
 */
export type PendingStyle = { at: number; style: CharStyleDto };

/** The pending style that applies with `selection`: only at its caret. */
export function pendingAt(
  pending: PendingStyle | null,
  selection: readonly [number, number],
): CharStyleDto | null {
  return pending && selection[0] === pending.at && selection[1] === pending.at
    ? pending.style
    : null;
}

/**
 * The characters `after` holds in place of `selection` of `before` (UTF-16 units of `after`),
 * when typing or pasting replaced the selection; null when nothing was put there.
 */
export function typedRange(
  before: string,
  after: string,
  selection: readonly [number, number],
): [number, number] | null {
  const start = Math.min(selection[0], selection[1]);
  const end = Math.max(selection[0], selection[1]);
  const length = after.length - (before.length - (end - start));
  if (length <= 0) return null;
  const typed: [number, number] = [start, start + length];
  const kept =
    after.slice(0, start) === before.slice(0, start) && after.slice(typed[1]) === before.slice(end);
  return kept ? typed : null;
}

/** The buttons of Photoshop's Character panel: its row of styles, and Standard Ligatures. */
export type CharToggle =
  | "fauxBold"
  | "fauxItalic"
  | "allCaps"
  | "smallCaps"
  | "superscript"
  | "subscript"
  | "underline"
  | "strikethrough"
  | "ligatures";

/** Photoshop's row of style buttons, in its order. */
export const STYLE_BUTTONS: readonly CharToggle[] = [
  "fauxBold",
  "fauxItalic",
  "allCaps",
  "smallCaps",
  "superscript",
  "subscript",
  "underline",
  "strikethrough",
];

/** The setting each button shows and sets, and the value it sets when pressed. */
const TOGGLES: Record<CharToggle, { field: CharField; value: CharSettings[CharField] }> = {
  fauxBold: { field: "fauxBold", value: true },
  fauxItalic: { field: "fauxItalic", value: true },
  allCaps: { field: "case", value: "allCaps" },
  smallCaps: { field: "case", value: "smallCaps" },
  superscript: { field: "baseline", value: "superscript" },
  subscript: { field: "baseline", value: "subscript" },
  underline: { field: "underline", value: true },
  strikethrough: { field: "strikethrough", value: true },
  ligatures: { field: "ligatures", value: true },
};

/** Whether a button shows pressed: the characters all set so (characters set apart: not). */
export function isPressed(
  toggle: CharToggle,
  settings: CharSettings,
  mixed: readonly CharField[],
): boolean {
  const { field, value } = TOGGLES[toggle];
  return !mixed.includes(field) && settings[field] === value;
}

/**
 * What a click on a button sets: pressed, its setting back to normal (All Caps and Small Caps
 * are one setting, as Superscript and Subscript: pressing one releases the other).
 */
export function toggleStyle(toggle: CharToggle, pressed: boolean): CharStyleDto {
  const { field, value } = TOGGLES[toggle];
  const off: Partial<Record<CharField, CharSettings[CharField]>> = {
    case: "normal",
    baseline: "normal",
  };
  const set = pressed ? (off[field] ?? false) : value;
  return { [field]: set } as CharStyleDto;
}
