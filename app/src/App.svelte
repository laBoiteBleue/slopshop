<script lang="ts">
  import { onMount } from "svelte";
  import { engine, type DocumentView, type EditRequest, type GpuInfo } from "./lib/engine";
  import { getLocale, locales, setLocale, t, type Locale } from "./lib/i18n/index.svelte";
  import Icon from "./lib/Icon.svelte";
  import LayersPanel from "./lib/LayersPanel.svelte";
  import Viewport from "./lib/Viewport.svelte";

  let doc = $state<DocumentView | null>(null);
  let gpu = $state<GpuInfo | null>(null);
  let gpuError = $state<string | null>(null);
  let error = $state<string | null>(null);
  let frameMs = $state<number | null>(null);

  /** Apply a document view returned by the engine, ignoring out-of-order (stale) responses. */
  async function sync(request: Promise<DocumentView | null>) {
    try {
      const view = await request;
      if (view && (!doc || view.revision >= doc.revision)) doc = view;
      error = null;
    } catch (e) {
      error = String(e);
    }
  }

  const edit = (request: EditRequest) => sync(engine.perform(request));
  const live = (request: EditRequest) => sync(engine.performLive(request));
  const endGesture = () => sync(engine.endGesture());
  const undo = () => sync(engine.undo());
  const redo = () => sync(engine.redo());

  function onkeydown(e: KeyboardEvent) {
    if (!(e.ctrlKey || e.metaKey)) return;
    // Text fields keep their own undo.
    if (e.target instanceof HTMLInputElement && ["text", "number"].includes(e.target.type)) return;
    const key = e.key.toLowerCase();
    if (key === "z" && !e.shiftKey) {
      e.preventDefault();
      void undo();
    } else if ((key === "z" && e.shiftKey) || key === "y") {
      e.preventDefault();
      void redo();
    }
  }

  onMount(() => {
    void sync(engine.document());
    engine.gpuInfo().then(
      (info) => (gpu = info),
      (e) => (gpuError = String(e)),
    );
  });
</script>

<svelte:window {onkeydown} />

<div class="app">
  <header class="menubar">
    <img class="logo" src="/favicon.svg" alt="" />
    <span class="brand">SlopShop</span>
    <span class="tag">{t("app.preAlpha")}</span>
    <div class="sep"></div>
    <button class="icon" onclick={undo} disabled={!doc?.canUndo} title={t("toolbar.undoHint")}>
      <Icon name="undo" />
    </button>
    <button class="icon" onclick={redo} disabled={!doc?.canRedo} title={t("toolbar.redoHint")}>
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

  <main>
    {#if doc}
      <section class="workspace">
        <div class="doc-tabs">
          <span class="doc-tab">
            {t("document.untitled")}
            <span class="doc-meta">
              {t("document.info", {
                width: doc.width,
                height: doc.height,
                space: t(`colorSpace.${doc.workingSpace}`),
              })}
            </span>
          </span>
        </div>
        <Viewport revision={doc.revision} onframe={(ms) => (frameMs = ms)} />
      </section>
      <LayersPanel {doc} onedit={edit} onlive={live} ongestureend={endGesture} />
    {:else}
      <p class="loading">{error ?? t("app.loading")}</p>
    {/if}
  </main>

  <footer class="status">
    <span>
      {#if gpu}
        {t("status.gpu", { name: gpu.name, backend: gpu.backend })}
      {:else if gpuError}
        {t("status.gpuUnavailable", { error: gpuError })}
      {:else}
        {t("status.gpuInit")}
      {/if}
    </span>
    {#if error && doc}
      <span class="error">{error}</span>
    {/if}
    <span class="right">
      {#if doc}
        {t("status.revision", { revision: doc.revision })}
      {/if}
      {#if frameMs !== null}
        · {t("status.frameTime", { ms: frameMs.toFixed(1) })}
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
    grid-template-columns: 1fr 260px;
    gap: 1px;
    min-height: 0;
    background: var(--border-dark);
  }

  .workspace {
    display: grid;
    grid-template-rows: 26px 1fr;
    min-width: 0;
    min-height: 0;
  }

  .doc-tabs {
    display: flex;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
  }

  .doc-tab {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
    background: var(--panel);
    border-right: 1px solid var(--border-dark);
    white-space: nowrap;
  }

  .doc-meta {
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }

  .loading {
    grid-column: 1 / -1;
    place-self: center;
    color: var(--text-muted);
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
