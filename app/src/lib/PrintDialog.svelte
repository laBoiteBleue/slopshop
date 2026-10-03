<script lang="ts">
  // File > Print (Ctrl+P), as Photoshop's Print Settings: the page and the image on it, the
  // paper and its orientation, the size (fitted to the paper, or at a resolution) and the
  // position. Print hands over to the system's print dialog (printer, copies, two-sided,
  // driver settings). The image is the engine's render of the document as displayed.
  import { onMount, untrack } from "svelte";
  import { engine } from "./engine";
  import { getLocale, t } from "./i18n/index.svelte";
  import type { MessageKey } from "./i18n/en";
  import Icon from "./Icon.svelte";
  import { PAPERS, layoutOf, printImage, type PaperId, type PrintSettings } from "./print";

  let {
    documentId,
    title,
    width,
    height,
    onclose,
  }: {
    documentId: number;
    /** The document's name, for the print job. */
    title: string;
    /** The document's size, pixels. */
    width: number;
    height: number;
    onclose: () => void;
  } = $props();

  const STORAGE_KEY = "slopshop.printSettings";
  /** Below this, a print looks soft: the dialog says so. */
  const LOW_PPI = 150;

  /** Paper, size and position are remembered; the orientation follows the image. */
  function restore(): PrintSettings {
    const landscape = untrack(() => width > height);
    const defaults: PrintSettings = {
      paper: "a4",
      landscape,
      fit: true,
      ppi: 300,
      center: true,
      left: 0,
      top: 0,
    };
    try {
      const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "null");
      if (saved && typeof saved === "object") {
        const paper = PAPERS.some((p) => p.id === saved.paper) ? saved.paper : defaults.paper;
        return {
          ...defaults,
          paper,
          fit: saved.fit !== false,
          ppi: Number.isFinite(saved.ppi) && saved.ppi > 0 ? saved.ppi : defaults.ppi,
          center: saved.center !== false,
        };
      }
    } catch {
      // No saved settings: the defaults.
    }
    return defaults;
  }

  let settings = $state<PrintSettings>(restore());
  const layout = $derived(layoutOf(settings, width, height));

  $effect(() => {
    const { paper, fit, ppi, center } = settings;
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({ paper, fit, ppi, center }));
    } catch {
      // Not remembered: fine.
    }
  });

  /** The rendered page (a JPEG), once the engine has made it. */
  let url = $state<string | null>(null);
  let printing = $state(false);
  let dialog: HTMLDialogElement;

  const cm = (mm: number) => Math.round(mm) / 10;
  const number = (value: number, digits: number) =>
    new Intl.NumberFormat(getLocale(), { maximumFractionDigits: digits }).format(value);

  /** A new printed width or height in cm (fitting stops): the resolution follows. */
  function setSize(cmValue: number, side: "width" | "height") {
    if (!Number.isFinite(cmValue) || cmValue <= 0) return;
    const pixels = side === "width" ? width : height;
    settings.fit = false;
    settings.ppi = pixels / (cmValue / 2.54);
  }

  function setPpi(ppi: number) {
    if (!Number.isFinite(ppi) || ppi <= 0) return;
    settings.fit = false;
    settings.ppi = ppi;
  }

  function setPosition(cmValue: number, side: "left" | "top") {
    if (!Number.isFinite(cmValue)) return;
    settings.center = false;
    settings[side] = cmValue * 10;
  }

  /** The preview: the page fitted in a box of this many CSS pixels. */
  const PREVIEW = 280;
  const previewScale = $derived(PREVIEW / Math.max(layout.pageWidth, layout.pageHeight));

  async function print() {
    if (!url || printing) return;
    printing = true;
    try {
      await printImage(url, layout, title);
      onclose();
    } finally {
      printing = false;
    }
  }

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    let created: string | null = null;
    let closed = false;
    engine.printPage(documentId).then(
      (jpeg) => {
        if (closed) return;
        created = URL.createObjectURL(new Blob([jpeg], { type: "image/jpeg" }));
        url = created;
      },
      () => {
        // The preview stays empty; printing is not possible.
      },
    );
    return () => {
      closed = true;
      window.removeEventListener("keydown", isolate, true);
      if (created) URL.revokeObjectURL(created);
    };
  });
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="print-title"
  oncancel={(e) => {
    e.preventDefault();
    if (!printing) onclose();
  }}
>
  <header id="print-title">{t("print.title")}</header>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      void print();
    }}
  >
    <div class="body">
      <div class="preview" style:width="{PREVIEW}px" style:height="{PREVIEW}px">
        <div
          class="page"
          style:width="{layout.pageWidth * previewScale}px"
          style:height="{layout.pageHeight * previewScale}px"
        >
          {#if url}
            <img
              src={url}
              alt=""
              draggable="false"
              style:left="{layout.left * previewScale}px"
              style:top="{layout.top * previewScale}px"
              style:width="{layout.width * previewScale}px"
              style:height="{layout.height * previewScale}px"
            />
          {:else}
            <span class="loading">{t("print.preparing")}</span>
          {/if}
        </div>
      </div>
      <div class="fields">
        <label for="print-paper">{t("print.paper")}</label>
        <select
          id="print-paper"
          class="wide"
          value={settings.paper}
          onchange={(e) => (settings.paper = e.currentTarget.value as PaperId)}
        >
          {#each PAPERS as paper (paper.id)}
            <option value={paper.id}>{t(`print.paper.${paper.id}` as MessageKey)}</option>
          {/each}
        </select>

        <span class="label">{t("newDocument.orientation")}</span>
        <div class="orientation wide" role="radiogroup" aria-label={t("newDocument.orientation")}>
          {#each [false, true] as landscape (landscape)}
            {@const label = t(landscape ? "newDocument.landscape" : "newDocument.portrait")}
            <button
              type="button"
              role="radio"
              aria-checked={settings.landscape === landscape}
              class:selected={settings.landscape === landscape}
              title={label}
              aria-label={label}
              onclick={() => (settings.landscape = landscape)}
            >
              <Icon name={landscape ? "landscape" : "portrait"} size={16} />
            </button>
          {/each}
        </div>

        <span class="section">{t("print.size")}</span>
        <label class="check wide">
          <input type="checkbox" bind:checked={settings.fit} />
          {t("print.fit")}
        </label>
        <label for="print-width">{t("sizeDialog.width")}</label>
        <input
          id="print-width"
          type="number"
          min="0.1"
          step="0.1"
          value={cm(layout.width)}
          onchange={(e) => setSize(e.currentTarget.valueAsNumber, "width")}
        />
        <span class="unit">cm</span>
        <label for="print-height">{t("sizeDialog.height")}</label>
        <input
          id="print-height"
          type="number"
          min="0.1"
          step="0.1"
          value={cm(layout.height)}
          onchange={(e) => setSize(e.currentTarget.valueAsNumber, "height")}
        />
        <span class="unit">cm</span>
        <label for="print-ppi">{t("print.resolution")}</label>
        <input
          id="print-ppi"
          type="number"
          min="1"
          step="1"
          value={Math.round(layout.ppi)}
          onchange={(e) => setPpi(e.currentTarget.valueAsNumber)}
        />
        <span class="unit">{t("print.ppi")}</span>
        {#if layout.ppi < LOW_PPI}
          <p class="hint wide">
            {t("print.lowResolution", { ppi: number(layout.ppi, 0) })}
          </p>
        {/if}

        <span class="section">{t("print.position")}</span>
        <label class="check wide">
          <input type="checkbox" bind:checked={settings.center} disabled={settings.fit} />
          {t("print.center")}
        </label>
        <label for="print-top">{t("print.top")}</label>
        <input
          id="print-top"
          type="number"
          step="0.1"
          disabled={settings.fit}
          value={cm(layout.top)}
          onchange={(e) => setPosition(e.currentTarget.valueAsNumber, "top")}
        />
        <span class="unit">cm</span>
        <label for="print-left">{t("print.left")}</label>
        <input
          id="print-left"
          type="number"
          step="0.1"
          disabled={settings.fit}
          value={cm(layout.left)}
          onchange={(e) => setPosition(e.currentTarget.valueAsNumber, "left")}
        />
        <span class="unit">cm</span>
      </div>
    </div>
    <div class="buttons">
      <button type="button" class="btn" onclick={onclose} disabled={printing}>
        {t("sizeDialog.cancel")}
      </button>
      <button type="submit" class="btn primary" disabled={!url || printing}>
        {t("print.print")}
      </button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 640px;
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
    display: flex;
    gap: 16px;
    padding: 12px 10px;
  }

  .preview {
    display: grid;
    place-items: center;
    flex: none;
    background: var(--pasteboard);
    border: 1px solid var(--border-dark);
  }

  .page {
    position: relative;
    overflow: hidden;
    background: #ffffff;
    box-shadow: 0 2px 8px #0008;
  }

  .page img {
    position: absolute;
  }

  .loading {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: #777777;
    font-size: 12px;
  }

  .fields {
    display: grid;
    grid-template-columns: auto 90px auto;
    align-content: start;
    align-items: center;
    gap: 6px 10px;
    flex: 1;
    min-width: 0;
  }

  .fields > label,
  .label {
    color: var(--text-muted);
  }

  .wide {
    grid-column: 2 / -1;
  }

  .check {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .section {
    grid-column: 1 / -1;
    margin-top: 6px;
    font-weight: 600;
  }

  .hint {
    margin: 0;
    color: var(--text-muted);
    font-size: 12px;
  }

  .unit {
    color: var(--text-muted);
  }

  input[type="number"] {
    min-width: 0;
  }

  .orientation {
    display: flex;
    gap: 4px;
  }

  .orientation button {
    display: grid;
    place-items: center;
    width: 28px;
    height: 24px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 3px;
    background: var(--field);
    color: var(--text);
  }

  .orientation button.selected {
    border-color: var(--accent);
    background: var(--selected);
  }

  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>
