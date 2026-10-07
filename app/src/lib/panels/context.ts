// What every panel of the dock gets from the app: the active document, its active layer, and
// the app's ways to change them. A panel reads it with `panelContext()`; the app provides it
// once, so a new panel needs no wiring of its own in App.svelte.

import { getContext, setContext } from "svelte";
import type { DocumentView, EditRequest, LayerView, SelectionMode } from "../engine";
import type { CombinedRow } from "../savedSelections";

export type PanelContext = {
  /** The active document: the dock shows only with one. */
  readonly doc: DocumentView;
  /** Its active layer (the one the Layers panel shows active), if any. */
  readonly activeLayer: LayerView | null;
  /** A discrete edit (one undo entry); settles once applied. */
  edit(documentId: number, request: EditRequest): Promise<void>;
  /** A live edit within a gesture. */
  live(documentId: number, request: EditRequest): void;
  /** End of the gesture: everything since it started becomes one undo entry. */
  gestureEnd(documentId: number): Promise<void>;
  /** Apply what an engine call returns (the document after it), reporting failures. */
  sync(request: Promise<DocumentView | null>): Promise<void>;
  /** A selection command on the active document (a transform under way applied first). */
  selectionCommand(run: (documentId: number) => Promise<DocumentView>): void;
  /** A fill layer's color, chosen in the color picker. */
  pickFillColor(layer: LayerView): void;
  /** A vector layer's fill or stroke color chosen (ADR 0041). */
  pickShapeColor(layer: LayerView, which: "fill" | "stroke"): void;
  /** Another library pattern chosen for a pattern fill layer (ADR 0042). */
  pickPattern(layer: LayerView): void;
  /** Select > Save Selection… (it asks a name). */
  saveSelection(): void;
  /** A saved selection loaded into the image (Shift adds, Alt subtracts, both intersect). */
  loadSelection(id: number, mode: SelectionMode): void;
  /** The saved selections the image's selection is made of, and how. */
  readonly combinedSelections: CombinedRow[];
  /** The document point under the pointer over the image (document pixels), if any. */
  readonly pointer: [number, number] | null;
  /** Select these layers in the Layers panel (the topmost active). */
  selectLayers(ids: number[]): void;
  /** A new layer showing source `source` above the active layer, selected (ADR 0040). */
  newLayerFromSource(source: number, name: string): void;
};

/** The context's key (tests give panels a context of their own). */
export const PANEL_CONTEXT = Symbol("panels");

/** The app provides the panels' context (once, while it initializes). */
export function setPanelContext(context: PanelContext) {
  setContext(PANEL_CONTEXT, context);
}

/** A panel's context, from the app. */
export function panelContext(): PanelContext {
  const context = getContext<PanelContext | undefined>(PANEL_CONTEXT);
  if (!context) throw new Error("a panel is shown outside the app's dock");
  return context;
}
