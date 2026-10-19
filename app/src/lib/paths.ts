// The Paths panel (Window > Paths, ADR 0041), as Photoshop's: the document's saved paths and
// its Work Path, then the active layer's shape path and vector mask path while it is active;
// which of them is chosen, what the Pen draws into and edits then, the names of new paths and
// the outlines their thumbnails show.
import * as affine from "./affine";
import type { DocumentView, LayerView, Matrix, PathRef, PathView } from "./engine";
import type { ShapeGeometry } from "./shapes";
import { editablePath, layerToDocument, type EditablePath } from "./pathEdit";
import { svgOfPath, type PathGeometry } from "./pen";
import { editableVectorMask } from "./vectorMask";

/** The Pen draws shape layers (Photoshop's Shape mode) or paths (Path mode). */
export type PenMode = "shape" | "path";

/** A row of the Paths panel. */
export type PathEntry = {
  ref: PathRef;
  /** A saved path, the Work Path, or the active layer's shape path or vector mask. */
  kind: "path" | "work" | "shape" | "vectorMask";
  /** A saved path's name; the layer's name for a layer's (the UI composes the row's name). */
  name: string;
  /** Its outline in document pixels (SVG path data), for the thumbnail. */
  outline: string;
  evenOdd: boolean;
};

/** A path chosen in the panel, of document `documentId`. */
export type PathChoice = { documentId: number; ref: PathRef };

export function sameRef(a: PathRef | null, b: PathRef | null): boolean {
  if (!a || !b || a.kind !== b.kind) return false;
  return a.kind === "path" ? a.id === (b as typeof a).id : a.layer === (b as typeof a).layer;
}

/** A layer's path as anchors show it: its path, or a live shape's as a path. */
function shownPath(
  own: ShapeGeometry,
  live: ShapeGeometry | null | undefined,
): PathGeometry | null {
  const geometry = own?.kind === "path" ? own : live;
  return geometry?.kind === "path" ? geometry : null;
}

function outlineOf(geometry: PathGeometry | null, matrix: Matrix | null): string {
  if (!geometry) return "";
  const m = matrix ?? affine.IDENTITY;
  return svgOfPath(geometry, (x, y) => affine.apply(m, x, y));
}

/**
 * The rows of the Paths panel for `doc` with `active` the active layer: the document's paths in
 * their order (the Work Path where it is), then `active`'s shape path (a vector layer's) and its
 * vector mask path.
 */
export function pathEntries(doc: DocumentView, active: LayerView | null): PathEntry[] {
  const entries: PathEntry[] = (doc.paths ?? []).map((p) => ({
    ref: { kind: "path", id: p.id },
    kind: p.work ? "work" : "path",
    name: p.name,
    outline: outlineOf(p.geometry, null),
    evenOdd: p.geometry.rule === "evenodd",
  }));
  if (!active) return entries;
  const matrix = layerToDocument(doc.layers, active.id);
  if (active.kind === "vector" && active.shape && !active.text) {
    const geometry = shownPath(active.shape.geometry, active.livePath);
    entries.push({
      ref: { kind: "shape", layer: active.id },
      kind: "shape",
      name: active.name,
      outline: outlineOf(geometry, matrix),
      evenOdd: geometry?.rule === "evenodd",
    });
  }
  if (active.vectorMask) {
    const geometry = shownPath(active.vectorMask.geometry, active.vectorMask.livePath);
    entries.push({
      ref: { kind: "vectorMask", layer: active.id },
      kind: "vectorMask",
      name: active.name,
      outline: outlineOf(geometry, matrix),
      evenOdd: geometry?.rule === "evenodd",
    });
  }
  return entries;
}

/** The path chosen in the panel, while it is still listed for `doc` (else none). */
export function chosenPath(
  choice: PathChoice | null,
  doc: DocumentView | null,
  active: LayerView | null,
): PathRef | null {
  if (!choice || !doc || choice.documentId !== doc.id) return null;
  return pathEntries(doc, active).some((e) => sameRef(e.ref, choice.ref)) ? choice.ref : null;
}

/** Document path `id` of `doc`. */
export function documentPath(doc: DocumentView, id: number): PathView | null {
  return doc.paths?.find((p) => p.id === id) ?? null;
}

/** What the Pen edits of document path `path`: its subpaths, in document pixels. */
export function editableDocumentPath(path: PathView): EditablePath {
  return {
    layerId: 0,
    geometry: path.geometry,
    live: false,
    matrix: affine.IDENTITY,
    pathId: path.id,
  };
}

/**
 * What the Pen edits: the path chosen in the Paths panel (a document path, or the active
 * layer's vector mask path or shape path), else, as before, the active layer's vector mask
 * when it is targeted (`masked`), else the active vector layer's path.
 */
export function penEditable(
  doc: DocumentView,
  active: LayerView | null,
  chosen: PathRef | null,
  masked: boolean,
): EditablePath | null {
  if (chosen?.kind === "path") {
    const path = documentPath(doc, chosen.id);
    return path ? editableDocumentPath(path) : null;
  }
  if (chosen?.kind === "vectorMask" || (!chosen && masked)) {
    return editableVectorMask(doc.layers, active);
  }
  return editablePath(doc.layers, active);
}

/**
 * Where a path the Pen finished goes: to the vector mask edited (either mode, as Photoshop's
 * Pen in a targeted vector mask); in Path mode, added to the chosen document path, else a new
 * Work Path; in Shape mode, a new shape layer.
 */
export type PenDestination =
  | { kind: "path"; id: number }
  | { kind: "vectorMask" }
  | { kind: "workPath" }
  | { kind: "shapeLayer" };

export function penDestination(editable: EditablePath | null, mode: PenMode): PenDestination {
  if (editable?.vectorMask) return { kind: "vectorMask" };
  if (mode === "shape") return { kind: "shapeLayer" };
  if (editable?.pathId !== undefined) return { kind: "path", id: editable.pathId };
  return { kind: "workPath" };
}

/** `path`'s subpaths with those of `drawn` (document pixels) after them. */
export function withSubpaths(path: PathGeometry, drawn: PathGeometry): PathGeometry {
  return { ...path, subpaths: [...path.subpaths, ...drawn.subpaths] };
}

/** The first of "Path 1", "Path 2"… (`format(n)`) that no saved path of `doc` uses. */
export function nextPathName(doc: DocumentView, format: (n: number) => string): string {
  const used = new Set((doc.paths ?? []).filter((p) => !p.work).map((p) => p.name));
  let n = 1;
  while (used.has(format(n))) n++;
  return format(n);
}

/** How a Ctrl+click on a path's thumbnail combines with the selection (Photoshop's keys). */
export function loadMode(e: { shiftKey: boolean; altKey: boolean }) {
  if (e.shiftKey && e.altKey) return "intersect" as const;
  if (e.shiftKey) return "add" as const;
  if (e.altKey) return "subtract" as const;
  return "replace" as const;
}

/** Photoshop's Make Work Path tolerance: 0.5 to 10 pixels, 2 by default. */
export const MIN_TOLERANCE = 0.5;
export const MAX_TOLERANCE = 10;
export const DEFAULT_TOLERANCE = 2;
