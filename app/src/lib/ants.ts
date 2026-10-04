// The marching ants drawn by the engine in the natively presented view (ADR 0024): when the
// engine presents to the window, it draws the selection's outline itself, from the selection's
// own coverage, so the ants are always in step with the image and cost no round trip. The UI only
// says whether, where (a drag's shift, Transform Selection's live matrix) and how they are
// shown, and presents the view again as often as the dashes must move. Frames over IPC (macOS,
// Linux) keep the SVG outline of `SelectionOutline`.

import { IDENTITY, andThen, translation } from "./affine";
import type { AntsRequest, Matrix } from "./engine";

/**
 * Time between two presents that move the dashes: a little under the 75 ms a dash takes to
 * advance by a pixel, so that none is skipped. The engine places them by the clock, whatever
 * the rate.
 */
export const ANTS_INTERVAL_MS = 66;

/** What decides whether the engine draws the ants, and where. */
export type AntsContext = {
  /** The engine presents to the window (else frames are drawn by the UI, with the SVG ants). */
  native: boolean;
  /** The document has a selection. */
  selected: boolean;
  /** Quick Mask shows the selection itself (and so do Select and Mask's other views). */
  hidden: boolean;
  /** Selected pixels floating in a drag move the outline with them, document pixels. */
  shift?: [number, number];
  /** Select > Transform Selection under way (or just applied): the outline is mapped by it. */
  matrix?: Matrix;
  reducedMotion: boolean;
};

/**
 * The ants to ask the engine for with each present, or null when it draws none. The selection
 * is placed by `matrix`, then moved by `shift`.
 */
export function antsRequest(context: AntsContext): AntsRequest | null {
  if (!context.native || !context.selected || context.hidden) return null;
  const mapped = context.matrix ?? IDENTITY;
  const [dx, dy] = context.shift ?? [0, 0];
  const matrix = dx === 0 && dy === 0 ? mapped : andThen(mapped, translation(dx, dy));
  return { matrix, march: !context.reducedMotion };
}

/** The system asks for less motion: the dashes stand still. */
export function prefersReducedMotion(): boolean {
  return typeof matchMedia === "function" && matchMedia("(prefers-reduced-motion: reduce)").matches;
}
