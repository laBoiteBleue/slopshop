<script lang="ts">
  // The Paths panel (Window > Paths, ADR 0041), as Photoshop's: the document's saved paths and
  // its Work Path (in italics), then the active layer's shape path and vector mask path while it
  // is active. A click chooses a path (the Pen then draws into it and edits it); a press on the
  // empty part of the list chooses none. Ctrl+click on a thumbnail loads the path as the
  // selection (Shift adds, Alt subtracts, both intersect). A double-click renames a saved path,
  // or saves the Work Path. The right-click menu has every command; Delete removes the path
  // chosen. The buttons: Fill Path with the foreground color, Stroke Path with the Brush, Load
  // Path as a Selection, Make Work Path from Selection, Add a Mask, New Path, Delete Path; Alt
  // on Fill, Stroke and Make Work Path opens their dialog, as Photoshop's.
  import ContextMenu from "./ContextMenu.svelte";
  import type { MenuItem } from "./MenuBar.svelte";
  import type { PathRef, SelectionMode } from "./engine";
  import Icon, { type IconName } from "./Icon.svelte";
  import PathThumbnail from "./PathThumbnail.svelte";
  import { loadMode, sameRef, type PathEntry } from "./paths";
  import { t } from "./i18n/index.svelte";
  import type { MessageKey } from "./i18n/en";
  import { keepFocus } from "./platform";

  let {
    entries,
    chosen,
    document,
    selected,
    canMask,
    nextName,
    onchoose,
    onload,
    onfill,
    onstroke,
    onworkpath,
    onmask,
    onnew,
    ondelete,
    onrename,
    onsave,
    onduplicate,
  }: {
    entries: PathEntry[];
    /** The path chosen, if any. */
    chosen: PathRef | null;
    /** The document's size: thumbnails show the canvas. */
    document: { width: number; height: number };
    /** Something is selected: Make Work Path from Selection is possible. */
    selected: boolean;
    /** The chosen path can mask the active layer (Add a Mask). */
    canMask: boolean;
    /** The name Save Path proposes ("Path n"). */
    nextName: string;
    onchoose: (ref: PathRef | null) => void;
    onload: (ref: PathRef, mode: SelectionMode) => void;
    /** Fill Path: with the foreground color, or through Edit > Fill's dialog (`dialog`). */
    onfill: (ref: PathRef, dialog: boolean) => void;
    /** Stroke Path: with the tool chosen last (the Brush first), or its dialog. */
    onstroke: (ref: PathRef, dialog: boolean) => void;
    /** Make Work Path from Selection: at the last tolerance, or its dialog. */
    onworkpath: (dialog: boolean) => void;
    onmask: (ref: PathRef) => void;
    onnew: () => void;
    ondelete: (ref: PathRef) => void;
    onrename: (id: number, name: string) => void;
    /** Save Path: the Work Path kept as `name`. */
    onsave: (id: number, name: string) => void;
    onduplicate: (ref: PathRef, name: string) => void;
  } = $props();

  /** A row's name as the panel shows it. */
  function nameOf(entry: PathEntry): string {
    switch (entry.kind) {
      case "work":
        return t("paths.work");
      case "shape":
        return t("paths.shapePath", { name: entry.name });
      case "vectorMask":
        return t("paths.vectorMask", { name: entry.name });
      default:
        return entry.name;
    }
  }

  /** The saved path or Work Path whose name is being typed: renamed, or saved. */
  let naming = $state<{ id: number; save: boolean } | null>(null);
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  let rows = $state<HTMLElement>();

  const chosenEntry = $derived(entries.find((e) => sameRef(e.ref, chosen)) ?? null);
  /** Only the document's paths are deleted from the panel. */
  const deletable = $derived(
    chosenEntry !== null && (chosenEntry.kind === "path" || chosenEntry.kind === "work"),
  );

  function click(e: MouseEvent, entry: PathEntry) {
    if (e.detail > 1) return;
    onchoose(entry.ref);
  }

  function thumbnailClick(e: MouseEvent, entry: PathEntry) {
    if (!(e.ctrlKey || e.metaKey)) return;
    e.stopPropagation();
    onload(entry.ref, loadMode(e));
  }

  function startNaming(entry: PathEntry) {
    if (entry.ref.kind !== "path") return;
    if (entry.kind === "path") naming = { id: entry.ref.id, save: false };
    else if (entry.kind === "work") naming = { id: entry.ref.id, save: true };
  }

  function commitName(entry: PathEntry, input: HTMLInputElement) {
    const current = naming;
    if (!current || entry.ref.kind !== "path" || current.id !== entry.ref.id) return;
    naming = null;
    const name = input.value.trim();
    if (!name) return;
    if (current.save) onsave(current.id, name);
    else if (name !== entry.name) onrename(current.id, name);
  }

  function openMenu(e: MouseEvent, entry: PathEntry) {
    e.preventDefault();
    onchoose(entry.ref);
    const command = (label: MessageKey, run: () => void, disabled = false): MenuItem => ({
      kind: "command",
      label: t(label),
      run,
      disabled,
    });
    const own = entry.kind === "path" || entry.kind === "work";
    menu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        ...(entry.kind === "work" ? [command("paths.save", () => startNaming(entry))] : []),
        command("paths.duplicate", () =>
          onduplicate(entry.ref, t("paths.copyName", { name: nameOf(entry) })),
        ),
        command("paths.delete", () => ondelete(entry.ref), !own),
        ...(entry.kind === "path" ? [command("paths.rename", () => startNaming(entry))] : []),
        { kind: "separator" },
        command("paths.makeWorkPath", () => onworkpath(true), !selected),
        command("paths.makeSelection", () => onload(entry.ref, "replace")),
        command("paths.fill", () => onfill(entry.ref, true)),
        command("paths.stroke", () => onstroke(entry.ref, true)),
        { kind: "separator" },
        command("paths.addMask", () => onmask(entry.ref), !canMask || entry.ref.kind !== "path"),
      ],
    };
  }

  function onkeydown(e: KeyboardEvent) {
    if (naming !== null || !chosen) return;
    if ((e.key === "Delete" || e.key === "Backspace") && deletable) {
      e.preventDefault();
      e.stopPropagation();
      ondelete(chosen);
    }
  }

  function focusInput(input: HTMLInputElement) {
    input.select();
  }

  type Button = {
    icon: IconName;
    label: MessageKey;
    disabled: boolean;
    run: (alt: boolean) => void;
  };
  const buttons = $derived<Button[]>([
    {
      icon: "pathFill",
      label: "paths.fillButton",
      disabled: !chosen,
      run: (alt) => chosen && onfill(chosen, alt),
    },
    {
      icon: "pathStroke",
      label: "paths.strokeButton",
      disabled: !chosen,
      run: (alt) => chosen && onstroke(chosen, alt),
    },
    {
      icon: "ellipse",
      label: "paths.loadButton",
      disabled: !chosen,
      run: () => chosen && onload(chosen, "replace"),
    },
    {
      icon: "pathFromSelection",
      label: "paths.workPathButton",
      disabled: !selected,
      run: (alt) => onworkpath(alt),
    },
    {
      icon: "pathMask",
      label: "paths.addMask",
      disabled: !canMask,
      run: () => chosen && onmask(chosen),
    },
    { icon: "plus", label: "paths.new", disabled: false, run: () => onnew() },
    {
      icon: "trash",
      label: "paths.delete",
      disabled: !deletable,
      run: () => chosen && ondelete(chosen),
    },
  ]);
</script>

<section class="panel" aria-label={t("paths.title")}>
  {#if entries.length === 0}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <p class="empty" onclick={() => onchoose(null)}>{t("paths.empty")}</p>
  {:else}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <ul
      class="list"
      role="listbox"
      aria-label={t("paths.title")}
      tabindex="0"
      bind:this={rows}
      {onkeydown}
      onpointerdown={(e) => {
        if (e.button === 0 && e.target === rows) onchoose(null);
      }}
    >
      {#each entries as entry (entry.ref.kind + ":" + (entry.ref.kind === "path" ? entry.ref.id : entry.ref.layer))}
        {@const current = sameRef(entry.ref, chosen)}
        <li
          role="option"
          aria-selected={current}
          class:current
          class:temporary={entry.kind !== "path"}
          title={t("paths.hint")}
          onclick={(e) => click(e, entry)}
          ondblclick={() => startNaming(entry)}
          oncontextmenu={(e) => openMenu(e, entry)}
          onkeydown={() => {}}
        >
          <!-- svelte-ignore a11y_click_events_have_key_events -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <span
            class="thumb"
            title={t("paths.thumbnailHint")}
            data-testid="path-thumbnail"
            onclick={(e) => thumbnailClick(e, entry)}
          >
            <PathThumbnail
              outline={entry.outline}
              evenOdd={entry.evenOdd}
              inverted={false}
              {document}
              size={28}
            />
          </span>
          {#if naming && entry.ref.kind === "path" && naming.id === entry.ref.id}
            <input
              class="rename"
              value={naming.save ? nextName : entry.name}
              aria-label={t(naming.save ? "paths.save" : "paths.rename")}
              use:focusInput
              onclick={(e) => e.stopPropagation()}
              onkeydown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") commitName(entry, e.currentTarget);
                else if (e.key === "Escape") naming = null;
              }}
              onblur={(e) => commitName(entry, e.currentTarget)}
            />
          {:else}
            <span class="name">{nameOf(entry)}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
  <div class="footer">
    {#each buttons as button (button.icon)}
      <button
        type="button"
        class="icon-btn"
        title={t(button.label)}
        aria-label={t(button.label)}
        disabled={button.disabled}
        onmousedown={keepFocus}
        onclick={(e) => button.run(e.altKey)}
      >
        <Icon name={button.icon} />
      </button>
    {/each}
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
    height: 36px;
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

  /* The Work Path and the layer's paths: temporary, in italics as in Photoshop. */
  li.temporary .name {
    font-style: italic;
  }

  .thumb {
    display: flex;
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
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
    padding: 2px 6px;
    border-top: 1px solid var(--border-dark);
  }
</style>
