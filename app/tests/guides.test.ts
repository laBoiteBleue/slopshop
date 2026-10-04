import { expect, test } from "vitest";
import type { Guide } from "../src/lib/engine";
import { dropped, dropsOut, guidePosition } from "../src/lib/guides";

const v = (position: number): Guide => ({ vertical: true, position });
const h = (position: number): Guide => ({ vertical: false, position });

test("a guide lands on a whole pixel", () => {
  expect(guidePosition(10.4)).toBe(10);
  expect(guidePosition(10.6)).toBe(11);
  expect(guidePosition(-3.6)).toBe(-4);
});

test("released outside the image's area, a guide is deleted", () => {
  const area = { left: 20, top: 40, right: 120, bottom: 90 };
  expect(dropsOut(20, 40, area)).toBe(false);
  expect(dropsOut(119, 89, area)).toBe(false);
  // Over a ruler (left of or above the image), or anywhere else.
  expect(dropsOut(10, 60, area)).toBe(true);
  expect(dropsOut(60, 30, area)).toBe(true);
  expect(dropsOut(120, 60, area)).toBe(true);
});

test("a drag out of a ruler adds a guide; of a guide, moves or deletes it", () => {
  const guides = [v(10), h(20)];
  expect(dropped(guides, null, h(5))).toEqual([v(10), h(20), h(5)]);
  // Out of a ruler and back: nothing.
  expect(dropped(guides, null, null)).toBeNull();
  expect(dropped(guides, 0, v(30))).toEqual([v(30), h(20)]);
  expect(dropped(guides, 1, null)).toEqual([v(10)]);
  // Put back where it was, or a guide that is not there: nothing to undo.
  expect(dropped(guides, 0, v(10))).toBeNull();
  expect(dropped(guides, 5, v(1))).toBeNull();
});
