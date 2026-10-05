// The eyedropper's loupe follows the pointer without asking the engine at each move: it keeps a
// tile of the pixels around the pointer and cuts the loupe from it, asking for the next tile
// (one at a time) when the pointer nears the edge of the one kept. Zoomed out or moving fast,
// the pointer crosses many document pixels per frame: the tile asked is then larger and placed
// ahead of the pointer, where it goes (`nextTile`), so that the loupe seldom waits for one.

import { LOUPE_RADIUS, LOUPE_SIDE } from "./eyedropper";

/** Document pixels on each side of the tile's center: 129² pixels, 66 KB. */
export const TILE_RADIUS = 64;
/** The next tile is asked once the loupe comes this close (document pixels) to the tile's edge. */
export const TILE_MARGIN = 16;
/** The largest tile asked: 513² pixels, 1 MB (the engine composites it in about 10 ms). */
export const MAX_TILE_RADIUS = 256;
/** How far ahead the pointer's motion is followed, in milliseconds (an ask and its answer). */
export const LEAD_MS = 100;

/** How the pointer moves: document pixels per millisecond, and per screen pixel (the zoom). */
export type Motion = { velocity: [number, number]; scale: number };

const STILL: Motion = { velocity: [0, 0], scale: 1 };

/**
 * The tile to ask for the loupe at document pixel (`x`, `y`) moving by `motion`: its center, where
 * the pointer will be `LEAD_MS` from now (the loupe still in it), and its radius, as many screen
 * pixels as `TILE_RADIUS` document pixels at 100 % and enough for that lead, within
 * `TILE_RADIUS` and `MAX_TILE_RADIUS`. Faster than one tile can follow (far zoomed out on a large
 * image), a small tile where the pointer goes, quick to come: the loupe shows the last pixels
 * meanwhile.
 */
export function nextTile(
  x: number,
  y: number,
  motion: Motion,
): { x: number; y: number; radius: number } {
  const lead = motion.velocity.map((v) => Math.round(v * LEAD_MS));
  const needed = Math.max(
    Math.ceil(TILE_RADIUS * motion.scale),
    ...lead.map((d) => Math.abs(d) + LOUPE_RADIUS + TILE_MARGIN),
  );
  const ahead = Math.max(...lead.map((d) => Math.abs(d) + LOUPE_RADIUS + TILE_MARGIN));
  if (ahead > MAX_TILE_RADIUS) return { x: x + lead[0], y: y + lead[1], radius: TILE_RADIUS };
  const radius = Math.min(Math.max(TILE_RADIUS, needed), MAX_TILE_RADIUS);
  // The loupe where it is now stays within the tile.
  const room = radius - LOUPE_RADIUS - TILE_MARGIN;
  const [dx, dy] = lead.map((d) => Math.min(Math.max(d, -room), room));
  return { x: x + dx, y: y + dy, radius };
}

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

  function ask(x: number, y: number, version: unknown, motion: Motion) {
    if (asking || (failed && failed[0] === x && failed[1] === y && failed[2] === version)) return;
    asking = true;
    failed = [x, y, version];
    const next = nextTile(x, y, motion);
    fetch(next.x, next.y, next.radius)
      .then((pixels) => {
        const side = 2 * next.radius + 1;
        if (pixels && pixels.length === side * side * 4) {
          kept = { tile: { ...next, pixels }, version };
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
     * arrive (`onready` is told); asks for the next tile when needed: when the loupe nears the
     * edge of the one kept, or will have left it `LEAD_MS` from now as the pointer moves.
     */
    at(
      x: number,
      y: number,
      version: unknown,
      motion: Motion = STILL,
    ): Uint8ClampedArray<ArrayBuffer> | null {
      if (kept && kept.version !== version) kept = null;
      const tile = kept?.tile;
      const ahead = nextTile(x, y, { ...motion, scale: 0 });
      if (
        !tile ||
        !covers(tile, x, y, LOUPE_RADIUS + TILE_MARGIN) ||
        !covers(tile, ahead.x, ahead.y, LOUPE_RADIUS)
      ) {
        ask(x, y, version, motion);
      }
      return tile && covers(tile, x, y, LOUPE_RADIUS) ? cut(tile, x, y) : null;
    },
    /** The loupe closed: nothing more is told. */
    drop() {
      dropped = true;
      kept = null;
    },
  };
}

/**
 * The pointer's motion from where it is seen (document points, at times in milliseconds): its
 * velocity smoothed over the last moves, and the last zoom known (`scale`, null where it cannot
 * be measured). A pause of `REST_MS` or more starts over still.
 */
export function pointerMotion() {
  const REST_MS = 150;
  let last: { point: [number, number]; time: number } | null = null;
  let motion: Motion = STILL;
  return {
    at(point: [number, number], time: number, scale: number | null): Motion {
      let velocity: [number, number] = [0, 0];
      const dt = last ? time - last.time : 0;
      if (last && dt > 0 && dt < REST_MS) {
        const now = [0, 1].map((c) => (point[c] - last!.point[c]) / dt);
        // Half the last move, half the ones before: steady, yet turning quickly.
        velocity = [0, 1].map((c) => (now[c] + motion.velocity[c]) / 2) as [number, number];
      } else if (last && dt === 0) {
        velocity = motion.velocity;
      }
      last = { point, time };
      motion = { velocity, scale: scale ?? motion.scale };
      return motion;
    },
  };
}
