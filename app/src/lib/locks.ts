// Layer locks (ADR 0046), as Photoshop's Layers panel shows and sets them: the four buttons
// of its "Lock:" row act on the selected layers, a padlock on a row says the layer is locked.
// The engine enforces the locks; this module only presents them and builds the edits.

import type { EditRequest, LayerView } from "./engine";
import { ancestors, type LayerTree } from "./layerTree";

/** A layer's locks: Lock Transparent Pixels, Image Pixels, Position, All. */
export type Locks = { transparency: boolean; pixels: boolean; position: boolean; all: boolean };

/** One lock, in the order of Photoshop's buttons. */
export type LockKind = keyof Locks;

export const LOCK_KINDS: readonly LockKind[] = ["transparency", "pixels", "position", "all"];

export const NO_LOCKS: Locks = { transparency: false, pixels: false, position: false, all: false };

/** Both sets of locks (a layer's own and a group's around it). */
function union(a: Locks, b: Locks): Locks {
  return {
    transparency: a.transparency || b.transparency,
    pixels: a.pixels || b.pixels,
    position: a.position || b.position,
    all: a.all || b.all,
  };
}

function any(locks: Locks): boolean {
  return locks.transparency || locks.pixels || locks.position || locks.all;
}

/** The locks of the groups around layer `id`, which apply to it too. */
export function inheritedLocks(tree: LayerTree, id: number): Locks {
  return ancestors(tree, id).reduce((locks, at) => {
    const group = tree.all.find((l) => l.id === at);
    return group ? union(locks, group.locks) : locks;
  }, NO_LOCKS);
}

/** What applies to layer `id`: its own locks and its groups'. */
export function effectiveLocks(tree: LayerTree, layer: LayerView): Locks {
  return union(layer.locks, inheritedLocks(tree, layer.id));
}

/**
 * The padlock a row shows, as Photoshop's: `full` for Lock All, `partial` for any other lock;
 * `inherited` when it comes from a group around the layer only (shown dimmed). `null`: none.
 */
export function lockMark(
  tree: LayerTree,
  layer: LayerView,
): { kind: "full" | "partial"; inherited: boolean } | null {
  const own = layer.locks;
  if (any(own)) return { kind: own.all ? "full" : "partial", inherited: false };
  const inherited = inheritedLocks(tree, layer.id);
  if (any(inherited)) return { kind: inherited.all ? "full" : "partial", inherited: true };
  return null;
}

/** How a lock's button shows for the selected layers: set on all of them, some, or none. */
export function lockState(layers: LayerView[], kind: LockKind): "on" | "mixed" | "off" {
  const set = layers.filter((l) => l.locks[kind]).length;
  if (set === 0) return "off";
  return set === layers.length ? "on" : "mixed";
}

/**
 * A click on a lock's button: set it on the selected layers, or clear it when all of them have
 * it (Photoshop's toggle). `null` without layers.
 */
export function lockToggle(layers: LayerView[], kind: LockKind): EditRequest | null {
  if (layers.length === 0) return null;
  const on = lockState(layers, kind) !== "on";
  return { kind: "setLayerLocks", ids: layers.map((l) => l.id), [kind]: on };
}

/**
 * The locks Layer > Lock Layers starts from for the selected layers: those all of them have,
 * as Photoshop's dialog does.
 */
export function commonLocks(layers: LayerView[]): Locks {
  const has = (kind: LockKind) => layers.length > 0 && layers.every((l) => l.locks[kind]);
  return {
    transparency: has("transparency"),
    pixels: has("pixels"),
    position: has("position"),
    all: has("all"),
  };
}

/** Layer > Lock Layers' OK: the selected layers get exactly `locks`. */
export function locksEdit(layers: LayerView[], locks: Locks): EditRequest | null {
  if (layers.length === 0) return null;
  return { kind: "setLayerLocks", ids: layers.map((l) => l.id), ...locks };
}

/**
 * The i18n key of an engine message saying that a lock refused an edit (the engine words it
 * "… is locked: <lock>", see `EditError::Locked`), or `null` for any other message.
 */
export function lockRefusalKey(message: string): string | null {
  const found = /is locked: (transparency|pixels|position|all)\b/.exec(message);
  return found ? `locks.refused.${found[1]}` : null;
}
