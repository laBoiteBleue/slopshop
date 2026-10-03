import { expect, test } from "vitest";
import {
  DEFAULT_QUICK_MASK_OPACITY,
  QUICK_MASK_COLORS,
  loadQuickMaskOpacity,
  quickMaskAction,
  quickMaskColors,
  saveQuickMaskOpacity,
} from "../src/lib/quickMask";

/** A storage in memory (module tests have no `localStorage`). */
function memory() {
  const items = new Map<string, string>();
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
  };
}

test("white paints the selection in, black out, a gray neither wholly", () => {
  expect(quickMaskAction({ foreground: "#FFFFFF", background: "#000000" })).toBe("add");
  expect(quickMaskAction({ foreground: "#000000", background: "#ffffff" })).toBe("remove");
  expect(quickMaskAction({ foreground: "#808080", background: "#ffffff" })).toBeNull();
});

test("Add and Remove are the two pairs X swaps between", () => {
  const add = quickMaskColors("add");
  const remove = quickMaskColors("remove");
  expect(quickMaskAction(add)).toBe("add");
  expect(quickMaskAction(remove)).toBe("remove");
  expect({ foreground: add.background, background: add.foreground }).toEqual(remove);
  // Photoshop's default colors remove, as black paints the mask there.
  expect(quickMaskAction(QUICK_MASK_COLORS)).toBe("remove");
});

test("the overlay's opacity is kept between sessions, in range", () => {
  const store = memory();
  expect(loadQuickMaskOpacity(store)).toBe(DEFAULT_QUICK_MASK_OPACITY);
  saveQuickMaskOpacity(72.4, store);
  expect(loadQuickMaskOpacity(store)).toBe(72);
  store.setItem("slopshop.quickMaskOpacity", "250");
  expect(loadQuickMaskOpacity(store)).toBe(100);
  store.setItem("slopshop.quickMaskOpacity", "nonsense");
  expect(loadQuickMaskOpacity(store)).toBe(DEFAULT_QUICK_MASK_OPACITY);
  // Storage blocked: the default, and nothing thrown.
  const blocked = {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("blocked");
    },
  };
  expect(loadQuickMaskOpacity(blocked)).toBe(DEFAULT_QUICK_MASK_OPACITY);
  saveQuickMaskOpacity(10, blocked);
});
