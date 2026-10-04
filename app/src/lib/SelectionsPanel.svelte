<script lang="ts">
  // The Selections panel (in the dock): the document's saved selections by name, a document's
  // objects rather than Photoshop's alpha channels. A click loads one (it becomes the
  // selection); Shift+click adds it, Alt+click subtracts it, Shift+Alt+click intersects, as the
  // selection tools' keys do. A double-click renames; the right-click menu has every command;
  // Delete removes the row last clicked, a press on the empty part of the list deselects it.
  // "+" saves the current selection.
  import ContextMenu from "./ContextMenu.svelte";
  import type { MenuItem } from "./MenuBar.svelte";
  import type { SavedSelectionView, SelectionMode } from "./engine";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
  import { keepFocus } from "./platform";

  let {
    saved,
    selected,
    onload,
    onsave,
    onreplace,
    onrename,
    ondelete,
  }: {
    saved: SavedSelectionView[];
    /** Something is selected now: it can be saved, or replace a saved one. */
    selected: boolean;
    onload: (id: number, mode: SelectionMode) => void;
    /** Save the current selection under a new name (asks it). */
    onsave: () => void;
    /** A saved selection gets the current selection. */
    onreplace: (id: number) => void;
    onrename: (id: number, name: string) => void;
    ondelete: (id: number) => void;
  } = $props();

  /** The row last clicked: Delete removes it. */
  let current = $state<number | null>(null);
  let renaming = $state<number | null>(null);
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);

  /** How a click combines, from its keys (those of the selection tools). */
  function modeOf(e: MouseEvent | KeyboardEvent): SelectionMode {
    if (e.shiftKey && e.altKey) return "intersect";
    if (e.shiftKey) return "add";
    if (e.altKey) return "subtract";
    return "replace";
  }

  function click(e: MouseEvent, entry: SavedSelectionView) {
    current = entry.id;
    if (e.detail > 1) return;
    onload(entry.id, modeOf(e));
  }

  function commitRename(entry: SavedSelectionView, input: HTMLInputElement) {
    if (renaming !== entry.id) return;
    renaming = null;
    const name = input.value.trim();
    if (name && name !== entry.name) onrename(entry.id, name);
  }

  function openMenu(e: MouseEvent, entry: SavedSelectionView) {
    e.preventDefault();
    current = entry.id;
    const command = (label: string, run: () => void, disabled = false): MenuItem => ({
      kind: "command",
      label,
      run,
      disabled,
    });
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        command(t("selections.load"), () => onload(entry.id, "replace")),
        command(t("selections.add"), () => onload(entry.id, "add")),
        command(t("selections.subtract"), () => onload(entry.id, "subtract")),
        command(t("selections.intersect"), () => onload(entry.id, "intersect")),
        { kind: "separator" },
        command(t("selections.replace"), () => onreplace(entry.id), !selected),
        command(t("selections.rename"), () => (renaming = entry.id)),
        command(t("selections.delete"), () => ondelete(entry.id)),
      ],
    };
  }

  function onkeydown(e: KeyboardEvent) {
    if (renaming !== null || current === null) return;
    if (e.key === "Delete" || e.key === "Backspace") {
      e.preventDefault();
      e.stopPropagation();
      ondelete(current);
      current = null;
    } else if (e.key === "Enter") {
      e.preventDefault();
      onload(current, modeOf(e));
    }
  }

  function focusInput(input: HTMLInputElement) {
    input.select();
  }
</script>

<section class="panel" aria-label={t("selections.title")}>
  {#if saved.length === 0}
    <p class="empty">{t("selections.empty")}</p>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <!-- A press on the empty part of the list deselects its row, as in the Layers panel. -->
    <ul
      class="list"
      role="listbox"
      aria-label={t("selections.title")}
      tabindex="0"
      {onkeydown}
      onpointerdown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) current = null;
      }}
    >
      {#each saved as entry (entry.id)}
        <li
          role="option"
          aria-selected={current === entry.id}
          class:current={current === entry.id}
          title={t("selections.hint")}
          onclick={(e) => click(e, entry)}
          ondblclick={() => (renaming = entry.id)}
          oncontextmenu={(e) => openMenu(e, entry)}
          onkeydown={() => {}}
        >
          <Icon name="marquee" size={16} />
          {#if renaming === entry.id}
            <input
              class="rename"
              value={entry.name}
              aria-label={t("selections.rename")}
              use:focusInput
              onclick={(e) => e.stopPropagation()}
              onkeydown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") commitRename(entry, e.currentTarget);
                else if (e.key === "Escape") renaming = null;
              }}
              onblur={(e) => commitRename(entry, e.currentTarget)}
            />
          {:else}
            <span class="name">{entry.name}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  <div class="footer">
    <button
      type="button"
      class="icon-btn"
      title={t("selections.save")}
      aria-label={t("selections.save")}
      disabled={!selected}
      onmousedown={keepFocus}
      onclick={onsave}
    >
      <Icon name="plus" />
    </button>
    <button
      type="button"
      class="icon-btn"
      title={t("selections.delete")}
      aria-label={t("selections.delete")}
      disabled={current === null || !saved.some((s) => s.id === current)}
      onmousedown={keepFocus}
      onclick={() => {
        if (current !== null) ondelete(current);
        current = null;
      }}
    >
      <Icon name="trash" />
    </button>
  </div>
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
    outline: none;
  }

  li {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 28px;
    padding: 0 10px;
    border-bottom: 1px solid var(--border-dark);
    cursor: default;
  }

  li:hover {
    background: var(--hover);
  }

  li.current {
    background: var(--selected);
  }

  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .rename {
    flex: 1;
    min-width: 0;
  }

  .footer {
    display: flex;
    justify-content: flex-end;
    gap: 2px;
    padding: 2px 6px;
    border-top: 1px solid var(--border-dark);
  }
</style>
