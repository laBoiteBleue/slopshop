// Layer > Align and Distribute, and the Move tool's buttons: the commands, their icons and
// labels, and when they apply. The engine does the alignment (slopshop_core::align): one
// implementation for the menu and the options bar.

import type { IconName } from "./Icon.svelte";
import type { MessageKey } from "./i18n/en";
import type { LayerView } from "./engine";
import { outermost, type LayerTree } from "./layerTree";

export type AlignId = "left" | "horizontalCenters" | "right" | "top" | "verticalCenters" | "bottom";

export type DistributeId =
  "horizontalCenters" | "verticalCenters" | "horizontalSpacing" | "verticalSpacing";

/** A command: its icon in the options bar, its menu label and the bar's tooltip. */
export type AlignCommand<Id> = { id: Id; icon: IconName; label: MessageKey; hint: MessageKey };

/** In Photoshop's order: the horizontal ones, then the vertical ones. */
export const ALIGNS: AlignCommand<AlignId>[] = [
  { id: "left", icon: "alignLeft", label: "menu.layer.align.left", hint: "options.align.left" },
  {
    id: "horizontalCenters",
    icon: "alignHorizontalCenters",
    label: "menu.layer.align.horizontalCenters",
    hint: "options.align.horizontalCenters",
  },
  { id: "right", icon: "alignRight", label: "menu.layer.align.right", hint: "options.align.right" },
  { id: "top", icon: "alignTop", label: "menu.layer.align.top", hint: "options.align.top" },
  {
    id: "verticalCenters",
    icon: "alignVerticalCenters",
    label: "menu.layer.align.verticalCenters",
    hint: "options.align.verticalCenters",
  },
  {
    id: "bottom",
    icon: "alignBottom",
    label: "menu.layer.align.bottom",
    hint: "options.align.bottom",
  },
];

export const DISTRIBUTES: AlignCommand<DistributeId>[] = [
  {
    id: "horizontalCenters",
    icon: "distributeHorizontalCenters",
    label: "menu.layer.distribute.horizontalCenters",
    hint: "options.distribute.horizontalCenters",
  },
  {
    id: "verticalCenters",
    icon: "distributeVerticalCenters",
    label: "menu.layer.distribute.verticalCenters",
    hint: "options.distribute.verticalCenters",
  },
  {
    id: "horizontalSpacing",
    icon: "distributeHorizontalSpacing",
    label: "menu.layer.distribute.horizontalSpacing",
    hint: "options.distribute.horizontalSpacing",
  },
  {
    id: "verticalSpacing",
    icon: "distributeVerticalSpacing",
    label: "menu.layer.distribute.verticalSpacing",
    hint: "options.distribute.verticalSpacing",
  },
];

/** Distribute needs three layers, a group counting as one (`DISTRIBUTE_MIN` in align.rs). */
export const DISTRIBUTE_MIN = 3;

/** Whether Distribute applies to `layers`: three of them, those inside a selected group aside. */
export function canDistribute(tree: LayerTree, layers: LayerView[]): boolean {
  const ids = outermost(
    tree,
    layers.map((l) => l.id),
  );
  return ids.length >= DISTRIBUTE_MIN;
}
