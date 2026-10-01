<script lang="ts">
  import { onMount, untrack } from "svelte";
  import {
    DEFAULT_QUALITY,
    EXPORT_FORMATS,
    engine,
    type ColorSpaceId,
    type ExportFormat,
    type ExportSpec,
  } from "./engine";
  import { hexToSrgb, srgbToHex } from "./color";
  import { t } from "./i18n/index.svelte";

  let {
    documentId,
    width,
    height,
    path,
    format,
    onexport,
    onclose,
  }: {
    documentId: number;
    /** Size of the document, in pixels. */
    width: number;
    height: number;
    /** The file chosen in the save dialog; its extension decided `format`. */
    path: string;
    format: ExportFormat;
    /** Export the document to `path` with `spec`. The owner closes the dialog. */
    onexport: (documentId: number, path: string, spec: ExportSpec) => void;
    onclose: () => void;
  } = $props();

  // Like "Save As" in image editors: the file (and so the format) was chosen first; this dialog
  // only holds that format's options, starting from the defaults for this document.
  const target = untrack(() => ({ documentId, path, format, width, height }));
  let spec = $state<ExportSpec | null>(null);
  /** Color spaces the format can store, for color and for gray samples. */
  let colorSpaces = $state<ColorSpaceId[]>([]);
  let graySpaces = $state<ColorSpaceId[]>([]);
  let spaces = $derived(spec?.gray ? graySpaces : colorSpaces);
  let failure = $state<string | null>(null);
  /** The format's size limit, when the document exceeds it: the export would fail. */
  let tooLargeFor = $state<number | null>(null);
  let dialog: HTMLDialogElement;

  const options = EXPORT_FORMATS[target.format];
  const fileName = target.path.split(/[\\/]/).pop() || target.path;

  async function load() {
    try {
      const [defaults, named, namedGray, maxSide] = await Promise.all([
        engine.exportDefaults(target.documentId, target.format),
        engine.exportSpaces(target.format, false),
        options.gray ? engine.exportSpaces(target.format, true) : Promise.resolve([]),
        engine.exportMaxSide(target.format),
      ]);
      tooLargeFor =
        maxSide !== null && (target.width > maxSide || target.height > maxSide) ? maxSide : null;
      // The document's own unnamed space is only offered when the defaults picked it.
      const custom: ColorSpaceId[] = defaults.space === "custom" ? ["custom"] : [];
      colorSpaces = [...named, ...custom];
      graySpaces = [...namedGray, ...custom];
      spec = defaults;
      failure = null;
    } catch (e) {
      failure = t("export.loadFailed", { error: String(e) });
    }
  }

  /** Gray on or off: keep the space if the file can still declare it, else sRGB. */
  function onGrayChange() {
    if (spec && !spaces.includes(spec.space)) {
      spec.space = spaces.includes("srgb") ? "srgb" : (spaces[0] ?? spec.space);
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
        {#if options.samples.length > 1}
          <label for="export-depth">{t("export.depth")}</label>
          <select id="export-depth" bind:value={spec.sample}>
            {#each options.samples as sample (sample)}
              <option value={sample}>{t(`export.depth.${sample}`)}</option>
            {/each}
          </select>
        {/if}

        <label for="export-space">{t("export.colorSpace")}</label>
        <select id="export-space" bind:value={spec.space}>
          {#each spaces as space (space)}
            <option value={space}>{spaceLabel(space)}</option>
          {/each}
        </select>

        {#if options.gray}
          <label class="check" title={t("export.gray.hint")}>
            <input type="checkbox" bind:checked={spec.gray} onchange={onGrayChange} />
            {t("export.gray")}
          </label>
        {/if}

        {#if options.compressions.length > 0}
          <label for="export-compression">{t("export.compression")}</label>
          <select
            id="export-compression"
            bind:value={spec.compression}
            onchange={() => {
              // Lossy WebP and JPEG 2000 have a quality, their lossless modes none.
              if (spec && (target.format === "webp" || target.format === "jp2")) {
                spec.quality = spec.compression === "lossy" ? DEFAULT_QUALITY : null;
              }
            }}
          >
            {#each options.compressions as compression (compression)}
              <option value={compression}>{t(`export.compression.${compression}`)}</option>
            {/each}
          </select>
        {/if}

        {#if spec.quality !== null}
          <label for="export-quality">{t("export.quality")}</label>
          <div class="quality">
            <input
              type="range"
              min="1"
              max="100"
              aria-label={t("export.quality")}
              bind:value={spec.quality}
            />
            <input id="export-quality" type="number" min="1" max="100" bind:value={spec.quality} />
          </div>
        {/if}

        <!-- Gray JPEG has no color to subsample. -->
        {#if options.subsamplings.length > 0 && !spec.gray}
          <label for="export-subsampling">{t("export.subsampling")}</label>
          <select id="export-subsampling" bind:value={spec.subsampling}>
            {#each options.subsamplings as subsampling (subsampling)}
              <option value={subsampling}>{t(`export.subsampling.${subsampling}`)}</option>
            {/each}
          </select>
        {/if}

        {#if options.alpha}
          <label class="check" title={t("export.alpha.hint")}>
            <input type="checkbox" bind:checked={spec.keepAlpha} />
            {t("export.alpha")}
          </label>
        {/if}
        {#if !spec.keepAlpha}
          <label for="export-matte">{t("export.matte")}</label>
          <input
            id="export-matte"
            type="color"
            title={t("export.matte.hint")}
            value={srgbToHex(spec.matte)}
            oninput={(e) => {
              if (spec) spec.matte = hexToSrgb(e.currentTarget.value);
            }}
          />
        {/if}
        {#if spec.sample === "u8"}
          <label class="check">
            <input type="checkbox" bind:checked={spec.dither} />
            {t("export.dither")}
          </label>
        {/if}
      {/if}

      {#if tooLargeFor !== null}
        <p class="failure">
          {t("export.tooLargeForFormat", {
            format: t(`export.format.${target.format}`),
            max: tooLargeFor,
            width: target.width,
            height: target.height,
          })}
        </p>
      {/if}
      {#if failure}
        <p class="failure">{failure}</p>
      {/if}
    </div>
    <footer>
      <button type="button" class="btn" onclick={onclose}>{t("export.cancel")}</button>
      <button type="submit" class="btn primary" disabled={!spec || tooLargeFor !== null}>
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

  .quality {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }

  .quality input[type="range"] {
    flex: 1;
    min-width: 0;
    accent-color: var(--accent);
  }

  .quality input[type="number"] {
    width: 44px;
  }

  input[type="color"] {
    width: 40px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--border-strong);
    background: none;
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
</style>
