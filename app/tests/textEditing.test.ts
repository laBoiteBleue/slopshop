import { describe, expect, test } from "vitest";
import type { TextLineView } from "../src/lib/engine";
import {
  clickSelection,
  countClick,
  lineAt,
  lineBoundary,
  lineRange,
  moveByLine,
  paragraphRange,
  selectionQuads,
  wordRange,
  type TextLayout,
} from "../src/lib/textEditing";
import type { Caret } from "../src/lib/typeText";

/**
 * A layout as the engine gives it, for lines of text (each with its place range): characters
 * 10 px wide from x = 0, lines 20 px apart, carets 16 px tall. `wrapped`: the lines wrapped
 * where the text has no line break (`lines` give their text, spaces included).
 */
function layout(lines: string[], wrapped = false): { layout: TextLayout; content: string } {
  const carets: Caret[] = [];
  const views: TextLineView[] = [];
  let start = 0;
  lines.forEach((line, l) => {
    const last = l === lines.length - 1;
    const hardBreak = !wrapped && !last;
    const top = 20 * l;
    // A wrapped line's last place is the next line's first.
    const count = last || hardBreak ? line.length + 1 : line.length;
    for (let i = 0; i < count; i++) carets.push([10 * i, top, 10 * i, top + 16]);
    const end = start + line.length + (hardBreak ? 1 : 0);
    views.push({
      start,
      end,
      hardBreak,
      corners: [
        [0, top],
        [10 * line.length, top],
        [10 * line.length, top + 16],
        [0, top + 16],
      ],
    });
    start = end;
  });
  return { layout: { carets, lines: views }, content: lines.join(wrapped ? "" : "\n") };
}

describe("lines", () => {
  const { layout: two, content } = layout(["abc", "de"]);

  test("a place's line: a line holds its line break", () => {
    expect(content).toBe("abc\nde");
    expect([0, 2, 3, 4, 6].map((i) => lineAt(two.lines, i))).toEqual([0, 0, 0, 1, 1]);
  });

  test("a wrap's place is the next line's", () => {
    const { layout: w } = layout(["ab ", "cd"], true);
    expect(lineAt(w.lines, 3)).toBe(1);
  });

  test("Home and End go to the drawn line's ends", () => {
    expect(lineBoundary(two, content, 1, false)).toBe(0);
    // Before the line break.
    expect(lineBoundary(two, content, 1, true)).toBe(3);
    expect(lineBoundary(two, content, 4, true)).toBe(6);
    expect(lineBoundary(two, content, 6, false)).toBe(4);
    // A wrapped line ends before the space it wrapped at.
    const wrapped = layout(["ab ", "cd"], true);
    expect(lineBoundary(wrapped.layout, wrapped.content, 0, true)).toBe(2);
    expect(lineBoundary(wrapped.layout, wrapped.content, 4, false)).toBe(3);
  });
});

describe("moves by line", () => {
  const { layout: three, content } = layout(["abcdef", "ab", "abcdef"]);

  test("go to the nearest place on the line above or below", () => {
    // From after "abc" down: the second line's end; then down again, the goal kept.
    const down = moveByLine(three, content, 3, 1, null);
    expect(down.index).toBe(9);
    const again = moveByLine(three, content, down.index, 1, down.goal);
    expect(again.index).toBe(10 + 3);
    const up = moveByLine(three, content, 13, -1, null);
    expect(up.index).toBe(9);
  });

  test("go to the text's start or end past its first or last line", () => {
    expect(moveByLine(three, content, 2, -1, null).index).toBe(0);
    expect(moveByLine(three, content, 12, 1, null).index).toBe(content.length);
  });

  test("follow a rotated text along its lines", () => {
    // The text turned a quarter: lines go down, the next line is to the left.
    const rotated: TextLayout = {
      carets: [0, 1, 2, 3, 4].map((i): Caret =>
        i < 3 ? [0, i * 10, -16, i * 10] : [-20, (i - 3) * 10, -36, (i - 3) * 10],
      ),
      lines: [
        {
          start: 0,
          end: 3,
          hardBreak: true,
          corners: [
            [0, 0],
            [0, 20],
            [-16, 20],
            [-16, 0],
          ],
        },
        {
          start: 3,
          end: 5,
          hardBreak: false,
          corners: [
            [-20, 0],
            [-20, 10],
            [-36, 10],
            [-36, 0],
          ],
        },
      ],
    };
    expect(moveByLine(rotated, "ab\nc", 1, 1, null).index).toBe(4);
  });
});

describe("selection highlight", () => {
  const { layout: two } = layout(["abc", "de"]);

  test("nothing for a caret", () => {
    expect(selectionQuads(two, 2, 2)).toEqual([]);
  });

  test("from caret to caret on one line, in either order", () => {
    const quad = [
      [10, 0],
      [30, 0],
      [30, 16],
      [10, 16],
    ];
    expect(selectionQuads(two, 1, 3)).toEqual([quad]);
    expect(selectionQuads(two, 3, 1)).toEqual([quad]);
  });

  test("across lines: the line break shows past the first line's end", () => {
    const quads = selectionQuads(two, 1, 5);
    expect(quads).toHaveLength(2);
    // A quarter of the line's height past "abc".
    expect(quads[0]).toEqual([
      [10, 0],
      [34, 0],
      [34, 16],
      [10, 16],
    ]);
    expect(quads[1]).toEqual([
      [0, 20],
      [10, 20],
      [10, 36],
      [0, 36],
    ]);
  });

  test("a wrapped line selected to its end reaches its box's end", () => {
    const { layout: w } = layout(["ab ", "cd"], true);
    const quads = selectionQuads(w, 1, 4);
    expect(quads[0][1]).toEqual([30, 0]);
    expect(quads[1]).toEqual([
      [0, 20],
      [10, 20],
      [10, 36],
      [0, 36],
    ]);
  });
});

describe("clicks", () => {
  test("count while they are near in time and place", () => {
    const one = countClick(null, 1000, [10, 10]);
    expect(one.count).toBe(1);
    const two = countClick(one, 1200, [11, 10]);
    expect(two.count).toBe(2);
    expect(countClick(two, 1400, [10, 11]).count).toBe(3);
    expect(countClick(two, 2000, [10, 10]).count).toBe(1);
    expect(countClick(two, 1300, [40, 10]).count).toBe(1);
  });

  const { layout: two, content } = layout(["Hello world", "again"]);

  test("select a word, a line, a paragraph, then everything", () => {
    expect(clickSelection(two, content, 2, 1)).toEqual([2, 2]);
    expect(clickSelection(two, content, 2, 2)).toEqual([0, 5]);
    expect(clickSelection(two, content, 8, 3)).toEqual([0, 11]);
    expect(clickSelection(two, content, 13, 4)).toEqual([12, 17]);
    expect(clickSelection(two, content, 2, 5)).toEqual([0, content.length]);
  });
});

describe("words and paragraphs", () => {
  test("a word, or the spaces where there is none", () => {
    expect(wordRange("Hello  world", 7)).toEqual([7, 12]);
    expect(wordRange("Hello  world", 6)).toEqual([5, 7]);
    // At a word's end (a click on its last letter's right half): the word.
    expect(wordRange("Hello  world", 5)).toEqual([0, 5]);
    expect(wordRange("Hello", 5)).toEqual([0, 5]);
    expect(wordRange("", 0)).toEqual([0, 0]);
    // Accented letters are part of words.
    expect(wordRange("Été ça", 1)).toEqual([0, 3]);
  });

  test("a paragraph runs between line breaks", () => {
    expect(paragraphRange("ab\ncd\nef", 4)).toEqual([3, 5]);
    expect(paragraphRange("ab\ncd", 0)).toEqual([0, 2]);
    expect(paragraphRange("ab\ncd", 5)).toEqual([3, 5]);
  });

  test("a line, wrapped or not, without its line break", () => {
    const { layout: two, content } = layout(["abc", "de"]);
    expect(lineRange(two, content, 1)).toEqual([0, 3]);
    const wrapped = layout(["ab ", "cd"], true);
    expect(lineRange(wrapped.layout, wrapped.content, 1)).toEqual([0, 3]);
  });
});
