<script lang="ts">
  // Quick Selection (W, ADR 0025): paint over an object and SAM 2.1 selects it whole. A stroke
  // becomes points along it (one per half brush); the engine keeps the strokes of a session
  // together, so the next strokes refine the same object: with Alt, they take parts away. As
  // with the other selection tools, Shift and Alt are shown by a badge by the pointer. The
  // brush is in document pixels, like Photoshop's.
  import type { SelectionMode } from "./engine";
  import { MODE_BADGES, modeFromKeys } from "./selection";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    mode,
    size,
    busy,
    onstroke,
  }: {
    mapping: ViewMapping;
    /** The options bar's mode, which keys override. */
    mode: SelectionMode;
    /** Brush diameter, document pixels. */
    size: number;
    /** A stroke is being turned into a selection. */
    busy: boolean;
    /**
     * A stroke: its points (document pixels), the mode the keys asked for (or null), and the
     * document region in view `[x, y, width, height]` (unclamped).
     */
    onstroke: (
      points: [number, number][],
      mode: SelectionMode | null,
      view: [number, number, number, number],
    ) => void;
  } = $props();

  let hover = $state<{ x: number; y: number } | null>(null);
  let keys = $state({ shiftKey: false, altKey: false });
  /** The stroke being painted, viewport pixels and document pixels. */
  let stroke = $state<{ screen: [number, number][]; points: [number, number][] } | null>(null);
  let strokeMode: SelectionMode | null = null;
  let element: SVGSVGElement;
  const badge = $derived(MODE_BADGES[modeFromKeys(keys) ?? mode]);
  const radius = $derived(size / 2 / mapping.docPerCss);

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
    strokeMode = modeFromKeys(e);
    stroke = { screen: [local(e)], points: [mapping.toDocument(e.clientX, e.clientY)] };
  }

  function move(e: PointerEvent) {
    track(e);
    const [x, y] = local(e);
    hover = { x, y };
    if (!stroke) return;
    stroke.screen.push([x, y]);
    // A point every half brush (at least 4 screen pixels apart).
    const point = mapping.toDocument(e.clientX, e.clientY);
    const last = stroke.points[stroke.points.length - 1];
    const spacing = Math.max(size / 2, 4 * mapping.docPerCss);
    if (Math.hypot(point[0] - last[0], point[1] - last[1]) >= spacing) stroke.points.push(point);
  }

  function up(e: PointerEvent) {
    if (!stroke) return;
    const points = stroke.points;
    stroke = null;
    const box = element.getBoundingClientRect();
    const [left, top] = mapping.toDocument(box.left, box.top);
    const [right, bottom] = mapping.toDocument(box.right, box.bottom);
    onstroke(points, strokeMode ?? modeFromKeys(e), [left, top, right - left, bottom - top]);
  }
</script>

<svelte:window onkeydown={track} onkeyup={track} />

<svg
  class="quick"
  class:hand={mapping.hand}
  class:busy
  role="presentation"
  bind:this={element}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={() => (stroke = null)}
  onpointerleave={() => (hover = null)}
>
  {#if stroke && stroke.screen.length > 1}
    <polyline
      class="stroke"
      points={stroke.screen.map(([x, y]) => `${x},${y}`).join(" ")}
      stroke-width={Math.max(radius * 2, 2)}
    />
  {/if}
  {#if hover && !mapping.hand}
    <circle class="brush" cx={hover.x} cy={hover.y} r={Math.max(radius, 2)} />
    <circle class="brush inner" cx={hover.x} cy={hover.y} r={Math.max(radius, 2)} />
    {#if badge}
      <text class="badge" x={hover.x + radius + 4} y={hover.y + 16}>{badge}</text>
    {/if}
  {/if}
</svg>

<style>
  .quick {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: none;
  }

  .quick.busy {
    cursor: progress;
  }

  /* Space held: the viewport pans. */
  .quick.hand {
    pointer-events: none;
  }

  .stroke {
    fill: none;
    stroke: #3b8eea;
    stroke-opacity: 0.35;
    stroke-linecap: round;
    stroke-linejoin: round;
    pointer-events: none;
  }

  .brush {
    fill: none;
    stroke: #000000;
    stroke-width: 1.5px;
    pointer-events: none;
  }

  .brush.inner {
    stroke: #ffffff;
    stroke-width: 0.75px;
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
