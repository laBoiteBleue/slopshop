// Layer > Bake to Pixels (ADR 0031): when its commands apply. The engine does the baking
// (slopshop_core::bake), on a worker; these rules gray the menu entries.

import type { LayerView } from "./engine";
import { childrenOf, outermost, type LayerTree } from "./layerTree";

/** A solid color or gradient fill layer. */
const isFill = (l: LayerView) => l.kind === "fill" || l.kind === "gradientFill";

/** Rasterize applies to a fill, a group, or a pixel layer carrying paint or effects. */
export function canRasterize(layers: LayerView[]): boolean {
  return layers.some((l) => isFill(l) || l.kind === "group" || l.painted);
}

/**
 * What Merge (Ctrl+E) does with `layers`: several merge together; one merges down onto the
 * visible layer right below it in its group (as in Photoshop); null when it cannot.
 */
export function mergeKind(tree: LayerTree, layers: LayerView[]): "layers" | "down" | null {
  const ids = outermost(
    tree,
    layers.map((l) => l.id),
  );
  if (ids.length > 1) return "layers";
  if (ids.length === 0) return null;
  const siblings = childrenOf(tree, tree.parents.get(ids[0]) ?? null);
  const below = siblings[siblings.findIndex((l) => l.id === ids[0]) - 1];
  return below?.visible ? "down" : null;
}

/** Merge Visible applies with two visible layers at the top level, or one group or fill. */
export function canMergeVisible(layers: LayerView[]): boolean {
  const visible = layers.filter((l) => l.visible);
  return visible.length > 1 || visible.some((l) => l.kind === "group" || isFill(l));
}

/** Flatten Image applies unless the document is a single plain pixel layer already. */
export function canFlatten(layers: LayerView[]): boolean {
  if (layers.length !== 1) return layers.length > 1;
  const [only] = layers;
  return only.kind !== "raster" || only.painted || only.mask !== null || !only.visible;
}
