import { test } from "vitest";
import assert from "node:assert/strict";
import type { LayerView } from "../src/lib/engine";
import {
  PLAIN,
  defaultEffect,
  effectsOf,
  fillEdit,
  hasEffects,
  simplified,
  styleEdit,
  withEffect,
  withFill,
} from "../src/lib/layerStyle";

function layer(id: number, changes: Partial<LayerView> = {}): LayerView {
  return {
    id,
    name: `Layer ${id}`,
    visible: true,
    opacity: 1,
    kind: "raster",
    swatch: [0, 0, 0, 0],
    blendMode: "normal",
    contentKey: 0,
    hasAlpha: true,
    mask: null,
    children: [],
    passThrough: false,
    clipped: false,
    transform: [1, 0, 0, 1, 0, 0],
    painted: false,
    entries: [],
    adjustment: null,
    ...changes,
  };
}

test("an effect is added at Photoshop's defaults, and keeps its settings when turned off", () => {
  const shadow = withEffect(null, "dropShadow", true);
  assert.deepEqual(shadow.dropShadow, defaultEffect("dropShadow"));
  assert.equal(shadow.dropShadow?.mode, "multiply");
  const tuned = { ...shadow, dropShadow: { ...shadow.dropShadow!, size: 30 } };
  const off = withEffect(tuned, "dropShadow", false);
  assert.deepEqual(off.dropShadow, { ...tuned.dropShadow, enabled: false });
  assert.equal(hasEffects(off), false);
  assert.equal(hasEffects(shadow), true);
});

test("a style that changes nothing is no style", () => {
  assert.equal(simplified(PLAIN), null);
  assert.equal(withFill(null, 1), null);
  assert.deepEqual(withFill(null, 0.5), { ...PLAIN, fillOpacity: 0.5 });
  // An effect turned off is still a setting kept.
  const off = withEffect(null, "stroke", false);
  assert.notEqual(simplified(off), null);
  assert.deepEqual(styleEdit(3, PLAIN), { kind: "setLayerStyle", id: 3, style: null });
});

test("a layer's effects are listed in Photoshop's order", () => {
  let style = withEffect(null, "dropShadow", true);
  style = withEffect(style, "stroke", false);
  assert.deepEqual(effectsOf(style), [
    { id: "stroke", enabled: false },
    { id: "dropShadow", enabled: true },
  ]);
  assert.deepEqual(effectsOf(null), []);
});

test("Fill reaches the selected pixel and fill layers only, in one edit", () => {
  const styled = withEffect(null, "stroke", true);
  const edit = fillEdit(
    [
      layer(1, { style: styled }),
      layer(2, { kind: "group" }),
      layer(3, { kind: "fill" }),
      layer(4, { kind: "adjustment" }),
    ],
    0.25,
  );
  assert.deepEqual(edit, {
    kind: "batch",
    edits: [
      { kind: "setLayerStyle", id: 1, style: { ...styled, fillOpacity: 0.25 } },
      { kind: "setLayerStyle", id: 3, style: { ...PLAIN, fillOpacity: 0.25 } },
    ],
  });
  assert.equal(fillEdit([layer(2, { kind: "group" })], 0.5), null);
});
