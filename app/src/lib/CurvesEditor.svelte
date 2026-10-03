<script lang="ts">
  // The Curves editor (Photoshop's): a channel menu, the curve on a 0–255 grid, its points.
  // Click to add a point, drag to move it, drag it out of the grid to remove it; Input and
  // Output edit the selected point. The curve drawn comes from the engine (`samples`).
  import { t } from "./i18n/index.svelte";
  import type { MessageKey } from "./i18n/en";

  type Point = [number, number];
  /** Composite, red, green, blue. */
  type Curves = Point[][];

  let {
    curves,
    samples,
    onlive,
    onend,
    onapply,
  }: {
    curves: number[][][];
    /** Each curve's output (0–1) at evenly spaced inputs. */
    samples: number[][];
    /** A step of a drag (part of one undoable gesture). */
    onlive: (curves: Curves) => void;
    onend: () => void;
    /** One undoable change. */
    onapply: (curves: Curves) => void;
  } = $props();

  const CHANNELS: MessageKey[] = [
    "adjustment.curves.rgb",
    "adjustment.curves.red",
    "adjustment.curves.green",
    "adjustment.curves.blue",
  ];
  const MAX_POINTS = 16;
  /** How far out of the grid (0–255 units) a dragged point is removed. */
  const REMOVE_DISTANCE = 24;

  let channel = $state(0);
  /** The points shown: the layer's, or the ones being dragged. */
  let local = $state<Curves>([]);
  let dragging = $state(false);
  $effect(() => {
    const current = curves;
    if (!dragging) local = current.map((c) => c.map((p) => [p[0], p[1]] as Point));
  });
  let selected = $state<number | null>(null);
  /** The dragged point is out of the grid: released there, it goes. */
  let removing = $state(false);
  let svg: SVGSVGElement;

  const points = $derived(local[channel] ?? []);
  const path = $derived.by(() => {
    const s = samples[channel] ?? [];
    return s
      .map((v, i) => `${i === 0 ? "M" : "L"}${(i / (s.length - 1)) * 255},${255 - v * 255}`)
      .join(" ");
  });

  /** The 0–255 position under the pointer (not clamped). */
  function position(e: PointerEvent): Point {
    const r = svg.getBoundingClientRect();
    return [((e.clientX - r.left) / r.width) * 255, (1 - (e.clientY - r.top) / r.height) * 255];
  }

  function withPoints(next: Point[]): Curves {
    return local.map((c, i) => (i === channel ? next : c));
  }

  /** Where point `index` may go: between its neighbours, inside the grid. */
  function clampPoint(list: Point[], index: number, [x, y]: Point): Point {
    const lo = index > 0 ? list[index - 1][0] + 1 : 0;
    const hi = index < list.length - 1 ? list[index + 1][0] - 1 : 255;
    return [Math.round(Math.min(Math.max(x, lo), hi)), Math.round(Math.min(Math.max(y, 0), 255))];
  }

  function onDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const [x, y] = position(e);
    // The point under the pointer, else a new one there.
    const near = points.findIndex((p) => Math.abs(p[0] - x) <= 8 && Math.abs(p[1] - y) <= 8);
    let index = near;
    if (index < 0) {
      if (points.length >= MAX_POINTS) return;
      const px = Math.round(Math.min(Math.max(x, 0), 255));
      if (points.some((p) => p[0] === px)) return;
      const next = [...points, [px, Math.round(Math.min(Math.max(y, 0), 255))] as Point].sort(
        (a, b) => a[0] - b[0],
      );
      index = next.findIndex((p) => p[0] === px);
      local = withPoints(next);
      onlive(local);
    }
    selected = index;
    dragging = true;
    removing = false;
    svg.setPointerCapture(e.pointerId);
  }

  function onMove(e: PointerEvent) {
    if (!dragging || selected === null) return;
    const [x, y] = position(e);
    const out = Math.max(-x, x - 255, -y, y - 255);
    // Interior points only: a curve keeps two points at least.
    removing = out > REMOVE_DISTANCE && points.length > 2;
    if (removing) return;
    const next = [...points];
    next[selected] = clampPoint(points, selected, [x, y]);
    local = withPoints(next);
    onlive(local);
  }

  function onUp() {
    if (!dragging) return;
    if (removing && selected !== null) {
      local = withPoints(points.filter((_, i) => i !== selected));
      onlive(local);
      selected = null;
    }
    dragging = false;
    removing = false;
    onend();
  }

  function onField(axis: 0 | 1, input: HTMLInputElement) {
    const value = input.value.trim() === "" ? NaN : Number(input.value);
    if (selected === null || !Number.isFinite(value)) {
      // Empty or invalid: the point's value again, rather than 0.
      if (current) input.value = String(current[axis]);
      return;
    }
    const next = [...points];
    const p: Point = [...next[selected]];
    p[axis] = value;
    next[selected] = clampPoint(points, selected, p);
    local = withPoints(next);
    onapply(local);
  }

  $effect(() => {
    void channel;
    selected = null;
  });
  const current = $derived(selected !== null ? points[selected] : null);
</script>

<div class="curves">
  <label class="label" for="curves-channel">{t("adjustment.curves.channel")}</label>
  <select id="curves-channel" bind:value={channel}>
    {#each CHANNELS as label, i (label)}
      <option value={i}>{t(label)}</option>
    {/each}
  </select>
  <svg
    bind:this={svg}
    class="graph"
    class:removing
    viewBox="0 0 255 255"
    role="application"
    aria-label={t("adjustment.curves")}
    onpointerdown={onDown}
    onpointermove={onMove}
    onpointerup={onUp}
    onpointercancel={onUp}
  >
    {#each [64, 128, 191] as v (v)}
      <line class="grid" x1={v} y1="0" x2={v} y2="255" />
      <line class="grid" x1="0" y1={v} x2="255" y2={v} />
    {/each}
    <line class="baseline" x1="0" y1="255" x2="255" y2="0" />
    <path class="curve channel-{channel}" d={path} />
    {#each points as p, i (i)}
      <rect
        class="point"
        class:selected={i === selected}
        x={p[0] - 3}
        y={255 - p[1] - 3}
        width="6"
        height="6"
      />
    {/each}
  </svg>
  <div class="io">
    <label class="label" for="curves-input">{t("adjustment.curves.input")}</label>
    <input
      id="curves-input"
      type="number"
      min="0"
      max="255"
      disabled={current === null}
      value={current?.[0] ?? ""}
      onchange={(e) => onField(0, e.currentTarget)}
    />
    <label class="label" for="curves-output">{t("adjustment.curves.output")}</label>
    <input
      id="curves-output"
      type="number"
      min="0"
      max="255"
      disabled={current === null}
      value={current?.[1] ?? ""}
      onchange={(e) => onField(1, e.currentTarget)}
    />
  </div>
</div>

<style>
  .curves {
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: center;
    gap: 4px 8px;
  }

  .label {
    color: var(--text-muted);
  }

  .graph {
    grid-column: 1 / -1;
    width: 100%;
    aspect-ratio: 1;
    background: var(--canvas, #1e1e1e);
    border: 1px solid var(--border-dark);
    cursor: crosshair;
    touch-action: none;
    overflow: visible;
  }

  .graph.removing {
    cursor: not-allowed;
  }

  .grid {
    stroke: var(--border-dark);
    stroke-width: 0.6;
  }

  .baseline {
    stroke: var(--text-muted);
    stroke-width: 0.5;
    opacity: 0.5;
  }

  .curve {
    fill: none;
    stroke: var(--text);
    stroke-width: 1.4;
  }

  .curve.channel-1 {
    stroke: #e05050;
  }

  .curve.channel-2 {
    stroke: #50c050;
  }

  .curve.channel-3 {
    stroke: #5080f0;
  }

  .point {
    fill: var(--panel);
    stroke: var(--text);
    stroke-width: 1;
  }

  .point.selected {
    fill: var(--text);
  }

  .io {
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: auto 56px auto 56px;
    align-items: center;
    gap: 4px 6px;
  }

  .io input {
    width: 56px;
    min-width: 0;
  }
</style>
