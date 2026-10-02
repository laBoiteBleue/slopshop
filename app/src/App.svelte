<script lang="ts">
  import { getVersion } from "@tauri-apps/api/app";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { listen } from "@tauri-apps/api/event";
  import { message, open as openDialog, save } from "@tauri-apps/plugin-dialog";
  import { onMount, untrack } from "svelte";
  import {
    DOCUMENT_CLOSED,
    DOCUMENT_EXTENSION,
    ADJUSTMENTS,
    EXPORT_FORMATS,
    engine,
    onExportEvents,
    onOpenEvents,
    type DocumentView,
    type EditRequest,
    type LayerMaskKind,
    type SelectionModify,
    type SelectionMode,
    type SelectionShape,
    type ExportFailed,
    type ExportFinished,
    type ExportFormat,
    type ExportProgress,
    type ExportSpec,
    type ExportStarted,
    type GpuInfo,
    type ImageTurn,
    type Bounds,
    type LayerView,
    type Matrix,
    type SnapTargets,
    type OpenFailed,
    type OpenFinished,
    type Opening,
    type VectorInfo,
    type SaveFailed,
    type AiComponent,
    type AiFeature,
    type AiFailure,
    type PromptPoint,
  } from "./lib/engine";
  import { getLocale, locales, setLocale, t, type Locale } from "./lib/i18n/index.svelte";
  import type { MessageKey } from "./lib/i18n/en";
  import ExportDialog from "./lib/ExportDialog.svelte";
  import VectorImportDialog from "./lib/VectorImportDialog.svelte";
  import SizeDialog from "./lib/SizeDialog.svelte";
  import PreferencesDialog from "./lib/PreferencesDialog.svelte";
  import MenuBar, { type Menu, type MenuItem } from "./lib/MenuBar.svelte";
  import { hasShortcutModifier, isWindows, modifierLabel, shortcutLetter } from "./lib/platform";
  import { formatZoom } from "./lib/format";
  import Icon from "./lib/Icon.svelte";
  import LayerThumbnail from "./lib/LayerThumbnail.svelte";
  import LayersPanel from "./lib/LayersPanel.svelte";
  import PropertiesPanel from "./lib/PropertiesPanel.svelte";
  import Viewport, { type FrameStats } from "./lib/Viewport.svelte";
  import Toolbar from "./lib/Toolbar.svelte";
  import OptionsBar from "./lib/OptionsBar.svelte";
  import { slotForLetter, slotOf, type ToolId, type ToolSlot } from "./lib/tools";
  import MarqueeTool from "./lib/MarqueeTool.svelte";
  import ModifyDialog from "./lib/ModifyDialog.svelte";
  import { MAX_FEATHER, MAX_MODIFY, stepBrush, MAX_REFINE } from "./lib/selection";
  import LassoTool from "./lib/LassoTool.svelte";
  import WandTool from "./lib/WandTool.svelte";
  import QuickSelectionTool from "./lib/QuickSelectionTool.svelte";
  import ObjectSelectionTool, { type ObjectHover } from "./lib/ObjectSelectionTool.svelte";
  import AiDownloadDialog from "./lib/AiDownloadDialog.svelte";
  import { failureMessage } from "./lib/ai";
  import ColorRangeDialog, { sampleAt, type ColorRangeState } from "./lib/ColorRangeDialog.svelte";
  import SelectionOutline from "./lib/SelectionOutline.svelte";
  import { SNAP_CSS_PX, snapMove, type Guide } from "./lib/snap";
  import FreeTransform from "./lib/FreeTransform.svelte";
  import CropBox from "./lib/CropBox.svelte";
  import * as affine from "./lib/affine";
  import ZoomSlider from "./lib/ZoomSlider.svelte";

  /** Open documents, in tab order. */
  let tabs = $state<DocumentView[]>([]);
  let activeId = $state<number | null>(null);
  let active = $derived(tabs.find((d) => d.id === activeId) ?? null);
  let ready = $state(false);
  /** Native presentation: the engine draws the canvas area under the page (ADR 0002). */
  let nativeCanvas = $state(false);

  let gpu = $state<GpuInfo | null>(null);
  let gpuError = $state<string | null>(null);
  let frame = $state<FrameStats | null>(null);
  /** Viewport of the active tab. */
  let viewport = $state<Viewport | null>(null);
  /** Layers panel of the active tab (the Layer menu acts on its selection). */
  let layersPanel = $state<LayersPanel | null>(null);
  /** The active layer when it is an adjustment layer: the Properties panel shows it. */
  let selectedAdjustment = $derived.by(() => {
    const layer = layersPanel?.selectedLayer() ?? null;
    return layer?.kind === "adjustment" ? layer : null;
  });
  /** Opens in progress (decoding a large image takes seconds). */
  let openings = $state<Opening[]>([]);
  /** Where a file being dragged over the window would go. */
  let dropTarget = $state<"tab" | "layer" | null>(null);

  let notices = $derived(active?.warnings.map((w) => t(`open.warning.${w}`)) ?? []);

  /** The file an export's options are being chosen for (after the save dialog). */
  let exportTarget = $state<{ documentId: number; path: string; format: ExportFormat } | null>(
    null,
  );
  let exportDoc = $derived(tabs.find((d) => d.id === exportTarget?.documentId) ?? null);
  /** The Save As dialog is open. */
  let choosingFile = false;
  /** The dialog opens on the last format used. */
  let lastExportFormat = $state<ExportFormat>("png");
  /** Exports running (rows done out of total). */
  let exports = $state<{ id: number; name: string; done: number; total: number }[]>([]);
  /**
   * Toasts shown above the status bar (translated), oldest first: export outcomes (a title and
   * one line per report entry) and errors. `key` is unique: `job-<id>` for an export job,
   * `local-<n>` for anything else.
   */
  type Toast = {
    key: string;
    title: string;
    lines: string[];
    kind: "done" | "notice" | "error";
    /** The written file, offered to show in its folder. */
    path?: string;
  };
  let toasts = $state<Toast[]>([]);
  /** Last `local-<n>` key given. */
  let lastLocalToast = 0;
  /** Open failures reported so far (see `openFiles`). */
  let openFailureCount = 0;

  function tabTitle(doc: DocumentView): string {
    return doc.name ?? t("document.untitled");
  }

  // --- Tabs ------------------------------------------------------------------------------------

  /**
   * Take a document view from the engine: add its tab if new, or update it unless it is stale
   * (answers can arrive out of order).
   */
  function upsert(view: DocumentView) {
    const index = tabs.findIndex((d) => d.id === view.id);
    if (index < 0) tabs.push(view);
    else if (view.revision >= tabs[index].revision) tabs[index] = view;
  }

  function activate(id: number) {
    if (id === activeId) return;
    activeId = id;
    frame = null;
  }

  async function refreshTabs() {
    tabs = await engine.documents();
    if (!tabs.some((d) => d.id === activeId)) activeId = tabs.at(-1)?.id ?? null;
  }

  /** Tabs being closed: a second close request for the same tab is ignored. */
  const closing = new Set<number>();

  async function closeTab(id: number) {
    if (closing.has(id) || !tabs.some((d) => d.id === id)) return;
    closing.add(id);
    try {
      if (!(await confirmClose(id))) return;
      await engine.closeDocument(id);
    } finally {
      closing.delete(id);
    }
    // Look the tab up again: other tabs may have been closed while waiting.
    const index = tabs.findIndex((d) => d.id === id);
    if (index < 0) return;
    tabs.splice(index, 1);
    if (activeId === id) {
      // Like browsers: the tab to the right, else the one to the left.
      activeId = (tabs[index] ?? tabs[index - 1])?.id ?? null;
      frame = null;
    }
  }

  async function newDocument() {
    const doc = await engine.newDocument();
    upsert(doc);
    activate(doc.id);
  }

  function cycleTabs(step: number) {
    if (tabs.length < 2) return;
    const index = tabs.findIndex((d) => d.id === activeId);
    activate(tabs[(index + step + tabs.length) % tabs.length].id);
  }

  // --- Edits -----------------------------------------------------------------------------------

  async function sync(request: Promise<DocumentView | null>) {
    try {
      const view = await request;
      if (view) upsert(view);
    } catch (e) {
      if (e === DOCUMENT_CLOSED) {
        // Made for a tab closed meanwhile: nothing was applied.
        await refreshTabs();
      } else {
        showError(String(e));
      }
    }
  }

  // Mutations name the document they were made for, taken when the user acts.
  // Any other edit applies a Free Transform in progress first: it is its own gesture.
  const edit = (id: number, request: EditRequest) => {
    commitTransform();
    return sync(engine.perform(id, request));
  };
  const live = (id: number, request: EditRequest) => {
    commitTransform();
    return sync(engine.performLive(id, request));
  };
  const endGesture = (id: number) => sync(engine.endGesture(id));
  const cancelGesture = (id: number) => sync(engine.cancelGesture(id));
  // Undo during a Free Transform cancels it (the transform is not applied yet).
  const undo = () => {
    if (transforming) cancelTransform();
    else if (active) void sync(engine.undo(active.id));
  };
  const redo = () => {
    commitTransform();
    if (active) void sync(engine.redo(active.id));
  };

  // --- Tab drag: reorder in the tab bar, or drop on the canvas to copy the tab's layers ---------
  //
  // Pointer events, not HTML5 drag and drop (intercepted by the window for file drops on
  // Windows). A tab is activated on release when it was not dragged, so that dragging another
  // tab onto the canvas keeps the active document on screen.

  /** Pointer travel, in CSS pixels, before a press on a tab becomes a drag. */
  const TAB_DRAG_THRESHOLD = 4;

  // Like layer reordering: the tabs stay in place while dragging, the dragged tab is dimmed and
  // an accent bar marks where it will land.
  type TabDrag = {
    id: number;
    pointerId: number;
    startX: number;
    startY: number;
    moved: boolean;
    /** Insertion position among all tabs (0 = before the first), when over the tab bar and
     * the drop would move the tab. */
    slot: number | null;
    /** Over the canvas of another document: dropping copies the dragged tab's layers there. */
    overCanvas: boolean;
  };
  let tabDrag = $state<TabDrag | null>(null);
  let tabbar: HTMLDivElement;
  let copyHint = $derived.by(() => {
    const doc = tabDrag?.overCanvas ? tabs.find((d) => d.id === tabDrag?.id) : null;
    return doc ? t("drop.copyLayers", { name: tabTitle(doc) }) : null;
  });

  function onTabPointerDown(e: PointerEvent, id: number) {
    if (e.button !== 0) return;
    tabDrag = {
      id,
      pointerId: e.pointerId,
      startX: e.clientX,
      startY: e.clientY,
      moved: false,
      slot: null,
      overCanvas: false,
    };
  }

  function onTabPointerMove(e: PointerEvent) {
    const drag = tabDrag;
    if (!drag || e.pointerId !== drag.pointerId) return;
    if (!drag.moved) {
      if (Math.hypot(e.clientX - drag.startX, e.clientY - drag.startY) < TAB_DRAG_THRESHOLD) {
        return;
      }
      drag.moved = true;
      (e.currentTarget as Element).setPointerCapture(e.pointerId);
    }
    const under = document.elementFromPoint(e.clientX, e.clientY);
    drag.overCanvas = activeId !== null && drag.id !== activeId && !!under?.closest(".stage");
    drag.slot = under?.closest(".tabbar") ? tabSlotAt(e.clientX, drag.id) : null;
  }

  /** Insertion position among all tabs for a tab dragged to `x`; `null` when dropping there
   * would leave the tab where it is (right before or after itself). */
  function tabSlotAt(x: number, draggedId: number): number | null {
    const elements = [...tabbar.querySelectorAll<HTMLElement>(".tab[data-id]")];
    const slot = elements.filter((el) => {
      const rect = el.getBoundingClientRect();
      return rect.left + rect.width / 2 < x;
    }).length;
    const from = tabs.findIndex((d) => d.id === draggedId);
    return slot === from || slot === from + 1 ? null : slot;
  }

  function onTabPointerUp(e: PointerEvent) {
    const drag = tabDrag;
    if (!drag || e.pointerId !== drag.pointerId) return;
    tabDrag = null;
    if (!drag.moved) {
      activate(drag.id);
    } else if (drag.overCanvas && activeId !== null) {
      void sync(engine.copyLayers(drag.id, activeId));
    } else if (drag.slot !== null) {
      moveTab(drag.id, drag.slot);
    }
  }

  /** Move a tab to an insertion position among all tabs (as shown by the drop bar). */
  function moveTab(id: number, slot: number) {
    const from = tabs.findIndex((d) => d.id === id);
    if (from < 0) return;
    // Position among the other tabs, as the engine counts it.
    const index = slot > from ? slot - 1 : slot;
    const [doc] = tabs.splice(from, 1);
    tabs.splice(index, 0, doc);
    void sync(engine.moveDocument(id, index).then(() => null));
  }

  function cancelTabDrag() {
    tabDrag = null;
  }

  // Layers dragged from the layers panel to another tab, as in Photoshop: hovering a tab shows
  // its document; dropping on its image or its layers panel copies the layers there. The panel
  // drives the drag until the tab changes; then the app keeps the pointer (the panel of the
  // first document goes away with it).
  const TAB_HOVER_MS = 400;
  type LayerDrag = { source: number; ids: number[]; pointerId: number };
  /** The panel's drag, while it lasts. */
  let panelDrag: LayerDrag | null = null;
  /** The drag once it left its tab, and whether the pointer is over a place to drop. */
  let layerTransfer = $state<(LayerDrag & { overTarget: boolean }) | null>(null);
  /** A tab under the dragged layers, shown after a short hover. */
  let tabHover = $state<{ id: number; timer: number } | null>(null);
  let mainElement: HTMLElement;

  // The tools (ADR 0013): the toolbar's active tool decides what a left press on the image does.
  let tool = $state<ToolId>("move");
  /** The variant each toolbar slot shows (the one used last), by slot key. */
  let toolChoices = $state<Record<string, ToolId>>({});

  function selectTool(id: ToolId) {
    // Free Transform's box and the crop frame would both take the pointer.
    if (id === "crop") commitTransform();
    tool = id;
    toolChoices[slotOf(id).key] = id;
  }

  /** A tool's key: the slot's tool used last, or with Shift the next variant (Photoshop). */
  function selectSlot(slot: ToolSlot, next: boolean) {
    const shown = toolChoices[slot.key] ?? slot.tools[0].id;
    if (!next || slot.tools.length < 2) return selectTool(shown);
    const index = slot.tools.findIndex((entry) => entry.id === shown);
    selectTool(slot.tools[(index + 1) % slot.tools.length].id);
  }

  // Selections (ADR 0024): the marquees draw shapes that the engine turns into masks.
  let selectionMode = $state<SelectionMode>("replace");
  let feather = $state(0);
  let antiAlias = $state(true);
  /** The Magic Wand's options (Photoshop's defaults). */
  let wand = $state({ tolerance: 32, contiguous: true, sampleAll: false });

  function magicWand(x: number, y: number, mode: SelectionMode | null) {
    const doc = active;
    if (!doc) return;
    commitTransform();
    // The active layer, unless every layer is sampled (or none is active).
    const layer = wand.sampleAll ? null : (layersPanel?.selectedLayer()?.id ?? null);
    void sync(
      engine.magicWand(
        doc.id,
        { x, y },
        { tolerance: wand.tolerance, contiguous: wand.contiguous, antiAlias },
        layer,
        mode ?? selectionMode,
      ),
    );
  }

  // AI selection (ADR 0025), with SAM 2.1. Object Selection: the object under the pointer
  // lights up, a click or a box selects it. Quick Selection: strokes become prompts; the strokes
  // of a session refine one object, while the document only changes by their own results.
  /**
   * The AI tools' options: Quick Selection's brush, every layer or the active one, and whether
   * edges are refined at full resolution (ViTMatte): by default for Object Selection, on demand
   * for Quick Selection (Photoshop's Enhance Edge).
   */
  let quick = $state({ size: 30, sampleAll: false, objectRefine: true, quickRefine: false });
  // On the processor (Linux), Refine Edges takes about a second per window: off by default.
  void engine.aiRuntime().then((runtime) => {
    if (runtime === "cpu") quick.objectRefine = false;
  });
  let aiBusy = $state(false);
  /** No hovering until a click asks again: the components are missing or AI cannot start. */
  let aiHoverBlocked = false;
  let quickSession: {
    id: number;
    documentId: number;
    /** The document's revision after the session's last result. */
    revision: number;
    mode: SelectionMode;
    region: [number, number, number, number];
    layer: number | null;
    points: PromptPoint[];
  } | null = null;
  let nextQuickSession = 1;
  /** The components to download before AI can run, and what to do once they are there. */
  let aiDownload = $state<{ components: AiComponent[]; then: () => void } | null>(null);
  /** Prompts per request at most: long strokes are thinned out evenly. */
  const MAX_PROMPTS = 48;

  /**
   * What the model sees, document pixels: the view when zoomed in (finer), the whole document
   * otherwise. `view` is the document area in the viewport (unclamped).
   */
  function aiRegion(
    doc: DocumentView,
    view: [number, number, number, number],
  ): [number, number, number, number] {
    const left = Math.max(0, Math.floor(view[0]));
    const top = Math.max(0, Math.floor(view[1]));
    const right = Math.min(doc.width, Math.ceil(view[0] + view[2]));
    const bottom = Math.min(doc.height, Math.ceil(view[1] + view[3]));
    const zoomedIn = (right - left) * (bottom - top) < doc.width * doc.height * 0.8;
    return zoomedIn && right > left && bottom > top
      ? [left, top, right - left, bottom - top]
      : [0, 0, doc.width, doc.height];
  }

  /** The layer the AI tools sample: the active one, unless every layer is (or none is active). */
  function aiLayer(): number | null {
    return quick.sampleAll ? null : (layersPanel?.selectedLayer()?.id ?? null);
  }

  /**
   * Runs an AI selection: on first use, asks to download what it needs, then runs it again;
   * reports other failures. Resolves to the document view, or null when nothing was applied.
   */
  async function runAi(task: () => Promise<DocumentView>): Promise<DocumentView | null> {
    aiBusy = true;
    try {
      const view = await task();
      upsert(view);
      aiHoverBlocked = false;
      return view;
    } catch (e) {
      const failure = e as Partial<AiFailure> | null;
      if (failure?.code === "notInstalled") {
        // The detail names the feature whose components are missing.
        const feature: AiFeature = failure.detail === "subject" ? "subject" : "segmentation";
        const components = await engine.aiComponents(feature);
        if (components) {
          return await new Promise((resolve) => {
            aiDownload = {
              components,
              then: () => void runAi(task).then(resolve),
            };
          });
        }
      } else {
        const message = failureMessage(e);
        if (message) showError(message);
      }
      return null;
    } finally {
      aiBusy = false;
    }
  }

  /** Object Selection's hover: the object under document point (`x`, `y`), or null. */
  async function objectHover(
    x: number,
    y: number,
    view: [number, number, number, number],
  ): Promise<ObjectHover | null> {
    const doc = active;
    if (!doc || aiHoverBlocked || aiBusy || x < 0 || y < 0 || x >= doc.width || y >= doc.height)
      return null;
    const region = aiRegion(doc, view);
    try {
      const mask = await engine.aiObjectHover(doc.id, x, y, region, aiLayer());
      return mask ? { ...mask, region } : null;
    } catch {
      // Hovering only shows; a click reports why AI cannot run (and offers the download).
      aiHoverBlocked = true;
      return null;
    }
  }

  /** Select > Subject: the main subject, refined at full resolution as Object Selection is. */
  function selectSubject() {
    const doc = active;
    if (!doc) return;
    commitTransform();
    quickSession = null;
    void runAi(() => engine.aiSelectSubject(doc.id, aiLayer(), "replace", quick.objectRefine));
  }

  /** Object Selection: the object at a point, or in a box (document pixels). */
  function objectSelect(
    point: [number, number] | null,
    box: [number, number, number, number] | null,
    keyMode: SelectionMode | null,
    view: [number, number, number, number],
  ) {
    const doc = active;
    if (!doc) return;
    commitTransform();
    quickSession = null;
    const request = {
      point,
      box,
      region: aiRegion(doc, view),
      layerId: aiLayer(),
      mode: keyMode ?? selectionMode,
      refine: quick.objectRefine,
    };
    void runAi(() => engine.aiObjectSelect(doc.id, request));
  }

  function quickStroke(
    stroke: [number, number][],
    keyMode: SelectionMode | null,
    view: [number, number, number, number],
  ) {
    const doc = active;
    if (!doc || stroke.length === 0) return;
    commitTransform();
    const continuing =
      quickSession !== null &&
      quickSession.documentId === doc.id &&
      quickSession.revision === doc.revision;
    if (!continuing) {
      const start = keyMode ?? selectionMode;
      quickSession = {
        id: nextQuickSession++,
        documentId: doc.id,
        revision: doc.revision,
        mode: start === "intersect" ? "replace" : start,
        region: aiRegion(doc, view),
        layer: aiLayer(),
        points: [],
      };
    }
    const session = quickSession;
    if (!session) return;
    // A stroke in the session's direction adds to the object; the other direction (Alt in an
    // adding session, Shift in a subtracting one) takes parts away.
    const direction = keyMode ?? (continuing ? "add" : selectionMode);
    const positive = (direction === "subtract") === (session.mode === "subtract");
    const [x0, y0, w, h] = session.region;
    for (const [x, y] of stroke) {
      if (x >= x0 && y >= y0 && x < x0 + w && y < y0 + h) session.points.push({ x, y, positive });
    }
    if (session.points.length === 0) return;
    const step = Math.max(1, session.points.length / MAX_PROMPTS);
    const points = Array.from(
      { length: Math.min(session.points.length, MAX_PROMPTS) },
      (_, i) => session.points[Math.floor(i * step)],
    );
    void runAi(() =>
      engine.aiSegment(doc.id, {
        session: session.id,
        points,
        region: session.region,
        layerId: session.layer,
        mode: session.mode,
        refine: quick.quickRefine,
      }),
    ).then((view) => {
      if (view) session.revision = view.revision;
      else if (quickSession === session) quickSession = null;
    });
  }

  function selectShape(shape: SelectionShape, mode: SelectionMode | null) {
    const doc = active;
    if (!doc) return;
    commitTransform();
    // Rectangles on whole pixels have no partial pixels to smooth.
    const smooth = shape.kind !== "rectangle" && antiAlias;
    void sync(engine.selectShape(doc.id, shape, mode ?? selectionMode, smooth, feather));
  }

  /** Select > Edit in Quick Mask Mode (Q): the view tints what the selection leaves out. */
  function toggleQuickMask() {
    const doc = active;
    if (doc) selectionCommand((id) => engine.setQuickMask(id, !doc.quickMask));
  }

  // Select > Modify: a dialog for the amount, remembered per change for the session.
  let modifyDialog = $state<{ kind: SelectionModify | "refine"; document: number } | null>(null);
  let modifyAmounts = $state<Record<SelectionModify | "refine", number>>({
    refine: 16,
    border: 10,
    smooth: 5,
    expand: 10,
    contract: 10,
    feather: 10,
  });

  function openModify(kind: SelectionModify | "refine") {
    if (active?.selectionKey != null) modifyDialog = { kind, document: active.id };
  }

  function applyModify(amount: number) {
    const dialog = modifyDialog;
    modifyDialog = null;
    if (!dialog) return;
    modifyAmounts[dialog.kind] = amount;
    commitTransform();
    const kind = dialog.kind;
    if (kind === "refine") {
      void runAi(() => engine.aiRefineSelection(dialog.document, amount, aiLayer()));
    } else {
      void sync(engine.modifySelection(dialog.document, kind, amount));
    }
  }

  // Select > Color Range: a panel beside the image, whose clicks sample colors.
  let colorRange = $state<ColorRangeState | null>(null);
  /** Edit > Preferences (Ctrl+K) is open. */
  let preferences = $state(false);

  function openColorRange() {
    const doc = active;
    if (!doc) return;
    commitTransform();
    colorRange = {
      document: doc.id,
      included: [],
      excluded: [],
      fuzziness: 40,
      invert: false,
      eyedropper: "pick",
    };
  }

  function applyColorRange() {
    const range = colorRange;
    colorRange = null;
    if (!range || (range.included.length === 0 && !range.invert)) return;
    void sync(
      engine.colorRange(range.document, {
        included: $state.snapshot(range.included),
        excluded: $state.snapshot(range.excluded),
        fuzziness: range.fuzziness,
        invert: range.invert,
        layerId: null,
      }),
    );
  }

  $effect(() => {
    if (colorRange && colorRange.document !== activeId) colorRange = null;
  });

  /** Image > Crop: to the selection's bounds when there is one, else the Crop tool. */
  function cropImage() {
    if (active?.selectionKey != null) selectionCommand(engine.cropToSelection);
    else selectTool("crop");
  }

  /** Layer > Layer Mask's new masks, on the selected layers that have none. */
  function addLayerMasks(kind: LayerMaskKind) {
    const doc = active;
    const ids = (layersPanel?.selectedLayers() ?? []).filter((l) => !l.mask).map((l) => l.id);
    if (doc && ids.length > 0) selectionCommand((id) => engine.addLayerMasks(id, ids, kind));
  }

  function selectionCommand(run: (id: number) => Promise<DocumentView>) {
    const doc = active;
    if (!doc) return;
    commitTransform();
    void sync(run(doc.id));
  }

  // The Move tool (ADR 0017): a left drag on the image moves the selected layers live, in whole
  // document pixels, one undo entry per drag. As in Photoshop: Auto-Select (the options bar)
  // takes the layer under the pointer, Ctrl inverting it; the moving layers snap to the canvas
  // and to the other layers (edges and centers, not with Ctrl), with magenta smart guides.
  let autoSelect = $state(true);
  /** View > Snap. */
  let snapping = $state(true);
  type MoveDrag = {
    document: number;
    /** Known once Auto-Select answered. */
    ids: number[] | null;
    targets: SnapTargets | null;
    /** The pointer's movement since the start, and the whole pixels sent so far. */
    raw: { x: number; y: number };
    applied: { x: number; y: number };
    docPerCss: number;
    free: boolean;
  };
  let moveDrag: MoveDrag | null = null;
  let guides = $state<Guide[]>([]);

  function onMoveStart(x: number, y: number, ctrl: boolean) {
    const doc = active;
    if (!doc) return;
    const drag: MoveDrag = {
      document: doc.id,
      ids: null,
      targets: null,
      raw: { x: 0, y: 0 },
      applied: { x: 0, y: 0 },
      docPerCss: 1,
      free: false,
    };
    moveDrag = drag;
    void (async () => {
      let ids = layersPanel?.selectedLayers().map((l) => l.id) ?? [];
      if (autoSelect !== ctrl) {
        const hit = await engine.layerAt(doc.id, Math.floor(x), Math.floor(y)).catch(() => null);
        if (hit !== null && !ids.includes(hit)) {
          layersPanel?.selectOnly(hit);
          ids = [hit];
        }
      }
      if (moveDrag !== drag) return;
      drag.ids = ids;
      if (ids.length > 0 && snapping) {
        drag.targets = await engine.moveSnapTargets(doc.id, ids).catch(() => null);
      }
      if (moveDrag === drag) flushMove(drag);
    })();
  }

  function onMoveDrag(dx: number, dy: number, docPerCss: number, free: boolean) {
    const drag = moveDrag;
    if (!drag) return;
    drag.raw = { x: drag.raw.x + dx, y: drag.raw.y + dy };
    drag.docPerCss = docPerCss;
    drag.free = free;
    flushMove(drag);
  }

  /** Send the whole pixels the drag has moved since it began, snapped (replacing the last). */
  function flushMove(drag: MoveDrag) {
    if (!drag.ids || drag.ids.length === 0) return;
    let { x, y } = drag.raw;
    let shown: Guide[] = [];
    const doc = tabs.find((d) => d.id === drag.document);
    if (drag.targets?.moving && doc && snapping && !drag.free) {
      const targets = [canvasBounds(doc), ...drag.targets.others];
      const threshold = SNAP_CSS_PX * drag.docPerCss;
      const snapped = snapMove(drag.targets.moving, x, y, targets, threshold);
      ({ x, y } = snapped);
      shown = snapped.guides;
    }
    guides = shown;
    const tx = Math.round(x);
    const ty = Math.round(y);
    if (tx === drag.applied.x && ty === drag.applied.y) return;
    drag.applied = { x: tx, y: ty };
    const move: EditRequest = { kind: "translateLayers", ids: drag.ids, dx: tx, dy: ty };
    void sync(engine.performLive(drag.document, move, true));
  }

  function canvasBounds(doc: DocumentView): Bounds {
    return { left: 0, top: 0, right: doc.width, bottom: doc.height };
  }

  function onMoveEnd() {
    const drag = moveDrag;
    moveDrag = null;
    guides = [];
    if (!drag?.ids) return;
    // Back where it started: no undo entry.
    if (drag.applied.x === 0 && drag.applied.y === 0) void cancelGesture(drag.document);
    else void endGesture(drag.document);
  }

  // Free Transform (Ctrl+T, ADR 0018): a box on the image scales, rotates and moves the selected
  // layers live, as one gesture replaced at each step; Enter applies it (one undo entry), Esc or
  // undo cancels it. Another edit, another tab or Ctrl+T again applies it first.
  type Transforming = {
    document: number;
    ids: number[];
    box: Bounds;
    /** What the box snaps to: the canvas and the other visible layers. */
    targets: Bounds[];
    matrix: Matrix;
  };
  let transforming = $state<Transforming | null>(null);

  async function startFreeTransform() {
    const doc = active;
    if (!doc || transforming) return;
    if (tool === "crop") tool = "move";
    const ids = layersPanel?.selectedLayers().map((l) => l.id) ?? [];
    if (ids.length === 0) return;
    const targets = await engine.moveSnapTargets(doc.id, ids).catch(() => null);
    // Nothing to transform (empty layers), or the user moved on meanwhile.
    if (!targets?.moving || active?.id !== doc.id || transforming) return;
    transforming = {
      document: doc.id,
      ids,
      box: targets.moving,
      targets: [canvasBounds(doc), ...targets.others],
      matrix: affine.IDENTITY,
    };
  }

  function onTransformChange(matrix: Matrix) {
    const current = transforming;
    if (!current) return;
    current.matrix = matrix;
    const request: EditRequest = { kind: "transformLayers", ids: current.ids, matrix };
    void sync(engine.performLive(current.document, request, true));
  }

  function commitTransform() {
    const current = transforming;
    if (!current) return;
    transforming = null;
    if (affine.isIdentity(current.matrix)) void cancelGesture(current.document);
    else void endGesture(current.document);
  }

  function cancelTransform() {
    const current = transforming;
    if (!current) return;
    transforming = null;
    void cancelGesture(current.document);
  }

  $effect(() => {
    if (transforming && transforming.document !== activeId) commitTransform();
  });

  /**
   * Edit > Transform's quarter turns and flips of the selected layers, about the center of their
   * bounds, placed on whole pixels so that their pixels are copied, not resampled.
   */
  async function quickTransform(by: Matrix) {
    commitTransform();
    const doc = active;
    const ids = layersPanel?.selectedLayers().map((l) => l.id) ?? [];
    if (!doc || ids.length === 0) return;
    const targets = await engine.moveSnapTargets(doc.id, ids).catch(() => null);
    const box = targets?.moving;
    if (!box) return;
    const around = affine.about(by, (box.left + box.right) / 2, (box.top + box.bottom) / 2);
    const matrix: Matrix = [...around];
    matrix[4] = Math.round(matrix[4]);
    matrix[5] = Math.round(matrix[5]);
    void edit(doc.id, { kind: "transformLayers", ids, matrix });
  }

  // Image > Image Size and Canvas Size (dialogs), and Image Rotation (ADR 0017): the whole
  // image, through the layers' transforms; nothing is cut or rewritten.
  let sizeDialog = $state<{ mode: "image" | "canvas"; document: number } | null>(null);
  let sizeDoc = $derived(sizeDialog && tabs.find((d) => d.id === sizeDialog?.document));

  function openSizeDialog(mode: "image" | "canvas") {
    if (active) sizeDialog = { mode, document: active.id };
  }

  function applySize(width: number, height: number, anchor: [number, number]) {
    const dialog = sizeDialog;
    sizeDialog = null;
    if (!dialog) return;
    void edit(
      dialog.document,
      dialog.mode === "image"
        ? { kind: "resizeImage", width, height }
        : { kind: "canvasSize", width, height, anchor },
    );
  }

  // The Crop tool (C, ADR 0017): a frame on the image while the tool is active; applying it
  // reframes the canvas, and nothing is deleted. As in Photoshop, a new frame then starts on the
  // new canvas, and Esc starts it over. What the frame snaps to is fetched when it opens.
  type Cropping = { document: number; width: number; height: number; targets: Bounds[] };
  let cropping = $state<Cropping | null>(null);
  /** Bumped by each frame requested: only the latest one opens. */
  let cropRequest = 0;
  /** A crop being applied: the next frame waits for the new canvas. */
  let cropApplying = $state(false);

  async function startCrop(doc: DocumentView) {
    const request = ++cropRequest;
    // Nothing moves: every visible layer is a target.
    const targets = await engine.moveSnapTargets(doc.id, []).catch(() => null);
    if (request !== cropRequest) return;
    cropping = {
      document: doc.id,
      width: doc.width,
      height: doc.height,
      targets: [canvasBounds(doc), ...(targets?.others ?? [])],
    };
  }

  $effect(() => {
    // The frame follows the tool, the active tab and its canvas (an undone crop, Image Size).
    const doc = active;
    const current = cropping;
    if (tool !== "crop" || !doc) {
      cropRequest++;
      if (current) cropping = null;
      return;
    }
    if (cropApplying) return;
    const stale =
      !current ||
      current.document !== doc.id ||
      current.width !== doc.width ||
      current.height !== doc.height;
    if (stale) untrack(() => void startCrop(doc));
  });

  function applyCrop(frame: Bounds) {
    const current = cropping;
    cropping = null;
    if (!current) return;
    const doc = tabs.find((d) => d.id === current.document);
    const unchanged =
      doc &&
      frame.left === 0 &&
      frame.top === 0 &&
      frame.right === doc.width &&
      frame.bottom === doc.height;
    if (unchanged) return;
    cropApplying = true;
    void edit(current.document, {
      kind: "crop",
      x: frame.left,
      y: frame.top,
      width: frame.right - frame.left,
      height: frame.bottom - frame.top,
    }).finally(() => (cropApplying = false));
  }

  function rotateImage(turn: ImageTurn) {
    if (active) void edit(active.id, { kind: "rotateImage", turn });
  }

  /** A thumbnail of the dragged layers following the pointer. */
  let dragGhost = $state<{ document: number; ids: number[]; x: number; y: number } | null>(null);

  function onLayerDrag(drag: { ids: number[]; pointerId: number; x: number; y: number } | null) {
    if (!drag || activeId === null) {
      panelDrag = null;
      if (!layerTransfer) {
        clearTabHover();
        dragGhost = null;
      }
      return;
    }
    panelDrag = { source: activeId, ids: drag.ids, pointerId: drag.pointerId };
    dragGhost = { document: activeId, ids: drag.ids, x: drag.x, y: drag.y };
    hoverTabAt(drag.x, drag.y);
  }

  /** A layer of a layer tree. */
  function findLayer(layers: LayerView[], id: number): LayerView | null {
    for (const layer of layers) {
      if (layer.id === id) return layer;
      const inside = findLayer(layer.children, id);
      if (inside) return inside;
    }
    return null;
  }

  function hoverTabAt(x: number, y: number) {
    const tab = document.elementFromPoint(x, y)?.closest<HTMLElement>(".tab[data-id]");
    const id = tab ? Number(tab.dataset.id) : null;
    if (id === null || id === activeId) return clearTabHover();
    if (tabHover?.id === id) return;
    clearTabHover();
    tabHover = { id, timer: window.setTimeout(() => showTabDuringDrag(id), TAB_HOVER_MS) };
  }

  function clearTabHover() {
    if (tabHover) window.clearTimeout(tabHover.timer);
    tabHover = null;
  }

  function showTabDuringDrag(id: number) {
    tabHover = null;
    if (!layerTransfer) {
      const drag = panelDrag;
      if (!drag) return;
      try {
        mainElement.setPointerCapture(drag.pointerId);
      } catch {
        // The pointer is already up.
        return;
      }
      layerTransfer = { ...drag, overTarget: false };
      panelDrag = null;
    }
    activate(id);
  }

  function onTransferMove(e: PointerEvent) {
    const transfer = layerTransfer;
    if (!transfer || e.pointerId !== transfer.pointerId) return;
    if (dragGhost) {
      dragGhost.x = e.clientX;
      dragGhost.y = e.clientY;
    }
    hoverTabAt(e.clientX, e.clientY);
    const under = document.elementFromPoint(e.clientX, e.clientY);
    transfer.overTarget =
      activeId !== null &&
      activeId !== transfer.source &&
      !!under?.closest(".stage, [data-drop='layer']");
  }

  function onTransferUp(e: PointerEvent) {
    const transfer = layerTransfer;
    if (!transfer || e.pointerId !== transfer.pointerId) return;
    endTransfer();
    if (transfer.overTarget && activeId !== null) {
      void sync(engine.copyLayers(transfer.source, activeId, transfer.ids));
    }
  }

  function endTransfer() {
    const transfer = layerTransfer;
    if (transfer && mainElement.hasPointerCapture(transfer.pointerId)) {
      mainElement.releasePointerCapture(transfer.pointerId);
    }
    layerTransfer = null;
    dragGhost = null;
    clearTabHover();
  }

  // Rename a tab: double-click its name. Enter or leaving the field commits, Escape cancels.
  let renamingTab = $state<number | null>(null);

  function focusAndSelect(input: HTMLInputElement) {
    input.focus();
    input.select();
  }

  function commitTabRename(doc: DocumentView, input: HTMLInputElement) {
    if (renamingTab !== doc.id) return;
    renamingTab = null;
    const name = input.value.trim();
    if (name && name !== tabTitle(doc)) void sync(engine.renameDocument(doc.id, name));
  }

  // --- Opening files ---------------------------------------------------------------------------

  /** The Import PDF / SVG dialog, while it is shown: settled with the choice, or null. */
  type VectorChoice = { pages: number[]; dpi: number };
  let vectorImport = $state<{
    path: string;
    info: VectorInfo;
    settle: (choice: VectorChoice | null) => void;
  } | null>(null);

  /** Settles when the dialogs asked before are done: files dropped meanwhile wait their turn. */
  let vectorTurn: Promise<unknown> = Promise.resolve();

  function askVectorImport(path: string, info: VectorInfo): Promise<VectorChoice | null> {
    const asked = vectorTurn.then(
      () => new Promise<VectorChoice | null>((settle) => (vectorImport = { path, info, settle })),
    );
    vectorTurn = asked;
    return asked;
  }

  function settleVectorImport(choice: VectorChoice | null) {
    const settle = vectorImport?.settle;
    vectorImport = null;
    settle?.(choice);
  }

  /**
   * Open files in new tabs, or as layers of a document, in the order of `paths`. PDFs and SVGs
   * ask which pages and at what resolution first (Import PDF / SVG), one after the other.
   * Progress and outcomes (new tabs, updated documents, failures) arrive as events.
   */
  async function openFiles(paths: string[], target: "tab" | { layerOf: number }) {
    const isVector = (path: string) => /\.(pdf|svgz?)$/i.test(path);
    const others = paths.filter((path) => !isVector(path));
    if (others.length > 0) await openPaths(others, target);
    const documentId = target === "tab" ? null : target.layerOf;
    for (const path of paths.filter(isVector)) {
      let info: VectorInfo;
      try {
        info = await engine.vectorInfo(path);
      } catch {
        // Not readable (damaged, encrypted…): the usual open reports why.
        await openPaths([path], target);
        continue;
      }
      const choice = await askVectorImport(path, info);
      if (choice === null) {
        void engine.closeVector(path);
        continue;
      }
      const failuresBefore = openFailureCount;
      try {
        await engine.openVectorPages(
          path,
          choice.pages,
          choice.dpi,
          documentId,
          t("vectorDialog.background"),
        );
      } catch (e) {
        if (openFailureCount === failuresBefore) showError(String(e));
      }
    }
  }

  async function openPaths(paths: string[], target: "tab" | { layerOf: number }) {
    const failuresBefore = openFailureCount;
    try {
      const summary = await engine.openImages(paths, target === "tab" ? null : target.layerOf);
      for (const [name, detail] of summary.failedArchives) {
        showError(t("open.failed", { name, error: t("open.error.archive", { detail }) }));
      }
      if (summary.skipped > 0) {
        showToast(null, {
          title: t("open.skipped", { count: summary.skipped }),
          lines: [],
          kind: "done",
        });
      }
    } catch (e) {
      // Failures normally come as events, with a localized message.
      if (openFailureCount === failuresBefore) showError(String(e));
    }
  }

  /** Open files in new tabs or, with `layerOf`, add them as top layers of that document. */
  /** Open a folder: its images like several files, in new tabs. */
  async function openFolderWithDialog() {
    const picked = await openDialog({
      title: t("menu.file.openFolder"),
      directory: true,
      multiple: false,
    });
    if (typeof picked === "string") await openFiles([picked], "tab");
  }

  async function openWithDialog(layerOf: number | null = null) {
    const picked = await openDialog({
      title: layerOf === null ? undefined : t("menu.file.importLayers"),
      multiple: true,
      directory: false,
    });
    const paths = picked === null ? [] : Array.isArray(picked) ? picked : [picked];
    if (paths.length > 0) await openFiles(paths, layerOf === null ? "tab" : { layerOf });
  }

  /** Opens already finished or failed: a late snapshot must not bring them back. */
  const settled = new Set<number>();

  function onOpenStarted(opening: Opening) {
    if (settled.has(opening.id) || openings.some((o) => o.id === opening.id)) return;
    openings.push(opening);
  }

  function onOpenFinished(finished: OpenFinished) {
    settled.add(finished.id);
    openings = openings.filter((o) => o.id !== finished.id);
    const isNew = !tabs.some((d) => d.id === finished.document.id);
    upsert(finished.document);
    if (isNew && finished.target.kind === "newTab") activate(finished.document.id);
  }

  function onOpenFailed(failed: OpenFailed) {
    settled.add(failed.id);
    openings = openings.filter((o) => o.id !== failed.id);
    // The target tab was closed during the decode: the user asked for that.
    if (failed.code === "documentClosed") return;
    const reason = t(`open.error.${failed.code}`, { detail: failed.detail });
    openFailureCount++;
    showError(t("open.failed", { name: failed.name, error: reason }));
  }

  /**
   * Drop zones: the image and the layers panel of the active tab add layers; anywhere else
   * opens new tabs.
   */
  function dropTargetAt(position: { x: number; y: number }): "tab" | "layer" {
    // Tauri labels the position physical, but only WebView2 reports device pixels; WebKit
    // (macOS, Linux) already reports CSS pixels.
    const scale = isWindows ? window.devicePixelRatio : 1;
    const element = document.elementFromPoint(position.x / scale, position.y / scale);
    if (active && element?.closest(".stage, [data-drop='layer']")) return "layer";
    return "tab";
  }

  // --- Saving ----------------------------------------------------------------------------------

  /** Documents being saved (a save of a large document takes seconds). */
  let saving = $state<number[]>([]);

  /**
   * The Save As and Export dialogs (ADR 0013), which share their outcome: a `.slop` file
   * becomes the document's file; an image format is written as a flattened copy (after its
   * options), and the document keeps its own file, so Ctrl+S never flattens it by surprise.
   * `formats`: `all` (Save As: `.slop` first, then every image format), `document` (saving
   * before closing: the layers must be kept) or `images` (Export: image formats, the last one
   * used first). The chosen document path, or null (cancelled, error shown, or an image copy
   * started).
   */
  async function chooseSaveAs(
    doc: DocumentView,
    formats: "all" | "document" | "images",
  ): Promise<string | null> {
    if (choosingFile || exportTarget) return null;
    choosingFile = true;
    try {
      const images = formats === "document" ? [] : exportFormatOrder();
      const documentFilter = { name: t("save.documentType"), extensions: [DOCUMENT_EXTENSION] };
      const imageFilters = images.map((format) => ({
        name: t(`export.format.${format}`),
        extensions: EXPORT_FORMATS[format].extensions,
      }));
      const path = await save(
        formats === "images"
          ? {
              title: t("export.title"),
              defaultPath: exportFileName(tabTitle(doc), EXPORT_FORMATS[images[0]].extensions[0]),
              filters: imageFilters,
            }
          : {
              title: t("save.title"),
              defaultPath: doc.path ?? exportFileName(tabTitle(doc), DOCUMENT_EXTENSION),
              filters: [documentFilter, ...imageFilters],
            },
      );
      if (path === null) return null;
      if (formats !== "images" && path.toLowerCase().endsWith(`.${DOCUMENT_EXTENSION}`)) {
        return path;
      }
      const format = formats === "document" ? null : formatOfPath(path);
      if (format === null) {
        showError(t("export.unsupportedExtension", { name: fileNameOf(path) }));
        return null;
      }
      exportTarget = { documentId: doc.id, path, format };
      return null;
    } catch (e) {
      showError(String(e));
      return null;
    } finally {
      choosingFile = false;
    }
  }

  /**
   * Save a document: to its file (incrementally), or, for Save As and a document that has no
   * file yet, through the Save As dialog. False when the document was not saved (dialog
   * cancelled, error shown, or an image copy chosen instead).
   */
  async function saveDocument(id: number, saveAs: boolean, documentOnly = false): Promise<boolean> {
    const doc = tabs.find((d) => d.id === id);
    if (!doc || saving.includes(id)) return false;
    let path: string | null = null;
    if (saveAs || doc.path === null) {
      path = await chooseSaveAs(doc, documentOnly ? "document" : "all");
      if (path === null) return false;
    }
    const name = path ? fileNameOf(path) : tabTitle(doc);
    saving.push(id);
    try {
      const view = await engine.saveDocument(id, path);
      upsert(view);
      // Save As and first saves say where the file went; plain saves only clear the mark.
      if (path !== null) {
        showToast(null, {
          title: t("save.done", { name }),
          lines: [],
          kind: "done",
          path: view.path ?? undefined,
        });
      }
      return true;
    } catch (e) {
      const failed: SaveFailed =
        typeof e === "object" && e !== null && "code" in e
          ? (e as SaveFailed)
          : { code: "internal", detail: String(e) };
      // Closed meanwhile: the user asked for that.
      if (failed.code !== "documentClosed") {
        const error = t(`save.error.${failed.code}`, { detail: failed.detail });
        showError(t("save.failed", { name, error }));
      }
      return false;
    } finally {
      saving = saving.filter((s) => s !== id);
    }
  }

  const saveActive = (saveAs: boolean) => activeId !== null && void saveDocument(activeId, saveAs);

  /** Ask before closing a tab with unsaved changes; true when it can close. */
  async function confirmClose(id: number): Promise<boolean> {
    const doc = tabs.find((d) => d.id === id);
    if (!doc?.dirty) return true;
    activate(id);
    const buttons = { yes: t("close.save"), no: t("close.discard"), cancel: t("close.cancel") };
    const answer = await message(t("close.unsaved", { name: tabTitle(doc) }), {
      title: t("close.title"),
      kind: "warning",
      buttons,
    });
    // The clicked label, or the standard name on platforms that report it.
    if (answer === buttons.yes || answer === "Yes") return saveDocument(id, false, true);
    return answer === buttons.no || answer === "No";
  }

  /** The window is closing with unsaved changes (the engine held it): ask, then quit. */
  let confirmingQuit = false;

  async function confirmQuit() {
    if (confirmingQuit) return;
    confirmingQuit = true;
    try {
      const names = tabs.filter((d) => d.dirty || saving.includes(d.id)).map(tabTitle);
      const buttons = { ok: t("quit.discard"), cancel: t("close.cancel") };
      const answer = await message(t("quit.unsaved", { names: names.join(", ") }), {
        title: t("close.title"),
        kind: "warning",
        buttons,
      });
      if (answer === buttons.ok || answer === "Ok") await engine.quit();
    } catch (e) {
      showError(String(e));
    } finally {
      confirmingQuit = false;
    }
  }

  // --- Export ---------------------------------------------------------------------------------

  /** Image formats in the order of the Save As file types: the last one used first. */
  function exportFormatOrder(): ExportFormat[] {
    const all = Object.keys(EXPORT_FORMATS) as ExportFormat[];
    return [lastExportFormat, ...all.filter((f) => f !== lastExportFormat)];
  }

  function formatOfPath(path: string): ExportFormat | null {
    const extension = path.toLowerCase().split(".").pop() ?? "";
    const formats = Object.entries(EXPORT_FORMATS) as [ExportFormat, { extensions: string[] }][];
    return formats.find(([, f]) => f.extensions.includes(extension))?.[0] ?? null;
  }

  /** The document name with `extension`, without characters files cannot have. */
  function exportFileName(name: string, extension: string): string {
    const safe = name.replace(/[\\/:*?"<>|]/g, "_");
    const stem = safe.replace(/\.[^.]+$/, "") || safe;
    return `${stem}.${extension}`;
  }

  function fileNameOf(path: string): string {
    return path.split(/[\\/]/).pop() || path;
  }

  function exportReason(failed: ExportFailed): string {
    return t(`export.error.${failed.code}`, { detail: failed.detail });
  }

  /** Start exporting (the job reports through events); the dialog closes. */
  async function startExport(documentId: number, path: string, spec: ExportSpec) {
    exportTarget = null;
    lastExportFormat = spec.format;
    try {
      await engine.exportDocument(documentId, path, spec);
    } catch (e) {
      const failed: ExportFailed =
        typeof e === "object" && e !== null && "code" in e
          ? (e as ExportFailed)
          : { code: "internal", detail: String(e) };
      // Closed meanwhile: the user asked for that.
      if (failed.code === "documentClosed") return;
      showToast(null, {
        title: t("export.failed", { name: fileNameOf(path), error: exportReason(failed) }),
        lines: [],
        kind: "error",
      });
    }
  }

  function onExportStarted(started: ExportStarted) {
    if (exports.some((job) => job.id === started.id)) return;
    exports.push({ id: started.id, name: started.name, done: 0, total: 0 });
  }

  function onExportProgress(progress: ExportProgress) {
    const job = exports.find((j) => j.id === progress.id);
    if (!job) return;
    job.done = progress.done;
    job.total = progress.total;
  }

  /** Remove a finished job; its name, for messages. */
  function endExport(id: number | undefined, path: string | null): string {
    const job = exports.find((j) => j.id === id);
    exports = exports.filter((j) => j.id !== id);
    return job?.name ?? (path ? fileNameOf(path) : "");
  }

  /** How long a toast stays, by kind; a report (`notice`) stays until dismissed. */
  const TOAST_MS = { done: 8000, error: 5000 } as const;

  /**
   * Show a toast next to the others; `id` is the export job it is about (null for anything
   * else). A success (long enough to reach its "show in folder" link) and an error disappear
   * by themselves.
   */
  function showToast(id: number | undefined | null, toast: Omit<Toast, "key">) {
    const key = id == null ? `local-${++lastLocalToast}` : `job-${id}`;
    toasts = [...toasts.filter((r) => r.key !== key), { key, ...toast }];
    if (toast.kind !== "notice") setTimeout(() => dismissToast(key), TOAST_MS[toast.kind]);
  }

  function showError(title: string) {
    showToast(null, { title, lines: [], kind: "error" });
  }

  function dismissToast(key: string) {
    toasts = toasts.filter((r) => r.key !== key);
  }

  function revealExport(path: string) {
    engine.revealInFolder(path).catch((e) => {
      showToast(null, {
        title: t("export.revealFailed", { error: String(e) }),
        lines: [],
        kind: "error",
      });
    });
  }

  function onExportFinished(finished: ExportFinished) {
    const name = endExport(finished.id, finished.path);
    const report = finished.notices.map((notice) =>
      t(`export.report.${notice.id}`, notice.count === null ? undefined : { count: notice.count }),
    );
    showToast(finished.id, {
      title: t("export.finished", { name }),
      lines: report,
      kind: report.length > 0 ? "notice" : "done",
      path: finished.path,
    });
  }

  function onExportFailed(failed: ExportFailed) {
    const name = endExport(failed.id, null);
    showToast(
      failed.id,
      failed.code === "cancelled"
        ? { title: t("export.cancelled", { name }), lines: [], kind: "done" }
        : {
            title: t("export.failed", { name, error: exportReason(failed) }),
            lines: [],
            kind: "error",
          },
    );
  }

  function percent(job: { done: number; total: number }): number {
    return job.total > 0 ? Math.floor((job.done * 100) / job.total) : 0;
  }

  // --- Paste ------------------------------------------------------------------------------------

  /** Paste the clipboard into the active document (as layers), or into a new tab. */
  /** Edit > Copy (Ctrl+C) on the selected layers; with `cut`, they are then deleted (Ctrl+X). */
  async function copyLayers(cut: boolean) {
    const doc = active;
    const ids = layersPanel?.selectedLayers().map((l) => l.id) ?? [];
    if (!doc || ids.length === 0) return;
    try {
      await engine.copyLayersToClipboard(doc.id, ids);
      if (cut) layersPanel?.deleteSelected();
    } catch (e) {
      showError(t("copy.failed", { error: String(e) }));
    }
  }

  async function paste(intoNewTab: boolean) {
    const target = intoNewTab ? null : activeId;
    try {
      const pasted = await engine.paste(target, t("paste.layerName"));
      if (pasted.kind === "image" || pasted.kind === "layers") {
        upsert(pasted.document);
        if (pasted.newTab) activate(pasted.document.id);
      } else if (pasted.kind === "nothing") {
        showError(t("paste.nothing"));
      }
    } catch (e) {
      if (e === DOCUMENT_CLOSED) await refreshTabs();
      else showError(t("paste.failed", { error: String(e) }));
    }
  }

  // --- Menu bar (ADR 0013) ------------------------------------------------------------------------

  /** A shortcut as shown in menus: `mod` (Ctrl or ⌘), `shift` and a key. */
  function keys(...parts: string[]): string {
    const names: Record<string, string> = {
      mod: modifierLabel,
      shift: t("key.shift"),
      alt: t("key.alt"),
      delete: t("key.delete"),
    };
    return parts.map((p) => names[p] ?? p).join("+");
  }

  /** Close the window: unsaved documents are asked about first (close-requested). */
  function quitApp() {
    getCurrentWindow()
      .close()
      .catch((e) => showError(String(e)));
  }

  async function showAbout() {
    const version = await getVersion().catch(() => "?");
    await message(t("about.text", { version }), { title: t("about.title"), kind: "info" });
  }

  function command(label: string, run: () => void, shortcut?: string, disabled = false): MenuItem {
    return { kind: "command", label, run, shortcut, disabled };
  }

  /** Commands on the selected layers: the Layer menu's, also the layers' context menu. */
  let layerCommands = $derived.by(() => {
    const doc = active;
    const layer = layersPanel?.selectedLayer() ?? null;
    const maskless = (layersPanel?.selectedLayers() ?? []).filter((l) => !l.mask);
    /** A new mask on the selected layers without one; from the selection, only with one. */
    const maskCommand = (kind: LayerMaskKind, label: MessageKey) =>
      command(
        t(label),
        () => addLayerMasks(kind),
        undefined,
        maskless.length === 0 || (kind.endsWith("Selection") && doc?.selectionKey == null),
      );
    const selectedCount = layersPanel?.selectedLayers().length ?? 0;
    const several = selectedCount > 1;
    return {
      duplicate: command(
        t(several ? "menu.layer.duplicateLayers" : "menu.layer.duplicate"),
        () => layersPanel?.duplicateSelected(),
        keys("mod", "J"),
        selectedCount === 0,
      ),
      rename: command(t("menu.layer.rename"), () => layersPanel?.renameSelected(), "F2", !layer),
      visibility: command(
        t(
          layer?.visible === false
            ? several
              ? "menu.layer.showLayers"
              : "menu.layer.showLayer"
            : several
              ? "menu.layer.hideLayers"
              : "menu.layer.hideLayer",
        ),
        () => layersPanel?.toggleSelectedVisibility(),
        undefined,
        selectedCount === 0,
      ),
      delete: command(
        t(several ? "layers.deleteSelected" : "layers.delete"),
        () => layersPanel?.deleteSelected(),
        keys("delete"),
        selectedCount === 0,
      ),
      newGroup: command(t("menu.layer.newGroup"), () => layersPanel?.newGroup(), undefined, !doc),
      group: command(
        t("menu.layer.group"),
        () => layersPanel?.groupSelected(),
        keys("mod", "G"),
        selectedCount === 0,
      ),
      clipping: command(
        t(layer?.clipped ? "menu.layer.releaseClipping" : "menu.layer.createClipping"),
        () => layersPanel?.toggleClippingSelected(),
        keys("alt", "mod", "G"),
        selectedCount === 0,
      ),
      ungroup: command(
        t("menu.layer.ungroup"),
        () => layersPanel?.ungroupSelected(),
        keys("shift", "mod", "G"),
        layer?.kind !== "group",
      ),
      maskRevealAll: maskCommand("revealAll", "menu.layer.maskRevealAll"),
      maskHideAll: maskCommand("hideAll", "menu.layer.maskHideAll"),
      maskRevealSelection: maskCommand("revealSelection", "menu.layer.maskRevealSelection"),
      maskHideSelection: maskCommand("hideSelection", "menu.layer.maskHideSelection"),
      maskFromTransparency: command(
        t("menu.layer.maskFromTransparency"),
        () => doc && layer && void sync(engine.addMaskFromTransparency(doc.id, layer.id)),
        undefined,
        !layer || !layer.hasAlpha || layer.mask !== null,
      ),
      maskToggle: command(
        t(layer?.mask?.enabled === false ? "menu.layer.maskEnable" : "menu.layer.maskDisable"),
        () =>
          doc &&
          layer?.mask &&
          void edit(doc.id, {
            kind: "setLayerMaskEnabled",
            id: layer.id,
            enabled: !layer.mask.enabled,
          }),
        undefined,
        !layer?.mask,
      ),
      maskDelete: command(
        t("menu.layer.maskDelete"),
        () => doc && layer && void edit(doc.id, { kind: "removeLayerMask", id: layer.id }),
        undefined,
        !layer?.mask,
      ),
    };
  });

  /** The layers' right-click menu (Photoshop shows the commands on the layers there). */
  let layerContextMenu = $derived.by((): MenuItem[] => {
    const c = layerCommands;
    const separator = { kind: "separator" as const };
    return [
      c.duplicate,
      c.rename,
      c.visibility,
      c.delete,
      separator,
      c.newGroup,
      c.group,
      c.ungroup,
      separator,
      c.clipping,
      c.maskRevealAll,
      c.maskRevealSelection,
      c.maskFromTransparency,
      c.maskToggle,
      c.maskDelete,
    ];
  });

  let menus = $derived.by((): Menu[] => {
    const doc = active;
    const busy = doc !== null && saving.includes(doc.id);
    const layer = layersPanel?.selectedLayer() ?? null;
    const selectedCount = layersPanel?.selectedLayers().length ?? 0;
    const cmd = (label: string, run: () => void, shortcut?: string, disabled = false) => ({
      kind: "command" as const,
      label,
      run,
      shortcut,
      disabled,
    });
    const separator = { kind: "separator" as const };
    return [
      {
        label: t("menu.file"),
        items: [
          cmd(t("menu.file.new"), () => void newDocument(), keys("mod", "N")),
          cmd(t("menu.file.open"), () => void openWithDialog(), keys("mod", "O")),
          cmd(t("menu.file.openFolder"), () => void openFolderWithDialog()),
          cmd(
            t("menu.file.importLayers"),
            () => doc && void openWithDialog(doc.id),
            keys("mod", "shift", "O"),
            !doc,
          ),
          separator,
          cmd(t("menu.file.close"), () => doc && void closeTab(doc.id), keys("mod", "W"), !doc),
          separator,
          cmd(t("menu.file.save"), () => saveActive(false), keys("mod", "S"), !doc || busy),
          cmd(
            t("menu.file.saveAs"),
            () => saveActive(true),
            keys("mod", "shift", "S"),
            !doc || busy,
          ),
          cmd(
            t("menu.file.export"),
            () => doc && void chooseSaveAs(doc, "images"),
            keys("mod", "shift", "E"),
            !doc,
          ),
          separator,
          cmd(t("menu.file.quit"), quitApp, keys("mod", "Q")),
        ],
      },
      {
        label: t("menu.edit"),
        items: [
          cmd(t("menu.edit.undo"), () => void undo(), keys("mod", "Z"), !doc?.canUndo),
          cmd(t("menu.edit.redo"), () => void redo(), keys("mod", "shift", "Z"), !doc?.canRedo),
          separator,
          cmd(
            t("menu.edit.cut"),
            () => void copyLayers(true),
            keys("mod", "X"),
            !doc || selectedCount === 0,
          ),
          cmd(
            t("menu.edit.copy"),
            () => void copyLayers(false),
            keys("mod", "C"),
            !doc || selectedCount === 0,
          ),
          cmd(t("menu.edit.paste"), () => void paste(false), keys("mod", "V")),
          cmd(t("menu.edit.pasteNewDocument"), () => void paste(true)),
          separator,
          cmd(
            t("menu.edit.freeTransform"),
            () => (transforming ? commitTransform() : void startFreeTransform()),
            keys("mod", "T"),
            !doc || selectedCount === 0,
          ),
          {
            kind: "submenu",
            label: t("menu.edit.transform"),
            disabled: !doc || selectedCount === 0,
            items: [
              cmd(
                t("menu.edit.transform.rotate180"),
                () => void quickTransform(affine.rotation(Math.PI)),
              ),
              cmd(
                t("menu.edit.transform.rotateCw"),
                () => void quickTransform(affine.rotation(Math.PI / 2)),
              ),
              cmd(
                t("menu.edit.transform.rotateCcw"),
                () => void quickTransform(affine.rotation(-Math.PI / 2)),
              ),
              separator,
              cmd(
                t("menu.edit.transform.flipHorizontal"),
                () => void quickTransform(affine.scaling(-1, 1)),
              ),
              cmd(
                t("menu.edit.transform.flipVertical"),
                () => void quickTransform(affine.scaling(1, -1)),
              ),
            ],
          },
          separator,
          {
            kind: "submenu",
            label: t("menu.edit.language"),
            items: Object.entries(locales).map(([code, { name }]) => ({
              kind: "command" as const,
              label: name,
              checked: getLocale() === code,
              run: () => setLocale(code as Locale),
            })),
          },
          cmd(t("menu.edit.preferences"), () => (preferences = true), keys("mod", "K")),
        ],
      },
      {
        label: t("menu.image"),
        items: [
          cmd(t("menu.image.crop"), cropImage, "C", !doc),
          cmd(
            t("menu.image.imageSize"),
            () => openSizeDialog("image"),
            keys("alt", "mod", "I"),
            !doc,
          ),
          cmd(
            t("menu.image.canvasSize"),
            () => openSizeDialog("canvas"),
            keys("alt", "mod", "C"),
            !doc,
          ),
          {
            kind: "submenu",
            label: t("menu.image.rotation"),
            disabled: !doc,
            items: [
              cmd(t("menu.image.rotation.halfTurn"), () => rotateImage("halfTurn")),
              cmd(t("menu.image.rotation.clockwise"), () => rotateImage("clockwise")),
              cmd(t("menu.image.rotation.counterClockwise"), () => rotateImage("counterClockwise")),
              separator,
              cmd(t("menu.image.rotation.flipHorizontal"), () => rotateImage("flipHorizontal")),
              cmd(t("menu.image.rotation.flipVertical"), () => rotateImage("flipVertical")),
            ],
          },
          separator,
          {
            kind: "submenu",
            label: t("layers.blendSpace"),
            disabled: !doc,
            items: (["perceptual", "linear"] as const).map((space) => ({
              kind: "command" as const,
              label: t(`layers.blendSpace.${space}`),
              checked: doc?.blendSpace === space,
              run: () => doc && void edit(doc.id, { kind: "setBlendSpace", space }),
            })),
          },
        ],
      },
      {
        label: t("menu.layer"),
        items: [
          cmd(t("layers.addFill"), () => layersPanel?.addFill(), undefined, !doc),
          {
            kind: "submenu",
            label: t("menu.layer.newAdjustment"),
            disabled: !doc,
            items: ADJUSTMENTS.map((adjustment) =>
              cmd(`${t(`adjustment.${adjustment}`)}…`, () =>
                layersPanel?.addAdjustment(adjustment),
              ),
            ),
          },
          {
            kind: "submenu",
            label: t("menu.layer.mask"),
            disabled: !layer,
            items: [
              layerCommands.maskRevealAll,
              layerCommands.maskHideAll,
              layerCommands.maskRevealSelection,
              layerCommands.maskHideSelection,
              layerCommands.maskFromTransparency,
              layerCommands.maskToggle,
              layerCommands.maskDelete,
            ],
          },
          separator,
          layerCommands.newGroup,
          layerCommands.group,
          layerCommands.ungroup,
          separator,
          layerCommands.clipping,
          separator,
          layerCommands.duplicate,
          layerCommands.rename,
          layerCommands.visibility,
          layerCommands.delete,
        ],
      },
      {
        label: t("menu.select"),
        items: [
          cmd(
            t("menu.select.all"),
            () => selectionCommand(engine.selectAll),
            keys("mod", "A"),
            !doc,
          ),
          cmd(
            t("menu.select.deselect"),
            () => selectionCommand(engine.deselect),
            keys("mod", "D"),
            doc?.selectionKey == null,
          ),
          cmd(
            t("menu.select.reselect"),
            () => selectionCommand(engine.reselect),
            keys("shift", "mod", "D"),
            !doc?.canReselect,
          ),
          cmd(
            t("menu.select.inverse"),
            () => selectionCommand(engine.invertSelection),
            keys("shift", "mod", "I"),
            !doc,
          ),
          separator,
          cmd(t("menu.select.colorRange"), openColorRange, undefined, !doc),
          cmd(t("menu.select.subject"), selectSubject, undefined, !doc),
          {
            kind: "submenu",
            label: t("menu.select.modify"),
            disabled: doc?.selectionKey == null,
            items: [
              cmd(t("menu.select.modify.border"), () => openModify("border")),
              cmd(t("menu.select.modify.smooth"), () => openModify("smooth")),
              cmd(t("menu.select.modify.expand"), () => openModify("expand")),
              cmd(t("menu.select.modify.contract"), () => openModify("contract")),
              cmd(
                t("menu.select.modify.feather"),
                () => openModify("feather"),
                keys("shift", "F6"),
              ),
            ],
          },
          cmd(
            t("menu.select.refineEdge"),
            () => openModify("refine"),
            undefined,
            doc?.selectionKey == null,
          ),
          separator,
          {
            ...cmd(t("menu.select.quickMask"), toggleQuickMask, "Q", !doc),
            checked: doc?.quickMask ?? false,
          },
          separator,
          cmd(
            t("menu.select.allLayers"),
            () => layersPanel?.selectAllLayers(),
            keys("alt", "mod", "A"),
            !doc,
          ),
          cmd(
            t("menu.select.deselectLayers"),
            () => layersPanel?.deselectLayers(),
            undefined,
            selectedCount === 0,
          ),
        ],
      },
      {
        label: t("menu.view"),
        items: [
          cmd(t("menu.view.zoomIn"), () => void viewport?.stepZoom(true), keys("mod", "+"), !doc),
          cmd(t("menu.view.zoomOut"), () => void viewport?.stepZoom(false), keys("mod", "-"), !doc),
          separator,
          cmd(t("menu.view.fit"), () => void viewport?.fit(), keys("mod", "0"), !doc),
          { ...cmd(t("menu.view.snap"), () => (snapping = !snapping)), checked: snapping },
          cmd(t("menu.view.actualSize"), () => void viewport?.zoomTo(1), keys("mod", "1"), !doc),
        ],
      },
      {
        label: t("menu.help"),
        items: [cmd(t("menu.help.about"), () => void showAbout())],
      },
    ];
  });

  // --- Keyboard --------------------------------------------------------------------------------

  function isTextField(target: EventTarget | null): boolean {
    return (
      target instanceof HTMLTextAreaElement ||
      (target instanceof HTMLInputElement && ["text", "number", "search"].includes(target.type))
    );
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" && layerTransfer) {
      endTransfer();
      return;
    }
    if (e.key === "Escape" && tabDrag) {
      cancelTabDrag();
      return;
    }
    if (e.ctrlKey && e.key === "Tab") {
      // Ctrl+Tab everywhere, like browsers and most editors (Cmd+Tab belongs to macOS).
      e.preventDefault();
      cycleTabs(e.shiftKey ? -1 : 1);
      return;
    }
    // Alt+Ctrl: Image Size (I) and Canvas Size (C), by the letter (see `shortcutLetter`: AltGr
    // may type another character, then the physical key counts).
    if (hasShortcutModifier(e) && e.altKey && !e.shiftKey && !isTextField(e.target)) {
      const letter = shortcutLetter(e);
      if (letter === "i" || letter === "c") {
        e.preventDefault();
        if (!e.repeat) openSizeDialog(letter === "i" ? "image" : "canvas");
        return;
      }
    }
    // The tools: a letter alone (V, M, C), Shift+letter for the next variant, as in Photoshop;
    // not while typing.
    if (!hasShortcutModifier(e) && !e.altKey && !isTextField(e.target)) {
      const slot = slotForLetter(shortcutLetter(e));
      if (slot) {
        e.preventDefault();
        if (!e.repeat) selectSlot(slot, e.shiftKey);
        return;
      }
    }
    // [ and ]: the brush size, by the physical keys as in Photoshop (^ and $ on AZERTY).
    if (
      tool === "quickSelection" &&
      (e.code === "BracketLeft" || e.code === "BracketRight") &&
      !hasShortcutModifier(e) &&
      !e.altKey &&
      !isTextField(e.target)
    ) {
      e.preventDefault();
      quick.size = stepBrush(quick.size, e.code === "BracketRight");
      return;
    }
    // Shift+F6: Select > Modify > Feather, as in Photoshop.
    if (e.key === "F6" && e.shiftKey && !hasShortcutModifier(e) && !e.altKey) {
      e.preventDefault();
      if (!e.repeat) openModify("feather");
      return;
    }
    // Q: Quick Mask, a letter alone like the tools.
    if (
      shortcutLetter(e) === "q" &&
      !hasShortcutModifier(e) &&
      !e.altKey &&
      !e.shiftKey &&
      !isTextField(e.target)
    ) {
      e.preventDefault();
      if (!e.repeat) toggleQuickMask();
      return;
    }
    // Select > Deselect, Reselect, Inverse and All (Ctrl+D, Shift+Ctrl+D, Shift+Ctrl+I, Ctrl+A);
    // not while typing.
    if (hasShortcutModifier(e) && !e.altKey && !isTextField(e.target) && active) {
      const command =
        shortcutLetter(e) === "d"
          ? e.shiftKey
            ? engine.reselect
            : engine.deselect
          : shortcutLetter(e) === "i" && e.shiftKey
            ? engine.invertSelection
            : shortcutLetter(e) === "a" && !e.shiftKey
              ? engine.selectAll
              : null;
      if (command) {
        e.preventDefault();
        if (!e.repeat) selectionCommand(command);
        return;
      }
    }
    if (!hasShortcutModifier(e) || e.altKey) return;
    // Letters as typed on Latin layouts (AZERTY too), see `shortcutLetter`.
    const key = shortcutLetter(e) ?? e.key.toLowerCase();
    if (e.shiftKey && key === "e") {
      e.preventDefault();
      if (!e.repeat && active) void chooseSaveAs(active, "images");
      return;
    }
    if (key === "s") {
      e.preventDefault();
      if (!e.repeat) saveActive(e.shiftKey);
      return;
    }
    if (key === "k" && !e.shiftKey) {
      e.preventDefault();
      preferences = true;
      return;
    }
    if (key === "q" && !e.shiftKey) {
      e.preventDefault();
      if (!e.repeat) quitApp();
      return;
    }
    if (key === "o") {
      e.preventDefault();
      if (!e.shiftKey) void openWithDialog();
      else if (active && !e.repeat) void openWithDialog(active.id);
      return;
    }
    if (key === "n" && !e.shiftKey) {
      e.preventDefault();
      void newDocument();
      return;
    }
    if (key === "w" && !e.shiftKey) {
      e.preventDefault();
      // A held key must not close tab after tab.
      if (!e.repeat && activeId !== null) void closeTab(activeId);
      return;
    }
    // Text fields keep their own undo and paste.
    if (e.target instanceof HTMLInputElement && ["text", "number"].includes(e.target.type)) return;
    if (key === "v" && !e.shiftKey) {
      e.preventDefault();
      if (!e.repeat) void paste(false);
      return;
    }
    if ((key === "c" || key === "x") && !e.shiftKey) {
      e.preventDefault();
      if (!e.repeat) void copyLayers(key === "x");
      return;
    }
    if (key === "t" && !e.shiftKey) {
      e.preventDefault();
      if (!e.repeat) {
        if (transforming) commitTransform();
        else void startFreeTransform();
      }
      return;
    }
    if (key === "z" && !e.shiftKey) {
      e.preventDefault();
      void undo();
    } else if ((key === "z" && e.shiftKey) || key === "y") {
      e.preventDefault();
      void redo();
    }
  }

  onMount(() => {
    let stopEvents: (() => void) | null = null;
    let stopExportEvents: (() => void) | null = null;
    let destroyed = false;
    void onExportEvents({
      started: onExportStarted,
      progress: onExportProgress,
      finished: onExportFinished,
      failed: onExportFailed,
    }).then((stop) => {
      if (destroyed) stop();
      else stopExportEvents = stop;
    });
    // Subscribe first, then catch up with what happened before (e.g. startup files).
    void onOpenEvents({
      started: onOpenStarted,
      finished: onOpenFinished,
      failed: onOpenFailed,
    }).then(async (stop) => {
      if (destroyed) return stop();
      stopEvents = stop;
      const [mode, documents, pending, failures] = await Promise.all([
        engine.presenterMode(),
        engine.documents(),
        engine.openings(),
        engine.openFailures(),
      ]);
      nativeCanvas = mode === "window";
      document.documentElement.classList.toggle("native-canvas", nativeCanvas);
      documents.forEach(upsert);
      if (activeId === null) activeId = tabs.at(-1)?.id ?? null;
      pending.forEach(onOpenStarted);
      const lastFailure = failures.at(-1);
      if (lastFailure) onOpenFailed(lastFailure);
      ready = true;
    });
    const stopDrop = getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload;
      if (payload.type === "enter" || payload.type === "over") {
        dropTarget = dropTargetAt(payload.position);
      } else if (payload.type === "leave") {
        dropTarget = null;
      } else {
        const target = dropTargetAt(payload.position);
        dropTarget = null;
        if (payload.paths.length === 0) return;
        void openFiles(
          payload.paths,
          target === "layer" && active ? { layerOf: active.id } : "tab",
        );
      }
    });
    const stopClose = listen("close-requested", () => void confirmQuit());
    engine.gpuInfo().then(
      (info) => (gpu = info),
      (e) => (gpuError = String(e)),
    );
    return () => {
      destroyed = true;
      stopEvents?.();
      stopExportEvents?.();
      void stopDrop.then((unlisten) => unlisten());
      void stopClose.then((unlisten) => unlisten());
    };
  });
</script>

<svelte:window {onkeydown} onblur={cancelTabDrag} />

<div class="app">
  <header class="menubar">
    <img class="logo" src="/favicon.svg" alt="" draggable="false" />
    <MenuBar {menus} />
    <span class="brand">SlopShop</span>
    <span class="tag">{t("app.preAlpha")}</span>
  </header>

  <OptionsBar
    {tool}
    bind:autoSelect
    bind:selectionMode
    bind:feather
    bind:antiAlias
    bind:wand
    bind:quick
  />

  <main
    class:has-panel={active !== null}
    class:transferring={layerTransfer !== null}
    bind:this={mainElement}
    onpointermove={onTransferMove}
    onpointerup={onTransferUp}
    onpointercancel={endTransfer}
  >
    <Toolbar {tool} choices={toolChoices} onselect={selectTool} />
    <section class="workspace">
      <div
        class="tabbar"
        class:drop={dropTarget === "tab"}
        class:reordering={tabDrag?.moved}
        role="tablist"
        bind:this={tabbar}
      >
        {#each tabs as doc, index (doc.id)}
          <div
            class="tab"
            class:active={doc.id === activeId}
            class:hovered={tabHover?.id === doc.id}
            class:dragging={tabDrag?.moved && tabDrag.id === doc.id}
            class:drop-before={tabDrag?.slot === index}
            class:drop-after={index === tabs.length - 1 && tabDrag?.slot === tabs.length}
            data-id={doc.id}
            role="tab"
            tabindex="-1"
            aria-selected={doc.id === activeId}
            title={t("tabs.hint", { name: tabTitle(doc) })}
            ondblclick={() => (renamingTab = doc.id)}
            onpointerdown={(e) => onTabPointerDown(e, doc.id)}
            onpointermove={onTabPointerMove}
            onpointerup={onTabPointerUp}
            onpointercancel={cancelTabDrag}
            onauxclick={(e) => {
              // Middle click closes, like browsers.
              if (e.button === 1) void closeTab(doc.id);
            }}
          >
            {#if renamingTab === doc.id}
              <input
                class="tab-rename"
                type="text"
                value={tabTitle(doc)}
                aria-label={t("tabs.rename")}
                spellcheck="false"
                {@attach focusAndSelect}
                onpointerdown={(e) => e.stopPropagation()}
                ondblclick={(e) => e.stopPropagation()}
                onkeydown={(e) => {
                  if (e.key === "Enter") commitTabRename(doc, e.currentTarget);
                  else if (e.key === "Escape") {
                    e.stopPropagation();
                    renamingTab = null;
                  }
                }}
                onblur={(e) => commitTabRename(doc, e.currentTarget)}
              />
            {:else}
              <span class="tab-name">{tabTitle(doc)}</span>
            {/if}
            {#if saving.includes(doc.id)}
              <span
                class="spinner"
                title={t("save.saving", { name: tabTitle(doc) })}
                aria-label={t("save.saving", { name: tabTitle(doc) })}
              ></span>
            {:else if doc.dirty}
              <span
                class="tab-dirty"
                title={t("save.unsavedMark")}
                aria-label={t("save.unsavedMark")}
              >
                ●
              </span>
            {/if}
            {#if doc.id === activeId && frame}
              <span class="tab-zoom">@ {formatZoom(frame.zoom)}</span>
            {/if}
            <button
              class="icon-btn tab-close"
              title={t("tabs.closeHint", { mod: modifierLabel })}
              onpointerdown={(e) => e.stopPropagation()}
              onclick={() => closeTab(doc.id)}
            >
              ✕
            </button>
          </div>
        {/each}
        {#each openings.filter((o) => o.target.kind === "newTab") as opening (opening.id)}
          <div class="tab pending" title={t("open.opening", { name: opening.name })}>
            <span class="tab-name">{opening.name}</span>
            <span class="spinner" aria-hidden="true"></span>
          </div>
        {/each}
      </div>

      <div class="stage" class:see-through={nativeCanvas && active}>
        {#if active}
          {#key active.id}
            <Viewport
              bind:this={viewport}
              native={nativeCanvas}
              documentId={active.id}
              revision={active.revision}
              quickMask={active.quickMask}
              onframe={(stats) => (frame = stats)}
              onmovestart={onMoveStart}
              onmove={tool === "move" && !transforming ? onMoveDrag : undefined}
              onmoveend={onMoveEnd}
              ondoubleclick={() => void startFreeTransform()}
              {guides}
            >
              {#snippet overlay(mapping)}
                {#if active?.selectionKey != null}
                  <SelectionOutline
                    hidden={active.quickMask}
                    {mapping}
                    documentId={active.id}
                    selectionKey={active.selectionKey}
                    width={active.width}
                    height={active.height}
                  />
                {/if}
                {#if cropping && cropping.document === active?.id}
                  <CropBox
                    {mapping}
                    canvas={canvasBounds(active)}
                    targets={snapping ? cropping.targets : []}
                    onapply={applyCrop}
                    oncancel={() => (cropping = null)}
                  />
                {:else if transforming && transforming.document === active?.id}
                  <FreeTransform
                    {mapping}
                    box={transforming.box}
                    targets={snapping ? transforming.targets : []}
                    onchange={onTransformChange}
                    oncommit={commitTransform}
                    oncancel={cancelTransform}
                  />
                {:else if colorRange && colorRange.document === active?.id}
                  <!-- Color Range open: a click on the image samples a color. -->
                  <div
                    class="sample-overlay"
                    role="presentation"
                    onpointerdown={(e) => {
                      if (e.button !== 0 || mapping.hand || !colorRange) return;
                      const [x, y] = mapping.toDocument(e.clientX, e.clientY);
                      sampleAt(colorRange, x, y, e);
                    }}
                  ></div>
                {:else if tool === "wand"}
                  <WandTool {mapping} mode={selectionMode} onpick={magicWand} />
                {:else if tool === "objectSelection"}
                  <ObjectSelectionTool
                    {mapping}
                    mode={selectionMode}
                    busy={aiBusy}
                    onhover={objectHover}
                    onselect={objectSelect}
                  />
                {:else if tool === "quickSelection"}
                  <QuickSelectionTool
                    {mapping}
                    mode={selectionMode}
                    size={quick.size}
                    busy={aiBusy}
                    onstroke={quickStroke}
                  />
                {:else if tool === "lasso" || tool === "polygonalLasso"}
                  <LassoTool
                    {mapping}
                    polygonal={tool === "polygonalLasso"}
                    mode={selectionMode}
                    onselect={selectShape}
                    ondeselect={() => selectionCommand(engine.deselect)}
                  />
                {:else if tool === "marquee" || tool === "ellipse"}
                  <MarqueeTool
                    {mapping}
                    kind={tool === "marquee" ? "rectangle" : "ellipse"}
                    mode={selectionMode}
                    onselect={selectShape}
                    ondeselect={() => selectionCommand(engine.deselect)}
                  />
                {/if}
              {/snippet}
            </Viewport>
          {/key}
        {:else if ready}
          <div class="welcome">
            <img src="/favicon.svg" alt="" draggable="false" />
            <p>{t("welcome.title")}</p>
            <div class="welcome-actions">
              <button class="btn primary" onclick={() => openWithDialog()}>
                {t("welcome.open")}
              </button>
              <button class="btn" onclick={newDocument}>{t("welcome.new")}</button>
            </div>
            <p class="muted">{t("welcome.drop")}</p>
          </div>
        {/if}
        {#if dropTarget}
          <div class="drop-hint" class:layer={dropTarget === "layer"}>
            {t(dropTarget === "layer" ? "drop.layer" : "drop.newTab")}
          </div>
        {:else if copyHint}
          <div class="drop-hint layer">{copyHint}</div>
        {/if}
        {#if layerTransfer?.overTarget}
          <div class="drop-hint layer">
            {t("drop.copySelectedLayers", { count: layerTransfer.ids.length })}
          </div>
        {/if}
      </div>
    </section>

    {#if active}
      <!-- Properties (the selected adjustment layer's, ADR 0020) below Layers: the list never moves. -->
      <div class="sidebar">
        {#key active.id}
          <LayersPanel
            bind:this={layersPanel}
            doc={active}
            onedit={edit}
            onlive={live}
            ongestureend={endGesture}
            contextMenu={layerContextMenu}
            onlayerdrag={onLayerDrag}
          />
        {/key}
        {#if selectedAdjustment}
          <PropertiesPanel
            documentId={active.id}
            layer={selectedAdjustment}
            onedit={edit}
            onlive={live}
            ongestureend={endGesture}
          />
        {/if}
      </div>
    {/if}
    {#if dragGhost}
      {@const ghostDoc = tabs.find((d) => d.id === dragGhost?.document)}
      {@const ghostLayer = ghostDoc ? findLayer(ghostDoc.layers, dragGhost.ids.at(-1) ?? -1) : null}
      {#if ghostDoc && ghostLayer}
        <div class="drag-ghost" style:left="{dragGhost.x + 14}px" style:top="{dragGhost.y + 10}px">
          {#if ghostLayer.kind === "group"}
            <span class="ghost-folder"><Icon name="folder" size={26} /></span>
          {:else}
            <LayerThumbnail documentId={ghostDoc.id} layer={ghostLayer} size={36} />
          {/if}
          <span class="ghost-name">{ghostLayer.name}</span>
          {#if dragGhost.ids.length > 1}<span class="ghost-count">{dragGhost.ids.length}</span>{/if}
        </div>
      {/if}
    {/if}
  </main>

  <footer class="status">
    {#if active}
      <ZoomSlider
        zoom={frame?.zoom ?? null}
        hint={t("view.hint", { mod: modifierLabel })}
        onzoom={(zoom) => viewport?.zoomTo(zoom) ?? Promise.resolve()}
        onstep={(zoomIn) => void viewport?.stepZoom(zoomIn)}
        onfit={() => void viewport?.fit()}
      />
    {/if}
    {#if active}
      <span class="doc-meta">
        {t("document.info", {
          width: active.width,
          height: active.height,
          space: t(`colorSpace.${active.workingSpace}`),
        })}
      </span>
    {/if}
    <span>
      {#if gpu}
        {t("status.gpu", { name: gpu.name, backend: gpu.backend })}
      {:else if gpuError}
        {t("status.gpuUnavailable", { error: gpuError })}
      {:else}
        {t("status.gpuInit")}
      {/if}
    </span>
    {#if openings.length > 0}
      <span class="busy">
        {t("open.opening", { name: openings.map((o) => o.name).join(", ") })}
      </span>
    {/if}
    {#if notices.length > 0}
      <span class="notice">{notices.join(" · ")}</span>
    {/if}
    <span class="right">
      {#if active}
        {t("status.revision", { revision: active.revision })}
      {/if}
      {#if frame}
        · {t("status.frameTime", {
          render: frame.renderMs.toFixed(1),
          total: frame.totalMs.toFixed(1),
        })}
      {/if}
    </span>
  </footer>
</div>

{#if colorRange && colorRange.document === activeId && active}
  <ColorRangeDialog
    bind:range={colorRange}
    width={active.width}
    height={active.height}
    onapply={applyColorRange}
    onclose={() => (colorRange = null)}
  />
{/if}

{#if modifyDialog && modifyDialog.document === activeId}
  <ModifyDialog
    kind={modifyDialog.kind}
    value={modifyAmounts[modifyDialog.kind]}
    max={modifyDialog.kind === "feather"
      ? MAX_FEATHER
      : modifyDialog.kind === "refine"
        ? MAX_REFINE
        : MAX_MODIFY}
    onapply={applyModify}
    onclose={() => (modifyDialog = null)}
  />
{/if}

{#if preferences}
  <PreferencesDialog onclose={() => (preferences = false)} />
{/if}

{#if aiDownload}
  <AiDownloadDialog
    components={aiDownload.components}
    ondone={() => {
      const then = aiDownload?.then;
      aiDownload = null;
      then?.();
    }}
    onclose={() => (aiDownload = null)}
  />
{/if}

{#if sizeDialog && sizeDoc}
  {#key sizeDialog}
    <SizeDialog
      mode={sizeDialog.mode}
      width={sizeDoc.width}
      height={sizeDoc.height}
      onapply={applySize}
      onclose={() => (sizeDialog = null)}
    />
  {/key}
{/if}

{#if vectorImport}
  {#key vectorImport}
    <VectorImportDialog
      path={vectorImport.path}
      name={fileNameOf(vectorImport.path)}
      info={vectorImport.info}
      onopen={(pages, dpi) => settleVectorImport({ pages, dpi })}
      onclose={() => settleVectorImport(null)}
    />
  {/key}
{/if}

{#if exportDoc && exportTarget}
  {#key exportTarget}
    <ExportDialog
      documentId={exportDoc.id}
      width={exportDoc.width}
      height={exportDoc.height}
      path={exportTarget.path}
      format={exportTarget.format}
      onexport={startExport}
      onclose={() => (exportTarget = null)}
    />
  {/key}
{/if}

<!-- Exports run in the background: progress and outcome in a card above the status bar. -->
{#if exports.length > 0 || toasts.length > 0}
  <aside class="export-card" aria-live="polite">
    {#each exports as job (job.id)}
      <div class="export-job">
        <div class="export-row">
          <span class="export-title">
            {t("export.progress", { name: job.name, percent: percent(job) })}
          </span>
          <button
            class="icon-btn card-button"
            title={t("export.stop")}
            aria-label={t("export.stop")}
            onclick={() => engine.cancelExport(job.id)}
          >
            ✕
          </button>
        </div>
        <div class="export-bar"><span style:width="{percent(job)}%"></span></div>
      </div>
    {/each}
    {#each toasts as result (result.key)}
      <div class="export-result {result.kind}">
        <div class="export-row">
          <span class="export-title">{result.title}</span>
          <button
            class="icon-btn card-button"
            title={t("export.dismiss")}
            aria-label={t("export.dismiss")}
            onclick={() => dismissToast(result.key)}
          >
            ✕
          </button>
        </div>
        {#each result.lines as line, i (i)}
          <p>{line}</p>
        {/each}
        {#if result.path}
          {@const path = result.path}
          <button class="card-link" onclick={() => revealExport(path)}>
            {t("export.showInFolder")}
          </button>
        {/if}
      </div>
    {/each}
  </aside>
{/if}

<style>
  /* Export progress and outcome, above the status bar. Opaque: it floats over the canvas. */
  .export-card {
    position: fixed;
    right: 12px;
    bottom: 30px;
    z-index: 10;
    display: grid;
    gap: 8px;
    width: 300px;
    max-height: calc(100vh - 80px);
    overflow-y: auto;
    padding: 8px 10px;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    box-shadow: 0 6px 20px #0008;
  }

  .export-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .export-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .export-bar {
    height: 4px;
    margin-top: 5px;
    border-radius: 2px;
    background: var(--slider-track);
    overflow: hidden;
  }

  .export-bar > span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.15s linear;
  }

  .export-result p {
    margin: 4px 0 0;
    color: var(--text-muted);
    user-select: text;
  }

  .export-result.notice .export-title {
    color: #e0b35a;
  }

  .export-result.error .export-title {
    color: var(--danger-fg);
    white-space: normal;
  }

  .card-button {
    width: 20px;
    height: 20px;
    font-size: 9px;
  }

  .card-link {
    margin-top: 4px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    text-decoration: underline;
  }

  .card-link:hover {
    color: var(--text);
  }

  .app {
    display: grid;
    grid-template-rows: 30px 32px 1fr 22px;
    height: 100vh;
  }

  .menubar {
    display: flex;
    align-items: stretch;
    gap: 6px;
    padding: 0 8px;
    background: var(--chrome);
    border-bottom: 1px solid var(--border-dark);
  }

  .logo {
    width: 16px;
    height: 16px;
    align-self: center;
  }

  .brand {
    align-self: center;
    margin-left: auto;
    font-weight: 600;
  }

  .tag {
    align-self: center;
    padding: 0 5px;
    border-radius: 2px;
    background: var(--brand-muted);
    color: var(--brand);
    font-size: 10px;
    line-height: 15px;
  }

  main {
    display: grid;
    grid-template-columns: 40px 1fr;
    gap: 1px;
    min-height: 0;
    background: var(--border-dark);
  }

  main.has-panel {
    grid-template-columns: 40px 1fr 260px;
  }

  /* Native presentation: the canvas area shows the window surface drawn by the engine. */
  :global(.native-canvas) main {
    background: transparent;
  }

  .sidebar {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .sidebar > :global(:first-child) {
    flex: 1 1 0;
  }

  .workspace {
    display: grid;
    grid-template-rows: 26px 1fr;
    min-width: 0;
    min-height: 0;
  }

  .tabbar {
    display: flex;
    overflow-x: auto;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    scrollbar-width: none;
  }

  .tabbar.drop {
    box-shadow: inset 0 0 0 2px var(--accent);
  }

  .tab {
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 240px;
    padding: 0 6px 0 12px;
    border-right: 1px solid var(--border-dark);
    color: var(--text-muted);
    white-space: nowrap;
    cursor: default;
  }

  .tab.active {
    background: var(--panel);
    color: var(--text);
  }

  .tab:not(.active):hover {
    background: var(--hover);
  }

  .tabbar.reordering,
  .tabbar.reordering .tab {
    cursor: grabbing;
  }

  .tab.dragging {
    opacity: 0.5;
  }

  /* Where a dragged tab will land, like the layer drop line. */
  .tab.drop-before::before,
  .tab.drop-after::after {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    background: var(--accent);
    z-index: 1;
  }

  .tab.drop-before::before {
    left: -1px;
  }

  .tab.drop-after::after {
    right: -1px;
  }

  .tab-name {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .tab-rename {
    width: 140px;
    height: 18px;
  }

  .tab-zoom {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .tab-dirty {
    color: var(--text-muted);
    font-size: 9px;
  }

  .tab-close {
    width: 18px;
    height: 18px;
    font-size: 10px;
    visibility: hidden;
  }

  .tab.active .tab-close,
  .tab:hover .tab-close {
    visibility: visible;
  }

  .tab.pending {
    font-style: italic;
  }

  .spinner {
    width: 10px;
    height: 10px;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .stage {
    position: relative;
    display: grid;
    min-width: 0;
    min-height: 0;
    background: var(--pasteboard);
  }

  /* Color Range open: clicks on the image sample colors. */
  .sample-overlay {
    position: absolute;
    inset: 0;
    cursor: crosshair;
  }

  .stage.see-through {
    background: transparent;
  }

  .welcome {
    place-self: center;
    display: grid;
    justify-items: center;
    gap: 10px;
    color: var(--text);
    font-size: 13px;
  }

  .welcome img {
    width: 56px;
    height: 56px;
    opacity: 0.9;
  }

  .welcome p {
    margin: 0;
  }

  .welcome-actions {
    display: flex;
    gap: 8px;
  }

  .muted {
    color: var(--text-muted);
    font-size: 11px;
  }

  .drop-hint {
    position: absolute;
    inset: 12px;
    display: grid;
    place-items: center;
    border: 2px dashed var(--accent);
    border-radius: 6px;
    background: #3b8eea1a;
    color: var(--text);
    font-size: 14px;
    pointer-events: none;
  }

  .drag-ghost {
    position: fixed;
    z-index: 300;
    display: flex;
    align-items: center;
    gap: 8px;
    max-width: 260px;
    padding: 4px 8px 4px 4px;
    background: var(--panel);
    border: 1px solid var(--border-strong);
    box-shadow: 0 4px 14px rgb(0 0 0 / 0.45);
    opacity: 0.9;
    pointer-events: none;
  }

  .ghost-folder {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    color: var(--text-muted);
  }

  .ghost-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ghost-count {
    min-width: 18px;
    padding: 0 5px;
    border-radius: 9px;
    background: var(--accent);
    color: var(--accent-text, #fff);
    font-size: 11px;
    text-align: center;
  }

  main.transferring {
    cursor: copy;
  }

  .tab.hovered {
    box-shadow: inset 0 -2px 0 var(--accent);
  }

  .drop-hint.layer {
    border-style: solid;
  }

  .status {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 0 8px;
    background: var(--chrome);
    border-top: 1px solid var(--border-dark);
    color: var(--text-muted);
    font-size: 10px;
    white-space: nowrap;
  }

  .status .doc-meta {
    font-variant-numeric: tabular-nums;
  }

  .status .busy {
    color: var(--accent);
  }

  .status .notice {
    overflow: hidden;
    text-overflow: ellipsis;
    color: #e0b35a;
  }

  .status .right {
    margin-left: auto;
    font-variant-numeric: tabular-nums;
  }
</style>
