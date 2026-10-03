import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { ViewInfo, ViewRequest } from "../../src/lib/engine";
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
let current: ViewInfo;
let answer: { frame: () => ArrayBuffer; view: ViewInfo };

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", FixedSizeObserver);
  frames = [];
  views = [];
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
  expect(onmovestart).toHaveBeenCalledWith(20, 35, false, true);
  // 10 CSS pixels at 200%: 5 document pixels; half a document pixel per CSS pixel.
  expect(onmove).toHaveBeenCalledWith(5, 0, 0.5, false);
  expect(onmoveend).toHaveBeenCalledOnce();
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
  await rerender({ guides: [{ x1: 20, y1: 20, x2: 20, y2: 40 }] });
  const guide = container.querySelector(".guide") as HTMLElement;
  // (20 - 10) × 2 = 20 CSS pixels from the left; 20 document pixels tall at 200%.
  expect(guide.style.left).toBe("20px");
  expect(guide.style.top).toBe("0px");
  expect(guide.style.height).toBe("40px");
});
