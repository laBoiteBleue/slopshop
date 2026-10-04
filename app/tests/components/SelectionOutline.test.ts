import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, waitFor } from "@testing-library/svelte";
import { afterEach, beforeEach, expect, test } from "vitest";
import SelectionOutline from "../../src/lib/SelectionOutline.svelte";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The viewport shows the document at 100%, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x, y],
  toDocument: (x, y) => [x, y],
  docPerCss: 1,
  hand: false,
};

/** One polyline of three points, as the engine sends them: line count, then n and the points. */
const TRIANGLE = new Uint32Array([1, 3, 10, 10, 50, 10, 50, 50]).buffer;

/** The window the overlay fills: jsdom lays nothing out, so it is told. */
const SIZE = { width: 200, height: 100 };

const sizes = {
  clientWidth: Object.getOwnPropertyDescriptor(Element.prototype, "clientWidth"),
  clientHeight: Object.getOwnPropertyDescriptor(Element.prototype, "clientHeight"),
};

let requests: Record<string, unknown>[];

beforeEach(() => {
  requests = [];
  Object.defineProperty(Element.prototype, "clientWidth", {
    configurable: true,
    get: () => SIZE.width,
  });
  Object.defineProperty(Element.prototype, "clientHeight", {
    configurable: true,
    get: () => SIZE.height,
  });
  mockIPC((command, args) => {
    if (command !== "selection_outline") return undefined;
    requests.push(args as Record<string, unknown>);
    return TRIANGLE;
  });
});

afterEach(() => {
  clearMocks();
  for (const name of ["clientWidth", "clientHeight"] as const) {
    if (sizes[name]) Object.defineProperty(Element.prototype, name, sizes[name]);
    else delete (Element.prototype as unknown as Record<string, unknown>)[name];
  }
});

function open(
  props: Partial<{
    hidden: boolean;
    shift: [number, number];
    selectionKey: number;
    matrix: [number, number, number, number, number, number];
  }> = {},
) {
  return render(SelectionOutline, {
    mapping: MAPPING,
    documentId: 7,
    selectionKey: 1,
    width: 400,
    height: 300,
    ...props,
  });
}

test("the engine is asked for the visible area and a view of margin around it", async () => {
  open();
  await waitFor(() => expect(requests).toHaveLength(1));
  // The window is 200 x 100 from the corner: the margin reaches the canvas's right edge (400),
  // and 100 below.
  expect(requests[0]).toEqual({ documentId: 7, x: 0, y: 0, width: 400, height: 200, zoom: 1 });
});

test("the outline the engine sends is drawn as marching ants on pixel centers", async () => {
  const { container } = open();
  await waitFor(() => expect(container.querySelector("path.ants")).toBeInTheDocument());
  expect(container.querySelector("path.ants")).toHaveAttribute(
    "d",
    "M10.5 10.5L50.5 10.5L50.5 50.5",
  );
});

test("a new selection asks again, a repeated one does not", async () => {
  const { rerender } = open();
  await waitFor(() => expect(requests).toHaveLength(1));
  await rerender({ selectionKey: 1 });
  await rerender({ selectionKey: 2 });
  await waitFor(() => expect(requests).toHaveLength(2));
  expect(requests).toHaveLength(2);
});

test("hidden, as in Quick Mask, it asks for nothing and draws nothing", async () => {
  const { container } = open({ hidden: true });
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(requests).toHaveLength(0);
  expect(container.querySelector("path")).not.toBeInTheDocument();
});

test("the outline is drawn moved while the selected pixels float in a drag", async () => {
  const { container } = open({ shift: [5, 3] });
  await waitFor(() => expect(container.querySelector("g")).toBeInTheDocument());
  expect(container.querySelector("g")).toHaveAttribute("transform", "translate(5 3)");
});

test("a matrix (Transform Selection) draws the outline mapped by it, live", async () => {
  const { container, rerender } = open({ matrix: [2, 0, 0, 2, 5, 0] });
  await waitFor(() => expect(container.querySelector("path.ants")).toBeInTheDocument());
  expect(container.querySelector("path.ants")).toHaveAttribute(
    "d",
    "M25.5 20.5L105.5 20.5L105.5 100.5",
  );
  await rerender({ matrix: [1, 0, 0, 1, 0, 0] });
  expect(container.querySelector("path.ants")).toHaveAttribute(
    "d",
    "M10.5 10.5L50.5 10.5L50.5 50.5",
  );
});

test("applied, the transformed outline stays until the new selection's outline arrives", async () => {
  // The engine answers the new selection's outline only when told.
  let release: (() => void) | null = null;
  clearMocks();
  mockIPC((command, args) => {
    if (command !== "selection_outline") return undefined;
    requests.push(args as Record<string, unknown>);
    if (requests.length === 1) return TRIANGLE;
    return new Promise((resolve) => (release = () => resolve(TRIANGLE)));
  });
  const { container, rerender } = open({ matrix: [2, 0, 0, 2, 5, 0] });
  await waitFor(() =>
    expect(container.querySelector("path.ants")).toHaveAttribute(
      "d",
      "M25.5 20.5L105.5 20.5L105.5 100.5",
    ),
  );
  // The new selection is in, its outline not fetched yet: still the transformed one.
  await rerender({ selectionKey: 2, matrix: undefined });
  expect(container.querySelector("path.ants")).toHaveAttribute(
    "d",
    "M25.5 20.5L105.5 20.5L105.5 100.5",
  );
  await waitFor(() => expect(release).not.toBeNull());
  release!();
  await waitFor(() =>
    expect(container.querySelector("path.ants")).toHaveAttribute(
      "d",
      "M10.5 10.5L50.5 10.5L50.5 50.5",
    ),
  );
});

test("cancelled, the outline is back at once", async () => {
  const { container, rerender } = open({ matrix: [2, 0, 0, 2, 5, 0] });
  await waitFor(() => expect(container.querySelector("path.ants")).toBeInTheDocument());
  await rerender({ matrix: undefined });
  expect(container.querySelector("path.ants")).toHaveAttribute(
    "d",
    "M10.5 10.5L50.5 10.5L50.5 50.5",
  );
});
