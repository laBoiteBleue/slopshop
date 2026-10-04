<script lang="ts">
  // The Brush (B) and the Eraser (E), ADR 0027: a left drag paints on the active layer. Every
  // pointer sample is kept (the coalesced ones too, as a pen sends more samples than frames),
  // with the pen's pressure (a mouse paints at full pressure); the owner sends them to the
  // engine in batches. Shift+click paints a straight line from where the last stroke ended, as
  // in Photoshop. The brush outline follows the pointer, at the brush's size on the canvas.
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    size,
    onstroke,
  }: {
    mapping: ViewMapping;
    /** Brush diameter, document pixels. */
    size: number;
    /**
     * Samples since the last call (`[x, y, pressure]`, document pixels), with `start` for the
     * first ones of a stroke (`line`: Shift held, a straight line from the last stroke's end)
     * and `end` for the last ones; at the start, whether Alt is held.
     */
    onstroke: (
      samples: [number, number, number][],
      phase: "start" | "line" | "move" | "end",
      keys?: { altKey: boolean },
    ) => void;
  } = $props();

  let hover = $state<{ x: number; y: number } | null>(null);
  let painting = false;
  let element: SVGSVGElement;
  const radius = $derived(size / 2 / mapping.docPerCss);

  function local(e: PointerEvent): [number, number] {
    const box = element.getBoundingClientRect();
    return [e.clientX - box.left, e.clientY - box.top];
  }

  function sample(e: PointerEvent): [number, number, number] {
    const [x, y] = mapping.toDocument(e.clientX, e.clientY);
    // A mouse reports 0.5 while pressed: only a pen's pressure is a pressure.
    const pressure = e.pointerType === "pen" ? e.pressure : 1;
    return [x, y, pressure];
  }

  function down(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand) return;
    e.preventDefault();
    if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
    element.setPointerCapture(e.pointerId);
    painting = true;
    onstroke([sample(e)], e.shiftKey ? "line" : "start", { altKey: e.altKey });
  }

  function move(e: PointerEvent) {
    const [x, y] = local(e);
    hover = { x, y };
    if (!painting) return;
    const events = e.getCoalescedEvents?.() ?? [];
    onstroke((events.length > 0 ? events : [e]).map(sample), "move");
  }

  function up(e: PointerEvent) {
    if (!painting) return;
    painting = false;
    onstroke([sample(e)], "end");
  }
</script>

<svg
  class="paint"
  class:hand={mapping.hand}
  role="presentation"
  bind:this={element}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
  onpointerleave={() => (hover = null)}
>
  {#if hover && !mapping.hand}
    {#if radius >= 3}
      <circle class="outline" cx={hover.x} cy={hover.y} r={radius} />
      <circle class="outline inner" cx={hover.x} cy={hover.y} r={radius} />
    {:else}
      <!-- Too small to see: a cross, as Photoshop shows. -->
      <path class="outline" d="M{hover.x - 5} {hover.y}h10M{hover.x} {hover.y - 5}v10" />
      <path class="outline inner" d="M{hover.x - 5} {hover.y}h10M{hover.x} {hover.y - 5}v10" />
    {/if}
  {/if}
</svg>

<style>
  .paint {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: none;
    /* A pen drag paints: no panning or scrolling gestures of the webview. */
    touch-action: none;
  }

  /* Space held: the viewport pans. */
  .paint.hand {
    pointer-events: none;
  }

  .outline {
    fill: none;
    stroke: #000000;
    stroke-width: 1.5px;
    pointer-events: none;
  }

  .outline.inner {
    stroke: #ffffff;
    stroke-width: 0.75px;
  }
</style>
