<script lang="ts">
  // The dock's Selections panel: the document's saved selections (see SelectionsPanel).
  import SelectionsPanel from "../SelectionsPanel.svelte";
  import { engine } from "../engine";
  import { panelContext } from "./context";

  const app = panelContext();
</script>

<SelectionsPanel
  saved={app.doc.savedSelections}
  selected={app.doc.selectionKey != null}
  onload={app.loadSelection}
  combined={app.combinedSelections}
  onsave={app.saveSelection}
  onreplace={(id) => app.selectionCommand((doc) => engine.saveSelection(doc, "", id))}
  onrename={(id, name) => void app.sync(engine.renameSavedSelection(app.doc.id, id, name))}
  ondelete={(id) => void app.sync(engine.deleteSavedSelection(app.doc.id, id))}
  ondeselect={() => app.selectionCommand(engine.deselect)}
  canReselect={app.doc.canReselect}
  onreselect={() => app.selectionCommand(engine.reselect)}
/>
