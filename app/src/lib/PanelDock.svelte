<script lang="ts">
  // The dock below Layers (maintainer's choice, 2026-10-04): a row of tab icons, one panel
  // unfolded under them. A click on a tab unfolds its panel; on the unfolded one's tab, the dock
  // folds down to the icons and Layers takes the room. The edge above the dock resizes it
  // (a double-click puts the default height back). Tabs are reordered by dragging them along
  // the row. The app saves all this with the layout.
  import type { Snippet } from "svelte";
  import Icon, { type IconName } from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
  import {
    DEFAULT_DOCK,
    MIN_DOCK_HEIGHT,
    clampDockHeight,
    clickTab,
    type DockPanel,
    type DockState,
  } from "./panelDock";
  import { tabSlot } from "./tabs";

  let {
    dock = $bindable(),
    panels,
    content,
    onselect,
    onreorder,
  }: {
    dock: DockState;
    /** A tab was clicked (after the dock changed). */
    onselect?: (panel: DockPanel) => void;
    /** The tab at `from` dropped at insertion position `slot` (0: before the first). */
    onreorder?: (from: number, slot: number) => void;
    /** The tabs, left to right. */
    panels: { id: DockPanel; icon: IconName; label: string }[];
    /** The unfolded panel's content. */
    content: Snippet<[DockPanel]>;
  } = $props();

  let section: HTMLElement;
  let drag: { pointerId: number; y: number; height: number } | null = null;
  let dragging = $state(false);

  function select(panel: DockPanel) {
    if (moved) {
      // The click ending a drag of the tab.
      moved = false;
      return;
    }
    dock = clickTab(dock, panel);
    onselect?.(panel);
  }

  // A tab dragged along the row: where it would land (as the document tabs do).
  const TAB_DRAG_THRESHOLD = 4;
  let tabRow: HTMLElement;
  let tabDrag = $state<{
    pointerId: number;
    from: number;
    x: number;
    moved: boolean;
    slot: number | null;
  } | null>(null);
  /** The last drag moved its tab: the click that follows does not select it. */
  let moved = false;

  function onTabDown(e: PointerEvent, from: number) {
    if (e.button !== 0) return;
    tabDrag = { pointerId: e.pointerId, from, x: e.clientX, moved: false, slot: null };
  }

  function onTabMove(e: PointerEvent) {
    if (!tabDrag || tabDrag.pointerId !== e.pointerId) return;
    if (!tabDrag.moved) {
      if (Math.abs(e.clientX - tabDrag.x) < TAB_DRAG_THRESHOLD) return;
      tabDrag.moved = true;
      (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
    }
    const middles = [...tabRow.querySelectorAll<HTMLElement>(".tab")].map((tab) => {
      const rect = tab.getBoundingClientRect();
      return rect.left + rect.width / 2;
    });
    tabDrag.slot = tabSlot(middles, e.clientX, tabDrag.from);
  }

  function onTabUp(e: PointerEvent) {
    if (!tabDrag || tabDrag.pointerId !== e.pointerId) return;
    const { from, slot } = tabDrag;
    moved = tabDrag.moved;
    tabDrag = null;
    if (moved && slot !== null) onreorder?.(from, slot);
  }

  /** The column's height: the dock's parent. */
  const column = () => section.parentElement?.clientHeight || window.innerHeight;

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    drag = { pointerId: e.pointerId, y: e.clientY, height: dock.height };
    dragging = true;
  }

  function onpointermove(e: PointerEvent) {
    if (drag?.pointerId !== e.pointerId) return;
    // The edge is on top: moving it up makes the dock taller.
    dock = { ...dock, height: clampDockHeight(drag.height + drag.y - e.clientY, column()) };
  }

  function onpointerup(e: PointerEvent) {
    if (drag?.pointerId !== e.pointerId) return;
    drag = null;
    dragging = false;
  }

  function ondblclick() {
    dock = { ...dock, height: DEFAULT_DOCK.height };
  }

  const open = $derived(panels.find((p) => p.id === dock.open) ?? null);
</script>

<section
  class="dock"
  class:unfolded={open !== null}
  bind:this={section}
  style:height={open ? `${dock.height}px` : undefined}
>
  {#if open}
    <div
      class="edge"
      class:dragging
      role="separator"
      aria-orientation="horizontal"
      aria-label={t("dock.resize")}
      aria-valuenow={dock.height}
      aria-valuemin={MIN_DOCK_HEIGHT}
      {onpointerdown}
      {onpointermove}
      {onpointerup}
      onpointercancel={onpointerup}
      {ondblclick}
    ></div>
  {/if}
  <div class="tabs" role="tablist" aria-label={t("dock.label")} bind:this={tabRow}>
    {#each panels as panel, index (panel.id)}
      {@const selected = dock.open === panel.id}
      <button
        type="button"
        class="tab"
        class:selected
        class:dragging={tabDrag?.moved && tabDrag.from === index}
        class:drop-before={tabDrag?.slot === index}
        class:drop-after={index === panels.length - 1 && tabDrag?.slot === panels.length}
        role="tab"
        aria-selected={selected}
        aria-controls="dock-panel"
        title={selected ? t("dock.fold") : panel.label}
        onclick={() => select(panel.id)}
        onpointerdown={(e) => onTabDown(e, index)}
        onpointermove={onTabMove}
        onpointerup={onTabUp}
        onpointercancel={() => (tabDrag = null)}
      >
        <Icon name={panel.icon} size={15} />
        {#if selected}<span>{panel.label}</span>{/if}
      </button>
    {/each}
  </div>
  {#if open}
    <div class="content" id="dock-panel" role="tabpanel" aria-label={open.label}>
      {@render content(open.id)}
    </div>
  {/if}
</section>

<style>
  .dock {
    position: relative;
    display: flex;
    flex-direction: column;
    flex: none;
    min-height: 0;
    background: var(--panel);
    border-top: 1px solid var(--border-dark);
  }

  .edge {
    position: absolute;
    top: -3px;
    left: 0;
    right: 0;
    height: 6px;
    z-index: 2;
    cursor: row-resize;
  }

  .edge.dragging,
  .edge:hover {
    background: var(--accent);
    opacity: 0.5;
  }

  .tabs {
    display: flex;
    flex: none;
    height: 26px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
  }

  .tab {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    border: none;
    border-right: 1px solid var(--border-dark);
    border-radius: 0;
    background: transparent;
    color: var(--text-muted);
    font-weight: 600;
  }

  .tab:hover {
    color: var(--text);
  }

  .tab.selected {
    background: var(--panel);
    color: var(--text);
  }

  .tab.dragging {
    opacity: 0.5;
  }

  .tab.drop-before {
    box-shadow: inset 2px 0 var(--accent);
  }

  .tab.drop-after {
    box-shadow: inset -2px 0 var(--accent);
  }

  .content {
    flex: 1 1 0;
    min-height: 0;
    overflow: auto;
  }
</style>
