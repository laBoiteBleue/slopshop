import { expect, test } from "vitest";
import { nextSelectionName, savedNamed } from "../src/lib/savedSelections";

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
