import { fireEvent, render } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import WarpOverlay from "../../src/lib/WarpOverlay.svelte";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";
import type { Homography } from "../../src/lib/homography";
import { evaluate, identityGrid, stride, type Grid, type SplitMode } from "../../src/lib/warpGrid";

/** The viewport shows the document at 200 %, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x * 2, y * 2],
  toDocument: (x, y) => [x / 2, y / 2],
  docPerCss: 0.5,
  hand: false,
};

/** The layer's content placed 10 pixels right of the document's corner. */
const MOVED: Homography = [1, 0, 0, 1, 10, 0, 0, 0, 1];

function open(grid: Grid = identityGrid([0, 0, 30, 60], 1, 2), split: SplitMode | null = null) {
  const onchange = vi.fn();
  const oncommit = vi.fn();
  const oncancel = vi.fn();
  const { container } = render(WarpOverlay, {
    mapping: MAPPING,
    grid,
    toDocument: MOVED,
    split,
    onchange,
    oncommit,
    oncancel,
  });
  const svg = container.querySelector(".warp-grid") as SVGElement;
  return { onchange, oncommit, oncancel, container, svg };
}

/** A press at document point (`x`, `y`), then a move to (`tx`, `ty`), then a release. */
async function drag(svg: Element, [x, y]: number[], [tx, ty]: number[]) {
  await fireEvent.pointerDown(svg, { pointerId: 1, button: 0, clientX: x * 2, clientY: y * 2 });
  await fireEvent.pointerMove(svg, { pointerId: 1, clientX: tx * 2, clientY: ty * 2 });
  await fireEvent.pointerUp(svg, { pointerId: 1 });
}

test("the grid's lines, anchors and handles are drawn where the view shows them", () => {
  const { container } = open();
  const paths = [...container.querySelectorAll(".warp-grid .line")];
  // Three rows of lines, two columns.
  expect(paths).toHaveLength(5);
  expect(paths[0].getAttribute("d")).toMatch(/^M20.00 0.00 L/);
  expect(paths[0].getAttribute("d")).toMatch(/L80.00 0.00$/);
  expect(container.querySelectorAll(".anchor")).toHaveLength(6);
  expect(container.querySelectorAll(".handle")).toHaveLength(14);
  const anchor = container.querySelector(".anchor") as Element;
  expect([anchor.getAttribute("x"), anchor.getAttribute("y")]).toEqual(["16.5", "-3.5"]);
});

test("dragging an anchor moves it and its handles", async () => {
  const grid = identityGrid([0, 0, 30, 60], 1, 2);
  const { onchange, svg } = open(grid);
  // The top-left anchor, at document (10, 0).
  await drag(svg, [10, 0], [15, 3]);
  const sent = onchange.mock.lastCall?.[0] as Grid;
  expect(sent.points[0]).toEqual([5, 3]);
  expect(sent.points[1]).toEqual([15, 3]);
  expect(sent.points[stride(grid)]).toEqual([5, 13]);
});

test("dragging a handle moves it alone among the edges", async () => {
  const grid = identityGrid([0, 0, 30, 60], 1, 2);
  const { onchange, svg } = open(grid);
  // The top edge's first handle, at content (10, 0).
  await drag(svg, [20, 0], [20, -6]);
  const sent = onchange.mock.lastCall?.[0] as Grid;
  expect(sent.points[1]).toEqual([10, -6]);
  expect(sent.points[0]).toEqual([0, 0]);
});

test("dragging inside a patch bends it under the pointer", async () => {
  const grid = identityGrid([0, 0, 30, 60], 1, 2);
  const { onchange, svg } = open(grid);
  // Content (15, 45): the second patch's middle.
  await drag(svg, [25, 45], [25, 51]);
  const sent = onchange.mock.lastCall?.[0] as Grid;
  const [x, y] = evaluate(sent, 0.5, 1.5);
  expect(x).toBeCloseTo(15, 9);
  expect(y).toBeCloseTo(51, 9);
});

test("a press outside the grid, or a click without a move, changes nothing", async () => {
  const { onchange, svg } = open();
  await drag(svg, [80, 80], [90, 90]);
  await fireEvent.pointerDown(svg, { pointerId: 2, button: 0, clientX: 50, clientY: 90 });
  await fireEvent.pointerUp(svg, { pointerId: 2 });
  expect(onchange).not.toHaveBeenCalled();
});

test("with Split Warp on, a click splits the grid there", async () => {
  const { onchange, svg } = open(identityGrid([0, 0, 30, 60], 1, 2), "crosswise");
  expect(svg).toHaveClass("splitting");
  await fireEvent.pointerDown(svg, { pointerId: 1, button: 0, clientX: 40, clientY: 30 });
  const sent = onchange.mock.lastCall?.[0] as Grid;
  expect([sent.columns, sent.rows]).toEqual([2, 3]);
});

test("Enter applies, Esc cancels, a double-click applies", async () => {
  const { oncommit, oncancel, svg } = open();
  const user = userEvent.setup();
  await user.keyboard("{Enter}");
  expect(oncommit).toHaveBeenCalledTimes(1);
  await user.keyboard("{Escape}");
  expect(oncancel).toHaveBeenCalledTimes(1);
  await user.dblClick(svg);
  expect(oncommit).toHaveBeenCalledTimes(2);
});
