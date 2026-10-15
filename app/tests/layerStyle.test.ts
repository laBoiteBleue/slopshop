import { expect, test } from "vitest";
import assert from "node:assert/strict";
import type { LayerView } from "../src/lib/engine";
import {
  DEFAULT_LIGHT,
  PLAIN,
  allEffectsCommand,
  lightOf,
  litStyle,
  sameLight,
  defaultEffect,
  effectsOf,
  effectsShown,
  fillEdit,
  hasEffects,
  simplified,
  styleEdit,
  usesFill,
  withEffect,
  withEffectsVisible,
  withFill,
  withoutEffect,
  withPatternOverlay,
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
    locks: { transparency: false, pixels: false, position: false, all: false },
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

test("Bevel and Emboss comes first, Photoshop's Inner Bevel lit from 120° at 30°", () => {
  assert.deepEqual(defaultEffect("bevel"), {
    enabled: true,
    style: "innerBevel",
    depth: 100,
    up: true,
    size: 5,
    soften: 0,
    angle: 120,
    altitude: 30,
    useGlobalLight: true,
    highlightColor: [1, 1, 1],
    highlightMode: "screen",
    highlightOpacity: 0.75,
    shadowColor: [0, 0, 0],
    shadowMode: "multiply",
    shadowOpacity: 0.75,
  });
  let style = withEffect(null, "stroke", true);
  style = withEffect(style, "bevel", true);
  assert.deepEqual(
    effectsOf(style).map((e) => e.id),
    ["bevel", "stroke"],
  );
});

test("a Pattern Overlay is added with its pattern, its settings kept when it changes", () => {
  // Without a pattern, nothing to add.
  expect(withEffect(null, "patternOverlay", true).patternOverlay).toBeNull();
  const style = withPatternOverlay(null, 4);
  expect(style.patternOverlay).toEqual({
    enabled: true,
    source: 4,
    scale: 1,
    angle: 0,
    link: true,
    mode: "normal",
    opacity: 1,
  });
  const scaled = {
    ...style,
    patternOverlay: { ...style.patternOverlay!, scale: 2, enabled: false },
  };
  expect(withPatternOverlay(scaled, 7).patternOverlay).toMatchObject({
    source: 7,
    scale: 2,
    enabled: true,
  });
  // Turned off and on with its checkbox, as the others.
  expect(withEffect(style, "patternOverlay", false).patternOverlay?.enabled).toBe(false);
});

test("effects show unless hidden; hiding them keeps each effect's own setting", () => {
  const style = withEffect(null, "stroke", true);
  expect(effectsShown(style)).toBe(true);
  expect(effectsShown(null)).toBe(true);
  const hidden = withEffectsVisible(style, false);
  expect(effectsShown(hidden)).toBe(false);
  expect(hidden.stroke).toEqual(style.stroke);
  expect(effectsOf(hidden)).toEqual([{ id: "stroke", enabled: true }]);
  expect(effectsShown(withEffectsVisible(hidden, true))).toBe(true);
});

test("Hide All Effects while some show, Show All Effects once all are hidden, groups included", () => {
  const style = withEffect(null, "dropShadow", true);
  const hidden = withEffectsVisible(style, false);
  // Fill alone is not an effect.
  const fillOnly = { ...PLAIN, fillOpacity: 0.5 };
  expect(allEffectsCommand([layer(1), layer(2, { style: fillOnly })])).toBe(null);
  expect(allEffectsCommand([layer(1, { style }), layer(2, { style: hidden })])).toBe("hide");
  expect(allEffectsCommand([layer(1, { style: hidden })])).toBe("show");
  const group = layer(3, { kind: "group", children: [layer(4, { style })] });
  expect(allEffectsCommand([layer(1, { style: hidden }), group])).toBe("hide");
  // An effect turned off still counts: its layer has effects to show or hide.
  const off = withEffect(null, "stroke", false);
  expect(allEffectsCommand([layer(1, { style: off })])).toBe("hide");
});

test("the global light lights the effects using it: shadows' angle, a bevel's angle and altitude", () => {
  const style = withEffect(
    withEffect(withEffect(null, "dropShadow", true), "innerShadow", true),
    "bevel",
    true,
  );
  const own = { ...style, innerShadow: { ...style.innerShadow!, useGlobalLight: false, angle: 5 } };
  const lit = litStyle(own, [45, 60]);
  expect(lit.dropShadow?.angle).toBe(45);
  expect(lit.innerShadow?.angle).toBe(5);
  expect(lit.bevel).toMatchObject({ angle: 45, altitude: 60 });
  // New effects use it, at Photoshop's light.
  expect(style.dropShadow?.useGlobalLight).toBe(true);
  expect(litStyle(style, DEFAULT_LIGHT)).toEqual(style);
});

test("an effect using the global light with another angle or altitude moves it", () => {
  const style = litStyle(withEffect(withEffect(null, "dropShadow", true), "bevel", true), [90, 30]);
  expect(lightOf(style, [90, 30])).toEqual([90, 30]);
  const turned = { ...style, dropShadow: { ...style.dropShadow!, angle: 10 } };
  expect(lightOf(turned, [90, 30])).toEqual([10, 30]);
  const raised = { ...style, bevel: { ...style.bevel!, altitude: 70 } };
  expect(lightOf(raised, [90, 30])).toEqual([90, 70]);
  // An effect on its own light moves nothing.
  const own = { ...turned, dropShadow: { ...turned.dropShadow!, useGlobalLight: false } };
  expect(lightOf(own, [90, 30])).toEqual([90, 30]);
  expect(sameLight([1, 2], [1, 2])).toBe(true);
  expect(sameLight([1, 2], [1, 3])).toBe(false);
});
