<script lang="ts">
  // Filter > Distort > Shear's curve editor, Photoshop's: a grid with a vertical line through
  // the curve's points, how far each row of the frame moves sideways. A press on the grid adds
  // a point there and drags it; a press on a point drags it (the first and last along their
  // row only); a point dragged off the grid is removed. Default straightens the line. A point
  // focused takes the arrows (Shift: by 10 %) and Delete.
  import { onMount } from "svelte";
  import {
    STRAIGHT,
    curveOf,
    grabbedPoint,
    gridPoint,
    offGrid,
    withPointAdded,
    withPointKey,
    withPointMoved,
    withPointRemoved,
    type ShearPoint,
  } from "./shearCurve";
  import { t } from "./i18n/index.svelte";

  let {
    points,
    onchange,
  }: {
    points: ShearPoint[];
    /** The curve, each time it changes. */
    onchange: (points: ShearPoint[]) => void;
  } = $props();

  // The drag: the points as they were when it started, and the one it moves.
  let drag: { from: ShearPoint[]; index: number } | null = null;
  let grid: HTMLElement;

  // The line, sampled every percent of the height (the grid is 200 wide, 200 high).
  const line = $derived.by(() => {
    const curve = curveOf(points);
    return Array.from({ length: 101 }, (_, row) => `${curve(row) + 100},${row * 2}`).join(" ");
  });

  function box() {
    return grid.getBoundingClientRect();
  }

  function press(e: PointerEvent) {
    if (e.button !== 0) return;
    const b = box();
    const index = grabbedPoint(points, b, e.clientX, e.clientY);
    if (index >= 0) {
      drag = { from: points, index };
    } else {
      const added = withPointAdded(points, gridPoint(b, e.clientX, e.clientY));
      if (!added) return;
      drag = { from: added.points, index: added.index };
      onchange(added.points);
    }
    grid.setPointerCapture?.(e.pointerId);
    e.preventDefault();
  }

  function move(e: PointerEvent) {
    if (!drag || !(e.buttons & 1)) return;
    const b = box();
    const { from, index } = drag;
    const removable = index > 0 && index < from.length - 1;
    onchange(
      removable && offGrid(b, e.clientX, e.clientY)
        ? withPointRemoved(from, index)
        : withPointMoved(from, index, gridPoint(b, e.clientX, e.clientY)),
    );
  }

  function release() {
    drag = null;
  }

  // The keys of a point focused, taken on the window before they go further: the filter's
  // dialog stops every key there (its own capture, set up after this one), so that the app's
  // shortcuts do not act behind it.
  onMount(() => {
    const key = (e: KeyboardEvent) => {
      const point = e.target instanceof HTMLElement ? e.target.closest("[data-point]") : null;
      if (!(point instanceof HTMLElement) || !grid.contains(point)) return;
      const next = withPointKey(points, Number(point.dataset.point), e.key, e.shiftKey);
      if (!next) return;
      e.preventDefault();
      onchange(next);
    };
    window.addEventListener("keydown", key, true);
    return () => window.removeEventListener("keydown", key, true);
  });
</script>

<div class="shear">
  <div
    class="grid"
    role="application"
    aria-label={t("filter.shear.curve")}
    bind:this={grid}
    onpointerdown={press}
    onpointermove={move}
    onpointerup={release}
    onpointercancel={release}
  >
    <svg viewBox="0 0 200 200" aria-hidden="true">
      {#each [50, 100, 150] as v (v)}
        <line class="rule" x1={v} y1="0" x2={v} y2="200" />
        <line class="rule" x1="0" y1={v} x2="200" y2={v} />
      {/each}
      <polyline class="curve" points={line} />
    </svg>
    {#each points as [row, offset], i (i)}
      <span
        class="point"
        role="slider"
        tabindex="0"
        aria-label={t("filter.shear.point", { n: i + 1 })}
        aria-valuemin="-100"
        aria-valuemax="100"
        aria-valuenow={offset}
        aria-valuetext="{Math.round(offset)} %, {Math.round(row)} %"
        style:left="{(offset + 100) / 2}%"
        style:top="{row}%"
        data-point={i}
      ></span>
    {/each}
  </div>
  <button type="button" class="btn" onclick={() => onchange(STRAIGHT)}>
    {t("filter.shear.default")}
  </button>
</div>

<style>
  .shear {
    display: grid;
    gap: 6px;
    justify-items: start;
  }

  /* Photoshop's grid: four squares a side, the line through them. */
  .grid {
    position: relative;
    width: 180px;
    height: 180px;
    border: 1px solid var(--border-dark);
    background: var(--field);
    cursor: crosshair;
    touch-action: none;
  }

  svg {
    display: block;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  .rule {
    stroke: var(--text);
    stroke-opacity: 0.25;
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }

  .curve {
    fill: none;
    stroke: var(--text);
    stroke-width: 1.5;
    vector-effect: non-scaling-stroke;
  }

  .point {
    position: absolute;
    width: 7px;
    height: 7px;
    margin: -4px 0 0 -4px;
    border: 1px solid var(--text);
    background: var(--field);
    pointer-events: none;
  }

  .point:focus-visible {
    outline: 1px solid var(--accent, var(--text));
    background: var(--text);
  }
</style>
