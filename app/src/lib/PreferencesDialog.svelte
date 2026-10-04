<script lang="ts">
  // Edit > Preferences (Ctrl+K): the interface's language, and the AI components (ADR 0025):
  // what is installed, its size and licenses; download what is missing, remove what is not
  // wanted.
  import { onMount } from "svelte";
  import { engine, type AiComponent } from "./engine";
  import AiDownloadDialog from "./AiDownloadDialog.svelte";
  import { componentName, failureMessage } from "./ai";
  import { formatBytes } from "./format";
  import { getLocale, locales, setLocale, t, type Locale } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";

  let { onclose }: { onclose: () => void } = $props();

  let dialog: HTMLDialogElement;
  /** `undefined` while loading; `null`: AI is not offered on this platform. */
  let components = $state<AiComponent[] | null | undefined>(undefined);
  let downloading = $state<AiComponent[] | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);

  async function refresh() {
    try {
      components = await engine.aiComponents(null);
    } catch (e) {
      error = failureMessage(e);
      components = null;
    }
  }

  async function remove(component: AiComponent) {
    busy = true;
    error = null;
    try {
      await engine.aiRemove(component.id);
    } catch (e) {
      error = failureMessage(e);
    }
    busy = false;
    await refresh();
  }

  onMount(() => {
    dialog.showModal();
    void refresh();
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="preferences-title"
  oncancel={(e) => {
    e.preventDefault();
    if (!downloading) onclose();
  }}
>
  <header id="preferences-title" {@attach movable("preferences")}>{t("preferences.title")}</header>
  <section class="body" aria-labelledby="preferences-language">
    <h2 id="preferences-language">{t("preferences.language")}</h2>
    <select
      aria-labelledby="preferences-language"
      value={getLocale()}
      onchange={(e) => setLocale(e.currentTarget.value as Locale)}
    >
      {#each Object.entries(locales) as [code, { name }] (code)}
        <option value={code}>{name}</option>
      {/each}
    </select>
  </section>
  <section class="body" aria-labelledby="preferences-ai">
    <h2 id="preferences-ai">{t("preferences.ai")}</h2>
    <p class="muted">{t("preferences.ai.intro")}</p>
    {#if components === null}
      <p>{t("ai.unsupported")}</p>
    {:else if components}
      <ul class="components">
        {#each components as component (component.id)}
          <li>
            <div class="info">
              <span class="name">{componentName(component.id)}</span>
              <span class="muted">
                {component.installed
                  ? t("ai.installed", { size: formatBytes(component.installedSize) })
                  : t("ai.notInstalled", { size: formatBytes(component.downloadSize) })}
                ·
                {#each component.licenses as license, i (license.url)}
                  {#if i > 0},
                  {/if}
                  <button
                    type="button"
                    class="link"
                    title={license.url}
                    onclick={() => void engine.aiOpenLicense(license.url)}
                  >
                    {license.name}
                  </button>
                {/each}
              </span>
            </div>
            {#if component.installed}
              <button
                type="button"
                class="btn small"
                disabled={busy}
                onclick={() => void remove(component)}
              >
                {t("ai.remove")}
              </button>
            {:else}
              <button
                type="button"
                class="btn small"
                disabled={busy}
                onclick={() => (downloading = [component])}
              >
                {t("ai.download")}
              </button>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
    {#if error}
      <p class="error" role="alert">{error}</p>
    {/if}
  </section>
  <footer>
    <button type="button" class="btn primary" onclick={onclose}>{t("preferences.close")}</button>
  </footer>
</dialog>

{#if downloading}
  <AiDownloadDialog
    components={downloading}
    ondone={() => {
      downloading = null;
      void refresh();
    }}
    onclose={() => {
      downloading = null;
      void refresh();
    }}
  />
{/if}

<style>
  dialog {
    width: 480px;
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
    gap: 8px;
    padding: 10px;
  }

  .body + .body {
    padding-top: 0;
  }

  select {
    justify-self: start;
  }

  h2 {
    margin: 0;
    font-size: inherit;
    font-weight: 600;
  }

  p {
    margin: 0;
  }

  .muted {
    color: var(--text-muted);
  }

  .components {
    display: grid;
    gap: 6px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 6px 8px;
    border-radius: 4px;
    background: var(--field);
  }

  li > .btn {
    flex: none;
    white-space: nowrap;
  }

  .info {
    display: grid;
    gap: 2px;
    min-width: 0;
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
