import { describe, expect, test } from "vitest";
import {
  caretAt,
  caretIndexAt,
  clampSizePt,
  commitsText,
  defaultTypeOptions,
  newText,
  optionsOf,
  styleChange,
  pixelsToPoints,
  pointsToPixels,
  styleKey,
  styleOfKey,
  weightName,
  withOptions,
  type Caret,
} from "../src/lib/typeText";

describe("sizes in points", () => {
  test("go through the document's resolution", () => {
    expect(pointsToPixels(12, 72)).toBe(12);
    expect(pointsToPixels(12, 300)).toBe(50);
    expect(pixelsToPoints(50, 300)).toBe(12);
  });

  test("stay in Photoshop's range and what the engine draws", () => {
    expect(clampSizePt(0, 72)).toBe(0.01);
    expect(clampSizePt(5000, 72)).toBe(1296);
    // 10,000 px at 1200 ppi is 600 pt.
    expect(clampSizePt(1000, 1200)).toBe(600);
    expect(clampSizePt(Number.NaN, 72)).toBe(12);
  });
});

describe("texts and options", () => {
  const options = { ...defaultTypeOptions("#ff0000"), sizePt: 24, align: "center" as const };

  test("a new text is empty, set in the options at the place clicked", () => {
    expect(newText(options, [10, 20], 144, "Inter Variable")).toEqual({
      content: "",
      family: "Inter Variable",
      weight: 400,
      italic: false,
      size: 48,
      color: [1, 0, 0, 1],
      tracking: 0,
      leading: null,
      align: "center",
      position: [10, 20],
    });
  });

  test("options change a text's font, size, color and alignment, not its string or place", () => {
    const text = newText(options, [10, 20], 72, "Inter Variable");
    const changed = withOptions(
      { ...text, content: "Hi", tracking: 50 },
      { ...options, family: "Arial", weight: 700, italic: true, sizePt: 10, align: "right" },
      72,
      "Inter Variable",
    );
    expect(changed).toMatchObject({
      content: "Hi",
      family: "Arial",
      weight: 700,
      italic: true,
      size: 10,
      tracking: 50,
      align: "right",
      position: [10, 20],
    });
  });

  test("a text's options come back as set, sizes in hundredths of a point", () => {
    const text = newText({ ...options, family: "Arial" }, [0, 0], 300, "Inter Variable");
    expect(optionsOf(text, 300)).toEqual({ ...options, family: "Arial" });
    expect(optionsOf({ ...text, size: 10 }, 300).sizePt).toBe(2.4);
  });

  test("the options of characters selected are the first one's", () => {
    const text = {
      ...newText(options, [0, 0], 72, "Inter Variable"),
      content: "Hello",
      spans: [{ start: 2, end: 5, style: { weight: 700, size: 30 } }],
    };
    expect(optionsOf(text, 72, [2, 4])).toMatchObject({ weight: 700, sizePt: 30 });
    expect(optionsOf(text, 72, [0, 4])).toMatchObject({ weight: 400, sizePt: 24 });
    expect(optionsOf(text, 72)).toMatchObject({ weight: 400 });
  });

  test("a change of options is a style of what changed, the alignment apart", () => {
    const shown = { ...options, family: "Arial" };
    expect(styleChange(shown, shown, 72, "Inter Variable")).toEqual({});
    expect(
      styleChange(
        shown,
        { ...shown, weight: 700, sizePt: 30, align: "left" },
        144,
        "Inter Variable",
      ),
    ).toEqual({ weight: 700, size: 60 });
    expect(styleChange(shown, { ...shown, color: "#0000FF" }, 72, "X")).toEqual({
      color: [0, 0, 1, 1],
    });
    // An empty family is the default one.
    expect(
      styleChange(
        { ...shown, family: "" },
        { ...shown, family: "Inter Variable" },
        72,
        "Inter Variable",
      ),
    ).toEqual({});
    expect(styleChange(shown, { ...shown, italic: true }, 72, "X")).toEqual({ italic: true });
  });
});

test("weights are named as Photoshop names them, styles keyed for the list", () => {
  expect(weightName(400)).toBe("regular");
  expect(weightName(700)).toBe("bold");
  expect(weightName(950)).toBe("black");
  expect(weightName(50)).toBe("thin");
  expect(styleKey({ weight: 700, italic: true })).toBe("700 italic");
  expect(styleOfKey("700 italic")).toEqual({ weight: 700, italic: true });
  expect(styleOfKey("300")).toEqual({ weight: 300, italic: false });
});

test("the caret stands at its place, or at the last one while the layout catches up", () => {
  const carets: Caret[] = [
    [0, 0, 0, 10],
    [5, 0, 5, 10],
  ];
  expect(caretAt(carets, 1)).toEqual([5, 0, 5, 10]);
  expect(caretAt(carets, 7)).toEqual([5, 0, 5, 10]);
  expect(caretAt([], 0)).toBeNull();
});

test("a click finds the nearest place in the text, none outside it", () => {
  // Two lines: "ab" then "c", 10 px apart, carets 8 px tall.
  const carets: Caret[] = [
    [0, 0, 0, 8],
    [5, 0, 5, 8],
    [10, 0, 10, 8],
    [0, 10, 0, 18],
    [6, 10, 6, 18],
  ];
  expect(caretIndexAt(carets, [4, 4], 8)).toBe(1);
  expect(caretIndexAt(carets, [9, 4], 8)).toBe(2);
  // Right of the short second line: its end.
  expect(caretIndexAt(carets, [12, 14], 8)).toBe(4);
  expect(caretIndexAt(carets, [1, 15], 8)).toBe(3);
  // Far away: outside.
  expect(caretIndexAt(carets, [40, 4], 8)).toBeNull();
  expect(caretIndexAt([], [0, 0], 8)).toBeNull();
});

test("Escape, Ctrl+Enter and the keypad's Enter commit; Enter alone is a new line", () => {
  const key = (key: string, code = key, ctrlKey = false, metaKey = false) => ({
    key,
    code,
    ctrlKey,
    metaKey,
  });
  expect(commitsText(key("Escape"), false)).toBe(true);
  expect(commitsText(key("Enter"), false)).toBe(false);
  expect(commitsText(key("Enter", "Enter", true), false)).toBe(true);
  expect(commitsText(key("Enter", "NumpadEnter"), false)).toBe(true);
  expect(commitsText(key("Enter", "Enter", false, true), true)).toBe(true);
  expect(commitsText(key("Enter", "Enter", true), true)).toBe(false);
  expect(commitsText(key("a"), false)).toBe(false);
});
