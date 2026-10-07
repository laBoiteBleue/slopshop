<script lang="ts">
  // The Sources panel (in the dock, ADR 0040): what the pixel layers show, kept once, like a
  // video editor's media bin. Each row is a source: its thumbnail, its name, its size and how
  // many layers show it. A click selects those layers; the right-click menu also makes a new
  // layer showing it. The source of the active layer is marked. Sources are read-only and go
  // away with the last layer (or history step) using them.
  import ContextMenu from "./ContextMenu.svelte";
  import type { MenuItem } from "./MenuBar.svelte";
  import type { SourceView } from "./engine";
  import SourceThumbnail from "./SourceThumbnail.svelte";
  import { t } from "./i18n/index.svelte";

  let {
    documentId,
    sources,
    name,
    active = null,
    onselect,
    onnewlayer,
  }: {
    documentId: number;
    sources: SourceView[];
    /** The name a source shows (its own, else its first layer's). */
    name: (source: SourceView) => string;
    /** The source of the active layer, marked. */
    active?: number | null;
    /** Select these layers (those showing a source). */
    onselect: (layers: number[]) => void;
    /** A new layer showing this source. */
    onnewlayer: (source: SourceView) => void;
  } = $props();

  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  function count(source: SourceView): string {
    const n = source.layers.length;
    return n === 1 ? t("sources.layers.one") : t("sources.layers.other", { count: n });
  }

  function openMenu(e: MouseEvent, source: SourceView) {
    e.preventDefault();
    const command = (label: string, run: () => void): MenuItem => ({
      kind: "command",
      label,
      run,
      disabled: false,
    });
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        command(t("sources.select"), () => onselect(source.layers)),
        command(t("sources.newLayer"), () => onnewlayer(source)),
      ],
    };
  }
</script>

<section class="panel" aria-label={t("sources.title")}>
  {#if sources.length === 0}
    <p class="empty">{t("sources.empty")}</p>
  {:else}
    <ul class="list" role="listbox" aria-label={t("sources.title")}>
      {#each sources as source (source.id)}
        <li
          role="option"
          aria-selected={active === source.id}
          class:active={active === source.id}
          onclick={() => onselect(source.layers)}
          oncontextmenu={(e) => openMenu(e, source)}
          onkeydown={() => {}}
        >
          <SourceThumbnail {documentId} source={source.id} size={32} />
          <span class="text">
            <span class="source-name">{name(source) || t("sources.unnamed")}</span>
            <span class="detail">
              {t("sources.size", { width: source.width, height: source.height })} · {count(source)}
            </span>
          </span>
        </li>
      {/each}
    </ul>
  {/if}
</section>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} onclose={() => (menu = null)} />
{/if}

<style>
  .panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  .empty {
    flex: 1;
    margin: 0;
    padding: 10px;
    color: var(--text-muted);
  }

  .list {
    flex: 1;
    min-height: 0;
    margin: 0;
    padding: 0;
    overflow: auto;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 44px;
    padding: 0 10px;
    border-bottom: 1px solid var(--border-dark);
    cursor: default;
  }

  li:hover {
    background: var(--hover);
  }

  /* The active layer's source: the accent along the row. */
  li.active {
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .source-name,
  .detail {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .detail {
    color: var(--text-muted);
    font-size: 11px;
  }
</style>
