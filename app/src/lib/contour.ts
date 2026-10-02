// Outlines of a binary mask, for drawing (Object Selection's hover): marching squares between
// cell centers, then corner cutting so that the cells' staircase reads as a smooth line.

/** Edge midpoints of the block whose top-left cell center is (x, y), in grid units. */
const EDGES = {
  top: (x: number, y: number): [number, number] => [x + 1, y + 0.5],
  right: (x: number, y: number): [number, number] => [x + 1.5, y + 1],
  bottom: (x: number, y: number): [number, number] => [x + 1, y + 1.5],
  left: (x: number, y: number): [number, number] => [x + 0.5, y + 1],
};
type Edge = keyof typeof EDGES;

/**
 * The segments of each marching-squares case, by the block's corners inside: top-left 8,
 * top-right 4, bottom-right 2, bottom-left 1. Saddles (5, 10) keep their two inside corners apart.
 */
const CASES: Edge[][][] = [
  [],
  [["left", "bottom"]],
  [["bottom", "right"]],
  [["left", "right"]],
  [["top", "right"]],
  [
    ["top", "right"],
    ["left", "bottom"],
  ],
  [["top", "bottom"]],
  [["top", "left"]],
  [["top", "left"]],
  [["top", "bottom"]],
  [
    ["top", "left"],
    ["bottom", "right"],
  ],
  [["top", "right"]],
  [["left", "right"]],
  [["bottom", "right"]],
  [["left", "bottom"]],
  [],
];

/**
 * The outlines of the cells of a `side`² mask (row-major, inside where non-zero) as closed
 * polylines in grid units (cell `i` spans `[i, i + 1]`), each smoothed by `smoothing` rounds of
 * Chaikin's corner cutting.
 */
export function outline(mask: Uint8Array, side: number, smoothing = 2): [number, number][][] {
  const inside = (x: number, y: number) =>
    x >= 0 && y >= 0 && x < side && y < side && mask[y * side + x] !== 0;
  // Each edge midpoint is shared by two blocks, each giving it one segment: a point has two
  // neighbors, so the segments chain into loops.
  const key = ([x, y]: [number, number]) => `${x},${y}`;
  const links = new Map<string, { point: [number, number]; next: string[] }>();
  const link = (a: [number, number], b: [number, number]) => {
    for (const [p, q] of [
      [a, b],
      [b, a],
    ]) {
      const k = key(p);
      const entry = links.get(k) ?? { point: p, next: [] };
      entry.next.push(key(q));
      links.set(k, entry);
    }
  };
  for (let y = -1; y < side; y++) {
    for (let x = -1; x < side; x++) {
      const index =
        (inside(x, y) ? 8 : 0) |
        (inside(x + 1, y) ? 4 : 0) |
        (inside(x + 1, y + 1) ? 2 : 0) |
        (inside(x, y + 1) ? 1 : 0);
      for (const [from, to] of CASES[index]) link(EDGES[from](x, y), EDGES[to](x, y));
    }
  }
  const loops: [number, number][][] = [];
  const seen = new Set<string>();
  for (const [start, entry] of links) {
    if (seen.has(start)) continue;
    const loop: [number, number][] = [];
    let previous = "";
    let current = start;
    let node = entry;
    while (!seen.has(current)) {
      seen.add(current);
      loop.push(node.point);
      const next = node.next.find((k) => k !== previous) ?? node.next[0];
      previous = current;
      current = next;
      const following = links.get(current);
      if (!following) break;
      node = following;
    }
    if (loop.length > 2) loops.push(smooth(loop, smoothing));
  }
  return loops;
}

/** Chaikin's corner cutting on a closed polyline, `rounds` times. */
function smooth(loop: [number, number][], rounds: number): [number, number][] {
  let points = loop;
  for (let r = 0; r < rounds; r++) {
    const next: [number, number][] = [];
    for (let i = 0; i < points.length; i++) {
      const [ax, ay] = points[i];
      const [bx, by] = points[(i + 1) % points.length];
      next.push([0.75 * ax + 0.25 * bx, 0.75 * ay + 0.25 * by]);
      next.push([0.25 * ax + 0.75 * bx, 0.25 * ay + 0.75 * by]);
    }
    points = next;
  }
  return points;
}
