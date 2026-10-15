// Character settings of a text layer (ADR 0041): what the Character settings and the Type
// tool's options bar show for the characters selected, or the whole text, and what a change
// there sends. The engine keeps the spans and applies the styles; this only reads them.
import type { CharStyleDto, TextDto } from "./engine";

/** A character's settings: sizes and leading in pixels (leading null: automatic). */
export type CharSettings = {
  family: string;
  weight: number;
  italic: boolean;
  size: number;
  color: [number, number, number, number];
  tracking: number;
  leading: number | null;
};

export type CharField = keyof CharSettings;

const FIELDS: CharField[] = ["family", "weight", "italic", "size", "color", "tracking", "leading"];

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
  };
}

function same(field: CharField, a: CharSettings, b: CharSettings): boolean {
  if (field === "color") return a.color.every((v, i) => v === b.color[i]);
  return a[field] === b[field];
}

/**
 * What characters `start` to `end` (UTF-16 units) are set in, as Photoshop shows them: the
 * first one's settings, and the fields that differ among them (`mixed`). At a caret
 * (`start === end`), the character before it (the first one at the text's start).
 */
export function styleAt(
  text: TextDto,
  start: number,
  end: number,
): { settings: CharSettings; mixed: CharField[] } {
  let from = Math.max(0, Math.min(start, end));
  let to = Math.min(text.content.length, Math.max(start, end));
  if (from >= to) {
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
  const settings = pieces[0];
  const mixed = FIELDS.filter((f) => pieces.some((p) => !same(f, p, settings)));
  return { settings, mixed };
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
