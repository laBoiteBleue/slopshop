// The edits the Layers panel sends for the selected layers, as in Photoshop: one undo entry for
// all of them (a batch), and nothing for layers already as asked. Null: nothing to send.

import { hexToSrgb, srgbToHex } from "./color";
import type { BlendModeId, EditRequest, LayerView } from "./engine";
import type { GradientFill } from "./gradientFill";
import { childrenOf, insertionPoint, outermost, type LayerTree } from "./layerTree";

/** One edit, or one batch (a single undo entry) for several. */
export function batchOf(edits: EditRequest[]): EditRequest {
  return edits.length === 1 ? edits[0] : { kind: "batch", edits };
}

function batchOrNone(edits: EditRequest[]): EditRequest | null {
  return edits.length > 0 ? batchOf(edits) : null;
}

/** Delete `layers`: a group takes its layers with it. */
export function removal(tree: LayerTree, layers: LayerView[]): EditRequest | null {
  const ids = outermost(
    tree,
    layers.map((l) => l.id),
  );
  return batchOrNone(ids.map((id): EditRequest => ({ kind: "removeLayer", id })));
}

/** Whether the clipping command releases `layers` (all clipped) rather than clips them. */
export function clippingReleases(layers: LayerView[]): boolean {
  return layers.length > 0 && layers.every((l) => l.clipped);
}

/** Clip `layers` to the layers below them, or release them when all are clipped. */
export function clippingToggle(layers: LayerView[]): EditRequest | null {
  const release = clippingReleases(layers);
  return batchOrNone(
    layers
      .filter((l) => l.clipped === release)
      .map((l): EditRequest => ({ kind: "setLayerClipped", id: l.id, clipped: !release })),
  );
}

/** Where Layer > Arrange moves the selected layers among the layers of their group. */
export type Arrangement = "front" | "forward" | "backward" | "back";

/**
 * Whether `arrangement` moves something (Photoshop grays it otherwise): one of `layers` has a
 * layer of its group that does not move above it (front, forward) or below it (backward, back).
 */
export function canArrange(
  tree: LayerTree,
  layers: LayerView[],
  arrangement: Arrangement,
): boolean {
  const ids = outermost(
    tree,
    layers.map((l) => l.id),
  );
  const moving = new Set(ids);
  const up = arrangement === "front" || arrangement === "forward";
  return ids.some((id) => {
    const siblings = childrenOf(tree, tree.parents.get(id) ?? null);
    const at = siblings.findIndex((l) => l.id === id);
    const passed = up ? siblings.slice(at + 1) : siblings.slice(0, at);
    return passed.some((l) => !moving.has(l.id));
  });
}

/** The groups among `layers` replaced by their layers, which become the selection. */
export function ungrouping(layers: LayerView[]): { request: EditRequest; layers: number[] } | null {
  const groups = layers.filter((l) => l.kind === "group");
  if (groups.length === 0) return null;
  const ungrouped = new Set(groups.map((g) => g.id));
  return {
    request: { kind: "ungroup", ids: groups.map((g) => g.id) },
    layers: groups.flatMap((g) => g.children.map((l) => l.id)).filter((id) => !ungrouped.has(id)),
  };
}

/** The mask the Layer menu speaks of: the active layer's, else the first selected one's. */
export function referenceMask(selection: LayerView[], active: LayerView | null): LayerView["mask"] {
  return active?.mask ?? selection.find((l) => l.mask)?.mask ?? null;
}

/** Disable the masks of `selection`, or enable them all when the reference mask is disabled. */
export function maskEnabledToggle(
  selection: LayerView[],
  active: LayerView | null,
): EditRequest | null {
  const enabled = referenceMask(selection, active)?.enabled === false;
  return batchOrNone(
    selection
      .filter((l) => l.mask && l.mask.enabled !== enabled)
      .map((l): EditRequest => ({ kind: "setLayerMaskEnabled", id: l.id, enabled })),
  );
}

/** Delete the masks of `layers`. */
export function maskRemoval(layers: LayerView[]): EditRequest | null {
  return batchOrNone(
    layers.filter((l) => l.mask).map((l): EditRequest => ({ kind: "removeLayerMask", id: l.id })),
  );
}

/**
 * A new solid color fill layer of `hex` (`#rrggbb`, sRGB) above `active` in its group, or at
 * the top without one, as the other new layers (Layer > New Fill Layer > Solid Color).
 */
export function newFill(
  tree: LayerTree,
  active: number | null,
  hex: string,
  name: string,
): EditRequest {
  const { parent, index } = insertionPoint(tree, active);
  return { kind: "addFillLayer", name, color: [...hexToSrgb(hex), 1], parent, index };
}

/**
 * A new gradient fill layer of `gradient` above `active`, placed as `newFill` places a fill
 * layer (Layer > New Fill Layer > Gradient).
 */
export function newGradientFillLayer(
  tree: LayerTree,
  active: number | null,
  gradient: GradientFill,
  name: string,
): EditRequest {
  const { parent, index } = insertionPoint(tree, active);
  return { kind: "addGradientFill", name, gradient, parent, index };
}

/** Whether `layer` has settings the Properties panel shows: an adjustment or a fill layer. */
export function hasProperties(layer: LayerView | null): layer is LayerView {
  return layer?.kind === "adjustment" || layer?.kind === "fill" || layer?.kind === "gradientFill";
}

/** A fill layer's color as `#rrggbb`, from its display swatch. */
export function fillHex(layer: LayerView): string {
  return srgbToHex(layer.swatch);
}

/** `layer`'s fill color set to `hex`; null when it is not a fill layer or has that color. */
export function fillColorEdit(layer: LayerView, hex: string): EditRequest | null {
  if (layer.kind !== "fill" || fillHex(layer) === hex.toLowerCase()) return null;
  return { kind: "setFillColor", id: layer.id, color: [...hexToSrgb(hex), 1] };
}

/** Show or hide `layers`. */
export function visibility(layers: LayerView[], visible: boolean): EditRequest | null {
  return batchOrNone(
    layers
      .filter((l) => l.visible !== visible)
      .map((l): EditRequest => ({ kind: "setLayerVisible", id: l.id, visible })),
  );
}

/** Hide the selected layers, or show them all when the `active` one is hidden. */
export function visibilityToggle(
  selection: LayerView[],
  active: LayerView | null,
): EditRequest | null {
  return visibility(selection, active?.visible === false);
}

/**
 * A click on the eye of `layer`: within a selection of several layers, shows or hides them
 * all (as the clicked one becomes); otherwise toggles that layer alone.
 */
export function eyeClick(layer: LayerView, selection: LayerView[]): EditRequest | null {
  const visible = !layer.visible;
  if (selection.length > 1 && selection.some((l) => l.id === layer.id)) {
    return visibility(selection, visible);
  }
  return { kind: "setLayerVisible", id: layer.id, visible };
}

/**
 * A blend mode for `layers`. "passThrough" applies to groups only; another mode ends a group's
 * pass through.
 */
export function blendModeChange(
  layers: LayerView[],
  mode: BlendModeId | "passThrough",
): EditRequest | null {
  const edits: EditRequest[] = [];
  for (const layer of layers) {
    if (mode === "passThrough") {
      if (layer.kind === "group" && !layer.passThrough) {
        edits.push({ kind: "setGroupPassThrough", id: layer.id, passThrough: true });
      }
      continue;
    }
    edits.push({ kind: "setLayerBlendMode", id: layer.id, mode });
    if (layer.kind === "group" && layer.passThrough) {
      edits.push({ kind: "setGroupPassThrough", id: layer.id, passThrough: false });
    }
  }
  return batchOrNone(edits);
}

/** The opacity of `layers`, in [0, 1]. */
export function opacityEdit(layers: LayerView[], opacity: number): EditRequest {
  return batchOf(layers.map((l): EditRequest => ({ kind: "setLayerOpacity", id: l.id, opacity })));
}

/** A layer's opacity as the panel shows it, whole percent (100 without a layer). */
export function opacityPercent(layer: LayerView | null): number {
  return layer ? Math.round(layer.opacity * 100) : 100;
}

/**
 * The opacity field's value (`text`, and as a number `n`): whole percent within [0, 100];
 * null when empty or invalid (the field shows the layer's again, rather than 0).
 */
export function typedOpacity(text: string, n: number): number | null {
  if (text.trim() === "" || !Number.isFinite(n)) return null;
  return Math.min(Math.max(Math.round(n), 0), 100);
}
