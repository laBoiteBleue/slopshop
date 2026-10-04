import { expect, test } from "vitest";
import { MIN_DOCK_HEIGHT, clampDockHeight, clickTab, followProperties } from "../src/lib/panelDock";

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

test("Properties unfolds for a layer that has some, then gives back what it replaced", () => {
  const dock = { open: "selections" as const, height: 280 };
  const shown = followProperties(dock, null, null, 4);
  expect(shown).toEqual({
    dock: { open: "properties", height: 280 },
    follow: { replaced: "selections" },
  });
  // Another adjustment layer: Properties stays, still to give back.
  expect(followProperties(shown.dock, shown.follow, 4, 7)).toEqual(shown);
  expect(followProperties(shown.dock, shown.follow, 7, null)).toEqual({ dock, follow: null });
  // A folded dock folds again.
  const folded = followProperties({ open: null, height: 280 }, null, null, 4);
  expect(followProperties(folded.dock, folded.follow, 4, null).dock.open).toBeNull();
  // The user chose meanwhile (no follow): nothing given back.
  expect(followProperties(shown.dock, null, 4, null)).toEqual({ dock: shown.dock, follow: null });
  // Already on Properties: nothing to give back later.
  expect(followProperties(shown.dock, null, null, 4)).toEqual({ dock: shown.dock, follow: null });
});
