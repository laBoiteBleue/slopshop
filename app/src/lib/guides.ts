// Guides (View > Rulers), as in Photoshop: dragged out of a ruler, moved by dragging them with
// the Move tool, deleted by dragging them back out of the image. What a drag does to the
// document's guides; the engine keeps them (one undo entry per drag).

import type { Guide } from "./engine";

/** A guide is grabbed within this many CSS pixels of it. */
export const GUIDE_GRAB_CSS = 4;

/** Where a guide dragged to document coordinate `at` lands: on a whole pixel. */
export function guidePosition(at: number): number {
  return Math.round(at);
}

/** Whether a guide released at window point (`x`, `y`) is deleted: outside the image's area. */
export function dropsOut(
  x: number,
  y: number,
  area: { left: number; top: number; right: number; bottom: number },
): boolean {
  return x < area.left || y < area.top || x >= area.right || y >= area.bottom;
}

/**
 * The guides once a drag ends: guide `index` (`null`: a new one, out of a ruler) moved to
 * `guide`, or deleted when `guide` is null. `null` when nothing changes (nothing to undo).
 */
export function dropped(
  guides: Guide[],
  index: number | null,
  guide: Guide | null,
): Guide[] | null {
  if (index === null) return guide ? [...guides, guide] : null;
  const old = guides[index];
  if (!old) return null;
  if (!guide) return guides.filter((_, i) => i !== index);
  if (guide.vertical === old.vertical && guide.position === old.position) return null;
  return guides.map((g, i) => (i === index ? guide : g));
}
