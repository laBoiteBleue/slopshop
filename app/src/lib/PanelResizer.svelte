<script lang="ts">
  import { t } from "./i18n/index.svelte";
  import { clampPanelWidth, DEFAULT_PANEL_WIDTH, MIN_PANEL_WIDTH } from "./panelWidth";

  /** The panels' width: dragging their left edge sets it (the app saves it with the layout). */
  let { width = $bindable() }: { width: number } = $props();

  let drag: { pointerId: number; x: number; width: number } | null = null;
  let dragging = $state(false);

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    drag = { pointerId: e.pointerId, x: e.clientX, width };
    dragging = true;
  }

  function onpointermove(e: PointerEvent) {
    if (drag?.pointerId !== e.pointerId) return;
    // The edge is on the left: moving it left widens the panels.
    width = clampPanelWidth(drag.width + drag.x - e.clientX, window.innerWidth);
  }

  function onpointerup(e: PointerEvent) {
    if (drag?.pointerId !== e.pointerId) return;
    drag = null;
    dragging = false;
  }

  /** A double-click puts the default width back. */
  function ondblclick() {
    width = DEFAULT_PANEL_WIDTH;
  }
</script>

<div
  class="resizer"
  class:dragging
  role="separator"
  aria-orientation="vertical"
  aria-label={t("panels.resize")}
  aria-valuenow={width}
  aria-valuemin={MIN_PANEL_WIDTH}
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  onpointercancel={onpointerup}
  {ondblclick}
></div>

<style>
  /* A strip over the column's left edge, wider than the 1 px line it straddles. */
  .resizer {
    position: absolute;
    top: 0;
    bottom: 0;
    left: -3px;
    z-index: 5;
    width: 6px;
    cursor: ew-resize;
    touch-action: none;
  }

  .resizer:hover,
  .resizer.dragging {
    background: linear-gradient(to right, transparent 2px, var(--accent) 2px 4px, transparent 4px);
  }
</style>
