import { describe, expect, test } from "vitest";
import {
  cssFill,
  fillCenter,
  fillEnds,
  fillPlacement,
  newGradientFill,
  placed,
  reversed,
  type GradientFill,
} from "../src/lib/gradientFill";
import type { Stop } from "../src/lib/gradient";

const SIZE = { width: 400, height: 300 };
const STOPS: Stop[] = [
  [0, 0, 0, 0],
  [1000, 255, 0, 0],
  [4096, 255, 255, 255],
];

const close = (a: number[], b: number[]) => a.forEach((v, i) => expect(v).toBeCloseTo(b[i], 6));

describe("a gradient fill's place", () => {
  test("a new one is Photoshop's: linear at 90°, from the bottom to the top, opaque", () => {
    const fill = newGradientFill(SIZE, STOPS);
    expect(fill.shape).toBe("linear");
    expect(fill.alpha).toEqual([1, 1]);
    expect(fill.stops).toEqual(STOPS);
    close(fill.from, [200, 300]);
    close(fill.to, [200, 0]);
  });

  test("at 0° a linear gradient spans the document's width, a radial one half of it", () => {
    const linear = fillEnds(SIZE, [200, 150], "linear", { angle: 0, scale: 1 });
    close(linear.from, [0, 150]);
    close(linear.to, [400, 150]);
    const radial = fillEnds(SIZE, [200, 150], "radial", { angle: 0, scale: 0.5 });
    close(radial.from, [200, 150]);
    close(radial.to, [300, 150]);
  });

  test("the angle and the scale are read back from the ends", () => {
    for (const shape of ["linear", "radial"] as const) {
      for (const angle of [-135, -30, 0, 45, 90, 170]) {
        const ends = fillEnds(SIZE, [120, 80], shape, { angle, scale: 1.7 });
        const placement = fillPlacement(SIZE, { stops: STOPS, alpha: [1, 1], shape, ...ends });
        expect(placement.angle).toBeCloseTo(angle, 6);
        expect(placement.scale).toBeCloseTo(1.7, 6);
      }
    }
  });

  test("a change of angle, scale or style keeps the center", () => {
    const fill: GradientFill = { ...newGradientFill(SIZE, STOPS), from: [50, 60], to: [150, 60] };
    expect(fillCenter(fill)).toEqual([100, 60]);
    const turned = placed(SIZE, fill, { angle: 90 });
    close(fillCenter(turned), [100, 60]);
    expect(fillPlacement(SIZE, turned).angle).toBeCloseTo(90, 6);
    // The scale is kept: 100 of a width of 400 is a quarter; turned, a quarter of the height.
    close(turned.from, [100, 60 + 37.5]);
    const radial = placed(SIZE, fill, { shape: "radial" });
    expect(radial.shape).toBe("radial");
    close(radial.from, [100, 60]);
    expect(fillPlacement(SIZE, radial).scale).toBeCloseTo(0.25, 6);
    const larger = placed(SIZE, fill, { scale: 0.5 });
    close(larger.from, [0, 60]);
    close(larger.to, [200, 60]);
  });

  test("a scale of 0 still goes somewhere", () => {
    const ends = fillEnds(SIZE, [10, 10], "linear", { angle: 0, scale: 0 });
    expect(ends.from).not.toEqual(ends.to);
  });
});

test("reversed: the stops mirrored, the opacities swapped, the place kept", () => {
  const fill: GradientFill = { ...newGradientFill(SIZE, STOPS), alpha: [1, 0.25] };
  const back = reversed(fill);
  expect(back.stops).toEqual([
    [0, 255, 255, 255],
    [3096, 255, 0, 0],
    [4096, 0, 0, 0],
  ]);
  expect(back.alpha).toEqual([0.25, 1]);
  expect(back.from).toEqual(fill.from);
  expect(reversed(back)).toEqual(fill);
});

test("the thumbnail's CSS follows the angle, the scale and the opacities", () => {
  const fill: GradientFill = { ...newGradientFill(SIZE, STOPS), alpha: [1, 0] };
  expect(cssFill(SIZE, fill)).toBe(
    "linear-gradient(0.00deg, rgba(0, 0, 0, 1.000) 0.00%, rgba(255, 0, 0, 0.756) 24.41%, " +
      "rgba(255, 255, 255, 0.000) 100.00%)",
  );
  const radial = placed(SIZE, fill, { shape: "radial", scale: 0.5 });
  expect(cssFill(SIZE, radial)).toBe(
    "radial-gradient(circle closest-side, rgba(0, 0, 0, 1.000) 0.00%, " +
      "rgba(255, 0, 0, 0.756) 12.21%, rgba(255, 255, 255, 0.000) 50.00%)",
  );
});
