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
    // A dashed selection with a turning arrow (Select > Reselect).
    reselect: "M4 9V4h5 M13 4h2 M20 9v2 M4 13v2 M9 20H4v-3 M20 15a5 5 0 1 1-5-5h3 M16 8l2 2-2 2",
    // Three sliders (the Properties panel).
    // A funnel (Filter > …, ADR 0034).
    filter: "M4 5h16l-6 7.5V18l-4 2v-7.5z",
    sliders: "M4 6h9 M17 6h3 M15 4v4 M4 12h3 M11 12h9 M9 10v4 M4 18h11 M19 18h1 M17 16v4",
    history: "M3.5 12a8.5 8.5 0 1 0 2.5-6 M3.5 3.5V8h4.5 M12 7.5V12l3 2",
    histogram: "M3 20h18 M5 20v-5 M8 20v-9 M11 20V6 M14 20v-7 M17 20v-11 M20 20v-4",
    info: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18Z M12 11v6 M12 7.5v.5",
    // Two chain links, one above the other (linked proportions).
    link: "M8.5 6.5a3.5 3.5 0 0 1 7 0v3a3.5 3.5 0 0 1-7 0Z M8.5 14.5a3.5 3.5 0 0 1 7 0v3a3.5 3.5 0 0 1-7 0Z M12 8.5v7",
    // The same links apart (proportions free).
    linkBroken:
      "M8.5 4.5a3.5 3.5 0 0 1 7 0v2a3.5 3.5 0 0 1-7 0Z M8.5 17.5a3.5 3.5 0 0 1 7 0v2a3.5 3.5 0 0 1-7 0Z M5 12h3 M16 12h3",
    // Liquify's tools (ADR 0037).
    liquifyForward: "M3 13c3-5 6 4 9-1s5-3 7-1 M16 6l5 5-5 5",
    liquifyReconstruct: "M5 9h9a5 5 0 0 1 0 10H8 M8 5L4 9l4 4",
    liquifySmooth: "M3 9c3-4 6 4 9 0s6 4 9 0 M3 16c3-4 6 4 9 0s6 4 9 0",
    liquifyTwirl:
      "M12 12a1.5 1.5 0 0 1 3 0 3.5 3.5 0 0 1-7 0 5.5 5.5 0 0 1 11 0 M19 12l-2 2.2-2.2-2",
    liquifyPucker:
      "M4 4l5 5 M9 5v4H5 M20 4l-5 5 M15 5v4h4 M4 20l5-5 M9 19v-4H5 M20 20l-5-5 M15 19v-4h4",
    liquifyBloat:
      "M9 9L4 4 M4 8V4h4 M15 9l5-5 M16 4h4v4 M9 15l-5 5 M4 16v4h4 M15 15l5 5 M16 20h4v-4",
    liquifyPushLeft: "M19 12H5 M9 8l-4 4 4 4 M21 5v14",
    liquifyFreeze: "M5 5h14v14H5Z M5 12l7-7 M5 19L19 5 M12 19l7-7",
    liquifyThaw:
      "M5 5h3 M11 5h2 M16 5h3v3 M19 11v2 M19 16v3h-3 M13 19h-2 M8 19H5v-3 M5 13v-2 M5 8V5 M9 15l6-6",
    // Tools.
    pointer: "M6 3v15.5l4.2-4 2.9 6.5 2.7-1.2-2.9-6.4H18Z",
    // Align and Distribute (the Move tool's options): boxes against a line, or evenly spaced.
    alignLeft: "M4 3v18 M8 6h12v4H8Z M8 14h7v4H8Z",
    alignHorizontalCenters: "M12 3v18 M6 6h12v4H6Z M8.5 14h7v4h-7Z",
    alignRight: "M20 3v18 M4 6h12v4H4Z M9 14h7v4H9Z",
    alignTop: "M3 4h18 M6 8h4v12H6Z M14 8h4v7h-4Z",
    alignVerticalCenters: "M3 12h18 M6 6h4v12H6Z M14 8.5h4v7h-4Z",
    alignBottom: "M3 20h18 M6 4h4v12H6Z M14 9h4v7h-4Z",
    distributeHorizontalCenters:
      "M3 8h4v8H3Z M10 6h4v12h-4Z M17 9h4v6h-4Z M5 3v2 M12 2v2 M19 4v3 M5 19v2 M12 20v2 M19 17v3",
    distributeVerticalCenters:
      "M8 3h8v4H8Z M6 10h12v4H6Z M9 17h6v4H9Z M3 5h2 M2 12h2 M4 19h3 M19 5h2 M20 12h2 M17 19h3",
    distributeHorizontalSpacing: "M3 4v16 M21 4v16 M9 7h6v10H9Z M5 12h2 M17 12h2",
    distributeVerticalSpacing: "M4 3h16 M4 21h16 M7 9h10v6H7Z M12 5v2 M12 17v2",
    crop: "M7 2v15h15 M2 7h15v15",
    marquee: "M4 4h3 M10 4h4 M17 4h3v3 M20 10v4 M20 17v3h-3 M14 20h-4 M7 20H4v-3 M4 14v-4 M4 7V4",
    ellipse:
      "M20.42 13.18A8.5 8.5 0 0 1 18.79 17.12 M17.12 18.79A8.5 8.5 0 0 1 13.18 20.42 M10.82 20.42A8.5 8.5 0 0 1 6.88 18.79 M5.21 17.12A8.5 8.5 0 0 1 3.58 13.18 M3.58 10.82A8.5 8.5 0 0 1 5.21 6.88 M6.88 5.21A8.5 8.5 0 0 1 10.82 3.58 M13.18 3.58A8.5 8.5 0 0 1 17.12 5.21 M18.79 6.88A8.5 8.5 0 0 1 20.42 10.82",
    lasso:
      "M12 4C7 4 3.5 6.5 3.5 9.5S7 15 12 15s8.5-2.5 8.5-5.5S17 4 12 4Z M6.5 14c-1.5 1.5-1.2 3.6.8 4.3c1.6.6 1.2 2.6-.8 2.7",
    polygonalLasso: "M4 8l7-5 9 4-2 9-8 1Z M10 17l-2.5 4",
    objectSelection: "M3 3h3 M9 3h3 M3 3v3 M3 9v3 M15 3v2 M3 15h2 M11 11l9 3.5-4 1.5-1.5 4Z",
    quickSelection:
      "M20.5 3.5L13 11 M13 11l-2.5 2.5a2 2 0 0 1-3-3L10 8l3 3 M3 13.5a6 6 0 0 0 .7 2.5 M5.2 18.2a6 6 0 0 0 2.3 1.4 M10 20a6 6 0 0 0 2.5-.7",
    wand: "M4 20L14.5 9.5 M13 8l3 3 M16 3v2.5 M20.5 7.5H18 M19.2 4.3l-1.6 1.6 M11.5 4.5l1 1 M19.5 12l-1-1",
    brush:
      "M20 3.5L11 12.5 M11 12.5l1.5 1.5 M9.5 13.5c-2 0-3.5 1.5-3.5 3.5 0 1.5-1 2.5-2.5 2.5 1.5 1 6.5 1.5 7.5-2.5Z",
    eraser: "M15.5 4l5 5L11 18.5H6.5L3.5 15.5Z M10.5 9l5 5 M11 18.5h9",
    // The eraser with a turning arrow: what was painted comes back off.
    restoreEraser:
      "M15.5 8l4.5 4.5L12.5 20H8.5L6 17.5Z M11.5 12.5l4.5 4.5 M12.5 20h8 M3 9a5 5 0 0 1 9-3 M12.5 2.5V6H9",
    swap: "M15 4h4v4 M19 4l-5 5 M9 20H5v-4 M5 20l5-5",
    eyedropper: "M14.5 4.5l5 5 M17 2.5l4.5 4.5-3 3-4.5-4.5Z M15.5 8.5L6 18l-2 2 M6 18l-1.5.5.5-1.5",
    eyedropperAdd:
      "M14.5 4.5l5 5 M17 2.5l4.5 4.5-3 3-4.5-4.5Z M15.5 8.5L6 18l-2 2 M6 18l-1.5.5.5-1.5 M5 3v6 M2 6h6",
    eyedropperSubtract:
      "M14.5 4.5l5 5 M17 2.5l4.5 4.5-3 3-4.5-4.5Z M15.5 8.5L6 18l-2 2 M6 18l-1.5.5.5-1.5 M2 6h6",
    // Selection modes.
    selectionReplace: "M5 5h14v14H5Z",
    selectionAdd: "M3 3h12v12H3Z M18 14v8 M14 18h8",
    selectionSubtract: "M3 3h12v12H3Z M14 18h8",
    selectionIntersect: "M3 3h12v12H3Z M9 9h12v12H9Z",
    // Page orientation (File > New).
    portrait: "M7 3h10v18H7Z",
    landscape: "M3 7h18v10H3Z",
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
