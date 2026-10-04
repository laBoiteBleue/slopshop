// Editing a raster layer's stack entries again (ADR 0034): which entries have settings, the
// settings of each step of an entry as edits send them, and the edit that sets an entry.

import type { AdjustmentSettings, AdjustmentView, EditRequest, StackEntryView } from "./engine";

/** Whether `entry` has settings to edit again: an applied adjustment other than Invert. */
export function editableEntry(entry: StackEntryView): boolean {
  return entry.kind === "effect" && entry.steps.length > 0 && entry.adjustment !== "invert";
}

/** An adjustment's settings as edits send them. */
export function settingsOf(view: AdjustmentView): AdjustmentSettings {
  return {
    adjustment: view.id,
    values: [...view.values],
    ...(view.curves ? { curves: view.curves } : {}),
    ...(view.gradient ? { gradient: view.gradient } : {}),
  };
}

/** The settings of each step of `entry`, bottom to top. */
export function stepSettings(entry: StackEntryView): AdjustmentSettings[] {
  return entry.steps.map(settingsOf);
}

/** What the adjustment dialog changed: its values (and Gradient Map's stops), or Curves' points. */
export type SettingsChange = {
  values?: number[];
  curves?: number[][][];
  gradient?: number[][];
};

/** `settings` with step `step` changed by `change` (what it does not say stays). */
export function withStep(
  settings: AdjustmentSettings[],
  step: number,
  change: SettingsChange,
): AdjustmentSettings[] {
  return settings.map((s, i) =>
    i !== step
      ? s
      : {
          ...s,
          ...(change.values && change.values.length > 0 ? { values: change.values } : {}),
          ...(change.curves ? { curves: change.curves } : {}),
          ...(change.gradient ? { gradient: change.gradient } : {}),
        },
  );
}

/** Whether two lists of settings are the same. */
export function sameSettings(a: AdjustmentSettings[], b: AdjustmentSettings[]): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** The edit that sets entry `index` of layer `id`'s stack: `settings` for its steps, its eye. */
export function entryEdit(
  id: number,
  index: number,
  settings: AdjustmentSettings[],
  hidden: boolean,
): EditRequest {
  return { kind: "setStackEntry", id, index, hidden, steps: settings };
}
