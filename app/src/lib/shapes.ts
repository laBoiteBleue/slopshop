// The shape tools (U, ADR 0041): Rectangle, Ellipse, Polygon and Line draw a vector layer. What
// a drag draws, the shape sent to the engine, and the outline shown while dragging.
import { hexToSrgb } from "./color";

export type ShapeKind = "rectangle" | "ellipse" | "polygon" | "line";

export type StrokeAlign = "inside" | "center" | "outside";

/** Where a shape's outline lies, in its layer's space (`crate::shape::GeometryDto`). */
export type ShapeGeometry =
  | {
      kind: "rectangle";
      rect: [number, number, number, number];
      radii: [number, number, number, number];
    }
  | { kind: "ellipse"; center: [number, number]; radii: [number, number] }
  | {
      kind: "polygon";
      center: [number, number];
      radius: number;
      sides: number;
      /** Inner corners' distance as a fraction of `radius`: a star; null: a polygon. */
      star: number | null;
      /** Of the first corner, degrees counterclockwise from the right (90: straight up). */
      rotation: number;
    }
  | { kind: "line"; from: [number, number]; to: [number, number] };

export type ShapeStroke = {
  /** sRGB-encoded RGBA in [0, 1]. */
  color: [number, number, number, number];
  /** Pixels. */
  width: number;
  align: StrokeAlign;
  cap: "butt" | "round" | "square";
  join: "miter" | "round" | "bevel";
  /** Dashes and gaps in turn, in stroke widths; empty: solid. */
  dashes: number[];
};

/** A vector shape (`crate::shape::ShapeDto`). */
export type Shape = {
  geometry: ShapeGeometry;
  /** sRGB-encoded RGBA in [0, 1]; null: no fill. */
  fill: [number, number, number, number] | null;
  stroke: ShapeStroke | null;
};

/** The shape tools' options bar. */
export type ShapeOptions = {
  /** `#rrggbb`, kept while there is no fill. */
  fill: string;
  filled: boolean;
  /** `#rrggbb`, kept while there is no stroke. The Line always draws with it. */
  stroke: string;
  stroked: boolean;
  strokeWidth: number;
  strokeAlign: StrokeAlign;
  /** Rectangle: its corners' radius, pixels. */
  radius: number;
  /** Polygon: its sides (or a star's points). */
  sides: number;
  star: boolean;
  /** A star's inner corners, as a share of its radius. */
  starRatio: number;
};

/** Whether `tool` is a shape tool, and which shape it draws. */
export function shapeKindOf(tool: string): ShapeKind | null {
  switch (tool) {
    case "shapeRectangle":
      return "rectangle";
    case "shapeEllipse":
      return "ellipse";
    case "shapePolygon":
      return "polygon";
    case "shapeLine":
      return "line";
    default:
      return null;
  }
}

export const MIN_SIDES = 3;
export const MAX_SIDES = 100;
export const MAX_STROKE_WIDTH = 1000;
export const MAX_RADIUS = 10000;

/** Filled with `fill` (the foreground color), no stroke, as Photoshop's first shape. */
export function defaultShapeOptions(fill: string): ShapeOptions {
  return {
    fill,
    filled: true,
    stroke: "#000000",
    stroked: false,
    strokeWidth: 3,
    strokeAlign: "inside",
    radius: 0,
    sides: 5,
    star: false,
    starRatio: 0.5,
  };
}

export type Box = { left: number; top: number; right: number; bottom: number };

/**
 * The box a drag from `from` to `to` draws (document pixels): a square with `square` (Shift),
 * centered on `from` with `centered` (Alt), as the Marquee's.
 */
export function dragBox(
  from: [number, number],
  to: [number, number],
  square: boolean,
  centered: boolean,
): Box {
  let dx = to[0] - from[0];
  let dy = to[1] - from[1];
  if (square) {
    const side = Math.max(Math.abs(dx), Math.abs(dy));
    dx = Math.sign(dx || 1) * side;
    dy = Math.sign(dy || 1) * side;
  }
  const [x0, y0] = centered ? [from[0] - dx, from[1] - dy] : from;
  const [x1, y1] = [from[0] + dx, from[1] + dy];
  return {
    left: Math.min(x0, x1),
    top: Math.min(y0, y1),
    right: Math.max(x0, x1),
    bottom: Math.max(y0, y1),
  };
}

/** The geometry a box-drawing tool draws in `box` with `options`. */
export function boxGeometry(
  kind: Exclude<ShapeKind, "line">,
  box: Box,
  options: ShapeOptions,
): ShapeGeometry {
  const width = box.right - box.left;
  const height = box.bottom - box.top;
  const center: [number, number] = [(box.left + box.right) / 2, (box.top + box.bottom) / 2];
  switch (kind) {
    case "rectangle": {
      const r = Math.min(Math.max(options.radius, 0), width / 2, height / 2);
      return { kind, rect: [box.left, box.top, box.right, box.bottom], radii: [r, r, r, r] };
    }
    case "ellipse":
      return { kind, center, radii: [width / 2, height / 2] };
    case "polygon":
      return {
        kind,
        center,
        radius: Math.min(width, height) / 2,
        sides: Math.round(Math.min(Math.max(options.sides, MIN_SIDES), MAX_SIDES)),
        star: options.star ? Math.min(Math.max(options.starRatio, 0.01), 1) : null,
        rotation: 90,
      };
  }
}

function rgba(hex: string): [number, number, number, number] {
  return [...hexToSrgb(hex), 1];
}

/**
 * The shape `geometry` drawn with `options`: its fill and stroke. A line has no inside: it is
 * drawn by its stroke, centered, whatever `stroked` says. `null` when it would show nothing.
 */
export function shapeOf(geometry: ShapeGeometry, options: ShapeOptions): Shape | null {
  const width = Math.min(Math.max(options.strokeWidth, 0), MAX_STROKE_WIDTH);
  const stroke = (color: string, align: StrokeAlign): ShapeStroke => ({
    color: rgba(color),
    width,
    align,
    cap: "butt",
    join: "miter",
    dashes: [],
  });
  if (geometry.kind === "line") {
    if (width <= 0) return null;
    return { geometry, fill: null, stroke: stroke(options.stroke, "center") };
  }
  const outlined = options.stroked && width > 0;
  if (!options.filled && !outlined) return null;
  return {
    geometry,
    fill: options.filled ? rgba(options.fill) : null,
    stroke: outlined ? stroke(options.stroke, options.strokeAlign) : null,
  };
}

/** Whether a geometry covers some area (a line some length): worth a layer. */
export function hasExtent(geometry: ShapeGeometry): boolean {
  switch (geometry.kind) {
    case "rectangle":
      return geometry.rect[2] > geometry.rect[0] && geometry.rect[3] > geometry.rect[1];
    case "ellipse":
      return geometry.radii[0] > 0 && geometry.radii[1] > 0;
    case "polygon":
      return geometry.radius > 0;
    case "line":
      return Math.hypot(geometry.to[0] - geometry.from[0], geometry.to[1] - geometry.from[1]) > 0;
  }
}

/** Points on a quarter turn of a rounded corner and on an ellipse, for the drawn outline. */
const ARC_STEPS = 8;
const ELLIPSE_STEPS = 64;

/**
 * The outline of `geometry` as points (its layer's space), closed unless it is a line: what
 * the tool shows while dragging, mapped to the screen point by point.
 */
export function outlinePoints(geometry: ShapeGeometry): [number, number][] {
  switch (geometry.kind) {
    case "line":
      return [geometry.from, geometry.to];
    case "ellipse": {
      const [cx, cy] = geometry.center;
      const [rx, ry] = geometry.radii;
      return Array.from({ length: ELLIPSE_STEPS }, (_, i) => {
        const a = (i / ELLIPSE_STEPS) * 2 * Math.PI;
        return [cx + rx * Math.cos(a), cy + ry * Math.sin(a)];
      });
    }
    case "polygon": {
      const corners = geometry.star === null ? geometry.sides : 2 * geometry.sides;
      return Array.from({ length: corners }, (_, i) => {
        const r =
          geometry.star !== null && i % 2 === 1 ? geometry.radius * geometry.star : geometry.radius;
        const a = ((geometry.rotation + (360 / corners) * i) * Math.PI) / 180;
        // Up is negative y.
        return [geometry.center[0] + r * Math.cos(a), geometry.center[1] - r * Math.sin(a)];
      });
    }
    case "rectangle": {
      const [l, t, r, b] = geometry.rect;
      const radius = geometry.radii[0];
      if (radius <= 0) {
        return [
          [l, t],
          [r, t],
          [r, b],
          [l, b],
        ];
      }
      // Corners clockwise from the top left, each around its own center.
      const corners: [number, number, number][] = [
        [l + radius, t + radius, 180],
        [r - radius, t + radius, 270],
        [r - radius, b - radius, 0],
        [l + radius, b - radius, 90],
      ];
      return corners.flatMap(([cx, cy, start]) =>
        Array.from({ length: ARC_STEPS + 1 }, (_, i): [number, number] => {
          const a = ((start + (90 * i) / ARC_STEPS) * Math.PI) / 180;
          return [cx + radius * Math.cos(a), cy + radius * Math.sin(a)];
        }),
      );
    }
  }
}

/** An SVG path of `points` (closed unless `open`), each mapped by `map`. */
export function svgPath(
  points: [number, number][],
  map: (x: number, y: number) => [number, number],
  open: boolean,
): string {
  const d = points
    .map(([x, y], i) => {
      const [vx, vy] = map(x, y);
      return `${i === 0 ? "M" : "L"}${vx.toFixed(2)} ${vy.toFixed(2)}`;
    })
    .join(" ");
  return open ? d : `${d} Z`;
}
