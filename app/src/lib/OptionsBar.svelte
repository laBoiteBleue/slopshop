<script lang="ts">
  // The options bar (ADR 0013): the settings of the active tool, under the menu bar. While a
  // frame or a box waits on the image (Crop, Free Transform), it also offers to apply or cancel
  // it, as Photoshop does.
  import Icon from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
  import { TOOLS, type ToolId } from "./tools";

  let {
    tool,
    autoSelect = $bindable(),
    zoomOut = $bindable(),
    hasDocument,
    pending,
    onactualsize,
    onfit,
  }: {
    tool: ToolId;
    /** Move tool: a drag takes the layer under the pointer (Ctrl inverts it). */
    autoSelect: boolean;
    /** Zoom tool: a click zooms out rather than in (Alt inverts it). */
    zoomOut: boolean;
    hasDocument: boolean;
    /** A crop frame or a transform box on the image, to apply or cancel. */
    pending: { commit: () => void; cancel: () => void } | null;
    onactualsize: () => void;
    onfit: () => void;
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
  {:else if tool === "zoom" || tool === "hand"}
    {#if tool === "zoom"}
      <button
        class="icon-btn"
        class:on={!zoomOut}
        aria-pressed={!zoomOut}
        title={t("menu.view.zoomIn")}
        aria-label={t("menu.view.zoomIn")}
        onclick={() => (zoomOut = false)}
      >
        <Icon name="zoomIn" />
      </button>
      <button
        class="icon-btn"
        class:on={zoomOut}
        aria-pressed={zoomOut}
        title={t("menu.view.zoomOut")}
        aria-label={t("menu.view.zoomOut")}
        onclick={() => (zoomOut = true)}
      >
        <Icon name="zoomOut" />
      </button>
      <span class="divider"></span>
    {/if}
    <button class="btn small" disabled={!hasDocument} onclick={onactualsize}>
      {t("menu.view.actualSize")}
    </button>
    <button class="btn small" disabled={!hasDocument} onclick={onfit}>
      {t("menu.view.fit")}
    </button>
  {/if}

  {#if pending}
    <span class="end">
      <button
        class="icon-btn"
        title={t("options.cancel")}
        aria-label={t("options.cancel")}
        onclick={pending.cancel}
      >
        <Icon name="cancel" />
      </button>
      <button
        class="icon-btn commit"
        title={t("options.commit")}
        aria-label={t("options.commit")}
        onclick={pending.commit}
      >
        <Icon name="check" />
      </button>
    </span>
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

  .icon-btn.on {
    background: var(--selected);
    color: var(--text);
  }

  .end {
    display: inline-flex;
    gap: 2px;
    margin-left: auto;
  }

  .icon-btn.commit {
    color: var(--accent);
  }
</style>
