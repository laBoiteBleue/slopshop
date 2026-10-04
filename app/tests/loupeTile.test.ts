import { expect, test, vi } from "vitest";
import { LOUPE_RADIUS, LOUPE_SIDE } from "../src/lib/eyedropper";
import { TILE_MARGIN, TILE_RADIUS, covers, cut, loupeTiles, type Tile } from "../src/lib/loupeTile";

const SIDE = 2 * TILE_RADIUS + 1;

/** A tile around (`cx`, `cy`) whose every pixel holds its own document coordinates (mod 256). */
function tileAround(cx: number, cy: number): Uint8ClampedArray<ArrayBuffer> {
  const pixels = new Uint8ClampedArray(SIDE * SIDE * 4);
  for (let row = 0; row < SIDE; row++) {
    for (let col = 0; col < SIDE; col++) {
      const x = cx - TILE_RADIUS + col;
      const y = cy - TILE_RADIUS + row;
      pixels.set([x & 255, y & 255, 7, 255], (row * SIDE + col) * 4);
    }
  }
  return pixels;
}

/** The document pixel at the middle of a loupe patch. */
function middle(patch: Uint8ClampedArray): [number, number] {
  const i = (LOUPE_RADIUS * LOUPE_SIDE + LOUPE_RADIUS) * 4;
  return [patch[i], patch[i + 1]];
}

test("a tile covers the pixels within its radius of its center", () => {
  const tile: Tile = { x: 100, y: 100, radius: 10, pixels: new Uint8ClampedArray() };
  expect(covers(tile, 100, 100, 10)).toBe(true);
  expect(covers(tile, 104, 96, 6)).toBe(true);
  expect(covers(tile, 105, 100, 6)).toBe(false);
  expect(covers(tile, 100, 95, 6)).toBe(false);
});

test("the loupe's pixels are cut from the tile around the point asked", () => {
  const tile = { x: 100, y: 50, radius: TILE_RADIUS, pixels: tileAround(100, 50) };
  const patch = cut(tile, 130, 40);
  expect(patch.length).toBe(LOUPE_SIDE * LOUPE_SIDE * 4);
  expect(middle(patch)).toEqual([130, 40]);
  // The top left pixel of the patch.
  expect([patch[0], patch[1]]).toEqual([130 - LOUPE_RADIUS, 40 - LOUPE_RADIUS]);
});

/** Tiles from a fetch that answers when told, each asked tile's center kept. */
function setup() {
  const answers: (() => void)[] = [];
  const fetch = vi.fn(
    (x: number, y: number, _radius: number) =>
      new Promise<Uint8ClampedArray | null>((resolve) =>
        answers.push(() => resolve(tileAround(x, y))),
      ),
  );
  const onready = vi.fn();
  const tiles = loupeTiles(fetch, onready);
  /** The oldest ask answered, then its `onready` awaited. */
  const answer = async () => {
    answers.shift()?.();
    await vi.waitFor(() => expect(onready).toHaveBeenCalled());
    onready.mockClear();
  };
  return { fetch, onready, tiles, answers, answer };
}

test("moves within the tile kept ask the engine nothing", async () => {
  const { fetch, tiles, answer } = setup();
  expect(tiles.at(200, 200, "v1")).toBeNull();
  expect(fetch).toHaveBeenCalledWith(200, 200, TILE_RADIUS);
  await answer();
  for (const [x, y] of [
    [200, 200],
    [210, 195],
    [240, 170],
  ]) {
    const patch = tiles.at(x, y, "v1");
    expect(patch && middle(patch)).toEqual([x, y]);
  }
  expect(fetch).toHaveBeenCalledTimes(1);
});

test("the next tile is asked near the edge, the one kept used meanwhile", async () => {
  const { fetch, tiles, answer } = setup();
  tiles.at(200, 200, "v1");
  await answer();
  const near = 200 + TILE_RADIUS - LOUPE_RADIUS - TILE_MARGIN + 1;
  const patch = tiles.at(near, 200, "v1");
  expect(patch && middle(patch)).toEqual([near & 255, 200]);
  expect(fetch).toHaveBeenLastCalledWith(near, 200, TILE_RADIUS);
  // One ask at a time.
  tiles.at(near + 1, 200, "v1");
  expect(fetch).toHaveBeenCalledTimes(2);
  await answer();
  expect(tiles.at(near + 1, 200, "v1")).not.toBeNull();
  expect(fetch).toHaveBeenCalledTimes(2);
});

test("past the tile kept, nothing is shown until the next one arrives", async () => {
  const { tiles, answer } = setup();
  tiles.at(200, 200, "v1");
  await answer();
  expect(tiles.at(600, 200, "v1")).toBeNull();
  await answer();
  expect(tiles.at(600, 200, "v1")).not.toBeNull();
});

test("a document changed drops the tile kept", async () => {
  const { fetch, tiles, answer } = setup();
  tiles.at(200, 200, "v1");
  await answer();
  expect(tiles.at(200, 200, "v2")).toBeNull();
  expect(fetch).toHaveBeenCalledTimes(2);
  await answer();
  expect(tiles.at(200, 200, "v2")).not.toBeNull();
});

test("a tile asked before the document changed is not shown after", async () => {
  const { fetch, tiles, answer } = setup();
  tiles.at(200, 200, "v1");
  await answer();
  // Asked for v1, answered once v2 is shown: asked again.
  tiles.at(600, 200, "v1");
  await answer();
  expect(tiles.at(600, 200, "v2")).toBeNull();
  expect(fetch).toHaveBeenCalledTimes(3);
});

test("an ask that brings nothing is not repeated until the pointer moves", async () => {
  const fetch = vi.fn(async () => null);
  const onready = vi.fn();
  const tiles = loupeTiles(fetch, onready);
  tiles.at(10, 10, "v1");
  await vi.waitFor(() => expect(onready).toHaveBeenCalledTimes(1));
  tiles.at(10, 10, "v1");
  expect(fetch).toHaveBeenCalledTimes(1);
  tiles.at(11, 10, "v1");
  expect(fetch).toHaveBeenCalledTimes(2);
});

test("once dropped, an answer tells nothing", async () => {
  const { tiles, onready, answers } = setup();
  tiles.at(200, 200, "v1");
  tiles.drop();
  answers.shift()?.();
  await new Promise((done) => setTimeout(done));
  expect(onready).not.toHaveBeenCalled();
});
