<script lang="ts">
  // A vector mask's thumbnail in the Layers panel (ADR 0041), as Photoshop draws it: the canvas
  // gray where the mask hides the layer, white where it shows it, the path's outline between.
  import type { LayerView, Matrix } from "./engine";
  import PathThumbnail from "./PathThumbnail.svelte";
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
  const evenOdd = $derived(mask.geometry.kind === "path" && mask.geometry.rule === "evenodd");
</script>

<PathThumbnail {outline} {evenOdd} inverted={mask.inverted} {document} {size} />
