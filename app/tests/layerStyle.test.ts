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
  usesFill,
  withEffect,
  withFill,
  withoutEffect,
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

test("Fill reaches the selected layers but adjustment layers, in one edit", () => {
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
      { kind: "setLayerStyle", id: 2, style: { ...PLAIN, fillOpacity: 0.25 } },
      { kind: "setLayerStyle", id: 3, style: { ...PLAIN, fillOpacity: 0.25 } },
    ],
  });
  assert.equal(fillEdit([layer(4, { kind: "adjustment" })], 0.5), null);
});

test("the glows and Inner Shadow come at Photoshop's defaults, in the dialog's order", () => {
  assert.deepEqual(withEffect(null, "outerGlow", true).outerGlow, {
    enabled: true,
    color: [1, 1, 190 / 255],
    mode: "screen",
    opacity: 0.75,
    spread: 0,
    size: 5,
  });
  assert.equal(withEffect(null, "innerShadow", true).innerShadow?.mode, "multiply");
  let style = withEffect(null, "dropShadow", true);
  for (const id of ["outerGlow", "innerGlow", "innerShadow", "stroke", "colorOverlay"] as const) {
    style = withEffect(style, id, true);
  }
  assert.deepEqual(
    effectsOf(style).map((e) => e.id),
    ["stroke", "innerShadow", "innerGlow", "colorOverlay", "outerGlow", "dropShadow"],
  );
});

test("deleting an effect drops its settings; the last one leaves no style", () => {
  const both = withEffect(withEffect(null, "stroke", true), "dropShadow", false);
  assert.deepEqual(withoutEffect(both, "dropShadow"), withEffect(null, "stroke", true));
  assert.equal(withoutEffect(withEffect(null, "stroke", true), "stroke"), null);
  // A Fill set stays.
  const faded = { ...withEffect(null, "stroke", true), fillOpacity: 0.5 };
  assert.deepEqual(withoutEffect(faded, "stroke"), { ...PLAIN, fillOpacity: 0.5 });
});

test("Fill means something with an effect, even off, or once set", () => {
  assert.equal(usesFill(null), false);
  assert.equal(usesFill(PLAIN), false);
  assert.equal(usesFill(withEffect(null, "stroke", false)), true);
  assert.equal(usesFill({ ...PLAIN, fillOpacity: 0.3 }), true);
});

test("Gradient Overlay comes black to white upward, aligned, between Color Overlay and Outer Glow", () => {
  assert.deepEqual(defaultEffect("gradientOverlay"), {
    enabled: true,
    stops: [
      [0, 0, 0, 0],
      [4096, 255, 255, 255],
    ],
    reverse: false,
    shape: "linear",
    angle: 90,
    scale: 100,
    align: true,
    mode: "normal",
    opacity: 1,
  });
  let style = withEffect(null, "outerGlow", true);
  style = withEffect(style, "gradientOverlay", true);
  style = withEffect(style, "colorOverlay", true);
  assert.deepEqual(
    effectsOf(style).map((e) => e.id),
    ["colorOverlay", "gradientOverlay", "outerGlow"],
  );
});

test("Satin comes black, Multiply, 50 %, 19°, 11 and 14 pixels, inverted, under the overlays", () => {
  assert.deepEqual(defaultEffect("satin"), {
    enabled: true,
    color: [0, 0, 0],
    mode: "multiply",
    opacity: 0.5,
    angle: 19,
    distance: 11,
    size: 14,
    invert: true,
  });
  let style = withEffect(null, "colorOverlay", true);
  style = withEffect(style, "satin", true);
  style = withEffect(style, "innerGlow", true);
  assert.deepEqual(
    effectsOf(style).map((e) => e.id),
    ["innerGlow", "satin", "colorOverlay"],
  );
});
