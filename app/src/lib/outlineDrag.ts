// Dragging the selection's outline alone (see `SelectionDrag`).

/**
 * The whole document pixels a dragged outline moves by for a pointer move of (`dx`, `dy`)
 * document pixels; `constrain` (Shift): along the nearest multiple of 45°, as in Photoshop.
 */
export function outlineOffset(dx: number, dy: number, constrain: boolean): [number, number] {
  if (constrain) {
    const step = Math.PI / 4;
    const angle = Math.round(Math.atan2(dy, dx) / step) * step;
    const length = Math.hypot(dx, dy);
    [dx, dy] = [Math.cos(angle) * length, Math.sin(angle) * length];
  }
  // `+ 0`: no negative zero.
  return [Math.round(dx) + 0, Math.round(dy) + 0];
}
