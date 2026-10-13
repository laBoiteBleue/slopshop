import { describe, expect, test } from "vitest";
import {
  caretAt,
  clampSizePt,
  commitsText,
  defaultTypeOptions,
  newText,
  optionsOf,
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
