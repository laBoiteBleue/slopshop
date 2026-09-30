<script lang="ts">
  import { hexToSrgb } from "./color";
  import { tick, untrack } from "svelte";
  import {
    BLEND_MODE_GROUPS,
    type BlendModeId,
    type BlendSpaceId,
    type DocumentView,
    type EditRequest,
    type LayerView,
  } from "./engine";
  import Icon from "./Icon.svelte";
  import LayerThumbnail from "./LayerThumbnail.svelte";
  import { t } from "./i18n/index.svelte";

  let {
    doc,
    onedit,
    onlive,
    ongestureend,
  }: {
    doc: DocumentView;
    /** A discrete edit (one undo entry) of document `documentId`; settles once applied. */
    onedit: (documentId: number, edit: EditRequest) => Promise<void>;
    /** A live edit within a gesture (applied immediately). */
    onlive: (documentId: number, edit: EditRequest) => void;
    /**
     * End of the gesture: everything since it started becomes one undo entry. Settles once the
     * document reflects the whole gesture.
     */
    ongestureend: (documentId: number) => Promise<void>;
  } = $props();

  // The panel shows one document for its whole life (it is keyed by document). Capture its id:
  // props are read lazily, and an edit committed while the panel is torn down (e.g. a rename
  // blurred by a tab switch) must still go to this document, not to the newly active one.
  const documentId = untrack(() => doc.id);
  const edit = (request: EditRequest) => onedit(documentId, request);
  const live = (request: EditRequest) => onlive(documentId, request);
  const gestureEnd = () => ongestureend(documentId);

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
  let list: HTMLUListElement;

  /** `#rrggbb` → sRGB-encoded RGBA in [0, 1]. The engine converts it to its working space. */
  export function addFill() {
    const name = t("layers.defaultFillName", { n: doc.layers.length + 1 });
    edit({ kind: "addFillLayer", name, color: [...hexToSrgb(newColor), 1] });
  }

  // Commands of the Layer menu, on the selected layer.

  /** The selected layer, if any. */
  export function selectedLayer(): LayerView | null {
    return selected;
  }

  export function renameSelected() {
    if (selected) renaming = selected.id;
  }

  export function deleteSelected() {
    if (selected) void edit({ kind: "removeLayer", id: selected.id });
  }

  // Rename: double-click on the name, or F2 on the selected layer.
  let renaming = $state<number | null>(null);

  /** Give keyboard focus back to the list, so F2 keeps working after a rename. */
  async function focusList() {
    await tick();
    list?.focus({ preventScroll: true });
  }

  function commitRename(layer: LayerView, input: HTMLInputElement, next: EventTarget | null) {
    // Blur can fire after Escape or after the field is gone: only commit an active rename.
    if (renaming !== layer.id) return;
    renaming = null;
    const name = input.value.trim();
    if (name && name !== layer.name) edit({ kind: "renameLayer", id: layer.id, name });
    // Refocus the list only if focus is not moving to another control (click, Tab).
    if (!(next instanceof Node) || list.contains(next)) void focusList();
  }

  function cancelRename() {
    renaming = null;
    void focusList();
  }

  function focusAndSelect(node: HTMLInputElement) {
    node.focus();
    node.select();
  }

  function isTextField(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLTextAreaElement ||
      target instanceof HTMLSelectElement ||
      (target instanceof HTMLInputElement && ["text", "number", "search"].includes(target.type))
    );
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && drag?.active) {
      drag = null;
      return;
    }
    if (e.key !== "F2" || e.repeat || e.ctrlKey || e.metaKey || e.altKey) return;
    if (isTextField(e.target) || !selected || renaming !== null || drag?.active) return;
    e.preventDefault();
    renaming = selected.id;
  }

  // Opacity: live while dragging the slider, one undo entry per drag. `change` does not fire
  // when a drag ends on its starting value, so the gesture also ends on any pointer release.
  function opacityPercent(layer: LayerView | null): number {
    return layer ? Math.round(layer.opacity * 100) : 100;
  }

  // While the user moves the slider (or steps the field), the slider and the field show their
  // value, not the document's: engine answers lag behind the input, and writing them back made
  // the slider jump backwards. The draft is dropped once the last answer has arrived.
  let opacityDraft = $state<{ layerId: number; percent: number } | null>(null);
  let draftVersion = 0;
  let sliderHeld = false;
  let shownOpacity = $derived(
    opacityDraft !== null && opacityDraft.layerId === selected?.id
      ? opacityDraft.percent
      : opacityPercent(selected),
  );

  function onOpacitySliderInput(value: string) {
    if (!selected) return;
    const percent = Number(value);
    opacityDraft = { layerId: selected.id, percent };
    draftVersion++;
    live({ kind: "setLayerOpacity", id: selected.id, opacity: percent / 100 });
  }

  function endOpacityGesture() {
    const ended = draftVersion;
    void gestureEnd().then(() => {
      if (ended === draftVersion && !sliderHeld) opacityDraft = null;
    });
  }

  function onOpacitySliderPointerDown() {
    sliderHeld = true;
    const end = () => {
      window.removeEventListener("pointerup", end, true);
      window.removeEventListener("pointercancel", end, true);
      window.removeEventListener("blur", end);
      sliderHeld = false;
      endOpacityGesture();
    };
    window.addEventListener("pointerup", end, true);
    window.addEventListener("pointercancel", end, true);
    window.addEventListener("blur", end);
  }

  // The field edits the layer that was selected when it got focus, even if the selection
  // changes before `change` fires (clicking another row commits the field on blur).
  let opacityFieldLayer: number | null = null;

  function onOpacityFieldChange(input: HTMLInputElement) {
    const target = doc.layers.find((l) => l.id === (opacityFieldLayer ?? selectedId)) ?? null;
    const n = input.valueAsNumber;
    // Empty or invalid: restore the displayed value instead of treating it as 0.
    if (!target || input.value.trim() === "" || !Number.isFinite(n)) {
      input.value = String(opacityPercent(selected));
      return;
    }
    const clamped = Math.min(Math.max(Math.round(n), 0), 100);
    if (clamped !== opacityPercent(target)) {
      const version = ++draftVersion;
      if (target.id === selectedId) opacityDraft = { layerId: target.id, percent: clamped };
      void edit({ kind: "setLayerOpacity", id: target.id, opacity: clamped / 100 }).then(() => {
        if (version === draftVersion && !sliderHeld) opacityDraft = null;
      });
    }
    // The field always shows the selected layer (written directly: Svelte skips unchanged values).
    input.value = String(target.id === selectedId ? clamped : opacityPercent(selected));
  }

  // Drag to reorder, with pointer events (HTML5 drag and drop is intercepted by Tauri on
  // Windows, where the window handles file drops). The pointer is captured only once a drag
  // really starts: capturing on pointerdown would retarget click/dblclick to the row and break
  // the buttons inside it (rename, visibility). Releases are tracked on the window, so a press
  // that leaves the list before the threshold cannot leave a stale drag behind.
  const DRAG_THRESHOLD = 4;
  type Drag = { id: number; from: number; startY: number; active: boolean; slot: number };
  let drag = $state<Drag | null>(null);

  function onRowPointerDown(e: PointerEvent, row: number, layer: LayerView) {
    drag = null;
    if (e.button !== 0) return;
    if (renaming !== null) {
      // A press on the row being renamed belongs to its input.
      if (renaming === layer.id) return;
      // Commit first (blur runs commitRename synchronously), then handle the press normally.
      list.querySelector<HTMLInputElement>("input.rename")?.blur();
    }
    if ((e.target as HTMLElement).closest("button.eye")) return;
    selectedId = layer.id;
    list.focus({ preventScroll: true });
    drag = { id: layer.id, from: row, startY: e.clientY, active: false, slot: row };
  }

  function onRowPointerMove(e: PointerEvent) {
    if (!drag) return;
    // A move without the primary button means the release was missed: drop the drag.
    if ((e.buttons & 1) === 0) {
      drag = null;
      return;
    }
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

  function onWindowPointerUp() {
    if (!drag) return;
    const { id, from, active, slot } = drag;
    drag = null;
    if (!active || slot === from || slot === from + 1) return;
    const finalRow = slot > from ? slot - 1 : slot;
    // Rows are displayed top to bottom; the stack index counts from the bottom.
    edit({ kind: "moveLayer", id, index: rows.length - 1 - finalRow });
  }
</script>

<svelte:window
  onkeydown={onWindowKeydown}
  onpointerup={onWindowPointerUp}
  onpointercancel={() => (drag = null)}
  onblur={() => (drag = null)}
/>

<section class="panel" aria-label={t("layers.title")}>
  <div class="tabs">
    <span class="tab active">{t("layers.title")}</span>
  </div>

  <div class="options">
    <select
      class="blend-mode"
      aria-label={t("layers.blendMode")}
      title={t("layers.blendMode")}
      value={selected?.blendMode ?? "normal"}
      disabled={!selected}
      onchange={(e) => {
        if (selected) {
          const mode = e.currentTarget.value as BlendModeId;
          void edit({ kind: "setLayerBlendMode", id: selected.id, mode });
        }
      }}
    >
      {#each BLEND_MODE_GROUPS as group, i (i)}
        {#if i > 0}<hr />{/if}
        {#each group as mode (mode)}
          <option value={mode}>{t(`blendMode.${mode}`)}</option>
        {/each}
      {/each}
    </select>
    <label for="layer-opacity">{t("layers.opacity")}</label>
    <input
      class="opacity-range"
      type="range"
      min="0"
      max="100"
      value={shownOpacity}
      style:--fill="{shownOpacity}%"
      disabled={!selected}
      aria-label={t("layers.opacity")}
      onpointerdown={onOpacitySliderPointerDown}
      oninput={(e) => onOpacitySliderInput(e.currentTarget.value)}
      onchange={endOpacityGesture}
    />
    <input
      id="layer-opacity"
      class="opacity-field"
      type="number"
      min="0"
      max="100"
      autocomplete="off"
      value={shownOpacity}
      disabled={!selected}
      onfocus={() => (opacityFieldLayer = selectedId)}
      onblur={() => (opacityFieldLayer = null)}
      onchange={(e) => onOpacityFieldChange(e.currentTarget)}
    />
    <span class="unit">%</span>
  </div>

  <div class="options">
    <label for="blend-space" title={t("layers.blendSpace.hint")}>{t("layers.blendSpace")}</label>
    <select
      id="blend-space"
      class="blend-space"
      title={t("layers.blendSpace.hint")}
      value={doc.blendSpace}
      onchange={(e) =>
        void edit({ kind: "setBlendSpace", space: e.currentTarget.value as BlendSpaceId })}
    >
      <option value="perceptual">{t("layers.blendSpace.perceptual")}</option>
      <option value="linear">{t("layers.blendSpace.linear")}</option>
    </select>
  </div>

  <ul bind:this={list} tabindex="-1" class:dragging={drag?.active}>
    {#each rows as layer, row (layer.id)}
      <li
        data-row={row}
        class:selected={layer.id === selectedId}
        class:hidden-layer={!layer.visible}
        class:drop-before={drag?.active && drag.slot === row}
        class:drop-after={drag?.active && row === rows.length - 1 && drag.slot === rows.length}
        onpointerdown={(e) => onRowPointerDown(e, row, layer)}
        onpointermove={onRowPointerMove}
      >
        <button
          class="eye"
          title={t(layer.visible ? "layers.hide" : "layers.show")}
          aria-pressed={layer.visible}
          onclick={() => edit({ kind: "setLayerVisible", id: layer.id, visible: !layer.visible })}
        >
          {#if layer.visible}<Icon name="eye" size={14} />{/if}
        </button>
        <span class="thumb"><LayerThumbnail {documentId} {layer} size={36} /></span>
        {#if layer.mask}
          <!-- Shift+click toggles the mask, as in Photoshop. -->
          <button
            class="mask-thumb"
            class:disabled={!layer.mask.enabled}
            title={t("layers.mask.hint")}
            aria-label={t("layers.mask.hint")}
            onpointerdown={(e) => {
              if (e.shiftKey) e.stopPropagation();
            }}
            onclick={(e) => {
              if (e.shiftKey && layer.mask) {
                void edit({
                  kind: "setLayerMaskEnabled",
                  id: layer.id,
                  enabled: !layer.mask.enabled,
                });
              }
            }}
          >
            <LayerThumbnail {documentId} {layer} size={36} mask />
          </button>
        {/if}
        {#if renaming === layer.id}
          <input
            class="rename"
            value={layer.name}
            spellcheck="false"
            autocomplete="off"
            autocorrect="off"
            use:focusAndSelect
            onpointerdown={(e) => e.stopPropagation()}
            onblur={(e) => commitRename(layer, e.currentTarget, e.relatedTarget)}
            onkeydown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
              if (e.key === "Escape") cancelRename();
            }}
          />
        {:else}
          <button
            class="name"
            tabindex="-1"
            title={t("layers.renameHint")}
            ondblclick={() => (renaming = layer.id)}
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
      onclick={() => selected && edit({ kind: "removeLayer", id: selected.id })}
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

  .blend-mode {
    width: 118px;
    min-width: 0;
  }

  .blend-space {
    flex: 1;
    min-width: 0;
  }

  .opacity-range {
    width: 96px;
    min-width: 0;
  }

  .opacity-field {
    width: 36px;
    height: 18px;
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

  ul:focus {
    outline: none;
  }

  ul.dragging {
    cursor: grabbing;
  }

  li {
    position: relative;
    display: flex;
    align-items: center;
    height: 46px;
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
    display: flex;
    margin: 0 8px;
    flex: none;
  }

  .mask-thumb {
    position: relative;
    display: flex;
    margin: 0 8px 0 0;
    padding: 0;
    border: 0;
    background: none;
    flex: none;
  }

  /* A disabled mask is crossed out in red, as in Photoshop. */
  .mask-thumb.disabled::after {
    content: "";
    position: absolute;
    inset: 0;
    background:
      linear-gradient(to top right, transparent 47%, #e5322d 47% 53%, transparent 53%),
      linear-gradient(to top left, transparent 47%, #e5322d 47% 53%, transparent 53%);
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
