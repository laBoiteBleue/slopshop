<script lang="ts">
  import { tick, untrack } from "svelte";
  import {
    BLEND_MODE_GROUPS,
    type BlendModeId,
    type DocumentView,
    type EditRequest,
    type LayerView,
    type AdjustmentId,
    type StackEntryView,
  } from "./engine";
  import ContextMenu from "./ContextMenu.svelte";
  import Icon from "./Icon.svelte";
  import type { MenuItem } from "./MenuBar.svelte";
  import LayerThumbnail from "./LayerThumbnail.svelte";
  import { t } from "./i18n/index.svelte";
  import {
    ancestors,
    canDropInto,
    carriesPaint,
    clipLineAt,
    dropTarget,
    flattenRows,
    insertionPoint,
    layerTree,
    outermost,
    slotAt,
    topmost,
  } from "./layerTree";
  import {
    blendModeChange,
    canArrange,
    clippingToggle,
    eyeClick,
    opacityEdit,
    opacityPercent,
    removal,
    typedOpacity,
    newFill,
    ungrouping,
    visibilityToggle,
    type Arrangement,
  } from "./layerEdits";
  import {
    afterLayersChange,
    allSelected,
    pressed,
    ranged,
    selectionOf,
    toggled,
    type LayerSelection,
  } from "./layerSelection";
  import { isTextField } from "./keymap";
  import {
    effectsOf,
    fillEdit,
    hasEffects,
    styleEdit,
    usesFill,
    withEffect,
    withoutEffect,
  } from "./layerStyle";
  import type { StylePage } from "./LayerStyleDialog.svelte";
  import { canDistribute } from "./align";
  import { mergeKind } from "./bake";
  import { editableEntry } from "./stackEntries";

  let {
    doc,
    onedit,
    onlive,
    ongestureend,
    onnudge,
    contextMenu = [],
    emptyContextMenu = [],
    onlayerdrag,
    hidden = [],
    onfillcolor,
    onstyle,
    onentryedit,
  }: {
    doc: DocumentView;
    /** A double-click on a fill layer's thumbnail: the app lets its color be chosen. */
    onfillcolor?: (layer: LayerView) => void;
    /** A double-click on a pixel or fill layer's row, or on one of its effects: the app opens
     * Layer Style (ADR 0032) on `page`. */
    onstyle?: (layer: LayerView, page: StylePage) => void;
    /** Edit entry `index` of `layer`'s stack again (its icon, a double-click; ADR 0034). */
    onentryedit?: (layer: LayerView, index: number) => void;
    /** The right-click menu of the layers (built by the app: the Layer menu's commands). */
    contextMenu?: MenuItem[];
    /** The right-click menu of the empty area below the layers (built by the app). */
    emptyContextMenu?: MenuItem[];
    /** Layers not listed: the preview of Image > Adjustments, while its dialog is open. */
    hidden?: number[];
    /**
     * A drag of layers in progress (where the pointer is), or its end (`null`): the app lets it
     * go on to another tab.
     */
    onlayerdrag?: (drag: { ids: number[]; pointerId: number; x: number; y: number } | null) => void;
    /** A discrete edit (one undo entry) of document `documentId`; settles once applied. */
    onedit: (documentId: number, edit: EditRequest) => Promise<void>;
    /** A live edit within a gesture (applied immediately). */
    onlive: (documentId: number, edit: EditRequest) => void;
    /**
     * End of the gesture: everything since it started becomes one undo entry. Settles once the
     * document reflects the whole gesture.
     */
    ongestureend: (documentId: number) => Promise<void>;
    /**
     * Arrows: the app may move the selected pixels instead of the layers (the Move tool with a
     * selection); whether it did.
     */
    onnudge?: (dx: number, dy: number) => boolean;
  } = $props();

  // The panel shows one document for its whole life (it is keyed by document). Capture its id:
  // props are read lazily, and an edit committed while the panel is torn down (e.g. a rename
  // blurred by a tab switch) must still go to this document, not to the newly active one.
  const documentId = untrack(() => doc.id);
  const edit = (request: EditRequest) => onedit(documentId, request);
  const live = (request: EditRequest) => onlive(documentId, request);
  const gestureEnd = () => ongestureend(documentId);

  // Panels list layers top to bottom, like every image editor. Groups (ADR 0015) show their
  // layers indented below them, unless folded (see layerTree.ts).
  /** Groups folded in the panel (UI state, like the selection). */
  let collapsed = $state<Set<number>>(new Set());
  /** Layers whose stack entries are shown below them (ADR 0029; UI state, folded at first). */
  let unfolded = $state<Set<number>>(new Set());

  function toggleEntries(id: number) {
    const next = new Set(unfolded);
    if (!next.delete(id)) next.add(id);
    unfolded = next;
  }

  /** The label of an entry: paint, or the adjustment applied (×n when applied in a row). */
  function entryLabel(entry: StackEntryView): string {
    const name =
      entry.kind === "paint" || entry.adjustment === null
        ? t("layers.entry.paint")
        : t(`adjustment.${entry.adjustment}`);
    return entry.count > 1 ? t("layers.entry.count", { name, n: entry.count }) : name;
  }

  /** Delete entry `index` (bottom to top) of `layer`'s stack. */
  function deleteEntry(layer: LayerView, index: number) {
    void edit({ kind: "deleteStackEntry", id: layer.id, index });
  }

  /** Hide or show entry `index` of `layer`'s stack by its eye (ADR 0034). */
  function toggleEntry(layer: LayerView, index: number) {
    const entry = layer.entries[index];
    if (entry) void edit({ kind: "setStackEntry", id: layer.id, index, hidden: !entry.hidden });
  }

  /** The right-click menu of an entry, where it is open. */
  let entryMenu = $state<{ x: number; y: number; layer: LayerView; index: number } | null>(null);

  let rows = $derived(flattenRows(doc.layers, collapsed, hidden));
  let tree = $derived(layerTree(doc.layers));
  let allLayers = $derived(tree.all);

  function toggleFold(id: number) {
    const next = new Set(collapsed);
    if (!next.delete(id)) next.add(id);
    collapsed = next;
  }

  // The selection's rules are in layerSelection.ts.
  let selectedIds = $state<number[]>([]);
  let activeId = $state<number | null>(null);
  let anchorId: number | null = null;
  let selectedSet = $derived(new Set(selectedIds));
  let selected = $derived(allLayers.find((l) => l.id === activeId) ?? null);
  /** Selected layers, depth first, bottom to top. */
  let selection = $derived(allLayers.filter((l) => selectedSet.has(l.id)));
  let knownIds = new Set<number>();

  function current(): LayerSelection {
    return { ids: selectedIds, active: activeId, anchor: anchorId };
  }

  function apply(next: LayerSelection) {
    selectedIds = next.ids;
    activeId = next.active;
    anchorId = next.anchor;
  }

  function select(ids: number[], active: number | null) {
    apply(selectionOf(ids, active));
  }

  /** Layers not shown in the list (the previews of Image > Adjustments): never selected. */
  let hiddenSet = $derived(new Set(hidden));

  $effect(() => {
    const order = allLayers.map((l) => l.id).filter((id) => !hiddenSet.has(id));
    const known = knownIds;
    knownIds = new Set(order);
    untrack(() => apply(afterLayersChange(current(), known, order)));
  });

  function toggleSelected(id: number) {
    apply(
      toggled(
        current(),
        id,
        allLayers.map((l) => l.id),
      ),
    );
  }

  function selectRange(id: number) {
    apply(
      ranged(
        current(),
        id,
        rows.map((r) => r.layer.id),
      ),
    );
  }

  let list: HTMLUListElement;

  /**
   * A solid color fill layer of `color` (`#rrggbb`: the foreground color) above the active
   * layer, selected (Layer > New Fill Layer > Solid Color): no dialog, its color stays editable
   * (Properties panel, double-click on its thumbnail).
   */
  export function addFill(color: string) {
    const n = allLayers.filter((l) => l.kind === "fill").length + 1;
    const name = t("layers.defaultFillName", { n });
    const before = new Set(allLayers.map((l) => l.id));
    void edit(newFill(tree, selected?.id ?? null, color, name)).then(() => {
      const added = allLayers.find((l) => !before.has(l.id));
      if (added) select([added.id], added.id);
    });
  }

  // Commands of the Layer and Select menus.

  /** The active layer, if any. */
  export function selectedLayer(): LayerView | null {
    return selected;
  }

  /**
   * Layers whose mask, rather than their pixels, is what painting reaches (UI state, as in
   * Photoshop: a click on a thumbnail chooses, a frame shows it on the active layer).
   */
  let maskTargets = $state<Set<number>>(new Set());

  function targetMask(id: number, mask: boolean) {
    if (maskTargets.has(id) === mask) return;
    const next = new Set(maskTargets);
    if (mask) next.add(id);
    else next.delete(id);
    maskTargets = next;
  }

  /** New masks become the target of painting, as in Photoshop. */
  export function targetMasks(ids: number[]) {
    maskTargets = new Set([...maskTargets, ...ids]);
  }

  /** Painting reaches the active layer's mask rather than its pixels. */
  export function paintsMask(): boolean {
    return selected?.mask != null && maskTargets.has(selected.id);
  }

  /** Every selected layer, bottom to top. */
  export function selectedLayers(): LayerView[] {
    return selection;
  }

  /** Renaming or dragging a layer: the layer keys (Delete, F2) wait. */
  export function busy(): boolean {
    return renaming !== null || drag?.active === true;
  }

  export function renameSelected() {
    if (selected) renaming = selected.id;
  }

  /** Send `request`, unless there is nothing to do. */
  function send(request: EditRequest | null) {
    if (request) void edit(request);
  }

  export function deleteSelected() {
    send(removal(tree, selection));
  }

  /**
   * Clip the selected layers to the layers below them, or release them when all are clipped
   * (Layer > Create/Release Clipping Mask, Alt+Ctrl+G).
   */
  export function toggleClippingSelected() {
    send(clippingToggle(selection));
  }

  const ARROWS: Record<string, [number, number]> = {
    ArrowLeft: [-1, 0],
    ArrowRight: [1, 0],
    ArrowUp: [0, -1],
    ArrowDown: [0, 1],
  };

  /** Move the selected layers by whole document pixels (one undo entry). */
  export function moveSelected(dx: number, dy: number) {
    if (selection.length === 0 || (dx === 0 && dy === 0)) return;
    void edit({ kind: "translateLayers", ids: selectedIds, dx, dy });
  }

  /** Copies of the selected layers, each above its original (Layer > Duplicate Layer, Ctrl+J). */
  export function duplicateSelected() {
    if (selection.length === 0) return;
    const nameFormat = t("layers.copyName", { name: "{name}" });
    void edit({ kind: "duplicateLayers", ids: selectedIds, nameFormat });
  }

  /** Hide the selected layers, or show them all when the active one is hidden. */
  export function toggleSelectedVisibility() {
    send(visibilityToggle(selection, selected));
  }

  // The right-click menu: where it is open, if it is, and whether it is the empty area's.
  let menuAt = $state<{ x: number; y: number; empty: boolean } | null>(null);
  const menuItems = $derived(menuAt?.empty ? emptyContextMenu : contextMenu);

  function onRowContextMenu(e: MouseEvent, layer: LayerView) {
    e.preventDefault();
    if (renaming !== null) return;
    // As in Photoshop: a layer outside the selection becomes the selection.
    if (!selectedSet.has(layer.id)) select([layer.id], layer.id);
    menuAt = { x: e.clientX, y: e.clientY, empty: false };
  }

  /** A right-click in the empty area: its own menu (new layers, paste), layers deselected. */
  function onEmptyContextMenu(e: MouseEvent) {
    const empty = e.target instanceof HTMLElement && e.target.classList.contains("empty");
    if (e.target !== e.currentTarget && !empty) return;
    e.preventDefault();
    if (renaming !== null) return;
    deselectLayers();
    menuAt = { x: e.clientX, y: e.clientY, empty: true };
  }

  /** The name of the next new pixel layer: "Layer N", as Photoshop counts them. */
  export function nextLayerName(): string {
    const n = allLayers.filter((l) => l.kind === "raster").length + 1;
    return t("layers.defaultLayerName", { n });
  }

  /**
   * A new empty layer to paint on, above the active layer (in its group) or at the top,
   * selected (Layer > New > Layer, Shift+Ctrl+N, ADR 0027).
   */
  export function newLayer() {
    const name = nextLayerName();
    const { parent, index } = insertionPoint(tree, selected?.id ?? null);
    const before = new Set(allLayers.map((l) => l.id));
    void edit({ kind: "addEmptyLayer", name, parent, index }).then(() => {
      const added = allLayers.find((l) => !before.has(l.id));
      if (added) select([added.id], added.id);
    });
  }

  /** The selected layers, or layers inside them, carry paint (Layer > Delete Paint). */
  export function selectionPainted(): boolean {
    return carriesPaint(selection);
  }

  /** Layer > Delete Paint: the selected layers' originals (pixels and masks) show again. */
  export function deletePaintSelected() {
    if (!selectionPainted()) return;
    void edit({ kind: "deletePaint", ids: selection.map((l) => l.id) });
  }

  /** A new empty group above the active layer, or at the top. */
  export function newGroup() {
    const n = allLayers.filter((l) => l.kind === "group").length + 1;
    const name = t("layers.defaultGroupName", { n });
    const { parent, index } = insertionPoint(tree, selected?.id ?? null);
    void edit({ kind: "addGroup", name, parent, index });
  }

  /**
   * A new adjustment layer above the active layer (in its group), or at the top, selected so
   * that the Properties panel shows it (Layer > New Adjustment Layer, ADR 0020).
   */
  export function addAdjustment(adjustment: AdjustmentId) {
    const label = t(`adjustment.${adjustment}`);
    const n = allLayers.filter((l) => l.adjustment?.id === adjustment).length + 1;
    const { parent, index } = insertionPoint(tree, selected?.id ?? null);
    const before = new Set(allLayers.map((l) => l.id));
    const name = t("layers.defaultAdjustmentName", { name: label, n });
    void edit({ kind: "addAdjustmentLayer", name, adjustment, parent, index }).then(() => {
      const added = allLayers.find((l) => !before.has(l.id));
      if (added) select([added.id], added.id);
    });
  }

  /** Put the selected layers into a new group (Layer > Group Layers, Ctrl+G). */
  export function groupSelected() {
    if (selection.length === 0) return;
    const n = allLayers.filter((l) => l.kind === "group").length + 1;
    void edit({ kind: "groupLayers", ids: selectedIds, name: t("layers.defaultGroupName", { n }) });
  }

  /** Replace the selected groups by their layers, which become the selection (Shift+Ctrl+G). */
  export function ungroupSelected() {
    const ungrouped = ungrouping(selection);
    if (!ungrouped) return;
    void edit(ungrouped.request).then(() => {
      if (ungrouped.layers.length > 0) selectLayers(ungrouped.layers);
    });
  }

  /** Some selected layer is a group (Layer > Ungroup Layers). */
  export function selectionHasGroup(): boolean {
    return selection.some((l) => l.kind === "group");
  }

  /** Layer > Arrange: the selected layers moved within their groups (one undo entry). */
  export function arrangeSelected(arrangement: Arrangement) {
    if (!canArrange(tree, selection, arrangement)) return;
    void edit({ kind: "arrangeLayers", ids: selectedIds, arrange: arrangement });
  }

  /** What Ctrl+E does with the selected layers: merge them, merge down, or nothing. */
  export function mergeKindSelected(): "layers" | "down" | null {
    return mergeKind(tree, selection);
  }

  /** Layer > Distribute applies: three selected layers, a group counting as one. */
  export function canDistributeSelected(): boolean {
    return canDistribute(tree, selection);
  }

  /** Whether Layer > Arrange's `arrangement` moves some selected layer. */
  export function canArrangeSelected(arrangement: Arrangement): boolean {
    return canArrange(tree, selection, arrangement);
  }

  export function selectAllLayers() {
    apply(
      allSelected(
        current(),
        allLayers.map((l) => l.id),
      ),
    );
  }

  export function deselectLayers() {
    select([], null);
  }

  /** Select `ids`, the topmost active (layers just placed). */
  export function selectLayers(ids: number[]) {
    select(ids, topmost(tree, ids));
  }

  /** Select one layer (e.g. picked on the image), unfolding the groups around it. */
  export function selectOnly(id: number) {
    const next = new Set(collapsed);
    for (const at of ancestors(tree, id)) next.delete(at);
    if (next.size !== collapsed.size) collapsed = next;
    select([id], id);
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

  function onWindowKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && drag?.active) {
      endDrag();
      return;
    }
    // Arrows move the selected layers by 1 pixel, 10 with Shift (the Move tool, ADR 0017).
    const arrow = ARROWS[e.key];
    if (arrow && !e.ctrlKey && !e.metaKey && !e.altKey) {
      if (
        e.target instanceof HTMLInputElement ||
        isTextField(e.target) ||
        e.target instanceof HTMLSelectElement
      )
        return;
      if (document.querySelector("dialog[open]") || renaming !== null) return;
      e.preventDefault();
      const step = e.shiftKey ? 10 : 1;
      if (onnudge?.(arrow[0] * step, arrow[1] * step)) return;
      moveSelected(arrow[0] * step, arrow[1] * step);
      return;
    }
    // Ctrl+J, Ctrl+G, Delete, F2…: commands of the app (`SHORTCUTS` in commands.ts).
  }

  // Opacity: live while dragging the slider, one undo entry per drag. `change` does not fire
  // when a drag ends on its starting value, so the gesture also ends on any pointer release.
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
    live(opacityEdit(selection, percent / 100));
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

  // Fill (ADR 0032): the selected layers' Fill Opacity (adjustment layers have none), live
  // while dragging (one undo entry per drag), as Opacity. Shown when it means something for
  // the active layer: an effect, or a Fill already set.
  let showsFill = $derived(
    selected !== null && selected.kind !== "adjustment" && usesFill(selected.style),
  );
  let fillDraft = $state<{ layerId: number; percent: number } | null>(null);
  let shownFill = $derived(
    fillDraft !== null && fillDraft.layerId === selected?.id
      ? fillDraft.percent
      : Math.round((selected?.style?.fillOpacity ?? 1) * 100),
  );

  function onFillInput(value: string) {
    if (!selected) return;
    const percent = Number(value);
    fillDraft = { layerId: selected.id, percent };
    const request = fillEdit(selection, percent / 100);
    if (request) live(request);
  }

  function onFillPointerDown() {
    const end = () => {
      window.removeEventListener("pointerup", end, true);
      window.removeEventListener("pointercancel", end, true);
      window.removeEventListener("blur", end);
      void gestureEnd().then(() => (fillDraft = null));
    };
    window.addEventListener("pointerup", end, true);
    window.addEventListener("pointercancel", end, true);
    window.addEventListener("blur", end);
  }

  function onFillFieldChange(input: HTMLInputElement) {
    const percent = typedOpacity(input.value, input.valueAsNumber);
    const request = percent === null ? null : fillEdit(selection, percent / 100);
    if (request) void edit(request);
    else input.value = String(shownFill);
  }

  // The field edits the layers that were selected when it got focus, even if the selection
  // changes before `change` fires (clicking another row commits the field on blur).
  let opacityFieldLayers: number[] | null = null;
  let opacityFieldActive: number | null = null;

  function onOpacityFieldChange(input: HTMLInputElement) {
    const ids = new Set(opacityFieldLayers ?? selectedIds);
    const targets = allLayers.filter((l) => ids.has(l.id));
    const shownId = opacityFieldLayers ? opacityFieldActive : activeId;
    const clamped = typedOpacity(input.value, input.valueAsNumber);
    // Empty or invalid: restore the displayed value instead of treating it as 0.
    if (targets.length === 0 || clamped === null) {
      input.value = String(opacityPercent(selected));
      return;
    }
    const changed = targets.filter((l) => opacityPercent(l) !== clamped);
    if (changed.length > 0) {
      const version = ++draftVersion;
      if (shownId !== null && shownId === activeId) {
        opacityDraft = { layerId: shownId, percent: clamped };
      }
      void edit(opacityEdit(changed, clamped / 100)).then(() => {
        if (version === draftVersion && !sliderHeld) opacityDraft = null;
      });
    }
    // The field always shows the active layer (written directly: Svelte skips unchanged values).
    input.value = String(shownId === activeId ? clamped : opacityPercent(selected));
  }

  /** A mode for every selected layer; "passThrough" applies to the selected groups. */
  function onBlendModeChange(value: string) {
    send(blendModeChange(selection, value as BlendModeId | "passThrough"));
  }

  // Drag to reorder, with pointer events (HTML5 drag and drop is intercepted by Tauri on
  // Windows, where the window handles file drops). The pointer is captured only once a drag
  // really starts: capturing on pointerdown would retarget click/dblclick to the row and break
  // the buttons inside it (rename, visibility). Releases are tracked on the window, so a press
  // that leaves the list before the threshold cannot leave a stale drag behind.
  const DRAG_THRESHOLD = 4;
  type Drag = {
    id: number;
    pointerId: number;
    from: number;
    startY: number;
    active: boolean;
    /** Insertion position among the displayed rows (0 = above the first row). */
    slot: number;
    /** A group row whose middle is under the pointer: the layers go into it. */
    into: number | null;
  };
  let drag = $state<Drag | null>(null);
  // A press on a layer of a multiple selection keeps the selection (to drag it all) and
  // selects that layer alone on release if no drag happened.
  let collapseOnRelease: number | null = null;

  function onRowPointerDown(e: PointerEvent, row: number, layer: LayerView) {
    drag = null;
    collapseOnRelease = null;
    if (e.button !== 0) return;
    if (renaming !== null) {
      // A press on the row being renamed belongs to its input.
      if (renaming === layer.id) return;
      // Commit first (blur runs commitRename synchronously), then handle the press normally.
      list.querySelector<HTMLInputElement>("input.rename")?.blur();
    }
    if ((e.target as HTMLElement).closest("button.eye")) return;
    list.focus({ preventScroll: true });
    // A click on the layer's thumbnail or its mask's chooses what painting reaches.
    const thumb = (e.target as HTMLElement).closest(".thumb, .mask-thumb");
    if (thumb && layer.mask && !e.shiftKey && !e.altKey) {
      targetMask(layer.id, thumb.classList.contains("mask-thumb"));
    }
    // Alt+click on the line between two layers of a level clips the upper one to the lower
    // one, or releases it (Photoshop).
    if (e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      const upper = clipLineAt(rows, row, e.clientY - r.top, r.bottom - e.clientY);
      if (upper) {
        e.preventDefault();
        void edit({ kind: "setLayerClipped", id: upper.id, clipped: !upper.clipped });
        return;
      }
    }
    if (e.shiftKey) {
      selectRange(layer.id);
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      toggleSelected(layer.id);
      return;
    }
    const press = pressed(current(), layer.id);
    apply(press.selection);
    if (press.collapse) collapseOnRelease = layer.id;
    drag = {
      id: layer.id,
      pointerId: e.pointerId,
      from: row,
      startY: e.clientY,
      active: false,
      slot: row,
      into: null,
    };
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
    const items = [...list.querySelectorAll<HTMLElement>("li[data-row]")];
    drag.slot = slotAt(
      items.map((el) => {
        const r = el.getBoundingClientRect();
        return r.top + r.height / 2;
      }),
      e.clientY,
    );
    onlayerdrag?.({
      ids: outermost(tree, movingIds(drag.id)),
      pointerId: drag.pointerId,
      x: e.clientX,
      y: e.clientY,
    });
    // The middle half of a group row: into the group (not into one being moved).
    drag.into = null;
    for (const el of items) {
      const r = el.getBoundingClientRect();
      if (e.clientY < r.top + r.height / 4 || e.clientY > r.bottom - r.height / 4) continue;
      const group = rows[Number(el.dataset.row)]?.layer;
      if (group && canDropInto(tree, group, movingIds(drag.id))) drag.into = group.id;
    }
  }

  /** End a drag of layers, for the panel and for the app. */
  function endDrag() {
    if (drag?.active) onlayerdrag?.(null);
    drag = null;
  }

  /** The layers a drag of `id` moves: the selection when `id` is part of it. */
  function movingIds(id: number): number[] {
    return selectedSet.has(id) ? selectedIds : [id];
  }

  function onWindowPointerUp(e: PointerEvent) {
    if (!drag) return;
    const { id, active, slot, into } = drag;
    endDrag();
    // Released outside the list (a tab, the image): not a move within this panel.
    const r = list.getBoundingClientRect();
    const inList =
      e.clientX >= r.left && e.clientX < r.right && e.clientY >= r.top && e.clientY < r.bottom;
    const collapse = collapseOnRelease;
    collapseOnRelease = null;
    if (!active) {
      if (collapse === id) select([id], id);
      return;
    }
    if (!inList) return;
    const ids = outermost(tree, movingIds(id));
    const target = dropTarget(tree, rows, new Set(ids), into, slot);
    // The engine leaves layers already in place alone (no undo entry when nothing moves).
    if (target) void edit({ kind: "moveLayers", ids, ...target });
  }
</script>

<svelte:window
  onkeydown={onWindowKeydown}
  onpointerup={onWindowPointerUp}
  onpointercancel={endDrag}
  onblur={endDrag}
/>

<!-- Files dropped on the panel become layers of this document (see dropTargetAt in App). -->
<section class="panel" data-drop="layer" aria-label={t("layers.title")}>
  <div class="tabs">
    <span class="tab active">{t("layers.title")}</span>
  </div>

  <div class="options">
    <select
      class="blend-mode"
      aria-label={t("layers.blendMode")}
      title={t("layers.blendMode")}
      value={selected?.kind === "group" && selected.passThrough
        ? "passThrough"
        : (selected?.blendMode ?? "normal")}
      disabled={!selected || selected.kind === "adjustment"}
      onchange={(e) => onBlendModeChange(e.currentTarget.value)}
    >
      {#if selected?.kind === "group"}
        <!-- Groups only: their layers blend through them (ADR 0015). -->
        <option value="passThrough">{t("blendMode.passThrough")}</option>
        <hr />
      {/if}
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
      onfocus={() => {
        opacityFieldLayers = [...selectedIds];
        opacityFieldActive = activeId;
      }}
      onblur={() => (opacityFieldLayers = null)}
      onchange={(e) => onOpacityFieldChange(e.currentTarget)}
    />
    <span class="unit">%</span>
  </div>
  <!-- Fill Opacity (ADR 0032): the content's opacity, its effects untouched. -->
  {#if showsFill}
    <div class="options fill">
      <label for="layer-fill">{t("layers.fill")}</label>
      <input
        class="opacity-range"
        type="range"
        min="0"
        max="100"
        value={shownFill}
        style:--fill="{shownFill}%"
        aria-label={t("layers.fill")}
        onpointerdown={onFillPointerDown}
        oninput={(e) => onFillInput(e.currentTarget.value)}
      />
      <input
        id="layer-fill"
        class="opacity-field"
        type="number"
        min="0"
        max="100"
        autocomplete="off"
        value={shownFill}
        onchange={(e) => onFillFieldChange(e.currentTarget)}
      />
      <span class="unit">%</span>
    </div>
  {/if}

  <!-- A click in the empty area below the layers deselects them, as in Photoshop. -->
  <ul
    bind:this={list}
    tabindex="-1"
    class:dragging={drag?.active}
    onpointerdown={(e) => {
      if (e.button === 0 && e.target === e.currentTarget) deselectLayers();
    }}
    oncontextmenu={onEmptyContextMenu}
  >
    {#each rows as { layer, depth, shown, clipping }, row (layer.id)}
      <li
        data-row={row}
        class:selected={selectedSet.has(layer.id)}
        class:hidden-layer={!shown}
        class:drop-into={drag?.active && drag.into === layer.id}
        class:drop-before={drag?.active && drag.into === null && drag.slot === row}
        class:drop-after={drag?.active &&
          drag.into === null &&
          row === rows.length - 1 &&
          drag.slot === rows.length}
        onpointerdown={(e) => onRowPointerDown(e, row, layer)}
        onpointermove={onRowPointerMove}
        oncontextmenu={(e) => onRowContextMenu(e, layer)}
        ondblclick={(e) => {
          if (e.defaultPrevented || layer.kind === "adjustment") return;
          const on = e.target as HTMLElement;
          if (on.closest(".name, .thumb, .mask-thumb, button, input")) return;
          onstyle?.(layer, "blending");
        }}
      >
        <button
          class="eye"
          title={t(layer.visible ? "layers.hide" : "layers.show")}
          aria-pressed={layer.visible}
          onclick={() => send(eyeClick(layer, selection))}
        >
          {#if layer.visible}<Icon name="eye" size={14} />{/if}
        </button>
        {#if depth > 0}<span class="indent" style:width="{depth * 16}px"></span>{/if}
        {#if clipping === "clipped"}
          <span class="clip-arrow" title={t("layers.clippedHint")}>
            <Icon name="clip" size={14} />
          </span>
        {/if}
        {#if layer.baking}
          <!-- Being baked (ADR 0031): what it becomes, shown small until its pixels come. -->
          <span class="thumb baking" title={t("layers.baking")}>
            <LayerThumbnail {documentId} {layer} size={36} />
          </span>
        {:else if layer.kind === "group"}
          <button
            class="fold"
            title={t(collapsed.has(layer.id) ? "layers.expand" : "layers.collapse")}
            aria-expanded={!collapsed.has(layer.id)}
            onpointerdown={(e) => e.stopPropagation()}
            onclick={() => toggleFold(layer.id)}
          >
            <Icon name={collapsed.has(layer.id) ? "chevronRight" : "chevronDown"} size={12} />
          </button>
          <span class="thumb folder"><Icon name="folder" size={26} /></span>
        {:else if layer.kind === "adjustment"}
          <span
            class="thumb folder"
            title={layer.adjustment ? t(`adjustment.${layer.adjustment.id}`) : ""}
          >
            <Icon name="adjust" size={24} />
          </span>
        {:else}
          <!-- A double-click on a fill layer's thumbnail picks its color, as in Photoshop. -->
          <span
            class="thumb"
            class:targeted={layer.mask && selected?.id === layer.id && !maskTargets.has(layer.id)}
            role="presentation"
            ondblclick={(e) => {
              if (layer.kind !== "fill") return;
              e.stopPropagation();
              onfillcolor?.(layer);
            }}
          >
            <LayerThumbnail {documentId} {layer} size={36} />
          </span>
        {/if}
        {#if layer.mask}
          <!-- Shift+click toggles the mask, as in Photoshop. -->
          <button
            class="mask-thumb"
            class:disabled={!layer.mask.enabled}
            class:targeted={selected?.id === layer.id && maskTargets.has(layer.id)}
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
            class:clip-base={clipping === "base"}
            tabindex="-1"
            title={t("layers.renameHint")}
            ondblclick={() => (renaming = layer.id)}
          >
            {layer.name}
          </button>
          <!-- The arrow tells a layer's pixels were painted; the mark is left for its mask. -->
          {#if layer.painted && layer.entries.length === 0}
            <span class="painted" title={t("layers.painted")}>
              <Icon name="brush" size={12} />
            </span>
          {/if}
          {#if hasEffects(layer.style)}
            <span class="fx" title={t("layers.fx")}>fx</span>
          {/if}
          {#if layer.entries.length > 0 || effectsOf(layer.style).length > 0}
            <button
              class="entries-fold"
              title={t(unfolded.has(layer.id) ? "layers.entries.hide" : "layers.entries.show")}
              aria-expanded={unfolded.has(layer.id)}
              onpointerdown={(e) => e.stopPropagation()}
              onclick={() => toggleEntries(layer.id)}
            >
              <Icon name={unfolded.has(layer.id) ? "chevronDown" : "chevronRight"} size={12} />
            </button>
          {/if}
        {/if}
      </li>
      {#if unfolded.has(layer.id) && effectsOf(layer.style).length > 0}
        <!-- Its effects (ADR 0032), each with its eye, as Photoshop lists them. -->
        <li class="entry effects-title" class:hidden-layer={!shown}>
          <span class="entry-indent" style:width="{30 + depth * 16 + 24}px"></span>
          <span class="entry-name">{t("layers.effects")}</span>
        </li>
        {#each effectsOf(layer.style) as effect (effect.id)}
          <li
            class="entry effect"
            class:hidden-layer={!shown}
            class:off={!effect.enabled}
            ondblclick={() => onstyle?.(layer, effect.id)}
          >
            <span class="entry-indent" style:width="{30 + depth * 16 + 12}px"></span>
            <button
              class="effect-eye"
              title={t(effect.enabled ? "layers.effect.hide" : "layers.effect.show")}
              aria-label={t(effect.enabled ? "layers.effect.hide" : "layers.effect.show")}
              aria-pressed={effect.enabled}
              onclick={() =>
                void edit(
                  styleEdit(layer.id, withEffect(layer.style ?? null, effect.id, !effect.enabled)),
                )}
            >
              <Icon name="eye" size={12} />
            </button>
            <span class="entry-name">{t(`style.${effect.id}`)}</span>
            <button
              class="entry-delete"
              title={t("layers.effect.delete")}
              aria-label={t("layers.effect.delete")}
              onclick={() =>
                void edit(styleEdit(layer.id, withoutEffect(layer.style ?? null, effect.id)))}
            >
              <Icon name="trash" size={12} />
            </button>
          </li>
        {/each}
      {/if}
      {#if layer.entries.length > 0 && unfolded.has(layer.id)}
        <!-- Its stack, newest on top, as Photoshop lists smart filters (ADR 0029). -->
        {#each layer.entries.toReversed() as entry, shownAt (shownAt)}
          {@const index = layer.entries.length - 1 - shownAt}
          <li
            class="entry"
            class:hidden-layer={!shown}
            class:off={entry.hidden}
            ondblclick={() => editableEntry(entry) && onentryedit?.(layer, index)}
            oncontextmenu={(e) => {
              e.preventDefault();
              e.stopPropagation();
              entryMenu = { x: e.clientX, y: e.clientY, layer, index };
            }}
          >
            <span class="entry-indent" style:width="{30 + depth * 16 + 12}px"></span>
            <button
              class="effect-eye"
              title={t(entry.hidden ? "layers.entry.show" : "layers.entry.hide")}
              aria-label={t(entry.hidden ? "layers.entry.show" : "layers.entry.hide")}
              aria-pressed={!entry.hidden}
              onclick={() => toggleEntry(layer, index)}
            >
              <Icon name="eye" size={12} />
            </button>
            <span class="entry-icon">
              <Icon name={entry.kind === "paint" ? "brush" : "adjust"} size={12} />
            </span>
            <span class="entry-name">{entryLabel(entry)}</span>
            {#if editableEntry(entry)}
              <button
                class="entry-delete"
                title={t("layers.entry.edit")}
                aria-label={t("layers.entry.edit")}
                onclick={() => onentryedit?.(layer, index)}
              >
                <Icon name="sliders" size={12} />
              </button>
            {/if}
            <button
              class="entry-delete"
              title={t("layers.entry.deleteHint")}
              aria-label={t("layers.entry.delete")}
              onclick={() => deleteEntry(layer, index)}
            >
              <Icon name="trash" size={12} />
            </button>
          </li>
        {/each}
      {/if}
    {:else}
      <li class="empty">{t("layers.empty")}</li>
    {/each}
  </ul>

  {#if menuAt && menuItems.length > 0}
    <ContextMenu x={menuAt.x} y={menuAt.y} items={menuItems} onclose={() => (menuAt = null)} />
  {/if}
  {#if entryMenu}
    {@const { layer, index } = entryMenu}
    <ContextMenu
      x={entryMenu.x}
      y={entryMenu.y}
      items={[
        ...(layer.entries[index] && editableEntry(layer.entries[index])
          ? [
              {
                kind: "command" as const,
                label: t("layers.entry.edit"),
                run: () => onentryedit?.(layer, index),
              },
            ]
          : []),
        {
          kind: "command",
          label: t(layer.entries[index]?.hidden ? "layers.entry.show" : "layers.entry.hide"),
          run: () => toggleEntry(layer, index),
        },
        { kind: "command", label: t("layers.entry.delete"), run: () => deleteEntry(layer, index) },
      ]}
      onclose={() => (entryMenu = null)}
    />
  {/if}

  <div class="footer">
    <button class="icon-btn" title={t("layers.newGroup")} onclick={newGroup}>
      <Icon name="folderPlus" />
    </button>
    <button class="icon-btn" title={t("layers.newLayer")} onclick={newLayer}>
      <Icon name="plus" />
    </button>
    <button
      class="icon-btn"
      title={t(selection.length > 1 ? "layers.deleteSelected" : "layers.delete")}
      disabled={selection.length === 0}
      onclick={deleteSelected}
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

  /* Fill sits under Opacity, aligned with it. */
  .options.fill {
    justify-content: flex-end;
    padding-top: 0;
  }

  .fx {
    margin-left: 4px;
    font-style: italic;
    font-weight: 600;
    font-size: 11px;
    color: var(--text-muted);
  }

  .effects-title .entry-name {
    color: var(--text-muted);
  }

  .effect-eye {
    display: grid;
    place-items: center;
    width: 18px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-muted);
  }

  .entry.off .entry-name,
  .entry.off .entry-icon,
  .entry.off .effect-eye {
    opacity: 0.4;
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

  li.drop-into {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .indent {
    flex: none;
  }

  .clip-arrow {
    display: grid;
    place-items: center;
    width: 16px;
    margin-left: 4px;
    color: var(--text-muted);
    flex: none;
  }

  .name.clip-base {
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .fold {
    display: grid;
    place-items: center;
    width: 16px;
    height: 100%;
    margin-left: 2px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-muted);
    flex: none;
  }

  .fold:hover {
    color: var(--text);
  }

  .thumb.folder {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    margin-left: 2px;
    color: var(--text-muted);
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

  /* What painting reaches on the active layer, when it has a mask: its pixels or its mask. */
  .thumb.targeted,
  .mask-thumb.targeted {
    outline: 2px solid var(--text);
    outline-offset: 1px;
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

  /* A layer's stack (ADR 0029): an arrow unfolds its entries below it. */
  .entries-fold {
    display: grid;
    place-items: center;
    flex: none;
    width: 16px;
    height: 100%;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-muted);
  }

  .entries-fold:hover {
    color: var(--text);
  }

  li.entry {
    height: 24px;
    gap: 6px;
    font-size: 0.92em;
    color: var(--text-muted);
  }

  .entry-indent {
    flex: none;
  }

  .entry-icon {
    display: grid;
    place-items: center;
    flex: none;
  }

  .entry-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .entry-delete {
    display: grid;
    place-items: center;
    flex: none;
    padding: 2px;
    border: 0;
    background: none;
    color: var(--text-muted);
    visibility: hidden;
  }

  li.entry:hover .entry-delete {
    visibility: visible;
  }

  .entry-delete:hover {
    color: var(--text);
  }

  /* Paint on the layer (ADR 0027): its original is kept, Delete Paint brings it back. */
  .painted {
    display: grid;
    place-items: center;
    flex: none;
    margin-right: 4px;
    color: var(--text-muted);
  }
</style>
