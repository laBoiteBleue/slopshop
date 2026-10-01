<script lang="ts">
  // The toolbar (ADR 0013): a vertical strip on the left, one button per tool in Photoshop's
  // order.
  import Icon from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
  import { TOOLS, type ToolId } from "./tools";

  let {
    tool,
    onselect,
  }: {
    tool: ToolId;
    onselect: (tool: ToolId) => void;
  } = $props();
</script>

<nav class="toolbar" aria-label={t("tools.label")}>
  {#each TOOLS as entry (entry.id)}
    <button
      class="tool"
      class:active={entry.id === tool}
      aria-pressed={entry.id === tool}
      title={t("tools.tooltip", { name: t(entry.name), key: entry.key })}
      aria-label={t(entry.name)}
      onclick={() => onselect(entry.id)}
    >
      <Icon name={entry.icon} size={18} />
    </button>
  {/each}
</nav>

<style>
  .toolbar {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 6px 0;
    background: var(--chrome);
  }

  .tool {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--text-muted);
  }

  .tool:hover {
    background: var(--overlay-hover);
    color: var(--text);
  }

  .tool.active {
    background: var(--selected);
    color: var(--text);
  }

  .tool:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
</style>
