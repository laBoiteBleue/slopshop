import { expect, test } from "vitest";
import {
  LOUPE_OUTER,
  LOUPE_SIDE,
  centerHex,
  eyedropperCursor,
  eyedropperFromKeys,
  tagAbove,
} from "../src/lib/eyedropper";

test("each eyedropper has its own pointer, the tip as the hotspot, a crosshair as fallback", () => {
  const cursors = (["pick", "add", "subtract"] as const).map(eyedropperCursor);
  expect(new Set(cursors).size).toBe(3);
  for (const cursor of cursors) {
    expect(cursor).toMatch(/^url\("data:image\/svg\+xml,.+"\) 4 20, crosshair$/);
    const svg = decodeURIComponent(cursor.slice('url("data:image/svg+xml,'.length));
    expect(svg).toContain("<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24'");
  }
});

test("Shift adds and Alt takes away, whichever eyedropper is chosen", () => {
  const none = { shiftKey: false, altKey: false };
  expect(eyedropperFromKeys("pick", none)).toBe("pick");
  expect(eyedropperFromKeys("subtract", none)).toBe("subtract");
  expect(eyedropperFromKeys("pick", { shiftKey: true, altKey: false })).toBe("add");
  expect(eyedropperFromKeys("add", { shiftKey: false, altKey: true })).toBe("subtract");
  expect(eyedropperFromKeys("subtract", { shiftKey: true, altKey: true })).toBe("add");
});

test("the value's tag goes under the loupe, above it at the bottom of the window", () => {
  expect(tagAbove(400, 800)).toBe(false);
  expect(tagAbove(800 - LOUPE_OUTER / 2 - 10, 800)).toBe(true);
});

test("the ring and the tag show the sampled pixel's color, none where nothing is shown", () => {
  const patch = new Uint8ClampedArray(LOUPE_SIDE * LOUPE_SIDE * 4);
  expect(centerHex(patch)).toBeNull();
  const middle = ((LOUPE_SIDE * LOUPE_SIDE - 1) / 2) * 4;
  patch.set([12, 34, 255, 255], middle);
  expect(centerHex(patch)).toBe("#0c22ff");
  expect(centerHex(new Uint8ClampedArray(4))).toBeNull();
});
