<script lang="ts">
  // Free Transform (Ctrl+T, as in Photoshop): a box around the selected layers with eight
  // handles. Drag inside to move, a handle to scale (corners keep the proportions, Shift frees
  // them; sides scale one way, Shift keeps the proportions; Alt scales about the center), and
  // outside to rotate about the center (Shift: steps of 15°). Enter, the ✓ button, a
  // double-click inside or a click outside (without dragging) applies; Esc or ✕ cancels. The owner applies `onchange`'s matrix live (ADR 0018).
  import * as affine from "./affine";
  import Icon from "./Icon.svelte";
  import type { Bounds, Matrix } from "./engine";
  import { getLocale, t } from "./i18n/index.svelte";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    box,
    onchange,
    oncommit,
    oncancel,
  }: {
    mapping: ViewMapping;
    /** The selected layers' bounds when the transform began, in document pixels. */
    box: Bounds;
    /** The transform since the beginning, a map of the document's space. */
    onchange: (matrix: Matrix) => void;
    oncommit: () => void;
    oncancel: () => void;
  } = $props();

  /** Rotation steps with Shift. */
  const ROTATION_STEP = Math.PI / 12;
  /** Scales never reach 0 (the transform must stay invertible). */
  const MIN_SCALE = 1e-3;

  /** Maps the box as it was to where it is now. */
  let matrix = $state<Matrix>(affine.IDENTITY);
  /** Rotation applied so far, for Shift's steps. */
  let rotation = 0;

  type Drag = {
    pointerId: number;
    kind: "move" | "scale" | "rotate";
    /** The handle dragged (scale). */
    handle: number;
    /** `matrix` and `rotation` when the drag began, and the document point it began at. */
    start: Matrix;
    startRotation: number;
    from: [number, number];
    /** Where the pointer went down (CSS pixels), and whether it has left that spot. */
    client: [number, number];
    moved: boolean;
  };
  /** A press moving less than this (CSS pixels) is a click, not a drag. */
  const CLICK_SLOP = 3;
  let drag = $state<Drag | null>(null);
  /** What the drag does, shown next to the pointer. */
  let readout = $state<{ text: string; x: number; y: number } | null>(null);

  const center = $derived<[number, number]>([
    (box.left + box.right) / 2,
    (box.top + box.bottom) / 2,
  ]);
  /** Handles in the box's coordinates, clockwise from the top-left corner (even: corners). */
  const handles = $derived.by((): [number, number][] => {
    const [cx, cy] = center;
    const { left: l, top: t, right: r, bottom: b } = box;
    return [
      [l, t],
      [cx, t],
      [r, t],
      [r, cy],
      [r, b],
      [cx, b],
      [l, b],
      [l, cy],
    ];
  });
  const screen = $derived(
    handles.map(([x, y]) => mapping.toViewport(...affine.apply(matrix, x, y))),
  );
  const screenCenter = $derived(mapping.toViewport(...affine.apply(matrix, ...center)));
  /** Size of the overlay, CSS pixels. */
  let width = $state(0);
  let height = $state(0);
  /** The ✓ / ✕ buttons: at the top right of the box as shown, kept inside the viewport. */
  const BUTTONS_WIDTH = 60;
  const buttonsAt = $derived.by((): [number, number] => {
    const xs = screen.map(([x]) => x);
    const ys = screen.map(([, y]) => y);
    const x = Math.min(Math.max(Math.max(...xs) + 10, 4), width - BUTTONS_WIDTH - 4);
    const y = Math.min(Math.max(Math.min(...ys) - 34, 4), height - 30);
    return [x, y];
  });
  const outline = $derived([0, 2, 4, 6].map((i) => `${screen[i][0]},${screen[i][1]}`).join(" "));

  const RESIZE_CURSORS = ["ew-resize", "nwse-resize", "ns-resize", "nesw-resize"];
  /** A resize cursor along the direction from the center to handle `i`, as shown. */
  function cursorFor(i: number): string {
    const [x, y] = screen[i];
    const angle = (Math.atan2(y - screenCenter[1], x - screenCenter[0]) * 180) / Math.PI;
    const step = Math.round((((angle % 180) + 180) % 180) / 45) % 4;
    return RESIZE_CURSORS[step];
  }

  function begin(e: PointerEvent, kind: Drag["kind"], handle = 0) {
    // Other buttons, or Space held: the viewport pans.
    if (e.button !== 0 || mapping.hand) return;
    e.stopPropagation();
    e.preventDefault();
    (e.currentTarget as Element).closest("svg")?.setPointerCapture(e.pointerId);
    drag = {
      pointerId: e.pointerId,
      kind,
      handle,
      start: matrix,
      startRotation: rotation,
      from: mapping.toDocument(e.clientX, e.clientY),
      client: [e.clientX, e.clientY],
      moved: false,
    };
  }

  const number = (v: number, digits: number) =>
    new Intl.NumberFormat(getLocale(), {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    }).format(v);

  function onPointerMove(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    if (!drag.moved) {
      const distance = Math.hypot(e.clientX - drag.client[0], e.clientY - drag.client[1]);
      if (distance < CLICK_SLOP) return;
      drag.moved = true;
    }
    const p = mapping.toDocument(e.clientX, e.clientY);
    const { start, from } = drag;
    let text = "";
    if (drag.kind === "move") {
      let [dx, dy] = [p[0] - from[0], p[1] - from[1]];
      // Shift: along one axis.
      if (e.shiftKey) {
        if (Math.abs(dx) >= Math.abs(dy)) dy = 0;
        else dx = 0;
      }
      matrix = affine.then(start, affine.translation(dx, dy));
      text = t("transform.readout.move", { dx: number(dx, 0), dy: number(dy, 0) });
    } else if (drag.kind === "rotate") {
      const [cx, cy] = affine.apply(start, ...center);
      const turned = Math.atan2(p[1] - cy, p[0] - cx) - Math.atan2(from[1] - cy, from[0] - cx);
      let total = drag.startRotation + turned;
      if (e.shiftKey) total = Math.round(total / ROTATION_STEP) * ROTATION_STEP;
      rotation = total;
      const by = affine.about(affine.rotation(total - drag.startRotation), cx, cy);
      matrix = affine.then(start, by);
      // Shown in (-180°, 180°], as Photoshop does.
      let degrees = ((((total * 180) / Math.PI) % 360) + 360) % 360;
      if (degrees > 180) degrees -= 360;
      text = t("transform.readout.angle", { angle: number(degrees, 1) });
    } else {
      const inverse = affine.invert(start);
      if (!inverse) return;
      const [qx, qy] = affine.apply(inverse, ...p);
      const [hx, hy] = handles[drag.handle];
      const [ax, ay] = e.altKey ? center : handles[(drag.handle + 4) % 8];
      const [wx, wy] = [hx - ax, hy - ay];
      const ux = wx !== 0 ? (qx - ax) / wx : 1;
      const uy = wy !== 0 ? (qy - ay) / wy : 1;
      let [sx, sy] = [1, 1];
      if (drag.handle % 2 === 0) {
        if (e.shiftKey) {
          [sx, sy] = [ux, uy];
        } else {
          // Along the diagonal: the pointer projected on it.
          const s = (wx * (qx - ax) + wy * (qy - ay)) / (wx * wx + wy * wy || 1);
          [sx, sy] = [s, s];
        }
      } else if (wy === 0) {
        sx = ux;
        if (e.shiftKey) sy = Math.abs(ux);
      } else {
        sy = uy;
        if (e.shiftKey) sx = Math.abs(uy);
      }
      const bounded = (s: number) => (Math.abs(s) < MIN_SCALE ? Math.sign(s || 1) * MIN_SCALE : s);
      [sx, sy] = [bounded(sx), bounded(sy)];
      const local = affine.about(affine.scaling(sx, sy), ax, ay);
      matrix = affine.then(local, start);
      // Relative to the size when the transform began.
      const [a, b, c, d] = matrix;
      const width = Math.hypot(a, b) * 100;
      const height = (Math.abs(a * d - b * c) / Math.hypot(a, b)) * 100;
      text = t("transform.readout.scale", { w: number(width, 1), h: number(height, 1) });
    }
    const rect = (e.currentTarget as Element).getBoundingClientRect();
    readout = { text, x: e.clientX - rect.left + 16, y: e.clientY - rect.top + 16 };
    onchange(matrix);
  }

  function onPointerUp(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    // A click outside the box, as in Photoshop: apply.
    const commit = drag.kind === "rotate" && !drag.moved;
    drag = null;
    readout = null;
    if (commit) oncommit();
  }

  function isTextField(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLTextAreaElement ||
      (target instanceof HTMLInputElement && ["text", "number", "search"].includes(target.type))
    );
  }

  function onKeydown(e: KeyboardEvent) {
    if (isTextField(e.target)) return;
    if (e.key === "Enter") {
      e.preventDefault();
      oncommit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      oncancel();
    }
  }

  /** A curved arrow: rotate. */
  const ROTATE_CURSOR =
    "url(\"data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='24' height='24'%3E%3Cpath d='M7 15a7 7 0 1 1 7 6' fill='none' stroke='black' stroke-width='4'/%3E%3Cpath d='M7 15a7 7 0 1 1 7 6' fill='none' stroke='white' stroke-width='2'/%3E%3Cpath d='M3 13h8l-4 5z' fill='white' stroke='black'/%3E%3C/svg%3E\") 12 12, crosshair";
</script>

<svelte:window onkeydown={onKeydown} />

<div class="overlay" bind:clientWidth={width} bind:clientHeight={height}></div>
<svg
  class="free-transform"
  role="presentation"
  style:cursor={drag?.kind === "scale"
    ? cursorFor(drag.handle)
    : drag?.kind === "move"
      ? "default"
      : ROTATE_CURSOR}
  onpointerdown={(e) => begin(e, "rotate")}
  onpointermove={onPointerMove}
  onpointerup={onPointerUp}
  onpointercancel={onPointerUp}
>
  <polygon
    class="body"
    points={outline}
    role="presentation"
    style:cursor={drag ? undefined : "default"}
    onpointerdown={(e) => begin(e, "move")}
    ondblclick={() => oncommit()}
  />
  {#each screen as [x, y], i (i)}
    <rect
      class="handle"
      x={x - 4}
      y={y - 4}
      width="8"
      height="8"
      role="presentation"
      style:cursor={drag ? undefined : cursorFor(i)}
      onpointerdown={(e) => begin(e, "scale", i)}
    />
  {/each}
  <circle class="pivot" cx={screenCenter[0]} cy={screenCenter[1]} r="3" />
</svg>
<div
  class="buttons"
  role="toolbar"
  tabindex="-1"
  style:left="{buttonsAt[0]}px"
  style:top="{buttonsAt[1]}px"
  onpointerdown={(e) => e.stopPropagation()}
>
  <button type="button" title={t("transform.apply")} onclick={() => oncommit()}>
    <Icon name="check" />
  </button>
  <button type="button" title={t("transform.cancel")} onclick={() => oncancel()}>
    <Icon name="close" />
  </button>
</div>
{#if readout}
  <div class="readout" style:left="{readout.x}px" style:top="{readout.y}px">{readout.text}</div>
{/if}

<style>
  .overlay {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }

  .buttons {
    position: absolute;
    display: flex;
    gap: 4px;
    padding: 3px;
    border-radius: 6px;
    background: rgba(20, 20, 20, 0.85);
  }

  .buttons button {
    display: grid;
    place-items: center;
    width: 24px;
    height: 22px;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: #fff;
    cursor: default;
  }

  .buttons button:hover {
    background: rgba(255, 255, 255, 0.15);
  }

  .free-transform {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  .body {
    fill: transparent;
    stroke: var(--accent, #2d8cff);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }

  .handle {
    fill: #fff;
    stroke: #1a1a1a;
    stroke-width: 1;
  }

  .pivot {
    fill: none;
    stroke: #fff;
    stroke-width: 1.5;
    pointer-events: none;
    filter: drop-shadow(0 0 1px #000);
  }

  .readout {
    position: absolute;
    padding: 3px 7px;
    border-radius: 4px;
    background: rgba(20, 20, 20, 0.85);
    color: #fff;
    font-size: 11px;
    white-space: nowrap;
    pointer-events: none;
  }
</style>
