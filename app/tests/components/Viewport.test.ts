import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { Matrix, ViewInfo, ViewRequest } from "../../src/lib/engine";
import Viewport from "../../src/lib/Viewport.svelte";

// The viewport is 200 × 100 CSS pixels at 100% display scale (jsdom's devicePixelRatio is 1).
class FixedSizeObserver {
  constructor(private readonly callback: ResizeObserverCallback) {}
  observe() {
    const entry = { contentRect: { width: 200, height: 100 } } as ResizeObserverEntry;
    this.callback([entry], this as unknown as ResizeObserver);
  }
  unobserve() {}
  disconnect() {}
}

/** A frame as the engine sends it (see `parseFrame` in engine.ts): header, then RGBA8. */
function frame(
  width: number,
  height: number,
  { zoom = 1, origin = [0, 0], fit = true, documentId = 1 } = {},
): ArrayBuffer {
  const buffer = new ArrayBuffer(56 + width * height * 4);
  const view = new DataView(buffer);
  view.setUint32(0, 2, true);
  view.setUint32(4, width, true);
  view.setUint32(8, height, true);
  view.setUint32(12, fit ? 1 : 0, true);
  view.setBigUint64(16, 0n, true);
  view.setFloat64(24, zoom, true);
  view.setFloat32(32, 5, true);
  view.setUint32(36, documentId, true);
  view.setFloat64(40, origin[0], true);
  view.setFloat64(48, origin[1], true);
  return buffer;
}

/** What the engine was asked, and how it answers: frames show the latest view, as its do. */
let frames: { documentId: number; width: number; height: number }[];
let views: ViewRequest[];
/** The native presents asked, with the ants they carry. */
let presents: { documentId: number; ants: unknown }[];
let current: ViewInfo;
let answer: { frame: () => ArrayBuffer; view: ViewInfo };

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", FixedSizeObserver);
  frames = [];
  views = [];
  presents = [];
  current = { zoom: 1, origin: [0, 0], fit: true };
  answer = {
    frame: () => frame(2, 1, current),
    view: { zoom: 2, origin: [10, 20], fit: false },
  };
  mockIPC((cmd, args) => {
    if (cmd === "render_view") {
      frames.push(args as (typeof frames)[number]);
      return answer.frame();
    }
    if (cmd === "present_view") {
      presents.push(args as (typeof presents)[number]);
      return {
        presented: true,
        complete: true,
        revision: 1,
        zoom: 1,
        origin: [0, 0],
        fit: true,
        renderMs: 1,
      };
    }
    if (cmd === "view") {
      views.push((args as { request: ViewRequest }).request);
      current = answer.view;
      return answer.view;
    }
  });
});
afterEach(() => {
  clearMocks();
  vi.unstubAllGlobals();
});

function open(props: Record<string, unknown> = {}) {
  const callbacks = {
    onframe: vi.fn(),
    onmovestart: vi.fn(),
    onmove: vi.fn(),
    onmoveend: vi.fn(),
    ondoubleclick: vi.fn(),
  };
  const view = render(Viewport, { documentId: 1, revision: 1, ...callbacks, ...props });
  const area = view.container.querySelector(".viewport") as HTMLElement;
  return { ...view, ...callbacks, area, user: userEvent.setup() };
}

test("a frame is asked at the viewport's size in device pixels, and reported", async () => {
  const { onframe } = open();
  await vi.waitFor(() => expect(onframe).toHaveBeenCalled());
  expect(frames).toEqual([{ documentId: 1, width: 200, height: 100 }]);
  expect(onframe).toHaveBeenLastCalledWith(
    expect.objectContaining({ zoom: 1, fit: true, renderMs: 5 }),
  );
});

test("a new revision draws a new frame; a frame of another document is dropped", async () => {
  const { onframe, rerender } = open();
  await vi.waitFor(() => expect(onframe).toHaveBeenCalledOnce());
  answer.frame = () => frame(2, 1, { ...current, documentId: 9 });
  await rerender({ revision: 2 });
  await vi.waitFor(() => expect(frames).toHaveLength(2));
  expect(onframe).toHaveBeenCalledOnce();
});

test("a frame that fails to render says so", async () => {
  answer.frame = () => {
    throw new Error("device lost");
  };
  open();
  expect(await screen.findByRole("alert")).toHaveTextContent("device lost");
});

test("the middle button pans, by device pixels", async () => {
  const { area, user } = open();
  await user.pointer([
    { keys: "[MouseMiddle>]", target: area, coords: { clientX: 50, clientY: 50 } },
    { target: area, coords: { clientX: 60, clientY: 45 } },
    { keys: "[/MouseMiddle]", target: area },
  ]);
  expect(views).toContainEqual({ kind: "pan", dx: 10, dy: -5 });
});

test("Space and a left drag pan too, even with the Move tool", async () => {
  const { area, onmovestart, user } = open();
  await user.keyboard("[Space>]");
  await user.pointer([
    { keys: "[MouseLeft>]", target: area, coords: { clientX: 50, clientY: 50 } },
    { target: area, coords: { clientX: 40, clientY: 50 } },
    { keys: "[/MouseLeft]", target: area },
  ]);
  await user.keyboard("[/Space]");
  expect(views).toContainEqual({ kind: "pan", dx: -10, dy: 0 });
  expect(onmovestart).not.toHaveBeenCalled();
});

test("the Move tool's drag starts at a document point and moves by document pixels", async () => {
  const { area, component, onmovestart, onmove, onmoveend, user } = open();
  // The engine's view: 200%, the document's (10, 20) at the corner.
  await component.zoomTo(2);
  expect(views).toContainEqual({ kind: "setZoom", zoom: 2 });
  await user.keyboard("[AltLeft>]");
  await user.pointer([
    { keys: "[MouseLeft>]", target: area, coords: { clientX: 20, clientY: 30 } },
    { target: area, coords: { clientX: 30, clientY: 30 } },
    { keys: "[/MouseLeft]", target: area },
  ]);
  await user.keyboard("[/AltLeft]");
  expect(onmovestart).toHaveBeenCalledWith(20, 35, { ctrl: false, alt: true, shift: false });
  // 10 CSS pixels at 200%: 5 document pixels; half a document pixel per CSS pixel.
  expect(onmove).toHaveBeenCalledWith(5, 0, 0.5, { free: false, shift: false });
  expect(onmoveend).toHaveBeenCalledOnce();
});

test("the Move tool's press and drag say whether Shift is held", async () => {
  const { area, onmovestart, onmove, user } = open();
  await user.keyboard("[ShiftLeft>]");
  await user.pointer([
    { keys: "[MouseLeft>]", target: area, coords: { clientX: 20, clientY: 30 } },
    { target: area, coords: { clientX: 30, clientY: 30 } },
    { keys: "[/MouseLeft]", target: area },
  ]);
  await user.keyboard("[/ShiftLeft]");
  expect(onmovestart).toHaveBeenCalledWith(expect.any(Number), expect.any(Number), {
    ctrl: false,
    alt: false,
    shift: true,
  });
  expect(onmove).toHaveBeenCalledWith(10, 0, 1, { free: false, shift: true });
});

test("a double-click with the Move tool asks for Free Transform", async () => {
  const { area, ondoubleclick, user } = open();
  await user.dblClick(area);
  expect(ondoubleclick).toHaveBeenCalledOnce();
});

test("the wheel zooms about the pointer, eased over a few frames", async () => {
  const { area } = open();
  area.dispatchEvent(
    new WheelEvent("wheel", { deltaY: -100, clientX: 40, clientY: 30, cancelable: true }),
  );
  // 100 pixels up: e^0.2, reached in steps (requests merged while one waits).
  await vi.waitFor(() => {
    const zooms = views.filter((v) => v.kind === "zoomBy");
    const total = zooms.reduce((product, v) => product * v.factor, 1);
    expect(total).toBeCloseTo(Math.exp(0.2), 6);
  });
  expect(views.every((v) => v.kind !== "zoomBy" || (v.x === 40 && v.y === 30))).toBe(true);
});

test("the app's zoom commands: fit, presets and a set zoom", async () => {
  const { component } = open();
  await component.fit();
  await component.stepZoom(true);
  await component.zoomTo(0.5);
  expect(views).toEqual([
    { kind: "fit" },
    { kind: "step", zoomIn: true, x: null, y: null },
    { kind: "setZoom", zoom: 0.5 },
  ]);
});

test("smart guides are drawn where the view shows them", async () => {
  const { component, container, rerender } = open();
  await component.zoomTo(2);
  await rerender({ smartGuides: [{ x1: 20, y1: 20, x2: 20, y2: 40 }] });
  const guide = container.querySelector("svg.smart-guides line.guide") as SVGLineElement;
  // (20 - 10) × 2 = 20 CSS pixels from the left; 20 document pixels tall at 200%.
  const at = (name: string) => Number(guide.getAttribute(name));
  expect([at("x1"), at("y1"), at("x2"), at("y2")]).toEqual([20, 0, 20, 40]);
});

// --- Native presentation: the engine draws the selection's ants in the view -----------------

const ANTS = { matrix: [1, 0, 0, 1, 0, 0] as Matrix, march: true };

/** Presents made while `ms` pass. */
async function presentsDuring(ms: number): Promise<number> {
  const before = presents.length;
  await new Promise((resolve) => setTimeout(resolve, ms));
  return presents.length - before;
}

test("a native present carries the ants to draw, where the selection is shown", async () => {
  const moved = { matrix: [1, 0, 0, 1, 30, -20] as Matrix, march: false };
  open({ native: true, ants: moved });
  await vi.waitFor(() => expect(presents).not.toHaveLength(0));
  expect(presents[0]).toMatchObject({ documentId: 1, ants: moved });
  expect(frames).toEqual([]);
});

test("without ants, a native present asks for none", async () => {
  open({ native: true });
  await vi.waitFor(() => expect(presents).not.toHaveLength(0));
  expect(presents[0].ants).toBeNull();
});

test("the view is presented again while the ants march, and not before they are shown", async () => {
  const { rerender } = open({ native: true });
  await vi.waitFor(() => expect(presents).not.toHaveLength(0));
  // Nothing marches: the view rests after its first present.
  expect(await presentsDuring(300)).toBe(0);
  await rerender({ ants: ANTS });
  // The ants appearing is a present; then one about every 66 ms.
  await vi.waitFor(() => expect(presents.length).toBeGreaterThanOrEqual(5), { timeout: 2000 });
  await rerender({ ants: null });
  // Gone: the timer stops with them (a last present in flight may land).
  await presentsDuring(100);
  expect(await presentsDuring(300)).toBe(0);
});

test("ants that stand still (reduced motion) are presented once, not again and again", async () => {
  const { rerender } = open({ native: true, ants: { ...ANTS, march: false } });
  await vi.waitFor(() => expect(presents).not.toHaveLength(0));
  await presentsDuring(100);
  expect(await presentsDuring(300)).toBe(0);
  // Moved (a drag's shift): one more present, with where they are now.
  const moved = { matrix: [1, 0, 0, 1, 4, 0] as Matrix, march: false };
  await rerender({ ants: moved });
  await vi.waitFor(() => expect(presents.at(-1)?.ants).toEqual(moved));
  await presentsDuring(100);
  expect(await presentsDuring(300)).toBe(0);
});

test("the same ants again present nothing new", async () => {
  const { rerender } = open({ native: true, ants: { ...ANTS, march: false } });
  await vi.waitFor(() => expect(presents).not.toHaveLength(0));
  await presentsDuring(100);
  await rerender({ ants: { matrix: [1, 0, 0, 1, 0, 0], march: false } });
  expect(await presentsDuring(300)).toBe(0);
});

test("frames over IPC never march: the SVG overlay draws the ants", async () => {
  const { onframe } = open({ ants: ANTS });
  await vi.waitFor(() => expect(onframe).toHaveBeenCalled());
  expect(await presentsDuring(300)).toBe(0);
  expect(frames).toHaveLength(1);
});

/** The image's area on the window: right of and below 18-pixel rulers (jsdom lays nothing out). */
function placeImage(area: HTMLElement) {
  area.getBoundingClientRect = () => DOMRect.fromRect({ x: 18, y: 18, width: 200, height: 100 });
}

test("View > Rulers: the document's pixels along the top and the left, labelled", async () => {
  const { container } = open({ rulers: true });
  await vi.waitFor(() => expect(container.querySelectorAll("svg.ruler")).toHaveLength(2));
  const [top, left] = container.querySelectorAll("svg.ruler");
  // 100%: a label every 100 pixels, from the document's origin at the corner.
  expect([...top.querySelectorAll("text")].map((t) => t.textContent?.trim())).toEqual([
    "0",
    "100",
    "200",
  ]);
  expect([...left.querySelectorAll("text")].map((t) => t.textContent?.trim())).toEqual([
    "0",
    "100",
  ]);
  expect(open().container.querySelector("svg.ruler")).not.toBeInTheDocument();
});

test("a guide dragged out of a ruler lands on a whole pixel where it is released", async () => {
  const onguides = vi.fn();
  const { area, component, container, user } = open({ rulers: true, onguides });
  placeImage(area);
  await component.zoomTo(2);
  const [top, left] = container.querySelectorAll("svg.ruler");
  // From the top ruler: a horizontal guide. At 200% from the document's (10, 20), the window's
  // y = 49 is the document's 20 + (49 - 18) / 2 = 35.5, landing on 36.
  await user.pointer([
    { keys: "[MouseLeft>]", target: top, coords: { clientX: 60, clientY: 10 } },
    { target: area, coords: { clientX: 60, clientY: 49 } },
  ]);
  expect(container.querySelector("line.guide.dragged")).toBeInTheDocument();
  await user.pointer({ keys: "[/MouseLeft]", target: area, coords: { clientX: 60, clientY: 49 } });
  expect(onguides).toHaveBeenCalledExactlyOnceWith([{ vertical: false, position: 36 }]);
  // From the left ruler, and back onto it: nothing.
  await user.pointer([
    { keys: "[MouseLeft>]", target: left, coords: { clientX: 10, clientY: 60 } },
    { target: area, coords: { clientX: 80, clientY: 60 } },
    { keys: "[/MouseLeft]", target: left, coords: { clientX: 10, clientY: 60 } },
  ]);
  expect(onguides).toHaveBeenCalledOnce();
});

test("with the Move tool a guide is moved, or deleted when dragged out of the image", async () => {
  const onguides = vi.fn();
  const onguidestart = vi.fn();
  const guides = [
    { vertical: true, position: 20 },
    { vertical: false, position: 30 },
  ];
  const { area, component, container, onmovestart, user } = open({
    rulers: true,
    guides,
    guidesMovable: true,
    onguides,
    onguidestart,
  });
  placeImage(area);
  await component.zoomTo(2);
  const grab = () => container.querySelector("line.grab.vertical") as SVGLineElement;
  // The vertical guide at x = 20: (20 - 10) × 2 = 20 CSS pixels into the image.
  expect(Number(grab().getAttribute("x1"))).toBe(20.5);
  await user.pointer([
    { keys: "[MouseLeft>]", target: grab(), coords: { clientX: 38, clientY: 50 } },
    { target: area, coords: { clientX: 58, clientY: 50 } },
    { keys: "[/MouseLeft]", target: area, coords: { clientX: 58, clientY: 50 } },
  ]);
  expect(onguidestart).toHaveBeenCalledOnce();
  // Not the Move tool's drag of the layers.
  expect(onmovestart).not.toHaveBeenCalled();
  expect(onguides).toHaveBeenLastCalledWith([
    { vertical: true, position: 30 },
    { vertical: false, position: 30 },
  ]);
  await user.pointer([
    { keys: "[MouseLeft>]", target: grab(), coords: { clientX: 38, clientY: 50 } },
    { target: area, coords: { clientX: 300, clientY: 50 } },
    { keys: "[/MouseLeft]", target: area, coords: { clientX: 300, clientY: 50 } },
  ]);
  expect(onguides).toHaveBeenLastCalledWith([{ vertical: false, position: 30 }]);
});

test("a dragged guide snaps where the owner says, Ctrl held letting it free", async () => {
  const onguides = vi.fn();
  const snapGuide = vi.fn((at: number, _vertical: boolean, _docPerCss: number, free: boolean) =>
    free ? at : 50,
  );
  const { area, component, container, user } = open({ rulers: true, onguides, snapGuide });
  placeImage(area);
  await component.zoomTo(2);
  const [top] = container.querySelectorAll("svg.ruler");
  await user.pointer([
    { keys: "[MouseLeft>]", target: top, coords: { clientX: 60, clientY: 10 } },
    { target: area, coords: { clientX: 60, clientY: 49 } },
    { keys: "[/MouseLeft]", target: area, coords: { clientX: 60, clientY: 49 } },
  ]);
  expect(snapGuide).toHaveBeenLastCalledWith(35.5, false, 0.5, false);
  expect(onguides).toHaveBeenLastCalledWith([{ vertical: false, position: 50 }]);
  await user.keyboard("[ControlLeft>]");
  await user.pointer([
    { keys: "[MouseLeft>]", target: top, coords: { clientX: 60, clientY: 10 } },
    { target: area, coords: { clientX: 60, clientY: 49 } },
    { keys: "[/MouseLeft]", target: area, coords: { clientX: 60, clientY: 49 } },
  ]);
  await user.keyboard("[/ControlLeft]");
  expect(onguides).toHaveBeenLastCalledWith([{ vertical: false, position: 36 }]);
});

test("Hide Extras hides the guides and the smart guides; guides cannot be picked up then", async () => {
  const guides = [{ vertical: true, position: 20 }];
  const smartGuides = [{ x1: 20, y1: 20, x2: 20, y2: 40 }];
  const { component, container, rerender } = open({ guides, smartGuides, guidesMovable: true });
  await component.zoomTo(2);
  expect(container.querySelector("svg.guides line.guide")).toBeInTheDocument();
  expect(container.querySelector("svg.smart-guides")).toBeInTheDocument();
  await rerender({ extras: false });
  expect(container.querySelector("svg.guides")).not.toBeInTheDocument();
  expect(container.querySelector("svg.smart-guides")).not.toBeInTheDocument();
});

test("the pixel grid shows by itself from 800%, over the canvas", async () => {
  answer.view = { zoom: 4, origin: [0, 0], fit: false };
  const { component, container, rerender } = open({ canvasSize: { width: 40, height: 30 } });
  await component.zoomTo(4);
  expect(container.querySelector(".pixel-grid")).not.toBeInTheDocument();
  answer.view = { zoom: 8, origin: [0, 0], fit: false };
  await component.zoomTo(8);
  const grid = container.querySelector(".pixel-grid") as HTMLElement;
  // 40 × 30 pixels at 800%, cut at the viewport's 200 × 100.
  expect([grid.style.width, grid.style.height, grid.style.backgroundSize]).toEqual([
    "200px",
    "100px",
    "8px 8px",
  ]);
  await rerender({ extras: false });
  expect(container.querySelector(".pixel-grid")).not.toBeInTheDocument();
});

test("rulers in a length unit at the document's resolution; a right-click asks for the unit", async () => {
  const onrulermenu = vi.fn();
  // 254 pixels per inch: 100 pixels per centimeter, so 0.01 cm per CSS pixel at 100%.
  const { container } = open({ rulers: true, rulerUnit: "cm", resolution: 254, onrulermenu });
  await vi.waitFor(() => expect(container.querySelectorAll("svg.ruler")).toHaveLength(2));
  const [top] = container.querySelectorAll("svg.ruler");
  expect([...top.querySelectorAll("text")].map((t) => t.textContent?.trim())).toEqual([
    "0",
    "1",
    "2",
  ]);
  const menu = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 30 });
  top.dispatchEvent(menu);
  expect(menu.defaultPrevented).toBe(true);
  // Taken: not the image's menu, which leaves a prevented right-click alone.
  expect(onrulermenu).toHaveBeenCalledWith(30, 0);
});
