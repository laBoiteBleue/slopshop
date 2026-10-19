<script lang="ts">
  // The dock's Paths panel (Window > Paths, ADR 0041): see PathsPanel.
  import PathsPanel from "../PathsPanel.svelte";
  import type { EditRequest, PathRef } from "../engine";
  import { t } from "../i18n/index.svelte";
  import { nextPathName, pathEntries } from "../paths";
  import { panelContext } from "./context";

  const app = panelContext();

  const entries = $derived(pathEntries(app.doc, app.activeLayer));
  const pathName = (n: number) => t("paths.defaultName", { n });

  /** Add a Mask: the chosen document path masks the active layer (a vector mask). */
  const canMask = $derived(
    app.chosenPath?.kind === "path" && app.activeLayer !== null && !app.activeLayer.vectorMask,
  );

  /** `request` performed; then the path it added (the newest one) chosen. */
  async function adding(request: EditRequest) {
    const before = new Set((app.doc.paths ?? []).map((p) => p.id));
    await app.edit(app.doc.id, request);
    const added = (app.doc.paths ?? []).find((p) => !before.has(p.id));
    if (added) app.choosePath({ kind: "path", id: added.id });
  }

  function remove(ref: PathRef) {
    if (ref.kind !== "path") return;
    void app.edit(app.doc.id, { kind: "deletePath", id: ref.id });
    app.choosePath(null);
  }
</script>

<PathsPanel
  {entries}
  chosen={app.chosenPath}
  document={app.doc}
  selected={app.doc.selectionKey != null}
  {canMask}
  nextName={nextPathName(app.doc, pathName)}
  onchoose={app.choosePath}
  onload={app.loadPathSelection}
  onfill={app.fillPath}
  onstroke={app.strokePath}
  onworkpath={app.makeWorkPath}
  onmask={(ref) => {
    const layer = app.activeLayer;
    if (ref.kind !== "path" || !layer) return;
    void app.edit(app.doc.id, {
      kind: "addVectorMask",
      ids: [layer.id],
      mask: "path",
      path: ref.id,
    });
  }}
  onnew={() => void adding({ kind: "newPath", name: nextPathName(app.doc, pathName) })}
  ondelete={remove}
  onrename={(id, name) => void app.edit(app.doc.id, { kind: "renamePath", id, name })}
  onsave={(id, name) => void app.edit(app.doc.id, { kind: "savePath", id, name })}
  onduplicate={(from, name) => void adding({ kind: "duplicatePath", from, name })}
/>
