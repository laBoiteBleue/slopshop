// Moving a dialog by its title bar, as on the desktop: it keeps its place when it opens again
// (Layer Style comes back after the color picker), for the session.

import type { Attachment } from "svelte/attachments";

/** How far a dialog was moved from its centered place, in CSS pixels. */
export type Offset = { x: number; y: number };

/** A box in the window, in CSS pixels. */
export type Box = { left: number; top: number; width: number; height: number };

/** The least of a dialog's title bar left in the window, so that it can be grabbed again. */
export const GRIP = 40;

/**
 * Keeps a dialog grabbable: its top edge within the window, and at least `GRIP` pixels of it
 * across. `centered` is the dialog's box where it opens, `view` the window's size.
 */
export function clampOffset(
  offset: Offset,
  centered: Box,
  view: { width: number; height: number },
): Offset {
  const clamp = (value: number, min: number, max: number) =>
    Math.min(Math.max(value, min), Math.max(min, max));
  return {
    x: clamp(offset.x, GRIP - centered.left - centered.width, view.width - GRIP - centered.left),
    y: clamp(offset.y, -centered.top, view.height - GRIP - centered.top),
  };
}

/** Where each dialog was left, by its name. */
const placed = new Map<string, Offset>();

/** Forgets where the dialogs were moved (tests). */
export function forgetDialogPlaces() {
  placed.clear();
}

/**
 * On a dialog's title bar: dragging it with the main button moves the dialog, which opens
 * again where it was left. `name` tells the dialogs apart.
 */
export function movable(name: string): Attachment<HTMLElement> {
  return (bar) => {
    const dialog = bar.closest("dialog");
    if (!dialog) return;
    let offset = placed.get(name) ?? { x: 0, y: 0 };
    const view = () => ({ width: window.innerWidth, height: window.innerHeight });
    /** The dialog's box without its move. */
    const centered = (): Box => {
      const box = dialog.getBoundingClientRect();
      return {
        left: box.left - offset.x,
        top: box.top - offset.y,
        width: box.width,
        height: box.height,
      };
    };
    const place = (next: Offset) => {
      offset = next;
      dialog.style.translate = offset.x || offset.y ? `${offset.x}px ${offset.y}px` : "";
    };
    place(offset);
    // The window may have shrunk since: once the dialog is shown, its place is kept within it.
    if (offset.x || offset.y) {
      requestAnimationFrame(() => place(clampOffset(offset, centered(), view())));
    }

    let drag: { pointerId: number; x: number; y: number; from: Offset; centered: Box } | null =
      null;
    const down = (e: PointerEvent) => {
      if (e.button !== 0) return;
      e.preventDefault();
      bar.setPointerCapture(e.pointerId);
      drag = {
        pointerId: e.pointerId,
        x: e.clientX,
        y: e.clientY,
        from: offset,
        centered: centered(),
      };
    };
    const move = (e: PointerEvent) => {
      if (drag?.pointerId !== e.pointerId) return;
      const next = { x: drag.from.x + e.clientX - drag.x, y: drag.from.y + e.clientY - drag.y };
      place(clampOffset(next, drag.centered, view()));
    };
    const up = (e: PointerEvent) => {
      if (drag?.pointerId !== e.pointerId) return;
      drag = null;
      placed.set(name, offset);
    };
    bar.addEventListener("pointerdown", down);
    bar.addEventListener("pointermove", move);
    bar.addEventListener("pointerup", up);
    bar.addEventListener("pointercancel", up);
    return () => {
      bar.removeEventListener("pointerdown", down);
      bar.removeEventListener("pointermove", move);
      bar.removeEventListener("pointerup", up);
      bar.removeEventListener("pointercancel", up);
    };
  };
}
