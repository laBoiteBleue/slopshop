<script lang="ts">
  // A newer version of SlopShop (ADR 0039): its notes, then Install and Restart downloads it
  // with a progress bar, installs it and restarts. Unsaved documents are asked about first.
  import { onMount } from "svelte";
  import { engine, type UpdateInfo } from "./engine";
  import { updateFailureMessage } from "./updates";
  import { formatBytes } from "./format";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";

  let {
    update,
    confirmInstall,
    onclose,
  }: {
    update: UpdateInfo;
    /** Asks about unsaved documents; true when installing may go on. */
    confirmInstall: () => Promise<boolean>;
    onclose: () => void;
  } = $props();

  let dialog: HTMLDialogElement;
  let progress = $state<{ done: number; total: number } | null>(null);
  let error = $state<string | null>(null);

  async function install() {
    error = null;
    if (!(await confirmInstall())) return;
    progress = { done: 0, total: 0 };
    try {
      // On success the application exits: nothing follows.
      await engine.updateInstall((p) => (progress = p));
    } catch (e) {
      error = updateFailureMessage(e);
    }
    progress = null;
  }

  function cancel() {
    if (progress) void engine.updateCancel();
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
  aria-labelledby="update-title"
  oncancel={(e) => {
    e.preventDefault();
    cancel();
  }}
>
  <header id="update-title" {@attach movable("update")}>
    {t("update.title", { version: update.version })}
  </header>
  <div class="body">
    <p>{t("update.current", { version: update.currentVersion })}</p>
    {#if update.notes}
      <section aria-labelledby="update-notes">
        <h2 id="update-notes">{t("update.notes")}</h2>
        <pre class="notes">{update.notes}</pre>
      </section>
    {/if}
    {#if progress}
      <div class="progress">
        <div class="bar" class:unknown={progress.total === 0}>
          <span style:width="{(progress.done / Math.max(progress.total, 1)) * 100}%"></span>
        </div>
        <span class="amount">
          {progress.total > 0
            ? t("update.progress", {
                done: formatBytes(progress.done),
                total: formatBytes(progress.total),
              })
            : formatBytes(progress.done)}
        </span>
      </div>
    {/if}
    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}
  </div>
  <footer>
    <button type="button" class="btn" onclick={cancel}>
      {t(progress ? "update.cancel" : "update.later")}
    </button>
    <button
      type="button"
      class="btn primary"
      disabled={progress !== null}
      onclick={() => void install()}
    >
      {t("update.install")}
    </button>
  </footer>
</dialog>

<style>
  dialog {
    width: 460px;
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

  section {
    display: grid;
    gap: 4px;
  }

  h2 {
    margin: 0;
    font-size: inherit;
    font-weight: 600;
  }

  .notes {
    max-height: 220px;
    margin: 0;
    padding: 6px 8px;
    overflow: auto;
    border-radius: 4px;
    background: var(--field);
    font: inherit;
    white-space: pre-wrap;
    user-select: text;
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

  /* No size announced: the bar only says that something is happening. */
  .bar.unknown > span {
    width: 30% !important;
    animation: slide 1.2s linear infinite;
  }

  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(340%);
    }
  }

  .amount {
    color: var(--text-muted);
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
