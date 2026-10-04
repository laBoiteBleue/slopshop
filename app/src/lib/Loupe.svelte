<script lang="ts">
  // The eyedropper's loupe: the pixels around the pointer, magnified, the sampled one framed in
  // the middle and its color on the ring (as Photoshop's sampling ring shows it). Hidden where
  // there is no image. It follows the pointer without waiting: one patch asked at a time, the
  // latest position next.
  import { onDestroy } from "svelte";
  import {
    LOUPE_CELL,
    LOUPE_RADIUS,
    LOUPE_SIDE,
    LOUPE_SIZE,
    centerColor,
    loupePlacement,
  } from "./eyedropper";
  import { latestWins } from "./latest";

  let {
    x,
    y,
    patch,
  }: {
    /** The pointer, window pixels. */
    x: number;
    y: number;
    /**
     * The pixels shown around a window point: `LOUPE_SIDE²` RGBA (straight alpha), the one
     * under it in the middle; null off the image.
     */
    patch: (clientX: number, clientY: number) => Promise<Uint8ClampedArray<ArrayBuffer> | null>;
  } = $props();

  let canvas: HTMLCanvasElement;
  let shown = $state(false);
  let ring = $state<string | null>(null);
  let closed = false;

  const asks = latestWins(async ([cx, cy]: [number, number]) => {
    const pixels = await patch(cx, cy);
    if (!closed) draw(pixels);
  });

  function draw(pixels: Uint8ClampedArray<ArrayBuffer> | null) {
    shown = pixels !== null && pixels.length === LOUPE_SIDE * LOUPE_SIDE * 4;
    if (!pixels || !shown) return;
    ring = centerColor(pixels);
    // Null in tests (jsdom draws nothing).
    canvas.getContext("2d")?.putImageData(new ImageData(pixels, LOUPE_SIDE, LOUPE_SIDE), 0, 0);
  }

  $effect(() => asks.push([x, y]));

  onDestroy(() => {
    closed = true;
    asks.drop();
  });

  let view = $state({ width: window.innerWidth, height: window.innerHeight });
  const place = $derived(loupePlacement(x, y, view));
</script>

<svelte:window onresize={() => (view = { width: window.innerWidth, height: window.innerHeight })} />

<div
  class="loupe"
  class:shown
  aria-hidden="true"
  style:left="{place.left}px"
  style:top="{place.top}px"
  style:width="{LOUPE_SIZE}px"
  style:height="{LOUPE_SIZE}px"
  style:--ring={ring ?? "var(--panel)"}
  style:--cell="{LOUPE_CELL}px"
>
  <canvas bind:this={canvas} width={LOUPE_SIDE} height={LOUPE_SIDE}></canvas>
  <div class="grid"></div>
  <div
    class="center"
    style:left="{LOUPE_RADIUS * LOUPE_CELL}px"
    style:top="{LOUPE_RADIUS * LOUPE_CELL}px"
  ></div>
</div>

<style>
  /* Above the color picker's blocker and dialog: the loupe is part of the pointer. */
  .loupe {
    position: fixed;
    z-index: 450;
    display: none;
    box-sizing: content-box;
    overflow: hidden;
    border: 5px solid var(--ring);
    border-radius: 50%;
    /* Transparent pixels show over a checkerboard. */
    background: repeating-conic-gradient(#cccccc 0 25%, #ffffff 0 50%) 0 0 / var(--cell) var(--cell);
    box-shadow:
      0 0 0 1px #000000a0,
      inset 0 0 0 1px #000000a0,
      0 6px 18px #0008;
    pointer-events: none;
    /* Centered on its box, so the border does not shift it. */
    translate: -5px -5px;
  }

  .loupe.shown {
    display: block;
  }

  canvas {
    display: block;
    width: 100%;
    height: 100%;
    image-rendering: pixelated;
  }

  .grid {
    position: absolute;
    inset: 0;
    background-image:
      linear-gradient(to right, #00000026 1px, transparent 1px),
      linear-gradient(to bottom, #00000026 1px, transparent 1px);
    background-size: var(--cell) var(--cell);
  }

  .center {
    position: absolute;
    box-sizing: border-box;
    width: var(--cell);
    height: var(--cell);
    border: 1px solid #ffffff;
    outline: 1px solid #000000;
  }
</style>
