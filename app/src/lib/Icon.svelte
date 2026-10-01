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
    hand: "M7 12V6.5a1.5 1.5 0 0 1 3 0V11 M10 11V4.5a1.5 1.5 0 0 1 3 0V11 M13 11V6a1.5 1.5 0 0 1 3 0v6 M16 12V9a1.5 1.5 0 0 1 3 0v5a8 8 0 0 1-8 8h-.5a7 7 0 0 1-5.4-2.6l-2.4-3a1.5 1.5 0 0 1 2.2-2L7 16",
    zoom: "M10 3a7 7 0 1 0 0 14a7 7 0 1 0 0-14Z M15 15l6 6",
    zoomIn: "M10 3a7 7 0 1 0 0 14a7 7 0 1 0 0-14Z M15 15l6 6 M7 10h6 M10 7v6",
    zoomOut: "M10 3a7 7 0 1 0 0 14a7 7 0 1 0 0-14Z M15 15l6 6 M7 10h6",
    check: "M5 12.5l4.5 4.5L19 7.5",
    cancel: "M12 3a9 9 0 1 0 0 18a9 9 0 1 0 0-18Z M5.6 5.6l12.8 12.8",
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
