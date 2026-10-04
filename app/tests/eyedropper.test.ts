import { expect, test } from "vitest";
import {
  LOUPE_OUTER,
  LOUPE_SIDE,
  LOUPE_TAG,
  centerHex,
  eyedropperCursor,
  eyedropperFromKeys,
  loupePlacement,
} from "../src/lib/eyedropper";

test("each eyedropper has its own pointer, a small cross centered on the hotspot, a crosshair as fallback", () => {
  const cursors = (["pick", "add", "subtract"] as const).map(eyedropperCursor);
  expect(new Set(cursors).size).toBe(3);
  for (const cursor of cursors) {
    expect(cursor).toMatch(/^url\("data:image\/svg\+xml,.+"\) 8 8, crosshair$/);
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

test("the loupe and its value sit above right of the pointer, on the other side where the window ends", () => {
  const view = { width: 1000, height: 800 };
  const height = LOUPE_OUTER + LOUPE_TAG;
  expect(loupePlacement(500, 400, view)).toEqual({ left: 516, top: 400 - 16 - height });
  // Near the right edge: to the left. Near the top: below.
  expect(loupePlacement(950, 400, view).left).toBe(950 - 16 - LOUPE_OUTER);
  expect(loupePlacement(500, 50, view).top).toBe(66);
  // A window too small for either side: kept inside.
  expect(loupePlacement(60, 60, { width: 180, height: 200 })).toEqual({
    left: 0,
    top: 200 - height,
  });
});

test("the ring and the tag show the sampled pixel's color, none where nothing is shown", () => {
  const patch = new Uint8ClampedArray(LOUPE_SIDE * LOUPE_SIDE * 4);
  expect(centerHex(patch)).toBeNull();
  const middle = ((LOUPE_SIDE * LOUPE_SIDE - 1) / 2) * 4;
  patch.set([12, 34, 255, 255], middle);
  expect(centerHex(patch)).toBe("#0c22ff");
  expect(centerHex(new Uint8ClampedArray(4))).toBeNull();
});
