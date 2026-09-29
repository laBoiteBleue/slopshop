<script lang="ts">
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { onMount } from "svelte";
  import {
    DOCUMENT_CLOSED,
    engine,
    onOpenEvents,
    type DocumentView,
    type EditRequest,
    type GpuInfo,
    type OpenFailed,
    type OpenFinished,
    type Opening,
  } from "./lib/engine";
  import { getLocale, locales, setLocale, t, type Locale } from "./lib/i18n/index.svelte";
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

  // --- Keyboard --------------------------------------------------------------------------------

  function onkeydown(e: KeyboardEvent) {
    if (e.ctrlKey && e.key === "Tab") {
      // Ctrl+Tab everywhere, like browsers and most editors (Cmd+Tab belongs to macOS).
      e.preventDefault();
      cycleTabs(e.shiftKey ? -1 : 1);
      return;
    }
    if (!hasShortcutModifier(e) || e.altKey) return;
    const key = e.key.toLowerCase();
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
    let destroyed = false;
    // Subscribe first, then catch up with what happened before (e.g. startup files).
    void onOpenEvents({
      started: onOpenStarted,
      finished: onOpenFinished,
      failed: onOpenFailed,
    }).then(async (stop) => {
      if (destroyed) return stop();
      stopEvents = stop;
      const [documents, pending, failures] = await Promise.all([
        engine.documents(),
        engine.openings(),
        engine.openFailures(),
      ]);
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
      void stopDrop.then((unlisten) => unlisten());
    };
  });
</script>

<svelte:window {onkeydown} />

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
      <div class="tabbar" class:drop={dropTarget === "tab"} role="tablist">
        {#each tabs as doc (doc.id)}
          <div
            class="tab"
            class:active={doc.id === activeId}
            role="tab"
            tabindex="-1"
            aria-selected={doc.id === activeId}
            title={tabTitle(doc)}
            onpointerdown={(e) => {
              if (e.button === 0) activate(doc.id);
            }}
            onauxclick={(e) => {
              // Middle click closes, like browsers.
              if (e.button === 1) void closeTab(doc.id);
            }}
          >
            <span class="tab-name">{tabTitle(doc)}</span>
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

      <div class="stage">
        {#if active}
          {#key active.id}
            <Viewport
              bind:this={viewport}
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

<style>
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

  .tab-name {
    overflow: hidden;
    text-overflow: ellipsis;
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
