import { expect, test } from "vitest";
import {
  DEFAULT_QUICK_MASK_OPACITY,
  MASK_COLORS,
  loadQuickMaskOpacity,
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

test("masks start with Photoshop's default colors: black paints, white behind", () => {
  expect(MASK_COLORS).toEqual({ foreground: "#000000", background: "#ffffff" });
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
