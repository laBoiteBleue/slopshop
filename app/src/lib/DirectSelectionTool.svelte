<script lang="ts">
  // Direct Selection (A, ADR 0041), as Photoshop's: the active vector layer's anchors shown on
  // the image; a press takes an anchor (it becomes the selected one, its handles shown) or a
  // handle of the selected anchor; a drag moves it, live, one undo entry per drag. Shift keeps
  // the move at 45° steps, Alt moves a handle alone. A live shape (rectangle, ellipse…) is
  // turned into a path first, after a question. Esc during a drag puts things back.
  import * as affine from "./affine";
  import type { Matrix } from "./engine";
  import { snapped45 } from "./gradient";
  import { HIT_PIXELS, hitAt, moved, svgOf, type Hit, type PenPath, type Point } from "./pen";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    paths,
    matrix,
    live = false,
    onedit,
    oncancel,
    onconvert,
  }: {
    mapping: ViewMapping;
    /** The layer's anchors, in its content's space. */
    paths: PenPath[];
    /** From the layer's content to the document. */
    matrix: Matrix;
    /** The layer is a live shape: a press asks to turn it into a path (`onconvert`). */
    live?: boolean;
    /** Anchors moved: `done` once the drag ends (the gesture's last edit). */
    onedit: (paths: PenPath[], done: boolean) => void;
    /** A drag given up (Esc): the gesture is taken back. */
    oncancel: () => void;
    onconvert: () => void;
  } = $props();

  /** A press moving less than this (CSS pixels) only selects. */
  const CLICK_SLOP = 3;

  /** The anchor whose handles are shown, as `[path, anchor]`. */
  let selected = $state<[number, number] | null>(null);
  let drag = $state<{
    pointerId: number;
    hit: Hit;
    /** What was taken and where it was, in the content's space. */
    from: Point;
    original: PenPath[];
    /** The anchors as the drag last moved them. */
    last: PenPath[];
    client: Point;
    moved: boolean;
  } | null>(null);

  const toContent = $derived(affine.invert(matrix));
  /** How far a press reaches, in the content's space. */
  const reach = $derived(
    (HIT_PIXELS * mapping.docPerCss) /
      Math.sqrt(Math.abs(matrix[0] * matrix[3] - matrix[1] * matrix[2])),
  );

  function contentPoint(e: PointerEvent): Point | null {
    if (!toContent) return null;
    const [x, y] = mapping.toDocument(e.clientX, e.clientY);
    return affine.apply(toContent, x, y);
  }

  /** Handles of other anchors than the selected one are not shown: they take no press. */
  function reachable(hit: Hit | null): Hit | null {
    if (!hit || hit.part === "point") return hit;
    return selected && selected[0] === hit.path && selected[1] === hit.anchor ? hit : null;
  }

  function begin(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand) return;
    const at = contentPoint(e);
    if (!at) return;
    e.preventDefault();
    if (live) {
      onconvert();
      return;
    }
    const hit = reachable(hitAt(paths, at, reach));
    if (!hit) {
      selected = null;
      return;
    }
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    if (hit.part === "point") selected = [hit.path, hit.anchor];
    const anchor = paths[hit.path].anchors[hit.anchor];
    drag = {
      pointerId: e.pointerId,
      hit,
      from: (hit.part === "point" ? anchor.point : anchor[hit.part]) as Point,
      original: paths,
      last: paths,
      client: [e.clientX, e.clientY],
      moved: false,
    };
  }

  function update(e: PointerEvent) {
    if (!drag || e.pointerId !== drag.pointerId) return;
    const at = contentPoint(e);
    if (!at) return;
    const movedEnough =
      drag.moved ||
      Math.hypot(e.clientX - drag.client[0], e.clientY - drag.client[1]) >= CLICK_SLOP;
    if (!movedEnough) return;
    const anchor = drag.original[drag.hit.path].anchors[drag.hit.anchor].point;
    // Shift: an anchor along 45° steps from where it was, a handle around its anchor.
    const origin = drag.hit.part === "point" ? drag.from : anchor;
    const to = snapped45(origin, at, e.shiftKey);
    const last = moved(drag.original, drag.hit, to, e.altKey);
    drag = { ...drag, moved: true, last };
    onedit(last, false);
  }

  function end(e: PointerEvent) {
    const current = drag;
    if (!current || e.pointerId !== current.pointerId) return;
    drag = null;
    if (current.moved) onedit(current.last, true);
  }

  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || !drag) return;
      e.preventDefault();
      e.stopPropagation();
      drag = null;
      oncancel();
    };
    window.addEventListener("keydown", onKey, { capture: true });
    return () => window.removeEventListener("keydown", onKey, { capture: true });
  });

  const shown = $derived.by(() => {
    const view = (p: Point): Point => {
      const [x, y] = affine.apply(matrix, p[0], p[1]);
      return mapping.toViewport(x, y);
    };
    const d = svgOf(paths, (x, y) => view([x, y]));
    const anchors = paths.flatMap((path, p) =>
      path.anchors.map((a, i) => ({
        at: view(a.point),
        selected: selected !== null && selected[0] === p && selected[1] === i,
      })),
    );
    const anchor = selected ? paths[selected[0]]?.anchors[selected[1]] : undefined;
    const handles = anchor
      ? [anchor.before, anchor.after]
          .filter((h): h is Point => h !== null)
          .map((h) => ({ from: view(anchor.point), to: view(h) }))
      : [];
    return { d, anchors, handles };
  });
</script>

<!-- Full-size: it takes the presses on the image while the tool is active. -->
<svg
  class="direct-selection"
  class:hand={mapping.hand}
  role="presentation"
  onpointerdown={begin}
  onpointermove={update}
  onpointerup={end}
  onpointercancel={() => (drag = null)}
>
  <path class="outline" d={shown.d} />
  {#each shown.handles as handle, i (i)}
    <line
      class="handle-line"
      x1={handle.from[0]}
      y1={handle.from[1]}
      x2={handle.to[0]}
      y2={handle.to[1]}
    />
    <circle class="handle" cx={handle.to[0]} cy={handle.to[1]} r="3" />
  {/each}
  {#each shown.anchors as anchor, i (i)}
    <rect
      class="anchor"
      class:selected={anchor.selected}
      x={anchor.at[0] - 3}
      y={anchor.at[1] - 3}
      width="6"
      height="6"
    />
  {/each}
</svg>

<style>
  .direct-selection {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: default;
  }

  .direct-selection.hand {
    pointer-events: none;
  }

  .outline {
    fill: none;
    stroke: var(--accent, #3b82f6);
    stroke-width: 1;
  }

  .handle-line {
    stroke: var(--accent, #3b82f6);
    stroke-width: 1;
  }

  .handle {
    fill: var(--accent, #3b82f6);
  }

  /* Anchors hollow, the selected one filled, as Photoshop shows them. */
  .anchor {
    fill: #ffffff;
    stroke: var(--accent, #3b82f6);
    stroke-width: 1;
  }

  .anchor.selected {
    fill: var(--accent, #3b82f6);
  }
</style>
