<script lang="ts">
  // Perspective Warp's modes in the options bar, as in Photoshop: Layout (draw and fit the
  // planes over the image) and Warp (move their corners; nothing to warp without a plane).
  import { t } from "./i18n/index.svelte";
  import type { PerspectiveMode } from "./perspectiveWarp";
  import { keepFocus } from "./platform";

  let {
    mode,
    warpable,
    onmode,
  }: {
    mode: PerspectiveMode;
    /** Whether there is a plane to warp. */
    warpable: boolean;
    onmode: (mode: PerspectiveMode) => void;
  } = $props();

  const MODES = ["layout", "warp"] as const;
</script>

<span class="fields">
  {#each MODES as entry (entry)}
    <button
      type="button"
      class="btn small"
      class:on={mode === entry}
      aria-pressed={mode === entry}
      disabled={entry === "warp" && !warpable}
      onmousedown={keepFocus}
      onclick={() => mode !== entry && onmode(entry)}
    >
      {t(`perspectiveWarp.mode.${entry}`)}
    </button>
  {/each}
</span>

<style>
  .fields {
    display: contents;
  }

  .btn.on {
    background: var(--selected);
  }
</style>
