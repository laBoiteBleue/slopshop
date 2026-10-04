import { expect, test } from "vitest";
import type { AdjustmentView, StackEntryView } from "../src/lib/engine";
import {
  editableEntry,
  entryEdit,
  sameSettings,
  settingsOf,
  stepSettings,
  withStep,
} from "../src/lib/stackEntries";

const levels: AdjustmentView = {
  id: "levels",
  values: [0, 1, 1, 0, 1],
  curves: null,
  curveSamples: null,
  gradient: null,
};

function entry(changes: Partial<StackEntryView>): StackEntryView {
  return {
    kind: "effect",
    adjustment: "levels",
    count: 1,
    hidden: false,
    steps: [levels],
    filter: null,
    filterSteps: [],
    ...changes,
  };
}

test("an applied adjustment with settings is editable; paint and Invert are not", () => {
  expect(editableEntry(entry({}))).toBe(true);
  expect(editableEntry(entry({ kind: "paint", adjustment: null, steps: [] }))).toBe(false);
  expect(
    editableEntry(
      entry({ adjustment: "invert", steps: [{ ...levels, id: "invert", values: [] }] }),
    ),
  ).toBe(false);
});

test("a Liquify entry is edited again in its workspace, not by settings", () => {
  expect(
    editableEntry(entry({ kind: "liquify", adjustment: null, steps: [], filterSteps: [] })),
  ).toBe(true);
});

test("settings travel as edits send them: Curves' points and Gradient Map's stops only when there", () => {
  expect(settingsOf(levels)).toEqual({ adjustment: "levels", values: [0, 1, 1, 0, 1] });
  const curves = {
    ...levels,
    id: "curves" as const,
    values: [],
    curves: [
      [
        [0, 0],
        [255, 255],
      ],
    ],
  };
  expect(settingsOf(curves)).toEqual({
    adjustment: "curves",
    values: [],
    curves: [
      [
        [0, 0],
        [255, 255],
      ],
    ],
  });
  const gradient = { ...levels, id: "gradientMap" as const, values: [0], gradient: [[0, 0, 0, 0]] };
  expect(settingsOf(gradient).gradient).toEqual([[0, 0, 0, 0]]);
  expect(stepSettings(entry({ count: 2, steps: [levels, levels] }))).toHaveLength(2);
});

test("a step changed by the dialog keeps what the change does not say, the others untouched", () => {
  const settings = stepSettings(entry({ count: 2, steps: [levels, levels] }));
  const changed = withStep(settings, 1, { values: [10, 1, 1, 0, 1] });
  expect(changed[0]).toBe(settings[0]);
  expect(changed[1]).toEqual({ adjustment: "levels", values: [10, 1, 1, 0, 1] });
  // Curves send their points with no values: the values stay.
  const curves = withStep(changed, 1, {
    values: [],
    curves: [
      [
        [0, 10],
        [255, 255],
      ],
    ],
  });
  expect(curves[1]).toEqual({
    adjustment: "levels",
    values: [10, 1, 1, 0, 1],
    curves: [
      [
        [0, 10],
        [255, 255],
      ],
    ],
  });
  expect(sameSettings(settings, stepSettings(entry({ count: 2, steps: [levels, levels] })))).toBe(
    true,
  );
  expect(sameSettings(settings, changed)).toBe(false);
});

test("the edit sets the entry's steps and its eye", () => {
  const settings = [settingsOf(levels)];
  expect(entryEdit(7, 2, settings, true)).toEqual({
    kind: "setStackEntry",
    id: 7,
    index: 2,
    hidden: true,
    steps: settings,
  });
});
