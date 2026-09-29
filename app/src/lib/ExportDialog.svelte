<script lang="ts">
  import { onMount, untrack } from "svelte";
  import {
    EXPORT_FORMATS,
    engine,
    type ColorSpaceId,
    type ExportFormat,
    type ExportSpec,
  } from "./engine";
  import { t } from "./i18n/index.svelte";

  let {
    documentId,
    path,
    format,
    onexport,
    onclose,
  }: {
    documentId: number;
    /** The file chosen in the save dialog; its extension decided `format`. */
    path: string;
    format: ExportFormat;
    /** Export the document to `path` with `spec`. The owner closes the dialog. */
    onexport: (documentId: number, path: string, spec: ExportSpec) => void;
    onclose: () => void;
  } = $props();

  // Like "Save As" in image editors: the file (and so the format) was chosen first; this dialog
  // only holds that format's options, starting from the defaults for this document.
  const target = untrack(() => ({ documentId, path, format }));
  let spec = $state<ExportSpec | null>(null);
  /** Color spaces the format can store. */
  let spaces = $state<ColorSpaceId[]>([]);
  let failure = $state<string | null>(null);
  let dialog: HTMLDialogElement;

  const options = EXPORT_FORMATS[target.format];
  const fileName = target.path.split(/[\\/]/).pop() || target.path;

  async function load() {
    try {
      const [defaults, named] = await Promise.all([
        engine.exportDefaults(target.documentId, target.format),
        engine.exportSpaces(target.format),
      ]);
      // The document's own unnamed space is only offered when the defaults picked it.
      spaces = defaults.space === "custom" ? [...named, "custom"] : named;
      spec = defaults;
      failure = null;
    } catch (e) {
      failure = t("export.loadFailed", { error: String(e) });
    }
  }

  function spaceLabel(space: ColorSpaceId): string {
    return space === "custom" ? t("export.colorSpace.custom") : t(`colorSpace.${space}`);
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (spec) onexport(target.documentId, target.path, $state.snapshot(spec));
  }

  onMount(() => {
    dialog.showModal();
    void load();
    // Modal: the app's shortcuts (undo, delete layer, zoom, …) must not act behind the dialog.
    // Native keyboard behavior (Tab, Enter, Escape, arrows in lists) is a default action and
    // still applies.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="export-title"
  oncancel={(e) => {
    // Escape: the owner closes the dialog.
    e.preventDefault();
    onclose();
  }}
>
  <form onsubmit={submit}>
    <header id="export-title">
      {t("export.titleFor", { format: t(`export.format.${target.format}`) })}
    </header>
    <p class="file" title={target.path}>{fileName}</p>
    <div class="fields">
      {#if spec}
        <label for="export-depth">{t("export.depth")}</label>
        <select id="export-depth" bind:value={spec.sample}>
          {#each options.samples as sample (sample)}
            <option value={sample}>{t(`export.depth.${sample}`)}</option>
          {/each}
        </select>

        <label for="export-space">{t("export.colorSpace")}</label>
        <select id="export-space" bind:value={spec.space}>
          {#each spaces as space (space)}
            <option value={space}>{spaceLabel(space)}</option>
          {/each}
        </select>

        {#if options.compressions.length > 0}
          <label for="export-compression">{t("export.compression")}</label>
          <select id="export-compression" bind:value={spec.compression}>
            {#each options.compressions as compression (compression)}
              <option value={compression}>{t(`export.compression.${compression}`)}</option>
            {/each}
          </select>
        {/if}

        <label class="check" title={t("export.alpha.hint")}>
          <input type="checkbox" bind:checked={spec.keepAlpha} />
          {t("export.alpha")}
        </label>
        {#if spec.sample === "u8"}
          <label class="check">
            <input type="checkbox" bind:checked={spec.dither} />
            {t("export.dither")}
          </label>
        {/if}
      {/if}

      {#if failure}
        <p class="failure">{failure}</p>
      {/if}
    </div>
    <footer>
      <button type="button" onclick={onclose}>{t("export.cancel")}</button>
      <button type="submit" class="primary" disabled={!spec}>
        {t("export.confirm")}
      </button>
    </footer>
  </form>
</dialog>

<style>
  dialog {
    width: 300px;
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

  .file {
    margin: 0;
    padding: 6px 10px 0;
    overflow: hidden;
    color: var(--text-muted);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  header {
    padding: 5px 10px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    font-weight: 600;
  }

  .fields {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: 6px 10px;
    padding: 10px;
  }

  .fields > label:not(.check) {
    color: var(--text-muted);
  }

  select {
    min-width: 0;
  }

  .check {
    grid-column: 2;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .check input {
    margin: 0;
    accent-color: var(--accent);
  }

  .failure {
    grid-column: 1 / -1;
    margin: 0;
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

  footer button {
    min-width: 76px;
    height: 22px;
    padding: 0 10px;
    border: 1px solid var(--border-strong);
    background: var(--field);
  }

  footer button:hover:not(:disabled) {
    background: var(--hover);
  }

  footer .primary {
    border-color: var(--accent);
    background: var(--accent);
    color: #ffffff;
  }

  footer .primary:hover:not(:disabled) {
    background: #4d9af0;
  }
</style>
