<script lang="ts" module>
  // Minimal stroke icons (24×24 grid). Add paths here rather than pulling an icon library.
  const paths = {
    eye: "M2 12s3.6-6.5 10-6.5S22 12 22 12s-3.6 6.5-10 6.5S2 12 2 12Z M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6Z",
    plus: "M12 5v14 M5 12h14",
    trash: "M4 7h16 M9 7V4h6v3 M6 7l1 13h10l1-13 M10 11v5 M14 11v5",
    folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z",
    folderPlus:
      "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z M12 10v6 M9 13h6",
    chevronRight: "M9 6l6 6-6 6",
    chevronDown: "M6 9l6 6 6-6",
    clip: "M8 4v9a3 3 0 0 0 3 3h7 M15 13l3 3-3 3",
    adjust: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18Z M12 3v18 M12 7h5 M12 11h7 M12 15h6",
    // Tools.
    move: "M12 3v18 M3 12h18 M9 6l3-3 3 3 M9 18l3 3 3-3 M6 9l-3 3 3 3 M18 9l3 3-3 3",
    crop: "M7 2v15h15 M2 7h15v15",
    marquee: "M4 4h3 M10 4h4 M17 4h3v3 M20 10v4 M20 17v3h-3 M14 20h-4 M7 20H4v-3 M4 14v-4 M4 7V4",
    ellipse:
      "M20.42 13.18A8.5 8.5 0 0 1 18.79 17.12 M17.12 18.79A8.5 8.5 0 0 1 13.18 20.42 M10.82 20.42A8.5 8.5 0 0 1 6.88 18.79 M5.21 17.12A8.5 8.5 0 0 1 3.58 13.18 M3.58 10.82A8.5 8.5 0 0 1 5.21 6.88 M6.88 5.21A8.5 8.5 0 0 1 10.82 3.58 M13.18 3.58A8.5 8.5 0 0 1 17.12 5.21 M18.79 6.88A8.5 8.5 0 0 1 20.42 10.82",
    lasso:
      "M12 4C7 4 3.5 6.5 3.5 9.5S7 15 12 15s8.5-2.5 8.5-5.5S17 4 12 4Z M6.5 14c-1.5 1.5-1.2 3.6.8 4.3c1.6.6 1.2 2.6-.8 2.7",
    polygonalLasso: "M4 8l7-5 9 4-2 9-8 1Z M10 17l-2.5 4",
    wand: "M4 20L14.5 9.5 M13 8l3 3 M16 3v2.5 M20.5 7.5H18 M19.2 4.3l-1.6 1.6 M11.5 4.5l1 1 M19.5 12l-1-1",
    // Selection modes.
    selectionReplace: "M5 5h14v14H5Z",
    selectionAdd: "M3 3h12v12H3Z M18 14v8 M14 18h8",
    selectionSubtract: "M3 3h12v12H3Z M14 18h8",
    selectionIntersect: "M3 3h12v12H3Z M9 9h12v12H9Z",
    // View.
    fitScreen: "M4 9V4h5 M15 4h5v5 M20 15v5h-5 M9 20H4v-5",
    actualPixels: "M4.5 8.5l3-2.5v12 M12 9v1.5 M12 14v1.5 M16.5 8.5l3-2.5v12",
  } as const;

  export type IconName = keyof typeof paths;
</script>

<script lang="ts">
  let { name, size = 16 }: { name: IconName; size?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={paths[name]} />
</svg>
