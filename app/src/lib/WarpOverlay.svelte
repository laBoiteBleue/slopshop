<script lang="ts">
  // Edit > Transform > Warp (ADR 0044), in Free Transform's place over the image: the layer's
  // warp grid (its patches' edges), which the options bar's style and bend change live. Enter
  // or a double-click applies (one undo entry), Esc cancels, as in Free Transform.
  import { isTextField } from "./keymap";
  import type { ViewMapping } from "./Viewport.svelte";
  import { linePath } from "./warp";

  let {
    mapping,
    lines,
    oncommit,
    oncancel,
  }: {
    mapping: ViewMapping;
    /** The grid's lines, in document pixels. */
    lines: [number, number][][];
    oncommit: () => void;
    oncancel: () => void;
  } = $props();

  const paths = $derived(lines.map((line) => linePath(line, mapping.toViewport)));

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
</script>

<svelte:window onkeydown={onKeydown} />

<svg class="warp-grid" role="presentation" ondblclick={() => oncommit()}>
  {#each paths as d, i (i)}
    <path class="line" {d} />
  {/each}
</svg>

<style>
  .warp-grid {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  .line {
    fill: none;
    stroke: var(--accent, #2d8cff);
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
</style>
