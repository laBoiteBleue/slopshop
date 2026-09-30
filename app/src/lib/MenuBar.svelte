<script lang="ts" module>
  /** An entry of a menu. Labels are already translated. */
  export type MenuItem =
    | {
        kind: "command";
        label: string;
        /** Displayed only: the shortcut itself is handled by the owner. */
        shortcut?: string;
        disabled?: boolean;
        /** Radio or toggle state, shown with a check mark. */
        checked?: boolean;
        run: () => void;
      }
    | { kind: "separator" }
    | { kind: "submenu"; label: string; items: MenuItem[]; disabled?: boolean };

  export type Menu = { label: string; items: MenuItem[] };
</script>

<script lang="ts">
  // The window's menu bar (ADR 0013): opens on click, then follows the pointer from menu to
  // menu, like desktop menu bars. Keyboard: arrows move, Enter activates, Escape closes.
  let { menus }: { menus: Menu[] } = $props();

  /** The open menu, and the highlighted entry of it and of its open submenu. */
  let open = $state<number | null>(null);
  let highlighted = $state<number | null>(null);
  let submenu = $state<number | null>(null);
  let subHighlighted = $state<number | null>(null);
  let bar: HTMLElement;

  const selectable = (item: MenuItem) => item.kind !== "separator" && !item.disabled;

  function openMenu(index: number | null) {
    open = index;
    highlighted = null;
    submenu = null;
    subHighlighted = null;
  }

  function activate(item: MenuItem, index: number, inSubmenu: boolean) {
    if (!selectable(item)) return;
    if (item.kind === "submenu") {
      submenu = index;
      subHighlighted = item.items.findIndex(selectable);
      return;
    }
    if (item.kind === "command") {
      openMenu(null);
      item.run();
    }
    void inSubmenu;
  }

  /** The next selectable entry of `items` from `from`, in direction `step`, wrapping. */
  function next(items: MenuItem[], from: number | null, step: number): number | null {
    const n = items.length;
    let i = from ?? (step > 0 ? -1 : n);
    for (let k = 0; k < n; k++) {
      i = (i + step + n) % n;
      if (selectable(items[i])) return i;
    }
    return null;
  }

  function onkeydown(e: KeyboardEvent) {
    if (open === null) return;
    const items = menus[open].items;
    const sub = submenu !== null ? items[submenu] : null;
    const subItems = sub?.kind === "submenu" ? sub.items : null;
    let handled = true;
    switch (e.key) {
      case "Escape":
        if (subItems) submenu = null;
        else openMenu(null);
        break;
      case "ArrowDown":
      case "ArrowUp": {
        const step = e.key === "ArrowDown" ? 1 : -1;
        if (subItems) subHighlighted = next(subItems, subHighlighted, step);
        else highlighted = next(items, highlighted, step);
        break;
      }
      case "ArrowRight": {
        const item = highlighted !== null ? items[highlighted] : null;
        if (!subItems && item?.kind === "submenu" && highlighted !== null) {
          activate(item, highlighted, false);
        } else {
          openMenu((open + 1) % menus.length);
        }
        break;
      }
      case "ArrowLeft":
        if (subItems) submenu = null;
        else openMenu((open - 1 + menus.length) % menus.length);
        break;
      case "Enter":
      case " ":
        if (subItems && subHighlighted !== null) {
          activate(subItems[subHighlighted], subHighlighted, true);
        } else if (highlighted !== null) {
          activate(items[highlighted], highlighted, false);
        }
        break;
      default:
        handled = false;
    }
    if (handled) {
      // The app's own shortcuts must not act while a menu is open.
      e.preventDefault();
      e.stopPropagation();
    }
  }

  function onpointerdown(e: PointerEvent) {
    if (open !== null && !bar.contains(e.target as Node)) openMenu(null);
  }
</script>

<svelte:window
  onkeydowncapture={onkeydown}
  onpointerdowncapture={onpointerdown}
  onblur={() => openMenu(null)}
/>

<div class="menus" role="menubar" tabindex="-1" bind:this={bar}>
  {#each menus as menu, m (menu.label)}
    <div class="menu">
      <button
        class="title"
        class:open={open === m}
        role="menuitem"
        aria-haspopup="menu"
        aria-expanded={open === m}
        onpointerdown={(e) => {
          if (e.button !== 0) return;
          openMenu(open === m ? null : m);
        }}
        onpointerenter={() => {
          if (open !== null && open !== m) openMenu(m);
        }}
      >
        {menu.label}
      </button>
      {#if open === m}
        {@render list(menu.items, false)}
      {/if}
    </div>
  {/each}
</div>

{#snippet list(items: MenuItem[], nested: boolean)}
  <ul class="dropdown" class:nested role="menu">
    {#each items as item, i (i)}
      {#if item.kind === "separator"}
        <li class="separator" role="separator"></li>
      {:else}
        {@const current = nested ? subHighlighted === i : highlighted === i}
        <li
          role={item.kind === "command" && item.checked !== undefined
            ? "menuitemradio"
            : "menuitem"}
          class="item"
          class:disabled={item.disabled}
          class:current
          aria-disabled={item.disabled}
          aria-checked={item.kind === "command" ? item.checked : undefined}
          onpointerenter={() => {
            if (nested) {
              subHighlighted = selectable(item) ? i : null;
              return;
            }
            highlighted = selectable(item) ? i : null;
            if (item.kind === "submenu" && selectable(item)) activate(item, i, false);
            else submenu = null;
          }}
          onpointerup={(e) => {
            if (e.button === 0 && item.kind === "command") activate(item, i, nested);
          }}
        >
          <span class="check">{item.kind === "command" && item.checked ? "✓" : ""}</span>
          <span class="label">{item.label}</span>
          {#if item.kind === "command" && item.shortcut}
            <span class="shortcut">{item.shortcut}</span>
          {:else if item.kind === "submenu"}
            <span class="arrow">▸</span>
          {/if}
          {#if item.kind === "submenu" && !nested && submenu === i}
            {@render list(item.items, true)}
          {/if}
        </li>
      {/if}
    {/each}
  </ul>
{/snippet}

<style>
  .menus {
    display: flex;
    align-items: stretch;
    height: 100%;
  }

  .menu {
    position: relative;
    display: flex;
  }

  .title {
    padding: 0 8px;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
  }

  .title:hover,
  .title.open {
    background: var(--hover);
  }

  .dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    z-index: 100;
    min-width: 230px;
    margin: 0;
    padding: 4px 0;
    list-style: none;
    background: var(--panel);
    border: 1px solid var(--border-strong);
    box-shadow: 0 6px 18px rgb(0 0 0 / 0.45);
  }

  .dropdown.nested {
    top: -5px;
    left: 100%;
  }

  .item {
    position: relative;
    display: flex;
    align-items: center;
    height: 24px;
    padding: 0 12px 0 0;
    white-space: nowrap;
    cursor: default;
  }

  .item.current:not(.disabled) {
    background: var(--accent);
    color: var(--accent-text, #fff);
  }

  .item.disabled {
    color: var(--text-muted);
    opacity: 0.6;
  }

  .check {
    width: 24px;
    text-align: center;
  }

  .label {
    flex: 1;
  }

  .shortcut,
  .arrow {
    margin-left: 24px;
    color: var(--text-muted);
  }

  .item.current:not(.disabled) .shortcut,
  .item.current:not(.disabled) .arrow {
    color: inherit;
  }

  .separator {
    height: 1px;
    margin: 4px 0;
    background: var(--border-strong);
  }
</style>
