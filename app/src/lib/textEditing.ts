// Editing a text with the Type tool (ADR 0041): selections and caret moves drawn from the
// engine's layout (its carets and lines, from parley). Places are counted in UTF-16 units, as
// the hidden text field counts them; points are in document pixels, so that a rotated or
// skewed text layer is followed.
import type { TextLineView } from "./engine";
import type { Caret } from "./typeText";

export type Point = [number, number];

/** What the engine says of a text: one caret per place, and its lines. */
export type TextLayout = { carets: readonly Caret[]; lines: readonly TextLineView[] };

/** The editing commands the Edit menu routes to the text being typed. */
export type TextCommand = "cut" | "copy" | "paste" | "selectAll";

/** A selection's highlight on one line: four corners, clockwise from the top left. */
export type Quad = [Point, Point, Point, Point];

/** The line holding place `index` (a caret at a wrap stands on the next line). */
export function lineAt(lines: readonly TextLineView[], index: number): number {
  if (lines.length === 0) return 0;
  const found = lines.findIndex((l) => index < l.end);
  return found === -1 ? lines.length - 1 : found;
}

/**
 * The last place a caret stands on `line`: before its line break, before the space it wrapped
 * at (the place after is the next line's start), or at the text's end on the last line.
 */
function lineLast(line: TextLineView, last: boolean): number {
  return last && !line.hardBreak ? line.end : Math.max(line.start, line.end - 1);
}

/** A caret's direction along its line (unit vector), perpendicular to its top-to-bottom. */
function along([x0, y0, x1, y1]: Caret): Point {
  const dx = x1 - x0;
  const dy = y1 - y0;
  const length = Math.hypot(dx, dy) || 1;
  return [dy / length, -dx / length];
}

/** How far along its line a caret stands. */
function advance(caret: Caret): number {
  const [ux, uy] = along(caret);
  return caret[0] * ux + caret[1] * uy;
}

/**
 * The highlight of the selection from `start` to `end` (places, in any order): one quad per
 * line it covers, from caret to caret, reaching the line's end where the selection goes on to
 * the next line (a selected line break shows as a narrow block past the line's end, as in
 * parley's selection geometry).
 */
export function selectionQuads(layout: TextLayout, start: number, end: number): Quad[] {
  const { carets, lines } = layout;
  const from = Math.min(start, end);
  const to = Math.max(start, end);
  if (from === to || carets.length === 0) return [];
  const caret = (i: number) => carets[Math.min(Math.max(i, 0), carets.length - 1)];
  const quads: Quad[] = [];
  lines.forEach((line, l) => {
    const last = l === lines.length - 1;
    if (to <= line.start || (from >= line.end && !last)) return;
    const a = caret(Math.max(from, line.start));
    const left: [Point, Point] = [
      [a[0], a[1]],
      [a[2], a[3]],
    ];
    let right: [Point, Point];
    if (to < line.end || (last && to <= line.end)) {
      const b = caret(to);
      right = [
        [b[0], b[1]],
        [b[2], b[3]],
      ];
    } else if (line.hardBreak) {
      // The line break selected: a block a quarter of the line's height wide past its end.
      const b = caret(line.end - 1);
      const [ux, uy] = along(b);
      const w = Math.hypot(b[2] - b[0], b[3] - b[1]) / 4;
      right = [
        [b[0] + ux * w, b[1] + uy * w],
        [b[2] + ux * w, b[3] + uy * w],
      ];
    } else {
      // Wrapped: to the line's end.
      right = [line.corners[1], line.corners[2]];
    }
    if (left[0][0] === right[0][0] && left[0][1] === right[0][1]) return;
    quads.push([left[0], right[0], right[1], left[1]]);
  });
  return quads;
}

/** The place nearest to `goal` (how far along the line) on line `l`. */
function nearestOnLine(layout: TextLayout, l: number, goal: number): number {
  const line = layout.lines[l];
  const lastIndex = lineLast(line, l === layout.lines.length - 1);
  let best = line.start;
  let distance = Infinity;
  for (let i = line.start; i <= lastIndex && i < layout.carets.length; i++) {
    const d = Math.abs(advance(layout.carets[i]) - goal);
    if (d < distance) {
      distance = d;
      best = i;
    }
  }
  return best;
}

/**
 * Up (`delta` -1) or Down (+1) from place `index`, on the lines as they are drawn: the place on
 * the line above or below nearest to `goal` (how far along the line the moves started; kept
 * through successive moves, as text editors do), or the text's start or end past its first or
 * last line.
 */
export function moveByLine(
  layout: TextLayout,
  content: string,
  index: number,
  delta: -1 | 1,
  goal: number | null,
): { index: number; goal: number } {
  const { carets, lines } = layout;
  if (carets.length === 0 || lines.length === 0) return { index, goal: goal ?? 0 };
  const from = carets[Math.min(Math.max(index, 0), carets.length - 1)];
  const keep = goal ?? advance(from);
  const target = lineAt(lines, index) + delta;
  if (target < 0) return { index: 0, goal: keep };
  if (target >= lines.length) return { index: content.length, goal: keep };
  return { index: nearestOnLine(layout, target, keep), goal: keep };
}

/** Home and End: the start or the end of the drawn line holding place `index`. */
export function lineBoundary(
  layout: TextLayout,
  content: string,
  index: number,
  end: boolean,
): number {
  const { lines } = layout;
  if (lines.length === 0) return end ? content.length : 0;
  const l = lineAt(lines, index);
  return end ? lineLast(lines[l], l === lines.length - 1) : lines[l].start;
}

/** The drawn line holding place `index`, without its line break (a triple click). */
export function lineRange(layout: TextLayout, content: string, index: number): [number, number] {
  const { lines } = layout;
  if (lines.length === 0) return [0, content.length];
  const l = lineAt(lines, index);
  const line = lines[l];
  return [line.start, line.hardBreak ? line.end - 1 : line.end];
}

/** The paragraph (between line breaks) holding place `index` (a quadruple click). */
export function paragraphRange(content: string, index: number): [number, number] {
  const start = content.lastIndexOf("\n", index - 1) + 1;
  const next = content.indexOf("\n", index);
  return [start, next === -1 ? content.length : next];
}

/**
 * The word holding place `index`, or the spaces or punctuation there (a double click), by the
 * platform's word boundaries. At a word's end (a click on its last letter's right half) the
 * word, not what follows it.
 */
export function wordRange(content: string, index: number): [number, number] {
  const segments = [...new Intl.Segmenter(undefined, { granularity: "word" }).segment(content)];
  if (segments.length === 0) return [0, 0];
  const at = Math.min(Math.max(index, 0), content.length - 1);
  let i = segments.findIndex((s) => at < s.index + s.segment.length);
  if (i === -1) i = segments.length - 1;
  const before = segments[i - 1];
  if (!segments[i].isWordLike && index === segments[i].index && before?.isWordLike) i -= 1;
  const found = segments[i];
  return [found.index, found.index + found.segment.length];
}

/** Consecutive presses, counted as the system counts double and triple clicks. */
export type Clicks = { time: number; point: Point; count: number };

/** Presses closer than this in time (ms) and distance (screen pixels) count together. */
const MULTI_CLICK_TIME = 500;
const MULTI_CLICK_DISTANCE = 4;

/** The press at `point` (screen pixels) at `time` (ms), after `previous`. */
export function countClick(previous: Clicks | null, time: number, point: Point): Clicks {
  const near =
    previous !== null &&
    time - previous.time <= MULTI_CLICK_TIME &&
    Math.hypot(point[0] - previous.point[0], point[1] - previous.point[1]) <= MULTI_CLICK_DISTANCE;
  return { time, point, count: near ? previous.count + 1 : 1 };
}

/**
 * What `count` presses select around place `index`, as in Photoshop: 1 places the caret, 2 a
 * word, 3 a line, 4 a paragraph, 5 and more the whole text.
 */
export function clickSelection(
  layout: TextLayout,
  content: string,
  index: number,
  count: number,
): [number, number] {
  switch (count) {
    case 1:
      return [index, index];
    case 2:
      return wordRange(content, index);
    case 3:
      return lineRange(layout, content, index);
    case 4:
      return paragraphRange(content, index);
    default:
      return [0, content.length];
  }
}
