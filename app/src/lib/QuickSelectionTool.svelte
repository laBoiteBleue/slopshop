<script lang="ts">
  // Quick Selection (W, ADR 0026): paint over an area and the selection grows to the similar
  // colors around the stroke, up to the image's edges, while it is painted, as in Photoshop;
  // the next strokes add to it, with Alt they take parts away; Esc drops the stroke under way.
  // A stroke becomes points along it (one per half brush). As with the other selection tools,
  // Shift and Alt are shown by a badge by the pointer. The brush is in document pixels, like
  // Photoshop's.
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
    /** The last stroke is still being turned into a selection. */
    busy: boolean;
    /**
     * The stroke so far (`move`: it goes on), done (`end`) or dropped with Esc (`cancel`): its
     * points (document pixels), the mode the keys asked for at its start (or null), and the
     * document region in view `[x, y, width, height]` (unclamped).
     */
    onstroke: (
      points: [number, number][],
      mode: SelectionMode | null,
      view: [number, number, number, number],
      phase: "move" | "end" | "cancel",
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

  /** The document area in view `[x, y, width, height]`. */
  function view(): [number, number, number, number] {
    const box = element.getBoundingClientRect();
    const [left, top] = mapping.toDocument(box.left, box.top);
    const [right, bottom] = mapping.toDocument(box.right, box.bottom);
    return [left, top, right - left, bottom - top];
  }

  function down(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand) return;
    e.preventDefault();
    if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
    element.setPointerCapture(e.pointerId);
    strokeMode = modeFromKeys(e);
    stroke = { screen: [local(e)], points: [mapping.toDocument(e.clientX, e.clientY)] };
    onstroke([...stroke.points], strokeMode, view(), "move");
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
    if (Math.hypot(point[0] - last[0], point[1] - last[1]) >= spacing) {
      stroke.points.push(point);
      onstroke([...stroke.points], strokeMode, view(), "move");
    }
  }

  function up(e: PointerEvent) {
    if (!stroke) return;
    const points = stroke.points;
    stroke = null;
    onstroke(points, strokeMode ?? modeFromKeys(e), view(), "end");
  }

  function onkeydown(e: KeyboardEvent) {
    track(e);
    if (e.key === "Escape" && stroke) {
      e.preventDefault();
      e.stopPropagation();
      stroke = null;
      onstroke([], strokeMode, view(), "cancel");
    }
  }
</script>

<svelte:window {onkeydown} onkeyup={track} />

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
