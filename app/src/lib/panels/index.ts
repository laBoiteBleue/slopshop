// Each panel's component (the list and its order are in registry.ts): a panel without one
// fails the type check.

import type { Component } from "svelte";
import type { PanelId } from "./registry";
import Properties from "./Properties.svelte";
import Histogram from "./Histogram.svelte";
import History from "./History.svelte";
import Info from "./Info.svelte";
import Selections from "./Selections.svelte";

export const PANEL_COMPONENTS: Record<PanelId, Component> = {
  properties: Properties,
  selections: Selections,
  history: History,
  histogram: Histogram,
  info: Info,
};
