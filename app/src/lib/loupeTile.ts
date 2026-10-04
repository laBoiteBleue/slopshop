// The eyedropper's loupe follows the pointer without asking the engine at each move: it keeps a
// tile of the pixels around the pointer and cuts the loupe from it, asking for the next tile
// (one at a time) when the pointer nears the edge of the one kept.

import { LOUPE_RADIUS, LOUPE_SIDE } from "./eyedropper";

/** Document pixels on each side of the tile's center: 129² pixels, 66 KB. */
export const TILE_RADIUS = 64;
/** The next tile is asked once the loupe comes this close (document pixels) to the tile's edge. */
export const TILE_MARGIN = 16;

/** `(2 × radius + 1)²` RGBA pixels centered on document pixel (`x`, `y`). */
export type Tile = { x: number; y: number; radius: number; pixels: Uint8ClampedArray };

/** Whether `tile` holds every pixel within `radius` of document pixel (`x`, `y`). */
export function covers(tile: Tile, x: number, y: number, radius: number): boolean {
  return (
    Math.abs(x - tile.x) + radius <= tile.radius && Math.abs(y - tile.y) + radius <= tile.radius
  );
}

/** The loupe's `LOUPE_SIDE²` pixels around document pixel (`x`, `y`), cut from `tile` (which covers them). */
export function cut(tile: Tile, x: number, y: number): Uint8ClampedArray<ArrayBuffer> {
  const side = 2 * tile.radius + 1;
  const out = new Uint8ClampedArray(LOUPE_SIDE * LOUPE_SIDE * 4);
  const left = x - LOUPE_RADIUS - (tile.x - tile.radius);
  const top = y - LOUPE_RADIUS - (tile.y - tile.radius);
  for (let row = 0; row < LOUPE_SIDE; row++) {
    const start = ((top + row) * side + left) * 4;
    out.set(tile.pixels.subarray(start, start + LOUPE_SIDE * 4), row * LOUPE_SIDE * 4);
  }
  return out;
}

/**
 * The tiles of one loupe: `fetch(x, y, radius)` gives the pixels around a document pixel (null if
 * none), `onready()` is told when a tile arrives.
 */
export function loupeTiles(
  fetch: (x: number, y: number, radius: number) => Promise<Uint8ClampedArray | null>,
  onready: () => void,
) {
  let kept: { tile: Tile; version: unknown } | null = null;
  let asking = false;
  let dropped = false;
  /** The last ask that brought nothing: not asked again until the pointer moves. */
  let failed: [number, number, unknown] | null = null;

  function ask(x: number, y: number, version: unknown) {
    if (asking || (failed && failed[0] === x && failed[1] === y && failed[2] === version)) return;
    asking = true;
    failed = [x, y, version];
    fetch(x, y, TILE_RADIUS)
      .then((pixels) => {
        const side = 2 * TILE_RADIUS + 1;
        if (pixels && pixels.length === side * side * 4) {
          kept = { tile: { x, y, radius: TILE_RADIUS, pixels }, version };
          failed = null;
        }
      })
      .catch(() => undefined)
      .finally(() => {
        asking = false;
        if (!dropped) onready();
      });
  }

  return {
    /**
     * The loupe's pixels around document pixel (`x`, `y`) of `version`, or null until they
     * arrive (`onready` is told); asks for the next tile when needed.
     */
    at(x: number, y: number, version: unknown): Uint8ClampedArray<ArrayBuffer> | null {
      if (kept && kept.version !== version) kept = null;
      const tile = kept?.tile;
      if (!tile || !covers(tile, x, y, LOUPE_RADIUS + TILE_MARGIN)) ask(x, y, version);
      return tile && covers(tile, x, y, LOUPE_RADIUS) ? cut(tile, x, y) : null;
    },
    /** The loupe closed: nothing more is told. */
    drop() {
      dropped = true;
      kept = null;
    },
  };
}
