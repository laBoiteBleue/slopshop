<script lang="ts" module>
  // Minimal stroke icons (24×24 grid). Add paths here rather than pulling an icon library.
  const paths = {
    eye: "M2 12s3.6-6.5 10-6.5S22 12 22 12s-3.6 6.5-10 6.5S2 12 2 12Z M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6Z",
    undo: "M9 14 4 9l5-5 M4 9h10.5a5.5 5.5 0 0 1 0 11H11",
    redo: "m15 14 5-5-5-5 M20 9H9.5a5.5 5.5 0 0 0 0 11H13",
    plus: "M12 5v14 M5 12h14",
    trash: "M4 7h16 M9 7V4h6v3 M6 7l1 13h10l1-13 M10 11v5 M14 11v5",
    open: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z",
    export: "M12 15V4 M8 8l4-4 4 4 M5 14v4a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-4",
    save: "M5 4h11l4 4v11a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1Z M8 4v5h7V4 M8 20v-6h8v6",
    saveAs:
      "M12 20H5a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1h11l4 4v4 M8 4v5h7V4 M8 20v-6h4 M18 15v6 M15 18h6",
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
