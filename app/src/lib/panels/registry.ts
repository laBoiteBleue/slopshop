// The panels of the dock below Layers (ADR 0030, ADR 0036), in their default order. The dock's
// tabs, the Window menu and the saved layout all come from this list. Adding a panel: a line
// here, its component in this folder (reading the app through `panelContext()`) listed in
// `index.ts`, and its title in the catalogs. Ids are stable: saved layouts name panels by them
// (an id a layout does not know is dropped, a panel it does not mention gets its default place).

import type { IconName } from "../Icon.svelte";
import type { MessageKey } from "../i18n/en";

export type PanelInfo = {
  /** Stable: saved layouts name panels by it. */
  id: string;
  icon: IconName;
  title: MessageKey;
};

export const PANELS = [
  { id: "properties", icon: "sliders", title: "properties.title" },
  { id: "selections", icon: "marquee", title: "selections.title" },
] as const satisfies readonly PanelInfo[];

export type PanelId = (typeof PANELS)[number]["id"];

export const PANEL_IDS: readonly PanelId[] = PANELS.map((p) => p.id);

export function isPanelId(id: unknown): id is PanelId {
  return PANEL_IDS.includes(id as PanelId);
}

export function panelInfo(id: PanelId): PanelInfo {
  return PANELS.find((p) => p.id === id) ?? PANELS[0];
}
