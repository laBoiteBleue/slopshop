// The document tabs: their order and how a dragged tab lands. Tabs are listed left to right,
// as the engine lists its documents.

/**
 * Put a document view from the engine in `tabs`: added if new, else updated unless it is
 * stale (answers can arrive out of order). Changes `tabs` in place (a Svelte state array).
 */
export function upsert<T extends { id: number; revision: number }>(tabs: T[], view: T) {
  const index = tabs.findIndex((d) => d.id === view.id);
  if (index < 0) tabs.push(view);
  else if (view.revision >= tabs[index].revision) tabs[index] = shared(tabs[index], view);
}

/**
 * `next` with every part equal to `previous`'s being `previous`'s own: a slider dragged sends
 * the whole document at each step, and what shows a layer that did not change (a row of the
 * Layers panel, its thumbnail) is then not run again. Neither is changed: the result is a new
 * object wherever something differs. Arrays of objects with an `id` are matched by id.
 */
export function shared<T>(previous: T, next: T): T {
  if (Object.is(previous, next)) return previous;
  if (Array.isArray(previous) && Array.isArray(next)) {
    const byId = new Map<unknown, unknown>();
    for (const item of previous) {
      if (isRecord(item) && "id" in item) byId.set(item.id, item);
    }
    let same = previous.length === next.length;
    const items = next.map((item, i) => {
      const before = isRecord(item) && "id" in item ? byId.get(item.id) : previous[i];
      const kept = before === undefined ? item : shared(before, item);
      same &&= kept === previous[i];
      return kept;
    });
    return (same ? previous : items) as T;
  }
  if (isRecord(previous) && isRecord(next) && !Array.isArray(previous) && !Array.isArray(next)) {
    const keys = Object.keys(next);
    let same = keys.length === Object.keys(previous).length;
    const out: Record<string, unknown> = {};
    for (const key of keys) {
      const kept = key in previous ? shared(previous[key], next[key]) : next[key];
      same &&= key in previous && kept === previous[key];
      out[key] = kept;
    }
    return (same ? previous : out) as T;
  }
  return next;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/** The tab `step` places after `active` (before when negative), going round; null if none. */
export function cycled(ids: number[], active: number | null, step: number): number | null {
  if (ids.length < 2) return null;
  const index = ids.indexOf(active ?? -1);
  return ids[(((index + step) % ids.length) + ids.length) % ids.length];
}

/**
 * Where a tab dragged from index `from` to `x` goes, as an insertion position among all tabs
 * (`middles`: each tab's middle, left to right); null when it would stay where it is (right
 * before or after itself).
 */
export function tabSlot(middles: number[], x: number, from: number): number | null {
  const slot = middles.filter((middle) => middle < x).length;
  return slot === from || slot === from + 1 ? null : slot;
}

/**
 * Move the tab at `from` to the insertion position `slot` (see `tabSlot`), in place; its new
 * index among the other tabs (as the engine counts it).
 */
export function moveTab<T>(tabs: T[], from: number, slot: number): number {
  const index = slot > from ? slot - 1 : slot;
  const [tab] = tabs.splice(from, 1);
  tabs.splice(index, 0, tab);
  return index;
}
