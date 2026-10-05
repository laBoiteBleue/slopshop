<script lang="ts">
  // The eyedropper's loupe, above right of the pointer: the pixels around it magnified, the
  // sampled one framed in the middle, Photoshop's sampling ring around them (the new color over
  // the current one) and the new color's value under it. Hidden off the image. Its pixels are cut
  // from a tile kept around the pointer (see loupeTile): it follows the pointer frame by frame,
  // always with the pixels of where it is drawn.
  import { onDestroy } from "svelte";
  import {
    centerHex,
    LOUPE_CELL,
    LOUPE_HALO,
    LOUPE_OUTER,
    LOUPE_RADIUS,
    LOUPE_RING,
    LOUPE_SIDE,
    LOUPE_SIZE,
    loupePlacement,
    type LoupeSource,
  } from "./eyedropper";
  import { loupeTiles, pointerMotion } from "./loupeTile";

  let {
    x,
    y,
    source,
    current = null,
  }: {
    /** The pointer, window pixels. */
    x: number;
    y: number;
    source: LoupeSource;
    /** The current color (`#rrggbb`), on the ring's lower half; the new color all round if none. */
    current?: string | null;
  } = $props();

  let canvas: HTMLCanvasElement;
  let shown = $state(false);
  /** The sampled pixel's color, null where nothing is shown. */
  let sampled = $state<string | null>(null);
  /** The pointer the pixels drawn are for: the loupe stays there until the next ones are. */
  let at = $state({ x: 0, y: 0 });
  let frame = 0;
  let closed = false;
  /** How the pointer moves over the document (see `pointerMotion`). */
  const motion = pointerMotion();

  const tiles = loupeTiles(
    (px, py, radius) => source.pixels(px, py, radius),
    () => schedule(),
  );

  function schedule() {
    if (!frame && !closed) frame = requestAnimationFrame(update);
  }

  // The pixels and the place are drawn in the same frame.
  function update() {
    frame = 0;
    const point = source.point(x, y);
    if (!point) {
      shown = false;
      return;
    }
    // The zoom: document pixels per screen pixel, where the pointer is.
    const beside = source.point(x + 8, y) ?? source.point(x - 8, y);
    const scale = beside ? Math.abs(beside[0] - point[0]) / 8 : null;
    const moving = motion.at(point, performance.now(), scale);
    const pixels = tiles.at(Math.floor(point[0]), Math.floor(point[1]), source.version, moving);
    if (!pixels) return;
    sampled = centerHex(pixels);
    // Null in tests (jsdom draws nothing).
    canvas.getContext("2d")?.putImageData(new ImageData(pixels, LOUPE_SIDE, LOUPE_SIDE), 0, 0);
    at = { x, y };
    shown = true;
  }

  $effect(() => {
    void [x, y, source];
    schedule();
  });

  onDestroy(() => {
    closed = true;
    cancelAnimationFrame(frame);
    tiles.drop();
  });

  let view = $state({ width: window.innerWidth, height: window.innerHeight });
  const place = $derived(loupePlacement(at.x, at.y, view));
  const ring = $derived(sampled ?? "var(--panel)");
</script>

<svelte:window onresize={() => (view = { width: window.innerWidth, height: window.innerHeight })} />

<div
  class="loupe"
  class:shown
  aria-hidden="true"
  style:left="{place.left}px"
  style:top="{place.top}px"
  style:width="{LOUPE_OUTER}px"
  style:--new={ring}
  style:--current={current ?? ring}
  style:--cell="{LOUPE_CELL}px"
  style:--size="{LOUPE_SIZE}px"
  style:--ring="{LOUPE_RING}px"
  style:--halo="{LOUPE_HALO}px"
>
  <div class="ring">
    <div class="pixels">
      <canvas bind:this={canvas} width={LOUPE_SIDE} height={LOUPE_SIDE}></canvas>
      <div class="grid"></div>
      <div
        class="center"
        style:left="{LOUPE_RADIUS * LOUPE_CELL}px"
        style:top="{LOUPE_RADIUS * LOUPE_CELL}px"
      ></div>
    </div>
  </div>
  {#if sampled}
    <span class="value">{sampled}</span>
  {/if}
</div>

<style>
  /* Above the color picker's blocker and dialog: the loupe is part of the pointer. */
  .loupe {
    position: fixed;
    z-index: 450;
    display: none;
    flex-direction: column;
    align-items: center;
    pointer-events: none;
  }

  .loupe.shown {
    display: flex;
  }

  /* Photoshop's sampling ring: the new color over the current one, in a neutral gray that reads
     on any image. */
  .ring {
    margin: var(--halo);
    padding: var(--ring);
    border-radius: 50%;
    background: linear-gradient(to bottom, var(--new) 50%, var(--current) 50%);
    box-shadow:
      0 0 0 1px #00000080,
      0 0 0 5px #8a8a8a,
      0 0 0 var(--halo) #00000090,
      0 10px 28px #000a;
  }

  .pixels {
    position: relative;
    width: var(--size);
    height: var(--size);
    overflow: hidden;
    border-radius: 50%;
    /* Transparent pixels show over a checkerboard. */
    background: repeating-conic-gradient(#cccccc 0 25%, #ffffff 0 50%) 0 0 / var(--cell) var(--cell);
    box-shadow: 0 0 0 1px #00000080;
  }

  canvas {
    display: block;
    width: 100%;
    height: 100%;
    image-rendering: pixelated;
  }

  /* The pixel grid, fading out towards the ring. */
  .grid {
    position: absolute;
    inset: 0;
    background-image:
      linear-gradient(to right, #0000002e 1px, transparent 1px),
      linear-gradient(to bottom, #0000002e 1px, transparent 1px);
    background-size: var(--cell) var(--cell);
    mask-image: radial-gradient(circle closest-side, #000 35%, transparent 100%);
  }

  .center {
    position: absolute;
    box-sizing: border-box;
    width: var(--cell);
    height: var(--cell);
    border: 1px solid #ffffff;
    outline: 1px solid #000000;
  }

  /* The new color's value, under the loupe. */
  .value {
    display: flex;
    gap: 6px;
    align-items: center;
    margin-top: 6px;
    padding: 2px 9px;
    border: 1px solid #000000;
    border-radius: 10px;
    background: #1b1b1bee;
    box-shadow: 0 2px 8px #0008;
    color: #e6e6e6;
    font:
      12px/16px ui-monospace,
      monospace;
    white-space: nowrap;
  }

  .value::before {
    content: "";
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--new);
    box-shadow: 0 0 0 1px #ffffff66;
  }
</style>
