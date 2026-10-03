import { expect, test } from "vitest";
import {
  DEFAULT_DOCK,
  MIN_DOCK_HEIGHT,
  clampDockHeight,
  clickTab,
  loadDock,
  saveDock,
} from "../src/lib/panelDock";

function memory() {
  const items = new Map<string, string>();
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
  };
}

test("a tab unfolds its panel; the unfolded one's tab folds the dock to its icons", () => {
  const folded = clickTab({ open: "properties", height: 300 }, "properties");
  expect(folded).toEqual({ open: null, height: 300 });
  expect(clickTab(folded, "selections")).toEqual({ open: "selections", height: 300 });
  expect(clickTab({ open: "properties", height: 300 }, "selections").open).toBe("selections");
});

test("the dock keeps its minimum height and leaves Layers room", () => {
  expect(clampDockHeight(10, 900)).toBe(MIN_DOCK_HEIGHT);
  expect(clampDockHeight(300.4, 900)).toBe(300);
  expect(clampDockHeight(850, 900)).toBe(740);
  // A column too short for both: the dock keeps its minimum.
  expect(clampDockHeight(400, 200)).toBe(MIN_DOCK_HEIGHT);
});

test("the dock's state is kept between sessions; anything odd reads as the default", () => {
  const store = memory();
  expect(loadDock(store)).toEqual(DEFAULT_DOCK);
  saveDock({ open: null, height: 333 }, store);
  expect(loadDock(store)).toEqual({ open: null, height: 333 });
  saveDock({ open: "selections", height: 200 }, store);
  expect(loadDock(store)).toEqual({ open: "selections", height: 200 });
  store.setItem("slopshop.dock", JSON.stringify({ open: "history", height: 5 }));
  expect(loadDock(store)).toEqual(DEFAULT_DOCK);
  store.setItem("slopshop.dock", "{not json");
  expect(loadDock(store)).toEqual(DEFAULT_DOCK);
});
