<script lang="ts">
  // A path's thumbnail (ADR 0041), as Photoshop draws those of the Paths panel and of vector
  // masks: the canvas gray, what the path encloses white, its outline between; `inverted` (a
  // vector mask hiding what it encloses) swaps them.
  let {
    outline,
    evenOdd = false,
    inverted = false,
    document,
    size,
  }: {
    /** SVG path data in document pixels. */
    outline: string;
    evenOdd?: boolean;
    inverted?: boolean;
    /** The document's size: the thumbnail shows the canvas. */
    document: { width: number; height: number };
    /** Side of the square box, in CSS pixels. */
    size: number;
  } = $props();

  const fitted = $derived.by(() => {
    const scale = size / Math.max(document.width, document.height, 1);
    return { width: document.width * scale, height: document.height * scale };
  });
</script>

<span class="box" style:width="{size}px" style:height="{size}px">
  <svg
    class:inverted
    width={fitted.width}
    height={fitted.height}
    viewBox="0 0 {document.width} {document.height}"
    preserveAspectRatio="none"
    aria-hidden="true"
  >
    <rect class="hidden" width={document.width} height={document.height} />
    {#if outline}
      <path class="shown" d={outline} fill-rule={evenOdd ? "evenodd" : "nonzero"} />
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
