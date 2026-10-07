// The Sources panel's rules (ADR 0040): what pixel layers show, kept once and shared by the
// layers duplicated from it. Make Unique gives a layer a source of its own.

import type { DocumentView, EditRequest, LayerView, SourceView } from "./engine";

/** Every layer of `layers`, groups' layers included (depth first). */
function flatten(layers: LayerView[]): LayerView[] {
  return layers.flatMap((layer) => [layer, ...flatten(layer.children)]);
}

/**
 * The name a source shows: its own (an opened file's), else the first layer's showing it (pixels
 * pasted or baked have none of their own).
 */
export function sourceName(source: SourceView, layers: LayerView[]): string {
  if (source.name) return source.name;
  const first = flatten(layers).find((layer) => layer.id === source.layers[0]);
  return first?.name ?? "";
}

/** The source `layer` shows, among the document's, if any. */
export function sourceOf(doc: DocumentView | null, layer: LayerView | null): SourceView | null {
  if (!doc || layer?.source == null) return null;
  return doc.sources?.find((source) => source.id === layer.source) ?? null;
}

/** The layers of `selection` sharing their source with other layers: what Make Unique changes. */
export function sharingLayers(doc: DocumentView | null, selection: LayerView[]): LayerView[] {
  return selection.filter((layer) => (sourceOf(doc, layer)?.layers.length ?? 0) > 1);
}

/** Layer > Make Unique on `selection`, or null when none of them shares its source. */
export function makeUniqueEdit(
  doc: DocumentView | null,
  selection: LayerView[],
): EditRequest | null {
  const ids = sharingLayers(doc, selection).map((layer) => layer.id);
  return ids.length > 0 ? { kind: "makeUnique", ids } : null;
}
