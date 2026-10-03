// The layer tree as the panels see it (ADR 0015): plain functions over the engine's
// `LayerView` tree, by stable ids. Layers of a level are listed bottom to top, as the engine
// lists them; the panel shows them top to bottom.

import type { LayerView } from "./engine";

/** A row of the Layers panel. */
export type Row = {
  layer: LayerView;
  /** Groups around the layer. */
  depth: number;
  /** Its group, null at the top level. */
  parent: number | null;
  /** Its index among its siblings (0 = bottom). */
  index: number;
  /** Visible, and so are all its groups. */
  shown: boolean;
  /** Clipped to the layer below it, or the base of the clipped layers above it (ADR 0016). */
  clipping: "clipped" | "base" | null;
};

/**
 * Rows as displayed, top to bottom: each group above its layers, unless `collapsed`. `hidden`
 * layers are not listed (with their layers).
 */
export function flattenRows(
  layers: LayerView[],
  collapsed: ReadonlySet<number>,
  hidden: readonly number[],
): Row[] {
  const out: Row[] = [];
  const visit = (level: LayerView[], depth: number, parent: number | null, shown: boolean) => {
    for (let index = level.length - 1; index >= 0; index--) {
      const layer = level[index];
      if (hidden.includes(layer.id)) continue;
      const visible = shown && layer.visible;
      // A clipped layer needs a layer below it; the base is the one the clipped layers rest on.
      const clipped = layer.clipped && index > 0;
      const base = !clipped && level[index + 1]?.clipped === true;
      const clipping = clipped ? "clipped" : base ? "base" : null;
      out.push({ layer, depth, parent, index, shown: visible, clipping });
      // A group being baked is shown as the layer it becomes (ADR 0030).
      if (layer.kind === "group" && !layer.baking && !collapsed.has(layer.id)) {
        visit(layer.children, depth + 1, layer.id, visible);
      }
    }
  };
  visit(layers, 0, null, true);
  return out;
}

/** How close to the line between two rows an Alt+click is on it, CSS pixels. */
const CLIP_LINE = 7;

/**
 * An Alt+click on row `row`, `fromTop` and `fromBottom` CSS pixels from its edges: on the line
 * with the row above or below it, the upper of the two layers, whose clipping it toggles (as in
 * Photoshop); null elsewhere, or between layers of different levels.
 */
export function clipLineAt(
  rows: Row[],
  row: number,
  fromTop: number,
  fromBottom: number,
): LayerView | null {
  const upper = fromTop < CLIP_LINE ? row - 1 : fromBottom < CLIP_LINE ? row : null;
  if (upper === null) return null;
  const [above, below] = [rows[upper], rows[upper + 1]];
  if (!above || !below || above.parent !== below.parent || above.depth !== below.depth) {
    return null;
  }
  return above.layer;
}

/** Every layer, depth first, each group before its layers, bottom to top (as the engine). */
export function walk(layers: LayerView[]): LayerView[] {
  return layers.flatMap((layer) => [layer, ...walk(layer.children)]);
}

/** The layer `id`, at any depth. */
export function findLayer(layers: LayerView[], id: number): LayerView | null {
  for (const layer of layers) {
    if (layer.id === id) return layer;
    const inside = findLayer(layer.children, id);
    if (inside) return inside;
  }
  return null;
}

/** The pixel layers seen through visible groups (what Image > Adjustments changes). */
export function visibleRasters(layers: LayerView[]): number[] {
  return layers
    .filter((l) => l.visible)
    .flatMap((l) => (l.kind === "raster" ? [l.id] : visibleRasters(l.children)));
}

/** `layers`, or layers inside them, carry paint (ADR 0027). */
export function carriesPaint(layers: LayerView[]): boolean {
  return layers.some((l) => l.painted || carriesPaint(l.children));
}

/** A document's layers with the lookups the panel needs. */
export type LayerTree = {
  /** The top level, bottom to top. */
  layers: LayerView[];
  /** Every layer (see `walk`). */
  all: LayerView[];
  /** The group of each layer that is in one. */
  parents: Map<number, number>;
};

export function layerTree(layers: LayerView[]): LayerTree {
  const all = walk(layers);
  const parents = new Map<number, number>();
  for (const layer of all) {
    for (const child of layer.children) parents.set(child.id, layer.id);
  }
  return { layers, all, parents };
}

/** The layers directly inside `parent` (null: the top level), bottom to top. */
export function childrenOf(tree: LayerTree, parent: number | null): LayerView[] {
  if (parent === null) return tree.layers;
  return tree.all.find((l) => l.id === parent)?.children ?? [];
}

/** The groups around `id`, innermost first. */
export function ancestors(tree: LayerTree, id: number): number[] {
  const out: number[] = [];
  for (let at = tree.parents.get(id); at !== undefined; at = tree.parents.get(at)) out.push(at);
  return out;
}

/** Whether `id` is `ancestor` or inside it. */
export function within(tree: LayerTree, id: number, ancestor: number): boolean {
  return id === ancestor || ancestors(tree, id).includes(ancestor);
}

/** `ids` without those inside another of them (they go with it). */
export function outermost(tree: LayerTree, ids: number[]): number[] {
  const set = new Set(ids);
  return ids.filter((id) => !ancestors(tree, id).some((at) => set.has(at)));
}

/** The topmost of `ids` in the stack. */
export function topmost(tree: LayerTree, ids: number[]): number | null {
  const set = new Set(ids);
  return tree.all.findLast((l) => set.has(l.id))?.id ?? null;
}

/** Where a new layer goes: just above `active` in its group, or at the top without one. */
export function insertionPoint(
  tree: LayerTree,
  active: number | null,
): { parent: number | null; index: number } {
  if (active === null) return { parent: null, index: tree.layers.length };
  const parent = tree.parents.get(active) ?? null;
  return { parent, index: childrenOf(tree, parent).findIndex((l) => l.id === active) + 1 };
}

/** The slot a pointer at `y` designates: how many rows have their middle above it. */
export function slotAt(middles: number[], y: number): number {
  return middles.filter((middle) => middle < y).length;
}

/** Layers being dragged can go into `layer`: a group, and not one of them or inside one. */
export function canDropInto(tree: LayerTree, layer: LayerView, moving: number[]): boolean {
  return layer.kind === "group" && !moving.some((id) => within(tree, layer.id, id));
}

/**
 * Where a drop puts the `moving` layers: into group `into` (at its top), else just above the
 * row at `slot` (below the last row: the bottom of the stack). `index` counts the layers of
 * `parent` that stay. Null for a drop inside one of the moving groups.
 */
export function dropTarget(
  tree: LayerTree,
  rows: Row[],
  moving: ReadonlySet<number>,
  into: number | null,
  slot: number,
): { parent: number | null; index: number } | null {
  const staying = (layers: LayerView[]) => layers.filter((l) => !moving.has(l.id));
  if (into !== null) return { parent: into, index: staying(childrenOf(tree, into)).length };
  const row = rows[slot];
  if (!row) return { parent: null, index: 0 };
  const parent = row.parent;
  if (parent !== null && [...moving].some((id) => within(tree, parent, id))) return null;
  const index = staying(childrenOf(tree, parent).slice(0, row.index + 1)).length;
  return { parent, index };
}
