<script lang="ts">
  import { onMount } from "svelte";
  import type { MenuItem } from "./MenuBar.svelte";

  // A right-click menu at a point of the window, looking like the menu bar's menus (ADR 0013).
  // It closes on a command, Escape, a press elsewhere or when the window loses focus. Keyboard:
  // arrows move, Enter activates. Submenus are not shown here (flat menus only).
  let {
    x,
    y,
    items,
    onclose,
  }: {
    x: number;
    y: number;
    items: MenuItem[];
    onclose: () => void;
  } = $props();

  let menu: HTMLUListElement;
  let highlighted = $state<number | null>(null);
  // Where the menu shows: at the pointer, moved back inside the window if it would overflow.
  let left = $state(0);
  let top = $state(0);

  onMount(() => {
    const { width, height } = menu.getBoundingClientRect();
    left = Math.max(0, Math.min(x, window.innerWidth - width));
    top = Math.max(0, Math.min(y, window.innerHeight - height));
    menu.focus({ preventScroll: true });
  });

  const selectable = (item: MenuItem) => item.kind === "command" && !item.disabled;

  function activate(item: MenuItem) {
    if (item.kind !== "command" || item.disabled) return;
    onclose();
    item.run();
  }

  function next(from: number | null, step: number): number | null {
    const n = items.length;
    let i = from ?? (step > 0 ? -1 : n);
    for (let k = 0; k < n; k++) {
      i = (i + step + n) % n;
      if (selectable(items[i])) return i;
    }
    return null;
  }

  function onkeydown(e: KeyboardEvent) {
    let handled = true;
    switch (e.key) {
      case "Escape":
        onclose();
        break;
      case "ArrowDown":
      case "ArrowUp":
        highlighted = next(highlighted, e.key === "ArrowDown" ? 1 : -1);
        break;
      case "Enter":
      case " ":
        if (highlighted !== null) activate(items[highlighted]);
        break;
      default:
        handled = false;
    }
    if (handled) {
      // The app's own shortcuts must not act while the menu is open.
      e.preventDefault();
      e.stopPropagation();
    }
  }

  function onpointerdown(e: PointerEvent) {
    if (!menu.contains(e.target as Node)) onclose();
  }
</script>

<svelte:window
  onkeydowncapture={onkeydown}
  onpointerdowncapture={onpointerdown}
  onblur={onclose}
  onresize={onclose}
/>

<ul
  class="context-menu"
  role="menu"
  tabindex="-1"
  bind:this={menu}
  style:left="{left}px"
  style:top="{top}px"
  oncontextmenu={(e) => e.preventDefault()}
>
  {#each items as item, i (i)}
    {#if item.kind === "separator"}
      <li class="separator" role="separator"></li>
    {:else if item.kind === "command"}
      <li
        role="menuitem"
        class="item"
        class:disabled={item.disabled}
        class:current={highlighted === i}
        aria-disabled={item.disabled}
        onpointerenter={() => (highlighted = selectable(item) ? i : null)}
        onpointerup={(e) => {
          if (e.button === 0 || e.button === 2) activate(item);
        }}
      >
        <span class="check">{item.checked ? "✓" : ""}</span>
        <span class="label">{item.label}</span>
        {#if item.shortcut}<span class="shortcut">{item.shortcut}</span>{/if}
      </li>
    {/if}
  {/each}
</ul>

<style>
  .context-menu {
    position: fixed;
    z-index: 200;
    min-width: 230px;
    margin: 0;
    padding: 4px 0;
    list-style: none;
    background: var(--panel);
    border: 1px solid var(--border-strong);
    box-shadow: 0 6px 18px rgb(0 0 0 / 0.45);
  }

  .context-menu:focus {
    outline: none;
  }

  .item {
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

  .shortcut {
    margin-left: 24px;
    color: var(--text-muted);
  }

  .item.current:not(.disabled) .shortcut {
    color: inherit;
  }

  .separator {
    height: 1px;
    margin: 4px 0;
    background: var(--border-strong);
  }
</style>
