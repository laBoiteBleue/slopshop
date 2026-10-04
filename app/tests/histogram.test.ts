import { expect, test } from "vitest";
import { channelCounts, histogramPath, histogramStats } from "../src/lib/histogram";

const counts = (entries: Record<number, number>) =>
  Array.from({ length: 256 }, (_, v) => entries[v] ?? 0);

test("statistics as Photoshop gives them", () => {
  // Half the pixels at 0, half at 200.
  expect(histogramStats(counts({ 0: 10, 200: 10 }))).toEqual({
    mean: 100,
    stdDev: 100,
    median: 0,
    pixels: 20,
  });
  expect(histogramStats(counts({ 50: 1, 60: 2, 70: 1 }))?.median).toBe(60);
  expect(histogramStats(counts({}))).toBeNull();
});

test("Colors adds the three colors up; a channel is its own counts", () => {
  const view = {
    red: counts({ 1: 1 }),
    green: counts({ 1: 2 }),
    blue: counts({ 2: 3 }),
    luminosity: counts({ 1: 4 }),
    step: 1,
  };
  expect(channelCounts(view, "colors").slice(0, 3)).toEqual([0, 3, 3]);
  expect(channelCounts(view, "luminosity")).toBe(view.luminosity);
});

test("the curve fills its box, the tallest count at the top", () => {
  const path = histogramPath(counts({ 0: 4, 255: 2 }), 256, 100);
  expect(path.startsWith("M0,100L0.00,0.00L1.00,0.00")).toBe(true);
  expect(path).toContain("L255.00,50.00L256.00,50.00");
  expect(path.endsWith("L256,100Z")).toBe(true);
  // To a shared scale: lower.
  expect(histogramPath(counts({ 0: 4 }), 256, 100, 8)).toContain("L0.00,50.00");
  expect(histogramPath(counts({}), 256, 100)).toBe("");
});
