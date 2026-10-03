<script lang="ts">
  // The Properties panel (Photoshop's): the parameters of the selected adjustment layer
  // (ADR 0020), as sliders with a number field each, checkboxes and color swatches. Dragging
  // applies live, one undo entry per drag; Reset puts the neutral values back.
  import type { EditRequest, LayerView } from "./engine";
  import AdjustmentFields from "./AdjustmentFields.svelte";
  import { t } from "./i18n/index.svelte";

  let {
    documentId,
    layer,
    onedit,
    onlive,
    ongestureend,
  }: {
    documentId: number;
    /** An adjustment layer. */
    layer: LayerView;
    onedit: (documentId: number, edit: EditRequest) => void;
    onlive: (documentId: number, edit: EditRequest) => void;
    ongestureend: (documentId: number) => void;
  } = $props();

  const adjustment = $derived(layer.adjustment);
  let fields = $state<AdjustmentFields | null>(null);

  /** The edit with these settings (Gradient Map: its stops, these or the layer's). */
  function request(values: number[], gradient?: number[][]): EditRequest | null {
    if (!adjustment) return null;
    const stops = gradient ?? adjustment.gradient ?? undefined;
    return {
      kind: "setAdjustment",
      id: layer.id,
      adjustment: adjustment.id,
      values,
      ...(stops ? { gradient: stops } : {}),
    };
  }

  /** Curves: the edit with these points (composite, red, green, blue). */
  function curvesRequest(curves: number[][][]): EditRequest {
    return { kind: "setAdjustment", id: layer.id, adjustment: "curves", values: [], curves };
  }

  function live(values: number[], gradient?: number[][]) {
    const edit = request(values, gradient);
    if (edit) onlive(documentId, edit);
  }

  function apply(values: number[], gradient?: number[][]) {
    const edit = request(values, gradient);
    if (edit) onedit(documentId, edit);
  }
</script>

{#if adjustment}
  <section class="panel" aria-label={t("properties.title")}>
    <div class="tabs"><span class="tab active">{t("properties.title")}</span></div>
    <div class="title">
      <span>{t(`adjustment.${adjustment.id}`)}</span>
      {#if fields?.resettable()}
        <button type="button" class="btn small" onclick={() => fields?.reset()}>
          {t("properties.reset")}
        </button>
      {/if}
    </div>
    {#if !fields?.resettable()}
      <p class="empty">{t("properties.noSettings")}</p>
    {/if}
    <AdjustmentFields
      bind:this={fields}
      {adjustment}
      owner={layer.id}
      onlive={live}
      onapply={apply}
      onend={() => ongestureend(documentId)}
      oncurveslive={(curves) => onlive(documentId, curvesRequest(curves))}
      oncurvesapply={(curves) => onedit(documentId, curvesRequest(curves))}
    />
  </section>
{/if}

<style>
  .panel {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    background: var(--panel);
    border-top: 1px solid var(--border-dark);
  }

  .tabs {
    display: flex;
    height: 26px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
  }

  .tab {
    display: flex;
    align-items: center;
    padding: 0 12px;
    color: var(--text-muted);
    font-weight: 600;
  }

  .tab.active {
    background: var(--panel);
    color: var(--text);
    border-right: 1px solid var(--border-dark);
  }

  .title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 8px 2px;
    font-weight: 600;
  }

  .empty {
    margin: 0;
    padding: 4px 8px 10px;
    color: var(--text-muted);
  }
</style>
