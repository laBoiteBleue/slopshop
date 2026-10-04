<script lang="ts">
  import { getVersion } from "@tauri-apps/api/app";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { listen } from "@tauri-apps/api/event";
  import { message, open as openDialog, save } from "@tauri-apps/plugin-dialog";
  import { tick, onMount, untrack } from "svelte";
  import {
    DOCUMENT_CLOSED,
    DOCUMENT_EXTENSION,
    ADJUSTMENTS,
    EXPORT_FORMATS,
    engine,
    onDocumentUpdated,
    onExportEvents,
    onAiProgress,
    onRecentFiles,
    onOpenEvents,
    type AdjustmentId,
    type AdjustmentSettings,
    type FilterId,
    type BrushRequest,
    type StrokeRequest,
    type ClipboardContents,
    type CopyRequest,
    type PasteKind,
    type PaintTarget,
    type DocumentInfo,
    type DocumentView,
    type BakeRequest,
    type EditRequest,
    type LayerStyle,
    type LayerView,
    type LayerMaskKind,
    type SelectionModify,
    type SelectionMode,
    type SelectionShape,
    type ExportFailed,
    type ExportFinished,
    type ExportFormat,
    type ExportProgress,
    type AiTaskProgress,
    type ExportSpec,
    type ExportStarted,
    type GpuInfo,
    type AutoCorrection,
    type ImageTurn,
    type TrimSettings,
    type Bounds,
    type Matrix,
    type SnapTargets,
    type MovePixelsRequest,
    type OpenFailed,
    type OpenFinished,
    type Opening,
    type VectorInfo,
    type SaveFailed,
    type AiComponent,
    type AiFeature,
    type AiFailure,
  } from "./lib/engine";
  import { t } from "./lib/i18n/index.svelte";
  import type { MessageKey } from "./lib/i18n/en";
  import ExportDialog from "./lib/ExportDialog.svelte";
  import VectorImportDialog from "./lib/VectorImportDialog.svelte";
  import SizeDialog from "./lib/SizeDialog.svelte";
  import PreferencesDialog from "./lib/PreferencesDialog.svelte";
  import KeyboardShortcutsDialog from "./lib/KeyboardShortcutsDialog.svelte";
  import MenuBar, { type Menu, type MenuItem } from "./lib/MenuBar.svelte";
  import { isMac, isWindows, modifierLabel } from "./lib/platform";
  import { SHORTCUTS, formatShortcut, type CommandId, type Shortcut } from "./lib/commands";
  import { formatZoom } from "./lib/format";
  import Icon from "./lib/Icon.svelte";
  import LayerThumbnail from "./lib/LayerThumbnail.svelte";
  import LayersPanel from "./lib/LayersPanel.svelte";
  import PropertiesPanel from "./lib/PropertiesPanel.svelte";
  import PanelResizer from "./lib/PanelResizer.svelte";
  import { clampPanelWidth, loadPanelWidth } from "./lib/panelWidth";
  import Viewport, { type FrameStats } from "./lib/Viewport.svelte";
  import Toolbar from "./lib/Toolbar.svelte";
  import {
    MASK_COLORS,
    loadQuickMaskOpacity,
    saveQuickMaskOpacity,
    type ColorPair,
  } from "./lib/quickMask";
  import OptionsBar from "./lib/OptionsBar.svelte";
  import { isEraser, isPaintTool, slotOf, slotTool, type ToolId, type ToolSlot } from "./lib/tools";
  import PaintTool from "./lib/PaintTool.svelte";
  import FillDialog, { type FillSettings } from "./lib/FillDialog.svelte";
  import LayerStyleDialog, { type StylePage } from "./lib/LayerStyleDialog.svelte";
  import { EFFECTS, styleEdit, withEffect, type EffectId } from "./lib/layerStyle";
  import AdjustDialog from "./lib/AdjustDialog.svelte";
  import FilterDialog from "./lib/FilterDialog.svelte";
  import StrokeDialog, { type StrokeSettings } from "./lib/StrokeDialog.svelte";
  import NewDocumentDialog, { type NewDocumentSettings } from "./lib/NewDocumentDialog.svelte";
  import ColorPickerDialog from "./lib/ColorPickerDialog.svelte";
  import RecentFiles from "./lib/RecentFiles.svelte";
  import DocumentInfoDialog from "./lib/DocumentInfoDialog.svelte";
  import PrintDialog from "./lib/PrintDialog.svelte";
  import { baseName, recentLabels } from "./lib/recent";
  import { exportFileName, formatOfPath, formatOrder, isVectorPath } from "./lib/fileNames";
  import { cycled, moveTab as moveTabTo, tabSlot, upsert as upsertTab } from "./lib/tabs";
  import { isTextField, keyAction } from "./lib/keymap";
  import { pasteUnfit } from "./lib/clipboard";
  import {
    autoLevelsEdit,
    canvasBounds,
    cropEdit,
    outsideCanvas,
    rotateEdit,
    sizeEdit,
  } from "./lib/imageEdits";
  import { landing, nudged, pixelTarget as movedPixels, type PixelTarget } from "./lib/moveTool";
  import { findLayer, visibleRasters, walk } from "./lib/layerTree";
  import {
    editableEntry,
    entryEdit,
    sameSettings,
    stepSettings,
    withStep,
    type SettingsChange,
  } from "./lib/stackEntries";
  import {
    FILTERS,
    applyFilterEdit,
    filterEntryEdit,
    filterSteps,
    filterable,
    type FilterSettings,
  } from "./lib/filters";
  import { ALIGNS, DISTRIBUTES, type AlignId, type DistributeId } from "./lib/align";
  import { canFlatten, canMergeVisible, canRasterize } from "./lib/bake";
  import {
    clippingReleases,
    fillColorEdit,
    fillHex,
    maskEnabledToggle,
    maskRemoval,
    referenceMask,
    type Arrangement,
  } from "./lib/layerEdits";
  import { hexToSrgb, srgbToHex } from "./lib/color";
  import MarqueeTool from "./lib/MarqueeTool.svelte";
  import ModifyDialog from "./lib/ModifyDialog.svelte";
  import SaveSelectionDialog from "./lib/SaveSelectionDialog.svelte";
  import SelectionsPanel from "./lib/SelectionsPanel.svelte";
  import PanelDock from "./lib/PanelDock.svelte";
  import { clickTab, loadDock, saveDock, type DockPanel } from "./lib/panelDock";
  import type { IconName } from "./lib/Icon.svelte";
  import RotateDialog from "./lib/RotateDialog.svelte";
  import TrimDialog from "./lib/TrimDialog.svelte";
  import { MAX_FEATHER, MAX_MODIFY, stepBrush } from "./lib/selection";
  import { latestWins } from "./lib/latest";
  import { combinedRows, loadedRows, type Combination } from "./lib/savedSelections";
  import StrokeTrail from "./lib/StrokeTrail.svelte";
  import SelectAndMaskPanel, {
    DEFAULT_REFINE,
    type RefineSettings,
  } from "./lib/SelectAndMaskPanel.svelte";
  import LassoTool from "./lib/LassoTool.svelte";
  import WandTool from "./lib/WandTool.svelte";
  import EyedropperOverlay from "./lib/EyedropperOverlay.svelte";
  import QuickSelectionTool from "./lib/QuickSelectionTool.svelte";
  import ObjectSelectionTool, { type ObjectHover } from "./lib/ObjectSelectionTool.svelte";
  import AiDownloadDialog from "./lib/AiDownloadDialog.svelte";
  import { failureMessage } from "./lib/ai";
  import ColorRangeDialog, {
    colorRangeRequest,
    sampleAt,
    type ColorRangeState,
  } from "./lib/ColorRangeDialog.svelte";
  import SelectionOutline from "./lib/SelectionOutline.svelte";
  import SelectionDrag from "./lib/SelectionDrag.svelte";
  import { SNAP_CSS_PX, type Guide } from "./lib/snap";
  import FreeTransform from "./lib/FreeTransform.svelte";
  import TransformFields from "./lib/TransformFields.svelte";
  import ContextMenu from "./lib/ContextMenu.svelte";
  import CropBox from "./lib/CropBox.svelte";
  import * as affine from "./lib/affine";
  import { antsRequest, prefersReducedMotion } from "./lib/ants";
  import ZoomSlider from "./lib/ZoomSlider.svelte";

  /** Open documents, in tab order. */
  let tabs = $state<DocumentView[]>([]);
  let activeId = $state<number | null>(null);
  let active = $derived(tabs.find((d) => d.id === activeId) ?? null);
  let ready = $state(false);
  /** File > Open Recent and the welcome page: files and folders, newest first (engine's list). */
  let recentFiles = $state<string[]>([]);
  /** Native presentation: the engine draws the canvas area under the page (ADR 0002). */
  let nativeCanvas = $state(false);

  let gpu = $state<GpuInfo | null>(null);
  let gpuError = $state<string | null>(null);
  let frame = $state<FrameStats | null>(null);
  /** Viewport of the active tab. */
  let viewport = $state<Viewport | null>(null);
  let layersPanelInstance = $state<LayersPanel | null>(null);
  /**
   * Layers panel of the active tab (the Layer menu acts on its selection). None without a tab:
   * the instance bound stays set for a moment after its tab closes, and asking it for its
   * selection would read the closed document (the menus did, and the update stopped there).
   */
  let layersPanel = $derived(active ? layersPanelInstance : null);
  /** The active layer when it is an adjustment or a fill layer: the Properties panel shows it. */
  let selectedProperties = $derived.by(() => {
    const layer = layersPanel?.selectedLayer() ?? null;
    return layer?.kind === "adjustment" || layer?.kind === "fill" ? layer : null;
  });
  /** The dock below Layers: the panel unfolded, if any, and its height. */
  let dock = $state(loadDock());
  /** Its panels, as its tabs and the Window menu list them. */
  const dockPanels = $derived<{ id: DockPanel; icon: IconName; label: string }[]>([
    { id: "properties", icon: "sliders", label: t("properties.title") },
    { id: "selections", icon: "marquee", label: t("selections.title") },
  ]);
  // An adjustment or fill layer just selected shows its properties, as in Photoshop.
  let shownProperties: number | null = null;
  $effect(() => {
    const id = selectedProperties?.id ?? null;
    if (id !== null && id !== shownProperties) dock = { ...dock, open: "properties" };
    shownProperties = id;
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
    upsertTab(tabs, view);
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

  /** Close tab `id`, unsaved changes asked about first. False when the user kept it open. */
  async function closeTab(id: number): Promise<boolean> {
    if (closing.has(id) || !tabs.some((d) => d.id === id)) return true;
    closing.add(id);
    try {
      if (!(await confirmClose(id))) return false;
      await engine.closeDocument(id);
    } finally {
      closing.delete(id);
    }
    // Look the tab up again: other tabs may have been closed while waiting.
    const index = tabs.findIndex((d) => d.id === id);
    if (index < 0) return true;
    tabs.splice(index, 1);
    if (activeId === id) {
      // Like browsers: the tab to the right, else the one to the left.
      activeId = (tabs[index] ?? tabs[index - 1])?.id ?? null;
      frame = null;
    }
    return true;
  }

  /**
   * File > Close All (Alt+Ctrl+W): every tab, from the active one, each unsaved one asked
   * about; Cancel stops there, as in Photoshop.
   */
  let closingAll = false;

  async function closeAll() {
    if (closingAll) return;
    closingAll = true;
    try {
      while (tabs.length > 0) {
        const id = activeId ?? tabs[0].id;
        if (!(await closeTab(id))) return;
      }
    } catch (e) {
      showError(String(e));
    } finally {
      closingAll = false;
    }
  }

  /** File > Document Info, while shown. */
  let documentInfo = $state<DocumentInfo | null>(null);

  async function showDocumentInfo() {
    const doc = active;
    if (!doc) return;
    try {
      documentInfo = await engine.documentInfo(doc.id);
    } catch (e) {
      showError(String(e));
    }
  }

  /** File > Import from Device (Windows): a scanner's or a camera's image in a new tab. */
  let acquiring = false;

  async function acquireImage() {
    if (acquiring) return;
    acquiring = true;
    try {
      const outcome = await engine.acquireImage();
      if (outcome === "noDevice") showError(t("acquire.noDevice"));
      // The new tab is untitled: its temporary file is no name.
      if (outcome === "opened") await refreshTabs();
    } catch (e) {
      showError(t("acquire.failed", { detail: String(e) }));
    } finally {
      acquiring = false;
    }
  }

  /** File > Print (Ctrl+P): the print settings of this document, while shown. */
  let printDialog = $state<number | null>(null);

  function printDocument() {
    if (active) printDialog = active.id;
  }

  /** File > New asks for the size and the background first (Photoshop's New dialog). */
  let newDialog = $state(false);

  /** The size of what the clipboard holds, offered by File > New (Photoshop's Clipboard preset). */
  let newClipboard = $state<[number, number] | null>(null);

  async function newDocument() {
    newClipboard = await engine.clipboardSize().catch(() => null);
    newDialog = true;
  }

  async function createDocument(settings: NewDocumentSettings) {
    newDialog = false;
    const transparent = settings.background === "transparent";
    const hex =
      settings.background === "background"
        ? colors.background
        : settings.background === "black"
          ? "#000000"
          : "#ffffff";
    try {
      const doc = await engine.newDocument({
        name: settings.name,
        width: settings.width,
        height: settings.height,
        background: transparent ? null : hexToSrgb(hex),
        // Photoshop's names: a transparent document starts with Layer 1.
        layerName: transparent
          ? t("layers.defaultLayerName", { n: 1 })
          : t("newDocument.backgroundLayer"),
        resolution: settings.resolution,
      });
      upsert(doc);
      activate(doc.id);
    } catch (e) {
      showError(String(e));
    }
  }

  function cycleTabs(step: number) {
    const id = cycled(
      tabs.map((d) => d.id),
      activeId,
      step,
    );
    if (id !== null) activate(id);
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
    const middles = [...tabbar.querySelectorAll<HTMLElement>(".tab[data-id]")].map((el) => {
      const rect = el.getBoundingClientRect();
      return rect.left + rect.width / 2;
    });
    return tabSlot(
      middles,
      x,
      tabs.findIndex((d) => d.id === draggedId),
    );
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
    const index = moveTabTo(tabs, from, slot);
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
    selectTool(slotTool(slot, toolChoices[slot.key], next));
  }

  // Selections (ADR 0024): the marquees draw shapes that the engine turns into masks.
  let selectionMode = $state<SelectionMode>("replace");
  let feather = $state(0);
  let antiAlias = $state(true);
  /** The Magic Wand's options (Photoshop's defaults). */
  let wand = $state({ tolerance: 32, contiguous: true, sampleAll: false });

  // Painting (ADR 0027): the Brush and the Eraser keep their own options, as in Photoshop.
  // Size in document pixels; hardness, opacity and flow as shares in [0, 1].
  let brushOptions = $state({
    size: 30,
    hardness: 1,
    opacity: 1,
    flow: 1,
    pressureSize: true,
    pressureOpacity: false,
  });
  let eraserOptions = $state({
    size: 50,
    hardness: 1,
    opacity: 1,
    flow: 1,
    pressureSize: true,
    pressureOpacity: false,
  });
  /** The foreground (the Brush's) and background colors, `#rrggbb` sRGB. */
  let colors = $state({ foreground: "#000000", background: "#ffffff" });
  /**
   * The grays masks are painted with (Quick Mask, a layer's mask), as Photoshop's: their own
   * pair, so that the drawing colors are as they were afterwards.
   */
  let maskColors = $state<ColorPair>({ ...MASK_COLORS });
  /** Quick Mask's overlay opacity, percent: an app preference. */
  let quickMaskOpacity = $state(loadQuickMaskOpacity());
  /** Painting goes to a mask (Quick Mask's, or the active layer's): colors are grays. */
  function paintsGray(): boolean {
    return (active?.quickMask ?? false) || (layersPanel?.paintsMask() ?? false);
  }
  /** The colors the swatches show and the tools paint: the mask grays when painting a mask. */
  function paintColors(): ColorPair {
    return paintsGray() ? maskColors : colors;
  }
  function setPaintColors(pair: ColorPair) {
    if (paintsGray()) maskColors = pair;
    else colors = pair;
  }
  // The opacity set: saved, and sent to the document in Quick Mask unless it shows it already
  // (its answer comes back here).
  $effect(() => saveQuickMaskOpacity(quickMaskOpacity));
  $effect(() => {
    const opacity = Math.round(quickMaskOpacity);
    const doc = active;
    if (doc?.quickMask && doc.quickMaskOpacity !== opacity) {
      untrack(() => void sync(engine.setQuickMask(doc.id, true, opacity)));
    }
  });

  /** The width of the panels on the right, as the user left it; narrower if the window is. */
  let panelWidth = $state(loadPanelWidth());
  let windowWidth = $state(window.innerWidth);
  const shownPanelWidth = $derived(clampPanelWidth(panelWidth, windowWidth));
  /** The color the color picker is open for. */
  let colorPicker = $state<"foreground" | "background" | null>(null);

  /** The image point under a window point, if it lies on the active document's canvas. */
  function canvasPointAt(clientX: number, clientY: number): [number, number] | null {
    const doc = active;
    const point = viewport?.documentPointAt(clientX, clientY) ?? null;
    if (!doc || !point || outsideCanvas(doc, point[0], point[1])) return null;
    return point;
  }

  /** The color picker's eyedropper: the color shown in the active document. */
  const pickerSample = {
    probe: (clientX: number, clientY: number) => canvasPointAt(clientX, clientY) !== null,
    at: async (clientX: number, clientY: number) => {
      const doc = active;
      const point = canvasPointAt(clientX, clientY);
      if (!doc || !point) return null;
      const shown = await engine.sampleColor(doc.id, point[0], point[1]);
      return shown ? (shown.map((v) => v / 255) as [number, number, number]) : null;
    },
    patch: async (clientX: number, clientY: number, radius: number) => {
      const point = canvasPointAt(clientX, clientY);
      return point ? loupePatch(point[0], point[1], radius) : null;
    },
  };

  /** The eyedropper's loupe: the pixels shown around a document point; none off the canvas. */
  async function loupePatch(x: number, y: number, radius: number) {
    const doc = active;
    if (!doc || outsideCanvas(doc, x, y)) return null;
    return engine.samplePatch(doc.id, x, y, radius);
  }
  /** The stroke being sent: samples wait while a batch is in flight (none is ever dropped). */
  let paintRun: {
    id: number;
    documentId: number;
    target: PaintTarget;
    layerId: number;
    brush: BrushRequest;
    color: [number, number, number] | null;
    restore: boolean;
    sending: boolean;
    waiting: [number, number, number][];
    ended: boolean;
    failed: boolean;
  } | null = null;
  let nextPaintStroke = 1;
  /** Where the last stroke ended, by document: Shift+click paints a line from there. */
  const lastPaintPoint = new Map<number, [number, number, number]>();

  /** A stroke's samples from the Brush or Eraser tool (see `PaintTool`). */
  function paintStroke(
    samples: [number, number, number][],
    phase: "start" | "line" | "move" | "end",
  ) {
    const doc = active;
    if (!doc) return;
    if (phase === "start" || phase === "line") {
      commitTransform();
      const layer = layersPanel?.selectedLayer() ?? null;
      paintRun = null;
      // Quick Mask paints the selection, whatever the layer; else the active layer's mask
      // when it is the target, or its pixels.
      const target: PaintTarget = doc.quickMask
        ? "quickMask"
        : layersPanel?.paintsMask()
          ? "mask"
          : "layer";
      // As in Photoshop: only pixels or a mask can be painted, and not while hidden.
      if (target !== "quickMask") {
        if (!layer || (target === "layer" && layer.kind !== "raster")) {
          showError(t("paint.needRaster"));
          return;
        }
        if (!layer.visible) {
          showError(t("paint.hidden"));
          return;
        }
      }
      if (tool === "restoreEraser" && target !== "layer") {
        showError(t("paint.restoreLayersOnly"));
        return;
      }
      const options = isEraser(tool) ? eraserOptions : brushOptions;
      paintRun = {
        id: nextPaintStroke++,
        documentId: doc.id,
        target,
        layerId: layer?.id ?? 0,
        brush: { ...options, spacing: 0.25 },
        // On a mask the Eraser paints the background color, as Photoshop's (white at first:
        // it selects or shows); on pixels it lowers alpha.
        color:
          isEraser(tool) && target === "layer"
            ? null
            : hexToSrgb(paintColors()[isEraser(tool) ? "background" : "foreground"]),
        restore: tool === "restoreEraser",
        sending: false,
        waiting: [],
        ended: false,
        failed: false,
      };
      const last = lastPaintPoint.get(doc.id);
      if (phase === "line" && last) samples = [last, ...samples];
    }
    const run = paintRun;
    if (!run || run.ended || run.documentId !== doc.id) return;
    run.waiting.push(...samples);
    if (phase === "end") {
      run.ended = true;
      const last = run.waiting.at(-1);
      if (last) lastPaintPoint.set(run.documentId, last);
    }
    sendPaint(run);
  }

  // --- Edit > Fill and Stroke, Delete with a selection: paint (ADR 0027, 0029) -----------------

  /**
   * What Fill, Stroke and Delete paint: the layer's pixels, its mask when it is the target, or
   * Quick Mask's image while it is on (`target`).
   */
  type PaintedLayer = { documentId: number; layerId: number; mask: boolean; target?: PaintTarget };

  /** The active layer as a `PaintedLayer`; `null`, with a notice, when it has no pixels. */
  function paintedLayer(): PaintedLayer | null {
    const doc = active;
    if (!doc) return null;
    commitTransform();
    if (doc.quickMask) return { documentId: doc.id, layerId: 0, mask: true, target: "quickMask" };
    const layer = layersPanel?.selectedLayer() ?? null;
    const mask = layersPanel?.paintsMask() ?? false;
    if (!layer || (!mask && layer.kind !== "raster")) {
      showError(t("paint.needRaster"));
      return null;
    }
    return { documentId: doc.id, layerId: layer.id, mask };
  }

  /**
   * `target` painted with `hex` (erased when `null`) at `opacity`: in the selection, along its
   * outline with `stroke`, or everywhere without a selection. One undo entry.
   */
  function paintPixels(
    target: PaintedLayer,
    hex: string | null,
    opacity = 1,
    stroke: StrokeRequest | null = null,
  ) {
    const kind = target.target ?? (target.mask ? "mask" : "layer");
    // Erasing a mask (Delete, Cut) paints the background color, as Photoshop does.
    const erased = hex === null && kind !== "layer" ? paintColors().background : hex;
    const color = erased === null ? null : hexToSrgb(erased);
    void sync(engine.fill(target.documentId, target.layerId, kind, color, opacity, stroke));
  }

  /** Delete with a selection: the selected pixels erased, as Photoshop's Clear. */
  function clearPixels() {
    const target = paintedLayer();
    if (target) paintPixels(target, null);
  }

  /** Photoshop's fixed fill colors: Black, 50% Gray and White (sRGB). */
  const FILL_COLORS = { black: "#000000", gray: "#808080", white: "#ffffff" };

  /**
   * Image > Adjustments is open (ADR 0029): on document `documentId`, for layers `ids`; the
   * canvas previews it with the adjustment layers `previews` (a live gesture, cancelled when
   * the dialog closes), hidden from the layers panel; `preview` shows them.
   */
  let adjustDialog = $state<{
    documentId: number;
    adjustment: AdjustmentId;
    ids: number[];
    previews: number[];
    preview: boolean;
    /** The settings chosen last (null: the neutral ones the preview started with). */
    values: number[] | null;
    curves: number[][][] | null;
    /** Gradient Map's stops chosen last (null: the preview's). */
    gradient: number[][] | null;
  } | null>(null);

  /**
   * The layers of document `documentId` before Image > Adjustments added its previews: the
   * others are the previews, hidden from the layers panel (never selected, so the Properties
   * panel does not show them) from the moment they arrive until they are gone.
   */
  let previewBase = $state<{ documentId: number; before: Set<number> } | null>(null);
  let previewHidden = $derived.by(() => {
    const base = previewBase;
    if (!base || active?.id !== base.documentId) return [];
    return walk(active.layers)
      .map((l) => l.id)
      .filter((id) => !base.before.has(id));
  });

  /** The previews are gone (the gesture cancelled or replaced): nothing to hide any more. */
  function endPreview(done: Promise<unknown>) {
    void done.finally(() => (previewBase = null));
  }

  /** The settings the adjustment dialog shows: those of its first preview layer. */
  let adjustShown = $derived.by(() => {
    const dialog = adjustDialog;
    if (!dialog) return null;
    const doc = tabs.find((d) => d.id === dialog.documentId);
    const first = dialog.previews[0];
    return doc && first !== undefined ? (findLayer(doc.layers, first)?.adjustment ?? null) : null;
  });

  /**
   * The layers Image > Adjustments applies to: every pixel layer shown, its groups shown too
   * (maintainer's choice: the whole visible image, not the selected layers).
   */
  function adjustTargets(): number[] {
    return active ? visibleRasters(active.layers) : [];
  }

  /**
   * Image > Auto Tone, Auto Contrast, Auto Color: Levels computed by the engine from the visible
   * image, applied as Image > Adjustments are (nothing done when there is nothing to change).
   */
  function autoLevels(correction: AutoCorrection) {
    const request = active && autoLevelsEdit(adjustTargets(), correction);
    if (active && request) void edit(active.id, request);
  }

  /** Image > Adjustments > `adjustment`: Invert at once, the others through their dialog. */
  async function openAdjust(adjustment: AdjustmentId) {
    const doc = active;
    const ids = adjustTargets();
    if (!doc || ids.length === 0 || adjustDialog) return;
    if (adjustment === "invert") {
      void edit(doc.id, { kind: "applyEffect", ids, adjustment, values: [] });
      return;
    }
    const before = new Set(walk(doc.layers).map((l) => l.id));
    previewBase = { documentId: doc.id, before };
    await live(doc.id, { kind: "previewEffect", ids, adjustment });
    const after = tabs.find((d) => d.id === doc.id);
    const previews = after
      ? walk(after.layers)
          .map((l) => l.id)
          .filter((id) => !before.has(id))
      : [];
    if (previews.length === 0) {
      endPreview(cancelGesture(doc.id));
      return;
    }
    adjustDialog = {
      documentId: doc.id,
      adjustment,
      ids,
      previews,
      preview: true,
      values: null,
      curves: null,
      gradient: null,
    };
  }

  /**
   * The preview showing the dialog's settings, whole each time: live edits sent while the
   * engine is busy merge into the last one, which must then say everything.
   */
  function showAdjust(dialog: NonNullable<typeof adjustDialog>) {
    const edits: EditRequest[] = dialog.previews.flatMap((id): EditRequest[] => [
      { kind: "setLayerVisible", id, visible: dialog.preview },
      ...(dialog.values || dialog.curves || dialog.gradient
        ? [
            {
              kind: "setAdjustment" as const,
              id,
              adjustment: dialog.adjustment,
              values: dialog.values ?? [],
              curves: dialog.curves ?? undefined,
              gradient: dialog.gradient ?? adjustShown?.gradient ?? undefined,
            },
          ]
        : []),
    ]);
    void live(dialog.documentId, { kind: "batch", edits });
  }

  /** The dialog's settings changed. */
  function adjustLive(values: number[], curves?: number[][][], gradient?: number[][]) {
    if (!adjustDialog) return;
    adjustDialog = {
      ...adjustDialog,
      values,
      curves: curves ?? null,
      gradient: gradient ?? adjustDialog.gradient,
    };
    showAdjust(adjustDialog);
  }

  function adjustPreview(preview: boolean) {
    if (!adjustDialog) return;
    adjustDialog = { ...adjustDialog, preview };
    showAdjust(adjustDialog);
  }

  /**
   * OK: the adjustment applied to the layers' stacks (one undo entry) in place of the preview,
   * in one go: the canvas goes from the preview straight to the result.
   */
  function applyAdjust() {
    const dialog = adjustDialog;
    // The settings chosen last: the engine may not show them yet.
    const shown = adjustShown;
    adjustDialog = null;
    if (!dialog) return;
    const values = dialog.values ?? shown?.values;
    if (!values) {
      endPreview(cancelGesture(dialog.documentId));
      return;
    }
    endPreview(
      sync(
        engine.replaceGesture(dialog.documentId, {
          kind: "applyEffect",
          ids: dialog.ids,
          adjustment: dialog.adjustment,
          values,
          curves: dialog.curves ?? shown?.curves ?? undefined,
          gradient: dialog.gradient ?? shown?.gradient ?? undefined,
        }),
      ),
    );
  }

  function cancelAdjust() {
    const dialog = adjustDialog;
    adjustDialog = null;
    if (dialog) endPreview(cancelGesture(dialog.documentId));
  }

  /**
   * An entry of a layer's stack edited again (ADR 0034): the adjustment dialog on entry `index`
   * of layer `layerId` (on its last application, for an entry read from an older file). The canvas follows the settings live (a gesture), the
   * entry hidden while Preview is off; OK makes it one undo entry with the eye the entry had,
   * Cancel takes it all back.
   */
  let entryDialog = $state<{
    documentId: number;
    layerId: number;
    index: number;
    step: number;
    preview: boolean;
    /** The entry's eye when the dialog opened. */
    hidden: boolean;
    original: AdjustmentSettings[];
    settings: AdjustmentSettings[];
  } | null>(null);

  /** The settings the entry dialog shows: its step's, as the engine has them. */
  let entryShown = $derived.by(() => {
    const dialog = entryDialog;
    const doc = dialog && tabs.find((d) => d.id === dialog.documentId);
    const layer = doc && findLayer(doc.layers, dialog.layerId);
    return (dialog && layer?.entries[dialog.index]?.steps[dialog.step]) ?? null;
  });

  /** Edit entry `index` of `layer`'s stack again, its newest application first. */
  function openEntry(layer: LayerView, index: number) {
    const doc = active;
    const entry = layer.entries[index];
    if (!doc || !entry || !editableEntry(entry) || adjustDialog || entryDialog || filterDialog) {
      return;
    }
    if (entry.kind === "filter" && entry.filter) {
      const settings = filterSteps(entry);
      const step = settings.length - 1;
      filterDialog = {
        documentId: doc.id,
        layerId: layer.id,
        filter: entry.filter,
        preview: true,
        values: settings[step].values,
        entry: { index, step, hidden: entry.hidden, original: settings, settings },
      };
      // A hidden entry shows while it is edited.
      if (entry.hidden) showFilter(filterDialog);
      return;
    }
    const settings = stepSettings(entry);
    entryDialog = {
      documentId: doc.id,
      layerId: layer.id,
      index,
      step: settings.length - 1,
      preview: true,
      hidden: entry.hidden,
      original: settings,
      settings,
    };
    // A hidden entry shows while it is edited.
    if (entry.hidden) showEntry(entryDialog);
  }

  /** The canvas shows the dialog's settings, whole each time (live edits merge). */
  function showEntry(dialog: NonNullable<typeof entryDialog>) {
    void live(
      dialog.documentId,
      entryEdit(dialog.layerId, dialog.index, dialog.settings, !dialog.preview),
    );
  }

  function entryLive(change: SettingsChange) {
    if (!entryDialog) return;
    entryDialog = {
      ...entryDialog,
      settings: withStep(entryDialog.settings, entryDialog.step, change),
    };
    showEntry(entryDialog);
  }

  function entryPreview(preview: boolean) {
    if (!entryDialog) return;
    entryDialog = { ...entryDialog, preview };
    showEntry(entryDialog);
  }

  /** OK: the settings chosen, one undo entry (none when nothing changed). */
  function applyEntry() {
    const dialog = entryDialog;
    entryDialog = null;
    if (!dialog) return;
    if (sameSettings(dialog.settings, dialog.original)) {
      void cancelGesture(dialog.documentId);
      return;
    }
    void sync(
      engine.replaceGesture(
        dialog.documentId,
        entryEdit(dialog.layerId, dialog.index, dialog.settings, dialog.hidden),
      ),
    );
  }

  function cancelEntry() {
    const dialog = entryDialog;
    entryDialog = null;
    if (dialog) void cancelGesture(dialog.documentId);
  }

  /** The filter applied last (Filter > Repeat, Ctrl+F), for the session. */
  let lastFilter = $state<FilterSettings | null>(null);
  /** Each filter's settings used last, its dialog's start, for the session. */
  const filterValues: Partial<Record<FilterId, number[]>> = {};

  /**
   * A filter's dialog (ADR 0034): applying it to the active layer, or editing a filter entry of
   * a stack again (`entry`, step `step` of it). The canvas follows the settings live (a
   * gesture); OK makes it one undo entry, Cancel takes it all back.
   */
  let filterDialog = $state<{
    documentId: number;
    layerId: number;
    filter: FilterId;
    preview: boolean;
    /** The settings shown (of the step edited). */
    values: number[];
    entry: {
      index: number;
      step: number;
      /** The entry's eye when the dialog opened. */
      hidden: boolean;
      original: FilterSettings[];
      settings: FilterSettings[];
    } | null;
  } | null>(null);

  /** The layer Filter > … applies to: the active one, a pixel layer shown (ADR 0034). */
  function filterLayer(): LayerView | null {
    const doc = active;
    const layer = layersPanel?.selectedLayer() ?? null;
    return doc && filterable(layer, layersPanel?.paintsMask() ?? false, doc.quickMask)
      ? layer
      : null;
  }

  /** Filter > `filter`…: its dialog on the active layer, at `values`. */
  function openFilter(filter: FilterId, values = filterValues[filter] ?? FILTERS[filter].defaults) {
    commitTransform();
    const doc = active;
    const layer = filterLayer();
    if (!doc || !layer || filterDialog || adjustDialog || entryDialog) return;
    filterDialog = {
      documentId: doc.id,
      layerId: layer.id,
      filter,
      preview: true,
      values: [...values],
      entry: null,
    };
  }

  /**
   * The canvas shows the dialog's settings: a filter applied as a gesture replaced at each
   * change (nothing while Preview is off), or the entry edited live (hidden while it is off).
   */
  function showFilter(dialog: NonNullable<typeof filterDialog>) {
    if (dialog.entry) {
      const { index, settings } = dialog.entry;
      void live(
        dialog.documentId,
        filterEntryEdit(dialog.layerId, index, settings, !dialog.preview),
      );
    } else if (dialog.preview) {
      const request = applyFilterEdit(dialog.layerId, {
        filter: dialog.filter,
        values: dialog.values,
      });
      void sync(engine.performLive(dialog.documentId, request, true));
    } else {
      void cancelGesture(dialog.documentId);
    }
  }

  function filterLive(values: number[]) {
    const dialog = filterDialog;
    if (!dialog) return;
    const entry = dialog.entry && {
      ...dialog.entry,
      settings: dialog.entry.settings.map((s, i) =>
        i === dialog.entry?.step ? { ...s, values } : s,
      ),
    };
    filterDialog = { ...dialog, values, entry };
    showFilter(filterDialog);
  }

  function filterPreview(preview: boolean) {
    if (!filterDialog) return;
    filterDialog = { ...filterDialog, preview };
    showFilter(filterDialog);
  }

  /** OK: one undo entry (none when an entry is left as it was). */
  function applyFilterDialog(values: number[]) {
    const dialog = filterDialog;
    filterDialog = null;
    if (!dialog) return;
    if (!dialog.entry) {
      const settings = { filter: dialog.filter, values };
      filterValues[dialog.filter] = values;
      lastFilter = settings;
      void sync(
        engine.replaceGesture(dialog.documentId, applyFilterEdit(dialog.layerId, settings)),
      );
      return;
    }
    const { index, hidden, original, step } = dialog.entry;
    const settings = dialog.entry.settings.map((s, i) => (i === step ? { ...s, values } : s));
    if (JSON.stringify(settings) === JSON.stringify(original)) {
      void cancelGesture(dialog.documentId);
      return;
    }
    void sync(
      engine.replaceGesture(
        dialog.documentId,
        filterEntryEdit(dialog.layerId, index, settings, hidden),
      ),
    );
  }

  function cancelFilter() {
    const dialog = filterDialog;
    filterDialog = null;
    if (dialog) void cancelGesture(dialog.documentId);
  }

  /** Filter > Repeat (Ctrl+F): the filter applied last, as it was, on the active layer. */
  function repeatFilter() {
    commitTransform();
    const doc = active;
    const layer = filterLayer();
    if (doc && layer && lastFilter) void edit(doc.id, applyFilterEdit(layer.id, lastFilter));
  }

  /** Edit > Fill is open, for this layer, with Color…'s color; hidden while it is picked. */
  let fillDialog = $state<(PaintedLayer & { color: string; picking: boolean }) | null>(null);
  /** Edit > Stroke is open, for this layer, with its color; hidden while the color is picked. */
  let strokeDialog = $state<(PaintedLayer & { color: string; picking: boolean }) | null>(null);
  /** A color picked for Fill or Stroke. */
  let pickColor = $state<{
    title: string;
    color: string;
    apply: (hex: string) => void;
    close?: () => void;
  } | null>(null);

  /**
   * Layer > Layer Style (ADR 0032) under way: the layer, the style as the dialog shows it (each
   * change sent live, one undo entry once OK), the page shown, and whether the color picker
   * replaces the dialog for a moment.
   */
  let styleDialog = $state<{
    documentId: number;
    layerId: number;
    style: LayerStyle | null;
    page: StylePage;
    picking: boolean;
  } | null>(null);

  /** Layer Style on `page` for `layer` (a pixel or fill layer); an effect's page turns it on. */
  function openStyle(layer: LayerView, page: StylePage) {
    const doc = active;
    if (!doc || layer.kind === "adjustment") return;
    let style = layer.style ?? null;
    if (page !== "blending" && !style?.[page]?.enabled) {
      style = withEffect(style, page, true);
      live(doc.id, styleEdit(layer.id, style));
    }
    styleDialog = { documentId: doc.id, layerId: layer.id, style, page, picking: false };
  }

  function changeStyle(style: LayerStyle) {
    if (!styleDialog) return;
    styleDialog.style = style;
    live(styleDialog.documentId, styleEdit(styleDialog.layerId, style));
  }

  function closeStyle(keep: boolean) {
    const dialog = styleDialog;
    styleDialog = null;
    if (dialog) void (keep ? endGesture : cancelGesture)(dialog.documentId);
  }

  /** An effect's color, in the picker; then the dialog again. */
  function pickStyleColor(effect: EffectId) {
    const dialog = styleDialog;
    const current = dialog?.style?.[effect];
    if (!dialog || !current) return;
    dialog.picking = true;
    pickColor = {
      title: t("style.color"),
      color: srgbToHex(current.color),
      apply: (hex) => {
        if (!styleDialog?.style?.[effect]) return;
        const style = structuredClone($state.snapshot(styleDialog.style));
        style[effect]!.color = hexToSrgb(hex);
        changeStyle(style);
        styleDialog.picking = false;
      },
      close: () => {
        if (styleDialog) styleDialog.picking = false;
      },
    };
  }

  /** Layer > Layer Style > Clear Layer Style: the selected layers' styles removed. */
  function clearStyles() {
    const doc = active;
    const edits = (layersPanel?.selectedLayers() ?? [])
      .filter((l) => l.style)
      .map((l) => styleEdit(l.id, null));
    if (doc && edits.length > 0) {
      void edit(doc.id, edits.length === 1 ? edits[0] : { kind: "batch", edits });
    }
  }

  /** A fill layer's color, chosen in the color picker (one undo entry). */
  function pickFillLayerColor(layer: LayerView) {
    const doc = active;
    if (!doc) return;
    pickColor = {
      title: t("colorPicker.fill"),
      color: fillHex(layer),
      apply: (hex) => {
        const request = fillColorEdit(layer, hex);
        if (request) void edit(doc.id, request);
      },
    };
  }

  function applyFill({ contents, opacity }: FillSettings) {
    const target = fillDialog;
    fillDialog = null;
    if (!target) return;
    const hex =
      contents === "color"
        ? target.color
        : contents === "foreground" || contents === "background"
          ? paintColors()[contents]
          : FILL_COLORS[contents];
    paintPixels(target, hex, opacity);
  }

  function openFill() {
    const target = paintedLayer();
    if (target) fillDialog = { ...target, color: paintColors().foreground, picking: false };
  }

  /** Fill's Color…: the picker, then the Fill dialog again with the color chosen. */
  function pickFillColor() {
    const dialog = fillDialog;
    if (!dialog) return;
    fillDialog = { ...dialog, picking: true };
    pickColor = {
      title: t("fillChoice.colorTitle"),
      color: dialog.color,
      apply: (hex) => {
        if (fillDialog) fillDialog = { ...fillDialog, color: hex, picking: false };
      },
      close: () => {
        if (fillDialog) fillDialog = { ...fillDialog, picking: false };
      },
    };
  }

  function openStroke() {
    const target = paintedLayer();
    if (target) strokeDialog = { ...target, color: paintColors().foreground, picking: false };
  }

  function applyStroke({ width, location, opacity }: StrokeSettings) {
    const target = strokeDialog;
    strokeDialog = null;
    if (target) paintPixels(target, target.color, opacity, { width, location });
  }

  function pickStrokeColor() {
    const dialog = strokeDialog;
    if (!dialog) return;
    strokeDialog = { ...dialog, picking: true };
    pickColor = {
      title: t("stroke.colorTitle"),
      color: dialog.color,
      apply: (hex) => {
        if (strokeDialog) strokeDialog = { ...strokeDialog, color: hex, picking: false };
      },
      close: () => {
        if (strokeDialog) strokeDialog = { ...strokeDialog, picking: false };
      },
    };
  }

  /** Sends the samples waiting, or leaves them for when the batch in flight returns. */
  function sendPaint(run: NonNullable<typeof paintRun>) {
    if (run.sending || run.failed || (run.waiting.length === 0 && !run.ended)) return;
    const samples = run.waiting;
    run.waiting = [];
    const end = run.ended;
    run.sending = true;
    engine
      .paintStroke(run.documentId, {
        stroke: run.id,
        target: run.target,
        layerId: run.layerId,
        brush: run.brush,
        color: run.color,
        restore: run.restore,
        samples,
        end,
      })
      .then((view) => {
        if (view) upsert(view);
        else if (active?.id === run.documentId) viewport?.redraw();
      })
      .catch((e) => {
        run.failed = true;
        if (e !== DOCUMENT_CLOSED) showError(String(e));
      })
      .finally(() => {
        run.sending = false;
        if (end || run.failed) {
          if (paintRun === run) paintRun = null;
        } else sendPaint(run);
      });
  }

  function magicWand(x: number, y: number, mode: SelectionMode | null) {
    const doc = active;
    if (!doc) return;
    commitTransform();
    // A click outside the image deselects, as with the other selection tools; with keys
    // (adding, subtracting), it does nothing.
    if (outsideCanvas(doc, x, y)) {
      if (mode === null && doc.selectionKey != null) selectionCommand(engine.deselect);
      return;
    }
    // The active layer, unless every layer is sampled (or none is active).
    const layer = wand.sampleAll ? null : (layersPanel?.selectedLayer()?.id ?? null);
    const options = { tolerance: wand.tolerance, contiguous: wand.contiguous, antiAlias };
    // Seconds on a large document: its progress shows, and Esc cancels it.
    void runAi("wand.task", (task) =>
      engine.magicWand(doc.id, { x, y }, options, layer, mode ?? selectionMode, task),
    );
  }

  // AI selection (ADR 0025), with SAM 2.1. Object Selection: the object under the pointer
  // lights up, a click or a box selects it. Quick Selection works on colors (ADR 0026).
  /**
   * The options of Object and Quick Selection: Quick Selection's brush, every layer or the active
   * one, and whether Object Selection refines edges at full resolution (ViTMatte).
   */
  let quick = $state({ size: 30, sampleAll: false, objectRefine: false });
  /**
   * Select > Subject refines its edges at full resolution (BiRefNet's soft mask, then ViTMatte),
   * except on the processor (Linux), where it takes about a second per window.
   */
  let subjectRefine = true;
  // On the processor, Select Subject does not refine (see above).
  void engine.aiRuntime().then((runtime) => {
    if (runtime === "cpu") subjectRefine = false;
  });
  let aiBusy = $state(false);
  /** No hovering until a click asks again: the components are missing or AI cannot start. */
  let aiHoverBlocked = false;
  /** A Quick Selection stroke is being turned into a selection. */
  let quickBusy = $state(false);
  /**
   * The Quick Selection stroke under way: one request at a time, the latest stroke so far
   * waiting meanwhile (a stroke's end is never dropped).
   */
  let quickRun: {
    id: number;
    documentId: number;
    mode: SelectionMode;
    region: [number, number, number, number];
    layer: number | null;
    /** A request is in flight. */
    sending: boolean;
    waiting: { points: [number, number][]; live: boolean } | null;
    /** Some request was sent: the document shows the stroke. */
    shown: boolean;
    cancelled: boolean;
  } | null = null;
  let nextQuickStroke = 1;
  /** The components to download before AI can run, and what to do once they are there. */
  let aiDownload = $state<{ components: AiComponent[]; then: () => void } | null>(null);

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
   * The AI request under way, shown with its progress (after a moment, so that quick ones do
   * not flash) and cancelled by Esc or its button: it stops at its next step.
   */
  let aiTask = $state<{
    id: number;
    label: string;
    done: number;
    total: number;
    shown: boolean;
  } | null>(null);
  let nextAiTask = 1;

  function cancelAiTask() {
    const task = aiTask;
    if (!task) return;
    aiTask = null;
    void engine.aiCancel(task.id).catch(() => undefined);
  }

  /**
   * Runs an AI selection, its progress shown under `label`: on first use, asks to download what
   * it needs, then runs it again; reports other failures (not a cancellation). Resolves to the
   * document view, or null when nothing was applied.
   */
  async function runAi(
    label: MessageKey,
    task: (id: number) => Promise<DocumentView>,
  ): Promise<DocumentView | null> {
    aiBusy = true;
    const id = nextAiTask++;
    aiTask = { id, label: t(label), done: 0, total: 0, shown: false };
    setTimeout(() => {
      if (aiTask?.id === id) aiTask.shown = true;
    }, 250);
    try {
      const view = await task(id);
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
              then: () => void runAi(label, task).then(resolve),
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
      if (aiTask?.id === id) aiTask = null;
    }
  }

  function onAiTaskProgress(progress: AiTaskProgress) {
    const task = aiTask;
    if (task?.id !== progress.task) return;
    task.done = progress.done;
    task.total = progress.total;
    // Refine Edge's windows take most of the time: say so.
    if (progress.stage === "refine") task.label = t("ai.task.refine");
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
    void runAi("ai.task.subject", (task) =>
      engine.aiSelectSubject(doc.id, aiLayer(), "replace", subjectRefine, task),
    );
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
    // A click outside the image deselects (with keys, it does nothing).
    if (point && outsideCanvas(doc, point[0], point[1])) {
      if (keyMode === null && doc.selectionKey != null) selectionCommand(engine.deselect);
      return;
    }
    const request = {
      point,
      box,
      region: aiRegion(doc, view),
      layerId: aiLayer(),
      mode: keyMode ?? selectionMode,
      refine: quick.objectRefine,
    };
    void runAi("ai.task.object", (task) => engine.aiObjectSelect(doc.id, { ...request, task }));
  }

  /**
   * Quick Selection (ADR 0026): the region of similar colors a stroke paints over, shown while
   * it is painted (each request replaces the previous one's selection; the stroke's end makes
   * one undo entry). As in Photoshop, the first stroke of a new selection switches the tool to
   * adding, a click outside the image deselects, and Esc drops the stroke under way.
   */
  function quickStroke(
    stroke: [number, number][],
    keyMode: SelectionMode | null,
    view: [number, number, number, number],
    phase: "move" | "end" | "cancel",
  ) {
    const doc = active;
    if (!doc) return;
    let run = quickRun;
    if (phase === "cancel") {
      if (run) {
        run.cancelled = true;
        run.waiting = null;
        if (!run.sending) finishQuick(run);
      }
      return;
    }
    if (!run || run.documentId !== doc.id) {
      commitTransform();
      const chosen = keyMode ?? selectionMode;
      run = quickRun = {
        id: nextQuickStroke++,
        documentId: doc.id,
        mode: chosen === "intersect" ? "replace" : chosen,
        region: aiRegion(doc, view),
        layer: aiLayer(),
        sending: false,
        waiting: null,
        shown: false,
        cancelled: false,
      };
    }
    const inside = stroke.filter(([x, y]) => !outsideCanvas(doc, x, y));
    if (inside.length === 0) {
      if (phase === "end") {
        // A click outside the image deselects (with keys, it does nothing).
        if (!run.shown && stroke.length === 1 && keyMode === null && doc.selectionKey != null) {
          selectionCommand(engine.deselect);
        }
        if (!run.shown) quickRun = null;
        else sendQuick(run, { points: stroke, live: false });
      }
      return;
    }
    if (phase === "end" && run.mode === "replace" && keyMode === null) selectionMode = "add";
    sendQuick(run, { points: stroke, live: phase !== "end" });
  }

  /** Sends the stroke so far, or keeps it for when the request in flight returns. */
  function sendQuick(
    run: NonNullable<typeof quickRun>,
    next: { points: [number, number][]; live: boolean },
  ) {
    if (run.cancelled) return;
    if (run.sending) {
      // The latest stroke so far replaces a waiting one; never a waiting end.
      if (!run.waiting || run.waiting.live) run.waiting = next;
      return;
    }
    run.sending = true;
    run.shown = true;
    quickBusy = true;
    void sync(
      engine.quickSelect(run.documentId, {
        stroke: run.id,
        points: next.points,
        radius: quick.size / 2,
        region: run.region,
        layerId: run.layer,
        mode: run.mode,
        live: next.live,
      }),
    ).finally(() => {
      run.sending = false;
      const waiting = run.waiting;
      run.waiting = null;
      if (run.cancelled) finishQuick(run);
      else if (waiting) sendQuick(run, waiting);
      else if (!next.live) finishQuick(run);
    });
  }

  /** The stroke is over: dropped (its selection reverted) if it was cancelled. */
  function finishQuick(run: NonNullable<typeof quickRun>) {
    if (quickRun === run) quickRun = null;
    quickBusy = false;
    if (run.cancelled && run.shown) void cancelGesture(run.documentId);
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
    if (doc) selectionCommand((id) => engine.setQuickMask(id, !doc.quickMask, quickMaskOpacity));
  }

  // Select > Modify: a dialog for the amount, remembered per change for the session.
  let modifyDialog = $state<{ kind: SelectionModify; document: number } | null>(null);
  let modifyAmounts = $state<Record<SelectionModify, number>>({
    border: 10,
    smooth: 5,
    expand: 10,
    contract: 10,
    feather: 10,
  });

  function openModify(kind: SelectionModify) {
    commitTransform();
    if (active?.selectionKey != null) modifyDialog = { kind, document: active.id };
  }

  /**
   * Select > Modify's amount shown live while the dialog is open: one request at a time, the
   * latest amount wins; each replaces the previous one in the engine.
   */
  const modifyPreview = latestWins((amount: number) => {
    const dialog = modifyDialog;
    if (!dialog) return Promise.resolve();
    return sync(engine.modifySelection(dialog.document, dialog.kind, amount, true));
  });

  /** Cancel: the selection as it was before the dialog. */
  function closeModify() {
    const dialog = modifyDialog;
    modifyDialog = null;
    modifyPreview.drop();
    if (dialog) void cancelGesture(dialog.document);
  }

  function applyModify(amount: number) {
    const dialog = modifyDialog;
    modifyDialog = null;
    modifyPreview.drop();
    if (!dialog) return;
    modifyAmounts[dialog.kind] = amount;
    void sync(engine.modifySelection(dialog.document, dialog.kind, amount));
  }

  // Select > Select and Mask: a panel beside the image refining the selection it opened on.
  let refining = $state<{ document: number } | null>(null);
  let refineSettings = $state<RefineSettings>(structuredClone(DEFAULT_REFINE));

  async function openSelectAndMask() {
    const doc = active;
    commitTransform();
    if (!doc || doc.selectionKey == null || refining) return;
    try {
      upsert(await engine.refineOpen(doc.id));
    } catch (e) {
      showError(String(e));
      return;
    }
    if (activeId === doc.id) refining = { document: doc.id };
  }

  /** The edge settings shown live: the latest wins. */
  const refinePreview = latestWins((edges: RefineSettings["edges"]) => {
    const session = refining;
    if (!session) return Promise.resolve();
    return sync(engine.refinePreview(session.document, edges, true));
  });

  function refineView(view: RefineSettings["view"]) {
    if (refining) void sync(engine.refineView(refining.document, view));
  }

  /** Edge detection on the base (ViTMatte), then the settings shown on it again. */
  function refineDetect(radius: number) {
    const session = refining;
    if (!session) return;
    const edges = $state.snapshot(refineSettings.edges);
    void runAi("ai.task.refine", async (task) => {
      await engine.aiRefineBase(session.document, radius, aiLayer(), task);
      return engine.refinePreview(session.document, edges, true);
    });
  }

  /** A refine-edge brush stroke under way: its samples and whether it erases. */
  let refineStroke = $state<{ samples: [number, number, number][]; erase: boolean } | null>(null);

  /** Select and Mask's refine-edge brush: the stroke is sent on release, then edge detection. */
  function refineBrushStroke(
    samples: [number, number, number][],
    phase: "start" | "line" | "move" | "end",
    keys?: { altKey: boolean },
  ) {
    const session = refining;
    if (!session) return;
    if (phase === "start" || phase === "line") {
      // Alt does the other of Paint and Erase, as in Photoshop.
      refineStroke = { samples, erase: refineSettings.brush.erase !== (keys?.altKey ?? false) };
      return;
    }
    if (!refineStroke) return;
    refineStroke.samples.push(...samples);
    if (phase !== "end") return;
    const stroke = $state.snapshot(refineStroke);
    refineStroke = null;
    const { size } = refineSettings.brush;
    const radius = refineSettings.radius;
    const edges = $state.snapshot(refineSettings.edges);
    void runAi("ai.task.refine", async (task) => {
      await engine.refineBrush(session.document, stroke.samples, size, stroke.erase);
      await engine.aiRefineBase(session.document, radius, aiLayer(), task);
      return engine.refinePreview(session.document, edges, true);
    });
  }

  function applySelectAndMask() {
    const session = refining;
    refining = null;
    refinePreview.drop();
    if (!session) return;
    const edges = $state.snapshot(refineSettings.edges);
    const layer = layersPanel?.selectedLayer() ?? null;
    if (refineSettings.output === "selection" || !layer) {
      void sync(engine.refinePreview(session.document, edges, false));
    } else {
      const nameFormat = t("layers.copyName", { name: "{name}" });
      const newLayer = refineSettings.output === "newLayer";
      void sync(engine.refineOutput(session.document, edges, layer.id, newLayer, nameFormat));
    }
  }

  function closeSelectAndMask() {
    const session = refining;
    refining = null;
    refinePreview.drop();
    if (session) void sync(engine.refineClose(session.document));
  }

  $effect(() => {
    if (refining && refining.document !== activeId) {
      const session = refining;
      refining = null;
      void sync(engine.refineClose(session.document));
    }
  });

  /**
   * What the selection was made of from the Selections panel (its rows show it), as long as it
   * is still that selection.
   */
  let selectionsCombination = $state.raw<Combination | null>(null);

  /** A saved selection loaded into the image (`mode`: Shift adds, Alt subtracts, both intersect). */
  async function loadSavedSelection(id: number, mode: SelectionMode) {
    const doc = active;
    if (!doc) return;
    commitTransform();
    const rows = loadedRows(selectionsCombination, doc.id, doc.selectionKey, id, mode);
    await sync(engine.loadSelection(doc.id, id, mode));
    const after = tabs.find((d) => d.id === doc.id);
    selectionsCombination = { document: doc.id, key: after?.selectionKey ?? null, rows };
  }

  /** Select > Save Selection is asking a name, for this document. */
  let saveSelectionFor = $state<number | null>(null);

  function openSaveSelection() {
    commitTransform();
    if (active?.selectionKey != null) saveSelectionFor = active.id;
  }

  function saveSelection(name: string, replace: number | null) {
    const id = saveSelectionFor;
    saveSelectionFor = null;
    if (id !== null) void sync(engine.saveSelection(id, name, replace));
  }

  // Select > Color Range: a panel beside the image, whose clicks sample colors.
  let colorRange = $state<ColorRangeState | null>(null);
  /** Edit > Preferences (Ctrl+K) is open. */
  let preferences = $state(false);

  /** Color Range's settings, kept from one use to the next (as Photoshop does). */
  let colorRangeSettings: {
    fuzziness: number;
    invert: boolean;
    localized: boolean;
    sampleAll: boolean;
  } = { fuzziness: 40, invert: false, localized: false, sampleAll: false };

  function openColorRange() {
    const doc = active;
    if (!doc) return;
    commitTransform();
    colorRange = {
      document: doc.id,
      included: [],
      excluded: [],
      eyedropper: "pick",
      ...colorRangeSettings,
      // A quarter of the image's larger side, at first.
      radius: Math.max(1, Math.round(Math.max(doc.width, doc.height) / 4)),
      layerId: layersPanel?.selectedLayer()?.id ?? null,
    };
  }

  function applyColorRange() {
    const range = colorRange;
    colorRange = null;
    if (!range) return;
    const { fuzziness, invert, localized, sampleAll } = range;
    colorRangeSettings = { fuzziness, invert, localized, sampleAll };
    if (range.included.length === 0 && !range.invert) return;
    // Seconds on a large image: its progress shows, and Esc cancels it.
    const request = colorRangeRequest(range);
    void runAi("colorRange.task", (task) => engine.colorRange(range.document, request, task));
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
    if (doc && ids.length > 0) {
      // As in Photoshop, the new masks are what painting reaches.
      layersPanel?.targetMasks(ids);
      selectionCommand((id) => engine.addLayerMasks(id, ids, kind));
    }
  }

  function selectionCommand(run: (id: number) => Promise<DocumentView>) {
    const doc = active;
    if (!doc) return;
    commitTransform();
    void sync(run(doc.id));
  }

  /**
   * Select > Grow (`contiguous`) and Similar: the Magic Wand's options around the selection's
   * colors, sampling what the Magic Wand samples.
   */
  function growSelection(contiguous: boolean) {
    const doc = active;
    if (!doc) return;
    commitTransform();
    const layer = wand.sampleAll ? null : (layersPanel?.selectedLayer()?.id ?? null);
    const options = { tolerance: wand.tolerance, contiguous, antiAlias };
    // As the Magic Wand: its progress shows, and Esc cancels it.
    void runAi(contiguous ? "grow.task" : "similar.task", (task) =>
      engine.growSelection(doc.id, options, layer, task),
    );
  }

  // The Move tool (ADR 0017): a left drag on the image moves the selected layers live, in whole
  // document pixels, one undo entry per drag. As in Photoshop: Auto-Select (the options bar)
  // takes the layer under the pointer, Ctrl inverting it; the moving layers snap to the canvas
  // and to the other layers (edges and centers, not with Ctrl), with magenta smart guides.
  // A drag from inside the selection moves the selected pixels of the active layer (or of its
  // mask when it is the target) with the selection, leaving a hole; Alt copies them.
  let autoSelect = $state(true);
  /** View > Snap. */
  let snapping = $state(true);
  type MoveDrag = {
    document: number;
    /** Known once Auto-Select answered. */
    ids: number[] | null;
    /** Inside the selection: the selected pixels move instead of the layers. */
    pixels: PixelDrag | null;
    targets: SnapTargets | null;
    /** The pointer's movement since the start, and the whole pixels sent so far. */
    raw: { x: number; y: number };
    applied: { x: number; y: number };
    docPerCss: number;
    free: boolean;
  };
  type PixelDrag = {
    drag: number;
    target: "layer" | "mask";
    layerId: number;
    copy: boolean;
    /** A move was sent: the end must be. */
    sent: boolean;
  };
  let moveDrag: MoveDrag | null = null;
  let nextPixelDrag = 1;
  let guides = $state<Guide[]>([]);

  function onMoveStart(x: number, y: number, ctrl: boolean, alt: boolean) {
    const doc = active;
    if (!doc) return;
    const drag: MoveDrag = {
      document: doc.id,
      ids: null,
      pixels: null,
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
      // Quick Mask hides the outline: the layers move.
      const inside =
        doc.selectionKey != null && !doc.quickMask
          ? await engine.selectionBoundsAt(doc.id, x, y).catch(() => null)
          : null;
      if (moveDrag !== drag) return;
      if (inside) {
        const target = pixelTarget();
        drag.ids = [];
        if (!target) return;
        drag.pixels = { ...target, drag: nextPixelDrag++, copy: alt, sent: false };
        if (snapping) {
          const others = await engine.moveSnapTargets(doc.id, []).catch(() => null);
          drag.targets = { moving: inside, others: others?.others ?? [] };
        }
      } else {
        drag.ids = ids;
        if (ids.length > 0 && snapping) {
          drag.targets = await engine.moveSnapTargets(doc.id, ids).catch(() => null);
        }
      }
      if (moveDrag === drag) flushMove(drag);
    })();
  }

  /**
   * What moving selected pixels takes, as painting does: the active layer's pixels, or its
   * mask when it is the target; null (a notice shown) when it cannot.
   */
  function pixelTarget(): PixelTarget | null {
    const found = movedPixels(
      layersPanel?.selectedLayer() ?? null,
      layersPanel?.paintsMask() ?? false,
    );
    if ("error" in found) {
      showError(t(found.error));
      return null;
    }
    return found;
  }

  /**
   * Arrows with the Move tool and a selection: the selected pixels move by (dx, dy), one undo
   * entry each, as in Photoshop. Whether they did (else the layers move).
   */
  function nudgePixels(dx: number, dy: number): boolean {
    const doc = active;
    if (!doc) return false;
    const moves = nudged(tool, doc.selectionKey != null && !doc.quickMask);
    if (moves === "layers") return false;
    if (moves === "outline") {
      void sync(engine.translateSelection(doc.id, dx, dy));
      return true;
    }
    const target = pixelTarget();
    if (target) {
      const request = { ...target, drag: nextPixelDrag++, dx, dy, copy: false, end: true };
      void sendPixels(doc.id, request);
    }
    return true;
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
    if (!drag.pixels && (!drag.ids || drag.ids.length === 0)) return;
    const doc = tabs.find((d) => d.id === drag.document);
    const snaps = doc && snapping && !drag.free;
    const landed = landing(
      drag.raw,
      snaps ? (drag.targets?.moving ?? null) : null,
      doc ? [canvasBounds(doc), ...(drag.targets?.others ?? [])] : [],
      SNAP_CSS_PX * drag.docPerCss,
    );
    guides = landed.guides;
    const { x: tx, y: ty } = landed;
    if (tx === drag.applied.x && ty === drag.applied.y) return;
    drag.applied = { x: tx, y: ty };
    if (drag.pixels) {
      drag.pixels.sent = true;
      // A layer's pixels float in the view: the outline follows them on screen.
      if (drag.pixels.target === "layer") {
        outlineShift = { drag: drag.pixels.drag, document: drag.document, x: tx, y: ty };
      }
      void sendPixels(drag.document, pixelRequest(drag.pixels, drag, false));
      return;
    }
    if (!drag.ids) return;
    const move: EditRequest = { kind: "translateLayers", ids: drag.ids, dx: tx, dy: ty };
    void sync(engine.performLive(drag.document, move, true));
  }

  /**
   * How far the outline is drawn from the selection while it is dragged: with a layer's pixels
   * floating, or alone with a selection tool.
   */
  let outlineShift = $state<{ drag: number; document: number; x: number; y: number } | null>(null);

  /**
   * A request of a pixel drag: the document once it changed; while a layer's pixels float only
   * the view changes, so it redraws.
   */
  async function sendPixels(documentId: number, request: MovePixelsRequest) {
    try {
      const view = await engine.movePixels(documentId, request);
      if (view) upsert(view);
      else if (active?.id === documentId) viewport?.redraw();
    } catch (e) {
      if (e === DOCUMENT_CLOSED) await refreshTabs();
      else showError(String(e));
    }
  }

  // The selection's outline moved alone (Photoshop): a drag from inside the selection with the
  // Marquees, the Lasso or the Magic Wand in New Selection mode (see `SelectionDrag`), or the
  // arrows with any selection tool (`nudged`). One undo entry.
  /** The Polygonal Lasso places a corner at each click: no drag there. */
  const OUTLINE_DRAG_TOOLS: ToolId[] = ["marquee", "ellipse", "lasso", "wand"];
  /** The outline drag under way, if any (its id is shared with pixel drags, for `outlineShift`). */
  let outlineDrag: number | null = null;

  function shiftOutline(dx: number, dy: number) {
    const doc = active;
    if (!doc) return;
    outlineDrag ??= nextPixelDrag++;
    outlineShift = { drag: outlineDrag, document: doc.id, x: dx, y: dy };
  }

  function moveOutline(dx: number, dy: number) {
    const doc = active;
    const id = outlineDrag;
    outlineDrag = null;
    const done = () => {
      if (outlineShift?.drag === id) outlineShift = null;
    };
    if (!doc || (dx === 0 && dy === 0)) {
      done();
      return;
    }
    // The outline stays moved until the moved selection arrives.
    void sync(engine.translateSelection(doc.id, dx, dy)).finally(done);
  }

  /** A click inside the selection with an outline tool: the tool's own click. */
  function outlineClick(x: number, y: number) {
    if (tool === "wand") magicWand(Math.floor(x), Math.floor(y), null);
    else selectionCommand(engine.deselect);
  }

  /** What `SelectionDrag` needs, the same for every outline tool. */
  const outlineDragProps = $derived({
    documentId: active?.id ?? 0,
    selectionKey: active?.selectionKey ?? null,
    enabled:
      OUTLINE_DRAG_TOOLS.includes(tool) &&
      selectionMode === "replace" &&
      active?.selectionKey != null &&
      !active.quickMask,
    onshift: shiftOutline,
    onmove: moveOutline,
    onclick: outlineClick,
  });

  function pixelRequest(pixels: PixelDrag, drag: MoveDrag, end: boolean): MovePixelsRequest {
    const { drag: id, target, layerId, copy } = pixels;
    return { drag: id, target, layerId, copy, dx: drag.applied.x, dy: drag.applied.y, end };
  }

  function onMoveEnd() {
    const drag = moveDrag;
    moveDrag = null;
    guides = [];
    if (drag?.pixels) {
      // Back where it started, the engine leaves no undo entry.
      if (drag.pixels.sent) {
        const id = drag.pixels.drag;
        // The outline stays moved until the moved selection arrives.
        void sendPixels(drag.document, pixelRequest(drag.pixels, drag, true)).finally(() => {
          if (outlineShift?.drag === id) outlineShift = null;
        });
      }
      return;
    }
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
    /** The reference point, in the box's coordinates (the center at first). */
    pivot: [number, number];
    /**
     * Layers just placed (files dropped on the image): their placement, applied before the
     * box's matrix and in the same undo entry, and the insertions that Esc takes back.
     */
    placed: { matrix: Matrix; insertions: number } | null;
    /**
     * Select > Transform Selection: the box transforms the selection's outline (drawn live),
     * not layers; Enter resamples the selection.
     */
    selection: boolean;
  };
  let transforming = $state<Transforming | null>(null);
  /** Transform Selection applied, while the engine resamples the selection. */
  // Raw: compared by identity when the engine answers.
  let appliedSelectionMatrix = $state.raw<{ document: number; matrix: Matrix } | null>(null);

  /** The map the active document's selection is drawn under: Transform Selection's, live or applied. */
  const selectionMatrix = $derived(
    active && transforming?.selection && transforming.document === active.id
      ? transforming.matrix
      : active && appliedSelectionMatrix?.document === active.id
        ? appliedSelectionMatrix.matrix
        : undefined,
  );
  /** Select and Mask shows the selection another way than its ants (the engine draws none then). */
  const antsHidden = $derived(
    active !== null && refining?.document === active.id && refineSettings.view !== "ants",
  );
  /**
   * The ants the engine draws in the native view (ADR 0024); frames over IPC keep the SVG ones.
   * Quick Mask shows the selection itself.
   */
  const nativeAnts = $derived(
    antsRequest({
      native: nativeCanvas,
      selected: active?.selectionKey != null,
      hidden: antsHidden || (active?.quickMask ?? false),
      shift:
        outlineShift && outlineShift.document === active?.id
          ? [outlineShift.x, outlineShift.y]
          : undefined,
      matrix: selectionMatrix,
      reducedMotion: prefersReducedMotion(),
    }),
  );

  /**
   * Free Transform of the selected layers or, for files just dropped on the image, of the new
   * layers `place.ids`, first centered on `place.at` and scaled down to fit the canvas when
   * larger (Photoshop's Place, with its default "Resize Image During Place"); without `at`,
   * where they are (floating pixels). With `pixels` and a selection, its pixels float first
   * (Edit > Free Transform, see `floatSelection`).
   */
  async function startFreeTransform(
    place: { ids: number[]; at: [number, number] | null; insertions: number } | null = null,
    pixels = false,
  ) {
    const doc = active;
    if (!doc || transforming) return;
    if (tool === "crop") tool = "move";
    // Edit > Free Transform with a selection: its pixels (a double-click takes the layer).
    if (pixels && !place && doc.selectionKey != null && (await floatSelection(doc))) return;
    const ids = place?.ids ?? layersPanel?.selectedLayers().map((l) => l.id) ?? [];
    if (ids.length === 0) return;
    const targets = await engine.moveSnapTargets(doc.id, ids).catch(() => null);
    // Nothing to transform (empty layers), or the user moved on meanwhile.
    if (!targets?.moving || active?.id !== doc.id || transforming) return;
    let box = targets.moving;
    let placed: Transforming["placed"] = null;
    if (place && !place.at) {
      placed = { matrix: affine.IDENTITY, insertions: place.insertions };
    } else if (place?.at) {
      const [width, height] = [box.right - box.left, box.bottom - box.top];
      const fit = Math.min(1, doc.width / width, doc.height / height);
      const [cx, cy] = [(box.left + box.right) / 2, (box.top + box.bottom) / 2];
      const matrix = affine.andThen(
        affine.about(affine.scaling(fit, fit), cx, cy),
        affine.translation(place.at[0] - cx, place.at[1] - cy),
      );
      const [left, top] = affine.apply(matrix, box.left, box.top);
      const [right, bottom] = affine.apply(matrix, box.right, box.bottom);
      box = { left, top, right, bottom };
      placed = { matrix, insertions: place.insertions };
    }
    transforming = {
      document: doc.id,
      ids,
      box,
      targets: [canvasBounds(doc), ...targets.others],
      matrix: affine.IDENTITY,
      pivot: [(box.left + box.right) / 2, (box.top + box.bottom) / 2],
      placed,
      selection: false,
    };
    if (placed) onTransformChange(affine.IDENTITY);
  }

  /**
   * Select > Transform Selection, as in Photoshop: Free Transform's box on the selection's
   * bounds; the outline follows live and Enter resamples the selection (one undo entry), the
   * layers untouched.
   */
  async function startSelectionTransform() {
    const doc = active;
    if (!doc || transforming || doc.selectionKey == null || doc.quickMask) return;
    if (tool === "crop") tool = "move";
    const box = await engine.selectionBounds(doc.id).catch(() => null);
    if (!box || active?.id !== doc.id || transforming) return;
    transforming = {
      document: doc.id,
      ids: [],
      box,
      targets: [canvasBounds(doc)],
      matrix: affine.IDENTITY,
      pivot: [(box.left + box.right) / 2, (box.top + box.bottom) / 2],
      placed: null,
      selection: true,
    };
  }

  function onTransformChange(matrix: Matrix) {
    const current = transforming;
    if (!current) return;
    current.matrix = matrix;
    // The selection's outline follows the matrix; nothing is sent until Enter.
    if (current.selection) return;
    const total = current.placed ? affine.andThen(current.placed.matrix, matrix) : matrix;
    const request: EditRequest = { kind: "transformLayers", ids: current.ids, matrix: total };
    void sync(engine.performLive(current.document, request, true));
  }

  function commitTransform() {
    const current = transforming;
    if (!current) return;
    transforming = null;
    if (current.selection) {
      if (!affine.isIdentity(current.matrix)) {
        // The outline stays transformed until the engine's selection replaces it.
        const shown = { document: current.document, matrix: current.matrix };
        appliedSelectionMatrix = shown;
        void sync(engine.transformSelection(current.document, current.matrix)).finally(() => {
          if (appliedSelectionMatrix === shown) appliedSelectionMatrix = null;
        });
      }
      return;
    }
    const total = current.placed
      ? affine.andThen(current.placed.matrix, current.matrix)
      : current.matrix;
    if (!affine.isIdentity(current.matrix)) lastTransform = current.matrix;
    if (affine.isIdentity(total)) void cancelGesture(current.document);
    else void endGesture(current.document);
  }

  /**
   * Ctrl+T with a selection, as in Photoshop: the selected pixels of the active raster layer
   * float in a new layer above it (leaving a hole), which Free Transform then transforms; Esc
   * takes it all back. Whether they floated (otherwise the layers are transformed whole).
   */
  /**
   * Layer > Layer via Copy (Ctrl+J) and Layer via Cut (Shift+Ctrl+J), as in Photoshop: with a
   * selection, the active layer's selected pixels in a new layer right above it, "Layer N",
   * where they were (Cut leaves a hole, as paint), deselected. Without a selection, or on a
   * layer without pixels, Copy duplicates the selected layers.
   */
  async function layerVia(cut: boolean) {
    const doc = active;
    if (!doc) return;
    commitTransform();
    const layer = layersPanel?.selectedLayer() ?? null;
    const pixels = doc.selectionKey != null && !doc.quickMask && layer?.kind === "raster";
    if (!pixels || !layer) {
      if (!cut) layersPanel?.duplicateSelected();
      else showError(t("layerVia.needRaster"));
      return;
    }
    const name = layersPanel?.nextLayerName() ?? "";
    let made: [DocumentView, number] | null;
    try {
      made = await engine.layerVia(doc.id, layer.id, cut, name);
    } catch (e) {
      if (e === DOCUMENT_CLOSED) await refreshTabs();
      else showError(String(e));
      return;
    }
    if (!made) {
      showError(t("layerVia.empty"));
      return;
    }
    const [view, id] = made;
    upsert(view);
    if (activeId !== doc.id) return;
    await tick();
    layersPanel?.selectLayers([id]);
  }

  async function floatSelection(doc: DocumentView): Promise<boolean> {
    const layer = layersPanel?.selectedLayer() ?? null;
    if (!layer || layer.kind !== "raster" || layersPanel?.paintsMask()) return false;
    const floated = await engine.floatPixels(doc.id, layer.id).catch((e) => {
      showError(String(e));
      return null;
    });
    if (!floated) return false;
    const [view, id] = floated;
    upsert(view);
    if (activeId !== doc.id) return true;
    await tick();
    layersPanel?.selectLayers([id]);
    await startFreeTransform({ ids: [id], at: null, insertions: 1 });
    return true;
  }

  /**
   * The last transform applied (Free Transform, Edit > Transform), a map of the document's
   * space: what Edit > Transform > Again repeats.
   */
  let lastTransform = $state<Matrix | null>(null);

  /** Edit > Transform > Again (Shift+Ctrl+T): the last transform, on the selected layers. */
  function repeatTransform() {
    commitTransform();
    const doc = active;
    const ids = layersPanel?.selectedLayers().map((l) => l.id) ?? [];
    if (!doc || ids.length === 0 || !lastTransform) return;
    void edit(doc.id, { kind: "transformLayers", ids, matrix: lastTransform });
  }

  /**
   * Duplicate and Transform Again (Alt+Shift+Ctrl+T), as in Photoshop: copies of the selected
   * layers, the last transform applied to them (one undo entry); the copies are selected, so
   * that the next one continues from them.
   */
  async function duplicateAndRepeat() {
    commitTransform();
    const doc = active;
    const ids = layersPanel?.selectedLayers().map((l) => l.id) ?? [];
    if (!doc || ids.length === 0 || !lastTransform) return;
    const known = new Set(doc.layers.map((l) => l.id));
    const nameFormat = t("layers.copyName", { name: "{name}" });
    const request: EditRequest = {
      kind: "duplicateTransformLayers",
      ids,
      nameFormat,
      matrix: lastTransform,
    };
    await edit(doc.id, request);
    const after = tabs.find((d) => d.id === doc.id);
    const copies = after?.layers.filter((l) => !known.has(l.id)).map((l) => l.id) ?? [];
    if (copies.length > 0 && activeId === doc.id) {
      await tick();
      layersPanel?.selectLayers(copies);
    }
  }

  /** Esc: the layers as they were; layers just placed are taken back, as in Photoshop. */
  function cancelTransform() {
    const current = transforming;
    if (!current) return;
    transforming = null;
    if (current.selection) return;
    void cancelGesture(current.document);
    for (let i = 0; i < (current.placed?.insertions ?? 0); i++) {
      void sync(engine.undo(current.document));
    }
  }

  /**
   * Files dropped on the image (or on the layers panel): new layers, placed under the pointer
   * (the canvas center from the panel), in Free Transform.
   */
  async function placeDropped(documentId: number, paths: string[], at: [number, number] | null) {
    const before = tabs.find((d) => d.id === documentId);
    if (!before) return;
    const known = new Set(before.layers.map((l) => l.id));
    await openFiles(paths, { layerOf: documentId });
    const doc = await engine.document(documentId).catch(() => null);
    if (!doc) return;
    upsert(doc);
    const ids = doc.layers.filter((l) => !known.has(l.id)).map((l) => l.id);
    if (ids.length === 0 || activeId !== documentId) return;
    // Each file that opened is one new top-level layer (a group for a layered file) and one
    // undo entry.
    const insertions = ids.length;
    await tick();
    layersPanel?.selectLayers(ids);
    commitTransform();
    await startFreeTransform({
      ids,
      at: at ?? [doc.width / 2, doc.height / 2],
      insertions,
    });
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
    lastTransform = matrix;
    void edit(doc.id, { kind: "transformLayers", ids, matrix });
  }

  // Image > Image Size and Canvas Size (dialogs), and Image Rotation (ADR 0017): the whole
  // image, through the layers' transforms; nothing is cut or rewritten.
  let sizeDialog = $state<{ mode: "image" | "canvas"; document: number } | null>(null);
  let sizeDoc = $derived(sizeDialog && tabs.find((d) => d.id === sizeDialog?.document));

  function openSizeDialog(mode: "image" | "canvas") {
    if (active) sizeDialog = { mode, document: active.id };
  }

  function applySize(width: number, height: number, anchor: [number, number], resolution: number) {
    const dialog = sizeDialog;
    sizeDialog = null;
    const doc = dialog && tabs.find((d) => d.id === dialog.document);
    if (!dialog || !doc) return;
    // OK without a change: nothing to undo, as in Photoshop.
    const change = sizeEdit(doc, dialog.mode, { width, height }, anchor, resolution);
    if (!change) return;
    const resized = edit(dialog.document, change.request);
    // The new size, fitted on screen (the maintainer's choice), unless only the resolution
    // changed or another tab is shown meanwhile.
    if (change.resized) {
      void resized.then(() => {
        if (activeId === dialog.document) void viewport?.fit();
      });
    }
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
    const crop = cropEdit(doc ?? current, frame);
    if (!crop) return;
    cropApplying = true;
    void edit(current.document, crop).finally(() => (cropApplying = false));
  }

  function rotateImage(turn: ImageTurn) {
    if (active) void edit(active.id, { kind: "rotateImage", turn });
  }

  // Image > Image Rotation > Arbitrary: Photoshop's dialog, its last angle remembered for the
  // session.
  let rotateDialog = $state<{ document: number } | null>(null);
  let rotateLast = $state({ angle: 0, clockwise: true });

  function applyRotate(angle: number, clockwise: boolean) {
    const dialog = rotateDialog;
    rotateDialog = null;
    if (!dialog) return;
    rotateLast = { angle, clockwise };
    // No turn: nothing to undo, as in Photoshop.
    const request = rotateEdit(angle, clockwise);
    if (!request) return;
    void edit(dialog.document, request).then(() => {
      if (activeId === dialog.document) void viewport?.fit();
    });
  }

  // Image > Trim (Photoshop's dialog, its settings remembered for the session) and Reveal All:
  // crops computed by the engine; nothing is deleted, and nothing to do leaves no undo entry.
  let trimDialog = $state<{ document: number } | null>(null);
  let trimLast = $state<TrimSettings>({
    basis: "transparent",
    top: true,
    bottom: true,
    left: true,
    right: true,
  });

  function applyTrim(settings: TrimSettings) {
    const dialog = trimDialog;
    trimDialog = null;
    if (!dialog) return;
    trimLast = settings;
    void edit(dialog.document, { kind: "trim", ...settings });
  }

  function revealAll() {
    if (active) void edit(active.id, { kind: "revealAll" });
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
    const others = paths.filter((path) => !isVectorPath(path));
    if (others.length > 0) await openPaths(others, target);
    const documentId = target === "tab" ? null : target.layerOf;
    for (const path of paths.filter(isVectorPath)) {
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
      const images =
        formats === "document"
          ? []
          : formatOrder(Object.keys(EXPORT_FORMATS) as ExportFormat[], lastExportFormat);
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
      const format = formats === "document" ? null : formatOfPath(path, EXPORT_FORMATS);
      if (format === null) {
        showError(t("export.unsupportedExtension", { name: baseName(path) }));
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
    const name = path ? baseName(path) : tabTitle(doc);
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
        title: t("export.failed", { name: baseName(path), error: exportReason(failed) }),
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
    return job?.name ?? (path ? baseName(path) : "");
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

  // --- Clipboard: Cut, Copy, Copy Merged, Paste, Paste in Place, Paste Into ---------------------

  /**
   * Edit > Copy (Ctrl+C), and Cut (Ctrl+X) with `cut`, as in Photoshop: with a selection, the
   * selected pixels of the active raster layer (or of its mask when it is the target), cut by
   * erasing them; otherwise the selected layers whole (groups, masks, adjustments…), cut by
   * deleting them. One undo entry for a cut.
   */
  async function copySelection(cut: boolean) {
    const doc = active;
    if (!doc) return;
    commitTransform();
    const layer = layersPanel?.selectedLayer() ?? null;
    const mask = layersPanel?.paintsMask() ?? false;
    const pixels = doc.selectionKey != null && layer !== null && (mask || layer.kind === "raster");
    const ids = layersPanel?.selectedLayers().map((l) => l.id) ?? [];
    if (!pixels && ids.length === 0) return;
    const request: CopyRequest = pixels
      ? { kind: "pixels", layerId: layer.id, target: mask ? "mask" : "layer" }
      : { kind: "layers", ids };
    try {
      if (!(await engine.copy(doc.id, request))) {
        showError(t("copy.nothingSelected"));
        return;
      }
    } catch (e) {
      showError(t("copy.failed", { error: String(e) }));
      return;
    }
    if (!cut) return;
    if (pixels) {
      // Erased as the Eraser does (paint, ADR 0027): Delete Paint brings them back.
      paintPixels({ documentId: doc.id, layerId: layer.id, mask }, null);
    } else layersPanel?.deleteSelected();
  }

  /** Edit > Copy Merged (Shift+Ctrl+C): the visible layers composited, in the selection. */
  async function copyMerged() {
    const doc = active;
    if (!doc) return;
    commitTransform();
    try {
      if (!(await engine.copy(doc.id, { kind: "merged", name: t("copy.mergedName") }))) {
        showError(t("copy.nothingSelected"));
      }
    } catch (e) {
      showError(t("copy.failed", { error: String(e) }));
    }
  }

  /**
   * Edit > Paste (Ctrl+V), Paste in Place (Shift+Ctrl+V) and Paste Into (Alt+Shift+Ctrl+V), into
   * the active document or, without one, a new tab. The pasted layers are selected; files copied
   * in the file manager are placed like dropped ones, in the middle of the view.
   */
  async function paste(kind: PasteKind, at: [number, number] | null = null) {
    const doc = active;
    commitTransform();
    const view = doc ? (viewport?.visibleRect() ?? null) : null;
    try {
      const pasted = await engine.paste(doc?.id ?? null, t("paste.layerName"), kind, view, at);
      if (pasted.kind === "layers") {
        upsert(pasted.document);
        if (pasted.newTab) activate(pasted.document.id);
        else if (activeId === pasted.document.id) {
          await tick();
          layersPanel?.selectLayers(pasted.ids);
        }
      } else if (pasted.kind === "files") {
        if (doc) {
          const center: [number, number] =
            at ??
            (view
              ? [(view[0] + view[2]) / 2, (view[1] + view[3]) / 2]
              : [doc.width / 2, doc.height / 2]);
          void placeDropped(doc.id, pasted.paths, center);
        } else void openFiles(pasted.paths, "tab");
      } else if (pasted.kind === "noSelection") {
        showError(t("paste.noSelection"));
      } else {
        showError(t("paste.nothing"));
      }
    } catch (e) {
      if (e === DOCUMENT_CLOSED) await refreshTabs();
      else showError(t("paste.failed", { error: String(e) }));
    }
  }

  /** What the clipboard held when last looked at (menus opening): grays the pastes that do not
   * apply. `null`: not known yet (nothing grayed). Shortcuts ignore it and always try. */
  let clipboard = $state<ClipboardContents | null>(null);

  async function refreshClipboard() {
    clipboard = await engine.clipboardContents().catch(() => null);
  }

  /** The image's right-click menu, where it opened and the document point under it. */
  let canvasMenu = $state<{ x: number; y: number; at: [number, number] | null } | null>(null);

  function openCanvasMenu(e: MouseEvent) {
    // Free Transform's box has its own menu.
    if (e.defaultPrevented || !active || transforming) return;
    e.preventDefault();
    canvasMenu = {
      x: e.clientX,
      y: e.clientY,
      at: viewport?.documentPointAt(e.clientX, e.clientY) ?? null,
    };
    void refreshClipboard();
  }

  /** The image's right-click menu: the most used commands, with their shortcuts. */
  let canvasMenuItems = $derived.by((): MenuItem[] => {
    const at = canvasMenu?.at ?? null;
    const separator = { kind: "separator" as const };
    const here: MenuItem = {
      kind: "command",
      label: t("menu.edit.pasteHere"),
      run: () => void paste("at", at),
      disabled: at === null || pasteUnfit(clipboard, "paste"),
    };
    return [
      item("cut"),
      item("copy"),
      item("copyMerged"),
      separator,
      here,
      item("paste"),
      item("pasteInPlace"),
      item("pasteInto"),
      separator,
      item("freeTransform"),
      separator,
      item("selectAll"),
      item("deselect"),
      item("inverse"),
      separator,
      item("fill"),
      command(t("menu.edit.stroke"), openStroke, undefined, active?.selectionKey == null),
    ];
  });

  // --- Menu bar (ADR 0013) ------------------------------------------------------------------------

  /** A shortcut as shown in menus and in Edit > Keyboard Shortcuts. */
  function shortcutText(shortcut: Shortcut): string {
    const names = {
      shift: t("key.shift"),
      alt: t("key.alt"),
      delete: t("key.delete"),
      backspace: t("key.backspace"),
    };
    return formatShortcut(shortcut, names, isMac);
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

  /** A command that has a shortcut (its keys are in `SHORTCUTS`, see `commands`). */
  type AppCommand = {
    label: string;
    run: () => void;
    disabled?: boolean;
    checked?: boolean;
    /**
     * Its shortcut acts while a text field has the focus (the File commands); the others leave
     * their keys to the field (its own undo, copy, paste…).
     */
    whileTyping?: boolean;
    /** A held key runs it again (undo, zoom); the others act once per press. */
    repeats?: boolean;
  };

  /** The Delete key: the selected pixels with a selection (Photoshop's Clear), else the layers. */
  function deleteKey() {
    if (layersPanel?.busy()) return;
    if (active?.selectionKey != null) clearPixels();
    else layersPanel?.deleteSelected();
  }

  /**
   * The commands that have a shortcut, defined once: the menus show them, the keyboard runs
   * them (`onkeydown`), Edit > Keyboard Shortcuts lists them.
   */
  let commands = $derived.by((): Record<CommandId, AppCommand> => {
    const doc = active;
    const busy = doc !== null && saving.includes(doc.id);
    const layer = layersPanel?.selectedLayer() ?? null;
    const selectedCount = layersPanel?.selectedLayers().length ?? 0;
    const several = selectedCount > 1;
    const noSelection = doc?.selectionKey == null;
    // Ctrl+E: Merge Layers with several, Merge Down with one (Photoshop).
    const merging = layersPanel?.mergeKindSelected() ?? null;
    // Image > Adjustments: on the pixel layers shown (ADR 0029).
    const adjustable = adjustTargets().length > 0;
    return {
      newDocument: { label: t("menu.file.new"), run: () => void newDocument(), whileTyping: true },
      open: { label: t("menu.file.open"), run: () => void openWithDialog(), whileTyping: true },
      importLayers: {
        label: t("menu.file.importLayers"),
        run: () => doc && void openWithDialog(doc.id),
        disabled: !doc,
        whileTyping: true,
      },
      close: {
        label: t("menu.file.close"),
        run: () => doc && void closeTab(doc.id),
        disabled: !doc,
        whileTyping: true,
      },
      closeAll: {
        label: t("menu.file.closeAll"),
        run: () => void closeAll(),
        disabled: !doc,
        whileTyping: true,
      },
      save: {
        label: t("menu.file.save"),
        run: () => saveActive(false),
        disabled: !doc || busy,
        whileTyping: true,
      },
      saveAs: {
        label: t("menu.file.saveAs"),
        run: () => saveActive(true),
        disabled: !doc || busy,
        whileTyping: true,
      },
      export: {
        label: t("menu.file.export"),
        run: () => doc && void chooseSaveAs(doc, "images"),
        disabled: !doc,
        whileTyping: true,
      },
      print: { label: t("menu.file.print"), run: printDocument, disabled: !doc, whileTyping: true },
      documentInfo: {
        label: t("menu.file.documentInfo"),
        run: () => void showDocumentInfo(),
        disabled: !doc,
        whileTyping: true,
      },
      quit: { label: t("menu.file.quit"), run: quitApp, whileTyping: true },
      // Undo during a Free Transform cancels it, even with nothing in the history yet.
      undo: {
        label: t("menu.edit.undo"),
        run: undo,
        disabled: !transforming && !doc?.canUndo,
        repeats: true,
      },
      redo: { label: t("menu.edit.redo"), run: redo, disabled: !doc?.canRedo, repeats: true },
      cut: {
        label: t("menu.edit.cut"),
        run: () => void copySelection(true),
        disabled: !doc || (noSelection && selectedCount === 0),
      },
      copy: {
        label: t("menu.edit.copy"),
        run: () => void copySelection(false),
        disabled: !doc || (noSelection && selectedCount === 0),
      },
      copyMerged: {
        label: t("menu.edit.copyMerged"),
        run: () => void copyMerged(),
        disabled: !doc,
      },
      paste: { label: t("menu.edit.paste"), run: () => void paste("paste") },
      pasteInPlace: { label: t("menu.edit.pasteInPlace"), run: () => void paste("inPlace") },
      pasteInto: {
        label: t("menu.edit.pasteInto"),
        run: () => void paste("into"),
        disabled: noSelection,
      },
      fill: {
        label: t("menu.edit.fill"),
        run: openFill,
        disabled: !doc,
      },
      freeTransform: {
        label: t("menu.edit.freeTransform"),
        run: () => (transforming ? commitTransform() : void startFreeTransform(null, true)),
        disabled: !doc || selectedCount === 0,
      },
      repeatTransform: {
        label: t("menu.edit.transform.again"),
        run: repeatTransform,
        disabled: !doc || selectedCount === 0 || !lastTransform,
      },
      duplicateRepeat: {
        label: t("menu.edit.transform.duplicateAgain"),
        run: () => void duplicateAndRepeat(),
        disabled: !doc || selectedCount === 0 || !lastTransform,
      },
      keyboardShortcuts: {
        label: t("menu.edit.keyboardShortcuts"),
        run: () => (shortcutsList = true),
        whileTyping: true,
      },
      preferences: {
        label: t("menu.edit.preferences"),
        run: () => (preferences = true),
        whileTyping: true,
      },
      repeatFilter: {
        label: lastFilter
          ? t("menu.filter.repeat", { name: t(`filter.${lastFilter.filter}`) })
          : t("menu.filter.repeatNone"),
        run: repeatFilter,
        disabled: !lastFilter || !filterLayer(),
      },
      repeatFilterSettings: {
        label: t("menu.filter.repeatSettings"),
        run: () => lastFilter && openFilter(lastFilter.filter, lastFilter.values),
        disabled: !lastFilter || !filterLayer(),
      },
      adjustLevels: {
        label: `${t("adjustment.levels")}…`,
        run: () => void openAdjust("levels"),
        disabled: !adjustable,
      },
      adjustCurves: {
        label: `${t("adjustment.curves")}…`,
        run: () => void openAdjust("curves"),
        disabled: !adjustable,
      },
      adjustHueSaturation: {
        label: `${t("adjustment.hueSaturation")}…`,
        run: () => void openAdjust("hueSaturation"),
        disabled: !adjustable,
      },
      adjustColorBalance: {
        label: `${t("adjustment.colorBalance")}…`,
        run: () => void openAdjust("colorBalance"),
        disabled: !adjustable,
      },
      adjustBlackWhite: {
        label: `${t("adjustment.blackWhite")}…`,
        run: () => void openAdjust("blackWhite"),
        disabled: !adjustable,
      },
      adjustInvert: {
        label: t("adjustment.invert"),
        run: () => void openAdjust("invert"),
        disabled: !adjustable,
      },
      autoTone: {
        label: t("menu.image.autoTone"),
        run: () => autoLevels("tone"),
        disabled: !adjustable,
      },
      autoContrast: {
        label: t("menu.image.autoContrast"),
        run: () => autoLevels("contrast"),
        disabled: !adjustable,
      },
      autoColor: {
        label: t("menu.image.autoColor"),
        run: () => autoLevels("color"),
        disabled: !adjustable,
      },
      imageSize: {
        label: t("menu.image.imageSize"),
        run: () => openSizeDialog("image"),
        disabled: !doc,
      },
      canvasSize: {
        label: t("menu.image.canvasSize"),
        run: () => openSizeDialog("canvas"),
        disabled: !doc,
      },
      newLayer: {
        label: t("menu.layer.newLayer"),
        run: () => layersPanel?.newLayer(),
        disabled: !doc,
      },
      layerViaCopy: {
        label: t("menu.layer.viaCopy"),
        run: () => void layerVia(false),
        disabled: selectedCount === 0,
      },
      layerViaCut: {
        label: t("menu.layer.viaCut"),
        run: () => void layerVia(true),
        disabled: doc?.selectionKey == null || doc.quickMask || layer?.kind !== "raster",
      },
      duplicateLayers: {
        label: t(several ? "menu.layer.duplicateLayers" : "menu.layer.duplicate"),
        run: () => layersPanel?.duplicateSelected(),
        disabled: selectedCount === 0,
      },
      groupLayers: {
        label: t("menu.layer.group"),
        run: () => layersPanel?.groupSelected(),
        disabled: selectedCount === 0,
      },
      ungroupLayers: {
        label: t("menu.layer.ungroup"),
        run: () => layersPanel?.ungroupSelected(),
        disabled: !layersPanel?.selectionHasGroup(),
      },
      clipping: {
        label: t(
          clippingReleases(layersPanel?.selectedLayers() ?? [])
            ? "menu.layer.releaseClipping"
            : "menu.layer.createClipping",
        ),
        run: () => layersPanel?.toggleClippingSelected(),
        disabled: selectedCount === 0,
      },
      bringToFront: arrangeCommand("front", "menu.layer.arrange.front"),
      bringForward: arrangeCommand("forward", "menu.layer.arrange.forward"),
      sendBackward: arrangeCommand("backward", "menu.layer.arrange.backward"),
      sendToBack: arrangeCommand("back", "menu.layer.arrange.back"),
      mergeLayers: {
        label: t(merging === "down" ? "menu.layer.bake.mergeDown" : "menu.layer.bake.merge"),
        run: () => bake({ kind: "merge", ids: selectedIds() }),
        disabled: merging === null,
      },
      mergeVisible: {
        label: t("menu.layer.bake.mergeVisible"),
        run: () => bake({ kind: "mergeVisible" }),
        disabled: !doc || !canMergeVisible(doc.layers),
      },
      newLayerFromVisible: {
        label: t("menu.layer.newFromVisible"),
        run: () => {
          const name = layersPanel?.nextLayerName();
          if (name) bake({ kind: "visible", name });
        },
        disabled: !doc,
      },
      renameLayer: {
        label: t("menu.layer.rename"),
        run: () => !layersPanel?.busy() && layersPanel?.renameSelected(),
        disabled: !layer,
      },
      deleteLayers: {
        label: t(several ? "layers.deleteSelected" : "layers.delete"),
        run: deleteKey,
        disabled: !doc || (noSelection && selectedCount === 0),
      },
      selectAll: {
        label: t("menu.select.all"),
        run: () => selectionCommand(engine.selectAll),
        disabled: !doc,
      },
      deselect: {
        label: t("menu.select.deselect"),
        run: () => selectionCommand(engine.deselect),
        disabled: noSelection,
      },
      reselect: {
        label: t("menu.select.reselect"),
        run: () => selectionCommand(engine.reselect),
        disabled: !doc?.canReselect,
      },
      inverse: {
        label: t("menu.select.inverse"),
        run: () => selectionCommand(engine.invertSelection),
        disabled: !doc,
      },
      feather: {
        label: t("menu.select.modify.feather"),
        run: () => openModify("feather"),
        disabled: noSelection,
      },
      quickMask: {
        label: t("menu.select.quickMask"),
        run: toggleQuickMask,
        disabled: !doc,
        checked: doc?.quickMask ?? false,
      },
      selectAllLayers: {
        label: t("menu.select.allLayers"),
        run: () => layersPanel?.selectAllLayers(),
        disabled: !doc || doc.layers.length === 0,
      },
      zoomIn: {
        label: t("menu.view.zoomIn"),
        run: () => void viewport?.stepZoom(true),
        disabled: !doc,
        repeats: true,
      },
      zoomOut: {
        label: t("menu.view.zoomOut"),
        run: () => void viewport?.stepZoom(false),
        disabled: !doc,
        repeats: true,
      },
      fitOnScreen: { label: t("menu.view.fit"), run: () => void viewport?.fit(), disabled: !doc },
      actualSize: {
        label: t("menu.view.actualSize"),
        run: () => void viewport?.zoomTo(1),
        disabled: !doc,
      },
    };
  });

  /** Layer > Align and the Move tool's buttons, on the selected layers (one undo entry). */
  function alignSelected(align: AlignId) {
    const ids = layersPanel?.selectedLayers().map((l) => l.id) ?? [];
    if (active && ids.length > 0) void edit(active.id, { kind: "alignLayers", ids, align });
  }

  /** Layer > Distribute and the Move tool's buttons, on the selected layers. */
  function distributeSelected(distribute: DistributeId) {
    const ids = layersPanel?.selectedLayers().map((l) => l.id) ?? [];
    if (active && layersPanel?.canDistributeSelected()) {
      void edit(active.id, { kind: "distributeLayers", ids, distribute });
    }
  }

  /** The ids of the selected layers, bottom to top. */
  function selectedIds(): number[] {
    return layersPanel?.selectedLayers().map((l) => l.id) ?? [];
  }

  /** Layer > Bake to Pixels: `request` on the active document (one undo entry). */
  function bake(request: BakeRequest) {
    if (active) void sync(engine.bakeLayers(active.id, request));
  }

  /** `item` under another label (in a submenu that already says "New"). */
  function relabeled(item: MenuItem, label: string): MenuItem {
    return item.kind === "separator" ? item : { ...item, label };
  }

  /** Layer > Arrange's commands, grayed when they would move nothing. */
  function arrangeCommand(arrangement: Arrangement, label: MessageKey) {
    return {
      label: t(label),
      run: () => layersPanel?.arrangeSelected(arrangement),
      disabled: !layersPanel?.canArrangeSelected(arrangement),
    };
  }

  /** The menu entry of a command that has a shortcut. */
  function item(id: CommandId): MenuItem {
    const c = commands[id];
    const shortcuts = SHORTCUTS[id].map(shortcutText);
    return {
      kind: "command",
      label: c.label,
      run: c.run,
      shortcut: shortcuts[0],
      shortcuts,
      disabled: c.disabled || pasteUnfit(clipboard, id),
      checked: c.checked,
    };
  }

  /** Commands on the selected layers: the Layer menu's, also the layers' context menu. */
  let layerCommands = $derived.by(() => {
    const doc = active;
    const layer = layersPanel?.selectedLayer() ?? null;
    const selection = layersPanel?.selectedLayers() ?? [];
    const maskless = selection.filter((l) => !l.mask);
    const mask = referenceMask(selection, layer);
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
      duplicate: item("duplicateLayers"),
      rename: item("renameLayer"),
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
      // The entry deletes the layers; the Delete key clears the selected pixels when there is a
      // selection (`deleteKey`).
      delete: {
        ...item("deleteLayers"),
        run: () => layersPanel?.deleteSelected(),
        disabled: selectedCount === 0,
      },
      newGroup: command(t("menu.layer.newGroup"), () => layersPanel?.newGroup(), undefined, !doc),
      newLayer: item("newLayer"),
      deletePaint: command(
        t("menu.layer.deletePaint"),
        () => layersPanel?.deletePaintSelected(),
        undefined,
        !layersPanel?.selectionPainted(),
      ),
      group: item("groupLayers"),
      clipping: item("clipping"),
      ungroup: item("ungroupLayers"),
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
      // On every selected layer with a mask, as the new masks.
      maskToggle: command(
        t(mask?.enabled === false ? "menu.layer.maskEnable" : "menu.layer.maskDisable"),
        () => {
          const request = maskEnabledToggle(selection, layer);
          if (doc && request) void edit(doc.id, request);
        },
        undefined,
        !mask,
      ),
      maskDelete: command(
        t("menu.layer.maskDelete"),
        () => {
          const request = maskRemoval(selection);
          if (doc && request) void edit(doc.id, request);
        },
        undefined,
        !mask,
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
      c.deletePaint,
      c.maskRevealAll,
      c.maskRevealSelection,
      c.maskFromTransparency,
      c.maskToggle,
      c.maskDelete,
    ];
  });

  /** The right-click menu of the layers panel's empty area: what adds layers. */
  let emptyLayersContextMenu = $derived.by((): MenuItem[] => {
    const doc = active;
    return [
      layerCommands.newLayer,
      command(t("layers.addFill"), () => layersPanel?.addFill(colors.foreground), undefined, !doc),
      layerCommands.newGroup,
      { kind: "separator" },
      item("paste"),
      { kind: "separator" },
      item("selectAllLayers"),
    ];
  });

  let menus = $derived.by((): Menu[] => {
    const doc = active;
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
          item("newDocument"),
          item("open"),
          cmd(t("menu.file.openFolder"), () => void openFolderWithDialog()),
          {
            kind: "submenu",
            label: t("menu.file.openRecent"),
            disabled: recentFiles.length === 0,
            items: [
              ...recentLabels(recentFiles).map((label, i) =>
                cmd(label, () => void openFiles([recentFiles[i]], "tab")),
              ),
              separator,
              cmd(t("menu.file.clearRecent"), () => void engine.clearRecentFiles()),
            ],
          },
          item("importLayers"),
          // Windows' scanning dialog (WIA); not available on other systems yet.
          ...(isWindows ? [cmd(t("menu.file.importDevice"), () => void acquireImage())] : []),
          separator,
          item("close"),
          item("closeAll"),
          separator,
          item("save"),
          item("saveAs"),
          item("export"),
          separator,
          item("print"),
          item("documentInfo"),
          separator,
          item("quit"),
        ],
      },
      {
        label: t("menu.edit"),
        items: [
          item("undo"),
          item("redo"),
          separator,
          item("cut"),
          item("copy"),
          item("copyMerged"),
          item("paste"),
          item("pasteInPlace"),
          item("pasteInto"),
          separator,
          item("fill"),
          cmd(t("menu.edit.stroke"), openStroke, undefined, !doc || doc.selectionKey == null),
          separator,
          item("freeTransform"),
          {
            kind: "submenu",
            label: t("menu.edit.transform"),
            disabled: !doc || selectedCount === 0,
            items: [
              item("repeatTransform"),
              separator,
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
          item("keyboardShortcuts"),
          item("preferences"),
        ],
      },
      {
        label: t("menu.image"),
        items: [
          {
            // Photoshop's order.
            kind: "submenu",
            label: t("menu.image.adjustments"),
            disabled: !doc,
            items: (() => {
              const adjustable = adjustTargets().length > 0;
              const dialog = (adjustment: AdjustmentId) =>
                cmd(
                  `${t(`adjustment.${adjustment}`)}…`,
                  () => void openAdjust(adjustment),
                  undefined,
                  !adjustable,
                );
              return [
                dialog("brightnessContrast"),
                item("adjustLevels"),
                item("adjustCurves"),
                dialog("exposure"),
                separator,
                dialog("vibrance"),
                item("adjustHueSaturation"),
                item("adjustColorBalance"),
                item("adjustBlackWhite"),
                dialog("photoFilter"),
                dialog("channelMixer"),
                separator,
                item("adjustInvert"),
                dialog("posterize"),
                dialog("threshold"),
                dialog("gradientMap"),
                dialog("selectiveColor"),
              ];
            })(),
          },
          separator,
          item("autoTone"),
          item("autoContrast"),
          item("autoColor"),
          separator,
          item("imageSize"),
          item("canvasSize"),
          {
            kind: "submenu",
            label: t("menu.image.rotation"),
            disabled: !doc,
            items: [
              cmd(t("menu.image.rotation.halfTurn"), () => rotateImage("halfTurn")),
              cmd(t("menu.image.rotation.clockwise"), () => rotateImage("clockwise")),
              cmd(t("menu.image.rotation.counterClockwise"), () => rotateImage("counterClockwise")),
              cmd(t("menu.image.rotation.arbitrary"), () => {
                if (doc) rotateDialog = { document: doc.id };
              }),
              separator,
              cmd(t("menu.image.rotation.flipHorizontal"), () => rotateImage("flipHorizontal")),
              cmd(t("menu.image.rotation.flipVertical"), () => rotateImage("flipVertical")),
            ],
          },
          cmd(t("menu.image.crop"), cropImage, undefined, !doc),
          cmd(
            t("menu.image.trim"),
            () => {
              if (doc) trimDialog = { document: doc.id };
            },
            undefined,
            !doc,
          ),
          cmd(t("menu.image.revealAll"), revealAll, undefined, !doc),
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
          {
            kind: "submenu",
            label: t("menu.layer.new"),
            disabled: !doc,
            items: [
              relabeled(layerCommands.newLayer, t("menu.layer.new.layer")),
              relabeled(layerCommands.newGroup, t("menu.layer.new.group")),
              separator,
              item("layerViaCopy"),
              item("layerViaCut"),
            ],
          },
          // At the top level, as in Photoshop: menus nest one level deep.
          {
            kind: "submenu",
            label: t("menu.layer.newFill"),
            disabled: !doc,
            items: [
              cmd(t("menu.layer.newFill.solidColor"), () =>
                layersPanel?.addFill(colors.foreground),
              ),
            ],
          },
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
          separator,
          layerCommands.duplicate,
          layerCommands.delete,
          layerCommands.rename,
          layerCommands.visibility,
          separator,
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
              separator,
              layerCommands.maskToggle,
              layerCommands.maskDelete,
            ],
          },
          layerCommands.clipping,
          layerCommands.deletePaint,
          {
            kind: "submenu",
            label: t("menu.layer.style"),
            disabled: !layer || layer.kind === "adjustment",
            items: [
              cmd(t("menu.layer.style.blending"), () => layer && openStyle(layer, "blending")),
              separator,
              ...EFFECTS.map((effect) =>
                cmd(`${t(effect.label)}…`, () => layer && openStyle(layer, effect.id)),
              ),
              separator,
              cmd(t("menu.layer.style.clear"), clearStyles, undefined, !layer?.style),
            ],
          },
          separator,
          layerCommands.group,
          layerCommands.ungroup,
          separator,
          {
            kind: "submenu",
            label: t("menu.layer.arrange"),
            disabled: selectedCount === 0,
            items: [
              item("bringToFront"),
              item("bringForward"),
              item("sendBackward"),
              item("sendToBack"),
            ],
          },
          {
            kind: "submenu",
            label: t("menu.layer.align"),
            disabled: selectedCount === 0,
            items: ALIGNS.map((a) => cmd(t(a.label), () => alignSelected(a.id))),
          },
          {
            kind: "submenu",
            label: t("menu.layer.distribute"),
            disabled: !layersPanel?.canDistributeSelected(),
            items: DISTRIBUTES.map((d) => cmd(t(d.label), () => distributeSelected(d.id))),
          },
          separator,
          item("newLayerFromVisible"),
          {
            kind: "submenu",
            label: t("menu.layer.bake"),
            disabled: !doc,
            items: [
              cmd(
                t("menu.layer.bake.rasterize"),
                () => bake({ kind: "rasterize", ids: selectedIds() }),
                undefined,
                !canRasterize(layersPanel?.selectedLayers() ?? []),
              ),
              separator,
              item("mergeLayers"),
              item("mergeVisible"),
              cmd(
                t("menu.layer.bake.flatten"),
                () => bake({ kind: "flatten", name: t("layers.flattenedName") }),
                undefined,
                !doc || !canFlatten(doc.layers),
              ),
            ],
          },
        ],
      },
      {
        label: t("menu.select"),
        items: [
          item("selectAll"),
          item("deselect"),
          item("reselect"),
          item("inverse"),
          separator,
          cmd(t("menu.select.subject"), selectSubject, undefined, !doc),
          cmd(t("menu.select.colorRange"), openColorRange, undefined, !doc),
          cmd(
            t("menu.select.refineEdge"),
            () => void openSelectAndMask(),
            undefined,
            doc?.selectionKey == null,
          ),
          separator,
          {
            kind: "submenu",
            label: t("menu.select.modify"),
            disabled: doc?.selectionKey == null,
            items: [
              cmd(t("menu.select.modify.border"), () => openModify("border")),
              cmd(t("menu.select.modify.smooth"), () => openModify("smooth")),
              cmd(t("menu.select.modify.expand"), () => openModify("expand")),
              cmd(t("menu.select.modify.contract"), () => openModify("contract")),
              item("feather"),
            ],
          },
          separator,
          cmd(
            t("menu.select.grow"),
            () => growSelection(true),
            undefined,
            doc?.selectionKey == null,
          ),
          cmd(
            t("menu.select.similar"),
            () => growSelection(false),
            undefined,
            doc?.selectionKey == null,
          ),
          cmd(
            t("menu.select.transform"),
            () => {
              commitTransform();
              void startSelectionTransform();
            },
            undefined,
            doc?.selectionKey == null || doc.quickMask,
          ),
          item("quickMask"),
          separator,
          cmd(t("menu.select.save"), openSaveSelection, undefined, doc?.selectionKey == null),
          {
            kind: "submenu",
            label: t("menu.select.load"),
            disabled: !doc || doc.savedSelections.length === 0,
            items: (doc?.savedSelections ?? []).map((saved) =>
              cmd(saved.name, () => selectionCommand((id) => engine.loadSelection(id, saved.id))),
            ),
          },
          separator,
          item("selectAllLayers"),
          cmd(
            t("menu.select.deselectLayers"),
            () => layersPanel?.deselectLayers(),
            undefined,
            selectedCount === 0,
          ),
        ],
      },
      {
        label: t("menu.filter"),
        items: [
          item("repeatFilter"),
          item("repeatFilterSettings"),
          separator,
          {
            kind: "submenu",
            label: t("menu.filter.blur"),
            items: [
              cmd(
                `${t("filter.gaussianBlur")}…`,
                () => openFilter("gaussianBlur"),
                undefined,
                !filterLayer(),
              ),
            ],
          },
        ],
      },
      {
        label: t("menu.view"),
        items: [
          item("zoomIn"),
          item("zoomOut"),
          separator,
          item("fitOnScreen"),
          { ...cmd(t("menu.view.snap"), () => (snapping = !snapping)), checked: snapping },
          item("actualSize"),
        ],
      },
      {
        // Window: the dock's panels, checked while unfolded; choosing one unfolds it (or, already
        // unfolded, folds the dock), as its tab does. Layers is always shown (ADR 0030).
        label: t("menu.window"),
        items: dockPanels.map((panel) => ({
          ...cmd(
            panel.label,
            () => {
              dock = clickTab(dock, panel.id);
              saveDock(dock);
            },
            undefined,
            !doc,
          ),
          checked: doc !== null && dock.open === panel.id,
        })),
      },
      {
        label: t("menu.help"),
        items: [cmd(t("menu.help.about"), () => void showAbout())],
      },
    ];
  });

  // --- Keyboard --------------------------------------------------------------------------------

  /** Edit > Keyboard Shortcuts is open. */
  let shortcutsList = $state(false);

  function onkeydown(e: KeyboardEvent) {
    const action = keyAction(
      {
        key: e.key,
        code: e.code,
        ctrlKey: e.ctrlKey,
        metaKey: e.metaKey,
        altKey: e.altKey,
        shiftKey: e.shiftKey,
        repeat: e.repeat,
        inTextField: isTextField(e.target),
        inSelect: e.target instanceof HTMLSelectElement,
      },
      {
        mac: isMac,
        tool,
        aiTask: aiTask !== null,
        layerTransfer: layerTransfer !== null,
        tabDrag: tabDrag !== null,
        dialogOpen: (modal) => !!document.querySelector(modal ? "dialog:modal" : "dialog[open]"),
        command: (id) => commands[id],
      },
    );
    if (action.kind === "none") return;
    if (action.kind === "endLayerTransfer") return endTransfer();
    if (action.kind === "cancelTabDrag") return cancelTabDrag();
    e.preventDefault();
    switch (action.kind) {
      case "cancelAiTask":
        return cancelAiTask();
      case "cycleTabs":
        return cycleTabs(action.step);
      case "toolSlot":
        return selectSlot(action.slot, action.next);
      case "paintSize": {
        const options = action.eraser ? eraserOptions : brushOptions;
        options.size = stepBrush(options.size, action.larger);
        return;
      }
      case "paintHardness": {
        const options = action.eraser ? eraserOptions : brushOptions;
        const hardness = options.hardness + (action.larger ? 0.25 : -0.25);
        options.hardness = Math.min(Math.max(hardness, 0), 1);
        return;
      }
      case "defaultColors":
        setPaintColors({ ...MASK_COLORS });
        return;
      case "swapColors": {
        const { foreground, background } = paintColors();
        setPaintColors({ foreground: background, background: foreground });
        return;
      }
      case "quickSelectionSize":
        quick.size = stepBrush(quick.size, action.larger);
        return;
      case "command":
        return commands[action.id].run();
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
    let stopDocumentUpdates: (() => void) | null = null;
    void onDocumentUpdated(upsert).then((stop) => {
      if (destroyed) stop();
      else stopDocumentUpdates = stop;
    });
    let stopAiProgress: (() => void) | null = null;
    void onAiProgress(onAiTaskProgress).then((stop) => {
      if (destroyed) stop();
      else stopAiProgress = stop;
    });
    let stopRecentFiles: (() => void) | null = null;
    void onRecentFiles((paths) => (recentFiles = paths)).then(async (stop) => {
      if (destroyed) return stop();
      stopRecentFiles = stop;
      recentFiles = await engine.recentFiles().catch(() => []);
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
        if (target === "layer" && active) {
          // Under the pointer on the image; centered from the layers panel.
          const scale = isWindows ? window.devicePixelRatio : 1;
          const [x, y] = [payload.position.x / scale, payload.position.y / scale];
          const onImage = document.elementFromPoint(x, y)?.closest(".stage") != null;
          const at = onImage ? (viewport?.documentPointAt(x, y) ?? null) : null;
          void placeDropped(active.id, payload.paths, at);
        } else void openFiles(payload.paths, "tab");
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
      stopAiProgress?.();
      stopDocumentUpdates?.();
      stopRecentFiles?.();
      void stopDrop.then((unlisten) => unlisten());
      void stopClose.then((unlisten) => unlisten());
    };
  });
</script>

<svelte:window {onkeydown} onblur={cancelTabDrag} bind:innerWidth={windowWidth} />

<div class="app">
  <header class="menubar">
    <img class="logo" src="/favicon.svg" alt="" draggable="false" />
    <MenuBar {menus} onopen={() => void refreshClipboard()} />
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
    bind:brush={brushOptions}
    bind:eraser={eraserOptions}
    transform={transforming ? transformBar : undefined}
    quickMask={active?.quickMask ?? false}
    bind:quickMaskOpacity
    alignable={(layersPanel?.selectedLayers().length ?? 0) > 0}
    distributable={layersPanel?.canDistributeSelected() ?? false}
    onalign={alignSelected}
    ondistribute={distributeSelected}
  />
  {#snippet transformBar()}
    {#if transforming}
      <TransformFields
        matrix={transforming.matrix}
        pivot={transforming.pivot}
        canvas={active ?? { width: 1, height: 1 }}
        onchange={onTransformChange}
      />
    {/if}
  {/snippet}

  <main
    class:has-panel={active !== null}
    style:--panel-width="{shownPanelWidth}px"
    class:transferring={layerTransfer !== null}
    bind:this={mainElement}
    onpointermove={onTransferMove}
    onpointerup={onTransferUp}
    onpointercancel={endTransfer}
  >
    <Toolbar
      {tool}
      choices={toolChoices}
      onselect={selectTool}
      bind:colors={paintColors, setPaintColors}
      onpickcolor={(which) => (colorPicker = which)}
    />
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
            {#if doc.quickMask}
              <span class="tab-mode">({t("quickMask.label")})</span>
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

      <div
        class="stage"
        class:see-through={nativeCanvas && active}
        role="presentation"
        oncontextmenu={openCanvasMenu}
      >
        {#if active}
          {#key active.id}
            <Viewport
              bind:this={viewport}
              native={nativeCanvas}
              ants={nativeAnts}
              documentId={active.id}
              revision={active.revision}
              quickMask={active.quickMask}
              quickMaskOpacity={active.quickMaskOpacity}
              onframe={(stats) => (frame = stats)}
              onmovestart={onMoveStart}
              onmove={tool === "move" && !transforming ? onMoveDrag : undefined}
              onmoveend={onMoveEnd}
              ondoubleclick={() => void startFreeTransform()}
              {guides}
            >
              {#snippet overlay(mapping)}
                {#if active?.selectionKey != null && !nativeCanvas}
                  <SelectionOutline
                    hidden={antsHidden}
                    shift={outlineShift?.document === active.id
                      ? [outlineShift.x, outlineShift.y]
                      : undefined}
                    {mapping}
                    documentId={active.id}
                    selectionKey={active.selectionKey}
                    width={active.width}
                    height={active.height}
                    matrix={selectionMatrix}
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
                    bind:matrix={transforming.matrix}
                    bind:pivot={transforming.pivot}
                    targets={snapping ? transforming.targets : []}
                    onchange={onTransformChange}
                    oncommit={commitTransform}
                    oncancel={cancelTransform}
                  />
                {:else if refining?.document === active?.id && refineSettings.brush.on}
                  <!-- Select and Mask's refine-edge brush: strokes mark where the model decides. -->
                  <PaintTool
                    {mapping}
                    size={refineSettings.brush.size}
                    onstroke={refineBrushStroke}
                  />
                  <StrokeTrail
                    {mapping}
                    points={refineStroke?.samples.map(([x, y]) => [x, y] as [number, number]) ?? []}
                    size={refineSettings.brush.size}
                  />
                {:else if colorRange && colorRange.document === active?.id}
                  <!-- Color Range open: a click on the image samples a color. -->
                  <EyedropperOverlay
                    {mapping}
                    kind={colorRange.eyedropper}
                    onsample={(x, y, keys) => colorRange && sampleAt(colorRange, x, y, keys)}
                    patch={loupePatch}
                  />
                {:else if tool === "wand"}
                  <SelectionDrag {mapping} {...outlineDragProps}>
                    <WandTool {mapping} mode={selectionMode} onpick={magicWand} />
                  </SelectionDrag>
                {:else if tool === "objectSelection"}
                  <ObjectSelectionTool
                    {mapping}
                    mode={selectionMode}
                    busy={aiBusy}
                    onhover={objectHover}
                    onselect={objectSelect}
                  />
                {:else if isPaintTool(tool)}
                  <PaintTool
                    {mapping}
                    size={(isEraser(tool) ? eraserOptions : brushOptions).size}
                    onstroke={paintStroke}
                  />
                {:else if tool === "quickSelection"}
                  <QuickSelectionTool
                    {mapping}
                    mode={selectionMode}
                    size={quick.size}
                    busy={quickBusy}
                    onstroke={quickStroke}
                  />
                {:else if tool === "lasso" || tool === "polygonalLasso"}
                  <SelectionDrag {mapping} {...outlineDragProps}>
                    <LassoTool
                      {mapping}
                      polygonal={tool === "polygonalLasso"}
                      mode={selectionMode}
                      onselect={selectShape}
                      ondeselect={() => selectionCommand(engine.deselect)}
                    />
                  </SelectionDrag>
                {:else if tool === "marquee" || tool === "ellipse"}
                  <SelectionDrag {mapping} {...outlineDragProps}>
                    <MarqueeTool
                      {mapping}
                      kind={tool === "marquee" ? "rectangle" : "ellipse"}
                      mode={selectionMode}
                      onselect={selectShape}
                      ondeselect={() => selectionCommand(engine.deselect)}
                    />
                  </SelectionDrag>
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
            {#if recentFiles.length > 0}
              <RecentFiles paths={recentFiles} onopen={(path) => void openFiles([path], "tab")} />
            {/if}
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
        <PanelResizer bind:width={panelWidth} />
        {#key active.id}
          <LayersPanel
            bind:this={layersPanelInstance}
            doc={active}
            onedit={edit}
            onlive={live}
            ongestureend={endGesture}
            onnudge={nudgePixels}
            contextMenu={layerContextMenu}
            emptyContextMenu={emptyLayersContextMenu}
            onlayerdrag={onLayerDrag}
            hidden={previewHidden}
            onfillcolor={pickFillLayerColor}
            onstyle={openStyle}
            onentryedit={openEntry}
          />
        {/key}
        <PanelDock bind:dock panels={dockPanels}>
          {#snippet content(panel)}
            {#if panel === "properties"}
              {#if selectedProperties}
                <PropertiesPanel
                  documentId={active.id}
                  layer={selectedProperties}
                  onfillcolor={pickFillLayerColor}
                  onedit={edit}
                  onlive={live}
                  ongestureend={endGesture}
                />
              {:else}
                <p class="dock-empty">{t("properties.empty")}</p>
              {/if}
            {:else}
              <SelectionsPanel
                saved={active.savedSelections}
                selected={active.selectionKey != null}
                onload={loadSavedSelection}
                combined={combinedRows(selectionsCombination, active.id, active.selectionKey)}
                onsave={openSaveSelection}
                onreplace={(id) => selectionCommand((doc) => engine.saveSelection(doc, "", id))}
                onrename={(id, name) => void sync(engine.renameSavedSelection(active.id, id, name))}
                ondelete={(id) => void sync(engine.deleteSavedSelection(active.id, id))}
                ondeselect={() => selectionCommand(engine.deselect)}
                canReselect={active.canReselect}
                onreselect={() => selectionCommand(engine.reselect)}
              />
            {/if}
          {/snippet}
        </PanelDock>
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

{#if colorPicker}
  {@const which = colorPicker}
  <ColorPickerDialog
    title={t(which === "foreground" ? "colorPicker.foreground" : "colorPicker.background")}
    color={paintColors()[which]}
    gray={paintsGray()}
    onapply={(hex) => {
      setPaintColors({ ...paintColors(), [which]: hex });
      colorPicker = null;
    }}
    onclose={() => (colorPicker = null)}
    sample={pickerSample}
  />
{/if}

{#if printDialog !== null}
  {@const doc = tabs.find((d) => d.id === printDialog)}
  {#if doc}
    <PrintDialog
      documentId={doc.id}
      title={tabTitle(doc)}
      width={doc.width}
      height={doc.height}
      resolution={doc.resolution}
      onclose={() => (printDialog = null)}
    />
  {/if}
{/if}

{#if documentInfo}
  <DocumentInfoDialog info={documentInfo} onclose={() => (documentInfo = null)} />
{/if}

{#if canvasMenu}
  <ContextMenu
    x={canvasMenu.x}
    y={canvasMenu.y}
    items={canvasMenuItems}
    onclose={() => (canvasMenu = null)}
  />
{/if}
{#if filterDialog}
  <FilterDialog
    filter={filterDialog.filter}
    values={filterDialog.values}
    preview={filterDialog.preview}
    onlive={filterLive}
    onpreview={filterPreview}
    onok={applyFilterDialog}
    oncancel={cancelFilter}
  />
{/if}
{#if entryDialog && entryShown}
  <AdjustDialog
    adjustment={entryShown}
    preview={entryDialog.preview}
    onlive={(values, gradient) => entryLive({ values, gradient })}
    oncurves={(curves) => entryLive({ curves })}
    onpreview={entryPreview}
    onok={applyEntry}
    oncancel={cancelEntry}
  />
{/if}
{#if adjustDialog && adjustShown}
  <AdjustDialog
    adjustment={adjustShown}
    preview={adjustDialog.preview}
    onlive={(values, gradient) => adjustLive(values, undefined, gradient)}
    oncurves={(curves) => adjustLive([], curves)}
    onpreview={adjustPreview}
    onok={applyAdjust}
    oncancel={cancelAdjust}
  />
{/if}
{#if styleDialog && !styleDialog.picking}
  <LayerStyleDialog
    style={styleDialog.style}
    page={styleDialog.page}
    onchange={changeStyle}
    onpage={(page) => styleDialog && (styleDialog.page = page)}
    onpickcolor={pickStyleColor}
    onok={() => closeStyle(true)}
    oncancel={() => closeStyle(false)}
  />
{/if}

{#if fillDialog && !fillDialog.picking}
  <FillDialog
    color={fillDialog.color}
    onpickcolor={pickFillColor}
    onchoose={applyFill}
    onclose={() => (fillDialog = null)}
  />
{/if}
{#if strokeDialog && !strokeDialog.picking}
  <StrokeDialog
    color={strokeDialog.color}
    onpickcolor={pickStrokeColor}
    onchoose={applyStroke}
    onclose={() => (strokeDialog = null)}
  />
{/if}
{#if pickColor}
  {@const picking = pickColor}
  <ColorPickerDialog
    title={picking.title}
    color={picking.color}
    onapply={(hex) => {
      // Taken before clearing: `picking` follows `pickColor`.
      const chosen = pickColor;
      pickColor = null;
      chosen?.apply(hex);
    }}
    onclose={() => {
      const chosen = pickColor;
      pickColor = null;
      chosen?.close?.();
    }}
    sample={pickerSample}
  />
{/if}
{#if newDialog}
  <NewDocumentDialog
    clipboard={newClipboard}
    oncreate={(s) => void createDocument(s)}
    onclose={() => (newDialog = false)}
  />
{/if}
{#if saveSelectionFor !== null && saveSelectionFor === activeId && active}
  <SaveSelectionDialog
    saved={active.savedSelections}
    onapply={saveSelection}
    onclose={() => (saveSelectionFor = null)}
  />
{/if}

{#if modifyDialog && modifyDialog.document === activeId}
  <ModifyDialog
    kind={modifyDialog.kind}
    value={modifyAmounts[modifyDialog.kind]}
    max={modifyDialog.kind === "feather" ? MAX_FEATHER : MAX_MODIFY}
    onapply={applyModify}
    onclose={closeModify}
    onpreview={modifyPreview.push}
  />
{/if}

{#if refining && refining.document === activeId}
  <SelectAndMaskPanel
    bind:settings={refineSettings}
    canOutputToLayer={(layersPanel?.selectedLayer() ?? null) !== null}
    busy={aiBusy}
    onview={refineView}
    onedges={refinePreview.push}
    ondetect={refineDetect}
    onapply={applySelectAndMask}
    onclose={closeSelectAndMask}
  />
{/if}

{#if preferences}
  <PreferencesDialog onclose={() => (preferences = false)} />
{/if}

{#if shortcutsList}
  <KeyboardShortcutsDialog
    {menus}
    extra={[
      {
        label: t("menu.edit.transform.duplicateAgain"),
        keys: SHORTCUTS.duplicateRepeat.map(shortcutText),
      },
    ]}
    onclose={() => (shortcutsList = false)}
  />
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

{#if trimDialog}
  <TrimDialog settings={trimLast} onapply={applyTrim} onclose={() => (trimDialog = null)} />
{/if}

{#if rotateDialog}
  <RotateDialog
    angle={rotateLast.angle}
    clockwise={rotateLast.clockwise}
    onapply={applyRotate}
    onclose={() => (rotateDialog = null)}
  />
{/if}

{#if sizeDialog && sizeDoc}
  {#key sizeDialog}
    <SizeDialog
      mode={sizeDialog.mode}
      width={sizeDoc.width}
      height={sizeDoc.height}
      resolution={sizeDoc.resolution}
      onapply={applySize}
      onclose={() => (sizeDialog = null)}
    />
  {/key}
{/if}

{#if vectorImport}
  {#key vectorImport}
    <VectorImportDialog
      path={vectorImport.path}
      name={baseName(vectorImport.path)}
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

<!-- Exports run in the background: progress and outcome in a card above the status bar; an AI
     selection under way shows there too. -->
{#if exports.length > 0 || toasts.length > 0 || aiTask?.shown}
  <aside class="export-card" aria-live="polite">
    {#if aiTask?.shown}
      {@const task = aiTask}
      {@const percent = task.total > 0 ? Math.round((100 * task.done) / task.total) : 0}
      <div class="export-job">
        <div class="export-row">
          <span class="export-title">
            {t("ai.task.progress", { label: task.label, percent })}
          </span>
          <button
            class="icon-btn card-button"
            title={t("ai.task.cancel")}
            aria-label={t("ai.task.cancel")}
            onclick={cancelAiTask}
          >
            ✕
          </button>
        </div>
        <div class="export-bar"><span style:width="{percent}%"></span></div>
      </div>
    {/if}
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
    grid-template-columns: 40px 1fr var(--panel-width);
  }

  /* Native presentation: the canvas area shows the window surface drawn by the engine. */
  :global(.native-canvas) main {
    background: transparent;
  }

  .sidebar {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .sidebar > :global(:nth-child(2)) {
    flex: 1 1 0;
  }

  .dock-empty {
    margin: 0;
    padding: 10px;
    color: var(--text-muted);
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

  /* "(Quick Mask)" after the name, as Photoshop titles the document. */
  .tab-mode {
    flex: none;
    color: #e57368;
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
