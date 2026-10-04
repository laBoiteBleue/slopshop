import { expect, test } from "vitest";
import { historyName } from "../src/lib/history";

test("history entries are named by their kind, with the adjustment or filter they applied", () => {
  expect(historyName({ kind: "brush", detail: null })).toEqual({ key: "history.brush" });
  expect(historyName({ kind: "filter", detail: "gaussianBlur" })).toEqual({
    key: "history.named",
    name: "filter.gaussianBlur",
  });
  expect(historyName({ kind: "adjustment", detail: "levels" })).toEqual({
    key: "history.named",
    name: "adjustment.levels",
  });
  expect(historyName({ kind: "newAdjustmentLayer", detail: "curves" })).toEqual({
    key: "history.newAdjustmentLayerOf",
    name: "adjustment.curves",
  });
  expect(historyName({ kind: "adjustmentSettings", detail: "invert" })).toEqual({
    key: "history.adjustmentSettingsOf",
    name: "adjustment.invert",
  });
  // A detail the catalogs do not know: the kind alone.
  expect(historyName({ kind: "filter", detail: "swirl" })).toEqual({ key: "history.filter" });
  // A kind newer than the catalogs: a generic name, never the identifier.
  expect(historyName({ kind: "liquify", detail: null })).toEqual({ key: "history.edit" });
});
