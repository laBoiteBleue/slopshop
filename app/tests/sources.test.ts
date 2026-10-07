import { expect, test } from "vitest";
import type { DocumentView, LayerView, SourceView } from "../src/lib/engine";
import { makeUniqueEdit, sharingLayers, sourceName, sourceOf } from "../src/lib/sources";

const layer = (id: number, name: string, source: number | null): LayerView =>
  ({ id, name, kind: "raster", children: [], source }) as unknown as LayerView;

const source = (id: number, name: string, layers: number[]): SourceView => ({
  id,
  name,
  width: 10,
  height: 8,
  layers,
});

const doc = (layers: LayerView[], sources: SourceView[]) =>
  ({ id: 1, layers, sources }) as unknown as DocumentView;

test("a source shows its own name, else the first layer's showing it, in a group too", () => {
  const pasted = source(2, "", [5, 3]);
  const group = { ...layer(4, "Group", null), kind: "group", children: [layer(5, "Inner", 2)] };
  const layers = [layer(3, "Pasted", 2), group as LayerView];
  expect(sourceName(source(1, "photo.jpg", [3]), layers)).toBe("photo.jpg");
  expect(sourceName(pasted, layers)).toBe("Inner");
  expect(sourceName(source(9, "", [42]), layers)).toBe("");
});

test("Make Unique applies to the selected layers sharing their source, only them", () => {
  const a = layer(1, "A", 7);
  const b = layer(2, "B", 7);
  const alone = layer(3, "Alone", 8);
  const empty = layer(4, "Empty", null);
  const d = doc([a, b, alone, empty], [source(7, "photo.jpg", [1, 2]), source(8, "", [3])]);
  expect(sourceOf(d, b)?.name).toBe("photo.jpg");
  expect(sourceOf(d, empty)).toBeNull();
  expect(sharingLayers(d, [b, alone, empty])).toEqual([b]);
  expect(makeUniqueEdit(d, [b, alone])).toEqual({ kind: "makeUnique", ids: [2] });
  expect(makeUniqueEdit(d, [alone, empty])).toBeNull();
  expect(makeUniqueEdit(null, [b])).toBeNull();
  // A document of before the sources (no list): nothing to make unique.
  expect(makeUniqueEdit(doc([a], undefined as unknown as SourceView[]), [a])).toBeNull();
});
