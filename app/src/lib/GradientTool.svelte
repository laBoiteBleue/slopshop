<script lang="ts">
  // The Gradient tool (G): a drag on the image draws the gradient's line, from where its first
  // color goes to where its last does; on release the owner lays it. Shift keeps the line at a
  // multiple of 45°, Escape drops it.
  import { snapped45 } from "./gradient";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    ongradient,
  }: {
    mapping: ViewMapping;
    /** The line drawn, document pixels. */
    ongradient: (from: [number, number], to: [number, number]) => void;
  } = $props();

  let drag = $state<{ pointerId: number; from: [number, number]; to: [number, number] } | null>(
    null,
  );

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand) return;
    e.preventDefault();
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    const at = mapping.toDocument(e.clientX, e.clientY);
    drag = { pointerId: e.pointerId, from: at, to: at };
  }

  function onPointerMove(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    drag = {
      ...drag,
      to: snapped45(drag.from, mapping.toDocument(e.clientX, e.clientY), e.shiftKey),
    };
  }

  function onPointerUp(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    const { from, to } = drag;
    drag = null;
    // Less than a pixel: a click, not a line.
    if (Math.hypot(to[0] - from[0], to[1] - from[1]) >= 1) ongradient(from, to);
  }

  const ends = $derived(drag && [mapping.toViewport(...drag.from), mapping.toViewport(...drag.to)]);
</script>

<svelte:window
  onkeydown={(e) => {
    if (drag && e.key === "Escape") {
      e.preventDefault();
      drag = null;
    }
  }}
/>

<svg
  class="gradient-tool"
  class:hand={mapping.hand}
  role="presentation"
  onpointerdown={onPointerDown}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={() => (drag = null)}
>
  {#if ends}
    {@const [[x0, y0], [x1, y1]] = ends}
    <line class="halo" {x1} {y1} x2={x0} y2={y0} />
    <line class="line" {x1} {y1} x2={x0} y2={y0} />
    <circle class="end" cx={x0} cy={y0} r="3.5" />
    <circle class="end" cx={x1} cy={y1} r="3.5" />
  {/if}
</svg>

<style>
  .gradient-tool {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: crosshair;
  }

  .gradient-tool.hand {
    pointer-events: none;
  }

  /* White over black, readable on any image. */
  .halo {
    stroke: rgba(0, 0, 0, 0.6);
    stroke-width: 3;
  }

  .line {
    stroke: #fff;
    stroke-width: 1;
  }

  .end {
    fill: #fff;
    stroke: #1a1a1a;
    stroke-width: 1;
  }
</style>
