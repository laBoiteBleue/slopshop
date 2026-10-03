// The edits the Layers panel sends for the selected layers, as in Photoshop: one undo entry for
// all of them (a batch), and nothing for layers already as asked. Null: nothing to send.

import type { BlendModeId, EditRequest, LayerView } from "./engine";
import { outermost, type LayerTree } from "./layerTree";

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

/** Clip `layers` to the layers below them, or release them when all are clipped. */
export function clippingToggle(layers: LayerView[]): EditRequest | null {
  const release = layers.length > 0 && layers.every((l) => l.clipped);
  return batchOrNone(
    layers
      .filter((l) => l.clipped === release)
      .map((l): EditRequest => ({ kind: "setLayerClipped", id: l.id, clipped: !release })),
  );
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
