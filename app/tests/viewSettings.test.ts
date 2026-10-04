import { expect, test } from "vitest";
import { DEFAULT_VIEW_SETTINGS, loadViewSettings, saveViewSettings } from "../src/lib/viewSettings";

function memory() {
  const items = new Map<string, string>();
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
  };
}

test("View > Snap is on at first and kept between sessions", () => {
  const store = memory();
  expect(loadViewSettings(store)).toEqual(DEFAULT_VIEW_SETTINGS);
  expect(DEFAULT_VIEW_SETTINGS.snap).toBe(true);
  saveViewSettings({ snap: false }, store);
  expect(loadViewSettings(store)).toEqual({ snap: false });
});

test("anything odd saved reads as the defaults", () => {
  for (const saved of ["{", "null", '{"snap":"no"}', "[]"]) {
    const store = memory();
    store.setItem("slopshop.view", saved);
    expect(loadViewSettings(store)).toEqual(DEFAULT_VIEW_SETTINGS);
  }
  const broken = {
    getItem: () => {
      throw new Error("no storage");
    },
    setItem: () => {
      throw new Error("no storage");
    },
  };
  expect(loadViewSettings(broken)).toEqual(DEFAULT_VIEW_SETTINGS);
  expect(() => saveViewSettings({ snap: false }, broken)).not.toThrow();
});
