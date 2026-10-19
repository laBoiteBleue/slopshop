// What every panel of the dock gets from the app: the active document, its active layer, and
// the app's ways to change them. A panel reads it with `panelContext()`; the app provides it
// once, so a new panel needs no wiring of its own in App.svelte.

import { getContext, setContext } from "svelte";
import type {
  CharStyleDto,
  DocumentView,
  EditRequest,
  FontsView,
  LayerView,
  PathRef,
  LutView,
  SelectionMode,
  TextAlign,
  TextDto,
} from "../engine";
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
  /** Color Lookup's Load 3D LUT: a `.cube` or `.3dl` file chosen and read (null: cancelled or
   * refused, the error shown). */
  loadColorLookup(): Promise<LutView | null>;
  /** The active text layer being typed: its text as typed and the characters selected. */
  readonly typedText: {
    text: TextDto;
    selection: [number, number];
    pending: CharStyleDto | null;
  } | null;
  /** The fonts to choose from, once listed (a text layer or the Type tool shown). */
  readonly fonts: FontsView | null;
  /** The styles a font family offers. */
  fontStyles(family: string): Promise<{ weight: number; italic: boolean }[]>;
  /** A style on the characters selected, or on the whole active text layer (ADR 0041). */
  styleText(change: CharStyleDto): void;
  /** The active text layer's alignment. */
  alignText(align: TextAlign): void;
  /** The color of the characters selected (or of the whole text), chosen in the color picker. */
  pickTextColor(): void;
  /** Delete a source and the layers showing it, once the user confirmed (ADR 0040). */
  deleteSource(source: number, name: string): void;
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
  /** The path chosen in the Paths panel (ADR 0041), while it is listed. */
  readonly chosenPath: PathRef | null;
  /** Choose a path in the Paths panel (none: `null`). */
  choosePath(ref: PathRef | null): void;
  /** Paths panel > Load Path as Selection (Ctrl+click: combined by `mode`). */
  loadPathSelection(ref: PathRef, mode: SelectionMode): void;
  /** Fill Path: with the foreground color, or through Edit > Fill's dialog. */
  fillPath(ref: PathRef, dialog: boolean): void;
  /** Stroke Path: with the tool chosen last, or through its dialog. */
  strokePath(ref: PathRef, dialog: boolean): void;
  /** Make Work Path from Selection: at the last tolerance, or through its dialog. */
  makeWorkPath(dialog: boolean): void;
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
