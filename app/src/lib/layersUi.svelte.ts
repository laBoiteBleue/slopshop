// What the Layers panel remembers of a document: its selection, folded groups, unfolded stacks,
// mask targets and scroll. UI state, neither undoable nor saved, kept as long as the document is
// open rather than as long as a panel shows it: the panel is recreated for each tab, and
// switching tabs back finds everything as it was left.

import { NO_LAYERS, type LayerSelection } from "./layerSelection";

export class LayersUi {
  /** Selected layers, the active one and the range anchor (rules in layerSelection.ts). */
  selection = $state<LayerSelection>(NO_LAYERS);
  /** The layers the selection last saw (listed ones): layers added since are told apart. */
  known: ReadonlySet<number> = new Set();
  /** Groups folded (ADR 0015). */
  collapsed = $state<ReadonlySet<number>>(new Set());
  /** Layers whose stack entries are listed below them (ADR 0029; folded at first). */
  unfolded = $state<ReadonlySet<number>>(new Set());
  /**
   * Layers whose mask, rather than their pixels, is what painting reaches (as in Photoshop: a
   * click on a thumbnail chooses).
   */
  maskTargets = $state<ReadonlySet<number>>(new Set());
  /** How far the list is scrolled, CSS pixels. */
  scroll = 0;
}

/** The `LayersUi` of each open document, made on first use. */
export class LayersUis {
  #byDocument = new Map<number, LayersUi>();

  of(documentId: number): LayersUi {
    let ui = this.#byDocument.get(documentId);
    if (!ui) {
      ui = new LayersUi();
      this.#byDocument.set(documentId, ui);
    }
    return ui;
  }

  /** Forget the documents not in `open` (closed tabs). */
  keep(open: Iterable<number>) {
    const kept = new Set(open);
    for (const id of [...this.#byDocument.keys()]) {
      if (!kept.has(id)) this.#byDocument.delete(id);
    }
  }
}
