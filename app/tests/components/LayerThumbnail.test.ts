import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { LayerView } from "../../src/lib/engine";
import LayerThumbnail from "../../src/lib/LayerThumbnail.svelte";

type Request = { documentId: number; layerId: number; maxSide: number; mask: boolean };

/** The thumbnails asked of the engine, and how it answers (by default a 4 x 2 image). */
let requests: Request[];
let answer: (request: Request) => Promise<ArrayBuffer> | ArrayBuffer;

/** What the engine sends: width, height (little-endian), then RGBA8 pixels. */
function image(width: number, height: number): ArrayBuffer {
  const buffer = new ArrayBuffer(8 + width * height * 4);
  const view = new DataView(buffer);
  view.setUint32(0, width, true);
  view.setUint32(4, height, true);
  return buffer;
}

/** Rows seen by the observer of visibility: the test decides when they scroll into view. */
let observed: { callback: IntersectionObserverCallback; disconnected: boolean }[];

beforeEach(() => {
  requests = [];
  observed = [];
  answer = () => image(4, 2);
  mockIPC((cmd, args) => {
    if (cmd !== "layer_thumbnail") return;
    requests.push(args as Request);
    return answer(args as Request);
  });
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      entry = {
        callback: undefined as unknown as IntersectionObserverCallback,
        disconnected: false,
      };
      constructor(callback: IntersectionObserverCallback) {
        this.entry.callback = callback;
        observed.push(this.entry);
      }
      observe() {}
      disconnect() {
        this.entry.disconnected = true;
      }
    },
  );
});
afterEach(() => {
  clearMocks();
  vi.unstubAllGlobals();
});

/** The thumbnails are kept by content key for the whole session: each test uses its own. */
let nextKey = 1000;
/** A fresh content key; the next ones are left free for the test to change the pixels. */
const freshKey = () => (nextKey += 10);

function layer(changes: Partial<LayerView> = {}): LayerView {
  return {
    id: 7,
    name: "Layer",
    kind: "raster",
    swatch: [1, 0, 0, 1],
    contentKey: freshKey(),
    mask: null,
    ...changes,
  } as LayerView;
}

function show(
  layerView: LayerView,
  props: { size?: number; mask?: boolean; documentId?: number } = {},
) {
  const view = render(LayerThumbnail, { documentId: 1, size: 36, layer: layerView, ...props });
  const canvas = () => view.container.querySelector("canvas") as HTMLCanvasElement | null;
  return { ...view, canvas };
}

/** The rows currently waiting in the observer scroll into view. */
async function scrollIntoView() {
  for (const entry of observed.filter((o) => !o.disconnected)) {
    entry.callback(
      [{ isIntersecting: true } as IntersectionObserverEntry],
      {} as IntersectionObserver,
    );
  }
  await Promise.resolve();
}

test("a raster's thumbnail is fetched only once its row has been scrolled into view", async () => {
  const { canvas } = show(layer());
  await Promise.resolve();
  expect(requests).toHaveLength(0);
  await scrollIntoView();
  await waitFor(() => expect(canvas()?.width).toBe(4));
  expect(requests).toEqual([{ documentId: 1, layerId: 7, maxSide: 36, mask: false }]);
});

test("the thumbnail keeps its aspect ratio in the box", async () => {
  answer = () => image(4, 2);
  const { canvas } = show(layer(), { size: 40 });
  await scrollIntoView();
  await waitFor(() => expect(canvas()?.height).toBe(2));
  expect(canvas()).toHaveStyle({ width: "40px", height: "20px" });
});

test("the thumbnail is asked in device pixels, so that it stays sharp on dense screens", async () => {
  vi.stubGlobal("devicePixelRatio", 2);
  show(layer(), { size: 36 });
  await scrollIntoView();
  await waitFor(() => expect(requests).toHaveLength(1));
  expect(requests[0].maxSide).toBe(72);
});

test("a fill or a group shows its color or nothing, and never asks the engine", async () => {
  const fill = show(layer({ kind: "fill", swatch: [1, 0, 0, 1] }));
  await scrollIntoView();
  expect(fill.canvas()).toBeNull();
  const swatch = fill.container.querySelector(".fill span") as HTMLElement;
  expect(swatch).toHaveStyle({ background: "rgb(255 0 0 / 1)" });
  show(layer({ kind: "adjustment" }));
  await scrollIntoView();
  expect(requests).toHaveLength(0);
});

test("a raster shared by several layers is fetched once", async () => {
  const shared = layer();
  const first = show(shared);
  await scrollIntoView();
  await waitFor(() => expect(first.canvas()?.width).toBe(4));
  // Another layer with the same content, in the same document or another one.
  const second = show({ ...shared, id: 8 }, { documentId: 2 });
  await scrollIntoView();
  await waitFor(() => expect(second.canvas()?.width).toBe(4));
  expect(requests).toHaveLength(1);
});

test("a thumbnail is fetched again when the layer's pixels change, or at another size", async () => {
  const content = layer();
  const { rerender, canvas } = show(content);
  await scrollIntoView();
  await waitFor(() => expect(requests).toHaveLength(1));
  answer = () => image(6, 3);
  await rerender({
    documentId: 1,
    size: 36,
    layer: { ...content, contentKey: content.contentKey + 1 },
  });
  await waitFor(() => expect(canvas()?.width).toBe(6));
  expect(requests).toHaveLength(2);
  await rerender({
    documentId: 1,
    size: 20,
    layer: { ...content, contentKey: content.contentKey + 1 },
  });
  await waitFor(() => expect(requests).toHaveLength(3));
  expect(requests[2].maxSide).toBe(20);
});

test("an answer that comes late for old pixels does not replace the new thumbnail", async () => {
  const content = layer();
  const late: ((buffer: ArrayBuffer) => void)[] = [];
  answer = () => new Promise<ArrayBuffer>((resolve) => late.push(resolve));
  const { rerender, canvas } = show(content);
  await scrollIntoView();
  await waitFor(() => expect(late).toHaveLength(1));
  await rerender({
    documentId: 1,
    size: 36,
    layer: { ...content, contentKey: content.contentKey + 1 },
  });
  await waitFor(() => expect(late).toHaveLength(2));
  late[1](image(6, 3));
  await waitFor(() => expect(canvas()?.width).toBe(6));
  late[0](image(2, 2));
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(canvas()?.width).toBe(6);
});

test("the mask shows the layer's mask, a thumbnail of its own", async () => {
  const withMask = layer({ mask: { enabled: true, contentKey: freshKey() } });
  const { canvas } = show(withMask, { mask: true });
  await scrollIntoView();
  await waitFor(() => expect(canvas()?.width).toBe(4));
  expect(requests).toEqual([{ documentId: 1, layerId: 7, maxSide: 36, mask: true }]);
});

test("the layer and its mask do not share a thumbnail even with the same content key", async () => {
  const key = freshKey();
  const both = layer({ contentKey: key, mask: { enabled: true, contentKey: key } });
  show(both);
  show(both, { mask: true });
  await scrollIntoView();
  await waitFor(() => expect(requests).toHaveLength(2));
  expect(requests.map((r) => r.mask).sort()).toEqual([false, true]);
});

test("a thumbnail the engine cannot give leaves the box empty", async () => {
  answer = () => Promise.reject("no such layer");
  const { canvas } = show(layer());
  await scrollIntoView();
  await waitFor(() => expect(requests).toHaveLength(1));
  await new Promise((resolve) => setTimeout(resolve, 0));
  // Not drawn: the canvas keeps its default size.
  expect(canvas()?.width).toBe(300);
});
