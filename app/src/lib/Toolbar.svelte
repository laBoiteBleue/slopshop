<script lang="ts">
  // The toolbar (ADR 0013): a vertical strip on the left, one button per slot in Photoshop's
  // order. A slot holding variants (Rectangular and Elliptical Marquee) shows the one used last,
  // with a corner mark; a right-click or a long press lists them all, as in Photoshop. Below
  // them, the foreground and background colors (ADR 0027): a click on one picks it, the arrow
  // swaps them (X), the small squares bring back black and white (D).
  import Icon from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";
  import { SLOTS, toolInfo, type ToolId, type ToolSlot } from "./tools";
  import { keepFocus } from "./platform";

  let {
    tool,
    choices,
    onselect,
    colors = $bindable(),
  }: {
    tool: ToolId;
    /** The variant each slot shows, by slot key. */
    choices: Record<string, ToolId>;
    onselect: (tool: ToolId) => void;
    /** The foreground and background colors, `#rrggbb` sRGB. */
    colors: { foreground: string; background: string };
  } = $props();

  /** A press held this long opens the variants. */
  const LONG_PRESS_MS = 400;
  let open = $state<{ slot: ToolSlot; top: number } | null>(null);
  let pressTimer = 0;
  /** The long press opened the list: the click that follows must not pick the shown tool. */
  let pressOpened = false;

  function shown(slot: ToolSlot): ToolId {
    return choices[slot.key] ?? slot.tools[0].id;
  }

  function openList(slot: ToolSlot, button: HTMLElement) {
    if (slot.tools.length < 2) return;
    const box = button.getBoundingClientRect();
    open = { slot, top: box.top };
  }

  function onPointerDown(e: PointerEvent, slot: ToolSlot) {
    if (e.button !== 0) return;
    pressOpened = false;
    const button = e.currentTarget as HTMLElement;
    window.clearTimeout(pressTimer);
    pressTimer = window.setTimeout(() => {
      pressOpened = true;
      openList(slot, button);
    }, LONG_PRESS_MS);
  }

  function cancelPress() {
    window.clearTimeout(pressTimer);
  }

  function pick(id: ToolId) {
    open = null;
    onselect(id);
  }
</script>

<svelte:window
  onpointerdown={(e) => {
    if (open && !(e.target as Element).closest(".variants")) open = null;
  }}
  onkeydown={(e) => {
    if (open && e.key === "Escape") {
      e.stopPropagation();
      open = null;
    }
  }}
/>

<nav class="toolbar" aria-label={t("tools.label")}>
  {#each SLOTS as slot (slot.key)}
    {@const id = shown(slot)}
    {@const info = toolInfo(id)}
    <button
      class="tool"
      class:active={slot.tools.some((entry) => entry.id === tool)}
      class:group={slot.tools.length > 1}
      aria-pressed={slot.tools.some((entry) => entry.id === tool)}
      aria-haspopup={slot.tools.length > 1 ? "menu" : undefined}
      title={t(slot.tools.length > 1 ? "tools.variants" : "tools.tooltip", {
        name: t(info.name),
        key: slot.key,
      })}
      aria-label={t(info.name)}
      onmousedown={keepFocus}
      onpointerdown={(e) => onPointerDown(e, slot)}
      onpointerup={cancelPress}
      onpointerleave={cancelPress}
      onclick={() => {
        if (!pressOpened) pick(id);
        pressOpened = false;
      }}
      oncontextmenu={(e) => {
        e.preventDefault();
        openList(slot, e.currentTarget);
      }}
    >
      <Icon name={info.icon} size={18} />
    </button>
  {/each}

  <div class="colors">
    <!-- The system's color picker opens on a click of the swatch (a hidden input under it). -->
    <label
      class="swatch background"
      style:background={colors.background}
      title={t("tools.background")}
    >
      <input
        type="color"
        value={colors.background}
        onchange={(e) => (colors.background = e.currentTarget.value)}
      />
    </label>
    <label
      class="swatch foreground"
      style:background={colors.foreground}
      title={t("tools.foreground")}
    >
      <input
        type="color"
        value={colors.foreground}
        onchange={(e) => (colors.foreground = e.currentTarget.value)}
      />
    </label>
    <button
      class="mini swap"
      title={t("tools.swapColors")}
      aria-label={t("tools.swapColors")}
      onmousedown={keepFocus}
      onclick={() => (colors = { foreground: colors.background, background: colors.foreground })}
    >
      <Icon name="swap" size={11} />
    </button>
    <button
      class="mini default"
      title={t("tools.defaultColors")}
      aria-label={t("tools.defaultColors")}
      onmousedown={keepFocus}
      onclick={() => (colors = { foreground: "#000000", background: "#ffffff" })}
    >
      <span class="mini-swatch back"></span>
      <span class="mini-swatch front"></span>
    </button>
  </div>
</nav>

{#if open}
  <div class="variants" role="menu" style:top="{open.top}px">
    {#each open.slot.tools as entry (entry.id)}
      <button
        class="variant"
        class:active={entry.id === tool}
        role="menuitemradio"
        onmousedown={keepFocus}
        aria-checked={entry.id === tool}
        onclick={() => pick(entry.id)}
      >
        <Icon name={entry.icon} size={16} />
        <span class="name">{t(entry.name)}</span>
        <span class="key">{open.slot.key}</span>
      </button>
    {/each}
  </div>
{/if}

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
    position: relative;
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

  /* The corner mark of a slot with variants. */
  .tool.group::after {
    content: "";
    position: absolute;
    right: 2px;
    bottom: 2px;
    border-left: 4px solid transparent;
    border-bottom: 4px solid currentColor;
    opacity: 0.7;
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

  /* Photoshop's color squares: the foreground over the background, offset. */
  .colors {
    position: relative;
    width: 34px;
    height: 34px;
    margin-top: 8px;
  }

  .swatch {
    position: absolute;
    width: 18px;
    height: 18px;
    border: 1px solid var(--border-dark);
    box-shadow: 0 0 0 1px #ffffff55 inset;
    cursor: pointer;
  }

  .swatch input {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    opacity: 0;
    cursor: pointer;
  }

  .foreground {
    top: 0;
    left: 2px;
    z-index: 1;
  }

  .background {
    right: 2px;
    bottom: 2px;
  }

  .mini {
    position: absolute;
    display: grid;
    place-items: center;
    width: 12px;
    height: 12px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text-muted);
  }

  .mini:hover {
    color: var(--text);
  }

  .swap {
    top: 0;
    right: 0;
  }

  .default {
    bottom: 0;
    left: 0;
  }

  .mini-swatch {
    position: absolute;
    width: 6px;
    height: 6px;
    border: 1px solid var(--text-muted);
  }

  .mini-swatch.front {
    top: 1px;
    left: 1px;
    background: #000000;
  }

  .mini-swatch.back {
    right: 1px;
    bottom: 1px;
    background: #ffffff;
  }

  .variants {
    position: fixed;
    left: 42px;
    z-index: 20;
    display: grid;
    min-width: 220px;
    padding: 4px;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    box-shadow: 0 6px 20px #0008;
  }

  .variant {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 26px;
    padding: 0 8px;
    border: 0;
    border-radius: 3px;
    background: transparent;
    text-align: left;
  }

  .variant:hover {
    background: var(--hover);
  }

  .variant.active {
    background: var(--selected);
  }

  .variant .name {
    flex: 1;
  }

  .variant .key {
    color: var(--text-muted);
  }
</style>
