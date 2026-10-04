import { expect, test } from "vitest";
import { DEFAULT_VIEW_SETTINGS, loadViewSettings, saveViewSettings } from "../src/lib/viewSettings";

function memory() {
  const items = new Map<string, string>();
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
  };
}

test("Rulers off and Snap on at first, both kept between sessions", () => {
  const store = memory();
  expect(loadViewSettings(store)).toEqual({ rulers: false, snap: true });
  saveViewSettings({ rulers: true, snap: false }, store);
  expect(loadViewSettings(store)).toEqual({ rulers: true, snap: false });
  // Saved before the rulers existed: they are off.
  store.setItem("slopshop.view", '{"snap":false}');
  expect(loadViewSettings(store)).toEqual({ rulers: false, snap: false });
});

test("anything odd saved reads as the defaults", () => {
  for (const saved of ["{", "null", '{"snap":"no","rulers":1}', "[]"]) {
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
  expect(() => saveViewSettings({ rulers: true, snap: false }, broken)).not.toThrow();
});
