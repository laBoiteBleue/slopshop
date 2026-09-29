<script lang="ts">
  import type { DocumentView, EditRequest, LayerView } from "./engine";
  import Icon from "./Icon.svelte";
  import { t } from "./i18n/index.svelte";

  let {
    doc,
    onedit,
    onlive,
    ongestureend,
  }: {
    doc: DocumentView;
    /** A discrete edit (one undo entry). */
    onedit: (edit: EditRequest) => void;
    /** A live edit within a gesture (applied immediately). */
    onlive: (edit: EditRequest) => void;
    /** End of the gesture: everything since it started becomes one undo entry. */
    ongestureend: () => void;
  } = $props();

  // Panels list layers top to bottom, like every image editor.
  let rows = $derived([...doc.layers].reverse());

  // Selection is UI state, not document state (it is not undoable).
  let selectedId = $state<number | null>(null);
  let selected = $derived(doc.layers.find((l) => l.id === selectedId) ?? null);
  let knownIds = new Set<number>();

  $effect(() => {
    const ids = doc.layers.map((l) => l.id);
    // Select newly created layers, and fall back to the top layer if the selection vanished.
    const created = ids.filter((id) => !knownIds.has(id));
    const first = knownIds.size === 0;
    knownIds = new Set(ids);
    if (created.length > 0 && !first) selectedId = created[created.length - 1];
    else if (selectedId === null || !ids.includes(selectedId)) selectedId = ids.at(-1) ?? null;
  });

  let newColor = $state("#e84ca3");
  let renaming = $state<number | null>(null);

  function swatch(layer: LayerView): string {
    const [r, g, b] = layer.swatch.map((v) => Math.round(Math.min(Math.max(v, 0), 1) * 255));
    return `rgb(${r} ${g} ${b} / ${layer.swatch[3]})`;
  }

  /** `#rrggbb` → sRGB-encoded RGBA in [0, 1]. The engine converts it to its working space. */
  function hexToSrgb(hex: string): [number, number, number, number] {
    const n = parseInt(hex.slice(1), 16);
    return [((n >> 16) & 255) / 255, ((n >> 8) & 255) / 255, (n & 255) / 255, 1];
  }

  function addFill() {
    const name = t("layers.defaultFillName", { n: doc.layers.length + 1 });
    onedit({ kind: "addFillLayer", name, color: hexToSrgb(newColor) });
  }

  function commitRename(layer: LayerView, input: HTMLInputElement) {
    // Blur can fire after Escape or after the field is gone: only commit an active rename.
    if (renaming !== layer.id) return;
    renaming = null;
    const name = input.value.trim();
    if (name && name !== layer.name) onedit({ kind: "renameLayer", id: layer.id, name });
  }

  function focusAndSelect(node: HTMLInputElement) {
    node.focus();
    node.select();
  }

  // Opacity: live while dragging, one undo entry per drag.
  function opacityPercent(layer: LayerView | null): number {
    return layer ? Math.round(layer.opacity * 100) : 100;
  }

  function setOpacity(value: string, live: boolean) {
    const percent = Math.min(Math.max(Number(value), 0), 100);
    if (!selected || !Number.isFinite(percent)) return;
    const edit: EditRequest = { kind: "setLayerOpacity", id: selected.id, opacity: percent / 100 };
    if (live) onlive(edit);
    else onedit(edit);
  }

  // Drag to reorder, with pointer events (HTML5 drag and drop is intercepted by Tauri on
  // Windows, where the window handles file drops). The pointer is captured only once a drag
  // really starts: capturing on pointerdown would retarget click/dblclick to the row and break
  // the buttons inside it (rename, visibility).
  const DRAG_THRESHOLD = 4;
  let list: HTMLUListElement;
  type Drag = { id: number; from: number; startY: number; active: boolean; slot: number };
  let drag = $state<Drag | null>(null);

  function onRowPointerDown(e: PointerEvent, row: number, layer: LayerView) {
    if (e.button !== 0 || renaming !== null) return;
    if ((e.target as HTMLElement).closest("button.eye")) return;
    selectedId = layer.id;
    drag = { id: layer.id, from: row, startY: e.clientY, active: false, slot: row };
  }

  function onRowPointerMove(e: PointerEvent) {
    if (!drag) return;
    if (!drag.active) {
      if (Math.abs(e.clientY - drag.startY) < DRAG_THRESHOLD) return;
      drag.active = true;
      // Keep receiving moves and the final pointerup even outside the list.
      (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    }
    // Slot = insertion position among displayed rows (0 = above the first row).
    const items = [...list.querySelectorAll<HTMLElement>("li[data-row]")];
    drag.slot = items.filter((el) => {
      const r = el.getBoundingClientRect();
      return r.top + r.height / 2 < e.clientY;
    }).length;
  }

  function onRowPointerUp() {
    if (!drag) return;
    const { id, from, active, slot } = drag;
    drag = null;
    if (!active || slot === from || slot === from + 1) return;
    const finalRow = slot > from ? slot - 1 : slot;
    // Rows are displayed top to bottom; the stack index counts from the bottom.
    onedit({ kind: "moveLayer", id, index: rows.length - 1 - finalRow });
  }
</script>

<section class="panel" aria-label={t("layers.title")}>
  <div class="tabs">
    <span class="tab active">{t("layers.title")}</span>
  </div>

  <div class="options">
    <label for="layer-opacity">{t("layers.opacity")}</label>
    <input
      class="opacity-range"
      type="range"
      min="0"
      max="100"
      value={opacityPercent(selected)}
      disabled={!selected}
      aria-label={t("layers.opacity")}
      oninput={(e) => setOpacity(e.currentTarget.value, true)}
      onchange={() => ongestureend()}
    />
    <input
      id="layer-opacity"
      class="opacity-field"
      type="number"
      min="0"
      max="100"
      value={opacityPercent(selected)}
      disabled={!selected}
      onchange={(e) => setOpacity(e.currentTarget.value, false)}
    />
    <span class="unit">%</span>
  </div>

  <ul bind:this={list} class:dragging={drag?.active}>
    {#each rows as layer, row (layer.id)}
      <li
        data-row={row}
        class:selected={layer.id === selectedId}
        class:hidden-layer={!layer.visible}
        class:drop-before={drag?.active && drag.slot === row}
        class:drop-after={drag?.active && row === rows.length - 1 && drag.slot === rows.length}
        onpointerdown={(e) => onRowPointerDown(e, row, layer)}
        onpointermove={onRowPointerMove}
        onpointerup={onRowPointerUp}
        onpointercancel={() => (drag = null)}
      >
        <button
          class="eye"
          title={t(layer.visible ? "layers.hide" : "layers.show")}
          aria-pressed={layer.visible}
          onclick={() => onedit({ kind: "setLayerVisible", id: layer.id, visible: !layer.visible })}
        >
          {#if layer.visible}<Icon name="eye" size={14} />{/if}
        </button>
        <span class="thumb"><span style:background={swatch(layer)}></span></span>
        {#if renaming === layer.id}
          <input
            class="rename"
            value={layer.name}
            use:focusAndSelect
            onpointerdown={(e) => e.stopPropagation()}
            onblur={(e) => commitRename(layer, e.currentTarget)}
            onkeydown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              if (e.key === "Escape") renaming = null;
            }}
          />
        {:else}
          <button
            class="name"
            title={t("layers.renameHint")}
            ondblclick={() => (renaming = layer.id)}
            onkeydown={(e) => {
              if (e.key === "F2") renaming = layer.id;
            }}
          >
            {layer.name}
          </button>
        {/if}
      </li>
    {:else}
      <li class="empty">{t("layers.empty")}</li>
    {/each}
  </ul>

  <div class="footer">
    <input type="color" bind:value={newColor} title={t("layers.fillColor")} />
    <button class="tool" title={t("layers.addFill")} onclick={addFill}>
      <Icon name="plus" />
    </button>
    <button
      class="tool"
      title={t("layers.delete")}
      disabled={!selected}
      onclick={() => selected && onedit({ kind: "removeLayer", id: selected.id })}
    >
      <Icon name="trash" />
    </button>
  </div>
</section>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--panel);
  }

  .tabs {
    display: flex;
    height: 26px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
  }

  .tab {
    display: flex;
    align-items: center;
    padding: 0 12px;
    color: var(--text-muted);
    font-weight: 600;
  }

  .tab.active {
    background: var(--panel);
    color: var(--text);
    border-right: 1px solid var(--border-dark);
  }

  .options {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border-dark);
  }

  .options label {
    color: var(--text-muted);
  }

  .opacity-range {
    flex: 1;
    min-width: 0;
    height: 14px;
    accent-color: var(--accent);
  }

  .opacity-field {
    width: 42px;
    text-align: right;
  }

  .unit {
    color: var(--text-muted);
  }

  ul {
    flex: 1;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
  }

  ul.dragging {
    cursor: grabbing;
  }

  li {
    position: relative;
    display: flex;
    align-items: center;
    height: 34px;
    padding-right: 8px;
    border-bottom: 1px solid var(--border-dark);
  }

  li.selected {
    background: var(--selected);
  }

  li.hidden-layer .thumb,
  li.hidden-layer .name {
    opacity: 0.5;
  }

  li.drop-before::before,
  li.drop-after::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    height: 2px;
    background: var(--accent);
    z-index: 1;
  }

  li.drop-before::before {
    top: -1px;
  }

  li.drop-after::after {
    bottom: -1px;
  }

  li.empty {
    justify-content: center;
    color: var(--text-muted);
    font-style: italic;
  }

  .eye {
    display: grid;
    place-items: center;
    align-self: stretch;
    width: 30px;
    padding: 0;
    border: 0;
    border-right: 1px solid var(--border-dark);
    border-radius: 0;
    background: none;
    color: var(--text);
  }

  .eye:hover {
    background: var(--hover);
  }

  .thumb {
    width: 26px;
    height: 26px;
    margin: 0 8px;
    flex: none;
    border: 1px solid var(--border-strong);
    background: repeating-conic-gradient(#c8c8c8 0 25%, #ffffff 0 50%) 0 0 / 8px 8px;
  }

  .thumb span {
    display: block;
    width: 100%;
    height: 100%;
  }

  .name {
    flex: 1;
    min-width: 0;
    padding: 0;
    border: 0;
    background: none;
    text-align: left;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: default;
  }

  .rename {
    flex: 1;
    min-width: 0;
  }

  .footer {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    gap: 2px;
    height: 28px;
    padding: 0 6px;
    border-top: 1px solid var(--border-dark);
    background: var(--panel-header);
  }

  .footer input[type="color"] {
    width: 22px;
    height: 18px;
    margin-right: auto;
    padding: 0;
    border: 1px solid var(--border-strong);
    background: none;
  }

  .tool {
    display: grid;
    place-items: center;
    width: 24px;
    height: 22px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-muted);
  }

  .tool:hover:not(:disabled) {
    color: var(--text);
    background: var(--hover);
  }
</style>
