import { describe, expect, test } from "vitest";
import type { TextDto } from "../src/lib/engine";
import {
  charSettings,
  isEmptyStyle,
  isPressed,
  pendingAt,
  STYLE_BUTTONS,
  styleAt,
  toggleStyle,
  typedRange,
  withOwnStyle,
} from "../src/lib/textStyle";

const RED: [number, number, number, number] = [1, 0, 0, 1];

function text(spans: TextDto["spans"] = []): TextDto {
  return {
    content: "Hello world",
    family: "Inter",
    weight: 400,
    italic: false,
    size: 20,
    color: [0, 0, 0, 1],
    tracking: 0,
    leading: null,
    align: "left",
    position: [0, 0],
    spans,
  };
}

describe("character settings", () => {
  test("take a span's over the text's own", () => {
    expect(charSettings(text())).toEqual({
      family: "Inter",
      weight: 400,
      italic: false,
      size: 20,
      color: [0, 0, 0, 1],
      tracking: 0,
      leading: null,
      // Absent from the text: the defaults, Standard Ligatures on.
      fauxBold: false,
      fauxItalic: false,
      case: "normal",
      baseline: "normal",
      underline: false,
      strikethrough: false,
      ligatures: true,
    });
    const s = charSettings(text(), { weight: 700, leading: 30 });
    expect([s.weight, s.leading, s.family]).toEqual([700, 30, "Inter"]);
    // A span's automatic leading over the text's set one.
    expect(charSettings({ ...text(), leading: 30 }, { leading: null }).leading).toBeNull();
  });
});

describe("the style of characters", () => {
  const bold = text([{ start: 0, end: 5, style: { weight: 700 } }]);

  test("is the first one's, with what differs among them", () => {
    expect(styleAt(bold, 0, 5)).toEqual({
      settings: charSettings(bold, { weight: 700 }),
      mixed: [],
    });
    const across = styleAt(bold, 3, 8);
    expect(across.settings.weight).toBe(700);
    expect(across.mixed).toEqual(["weight"]);
    expect(styleAt(bold, 6, 11).mixed).toEqual([]);
    expect(styleAt(bold, 6, 11).settings.weight).toBe(400);
  });

  test("at a caret is the character's before it, the first one's at the start", () => {
    expect(styleAt(bold, 5, 5).settings.weight).toBe(700);
    expect(styleAt(bold, 6, 6).settings.weight).toBe(400);
    expect(styleAt(bold, 0, 0).settings.weight).toBe(700);
  });

  test("covers the gaps between spans", () => {
    const two = text([
      { start: 0, end: 2, style: { color: RED } },
      { start: 4, end: 6, style: { color: RED } },
    ]);
    expect(styleAt(two, 0, 6).mixed).toEqual(["color"]);
    expect(styleAt(two, 0, 2).mixed).toEqual([]);
  });

  test("of an empty text is the text's own", () => {
    expect(styleAt({ ...text(), content: "" }, 0, 0).settings).toEqual(charSettings(text()));
  });
});

describe("styles", () => {
  test("on the whole text change its own settings", () => {
    const t = withOwnStyle(text(), { size: 40, color: RED });
    expect([t.size, t.color, t.weight]).toEqual([40, RED, 400]);
    expect(isEmptyStyle({})).toBe(true);
    expect(isEmptyStyle({ italic: false })).toBe(false);
  });
});

describe("Photoshop's style buttons", () => {
  const settings = charSettings(text(), { underline: true, case: "smallCaps" });

  test("show pressed what the characters all are", () => {
    expect(STYLE_BUTTONS.filter((b) => isPressed(b, settings, []))).toEqual([
      "smallCaps",
      "underline",
    ]);
    expect(isPressed("ligatures", settings, [])).toBe(true);
    // Characters set apart: not pressed.
    expect(isPressed("underline", settings, ["underline"])).toBe(false);
    expect(isPressed("smallCaps", settings, ["case"])).toBe(false);
  });

  test("set their style, or back to normal when pressed", () => {
    expect(toggleStyle("fauxBold", false)).toEqual({ fauxBold: true });
    expect(toggleStyle("fauxBold", true)).toEqual({ fauxBold: false });
    // All Caps and Small Caps are one setting, as Superscript and Subscript.
    expect(toggleStyle("allCaps", false)).toEqual({ case: "allCaps" });
    expect(toggleStyle("smallCaps", true)).toEqual({ case: "normal" });
    expect(toggleStyle("subscript", false)).toEqual({ baseline: "subscript" });
    expect(toggleStyle("superscript", true)).toEqual({ baseline: "normal" });
    expect(toggleStyle("ligatures", true)).toEqual({ ligatures: false });
  });
});

describe("the pending style", () => {
  test("applies at its caret only", () => {
    const pending = { at: 3, style: { weight: 700 } };
    expect(pendingAt(pending, [3, 3])).toEqual({ weight: 700 });
    expect(pendingAt(pending, [4, 4])).toBeNull();
    expect(pendingAt(pending, [3, 5])).toBeNull();
    expect(pendingAt(null, [3, 3])).toBeNull();
  });

  test("shows over the character before the caret", () => {
    const shown = styleAt(text(), 3, 3, { size: 40, underline: true });
    expect([shown.settings.size, shown.settings.underline, shown.settings.weight]).toEqual([
      40,
      true,
      400,
    ]);
    // Not over characters selected.
    expect(styleAt(text(), 0, 3, { size: 40 }).settings.size).toBe(20);
  });

  test("goes to the characters typed in place of the selection", () => {
    // Typed at the caret, even where the text has the same letter after it.
    expect(typedRange("Hello", "Helllo", [3, 3])).toEqual([3, 4]);
    expect(typedRange("Hello", "Hexyzllo", [2, 2])).toEqual([2, 5]);
    // A selection replaced (pasted).
    expect(typedRange("Hello", "HYo", [1, 4])).toEqual([1, 2]);
    // Nothing typed (deleted), or not where the selection was.
    expect(typedRange("Hello", "Hell", [5, 5])).toBeNull();
    expect(typedRange("Hello", "Hello!", [2, 2])).toBeNull();
  });
});
