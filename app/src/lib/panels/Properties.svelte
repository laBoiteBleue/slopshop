<script lang="ts">
  // The dock's Properties panel: the settings of the active layer when it has some (an
  // adjustment, a fill or a vector layer, ADR 0020, 0041), else a hint.
  import PropertiesPanel from "../PropertiesPanel.svelte";
  import { hasProperties } from "../layerEdits";
  import { t } from "../i18n/index.svelte";
  import { panelContext } from "./context";

  const app = panelContext();
  const layer = $derived(hasProperties(app.activeLayer) ? app.activeLayer : null);
</script>

{#if layer}
  <PropertiesPanel
    documentId={app.doc.id}
    {layer}
    size={app.doc}
    onfillcolor={app.pickFillColor}
    onshapecolor={app.pickShapeColor}
    onpattern={app.pickPattern}
    onedit={app.edit}
    onlive={app.live}
    ongestureend={app.gestureEnd}
  />
{:else}
  <p class="empty">{t("properties.empty")}</p>
{/if}

<style>
  .empty {
    margin: 0;
    padding: 10px;
    color: var(--text-muted);
  }
</style>
