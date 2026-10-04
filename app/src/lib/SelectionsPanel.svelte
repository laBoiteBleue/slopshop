<script lang="ts">
  // The Selections panel (in the dock): the document's saved selections by name, a document's
  // objects rather than Photoshop's alpha channels. A click loads one (it becomes the
  // selection); Shift+click adds it, Alt+click subtracts it, Shift+Alt+click intersects, as the
  // selection tools' keys do. A double-click renames; the right-click menu has every command;
  // Delete removes the row last clicked; a press anywhere but on a row (the empty part of the
  // list, the image, another panel) deselects it, the panel's buttons and menu aside; on the
  // empty part of the list it also deselects in the image. "+" saves
  // the current selection.
  import ContextMenu from "./ContextMenu.svelte";
  import type { MenuItem } from "./MenuBar.svelte";
  import type { SavedSelectionView, SelectionMode } from "./engine";
  import type { CombinedRow } from "./savedSelections";
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
    ondeselect,
    canReselect = false,
    onreselect,
    combined = [],
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
    /** A press on the empty part of the list: nothing selected in the image (Select > Deselect),
     * as a press under the layers deselects them. */
    ondeselect?: () => void;
    /** Select > Reselect has a selection to bring back: the pinned first row does it. */
    canReselect?: boolean;
    onreselect?: () => void;
    /** The saved selections the image's selection is made of, and how: shown on their rows. */
    combined?: CombinedRow[];
  } = $props();

  /** The sign of a row in the combination (a plain load: none). */
  const SIGNS: Record<SelectionMode, string> = {
    replace: "",
    add: "+",
    subtract: "−",
    intersect: "∩",
  };
  const combinedMode = (id: number) => combined.find((row) => row.id === id)?.mode ?? null;

  /** The row last clicked: Delete removes it. */
  let current = $state<number | null>(null);
  let rows = $state<HTMLElement>();
  let footer = $state<HTMLElement>();

  // A press outside the rows deselects the row; the panel's buttons act on it, and its menu
  // was opened for it.
  $effect(() => {
    const press = (e: PointerEvent) => {
      const target = e.target as Node | null;
      if (menu || !target) return;
      const onRow = rows?.contains(target) && target !== rows;
      if (!onRow && !footer?.contains(target)) current = null;
    };
    window.addEventListener("pointerdown", press, true);
    return () => window.removeEventListener("pointerdown", press, true);
  });
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
  <!-- Pinned above the saved ones, never scrolled away: the selection Deselect removed last. -->
  <button
    type="button"
    class="last"
    title={t("selections.last.hint")}
    disabled={!canReselect}
    onmousedown={keepFocus}
    onclick={() => onreselect?.()}
  >
    <Icon name="reselect" size={16} />
    <span class="name">{t("selections.last")}</span>
  </button>
  {#if saved.length === 0}
    <p class="empty">{t("selections.empty")}</p>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <ul
      class="list"
      role="listbox"
      aria-label={t("selections.title")}
      tabindex="0"
      bind:this={rows}
      {onkeydown}
      onpointerdown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) ondeselect?.();
      }}
    >
      {#each saved as entry (entry.id)}
        <li
          role="option"
          aria-selected={current === entry.id}
          class:current={current === entry.id}
          class:combined={combinedMode(entry.id) !== null}
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
          {#if combinedMode(entry.id)}
            {@const mode = combinedMode(entry.id) as SelectionMode}
            <span class="sign" title={t(`selections.combined.${mode}`)}>{SIGNS[mode]}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  <div class="footer" bind:this={footer}>
    <span class="keys">{t("selections.keys")}</span>
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

  .last {
    display: flex;
    flex: none;
    align-items: center;
    gap: 8px;
    height: 28px;
    padding: 0 10px;
    border: none;
    border-bottom: 1px solid var(--border-strong);
    border-radius: 0;
    background: transparent;
    color: var(--text);
    text-align: left;
    font-style: italic;
  }

  .last:hover:not(:disabled) {
    background: var(--hover);
  }

  .last:disabled {
    color: var(--text-disabled);
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

  /* In the selection now: the accent along the row, and its sign. */
  li.combined {
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .sign {
    margin-left: auto;
    color: var(--accent);
    font-weight: 600;
  }

  .keys {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--text-muted);
    font-size: 11px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
    padding: 2px 6px;
    border-top: 1px solid var(--border-dark);
  }
</style>
