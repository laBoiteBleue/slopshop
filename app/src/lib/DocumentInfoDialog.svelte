<script lang="ts">
  // File > Document Info (Alt+Shift+Ctrl+I): what the document is made of and the files it
  // comes from, read-only. Photoshop's File Info edits metadata (XMP), which SlopShop does not
  // keep yet: this shows what it does know.
  import { onMount } from "svelte";
  import type { DocumentInfo, FileInfo } from "./engine";
  import { formatBytes } from "./format";
  import { getLocale, t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";
  import type { MessageKey } from "./i18n/en";

  let { info, onclose }: { info: DocumentInfo; onclose: () => void } = $props();

  let dialog: HTMLDialogElement;

  const number = (value: number, digits = 0) =>
    new Intl.NumberFormat(getLocale(), { maximumFractionDigits: digits }).format(value);

  const megapixels = $derived((info.width * info.height) / 1e6);

  const LAYER_KINDS: { key: keyof DocumentInfo["layers"]; label: MessageKey }[] = [
    { key: "raster", label: "documentInfo.raster" },
    { key: "fill", label: "documentInfo.fill" },
    { key: "vector", label: "documentInfo.vector" },
    { key: "adjustment", label: "documentInfo.adjustment" },
    { key: "group", label: "documentInfo.group" },
    { key: "masks", label: "documentInfo.masks" },
  ];

  function spaceName(id: string): string {
    return t(`colorSpace.${id}` as MessageKey);
  }

  function fileSize(file: FileInfo): string {
    return file.bytes === null ? t("documentInfo.missing") : formatBytes(file.bytes);
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
  aria-labelledby="document-info-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <form
    onsubmit={(e) => {
      e.preventDefault();
      onclose();
    }}
  >
    <header id="document-info-title" {@attach movable("document-info")}>
      {t("documentInfo.title")}
    </header>
    <dl>
      <dt>{t("documentInfo.name")}</dt>
      <dd>{info.name ?? t("document.untitled")}</dd>

      <dt>{t("documentInfo.size")}</dt>
      <dd>
        {t("documentInfo.sizeValue", {
          width: number(info.width),
          height: number(info.height),
          megapixels: number(megapixels, megapixels < 10 ? 1 : 0),
        })}
      </dd>

      <dt>{t("sizeDialog.resolution")}</dt>
      <dd>
        {t("documentInfo.resolutionValue", {
          ppi: number(info.resolution, 2),
          width: number((info.width / info.resolution) * 2.54, 1),
          height: number((info.height / info.resolution) * 2.54, 1),
        })}
      </dd>

      <dt>{t("documentInfo.colorSpace")}</dt>
      <dd>{spaceName(info.workingSpace)}</dd>

      <dt>{t("layers.blendSpace")}</dt>
      <dd>{t(`layers.blendSpace.${info.blendSpace}` as MessageKey)}</dd>

      {#each LAYER_KINDS.filter((kind) => info.layers[kind.key] > 0) as kind (kind.key)}
        <dt>{t(kind.label)}</dt>
        <dd>{number(info.layers[kind.key])}</dd>
      {/each}

      {#if info.formats.length > 0}
        <dt>{t("documentInfo.formats")}</dt>
        <dd>
          {#each info.formats as format, i (i)}
            <div>
              {t(format.float ? "documentInfo.bitsFloat" : "documentInfo.bits", {
                bits: format.bits,
              })}
              {t(`documentInfo.channels.${format.channels}` as MessageKey)} · {spaceName(
                format.space,
              )}
              <span class="muted">× {number(format.layers)}</span>
            </div>
          {/each}
        </dd>
      {/if}

      <dt>{t("documentInfo.memory")}</dt>
      <dd>{formatBytes(info.memoryBytes)}</dd>

      {#if info.source}
        <dt>{t("documentInfo.source")}</dt>
        <dd class="path">
          {info.source.path}
          <span class="muted">({fileSize(info.source)})</span>
        </dd>
      {/if}
      {#if info.file && info.file.path !== info.source?.path}
        <dt>{t("documentInfo.file")}</dt>
        <dd class="path">
          {info.file.path}
          <span class="muted">({fileSize(info.file)})</span>
        </dd>
      {/if}
    </dl>
    <footer>
      <!-- svelte-ignore a11y_autofocus -->
      <button type="submit" class="btn primary" autofocus>{t("sizeDialog.ok")}</button>
    </footer>
  </form>
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

  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 6px 14px;
    margin: 0;
    padding: 12px 10px;
  }

  dt {
    color: var(--text-muted);
  }

  dd {
    margin: 0;
    min-width: 0;
  }

  .path {
    overflow-wrap: anywhere;
    user-select: text;
  }

  .muted {
    color: var(--text-muted);
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border-dark);
  }
</style>
