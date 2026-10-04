import { expect, test } from "vitest";
import { LayersUis } from "../src/lib/layersUi.svelte";

test("each open document has its own Layers panel state, kept until its tab closes", () => {
  const uis = new LayersUis();
  const cat = uis.of(1);
  cat.selection = { ids: [3], active: 3, anchor: 3 };
  expect(uis.of(1)).toBe(cat);
  expect(uis.of(2)).not.toBe(cat);
  expect(uis.of(2).selection.ids).toEqual([]);
  uis.keep([2]);
  expect(uis.of(1)).not.toBe(cat);
  expect(uis.of(1).selection.ids).toEqual([]);
});
