<script lang="ts">
  // Edit > Keyboard Shortcuts (Alt+Shift+Ctrl+K): every shortcut, read from the menus (whose
  // entries take theirs from `SHORTCUTS`), the toolbar's letters and the keys that act without
  // a menu entry. A list to read, with a filter; shortcuts are not customizable yet.
  import { onMount } from "svelte";
  import type { Menu, MenuItem } from "./MenuBar.svelte";
  import { SLOTS } from "./tools";
  import { modifierLabel } from "./platform";
  import { t } from "./i18n/index.svelte";

  type Entry = { label: string; keys: string[] };

  let {
    menus,
    extra = [],
    onclose,
  }: {
    menus: Menu[];
    /** Commands with a shortcut but without a menu entry (Duplicate and Transform Again). */
    extra?: Entry[];
    onclose: () => void;
  } = $props();
  type Section = { title: string; entries: Entry[] };

  let dialog: HTMLDialogElement;
  let filter = $state("");

  /** The entries of a menu that have a shortcut, submenus included ("Transform > …"). */
  function entries(items: MenuItem[], prefix = ""): Entry[] {
    return items.flatMap((item): Entry[] => {
      if (item.kind === "submenu") return entries(item.items, `${prefix}${item.label} › `);
      if (item.kind !== "command" || !item.shortcuts) return [];
      return [{ label: prefix + item.label, keys: item.shortcuts }];
    });
  }

  let sections = $derived.by((): Section[] => {
    const shift = t("key.shift");
    const tools: Entry[] = SLOTS.flatMap((slot) =>
      slot.tools.map((tool) => ({
        label: t(tool.name),
        keys: slot.tools.length > 1 ? [slot.key, `${shift}+${slot.key}`] : [slot.key],
      })),
    );
    const other: Entry[] = [
      ...extra,
      { label: t("shortcuts.pan"), keys: [t("shortcuts.spaceDrag")] },
      { label: t("shortcuts.apply"), keys: [t("shortcuts.enter")] },
      { label: t("shortcuts.cancel"), keys: [t("shortcuts.escape")] },
      { label: t("shortcuts.nudge"), keys: [t("shortcuts.arrows")] },
      { label: t("shortcuts.nudgeMore"), keys: [`${shift}+${t("shortcuts.arrows")}`] },
      { label: t("shortcuts.brushSize"), keys: ["[ ]"] },
      { label: t("shortcuts.brushHardness"), keys: [`${shift}+[ ]`] },
      { label: t("tools.defaultColors"), keys: ["D"] },
      { label: t("tools.swapColors"), keys: ["X"] },
      { label: t("shortcuts.nextDocument"), keys: ["Ctrl+Tab"] },
      { label: t("shortcuts.previousDocument"), keys: [`${shift}+Ctrl+Tab`] },
      { label: t("shortcuts.freely"), keys: [modifierLabel] },
    ];
    return [
      ...menus.map((menu) => ({ title: menu.label, entries: entries(menu.items) })),
      { title: t("tools.label"), entries: tools },
      { title: t("shortcuts.other"), entries: other },
    ];
  });

  /** The sections whose entries match the filter (a label or a shortcut), empty ones dropped. */
  let shown = $derived.by(() => {
    const words = filter.trim().toLocaleLowerCase();
    return sections
      .map((section) => ({
        ...section,
        entries: section.entries.filter(
          (entry) =>
            words === "" ||
            entry.label.toLocaleLowerCase().includes(words) ||
            entry.keys.some((key) => key.toLocaleLowerCase().includes(words)),
        ),
      }))
      .filter((section) => section.entries.length > 0);
  });

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="shortcuts-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <header id="shortcuts-title">{t("shortcuts.title")}</header>
  <div class="body">
    <!-- svelte-ignore a11y_autofocus -->
    <input
      type="search"
      placeholder={t("shortcuts.filter")}
      aria-label={t("shortcuts.filter")}
      bind:value={filter}
      autofocus
    />
    <div class="list">
      {#each shown as section (section.title)}
        <section>
          <h2>{section.title}</h2>
          <dl>
            {#each section.entries as entry (entry.label)}
              <dt>{entry.label}</dt>
              <dd>
                {#each entry.keys as key, i (key)}
                  {#if i > 0}<span class="or">{t("shortcuts.or")}</span>{/if}
                  <kbd>{key}</kbd>
                {/each}
              </dd>
            {/each}
          </dl>
        </section>
      {:else}
        <p class="muted">{t("shortcuts.none")}</p>
      {/each}
    </div>
  </div>
  <footer>
    <button type="button" class="btn primary" onclick={onclose}>{t("preferences.close")}</button>
  </footer>
</dialog>

<style>
  dialog {
    width: 560px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    color: var(--text);
    box-shadow: 0 10px 32px #0009;
  }

  dialog::backdrop {
    background: #00000055;
  }

  header {
    padding: 5px 10px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    font-weight: 600;
  }

  .body {
    display: grid;
    gap: 8px;
    padding: 10px;
  }

  input {
    width: 100%;
    box-sizing: border-box;
  }

  .list {
    display: grid;
    gap: 12px;
    max-height: min(60vh, 520px);
    overflow-y: auto;
    padding-right: 4px;
  }

  h2 {
    margin: 0 0 4px;
    font-size: inherit;
    font-weight: 600;
  }

  dl {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 2px 16px;
    margin: 0;
  }

  dt {
    color: var(--text);
  }

  dd {
    display: flex;
    align-items: center;
    gap: 4px;
    margin: 0;
    justify-self: end;
  }

  kbd {
    padding: 0 5px;
    border: 1px solid var(--border-dark);
    border-radius: 3px;
    background: var(--field);
    font: inherit;
    white-space: nowrap;
  }

  .or,
  .muted {
    color: var(--text-muted);
  }

  p {
    margin: 0;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>
