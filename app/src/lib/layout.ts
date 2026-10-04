// How the window is arranged around the image (the Window menu, ADR 0036): the dock's tabs in
// order, the one unfolded and its height, the panels column's width, and whether the options
// bar and the toolbar are shown. Remembered on this machine as one versioned record. Whatever
// cannot be read (an unknown panel, a bad number, a corrupt record) gets its default, so a
// panel renamed or removed never breaks the layout. Open documents are not part of it.

import { DEFAULT_DOCK, MIN_DOCK_HEIGHT, type DockState } from "./panelDock";
import { DEFAULT_PANEL_WIDTH, MIN_PANEL_WIDTH } from "./panelWidth";
import { PANEL_IDS, isPanelId, type PanelId } from "./panels/registry";

export type Layout = {
  dock: DockState;
  /** The dock's tabs, left to right. */
  order: PanelId[];
  /** The panels column's width, CSS pixels (narrower when the window is). */
  panelWidth: number;
  optionsBar: boolean;
  toolbar: boolean;
};

export function defaultLayout(): Layout {
  return {
    dock: { ...DEFAULT_DOCK },
    order: [...PANEL_IDS],
    panelWidth: DEFAULT_PANEL_WIDTH,
    optionsBar: true,
    toolbar: true,
  };
}

const STORAGE_KEY = "slopshop.layout";
const VERSION = 1;
/** Before the layout was one record (2026-10-04): the dock and the width, each on its own. */
const OLD_DOCK_KEY = "slopshop.dock";
const OLD_WIDTH_KEY = "slopshop.panelWidth";

type Store = Pick<Storage, "getItem" | "setItem">;

function parse(text: string | null): unknown {
  try {
    return JSON.parse(text ?? "null");
  } catch {
    return null;
  }
}

/** The layout saved last (or what older versions kept), each part checked. */
export function loadLayout(store?: Store): Layout {
  try {
    const items = store ?? localStorage;
    const saved = parse(items.getItem(STORAGE_KEY)) as Record<string, unknown> | null;
    if (saved?.version === VERSION) return readLayout(saved);
    return readLayout({
      dock: parse(items.getItem(OLD_DOCK_KEY)),
      panelWidth: Number(items.getItem(OLD_WIDTH_KEY)),
    });
  } catch {
    // Storage unavailable.
    return defaultLayout();
  }
}

/** `saved`'s parts that make sense, the defaults for the others. */
function readLayout(saved: Record<string, unknown>): Layout {
  const layout = defaultLayout();
  const dock = (saved.dock ?? {}) as Record<string, unknown>;
  const known = Array.isArray(saved.order) ? [...new Set(saved.order.filter(isPanelId))] : [];
  const height = Number(dock.height);
  const width = Number(saved.panelWidth);
  return {
    dock: {
      open: dock.open === null || isPanelId(dock.open) ? dock.open : layout.dock.open,
      height: Number.isFinite(height) && height >= MIN_DOCK_HEIGHT ? height : layout.dock.height,
    },
    // Panels the record does not mention (new ones) keep their default place, at the end.
    order: [...known, ...layout.order.filter((id) => !known.includes(id))],
    panelWidth: Number.isFinite(width) && width >= MIN_PANEL_WIDTH ? width : layout.panelWidth,
    optionsBar: saved.optionsBar !== false,
    toolbar: saved.toolbar !== false,
  };
}

export function saveLayout(layout: Layout, store?: Store) {
  try {
    (store ?? localStorage).setItem(STORAGE_KEY, JSON.stringify({ version: VERSION, ...layout }));
  } catch {
    // Not remembered: it lasts for the session.
  }
}
