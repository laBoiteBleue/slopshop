<script lang="ts" module>
  /** The object under the pointer: SAM's mask (`side`² cells, 255 inside) over `region`. */
  export type ObjectHover = {
    side: number;
    mask: Uint8Array;
    /** Document pixels `[x, y, width, height]`. */
    region: [number, number, number, number];
  };
</script>

<script lang="ts">
  // Object Selection (W, ADR 0025), as in Photoshop: the object under the pointer lights up
  // (SAM's coarse mask, decoded in milliseconds on the image it has already seen); a click
  // selects it; a drag draws a box and selects the object in it. Shift, Alt or both add,
  // subtract or intersect, shown by a badge by the pointer.
  import type { SelectionMode } from "./engine";
  import { MODE_BADGES, modeFromKeys } from "./selection";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    mode,
    busy,
    onhover,
    onselect,
  }: {
    mapping: ViewMapping;
    /** The options bar's mode, which keys override. */
    mode: SelectionMode;
    /** A selection is being made. */
    busy: boolean;
    /** The object under document point (`x`, `y`), the view being `view` (document pixels). */
    onhover: (
      x: number,
      y: number,
      view: [number, number, number, number],
    ) => Promise<ObjectHover | null>;
    /** A click (a point) or a drag (a box `[left, top, right, bottom]`), document pixels. */
    onselect: (
      point: [number, number] | null,
      box: [number, number, number, number] | null,
      mode: SelectionMode | null,
      view: [number, number, number, number],
    ) => void;
  } = $props();

  /** A press moves less than this (CSS pixels) to be a click. */
  const CLICK = 4;

  let pointer = $state<{ x: number; y: number } | null>(null);
  let keys = $state({ shiftKey: false, altKey: false });
  let drag = $state<{ start: [number, number]; end: [number, number] } | null>(null);
  let hovered = $state.raw<ObjectHover | null>(null);
  let element: SVGSVGElement;
  let canvas = $state<HTMLCanvasElement>();
  const badge = $derived(MODE_BADGES[modeFromKeys(keys) ?? mode]);

  /** The document area in view, `[x, y, width, height]` (unclamped). */
  function view(): [number, number, number, number] {
    const box = element.getBoundingClientRect();
    const [left, top] = mapping.toDocument(box.left, box.top);
    const [right, bottom] = mapping.toDocument(box.right, box.bottom);
    return [left, top, right - left, bottom - top];
  }

  // Hover: one request at a time, the latest position (window pixels) wins.
  let inFlight = false;
  let wanted: [number, number] | null = null;
  function ask(clientX: number, clientY: number) {
    wanted = [clientX, clientY];
    if (inFlight) return;
    const [x, y] = mapping.toDocument(clientX, clientY);
    wanted = null;
    inFlight = true;
    void onhover(x, y, view())
      .then((result) => {
        if (pointer && !drag) hovered = result;
      })
      .finally(() => {
        inFlight = false;
        if (wanted && pointer && !drag) ask(...wanted);
      });
  }

  // The highlight: the mask's cells tinted, stretched over its region (smoothed).
  $effect(() => {
    const hover = hovered;
    if (!canvas || !hover) return;
    canvas.width = hover.side;
    canvas.height = hover.side;
    const rgba = new Uint8ClampedArray(hover.side * hover.side * 4);
    for (let i = 0; i < hover.mask.length; i++) {
      if (hover.mask[i] === 0) continue;
      rgba[i * 4] = 59;
      rgba[i * 4 + 1] = 142;
      rgba[i * 4 + 2] = 234;
      rgba[i * 4 + 3] = 110;
    }
    canvas.getContext("2d")?.putImageData(new ImageData(rgba, hover.side, hover.side), 0, 0);
  });

  /** Where the highlight goes, in viewport pixels. */
  const placement = $derived.by(() => {
    if (!hovered) return null;
    const [x, y, w, h] = hovered.region;
    const [left, top] = mapping.toViewport(x, y);
    const [right, bottom] = mapping.toViewport(x + w, y + h);
    return { left, top, width: right - left, height: bottom - top };
  });

  function track(e: PointerEvent | KeyboardEvent) {
    keys = { shiftKey: e.shiftKey, altKey: e.altKey };
  }

  function local(e: PointerEvent): [number, number] {
    const box = element.getBoundingClientRect();
    return [e.clientX - box.left, e.clientY - box.top];
  }

  function down(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand || busy) return;
    e.preventDefault();
    if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
    element.setPointerCapture(e.pointerId);
    const at = local(e);
    drag = { start: at, end: at };
  }

  function move(e: PointerEvent) {
    track(e);
    const [x, y] = local(e);
    pointer = { x, y };
    if (drag) {
      drag.end = [x, y];
      return;
    }
    if (!mapping.hand && !busy) ask(e.clientX, e.clientY);
  }

  function up(e: PointerEvent) {
    if (!drag) return;
    const { start, end } = drag;
    drag = null;
    hovered = null;
    const keyMode = modeFromKeys(e);
    const box = element.getBoundingClientRect();
    if (Math.hypot(end[0] - start[0], end[1] - start[1]) < CLICK) {
      onselect(mapping.toDocument(e.clientX, e.clientY), null, keyMode, view());
      return;
    }
    const [x0, y0] = mapping.toDocument(box.left + start[0], box.top + start[1]);
    const [x1, y1] = mapping.toDocument(box.left + end[0], box.top + end[1]);
    onselect(
      null,
      [Math.min(x0, x1), Math.min(y0, y1), Math.max(x0, x1), Math.max(y0, y1)],
      keyMode,
      view(),
    );
  }
</script>

<svelte:window onkeydown={track} onkeyup={track} />

{#if hovered && placement && !drag && !busy}
  <canvas
    bind:this={canvas}
    class="highlight"
    style:left="{placement.left}px"
    style:top="{placement.top}px"
    style:width="{placement.width}px"
    style:height="{placement.height}px"
  ></canvas>
{/if}
<svg
  class="object"
  class:hand={mapping.hand}
  class:busy
  role="presentation"
  bind:this={element}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={() => (drag = null)}
  onpointerleave={() => {
    pointer = null;
    hovered = null;
  }}
>
  {#if drag}
    <rect
      class="box"
      x={Math.min(drag.start[0], drag.end[0])}
      y={Math.min(drag.start[1], drag.end[1])}
      width={Math.abs(drag.end[0] - drag.start[0])}
      height={Math.abs(drag.end[1] - drag.start[1])}
    />
  {/if}
  {#if pointer && badge && !mapping.hand}
    <text class="badge" x={pointer.x + 8} y={pointer.y + 16}>{badge}</text>
  {/if}
</svg>

<style>
  .highlight {
    position: absolute;
    pointer-events: none;
    image-rendering: auto;
  }

  .object {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: default;
  }

  .object.busy {
    cursor: progress;
  }

  /* Space held: the viewport pans. */
  .object.hand {
    pointer-events: none;
  }

  .box {
    fill: none;
    stroke: #ffffff;
    stroke-width: 1px;
    stroke-dasharray: 4 3;
    pointer-events: none;
  }

  .badge {
    fill: #ffffff;
    font-size: 13px;
    font-weight: 700;
    paint-order: stroke;
    stroke: #000000;
    stroke-width: 3px;
    pointer-events: none;
  }
</style>
