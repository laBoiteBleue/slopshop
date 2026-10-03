<script lang="ts">
  // Moving the selection's outline alone, its pixels left where they are, as in Photoshop: with
  // a selection tool in New Selection mode, a press inside the selection (where its outline
  // shows it) without Shift or Alt drags the outline instead of drawing (Shift during the drag:
  // by steps of 45°); a click there without dragging stays the tool's click. Whether the pointer
  // is inside is asked of the engine while it hovers, so that a press decides at once; the
  // outline moves on screen during the drag (`onshift`), the selection once at the end
  // (`onmove`). The pointer is the arrow while over the selection.
  import type { Snippet } from "svelte";
  import { engine } from "./engine";
  import { outlineOffset } from "./outlineDrag";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    documentId,
    selectionKey,
    enabled,
    onshift,
    onmove,
    onclick,
    children,
  }: {
    mapping: ViewMapping;
    documentId: number;
    /** The selection, or null without one. */
    selectionKey: number | null;
    /** The tool and the mode allow it (New Selection, not in Quick Mask). */
    enabled: boolean;
    /** The outline is dragged by (`dx`, `dy`) whole document pixels so far. */
    onshift: (dx: number, dy: number) => void;
    /** The drag ended (`dx`, `dy` may be 0: back where it started). */
    onmove: (dx: number, dy: number) => void;
    /** A click inside the selection, at document point (`x`, `y`): the tool's click. */
    onclick: (x: number, y: number) => void;
    children: Snippet;
  } = $props();

  /** A press moving less than this (CSS pixels) is a click. */
  const CLICK_SLOP = 3;
  /** How far (CSS pixels) a press may be from the last point answered while hovering. */
  const HOVER_SLOP = 3;

  /** The last answer: whether document pixel (`x`, `y`) of selection `key` is inside it. */
  type Hit = { key: number; x: number; y: number; inside: boolean };
  let hit = $state.raw<Hit | null>(null);
  let wanted: Omit<Hit, "inside"> | null = null;
  let asking = false;

  async function ask() {
    if (asking) return;
    asking = true;
    while (wanted) {
      const point = wanted;
      wanted = null;
      const bounds = await engine
        .selectionBoundsAt(documentId, point.x + 0.5, point.y + 0.5)
        .catch(() => null);
      hit = { ...point, inside: bounds !== null };
    }
    asking = false;
  }

  /** Whether the pointer at `e` is inside the selection, as last answered nearby. */
  function inside(e: PointerEvent): boolean {
    if (!hit?.inside || hit.key !== selectionKey) return false;
    const [x, y] = mapping.toViewport(hit.x + 0.5, hit.y + 0.5);
    const box = element.getBoundingClientRect();
    const reach = HOVER_SLOP + 0.5 / mapping.docPerCss;
    return Math.hypot(e.clientX - box.left - x, e.clientY - box.top - y) <= reach;
  }

  const over = $derived(enabled && hit?.inside === true && hit.key === selectionKey);

  type Drag = {
    pointerId: number;
    client: [number, number];
    from: [number, number];
    moved: boolean;
    shift: [number, number];
  };
  let drag: Drag | null = null;
  let element: HTMLDivElement;

  function hover(e: PointerEvent) {
    if (drag) {
      move(e);
      return;
    }
    if (!enabled || selectionKey === null || mapping.hand) return;
    const [x, y] = mapping.toDocument(e.clientX, e.clientY).map(Math.floor);
    if (hit?.key === selectionKey && hit.x === x && hit.y === y) return;
    wanted = { key: selectionKey, x, y };
    void ask();
  }

  function begin(e: PointerEvent) {
    if (!enabled || e.button !== 0 || mapping.hand || e.shiftKey || e.altKey || !inside(e)) {
      return;
    }
    // Not the tool's press.
    e.stopPropagation();
    e.preventDefault();
    if (document.activeElement instanceof HTMLInputElement) document.activeElement.blur();
    element.setPointerCapture(e.pointerId);
    drag = {
      pointerId: e.pointerId,
      client: [e.clientX, e.clientY],
      from: mapping.toDocument(e.clientX, e.clientY),
      moved: false,
      shift: [0, 0],
    };
  }

  function move(e: PointerEvent) {
    const current = drag;
    if (!current || e.pointerId !== current.pointerId) return;
    e.stopPropagation();
    const distance = Math.hypot(e.clientX - current.client[0], e.clientY - current.client[1]);
    if (!current.moved && distance < CLICK_SLOP) return;
    current.moved = true;
    const [x, y] = mapping.toDocument(e.clientX, e.clientY);
    const shift = outlineOffset(x - current.from[0], y - current.from[1], e.shiftKey);
    if (shift[0] === current.shift[0] && shift[1] === current.shift[1]) return;
    current.shift = shift;
    onshift(shift[0], shift[1]);
  }

  function end(e: PointerEvent) {
    const current = drag;
    if (!current || e.pointerId !== current.pointerId) return;
    e.stopPropagation();
    drag = null;
    if (current.moved) onmove(current.shift[0], current.shift[1]);
    else onclick(current.from[0], current.from[1]);
  }

  function cancel(e: PointerEvent) {
    const current = drag;
    if (!current || e.pointerId !== current.pointerId) return;
    drag = null;
    onmove(0, 0);
  }
</script>

<div
  class="selection-drag"
  class:over
  role="presentation"
  bind:this={element}
  onpointerdowncapture={begin}
  onpointermovecapture={hover}
  onpointerupcapture={end}
  onpointercancelcapture={cancel}
>
  {@render children()}
</div>

<style>
  .selection-drag {
    position: absolute;
    inset: 0;
  }

  /* Over the selection, the press moves it: the arrow, as in Photoshop. */
  .selection-drag.over :global(svg) {
    cursor: default;
  }
</style>
