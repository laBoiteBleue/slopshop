import { expect, test } from "vitest";
import {
  MAJOR_MIN_CSS,
  MINOR_MIN_CSS,
  labelDigits,
  majorStep,
  rulerTicks,
} from "../src/lib/rulers";

test("labels are 1, 2 or 5 × 10ⁿ pixels apart, far enough on screen, never a fraction", () => {
  // 100%: 60 CSS pixels at least, so 100.
  expect(majorStep(1)).toBe(100);
  // 400%: 15 pixels at least, so 20.
  expect(majorStep(0.25)).toBe(20);
  // 6400%: under a pixel would do, but labels stay on whole pixels.
  expect(majorStep(1 / 64)).toBe(1);
  // Zoomed far out: 60 000 pixels at least, so 100 000.
  expect(majorStep(1000)).toBe(100_000);
  expect(majorStep(0)).toBe(1);
  expect(majorStep(Number.NaN)).toBe(1);
});

test("a ruler's ticks: labelled ones on the major steps, others between, at their places", () => {
  // 100%, the document's x = -30 at the ruler's start, 250 CSS pixels long.
  const ticks = rulerTicks(-30, 1, 250);
  expect(ticks.filter((t) => t.major).map((t) => t.value)).toEqual([0, 100, 200]);
  // 100 divided in 10: a tick every 10 pixels, from -30 to 220.
  expect(ticks.map((t) => t.value)).toEqual(Array.from({ length: 26 }, (_, i) => -30 + i * 10));
  expect(ticks.find((t) => t.value === 0)?.at).toBe(30);
});

test("ticks never crowd: labels and ticks keep their distances at any zoom", () => {
  for (const docPerCss of [1 / 64, 1 / 7, 0.5, 1, 3, 17, 250, 1000]) {
    const ticks = rulerTicks(123.4, docPerCss, 1200);
    const gaps = (list: { at: number }[]) => list.slice(1).map((t, i) => t.at - list[i].at);
    for (const gap of gaps(ticks)) expect(gap).toBeGreaterThanOrEqual(MINOR_MIN_CSS - 1e-9);
    for (const gap of gaps(ticks.filter((t) => t.major))) {
      expect(gap).toBeGreaterThanOrEqual(MAJOR_MIN_CSS - 1e-9);
    }
    expect(ticks.every((t) => t.at >= 0 && t.at <= 1200)).toBe(true);
  }
});

test("nothing to graduate draws no ticks", () => {
  expect(rulerTicks(0, 1, 0)).toEqual([]);
  expect(rulerTicks(Number.NaN, 1, 100)).toEqual([]);
  expect(rulerTicks(0, 0, 100)).toEqual([]);
});

test("in a length unit, labels may be fractions: 1, 2 or 5 × 10ⁿ, n below 0 too", () => {
  // 0.01 cm per CSS pixel: 0.6 cm at least, so 1 cm.
  expect(majorStep(0.01, false)).toBe(1);
  // 0.005 cm per CSS pixel: 0.3 cm at least, so 0.5 cm; pixels would say 1.
  expect(majorStep(0.005, false)).toBe(0.5);
  expect(majorStep(0.005)).toBe(1);
  expect(majorStep(0.0001, false)).toBeCloseTo(0.01, 12);
  expect(labelDigits(100)).toBe(0);
  expect(labelDigits(0.5)).toBe(1);
  expect(labelDigits(0.2)).toBe(1);
  expect(labelDigits(0.05)).toBe(2);
});

test("a centimeter ruler: labels every half centimeter, exact values", () => {
  // 0.005 cm per CSS pixel, from 0: labels at 0, 0.5, 1… every 100 CSS pixels.
  const ticks = rulerTicks(0, 0.005, 250, false);
  const labels = ticks.filter((t) => t.major);
  expect(labels.map((t) => t.value)).toEqual([0, 0.5, 1]);
  expect(labels.map((t) => t.at)).toEqual([0, 100, 200]);
});
