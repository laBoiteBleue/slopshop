import { describe, expect, test } from "vitest";
import type { TextDto } from "../src/lib/engine";
import { charSettings, isEmptyStyle, styleAt, withOwnStyle } from "../src/lib/textStyle";

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
