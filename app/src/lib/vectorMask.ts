// Vector masks (ADR 0041): Layer > Vector Mask's commands on the selected layers, the path the
// Pen edits when a layer's vector mask is targeted, the paths it adds to it, and its outline
// for the thumbnail.
import * as affine from "./affine";
import type { EditRequest, LayerView, Matrix, VectorMaskKind } from "./engine";
import { batchOrNone } from "./layerEdits";
import {
  layerToDocument,
  mappedPath,
  pathsOf,
  geometryOfPaths,
  type EditablePath,
} from "./pathEdit";
import { svgOfPath, type PathGeometry } from "./pen";

/**
 * The vector layer whose path Layer > Vector Mask > Current Path takes: with two layers
 * selected, the one that is not active, when it is a vector layer (its path masks the active
 * one). Null otherwise.
 */
export function currentPathSource(
  selection: LayerView[],
  active: LayerView | null,
): LayerView | null {
  if (!active || selection.length !== 2) return null;
  const other = selection.find((l) => l.id !== active.id);
  return other?.kind === "vector" && other.shape ? other : null;
}

/**
 * Layer > Vector Mask > Reveal All, Hide All (on the selected layers without a vector mask) or
 * Current Path (on the active layer, from `currentPathSource`); null when nothing would change.
 */
export function vectorMaskAddition(
  selection: LayerView[],
  active: LayerView | null,
  kind: VectorMaskKind,
): EditRequest | null {
  if (kind === "currentPath") {
    const from = currentPathSource(selection, active);
    if (!from || !active || active.vectorMask) return null;
    return { kind: "addVectorMask", ids: [active.id], mask: kind, from: from.id };
  }
  const ids = selection.filter((l) => !l.vectorMask).map((l) => l.id);
  return ids.length > 0 ? { kind: "addVectorMask", ids, mask: kind } : null;
}

/** The vector mask the menu speaks of: the active layer's, else the first selected one's. */
export function referenceVectorMask(
  selection: LayerView[],
  active: LayerView | null,
): LayerView["vectorMask"] {
  return active?.vectorMask ?? selection.find((l) => l.vectorMask)?.vectorMask ?? null;
}

/** Disable the vector masks of `selection`, or enable them all when the reference is off. */
export function vectorMaskEnabledToggle(
  selection: LayerView[],
  active: LayerView | null,
): EditRequest | null {
  const enabled = referenceVectorMask(selection, active)?.enabled === false;
  return batchOrNone(
    selection
      .filter((l) => l.vectorMask && l.vectorMask.enabled !== enabled)
      .map((l): EditRequest => ({ kind: "setVectorMaskEnabled", id: l.id, enabled })),
  );
}

/** Delete the vector masks of `layers`. */
export function vectorMaskRemoval(layers: LayerView[]): EditRequest | null {
  return batchOrNone(
    layers
      .filter((l) => l.vectorMask)
      .map((l): EditRequest => ({ kind: "removeVectorMask", id: l.id })),
  );
}

/** A vector mask's path as anchors can show it: its path, or a live shape's as a path. */
function maskPath(mask: NonNullable<LayerView["vectorMask"]>): PathGeometry | null {
  const own = mask.geometry;
  const geometry = own.kind === "path" ? own : mask.livePath;
  return geometry?.kind === "path" ? geometry : null;
}

/** What the Pen edits of `layer`'s vector mask (targeted), null without one. */
export function editableVectorMask(
  layers: LayerView[],
  layer: LayerView | null,
): EditablePath | null {
  const mask = layer?.vectorMask;
  if (!layer || !mask) return null;
  const geometry = maskPath(mask);
  if (!geometry) return null;
  return {
    layerId: layer.id,
    geometry,
    live: mask.geometry.kind !== "path",
    matrix: layerToDocument(layers, layer.id),
    vectorMask: true,
  };
}

/**
 * `target`'s vector mask path with the paths of `drawn` (document pixels, the Pen's) added, in
 * the layer's space; null when the layer is not placed by an affine map.
 */
export function withPathAdded(target: EditablePath, drawn: PathGeometry): PathGeometry | null {
  const inverse = target.matrix ? affine.invert(target.matrix) : null;
  if (!inverse) return null;
  const local = pathsOf(drawn).map((p) => mappedPath(p, (q) => affine.apply(inverse, q[0], q[1])));
  return {
    kind: "path",
    rule: target.geometry.rule,
    subpaths: [...target.geometry.subpaths, ...geometryOfPaths(local, drawn.rule).subpaths],
  };
}

/** A vector mask's outline in the document (SVG path data), for its thumbnail. */
export function maskOutline(
  mask: NonNullable<LayerView["vectorMask"]>,
  matrix: Matrix | null,
): string {
  const geometry = maskPath(mask);
  if (!geometry) return "";
  const m = matrix ?? affine.IDENTITY;
  return svgOfPath(geometry, (x, y) => affine.apply(m, x, y));
}
