<script lang="ts">
  // The dock's Properties panel: the settings of the active layer when it has some (an
  // adjustment, a fill, a vector or a text layer, ADR 0020, 0041) and of its vector mask, else
  // a hint.
  import PropertiesPanel from "../PropertiesPanel.svelte";
  import VectorMaskProperties from "../VectorMaskProperties.svelte";
  import { hasProperties } from "../layerEdits";
  import { t } from "../i18n/index.svelte";
  import { panelContext } from "./context";

  const app = panelContext();
  const layer = $derived(hasProperties(app.activeLayer) ? app.activeLayer : null);
  const masked = $derived(app.activeLayer?.vectorMask ? app.activeLayer : null);
</script>

{#if layer}
  <PropertiesPanel
    documentId={app.doc.id}
    {layer}
    layers={app.doc.layers}
    size={app.doc}
    onfillcolor={app.pickFillColor}
    onshapecolor={app.pickShapeColor}
    onpattern={app.pickPattern}
    typedText={app.typedText}
    ppi={app.doc.resolution}
    fonts={app.fonts}
    stylesOf={app.fontStyles}
    onstyletext={app.styleText}
    onaligntext={app.alignText}
    ontextcolor={app.pickTextColor}
    loadLut={app.loadColorLookup}
    onedit={app.edit}
    onlive={app.live}
    ongestureend={app.gestureEnd}
  />
{/if}
{#if masked}
  <VectorMaskProperties documentId={app.doc.id} layer={masked} onedit={app.edit} />
{/if}
{#if !layer && !masked}
  <p class="empty">{t("properties.empty")}</p>
{/if}

<style>
  .empty {
    margin: 0;
    padding: 10px;
    color: var(--text-muted);
  }
</style>
