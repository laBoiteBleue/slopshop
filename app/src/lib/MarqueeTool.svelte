<script lang="ts">
  // The Rectangular and Elliptical Marquee tools (M, ADR 0024): drag on the image to draw a
  // rectangle or an ellipse on whole document pixels, shown with marching ants while it grows.
  // As in Photoshop: Shift, Alt or both at the press add, subtract or intersect (else the options
  // bar's mode); during the drag, Shift makes a square or a circle and Alt draws from the center
  // (once the keys used at the press were released). A click without dragging deselects. A
  // small +, − or × next to the pointer tells how the next shape combines.
  import type { SelectionMode, SelectionShape } from "./engine";
  import { t } from "./i18n/index.svelte";
  import { modeFromKeys } from "./selection";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    kind,
    mode,
    onselect,
    ondeselect,
  }: {
    mapping: ViewMapping;
    kind: "rectangle" | "ellipse";
    /** The options bar's mode, which keys override. */
    mode: SelectionMode;
    /** A shape was drawn; `mode`: the one the keys asked for, or null. */
    onselect: (shape: SelectionShape, mode: SelectionMode | null) => void;
    /** A click without dragging, without keys. */
    ondeselect: () => void;
  } = $props();

  /** A press moving less than this (CSS pixels) is a click. */
  const CLICK_SLOP = 3;

  type Drag = {
    pointerId: number;
    /** Where the press was, in document pixels (whole) and on screen. */
    from: [number, number];
    client: [number, number];
    to: [number, number];
    mode: SelectionMode | null;
    /** The keys used at the press were released: they now shape the drag. */
    shiftFree: boolean;
    altFree: boolean;
    square: boolean;
    centered: boolean;
    moved: boolean;
  };
  let drag = $state<Drag | null>(null);
  /** The pointer over the image (viewport pixels) and the keys held, for the mode badge. */
  let hover = $state<{ x: number; y: number } | null>(null);
  let keys = $state({ shiftKey: false, altKey: false });
  let element: SVGSVGElement;

  const BADGES: Record<SelectionMode, string> = {
    replace: "",
    add: "+",
    subtract: "−",
    intersect: "×",
  };
  /** The mode of the drag (fixed at the press), or the one a press would take now. */
  const badge = $derived(BADGES[drag ? (drag.mode ?? mode) : (modeFromKeys(keys) ?? mode)]);

  function track(e: PointerEvent | KeyboardEvent) {
    keys = { shiftKey: e.shiftKey, altKey: e.altKey };
  }

  function documentPoint(e: PointerEvent): [number, number] {
    const [x, y] = mapping.toDocument(e.clientX, e.clientY);
    return [Math.round(x), Math.round(y)];
  }

  function begin(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand) return;
    e.preventDefault();
    // The press keeps the focus where it was: a field being typed in (Feather) is done.
    if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    const from = documentPoint(e);
    drag = {
      pointerId: e.pointerId,
      from,
      client: [e.clientX, e.clientY],
      to: from,
      mode: modeFromKeys(e),
      shiftFree: !e.shiftKey,
      altFree: !e.altKey,
      square: false,
      centered: false,
      moved: false,
    };
  }

  function update(e: PointerEvent) {
    track(e);
    const box = element.getBoundingClientRect();
    hover = { x: e.clientX - box.left, y: e.clientY - box.top };
    if (!drag || e.pointerId !== drag.pointerId) return;
    const moved =
      drag.moved ||
      Math.hypot(e.clientX - drag.client[0], e.clientY - drag.client[1]) >= CLICK_SLOP;
    const shiftFree = drag.shiftFree || !e.shiftKey;
    const altFree = drag.altFree || !e.altKey;
    drag = {
      ...drag,
      to: documentPoint(e),
      moved,
      shiftFree,
      altFree,
      square: shiftFree && e.shiftKey,
      centered: altFree && e.altKey,
    };
  }

  /** The drawn box, in document pixels. */
  function box(d: Drag): { left: number; top: number; right: number; bottom: number } {
    let dx = d.to[0] - d.from[0];
    let dy = d.to[1] - d.from[1];
    if (d.square) {
      const side = Math.max(Math.abs(dx), Math.abs(dy));
      dx = Math.sign(dx || 1) * side;
      dy = Math.sign(dy || 1) * side;
    }
    const [x0, y0] = d.centered ? [d.from[0] - dx, d.from[1] - dy] : d.from;
    const [x1, y1] = [d.from[0] + dx, d.from[1] + dy];
    return {
      left: Math.min(x0, x1),
      top: Math.min(y0, y1),
      right: Math.max(x0, x1),
      bottom: Math.max(y0, y1),
    };
  }

  function end(e: PointerEvent) {
    const current = drag;
    if (!current || e.pointerId !== current.pointerId) return;
    drag = null;
    const b = box(current);
    if (!current.moved || b.right === b.left || b.bottom === b.top) {
      if (!current.moved && current.mode === null) ondeselect();
      return;
    }
    onselect({ kind, ...b }, current.mode);
  }

  const shown = $derived.by(() => {
    if (!drag?.moved) return null;
    const b = box(drag);
    const [x0, y0] = mapping.toViewport(b.left, b.top);
    const [x1, y1] = mapping.toViewport(b.right, b.bottom);
    return { x0, y0, x1, y1, width: b.right - b.left, height: b.bottom - b.top };
  });
</script>

<!-- Full-size: it takes the presses on the image while the tool is active. -->
<svelte:window onkeydown={track} onkeyup={track} />

<svg
  class="marquee"
  class:hand={mapping.hand}
  role="presentation"
  bind:this={element}
  onpointerdown={begin}
  onpointermove={update}
  onpointerleave={() => (hover = null)}
  onpointerup={end}
  onpointercancel={() => (drag = null)}
>
  {#if shown}
    {#if kind === "rectangle"}
      <rect
        class="under"
        x={shown.x0}
        y={shown.y0}
        width={shown.x1 - shown.x0}
        height={shown.y1 - shown.y0}
      />
      <rect
        class="ants"
        x={shown.x0}
        y={shown.y0}
        width={shown.x1 - shown.x0}
        height={shown.y1 - shown.y0}
      />
    {:else}
      {@const cx = (shown.x0 + shown.x1) / 2}
      {@const cy = (shown.y0 + shown.y1) / 2}
      {@const rx = (shown.x1 - shown.x0) / 2}
      {@const ry = (shown.y1 - shown.y0) / 2}
      <ellipse class="under" {cx} {cy} {rx} {ry} />
      <ellipse class="ants" {cx} {cy} {rx} {ry} />
    {/if}
    <text class="readout" x={shown.x1 + 10} y={shown.y1 + 18}>
      {t("selection.readout", { width: shown.width, height: shown.height })}
    </text>
  {/if}
  {#if hover && badge && !mapping.hand}
    <text class="badge" x={hover.x + 8} y={hover.y + 16}>{badge}</text>
  {/if}
</svg>

<style>
  .marquee {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: crosshair;
  }

  /* Space held: the viewport pans. */
  .marquee.hand {
    pointer-events: none;
  }

  .under,
  .ants {
    fill: none;
    stroke-width: 1;
    shape-rendering: crispEdges;
  }

  .under {
    stroke: #ffffff;
  }

  .ants {
    stroke: #000000;
    stroke-dasharray: 4 4;
    animation: march 0.6s linear infinite;
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

  .readout {
    fill: #ffffff;
    font-size: 11px;
    paint-order: stroke;
    stroke: #000000b3;
    stroke-width: 3px;
  }

  @keyframes march {
    to {
      stroke-dashoffset: -8;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .ants {
      animation: none;
    }
  }
</style>
