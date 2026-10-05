// Gradient fill layers (Layer > New Fill Layer > Gradient): the gradient as the engine keeps it
// (stops, the opacity at each end, the shape, from `from` to `to` in the layer's content
// space), and Photoshop's settings for it, an angle and a scale. At 100 % a linear gradient
// spans the document along its angle, and a radial one reaches from its center to the
// document's edge along it.
//
// No imports but types: this module also runs under Node for its tests.

import type { Stop } from "./gradient";

export type GradientShape = "linear" | "radial";

/** A gradient fill as the engine has it (`LayerView.gradientFill`). */
export type GradientFill = {
  stops: Stop[];
  alpha: [number, number];
  shape: GradientShape;
  from: [number, number];
  to: [number, number];
};

/** The angle in degrees (counterclockwise from the right, as in Photoshop), the scale (1: 100 %). */
export type FillPlacement = { angle: number; scale: number };

type Size = { width: number; height: number };

const RADIANS = Math.PI / 180;

/** Half the document's extent along the direction of `radians`. */
function reach({ width, height }: Size, radians: number): number {
  return (Math.abs(Math.cos(radians)) * width + Math.abs(Math.sin(radians)) * height) / 2;
}

/** The gradient's center: a linear one's middle, a radial one's start. */
export function fillCenter(fill: GradientFill): [number, number] {
  if (fill.shape === "radial") return fill.from;
  return [(fill.from[0] + fill.to[0]) / 2, (fill.from[1] + fill.to[1]) / 2];
}

/**
 * The ends of a gradient of `shape` about `center`, placed by `placement` in a document of
 * `size`; never one point (half a pixel at least).
 */
export function fillEnds(
  size: Size,
  center: [number, number],
  shape: GradientShape,
  { angle, scale }: FillPlacement,
): { from: [number, number]; to: [number, number] } {
  const radians = angle * RADIANS;
  const r = Math.max(reach(size, radians) * scale, 0.5);
  // Document y goes down: a positive angle goes up.
  const [dx, dy] = [Math.cos(radians) * r, -Math.sin(radians) * r];
  const [cx, cy] = center;
  const to: [number, number] = [cx + dx, cy + dy];
  return { from: shape === "radial" ? [cx, cy] : [cx - dx, cy - dy], to };
}

/** The angle and the scale of `fill` in a document of `size` (`fillEnds`' inverse). */
export function fillPlacement(size: Size, fill: GradientFill): FillPlacement {
  const [dx, dy] = [fill.to[0] - fill.from[0], fill.to[1] - fill.from[1]];
  const radians = Math.atan2(-dy, dx);
  const length = Math.hypot(dx, dy);
  const r = fill.shape === "radial" ? length : length / 2;
  return { angle: radians / RADIANS, scale: r / Math.max(reach(size, radians), 1e-9) };
}

/** `fill` with `changes` to its shape, angle or scale, about its center. */
export function placed(
  size: Size,
  fill: GradientFill,
  changes: Partial<FillPlacement> & { shape?: GradientShape },
): GradientFill {
  const shape = changes.shape ?? fill.shape;
  const placement = { ...fillPlacement(size, fill), ...changes };
  return { ...fill, shape, ...fillEnds(size, fillCenter(fill), shape, placement) };
}

/** `fill` the other way round: its stops mirrored, its opacities swapped. */
export function reversed(fill: GradientFill): GradientFill {
  const stops = fill.stops.map(([location, r, g, b]): Stop => [4096 - location, r, g, b]).reverse();
  return { ...fill, stops, alpha: [fill.alpha[1], fill.alpha[0]] };
}

/**
 * A new gradient fill of `stops` across a document of `size`, as in Photoshop: linear, at 90°
 * (its start at the bottom), 100 %, opaque.
 */
export function newGradientFill(size: Size, stops: Stop[]): GradientFill {
  const center: [number, number] = [size.width / 2, size.height / 2];
  return {
    stops,
    alpha: [1, 1],
    shape: "linear",
    ...fillEnds(size, center, "linear", { angle: 90, scale: 1 }),
  };
}

/**
 * A CSS background drawing `fill` roughly, for a thumbnail: its colors and opacities along its
 * angle (linear) or from the middle (radial), at its scale.
 */
export function cssFill(size: Size, fill: GradientFill): string {
  const { angle, scale } = fillPlacement(size, fill);
  const [a0, a1] = fill.alpha;
  const color = ([location, r, g, b]: Stop, at: number) => {
    const alpha = a0 + ((a1 - a0) * location) / 4096;
    return `rgba(${r}, ${g}, ${b}, ${alpha.toFixed(3)}) ${at.toFixed(2)}%`;
  };
  if (fill.shape === "radial") {
    const parts = fill.stops.map((s) => color(s, (s[0] / 4096) * scale * 100));
    return `radial-gradient(circle closest-side, ${parts.join(", ")})`;
  }
  const parts = fill.stops.map((s) => color(s, 50 + (s[0] / 4096 - 0.5) * scale * 100));
  // CSS angles go clockwise from the top.
  return `linear-gradient(${(90 - angle).toFixed(2)}deg, ${parts.join(", ")})`;
}
