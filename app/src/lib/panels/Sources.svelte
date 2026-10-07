<script lang="ts">
  // The dock's Sources panel: what the pixel layers show, kept once (see SourcesPanel).
  import SourcesPanel from "../SourcesPanel.svelte";
  import { sourceName } from "../sources";
  import { panelContext } from "./context";

  const app = panelContext();
</script>

<SourcesPanel
  documentId={app.doc.id}
  sources={app.doc.sources ?? []}
  name={(source) => sourceName(source, app.doc.layers)}
  active={app.activeLayer?.source ?? null}
  onselect={app.selectLayers}
  onnewlayer={(source) => app.newLayerFromSource(source.id, sourceName(source, app.doc.layers))}
/>
