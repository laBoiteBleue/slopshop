<script lang="ts">
  import { hexToSrgb } from "./color";
  import { tick, untrack } from "svelte";
  import {
    BLEND_MODE_GROUPS,
    type BlendModeId,
    type DocumentView,
    type EditRequest,
    type LayerView,
    type AdjustmentId,
  } from "./engine";
  import ContextMenu from "./ContextMenu.svelte";
  import Icon from "./Icon.svelte";
  import type { MenuItem } from "./MenuBar.svelte";
  import LayerThumbnail from "./LayerThumbnail.svelte";
  import { t } from "./i18n/index.svelte";

  let {
    doc,
    onedit,
    onlive,
    ongestureend,
    onnudge,
    contextMenu = [],
    emptyContextMenu = [],
    onlayerdrag,
  }: {
    doc: DocumentView;
    /** The right-click menu of the layers (built by the app: the Layer menu's commands). */
    contextMenu?: MenuItem[];
    /** The right-click menu of the empty area below the layers (built by the app). */
    emptyContextMenu?: MenuItem[];
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
  // layers indented below them, unless folded.
  type Row = {
    layer: LayerView;
    /** Groups around the layer. */
    depth: number;
    /** Its group, null at the top level. */
    parent: number | null;
    /** Its index among its siblings (0 = bottom). */
    index: number;
    /** Visible, and so are all its groups. */
    shown: boolean;
    /** Clipped to the layer below it, or the base of the clipped layers above it (ADR 0016). */
    clipping: "clipped" | "base" | null;
  };
  /** Groups folded in the panel (UI state, like the selection). */
  let collapsed = $state<Set<number>>(new Set());

  /** Rows as displayed, top to bottom: each group above its layers, unless folded. */
  function flatten(
    layers: LayerView[],
    depth: number,
    parent: number | null,
    shown: boolean,
    out: Row[],
  ): Row[] {
    for (let index = layers.length - 1; index >= 0; index--) {
      const layer = layers[index];
      const visible = shown && layer.visible;
      // A clipped layer needs a layer below it; the base is the one the clipped layers rest on.
      const clipped = layer.clipped && index > 0;
      const base = !clipped && layers[index + 1]?.clipped === true;
      const clipping = clipped ? "clipped" : base ? "base" : null;
      out.push({ layer, depth, parent, index, shown: visible, clipping });
      if (layer.kind === "group" && !collapsed.has(layer.id)) {
        flatten(layer.children, depth + 1, layer.id, visible, out);
      }
    }
    return out;
  }
  let rows = $derived(flatten(doc.layers, 0, null, true, []));

  /** Every layer, depth first, each group before its layers, bottom to top (as the engine). */
  function walk(layers: LayerView[], out: LayerView[]): LayerView[] {
    for (const layer of layers) {
      out.push(layer);
      walk(layer.children, out);
    }
    return out;
  }
  let allLayers = $derived(walk(doc.layers, []));
  /** The group of each layer that is in one. */
  let parents = $derived.by(() => {
    const map = new Map<number, number>();
    for (const layer of allLayers) {
      for (const child of layer.children) map.set(child.id, layer.id);
    }
    return map;
  });

  /** The layers directly inside `parent` (null: the top level), bottom to top. */
  function childrenOf(parent: number | null): LayerView[] {
    if (parent === null) return doc.layers;
    return allLayers.find((l) => l.id === parent)?.children ?? [];
  }

  /** Whether `id` is `ancestor` or inside it. */
  function within(id: number, ancestor: number): boolean {
    for (let at: number | undefined = id; at !== undefined; at = parents.get(at)) {
      if (at === ancestor) return true;
    }
    return false;
  }

  /** `ids` without those inside another of them (they go with it). */
  function outermost(ids: number[]): number[] {
    const set = new Set(ids);
    return ids.filter((id) => {
      for (let at = parents.get(id); at !== undefined; at = parents.get(at)) {
        if (set.has(at)) return false;
      }
      return true;
    });
  }

  function toggleFold(id: number) {
    const next = new Set(collapsed);
    if (!next.delete(id)) next.add(id);
    collapsed = next;
  }

  // Selection is UI state, not document state (it is not undoable). Several layers can be
  // selected, as in Photoshop: click selects one, Ctrl+click adds or removes one, Shift+click
  // selects a range from the anchor. The active layer (always a selected one, when any is) is
  // the one rename, the Layer menu and the options show; actions apply to every selected layer.
  let selectedIds = $state<number[]>([]);
  let activeId = $state<number | null>(null);
  let anchorId: number | null = null;
  let selectedSet = $derived(new Set(selectedIds));
  let selected = $derived(allLayers.find((l) => l.id === activeId) ?? null);
  /** Selected layers, depth first, bottom to top. */
  let selection = $derived(allLayers.filter((l) => selectedSet.has(l.id)));
  let knownIds = new Set<number>();

  function select(ids: number[], active: number | null) {
    selectedIds = ids;
    activeId = active;
    anchorId = active;
  }

  /** The topmost of `ids` in the stack. */
  function topmost(ids: number[]): number | null {
    const set = new Set(ids);
    return allLayers.findLast((l) => set.has(l.id))?.id ?? null;
  }

  $effect(() => {
    const ids = allLayers.map((l) => l.id);
    const created = ids.filter((id) => !knownIds.has(id));
    const first = knownIds.size === 0;
    knownIds = new Set(ids);
    untrack(() => {
      // New layers become the selection (several for a layered import).
      if (created.length > 0 && !first) {
        select(created, created[created.length - 1]);
        return;
      }
      // Deleted layers leave it; when none is left, the top layer is selected (a deliberate
      // "Deselect Layers" keeps the selection empty).
      const present = new Set(ids);
      const kept = selectedIds.filter((id) => present.has(id));
      if (first || (kept.length === 0 && selectedIds.length > 0)) {
        const top = ids.at(-1) ?? null;
        select(top === null ? [] : [top], top);
      } else if (kept.length < selectedIds.length) {
        selectedIds = kept;
        if (activeId === null || !present.has(activeId)) activeId = topmost(kept);
        if (anchorId === null || !present.has(anchorId)) anchorId = activeId;
      }
    });
  });

  /** One edit, or one batch (a single undo entry) for several. */
  function batchOf(edits: EditRequest[]): EditRequest {
    return edits.length === 1 ? edits[0] : { kind: "batch", edits };
  }

  function toggleSelected(id: number) {
    if (selectedSet.has(id)) {
      selectedIds = selectedIds.filter((s) => s !== id);
      if (activeId === id) activeId = topmost(selectedIds);
      anchorId = activeId;
    } else {
      selectedIds = [...selectedIds, id];
      activeId = id;
      anchorId = id;
    }
  }

  /** Select the rows from the anchor to `id` (inclusive); the anchor stays. */
  function selectRange(id: number) {
    const displayed = rows.map((r) => r.layer.id);
    const from = displayed.indexOf(anchorId ?? id);
    const to = displayed.indexOf(id);
    if (from < 0 || to < 0) return select([id], id);
    selectedIds = displayed.slice(Math.min(from, to), Math.max(from, to) + 1);
    activeId = id;
  }

  let newColor = $state("#ffffff");
  let list: HTMLUListElement;

  /** `#rrggbb` → sRGB-encoded RGBA in [0, 1]. The engine converts it to its working space. */
  export function addFill() {
    const name = t("layers.defaultFillName", { n: doc.layers.length + 1 });
    edit({ kind: "addFillLayer", name, color: [...hexToSrgb(newColor), 1] });
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

  export function deleteSelected() {
    if (selection.length === 0) return;
    // A group takes its layers with it.
    const ids = outermost(selection.map((l) => l.id));
    void edit(batchOf(ids.map((id) => ({ kind: "removeLayer", id }))));
  }

  /**
   * Clip the selected layers to the layers below them, or release them when all are clipped
   * (Layer > Create/Release Clipping Mask, Alt+Ctrl+G).
   */
  export function toggleClippingSelected() {
    if (selection.length === 0) return;
    const release = selection.every((l) => l.clipped);
    const edits = selection
      .filter((l) => l.clipped === release)
      .map((l): EditRequest => ({ kind: "setLayerClipped", id: l.id, clipped: !release }));
    if (edits.length > 0) void edit(batchOf(edits));
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
    if (selection.length === 0) return;
    const visible = selected?.visible === false;
    const edits = selection
      .filter((l) => l.visible !== visible)
      .map((l): EditRequest => ({ kind: "setLayerVisible", id: l.id, visible }));
    if (edits.length > 0) void edit(batchOf(edits));
  }

  /**
   * The eye of `layer`: within a selection of several layers, shows or hides them all (as the
   * clicked one becomes, one undo entry); otherwise toggles that layer alone.
   */
  function toggleEye(layer: LayerView) {
    const visible = !layer.visible;
    if (selection.length > 1 && selectedSet.has(layer.id)) {
      const edits = selection
        .filter((l) => l.visible !== visible)
        .map((l): EditRequest => ({ kind: "setLayerVisible", id: l.id, visible }));
      if (edits.length > 0) void edit(batchOf(edits));
      return;
    }
    void edit({ kind: "setLayerVisible", id: layer.id, visible });
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

  /**
   * A new empty layer to paint on, above the active layer (in its group) or at the top,
   * selected (Layer > New > Layer, Shift+Ctrl+N, ADR 0027).
   */
  export function newLayer() {
    const n = allLayers.filter((l) => l.kind === "raster").length + 1;
    const name = t("layers.defaultLayerName", { n });
    const parent = selected ? (parents.get(selected.id) ?? null) : null;
    const index = selected
      ? childrenOf(parent).findIndex((l) => l.id === selected?.id) + 1
      : doc.layers.length;
    const before = new Set(allLayers.map((l) => l.id));
    void edit({ kind: "addEmptyLayer", name, parent, index }).then(() => {
      const added = allLayers.find((l) => !before.has(l.id));
      if (added) select([added.id], added.id);
    });
  }

  /** The selected layers, or layers inside them, carry paint (Layer > Delete Paint). */
  export function selectionPainted(): boolean {
    const painted = (layer: LayerView): boolean => layer.painted || layer.children.some(painted);
    return selection.some(painted);
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
    const parent = selected ? (parents.get(selected.id) ?? null) : null;
    const index = selected
      ? childrenOf(parent).findIndex((l) => l.id === selected?.id) + 1
      : doc.layers.length;
    void edit({ kind: "addGroup", name, parent, index });
  }

  /**
   * A new adjustment layer above the active layer (in its group), or at the top, selected so
   * that the Properties panel shows it (Layer > New Adjustment Layer, ADR 0020).
   */
  export function addAdjustment(adjustment: AdjustmentId) {
    const label = t(`adjustment.${adjustment}`);
    const n = allLayers.filter((l) => l.adjustment?.id === adjustment).length + 1;
    const parent = selected ? (parents.get(selected.id) ?? null) : null;
    const index = selected
      ? childrenOf(parent).findIndex((l) => l.id === selected?.id) + 1
      : doc.layers.length;
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

  /** Replace the active group by its layers, which become the selection (Shift+Ctrl+G). */
  export function ungroupSelected() {
    const group = selected;
    if (group?.kind !== "group") return;
    const children = group.children.map((l) => l.id);
    void edit({ kind: "ungroup", id: group.id }).then(() => {
      if (children.length > 0) select(children, children[children.length - 1]);
    });
  }

  export function selectAllLayers() {
    const ids = allLayers.map((l) => l.id);
    selectedIds = ids;
    if (activeId === null) activeId = ids.at(-1) ?? null;
    anchorId = activeId;
  }

  export function deselectLayers() {
    select([], null);
  }

  /** Select one layer (e.g. picked on the image), unfolding the groups around it. */
  /** Select `ids`, the topmost active (layers just placed). */
  export function selectLayers(ids: number[]) {
    select(ids, topmost(ids));
  }

  export function selectOnly(id: number) {
    const next = new Set(collapsed);
    for (let at = parents.get(id); at !== undefined; at = parents.get(at)) next.delete(at);
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

  function isTextField(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLTextAreaElement ||
      target instanceof HTMLSelectElement ||
      (target instanceof HTMLInputElement && ["text", "number", "search"].includes(target.type))
    );
  }

  function onWindowKeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && drag?.active) {
      endDrag();
      return;
    }
    // Arrows move the selected layers by 1 pixel, 10 with Shift (the Move tool, ADR 0017).
    const arrow = ARROWS[e.key];
    if (arrow && !e.ctrlKey && !e.metaKey && !e.altKey) {
      if (e.target instanceof HTMLInputElement || isTextField(e.target)) return;
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

  /** Set the opacity of `layers` (one undo entry). */
  function opacityEdit(layers: LayerView[], opacity: number): EditRequest {
    return batchOf(layers.map((l) => ({ kind: "setLayerOpacity", id: l.id, opacity })));
  }

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

  // The field edits the layers that were selected when it got focus, even if the selection
  // changes before `change` fires (clicking another row commits the field on blur).
  let opacityFieldLayers: number[] | null = null;
  let opacityFieldActive: number | null = null;

  function onOpacityFieldChange(input: HTMLInputElement) {
    const ids = new Set(opacityFieldLayers ?? selectedIds);
    const targets = allLayers.filter((l) => ids.has(l.id));
    const shownId = opacityFieldLayers ? opacityFieldActive : activeId;
    const n = input.valueAsNumber;
    // Empty or invalid: restore the displayed value instead of treating it as 0.
    if (targets.length === 0 || input.value.trim() === "" || !Number.isFinite(n)) {
      input.value = String(opacityPercent(selected));
      return;
    }
    const clamped = Math.min(Math.max(Math.round(n), 0), 100);
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
    const edits: EditRequest[] = [];
    for (const layer of selection) {
      if (value === "passThrough") {
        if (layer.kind === "group" && !layer.passThrough) {
          edits.push({ kind: "setGroupPassThrough", id: layer.id, passThrough: true });
        }
        continue;
      }
      edits.push({ kind: "setLayerBlendMode", id: layer.id, mode: value as BlendModeId });
      if (layer.kind === "group" && layer.passThrough) {
        edits.push({ kind: "setGroupPassThrough", id: layer.id, passThrough: false });
      }
    }
    if (edits.length > 0) void edit(batchOf(edits));
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
      const edge = 7;
      const upper = e.clientY - r.top < edge ? row - 1 : r.bottom - e.clientY < edge ? row : null;
      const above = upper !== null ? rows[upper] : undefined;
      const below = upper !== null ? rows[upper + 1] : undefined;
      if (above && below && above.parent === below.parent && below.depth === above.depth) {
        e.preventDefault();
        void edit({
          kind: "setLayerClipped",
          id: above.layer.id,
          clipped: !above.layer.clipped,
        });
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
    if (selectedSet.has(layer.id) && selectedIds.length > 1) {
      activeId = layer.id;
      anchorId = layer.id;
      collapseOnRelease = layer.id;
    } else {
      select([layer.id], layer.id);
    }
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
    drag.slot = items.filter((el) => {
      const r = el.getBoundingClientRect();
      return r.top + r.height / 2 < e.clientY;
    }).length;
    onlayerdrag?.({
      ids: outermost(movingIds(drag.id)),
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
      const moving = movingIds(drag.id);
      if (group?.kind === "group" && !moving.some((id) => within(group.id, id))) {
        drag.into = group.id;
      }
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
    const ids = outermost(movingIds(id));
    const target = dropTarget(new Set(ids), into, slot);
    // The engine leaves layers already in place alone (no undo entry when nothing moves).
    if (target) void edit({ kind: "moveLayers", ids, ...target });
  }

  /**
   * Where a drop puts the `moving` layers: into group `into` (at its top), else just above the
   * row at `slot` (below the last row: the bottom of the stack). `index` counts the layers of
   * `parent` that stay. Null for a drop inside one of the moving groups.
   */
  function dropTarget(
    moving: Set<number>,
    into: number | null,
    slot: number,
  ): { parent: number | null; index: number } | null {
    const staying = (layers: LayerView[]) => layers.filter((l) => !moving.has(l.id));
    if (into !== null) return { parent: into, index: staying(childrenOf(into)).length };
    const row = rows[slot];
    if (!row) return { parent: null, index: 0 };
    if (row.parent !== null && [...moving].some((id) => within(row.parent as number, id))) {
      return null;
    }
    const index = staying(childrenOf(row.parent).slice(0, row.index + 1)).length;
    return { parent: row.parent, index };
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
      >
        <button
          class="eye"
          title={t(layer.visible ? "layers.hide" : "layers.show")}
          aria-pressed={layer.visible}
          onclick={() => toggleEye(layer)}
        >
          {#if layer.visible}<Icon name="eye" size={14} />{/if}
        </button>
        {#if depth > 0}<span class="indent" style:width="{depth * 16}px"></span>{/if}
        {#if clipping === "clipped"}
          <span class="clip-arrow" title={t("layers.clippedHint")}>
            <Icon name="clip" size={14} />
          </span>
        {/if}
        {#if layer.kind === "group"}
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
          <span
            class="thumb"
            class:targeted={layer.mask && selected?.id === layer.id && !maskTargets.has(layer.id)}
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
          {#if layer.painted}
            <span class="painted" title={t("layers.painted")}>
              <Icon name="brush" size={12} />
            </span>
          {/if}
        {/if}
      </li>
    {:else}
      <li class="empty">{t("layers.empty")}</li>
    {/each}
  </ul>

  {#if menuAt && menuItems.length > 0}
    <ContextMenu x={menuAt.x} y={menuAt.y} items={menuItems} onclose={() => (menuAt = null)} />
  {/if}

  <div class="footer">
    <input type="color" bind:value={newColor} title={t("layers.fillColor")} />
    <button class="icon-btn" title={t("layers.newGroup")} onclick={newGroup}>
      <Icon name="folderPlus" />
    </button>
    <button class="icon-btn" title={t("layers.addFill")} onclick={addFill}>
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

  .footer input[type="color"] {
    width: 22px;
    height: 18px;
    margin-right: auto;
    padding: 0;
    border: 1px solid var(--border-strong);
    background: none;
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
