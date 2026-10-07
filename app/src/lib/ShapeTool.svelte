<script lang="ts">
  // The shape tools (U, ADR 0041): drag on the image to draw a rectangle, an ellipse, a polygon
  // (in a box) or a line, its outline shown while it grows; the release adds a vector layer. As
  // in Photoshop, Shift makes a square, a circle or a line at 45° steps, and Alt draws a box
  // from its center (once the keys held at the press were released). Esc gives up the drag.
  import { t } from "./i18n/index.svelte";
  import { snapped45 } from "./gradient";
  import {
    boxGeometry,
    dragBox,
    hasExtent,
    outlinePoints,
    svgPath,
    type ShapeGeometry,
    type ShapeKind,
    type ShapeOptions,
  } from "./shapes";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    kind,
    options,
    onshape,
  }: {
    mapping: ViewMapping;
    kind: ShapeKind;
    options: ShapeOptions;
    /** A shape was drawn, in document pixels. */
    onshape: (geometry: ShapeGeometry) => void;
  } = $props();

  /** A press moving less than this (CSS pixels) draws nothing. */
  const CLICK_SLOP = 3;

  type Drag = {
    pointerId: number;
    /** Where the press was, in document pixels and on screen. */
    from: [number, number];
    client: [number, number];
    to: [number, number];
    /** The keys held at the press were released: they now shape the drag. */
    shiftFree: boolean;
    altFree: boolean;
    shift: boolean;
    alt: boolean;
    moved: boolean;
  };
  let drag = $state<Drag | null>(null);

  /** Boxes on whole document pixels (crisp edges, as Photoshop's Align Edges). */
  function documentPoint(e: PointerEvent): [number, number] {
    const [x, y] = mapping.toDocument(e.clientX, e.clientY);
    return kind === "line" ? [x, y] : [Math.round(x), Math.round(y)];
  }

  function begin(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand) return;
    e.preventDefault();
    if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    const from = documentPoint(e);
    drag = {
      pointerId: e.pointerId,
      from,
      client: [e.clientX, e.clientY],
      to: from,
      shiftFree: !e.shiftKey,
      altFree: !e.altKey,
      shift: false,
      alt: false,
      moved: false,
    };
  }

  function update(e: PointerEvent) {
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
      shift: shiftFree && e.shiftKey,
      alt: altFree && e.altKey,
    };
  }

  function geometryOf(d: Drag): ShapeGeometry {
    if (kind === "line") return { kind, from: d.from, to: snapped45(d.from, d.to, d.shift) };
    return boxGeometry(kind, dragBox(d.from, d.to, d.shift, d.alt), options);
  }

  function end(e: PointerEvent) {
    const current = drag;
    if (!current || e.pointerId !== current.pointerId) return;
    drag = null;
    if (!current.moved) return;
    const geometry = geometryOf(current);
    if (hasExtent(geometry)) onshape(geometry);
  }

  function cancel(e: KeyboardEvent) {
    if (e.key === "Escape" && drag) {
      e.preventDefault();
      drag = null;
    }
  }

  const shown = $derived.by(() => {
    if (!drag?.moved) return null;
    const geometry = geometryOf(drag);
    const d = svgPath(outlinePoints(geometry), mapping.toViewport, geometry.kind === "line");
    const [x, y] = mapping.toViewport(drag.to[0], drag.to[1]);
    const box =
      geometry.kind === "line"
        ? {
            width: Math.abs(geometry.to[0] - geometry.from[0]),
            height: Math.abs(geometry.to[1] - geometry.from[1]),
          }
        : (() => {
            const b = dragBox(drag.from, drag.to, drag.shift, drag.alt);
            return { width: b.right - b.left, height: b.bottom - b.top };
          })();
    return {
      d,
      x,
      y,
      width: Math.round(box.width),
      height: Math.round(box.height),
    };
  });
</script>

<svelte:window onkeydown={cancel} />

<!-- Full-size: it takes the presses on the image while the tool is active. -->
<svg
  class="shape-tool"
  class:hand={mapping.hand}
  role="presentation"
  onpointerdown={begin}
  onpointermove={update}
  onpointerup={end}
  onpointercancel={() => (drag = null)}
>
  {#if shown}
    <path class="under" d={shown.d} />
    <path class="outline" d={shown.d} />
    <text class="readout" x={shown.x + 10} y={shown.y + 18}>
      {t("selection.readout", { width: shown.width, height: shown.height })}
    </text>
  {/if}
</svg>

<style>
  .shape-tool {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: crosshair;
  }

  /* Space held: the viewport pans. */
  .shape-tool.hand {
    pointer-events: none;
  }

  .under,
  .outline {
    fill: none;
    stroke-linejoin: round;
  }

  .under {
    stroke: #000000;
    stroke-width: 3;
    opacity: 0.6;
  }

  .outline {
    stroke: var(--accent, #3b82f6);
    stroke-width: 1;
  }

  .readout {
    fill: #ffffff;
    font-size: 11px;
    paint-order: stroke;
    stroke: #000000b3;
    stroke-width: 3px;
  }
</style>
