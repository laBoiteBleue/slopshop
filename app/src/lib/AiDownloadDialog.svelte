<script lang="ts">
  // Asks before downloading AI components (ADR 0025): what they are, their size, their
  // licenses (the ones that are not permissive open source must be accepted), then downloads
  // them with a progress bar. Cancel keeps what was fetched for the next attempt.
  import { onMount } from "svelte";
  import { engine, type AiComponent } from "./engine";
  import { componentName, failureMessage } from "./ai";
  import { formatBytes } from "./format";
  import { t } from "./i18n/index.svelte";

  let {
    components,
    ondone,
    onclose,
  }: {
    /** The components to download (installed ones are skipped). */
    components: AiComponent[];
    ondone: () => void;
    onclose: () => void;
  } = $props();

  let dialog: HTMLDialogElement;
  const missing = $derived(components.filter((c) => !c.installed));
  const download = $derived(missing.reduce((sum, c) => sum + c.downloadSize, 0));
  const disk = $derived(missing.reduce((sum, c) => sum + c.installedSize, 0));
  const mustAccept = $derived(missing.some((c) => c.licenses.some((l) => l.accept)));
  const nonCommercial = $derived(missing.some((c) => c.licenses.some((l) => !l.commercial)));

  let accepted = $state(false);
  let progress = $state<{ done: number; total: number } | null>(null);
  let error = $state<string | null>(null);

  async function start() {
    error = null;
    progress = { done: 0, total: download };
    try {
      await engine.aiInstall(
        missing.map((c) => c.id),
        (p) => (progress = p),
      );
      ondone();
    } catch (e) {
      error = failureMessage(e);
      progress = null;
    }
  }

  function cancel() {
    if (progress) void engine.aiCancelInstall();
    else onclose();
  }

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="ai-download-title"
  oncancel={(e) => {
    e.preventDefault();
    cancel();
  }}
>
  <header id="ai-download-title">{t("ai.download.title")}</header>
  <div class="body">
    <p>{t("ai.download.intro")}</p>
    <ul class="components">
      {#each missing as component (component.id)}
        <li>
          <div class="row">
            <span class="name">{componentName(component.id)}</span>
            <span class="size">{formatBytes(component.downloadSize)}</span>
          </div>
          <div class="licenses">
            {#each component.licenses as license (license.url)}
              <button
                type="button"
                class="link"
                title={license.url}
                onclick={() => void engine.aiOpenLicense(license.url)}
              >
                {license.name}
              </button>
              {#if !license.commercial}
                <span class="badge">{t("ai.download.nonCommercial")}</span>
              {/if}
            {/each}
          </div>
        </li>
      {/each}
    </ul>
    <p class="total">
      {t("ai.download.total", { download: formatBytes(download), disk: formatBytes(disk) })}
    </p>
    {#if nonCommercial}
      <p class="warning">{t("ai.download.nonCommercialNote")}</p>
    {/if}
    {#if mustAccept}
      <label class="check">
        <input type="checkbox" bind:checked={accepted} disabled={progress !== null} />
        {t("ai.download.accept")}
      </label>
    {/if}
    {#if progress}
      <div class="progress">
        <div class="bar">
          <span style:width="{(progress.done / Math.max(progress.total, 1)) * 100}%"></span>
        </div>
        <span class="amount">
          {t("ai.download.progress", {
            done: formatBytes(progress.done),
            total: formatBytes(progress.total),
          })}
        </span>
      </div>
    {/if}
    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}
  </div>
  <footer>
    <button type="button" class="btn" onclick={cancel}>{t("ai.download.cancel")}</button>
    <button
      type="button"
      class="btn primary"
      disabled={progress !== null || (mustAccept && !accepted) || missing.length === 0}
      onclick={() => void start()}
    >
      {t(error ? "ai.download.retry" : "ai.download.start")}
    </button>
  </footer>
</dialog>

<style>
  dialog {
    width: 420px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    color: var(--text);
    box-shadow: 0 10px 32px #0009;
  }

  dialog::backdrop {
    background: #00000055;
  }

  header {
    padding: 5px 10px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    font-weight: 600;
  }

  .body {
    display: grid;
    gap: 10px;
    padding: 10px;
  }

  p {
    margin: 0;
  }

  .components {
    display: grid;
    gap: 8px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .row {
    display: flex;
    justify-content: space-between;
    gap: 10px;
  }

  .size,
  .total,
  .amount {
    color: var(--text-muted);
  }

  .licenses {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px 10px;
    margin-top: 2px;
  }

  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent);
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }

  .badge {
    padding: 0 5px;
    border-radius: 3px;
    background: var(--danger-bg);
    color: var(--danger-fg);
    font-size: 0.9em;
  }

  .warning {
    color: var(--danger-fg);
  }

  .check {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .check input {
    margin: 0;
    accent-color: var(--accent);
  }

  .progress {
    display: grid;
    gap: 4px;
  }

  .bar {
    height: 4px;
    border-radius: 2px;
    background: var(--slider-track);
    overflow: hidden;
  }

  .bar > span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.15s linear;
  }

  .error {
    color: var(--danger-fg);
    user-select: text;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>
