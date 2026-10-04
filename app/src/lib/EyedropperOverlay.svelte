<script lang="ts">
  // The image while Select > Color Range is open: the eyedropper's pointer (Shift shows the one
  // adding, Alt the one taking away), a loupe over the pixels around it, and a click samples.
  import {
    eyedropperCursor,
    eyedropperFromKeys,
    type EyedropperKind,
    type LoupeSource,
  } from "./eyedropper";
  import Loupe from "./Loupe.svelte";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    kind,
    onsample,
    loupe,
  }: {
    mapping: ViewMapping;
    /** The eyedropper chosen, which Shift and Alt override. */
    kind: EyedropperKind;
    /** A click at document point (`x`, `y`), with the keys held. */
    onsample: (x: number, y: number, keys: { shiftKey: boolean; altKey: boolean }) => void;
    /** The pixels the loupe shows. */
    loupe: LoupeSource;
  } = $props();

  let keys = $state({ shiftKey: false, altKey: false });
  /** The pointer over the image, window pixels. */
  let hover = $state<{ x: number; y: number } | null>(null);
  /** The loupe is drawn: it is the pointer. */
  let loupeShown = $state(false);
  const shownKind = $derived(eyedropperFromKeys(kind, keys));

  function track(e: PointerEvent | KeyboardEvent) {
    keys = { shiftKey: e.shiftKey, altKey: e.altKey };
  }
</script>

<svelte:window onkeydown={track} onkeyup={track} />

<div
  class="eyedropper"
  class:hand={mapping.hand}
  role="presentation"
  style:cursor={loupeShown ? "none" : eyedropperCursor(shownKind)}
  onpointerdown={(e) => {
    if (e.button !== 0 || mapping.hand) return;
    const [x, y] = mapping.toDocument(e.clientX, e.clientY);
    onsample(x, y, e);
  }}
  onpointermove={(e) => {
    track(e);
    hover = { x: e.clientX, y: e.clientY };
  }}
  onpointerleave={() => (hover = null)}
></div>
{#if hover && !mapping.hand}
  <Loupe x={hover.x} y={hover.y} source={loupe} sign={shownKind} bind:shown={loupeShown} />
{/if}

<style>
  .eyedropper {
    position: absolute;
    inset: 0;
  }

  /* Space held: the viewport pans. */
  .eyedropper.hand {
    pointer-events: none;
  }
</style>
