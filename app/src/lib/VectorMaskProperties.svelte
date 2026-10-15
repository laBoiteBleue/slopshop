<script lang="ts">
  // The Properties panel's section for a layer's vector mask (ADR 0041): its feather, in the
  // layer's pixels (0 to 250), as Photoshop's mask properties give it.
  import type { EditRequest, LayerView } from "./engine";
  import { t } from "./i18n/index.svelte";

  /** The largest feather (the engine's `MAX_FEATHER`). */
  const MAX_FEATHER = 250;

  let {
    documentId,
    layer,
    onedit,
  }: {
    documentId: number;
    /** A layer with a vector mask. */
    layer: LayerView;
    onedit: (documentId: number, edit: EditRequest) => void;
  } = $props();

  function setFeather(e: Event) {
    const input = e.currentTarget as HTMLInputElement;
    const value = input.valueAsNumber;
    if (!Number.isFinite(value)) {
      input.value = input.defaultValue;
      return;
    }
    const feather = Math.min(Math.max(value, 0), MAX_FEATHER);
    if (feather !== layer.vectorMask?.feather) {
      onedit(documentId, { kind: "setVectorMaskFeather", id: layer.id, feather });
    }
  }
</script>

{#if layer.vectorMask}
  <section class="panel" aria-label={t("properties.vectorMask")}>
    <div class="title"><span>{t("properties.vectorMask")}</span></div>
    <div class="row">
      <label for="vector-mask-feather">{t("properties.vectorMask.feather")}</label>
      <input
        id="vector-mask-feather"
        type="number"
        min="0"
        max={MAX_FEATHER}
        step="0.1"
        value={layer.vectorMask.feather}
        onchange={setFeather}
      />
      <span class="unit">px</span>
    </div>
  </section>
{/if}

<style>
  .panel {
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    background: var(--panel);
  }

  .title {
    padding: 6px 8px 2px;
    font-weight: 600;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 10px;
  }

  .row > label {
    width: 110px;
  }

  .row input {
    width: 64px;
  }

  .unit {
    color: var(--text-muted);
  }
</style>
