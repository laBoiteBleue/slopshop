// Paragraph text's box with the Type tool (ADR 0041): the box a drag draws, and its handles
// resizing it while the text is edited. The box is kept in the layer's own space (its top left
// `position` and its `size`); the engine gives its corners in document pixels, through which a
// point on the image is brought back to the box's own space, so that a rotated or skewed text
// layer is resized along its own edges.

export type Point = [number, number];

/** A box's corners in document pixels: top left, top right, bottom right, bottom left. */
export type Corners = [Point, Point, Point, Point];

/** A box in the layer's own space. */
export type Frame = { position: Point; size: Point };

/** The smallest box, in the layer's pixels (Photoshop's boxes cannot be emptied either). */
export const MIN_FRAME = 1;

/** A drag shorter than this (screen pixels) is a click: point text. */
export const DRAG_THRESHOLD = 3;

/** The box a drag from `a` to `b` (document pixels) draws, for a new paragraph text. */
export function frameOfDrag(a: Point, b: Point): Frame {
  const left = Math.min(a[0], b[0]);
  const top = Math.min(a[1], b[1]);
  return {
    position: [left, top],
    size: [Math.max(Math.abs(b[0] - a[0]), MIN_FRAME), Math.max(Math.abs(b[1] - a[1]), MIN_FRAME)],
  };
}

/**
 * Where document point `p` lies in the box's own space, from its top left corner: the box's
 * corners map `[0, size]` to the document, so the inverse of that map. `null` for a flattened
 * box.
 */
export function boxPoint(corners: Corners, size: Point, p: Point): Point | null {
  const [tl, tr, , bl] = corners;
  // The box's axes per layer pixel.
  const ux = (tr[0] - tl[0]) / size[0];
  const uy = (tr[1] - tl[1]) / size[0];
  const vx = (bl[0] - tl[0]) / size[1];
  const vy = (bl[1] - tl[1]) / size[1];
  const det = ux * vy - uy * vx;
  if (!Number.isFinite(det) || Math.abs(det) < 1e-12) return null;
  const dx = p[0] - tl[0];
  const dy = p[1] - tl[1];
  return [(dx * vy - dy * vx) / det, (ux * dy - uy * dx) / det];
}

/** Whether document point `p` is in the box. */
export function inFrame(corners: Corners, size: Point, p: Point): boolean {
  const local = boxPoint(corners, size, p);
  if (!local) return false;
  return local[0] >= 0 && local[0] <= size[0] && local[1] >= 0 && local[1] <= size[1];
}

/** The handles, clockwise from the top left corner: corners and the middles of the edges. */
export const HANDLES = 8;

/** Handle `i`'s place in document pixels. */
export function handlePoint(corners: Corners, i: number): Point {
  const corner = corners[i / 2];
  if (i % 2 === 0) return corner;
  const a = corners[(i - 1) / 2];
  const b = corners[((i + 1) / 2) % 4];
  return [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
}

/** Which edges handle `i` moves: left, top, right, bottom. */
function edgesOf(i: number): [boolean, boolean, boolean, boolean] {
  return [i === 0 || i === 6 || i === 7, i <= 2, i >= 2 && i <= 4, i >= 4 && i <= 6];
}

/**
 * The box once handle `i` is dragged to document point `p`, from `start` (the box when the
 * drag began, and its corners then): the edges it holds follow the point, never closer than
 * the smallest box to the opposite edge.
 */
export function resizeFrame(start: Frame, corners: Corners, i: number, p: Point): Frame {
  const local = boxPoint(corners, start.size, p);
  if (!local) return start;
  const [w, h] = start.size;
  const [moveLeft, moveTop, moveRight, moveBottom] = edgesOf(i);
  const left = moveLeft ? Math.min(local[0], w - MIN_FRAME) : 0;
  const top = moveTop ? Math.min(local[1], h - MIN_FRAME) : 0;
  const right = moveRight ? Math.max(local[0], MIN_FRAME) : w;
  const bottom = moveBottom ? Math.max(local[1], MIN_FRAME) : h;
  return {
    position: [start.position[0] + left, start.position[1] + top],
    size: [right - left, bottom - top],
  };
}
