<script lang="ts">
  // The Pen tool (P, ADR 0041), one tool where Photoshop has six. Each click adds a corner
  // anchor; a drag adds a smooth one, its handles opposite (Alt during the drag moves the
  // outgoing handle alone). Shift keeps the next anchor, or the handle, on 45° steps. A line
  // follows the pointer to show the next segment. A click on the first anchor closes the path
  // and Enter finishes an open one: either adds a vector layer, filled and stroked as the
  // options bar says. Backspace removes the last anchor, Esc drops the path. Ctrl+Enter makes
  // a selection of the path being drawn (dropping it), or else of the active vector layer's.
  // Changing tool finishes the path being drawn, as Enter does.
  //
  // The active vector layer's path shows its anchors and handles, edited as with Photoshop's
  // Pen and Direct Selection: Ctrl-drag an anchor, a handle (a smooth anchor stays smooth) or
  // a segment (the whole path); a click on a segment adds an anchor, on an anchor deletes it,
  // on an open path's end continues it; Alt+click on an anchor makes it a corner, Alt-drag
  // pulls new smooth handles out of it, Alt-drag on a handle moves it alone. Each gesture is
  // one edit of the layer's shape (its linked copies follow). A live shape is first turned
  // into a regular path (`onlive` asks); a layer in perspective is refused (`onrefused`).
  import { onDestroy, untrack } from "svelte";
  import * as affine from "./affine";
  import {
    cornered,
    geometryOfPaths,
    hitTest,
    isEnd,
    mappedPath,
    movedAnchor,
    movedHandle,
    movedPath,
    pathsOf,
    removed,
    replaced,
    reversed,
    split,
    withAnchorAt,
    type EditablePath,
    type Hit,
  } from "./pathEdit";
  import {
    closed,
    corner,
    dragged,
    geometryOf,
    isDrawable,
    nextPoint,
    rubberBand,
    svgOfPath,
    withAnchor,
    withoutLast,
    type Anchor,
    type PathGeometry,
    type PenPath,
    type Point,
  } from "./pen";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    documentId,
    editable = null,
    onpath,
    onmakeselection,
    onreshape = () => {},
    onlive = () => {},
    onrefused = () => {},
  }: {
    mapping: ViewMapping;
    /** The document drawn on (the tool lives as long as its view). */
    documentId: number;
    /** A path was finished (closed, Enter, or the tool left) on `documentId`, in document
     *  pixels. */
    onpath: (geometry: PathGeometry, documentId: number) => void;
    /** Ctrl+Enter: a selection of `geometry` (the path being drawn), or with `null` of the
     *  active layer's path. */
    onmakeselection: (geometry: PathGeometry | null) => void;
    /** The active vector layer, when there is one. */
    editable?: EditablePath | null;
    /** Layer `layerId`'s path edited: its new geometry, in its space. */
    onreshape?: (layerId: number, geometry: PathGeometry) => void;
    /** An edit of live shape `layerId` was tried: it must become a path first. */
    onlive?: (layerId: number) => void;
    /** An edit of a layer in perspective was tried. */
    onrefused?: () => void;
  } = $props();

  /** A press moving less than this (CSS pixels) is a click: a corner anchor. */
  const CLICK_SLOP = 3;
  /** A press this close (CSS pixels) to the first anchor closes the path. */
  const CLOSE_RADIUS = 6;

  type Drag = {
    pointerId: number;
    /** The anchor being placed (or the first one, when closing). */
    point: Point;
    client: [number, number];
    moved: boolean;
    /** Alt was pressed during the drag: the incoming handle stays here. */
    frozenIn: Point | null;
    /** The press was on the first anchor: the release closes the path. */
    closing: boolean;
    /** The anchor as the drag shapes it. */
    anchor: Anchor;
  };

  /** A gesture on the layer's path: what it moves, and the paths as it leaves them. */
  type EditDrag = {
    pointerId: number;
    client: [number, number];
    /** Where the press was, in document pixels. */
    start: Point;
    moved: boolean;
    /** What the press landed on. */
    hit: Hit;
    /** Alt on an anchor: a click makes it a corner, a drag pulls new smooth handles. */
    convert: boolean;
    /** Alt on a handle: it moves alone. */
    independent: boolean;
    paths: PenPath[];
    current: PenPath[];
  };

  let path = $state<PenPath | null>(null);
  let drag = $state<Drag | null>(null);
  let editDrag = $state<EditDrag | null>(null);
  /** The path being drawn continues subpath `index` of the layer's `paths` (document pixels). */
  let continuing = $state<{ layerId: number; paths: PenPath[]; index: number } | null>(null);
  /** Paths just sent, shown until the layer's new path comes back. */
  let committed = $state<{ from: EditablePath; paths: PenPath[] } | null>(null);
  /** Ctrl (or Cmd) and Alt, as the pointer last saw them. */
  let mods = $state({ ctrl: false, alt: false });
  /** The pointer, in document pixels and in viewport pixels. */
  let pointer = $state<{ doc: Point; view: Point } | null>(null);
  let shift = $state(false);
  let element: SVGSVGElement;

  function locate(e: PointerEvent): { doc: Point; view: Point } {
    const box = element.getBoundingClientRect();
    return {
      doc: mapping.toDocument(e.clientX, e.clientY),
      view: [e.clientX - box.left, e.clientY - box.top],
    };
  }

  function screenDistance(a: Point, b: Point): number {
    const [ax, ay] = mapping.toViewport(a[0], a[1]);
    const [bx, by] = mapping.toViewport(b[0], b[1]);
    return Math.hypot(ax - bx, ay - by);
  }

  /** Whether a press at `at` lands on the first anchor of a path that can close. */
  function onFirst(at: Point): boolean {
    return (
      path !== null &&
      path.anchors.length >= 2 &&
      screenDistance(path.anchors[0].point, at) <= CLOSE_RADIUS
    );
  }

  /** The document of the view, kept: when another tab is shown, the path finishes on its own. */
  const drawnOn = untrack(() => documentId);

  const toView = (p: Point): Point => mapping.toViewport(p[0], p[1]);

  /** The layer's paths in document pixels, null when there is none to edit. */
  const layerPaths = $derived.by(() => {
    if (!editable?.matrix) return null;
    const m = editable.matrix;
    return pathsOf(editable.geometry).map((p) => mappedPath(p, (q) => affine.apply(m, q[0], q[1])));
  });

  /** The paths shown: as a gesture leaves them, else as last sent, else the layer's. */
  const shownPaths = $derived(
    editDrag?.current ??
      (committed && committed.from === editable ? committed.paths : null) ??
      layerPaths,
  );

  /** Send `paths` (document pixels) as layer `layerId`'s new path. */
  function commit(layerId: number, paths: PenPath[]) {
    const from = editable;
    if (!from?.matrix || from.layerId !== layerId) return;
    const inverse = affine.invert(from.matrix);
    if (!inverse) return;
    committed = { from, paths };
    const local = paths.map((p) => mappedPath(p, (q) => affine.apply(inverse, q[0], q[1])));
    onreshape(layerId, geometryOfPaths(local, from.geometry.rule));
  }

  function finish(done: PenPath | null) {
    const going = continuing;
    path = null;
    drag = null;
    continuing = null;
    if (going) {
      if (isDrawable(done)) commit(going.layerId, replaced(going.paths, going.index, done));
    } else if (isDrawable(done)) {
      onpath(geometryOf(done), drawnOn);
    }
  }

  /**
   * A press with no path being drawn, on the layer's path: an edit gesture begun, or a click
   * handled (adding, deleting, continuing). False when the press starts a new path.
   */
  function beginEdit(e: PointerEvent, at: { doc: Point; view: Point }): boolean {
    const target = editable;
    if (!target) return false;
    const ctrl = e.ctrlKey || e.metaKey;
    if (!target.matrix || !layerPaths) {
      if (ctrl || e.altKey) onrefused();
      return ctrl || e.altKey;
    }
    const paths = shownPaths ?? layerPaths;
    const hit = hitTest(paths, at.view, CLOSE_RADIUS, toView, ctrl || e.altKey);
    // Ctrl away from the path: nothing (no new anchor).
    if (!hit) return ctrl;
    if (target.live) {
      onlive(target.layerId);
      return true;
    }
    if (ctrl || e.altKey) {
      editDrag = {
        pointerId: e.pointerId,
        client: [e.clientX, e.clientY],
        start: at.doc,
        moved: false,
        hit,
        convert: e.altKey && !ctrl && hit.kind === "anchor",
        independent: e.altKey && hit.kind === "handle",
        paths,
        current: paths,
      };
      return true;
    }
    const subpath = paths[hit.path];
    if (hit.kind === "segment") {
      commit(target.layerId, replaced(paths, hit.path, split(subpath, hit.index, hit.t)));
    } else if (hit.kind === "anchor" && isEnd(subpath, hit.index)) {
      continuing = { layerId: target.layerId, paths, index: hit.path };
      path = hit.index === 0 ? reversed(subpath) : subpath;
    } else if (hit.kind === "anchor") {
      const left = removed(subpath, hit.index);
      if (left) commit(target.layerId, replaced(paths, hit.path, left));
    }
    return true;
  }

  /** The paths as edit gesture `current` leaves them, the pointer at `to`. */
  function edited(current: EditDrag, to: Point, shiftKey: boolean): PenPath[] {
    const { hit, paths } = current;
    const subpath = paths[hit.path];
    const [dx, dy] = [to[0] - current.start[0], to[1] - current.start[1]];
    let next: PenPath;
    if (hit.kind === "segment") {
      next = movedPath(subpath, dx, dy);
    } else if (hit.kind === "handle") {
      next = movedHandle(subpath, hit.index, hit.which, to, current.independent);
    } else if (current.convert) {
      const point = subpath.anchors[hit.index].point;
      next = withAnchorAt(subpath, hit.index, dragged(point, to, shiftKey, null));
    } else {
      next = movedAnchor(subpath, hit.index, dx, dy);
    }
    return replaced(paths, hit.path, next);
  }

  function begin(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand) return;
    e.preventDefault();
    if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
    element.setPointerCapture?.(e.pointerId);
    const at = locate(e);
    pointer = at;
    if (!path && beginEdit(e, at)) return;
    const closing = onFirst(at.doc);
    const point = closing && path ? path.anchors[0].point : nextPoint(path, at.doc, e.shiftKey);
    const anchor = closing && path ? path.anchors[0] : corner(point);
    if (!closing) path = withAnchor(path, anchor);
    drag = {
      pointerId: e.pointerId,
      point,
      client: [e.clientX, e.clientY],
      moved: false,
      frozenIn: null,
      closing,
      anchor,
    };
  }

  function move(e: PointerEvent) {
    shift = e.shiftKey;
    mods = { ctrl: e.ctrlKey || e.metaKey, alt: e.altKey };
    pointer = locate(e);
    const editing = editDrag;
    if (editing && e.pointerId === editing.pointerId) {
      const moved =
        editing.moved ||
        Math.hypot(e.clientX - editing.client[0], e.clientY - editing.client[1]) >= CLICK_SLOP;
      if (moved) {
        editDrag = { ...editing, moved, current: edited(editing, pointer.doc, e.shiftKey) };
      }
      return;
    }
    const current = drag;
    if (!current || e.pointerId !== current.pointerId) return;
    const moved =
      current.moved ||
      Math.hypot(e.clientX - current.client[0], e.clientY - current.client[1]) >= CLICK_SLOP;
    if (!moved) return;
    // Alt pressed during the drag: the incoming handle stays where it is.
    const frozenIn = current.frozenIn ?? (e.altKey && current.moved ? current.anchor.in : null);
    const anchor = dragged(current.point, pointer.doc, e.shiftKey, frozenIn);
    drag = { ...current, moved, frozenIn, anchor };
    if (!current.closing && path) {
      path = { ...path, anchors: [...path.anchors.slice(0, -1), anchor] };
    }
  }

  function end(e: PointerEvent) {
    const editing = editDrag;
    if (editing && e.pointerId === editing.pointerId) {
      editDrag = null;
      const { hit, paths } = editing;
      const layerId = editable?.layerId;
      if (layerId === undefined) return;
      if (editing.moved) {
        commit(layerId, editing.current);
      } else if (editing.convert && hit.kind === "anchor") {
        // Alt+click: a corner (nothing to do on one already).
        const a = paths[hit.path].anchors[hit.index];
        const handled = [a.in, a.out].some((h) => h[0] !== a.point[0] || h[1] !== a.point[1]);
        if (handled)
          commit(layerId, replaced(paths, hit.path, cornered(paths[hit.path], hit.index)));
      }
      return;
    }
    const current = drag;
    if (!current || e.pointerId !== current.pointerId) return;
    drag = null;
    if (current.closing && path) finish(closed(path, current.moved ? current.anchor : undefined));
  }

  onDestroy(() => finish(path));

  // The Pen's keys, before the app's own shortcuts see them.
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      shift = e.shiftKey;
      if (e.type !== "keydown") return;
      if (e.key === "Escape" && editDrag) {
        // A gesture dropped: the path as it was.
        editDrag = null;
        e.preventDefault();
        e.stopPropagation();
        return;
      }
      if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
        const drawn = path;
        path = null;
        drag = null;
        continuing = null;
        onmakeselection(isDrawable(drawn) ? geometryOf(drawn) : null);
      } else if (!path) {
        return;
      } else if (e.key === "Enter") {
        finish(path);
      } else if (e.key === "Escape") {
        path = null;
        drag = null;
        continuing = null;
      } else if (e.key === "Backspace" || e.key === "Delete") {
        if (drag) return;
        path = withoutLast(path);
        if (!path) continuing = null;
      } else {
        return;
      }
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", onKey, { capture: true });
    window.addEventListener("keyup", onKey, { capture: true });
    return () => {
      window.removeEventListener("keydown", onKey, { capture: true });
      window.removeEventListener("keyup", onKey, { capture: true });
    };
  });

  const toViewport = (x: number, y: number): Point => mapping.toViewport(x, y);

  /** The layer's path, when nothing is drawn: its outline, anchors and handles. */
  const shownLayer = $derived.by(() => {
    if (path || !shownPaths) return null;
    const anchors = shownPaths.flatMap((p) => p.anchors);
    return {
      outline: svgOfPath(geometryOfPaths(shownPaths, "nonzero"), toViewport),
      anchors: anchors.map((a) => toViewport(a.point[0], a.point[1])),
      handles: anchors.flatMap((a) =>
        [a.in, a.out]
          .filter((h) => h[0] !== a.point[0] || h[1] !== a.point[1])
          .map((h) => ({ from: toViewport(a.point[0], a.point[1]), to: toViewport(h[0], h[1]) })),
      ),
    };
  });

  /** What a click would do on the layer's path (Photoshop's Pen cursors): add, delete. */
  const hint = $derived.by(() => {
    if (path || drag || editDrag || !pointer || !shownPaths || mods.ctrl || mods.alt) return null;
    if (!editable || editable.live) return null;
    const hit = hitTest(shownPaths, pointer.view, CLOSE_RADIUS, toView, false);
    if (hit?.kind === "segment") return "add";
    if (hit?.kind === "anchor" && !isEnd(shownPaths[hit.path], hit.index)) return "delete";
    return null;
  });

  /** What is drawn over the image: the path, the next segment, anchors and handles. */
  const shown = $derived.by(() => {
    if (!path) return null;
    const closing = drag?.closing ? closed(path, drag.moved ? drag.anchor : undefined) : null;
    const outline = svgOfPath(geometryOf(closing ?? path), toViewport);
    const band =
      !drag && pointer
        ? rubberBand(
            path,
            onFirst(pointer.doc) ? path.anchors[0].point : nextPoint(path, pointer.doc, shift),
          )
        : null;
    const anchors = (closing ?? path).anchors;
    // The handles of the anchor being placed, or of the last one.
    const handled = drag ? drag.anchor : anchors[anchors.length - 1];
    const handles = [handled.in, handled.out]
      .filter((h) => h[0] !== handled.point[0] || h[1] !== handled.point[1])
      .map((h) => toViewport(h[0], h[1]));
    return {
      outline,
      band: band ? svgOfPath(band, toViewport) : null,
      anchors: anchors.map((a) => toViewport(a.point[0], a.point[1])),
      handled: toViewport(handled.point[0], handled.point[1]),
      handles,
      closingHint: !drag && pointer !== null && onFirst(pointer.doc),
    };
  });
</script>

<!-- Full-size: it takes the presses on the image while the tool is active. -->
<svg
  class="pen-tool"
  class:hand={mapping.hand}
  role="presentation"
  bind:this={element}
  onpointerdown={begin}
  onpointermove={move}
  onpointerleave={() => (pointer = null)}
  onpointerup={end}
  onpointercancel={() => {
    drag = null;
    editDrag = null;
  }}
>
  {#if shownLayer}
    <path class="under" d={shownLayer.outline} />
    <path class="outline" d={shownLayer.outline} data-testid="pen-layer-path" />
    {#each shownLayer.handles as { from, to }, i (i)}
      <line class="handle" x1={from[0]} y1={from[1]} x2={to[0]} y2={to[1]} />
      <circle class="knob" cx={to[0]} cy={to[1]} r="3" />
    {/each}
    {#each shownLayer.anchors as [x, y], i (i)}
      <rect
        class="anchor"
        x={x - 3}
        y={y - 3}
        width="6"
        height="6"
        data-testid="pen-layer-anchor"
      />
    {/each}
    {#if hint && pointer}
      <!-- A click adds an anchor (+) or deletes this one (−). -->
      <g
        class="hint"
        transform="translate({pointer.view[0] + 10} {pointer.view[1] + 10})"
        data-testid="pen-hint-{hint}"
      >
        <line x1="-3" y1="0" x2="3" y2="0" />
        {#if hint === "add"}
          <line x1="0" y1="-3" x2="0" y2="3" />
        {/if}
      </g>
    {/if}
  {/if}
  {#if shown}
    <path class="under" d={shown.outline} data-testid="pen-path" />
    <path class="outline" d={shown.outline} />
    {#if shown.band}
      <path class="band" d={shown.band} data-testid="pen-band" />
    {/if}
    {#each shown.handles as [x, y], i (i)}
      <line class="handle" x1={shown.handled[0]} y1={shown.handled[1]} x2={x} y2={y} />
      <circle class="knob" cx={x} cy={y} r="3" />
    {/each}
    {#each shown.anchors as [x, y], i (i)}
      <rect class="anchor" x={x - 3} y={y - 3} width="6" height="6" data-testid="pen-anchor" />
    {/each}
    {#if shown.closingHint && pointer}
      <!-- On the first anchor: a click closes the path. -->
      <circle class="close" cx={pointer.view[0] + 10} cy={pointer.view[1] + 10} r="3" />
    {/if}
  {/if}
</svg>

<style>
  .pen-tool {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: crosshair;
  }

  /* Space held: the viewport pans. */
  .pen-tool.hand {
    pointer-events: none;
  }

  .under,
  .outline,
  .band {
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

  .band {
    stroke: var(--accent, #3b82f6);
    stroke-width: 1;
    stroke-dasharray: 4 3;
  }

  .handle {
    stroke: var(--accent, #3b82f6);
    stroke-width: 1;
  }

  .knob {
    fill: var(--accent, #3b82f6);
  }

  .anchor {
    fill: #ffffff;
    stroke: var(--accent, #3b82f6);
    stroke-width: 1;
  }

  .hint line {
    stroke: #ffffff;
    stroke-width: 1.5;
  }

  .close {
    fill: none;
    stroke: #ffffff;
    stroke-width: 1.5;
    paint-order: stroke;
  }
</style>
