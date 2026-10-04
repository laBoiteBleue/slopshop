<script lang="ts">
  // The dock's Info panel (ADR 0036): the color shown under the pointer (8-bit sRGB, as the
  // eyedropper reads it) and where it is, the selection's size and place, the document's size.
  // The engine is asked only while the panel shows (one request at a time as the pointer moves).
  import { engine, type Bounds } from "../engine";
  import { srgbToHex } from "../color";
  import { t } from "../i18n/index.svelte";
  import { latestWins } from "../latest";
  import { panelContext } from "./context";

  const app = panelContext();

  /** The pixel under the pointer, when it is over the canvas. */
  const pixel = $derived.by(() => {
    const point = app.pointer;
    if (!point) return null;
    const [x, y] = point.map(Math.floor);
    return x >= 0 && y >= 0 && x < app.doc.width && y < app.doc.height ? { x, y } : null;
  });

  let color = $state<[number, number, number] | null>(null);
  const sampler = latestWins(async (at: { documentId: number; x: number; y: number }) => {
    const sampled = await engine.sampleColor(at.documentId, at.x + 0.5, at.y + 0.5);
    if (at.documentId === app.doc.id) color = sampled;
  });
  $effect(() => {
    void app.doc.revision;
    if (pixel) sampler.push({ documentId: app.doc.id, ...pixel });
    else color = null;
  });

  let selection = $state<Bounds | null>(null);
  const bounds = latestWins(async (documentId: number) => {
    const found = await engine.selectionBounds(documentId);
    if (documentId === app.doc.id) selection = found;
  });
  $effect(() => {
    if (app.doc.selectionKey == null) selection = null;
    else bounds.push(app.doc.id);
  });
  $effect(() => () => {
    sampler.drop();
    bounds.drop();
  });

  const NONE = "—";
</script>

<section class="info" aria-label={t("info.title")}>
  <dl class="grid">
    <dt>{t("info.red")}</dt>
    <dd>{color?.[0] ?? NONE}</dd>
    <dt>{t("info.x")}</dt>
    <dd>{pixel?.x ?? NONE}</dd>
    <dt>{t("info.green")}</dt>
    <dd>{color?.[1] ?? NONE}</dd>
    <dt>{t("info.y")}</dt>
    <dd>{pixel?.y ?? NONE}</dd>
    <dt>{t("info.blue")}</dt>
    <dd>{color?.[2] ?? NONE}</dd>
    <dt>{t("info.hex")}</dt>
    <dd>{color ? srgbToHex(color.map((v) => v / 255)) : NONE}</dd>
  </dl>
  <dl class="sizes">
    <dt>{t("info.selection")}</dt>
    <dd>
      {#if selection}
        {t("info.size", {
          width: selection.right - selection.left,
          height: selection.bottom - selection.top,
        })}
        <span class="muted">{t("info.at", { x: selection.left, y: selection.top })}</span>
      {:else}
        {NONE}
      {/if}
    </dd>
    <dt>{t("info.document")}</dt>
    <dd>{t("info.size", { width: app.doc.width, height: app.doc.height })}</dd>
  </dl>
</section>

<style>
  .info {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 10px;
  }

  dl {
    display: grid;
    gap: 2px 8px;
    margin: 0;
  }

  .grid {
    grid-template-columns: auto 1fr auto 1fr;
  }

  .sizes {
    grid-template-columns: auto 1fr;
    padding-top: 6px;
    border-top: 1px solid var(--border-dark);
  }

  dt,
  .muted {
    color: var(--text-muted);
  }

  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }
</style>
