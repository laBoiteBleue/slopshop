<script lang="ts">
  // A vector mask's thumbnail in the Layers panel (ADR 0041), as Photoshop draws it: the canvas
  // gray where the mask hides the layer, white where it shows it, the path's outline between.
  import type { LayerView, Matrix } from "./engine";
  import { maskOutline } from "./vectorMask";

  let {
    mask,
    matrix,
    document,
    size,
  }: {
    mask: NonNullable<LayerView["vectorMask"]>;
    /** From the layer's space to the document's (null in perspective: drawn unplaced). */
    matrix: Matrix | null;
    /** The document's size: the thumbnail shows the canvas. */
    document: { width: number; height: number };
    /** Side of the square box, in CSS pixels. */
    size: number;
  } = $props();

  const outline = $derived(maskOutline(mask, matrix));
  const rule = $derived(mask.geometry.kind === "path" && mask.geometry.rule === "evenodd");
  const fitted = $derived.by(() => {
    const scale = size / Math.max(document.width, document.height, 1);
    return { width: document.width * scale, height: document.height * scale };
  });
</script>

<span class="box" style:width="{size}px" style:height="{size}px">
  <svg
    class:inverted={mask.inverted}
    width={fitted.width}
    height={fitted.height}
    viewBox="0 0 {document.width} {document.height}"
    preserveAspectRatio="none"
    aria-hidden="true"
  >
    <rect class="hidden" width={document.width} height={document.height} />
    {#if outline}
      <path class="shown" d={outline} fill-rule={rule ? "evenodd" : "nonzero"} />
    {/if}
  </svg>
</span>

<style>
  .box {
    display: grid;
    place-items: center;
    flex: none;
  }

  svg {
    outline: 1px solid var(--border-strong);
  }

  .hidden {
    fill: #808080;
  }

  .shown {
    fill: #ffffff;
    stroke: #303030;
    stroke-width: 1px;
    vector-effect: non-scaling-stroke;
  }

  .inverted .hidden {
    fill: #ffffff;
  }

  .inverted .shown {
    fill: #808080;
  }
</style>
