<script lang="ts">
  // The options bar (ADR 0013): the settings of the active tool, under the menu bar. Only what
  // belongs to the tool: view and apply/cancel commands live in the menus and on the keys.
  import type { SelectionMode } from "./engine";
  import Icon, { type IconName } from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
  import type { MessageKey } from "./i18n/en";
  import { MAX_FEATHER } from "./selection";
  import { isSelectionTool, toolInfo, type ToolId } from "./tools";
  import { keepFocus } from "./platform";

  let {
    tool,
    autoSelect = $bindable(),
    selectionMode = $bindable(),
    feather = $bindable(),
    antiAlias = $bindable(),
  }: {
    tool: ToolId;
    /** Move tool: a drag takes the layer under the pointer (Ctrl inverts it). */
    autoSelect: boolean;
    /** Selection tools: how a new shape combines with the selection (keys override it). */
    selectionMode: SelectionMode;
    /** Selection tools: Gaussian softening of the edge, in pixels. */
    feather: number;
    /** Elliptical Marquee and lassos: smooth edges. */
    antiAlias: boolean;
  } = $props();

  const current = $derived(toolInfo(tool));

  const MODES: { mode: SelectionMode; icon: IconName; label: MessageKey }[] = [
    { mode: "replace", icon: "selectionReplace", label: "options.mode.replace" },
    { mode: "add", icon: "selectionAdd", label: "options.mode.add" },
    { mode: "subtract", icon: "selectionSubtract", label: "options.mode.subtract" },
    { mode: "intersect", icon: "selectionIntersect", label: "options.mode.intersect" },
  ];

  /** Typed values apply at once (the next shape uses them, even while the field has focus). */
  function setFeather(value: number) {
    if (Number.isFinite(value)) feather = Math.min(Math.max(value, 0), MAX_FEATHER);
  }
</script>

<div class="options" role="toolbar" aria-label={t("options.label")}>
  <span class="tool-icon" title={t(current.name)}>
    <Icon name={current.icon} size={16} />
  </span>
  <span class="divider"></span>

  {#if tool === "move"}
    <label class="option">
      <input type="checkbox" bind:checked={autoSelect} />
      {t("options.autoSelect")}
    </label>
  {:else if isSelectionTool(tool)}
    {#each MODES as entry (entry.mode)}
      <button
        class="icon-btn"
        class:on={selectionMode === entry.mode}
        onmousedown={keepFocus}
        aria-pressed={selectionMode === entry.mode}
        title={t(entry.label)}
        aria-label={t(entry.label)}
        onclick={() => (selectionMode = entry.mode)}
      >
        <Icon name={entry.icon} />
      </button>
    {/each}
    <span class="divider"></span>
    <label class="option">
      {t("options.feather")}
      <input
        type="number"
        min="0"
        max={MAX_FEATHER}
        step="1"
        value={feather}
        oninput={(e) => setFeather(e.currentTarget.valueAsNumber)}
        onchange={(e) => (e.currentTarget.valueAsNumber = feather)}
      />
      px
    </label>
    {#if tool !== "marquee"}
      <label class="option">
        <input type="checkbox" bind:checked={antiAlias} />
        {t("options.antiAlias")}
      </label>
    {/if}
  {/if}
</div>

<style>
  .options {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 0 8px;
    background: var(--chrome);
    border-bottom: 1px solid var(--border-dark);
  }

  .tool-icon {
    display: grid;
    place-items: center;
    width: 24px;
    color: var(--text);
  }

  .divider {
    align-self: stretch;
    width: 1px;
    margin: 6px 2px;
    background: var(--border-strong);
  }

  .option {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }

  .option input[type="checkbox"] {
    margin: 0;
  }

  .option input[type="number"] {
    width: 52px;
  }

  .icon-btn.on {
    background: var(--selected);
    color: var(--text);
  }
</style>
