<script lang="ts">
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open as openDialog, save } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import {
    DOCUMENT_CLOSED,
    EXPORT_FORMATS,
    engine,
    onExportEvents,
    onOpenEvents,
    type DocumentView,
    type EditRequest,
    type ExportFailed,
    type ExportFinished,
    type ExportFormat,
    type ExportProgress,
    type ExportSpec,
    type ExportStarted,
    type GpuInfo,
    type OpenFailed,
    type OpenFinished,
    type Opening,
  } from "./lib/engine";
  import { getLocale, locales, setLocale, t, type Locale } from "./lib/i18n/index.svelte";
  import ExportDialog from "./lib/ExportDialog.svelte";
  import Icon from "./lib/Icon.svelte";
  import { hasShortcutModifier, isWindows, modifierLabel } from "./lib/platform";
  import { formatZoom } from "./lib/format";
  import LayersPanel from "./lib/LayersPanel.svelte";
  import Viewport, { type FrameStats } from "./lib/Viewport.svelte";
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
  let error = $state<string | null>(null);
  let frame = $state<FrameStats | null>(null);
  /** Viewport of the active tab. */
  let viewport = $state<Viewport | null>(null);
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
  /** The save dialog is open. */
  let choosingExportFile = false;
  /** The dialog opens on the last format used. */
  let lastExportFormat = $state<ExportFormat>("png");
  /** Exports running (rows done out of total). */
  let exports = $state<{ id: number; name: string; done: number; total: number }[]>([]);
  /** Outcome of the last export (translated): a title and one line per report entry. */
  let exportResult = $state<{
    title: string;
    lines: string[];
    kind: "done" | "notice" | "error";
  } | null>(null);

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
      error = null;
    } catch (e) {
      if (e === DOCUMENT_CLOSED) {
        // Made for a tab closed meanwhile: nothing was applied.
        await refreshTabs();
      } else {
        error = String(e);
      }
    }
  }

  // Mutations name the document they were made for, taken when the user acts.
  const edit = (id: number, request: EditRequest) => sync(engine.perform(id, request));
  const live = (id: number, request: EditRequest) => sync(engine.performLive(id, request));
  const endGesture = (id: number) => sync(engine.endGesture(id));
  const undo = () => active && sync(engine.undo(active.id));
  const redo = () => active && sync(engine.redo(active.id));

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

  /** Open files in new tabs, or as layers of a document. Progress arrives as events. */
  async function openFiles(paths: string[], target: "tab" | { layerOf: number }) {
    error = null;
    await Promise.all(
      paths.map(async (path) => {
        try {
          if (target === "tab") {
            const doc = await engine.openImage(path);
            upsert(doc);
            activate(doc.id);
          } else {
            upsert(await engine.addImageLayer(target.layerOf, path));
          }
        } catch (e) {
          // The failure event normally already set a localized message; a target tab closed
          // meanwhile is not an error.
          if (e !== DOCUMENT_CLOSED) error ??= String(e);
        }
      }),
    );
  }

  async function openWithDialog() {
    const picked = await openDialog({ multiple: true, directory: false });
    const paths = picked === null ? [] : Array.isArray(picked) ? picked : [picked];
    if (paths.length > 0) await openFiles(paths, "tab");
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
    error = t("open.failed", { name: failed.name, error: reason });
  }

  /** Drop zones: the image of the active tab adds layers; anywhere else opens new tabs. */
  function dropTargetAt(position: { x: number; y: number }): "tab" | "layer" {
    // Tauri labels the position physical, but only WebView2 reports device pixels; WebKit
    // (macOS, Linux) already reports CSS pixels.
    const scale = isWindows ? window.devicePixelRatio : 1;
    const element = document.elementFromPoint(position.x / scale, position.y / scale);
    if (active && element?.closest(".stage")) return "layer";
    return "tab";
  }

  // --- Export ---------------------------------------------------------------------------------

  /** Formats in the order of the save dialog's file types: the last one used first. */
  function exportFormatOrder(): ExportFormat[] {
    const all: ExportFormat[] = ["png", "tiff", "exr"];
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

  /**
   * Export, like "Save As": the save dialog first (its file types are the formats; the system
   * adds the extension of the chosen type and asks before overwriting), then the options of
   * the format of the chosen file.
   */
  async function chooseExportFile() {
    const doc = active;
    if (!doc || choosingExportFile || exportTarget) return;
    choosingExportFile = true;
    try {
      const order = exportFormatOrder();
      const path = await save({
        title: t("export.title"),
        defaultPath: exportFileName(tabTitle(doc), EXPORT_FORMATS[order[0]].extensions[0]),
        filters: order.map((format) => ({
          name: t(`export.format.${format}`),
          extensions: EXPORT_FORMATS[format].extensions,
        })),
      });
      if (path === null) return;
      const format = formatOfPath(path);
      if (format === null) {
        exportResult = {
          title: t("export.unsupportedExtension", { name: fileNameOf(path) }),
          lines: [],
          kind: "error",
        };
        return;
      }
      exportTarget = { documentId: doc.id, path, format };
    } catch (e) {
      exportResult = { title: String(e), lines: [], kind: "error" };
    } finally {
      choosingExportFile = false;
    }
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
    exportResult = null;
    try {
      await engine.exportDocument(documentId, path, spec);
    } catch (e) {
      const failed: ExportFailed =
        typeof e === "object" && e !== null && "code" in e
          ? (e as ExportFailed)
          : { code: "internal", detail: String(e) };
      // Closed meanwhile: the user asked for that.
      if (failed.code === "documentClosed") return;
      exportResult = {
        title: t("export.failed", { name: fileNameOf(path), error: exportReason(failed) }),
        lines: [],
        kind: "error",
      };
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

  /** Show an export outcome; a plain success disappears by itself after a few seconds. */
  function showExportResult(result: NonNullable<typeof exportResult>) {
    exportResult = result;
    if (result.kind !== "done") return;
    setTimeout(() => {
      if (exportResult === result) exportResult = null;
    }, 4000);
  }

  function onExportFinished(finished: ExportFinished) {
    const name = endExport(finished.id, finished.path);
    const report = finished.notices.map((notice) =>
      t(`export.report.${notice.id}`, notice.count === null ? undefined : { count: notice.count }),
    );
    showExportResult({
      title: t("export.finished", { name }),
      lines: report,
      kind: report.length > 0 ? "notice" : "done",
    });
  }

  function onExportFailed(failed: ExportFailed) {
    const name = endExport(failed.id, null);
    showExportResult(
      failed.code === "cancelled"
        ? { title: t("export.cancelled"), lines: [], kind: "done" }
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

  // --- Keyboard --------------------------------------------------------------------------------

  function onkeydown(e: KeyboardEvent) {
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
    if (!hasShortcutModifier(e) || e.altKey) return;
    const key = e.key.toLowerCase();
    // Also the physical key, like the zoom digits: layouts differ.
    if (e.shiftKey && (key === "e" || e.code === "KeyE")) {
      e.preventDefault();
      if (!e.repeat) void chooseExportFile();
      return;
    }
    if (key === "o" && !e.shiftKey) {
      e.preventDefault();
      void openWithDialog();
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
    // Text fields keep their own undo.
    if (e.target instanceof HTMLInputElement && ["text", "number"].includes(e.target.type)) return;
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
      if (lastFailure && !error) onOpenFailed(lastFailure);
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
    engine.gpuInfo().then(
      (info) => (gpu = info),
      (e) => (gpuError = String(e)),
    );
    return () => {
      destroyed = true;
      stopEvents?.();
      stopExportEvents?.();
      void stopDrop.then((unlisten) => unlisten());
    };
  });
</script>

<svelte:window {onkeydown} onblur={cancelTabDrag} />

<div class="app">
  <header class="menubar">
    <img class="logo" src="/favicon.svg" alt="" draggable="false" />
    <span class="brand">SlopShop</span>
    <span class="tag">{t("app.preAlpha")}</span>
    <div class="sep"></div>
    <button class="icon" onclick={newDocument} title={t("tabs.newHint", { mod: modifierLabel })}>
      <Icon name="plus" />
    </button>
    <button class="icon" onclick={openWithDialog} title={t("open.hint", { mod: modifierLabel })}>
      <Icon name="open" />
    </button>
    <button
      class="icon"
      onclick={chooseExportFile}
      disabled={!active}
      title={t("export.hint", { mod: modifierLabel })}
    >
      <Icon name="export" />
    </button>
    <div class="sep"></div>
    <button
      class="icon"
      onclick={undo}
      disabled={!active?.canUndo}
      title={t("toolbar.undoHint", { mod: modifierLabel })}
    >
      <Icon name="undo" />
    </button>
    <button
      class="icon"
      onclick={redo}
      disabled={!active?.canRedo}
      title={t("toolbar.redoHint", { mod: modifierLabel })}
    >
      <Icon name="redo" />
    </button>
    <select
      class="locale"
      aria-label={t("app.language")}
      value={getLocale()}
      onchange={(e) => setLocale(e.currentTarget.value as Locale)}
    >
      {#each Object.entries(locales) as [code, { name }] (code)}
        <option value={code}>{name}</option>
      {/each}
    </select>
  </header>

  <main class:has-panel={active !== null}>
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
            {#if doc.id === activeId && frame}
              <span class="tab-zoom">@ {formatZoom(frame.zoom)}</span>
            {/if}
            <button
              class="tab-close"
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
              onframe={(stats) => (frame = stats)}
            />
          {/key}
        {:else if ready}
          <div class="welcome">
            <img src="/favicon.svg" alt="" draggable="false" />
            <p>{t("welcome.title")}</p>
            <div class="welcome-actions">
              <button onclick={openWithDialog}>{t("welcome.open")}</button>
              <button onclick={newDocument}>{t("welcome.new")}</button>
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
      </div>
    </section>

    {#if active}
      {#key active.id}
        <LayersPanel doc={active} onedit={edit} onlive={live} ongestureend={endGesture} />
      {/key}
    {/if}
  </main>

  <footer class="status">
    {#if active}
      <ZoomSlider
        zoom={frame?.zoom ?? null}
        hint={t("view.hint", { mod: modifierLabel })}
        onzoom={(zoom) => viewport?.zoomTo(zoom) ?? Promise.resolve()}
        onstep={(zoomIn) => void viewport?.stepZoom(zoomIn)}
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
    {#if error}
      <span class="error">{error}</span>
    {:else if notices.length > 0}
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

{#if exportDoc && exportTarget}
  {#key exportTarget}
    <ExportDialog
      documentId={exportDoc.id}
      path={exportTarget.path}
      format={exportTarget.format}
      onexport={startExport}
      onclose={() => (exportTarget = null)}
    />
  {/key}
{/if}

<!-- Exports run in the background: progress and outcome in a card above the status bar. -->
{#if exports.length > 0 || exportResult}
  <aside class="export-card" aria-live="polite">
    {#each exports as job (job.id)}
      <div class="export-job">
        <div class="export-row">
          <span class="export-title">
            {t("export.progress", { name: job.name, percent: percent(job) })}
          </span>
          <button
            class="card-button"
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
    {#if exportResult}
      <div class="export-result {exportResult.kind}">
        <div class="export-row">
          <span class="export-title">{exportResult.title}</span>
          <button
            class="card-button"
            title={t("export.dismiss")}
            aria-label={t("export.dismiss")}
            onclick={() => (exportResult = null)}
          >
            ✕
          </button>
        </div>
        {#each exportResult.lines as line, i (i)}
          <p>{line}</p>
        {/each}
      </div>
    {/if}
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
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-muted);
    font-size: 9px;
  }

  .card-button:hover {
    background: var(--hover);
    color: var(--text);
  }

  .app {
    display: grid;
    grid-template-rows: 30px 1fr 22px;
    height: 100vh;
  }

  .menubar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    background: var(--chrome);
    border-bottom: 1px solid var(--border-dark);
  }

  .logo {
    width: 16px;
    height: 16px;
  }

  .brand {
    font-weight: 600;
  }

  .tag {
    padding: 0 5px;
    border-radius: 2px;
    background: var(--brand-muted);
    color: var(--brand);
    font-size: 10px;
    line-height: 15px;
  }

  .sep {
    width: 1px;
    height: 16px;
    margin: 0 4px;
    background: var(--border-strong);
  }

  .icon {
    display: grid;
    place-items: center;
    width: 26px;
    height: 22px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
  }

  .icon:hover:not(:disabled) {
    background: var(--hover);
  }

  .locale {
    margin-left: auto;
  }

  main {
    display: grid;
    grid-template-columns: 1fr;
    gap: 1px;
    min-height: 0;
    background: var(--border-dark);
  }

  main.has-panel {
    grid-template-columns: 1fr 260px;
  }

  /* Native presentation: the canvas area shows the window surface drawn by the engine. */
  :global(.native-canvas) main {
    background: transparent;
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

  .tab-close {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-muted);
    font-size: 10px;
    visibility: hidden;
  }

  .tab.active .tab-close,
  .tab:hover .tab-close {
    visibility: visible;
  }

  .tab-close:hover {
    background: var(--hover);
    color: var(--text);
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

  .welcome-actions button {
    padding: 5px 14px;
    border: 1px solid var(--border-strong);
    background: var(--field);
  }

  .welcome-actions button:hover {
    background: var(--hover);
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

  .status .error {
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--danger-fg);
  }

  .status .right {
    margin-left: auto;
    font-variant-numeric: tabular-nums;
  }
</style>
