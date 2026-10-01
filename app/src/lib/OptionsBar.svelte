<script lang="ts">
  // The options bar (ADR 0013): the settings of the active tool, under the menu bar. Only what
  // belongs to the tool: view and apply/cancel commands live in the menus and on the keys.
  import Icon from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
  import { TOOLS, type ToolId } from "./tools";

  let {
    tool,
    autoSelect = $bindable(),
  }: {
    tool: ToolId;
    /** Move tool: a drag takes the layer under the pointer (Ctrl inverts it). */
    autoSelect: boolean;
  } = $props();

  const current = $derived(TOOLS.find((entry) => entry.id === tool) ?? TOOLS[0]);
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

  .option input {
    margin: 0;
  }
</style>
