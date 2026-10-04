import { test } from "vitest";
import assert from "node:assert/strict";
import { clampOffset, GRIP } from "../src/lib/dialogDrag";

const view = { width: 1000, height: 800 };
/** A 400 x 300 dialog centered in the window. */
const centered = { left: 300, top: 250, width: 400, height: 300 };

test("a dialog moves freely within the window", () => {
  assert.deepEqual(clampOffset({ x: -120, y: 80 }, centered, view), { x: -120, y: 80 });
});

test("its title bar stays in the window: top edge inside, a grip left across", () => {
  // Up: its top edge stops at the window's.
  assert.deepEqual(clampOffset({ x: 0, y: -400 }, centered, view), { x: 0, y: -250 });
  // Down: a grip of its title bar left above the window's bottom.
  assert.deepEqual(clampOffset({ x: 0, y: 900 }, centered, view), { x: 0, y: 800 - GRIP - 250 });
  // Sideways: a grip of it left on either side.
  assert.deepEqual(clampOffset({ x: -900, y: 0 }, centered, view), { x: GRIP - 700, y: 0 });
  assert.deepEqual(clampOffset({ x: 900, y: 0 }, centered, view), { x: 1000 - GRIP - 300, y: 0 });
});

test("in a window too small for the grip, the dialog's top edge stays in it", () => {
  const tiny = { width: 20, height: 20 };
  const box = { left: 10, top: 10, width: 400, height: 300 };
  assert.deepEqual(clampOffset({ x: 50, y: 50 }, box, tiny), { x: -30, y: -10 });
});
