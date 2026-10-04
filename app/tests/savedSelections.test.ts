import { expect, test } from "vitest";
import {
  combinedRows,
  loadedRows,
  nextSelectionName,
  savedNamed,
  type Combination,
} from "../src/lib/savedSelections";

const format = (n: number) => `Selection ${n}`;

test("a new selection takes the first free default name", () => {
  expect(nextSelectionName([], format)).toBe("Selection 1");
  const saved = [
    { id: 1, name: "Selection 1" },
    { id: 4, name: "Hair" },
    { id: 5, name: "Selection 3" },
  ];
  expect(nextSelectionName(saved, format)).toBe("Selection 2");
});

test("a name already used is the saved selection it replaces, spaces around it aside", () => {
  const saved = [
    { id: 1, name: "Hair" },
    { id: 2, name: "Shirt" },
  ];
  expect(savedNamed(saved, "  Shirt ")).toEqual({ id: 2, name: "Shirt" });
  expect(savedNamed(saved, "shirt")).toBeNull();
  expect(savedNamed(saved, "Sky")).toBeNull();
});

test("a combination: replacing starts it, Shift / Alt add rows, another change ends it", () => {
  // Loaded by a plain click: one row.
  let rows = loadedRows(null, 1, null, 4, "replace");
  expect(rows).toEqual([{ id: 4, mode: "replace" }]);
  let combination: Combination = { document: 1, key: 10, rows };
  expect(combinedRows(combination, 1, 10)).toEqual(rows);
  // Shift then Alt on the selection it made: more rows.
  rows = loadedRows(combination, 1, 10, 5, "add");
  combination = { document: 1, key: 11, rows };
  rows = loadedRows(combination, 1, 11, 6, "subtract");
  expect(rows.map((r) => r.mode)).toEqual(["replace", "add", "subtract"]);
  // The same saved selection again: its row moves to the end, the last way.
  expect(loadedRows({ document: 1, key: 12, rows }, 1, 12, 5, "intersect")).toEqual([
    { id: 4, mode: "replace" },
    { id: 6, mode: "subtract" },
    { id: 5, mode: "intersect" },
  ]);
  // The selection changed otherwise (a tool, Deselect, undo), or another document: nothing.
  expect(combinedRows({ document: 1, key: 12, rows }, 1, 13)).toEqual([]);
  expect(combinedRows({ document: 1, key: 12, rows }, 2, 12)).toEqual([]);
  expect(combinedRows({ document: 1, key: null, rows }, 1, null)).toEqual([]);
  // Combining onto a selection made otherwise: only the row added.
  expect(loadedRows({ document: 1, key: 12, rows }, 1, 99, 7, "add")).toEqual([
    { id: 7, mode: "add" },
  ]);
});
