import { expect, test } from "vitest";
import {
  BRUSH_LIMITS,
  DEFAULT_BRUSH,
  LIQUIFY_TOOLS,
  MAX_FRAME_PIXELS,
  MAX_ZOOM,
  MIN_ZOOM,
  clampBrush,
  effectiveTool,
  fitView,
  frameRequest,
  frameScale,
  panned,
  steppedSize,
  toCanvas,
  toLayer,
  toolForKey,
  toolbarTool,
  wheelZoom,
  zoomAt,
  zoomPercent,
} from "../src/lib/liquify";

test("each tool has Photoshop's key, in either case; other keys choose nothing", () => {
  const keys = Object.fromEntries(LIQUIFY_TOOLS.map((tool) => [tool.key, tool.id]));
  expect(keys).toEqual({
    w: "forwardWarp",
    r: "reconstruct",
    e: "smooth",
    c: "twirlClockwise",
    s: "pucker",
    b: "bloat",
    o: "pushLeft",
    f: "freeze",
    d: "thaw",
  });
  for (const [key, id] of Object.entries(keys)) {
    expect(toolForKey(key)).toBe(id);
    expect(toolForKey(key.toUpperCase())).toBe(id);
  }
  // No Hand and no Zoom tool: navigation is the mouse's.
  expect(toolForKey("h")).toBeNull();
  expect(toolForKey("z")).toBeNull();
  expect(toolForKey("Enter")).toBeNull();
  expect(toolForKey("[")).toBeNull();
});

test("Alt turns the twirl, bloats a pucker and thaws a freeze, and back; the others stay", () => {
  expect(effectiveTool("twirlClockwise", true)).toBe("twirlCounterclockwise");
  expect(effectiveTool("twirlCounterclockwise", true)).toBe("twirlClockwise");
  expect(effectiveTool("pucker", true)).toBe("bloat");
  expect(effectiveTool("bloat", true)).toBe("pucker");
  expect(effectiveTool("freeze", true)).toBe("thaw");
  expect(effectiveTool("thaw", true)).toBe("freeze");
  for (const tool of LIQUIFY_TOOLS) {
    expect(effectiveTool(tool.id, false)).toBe(tool.id);
  }
  for (const id of ["forwardWarp", "reconstruct", "smooth", "pushLeft"] as const) {
    expect(effectiveTool(id, true)).toBe(id);
  }
  // Both twirls are the one button.
  expect(toolbarTool("twirlCounterclockwise")).toBe("twirlClockwise");
  expect(toolbarTool("pucker")).toBe("pucker");
});

test("the brush starts at Photoshop's settings and is kept within its ranges", () => {
  expect(DEFAULT_BRUSH).toEqual({ size: 100, density: 50, pressure: 100, rate: 80 });
  expect(clampBrush({ size: 0, density: -5, pressure: 0, rate: 500 })).toEqual({
    size: BRUSH_LIMITS.size.min,
    density: 0,
    pressure: BRUSH_LIMITS.pressure.min,
    rate: 100,
  });
  expect(clampBrush({ size: 1e9, density: 101, pressure: 1000, rate: -1 })).toEqual({
    size: 15000,
    density: 100,
    pressure: 100,
    rate: 0,
  });
  expect(clampBrush({ size: NaN, density: 50, pressure: 50, rate: 50 }).size).toBe(1);
  expect(clampBrush(DEFAULT_BRUSH)).toEqual(DEFAULT_BRUSH);
});

test("[ and ] change the brush's size by a tenth, by at least a pixel, within its range", () => {
  expect(steppedSize(100, 1)).toBe(110);
  expect(steppedSize(100, -1)).toBe(90);
  expect(steppedSize(5, 1)).toBe(6);
  expect(steppedSize(5, -1)).toBe(4);
  expect(steppedSize(1, -1)).toBe(1);
  expect(steppedSize(14999, 1)).toBe(15000);
  expect(steppedSize(15000, 1)).toBe(15000);
  // Always moves while it can.
  let size = 1;
  for (let i = 0; i < 5; i++) {
    const next = steppedSize(size, 1);
    expect(next).toBeGreaterThan(size);
    size = next;
  }
});

test("the layer fits the stage, centered, never enlarged", () => {
  const big = fitView({ width: 4000, height: 2000 }, { width: 1016, height: 616 });
  expect(big.zoom).toBeCloseTo(1000 / 4000 > 600 / 2000 ? 600 / 2000 : 1000 / 4000);
  // Centered: the layer's middle is the stage's.
  const [cx, cy] = toCanvas(big, 2000, 1000);
  expect(cx).toBeCloseTo(508);
  expect(cy).toBeCloseTo(308);
  const small = fitView({ width: 100, height: 50 }, { width: 800, height: 600 });
  expect(small.zoom).toBe(1);
  expect(toCanvas(small, 50, 25)).toEqual([400, 300]);
  // A stage with no room: still a usable view.
  expect(fitView({ width: 100, height: 100 }, { width: 0, height: 0 }).zoom).toBeGreaterThan(0);
});

test("canvas and layer points map both ways", () => {
  const view = { x: 120, y: -30, zoom: 2.5 };
  const [x, y] = toLayer(view, 50, 80);
  expect([x, y]).toEqual([140, 2]);
  expect(toCanvas(view, x, y)).toEqual([50, 80]);
});

test("zooming keeps the layer point under the pointer, within the limits", () => {
  const view = { x: 10, y: 20, zoom: 1 };
  const zoomed = zoomAt(view, 2, 300, 200);
  expect(zoomed.zoom).toBe(2);
  expect(toLayer(zoomed, 300, 200)).toEqual(toLayer(view, 300, 200));
  const out = zoomAt(view, 0.25, 40, 40);
  expect(toLayer(out, 40, 40)[0]).toBeCloseTo(toLayer(view, 40, 40)[0]);
  expect(zoomAt(view, 1e9, 0, 0).zoom).toBe(MAX_ZOOM);
  expect(zoomAt(view, 1e-9, 0, 0).zoom).toBe(MIN_ZOOM);
});

test("a drag pans: the layer follows the pointer", () => {
  const view = { x: 100, y: 100, zoom: 2 };
  const moved = panned(view, 40, -20);
  // The layer point that was under the pointer is under it still, 40 right and 20 up.
  const [x, y] = toLayer(view, 200, 200);
  expect(toCanvas(moved, x, y)).toEqual([240, 180]);
  expect(moved.zoom).toBe(2);
});

test("the wheel zooms by 22% a notch, in to scroll up, out to scroll down", () => {
  expect(wheelZoom(-100)).toBeGreaterThan(1);
  expect(wheelZoom(100)).toBeLessThan(1);
  expect(wheelZoom(-100) * wheelZoom(100)).toBeCloseTo(1);
  expect(wheelZoom(-100)).toBeCloseTo(1.221, 2);
  expect(wheelZoom(0)).toBe(1);
});

test("frames are drawn at the screen's density, within a cap of pixels", () => {
  const view = { x: 5, y: 6, zoom: 0.5 };
  const canvas = { width: 1000, height: 600 };
  expect(frameScale(canvas, 1.5)).toBe(1.5);
  expect(frameRequest(view, canvas, 1.5, true)).toEqual({
    x: 5,
    y: 6,
    zoom: 0.75,
    width: 1500,
    height: 900,
    overlay: true,
  });
  // A huge stage at a high density: reduced to the cap.
  const huge = { width: 3000, height: 2000 };
  const request = frameRequest(view, huge, 3, false);
  expect(request.width * request.height).toBeLessThanOrEqual(MAX_FRAME_PIXELS * 1.01);
  expect(request.zoom).toBeCloseTo(0.5 * frameScale(huge, 3));
  expect(frameRequest(view, { width: 0, height: 0 }, 1, false)).toMatchObject({
    width: 1,
    height: 1,
  });
});

test("the zoom reads as a percentage", () => {
  expect(zoomPercent({ x: 0, y: 0, zoom: 1 })).toBe("100%");
  expect(zoomPercent({ x: 0, y: 0, zoom: 0.256 })).toBe("26%");
  expect(zoomPercent({ x: 0, y: 0, zoom: 0.0345 })).toBe("3.5%");
  expect(zoomPercent({ x: 0, y: 0, zoom: 8 })).toBe("800%");
});
