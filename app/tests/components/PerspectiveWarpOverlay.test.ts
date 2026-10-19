import { fireEvent, render } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import PerspectiveWarpOverlay from "../../src/lib/PerspectiveWarpOverlay.svelte";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";
import type { Homography } from "../../src/lib/homography";
import {
  addPlane,
  NO_PLANES,
  type PerspectiveMode,
  type Planes,
} from "../../src/lib/perspectiveWarp";

/** The viewport shows the document at 200 %, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x * 2, y * 2],
  toDocument: (x, y) => [x / 2, y / 2],
  docPerCss: 0.5,
  hand: false,
};

/** The layer's content placed 10 pixels right of the document's corner. */
const MOVED: Homography = [1, 0, 0, 1, 10, 0, 0, 0, 1];

/** One plane over content (0, 0)–(40, 30). */
const ONE = addPlane(NO_PLANES, [0, 0], [40, 30], 0) as Planes;

function open(planes: Planes, mode: PerspectiveMode, selected: number | null = null) {
  const onchange = vi.fn();
  const onselect = vi.fn();
  const oncommit = vi.fn();
  const oncancel = vi.fn();
  const { container, rerender } = render(PerspectiveWarpOverlay, {
    mapping: MAPPING,
    planes,
    mode,
    toDocument: MOVED,
    selected,
    onchange,
    onselect,
    oncommit,
    oncancel,
  });
  const svg = container.querySelector(".perspective-warp") as SVGElement;
  return { onchange, onselect, oncommit, oncancel, container, svg, rerender };
}

/** A press at document point (`x`, `y`), then a move to (`tx`, `ty`), then a release. */
async function drag(svg: Element, [x, y]: number[], [tx, ty]: number[]) {
  await fireEvent.pointerDown(svg, { pointerId: 1, button: 0, clientX: x * 2, clientY: y * 2 });
  await fireEvent.pointerMove(svg, { pointerId: 1, clientX: tx * 2, clientY: ty * 2 });
  await fireEvent.pointerUp(svg, { pointerId: 1 });
}

test("each plane's edges and lines across, and its corners, are drawn where the view shows them", () => {
  const { container } = open(ONE, "layout");
  expect(container.querySelectorAll(".plane")).toHaveLength(1);
  // Four edges, two lines across each way.
  expect(container.querySelectorAll(".plane line")).toHaveLength(8);
  expect(container.querySelectorAll(".plane line.edge")).toHaveLength(4);
  const pins = [...container.querySelectorAll(".pin")];
  expect(pins).toHaveLength(4);
  // Content (40, 0): document (50, 0), at 200 %.
  expect([pins[1].getAttribute("cx"), pins[1].getAttribute("cy")]).toEqual(["100", "0"]);
});

test("in Layout mode, a drag on the image draws a plane", async () => {
  const { onchange, onselect, svg } = open(NO_PLANES, "layout");
  // Document (20, 10) to (60, 40): content (10, 10) to (50, 40).
  await drag(svg, [20, 10], [60, 40]);
  const sent = onchange.mock.lastCall?.[0] as Planes;
  expect(sent.planes).toEqual([[0, 1, 2, 3]]);
  expect(sent.layout).toEqual([
    [10, 10],
    [50, 10],
    [50, 40],
    [10, 40],
  ]);
  expect(onselect).toHaveBeenLastCalledWith(0);
});

test("a plane drawn by another one's corners shares its edge", async () => {
  const { onchange, svg } = open(ONE, "layout");
  // From near the plane's top right corner (content (40, 0)): 7 CSS pixels off, beyond the
  // reach of its pin but within the snap.
  await drag(svg, [53, 2], [80, 29]);
  const sent = onchange.mock.lastCall?.[0] as Planes;
  expect(sent.planes[1]).toEqual([1, 4, 5, 2]);
});

test("in Layout mode, dragging a corner lays it out there; in Warp mode, moves it", async () => {
  const layout = open(ONE, "layout");
  // The top-left corner, at document (10, 0).
  await drag(layout.svg, [10, 0], [13, 2]);
  const laid = layout.onchange.mock.lastCall?.[0] as Planes;
  expect(laid.layout[0]).toEqual([3, 2]);
  expect(laid.warped[0]).toEqual([3, 2]);
  const warp = open(ONE, "warp");
  await drag(warp.svg, [10, 0], [13, 2]);
  const moved = warp.onchange.mock.lastCall?.[0] as Planes;
  expect(moved.layout[0]).toEqual([0, 0]);
  expect(moved.warped[0]).toEqual([3, 2]);
});

test("a corner dragged across its plane stays where it was", async () => {
  const { onchange, svg } = open(ONE, "warp");
  await drag(svg, [10, 0], [60, 40]);
  expect(onchange).not.toHaveBeenCalled();
});

test("in Warp mode, a drag on the image draws nothing", async () => {
  const { onchange, svg } = open(ONE, "warp");
  await drag(svg, [80, 80], [120, 120]);
  expect(onchange).not.toHaveBeenCalled();
});

test("a click selects the plane under it; Delete removes it in Layout mode", async () => {
  const { onselect, onchange, svg, rerender } = open(ONE, "layout");
  await fireEvent.pointerDown(svg, { pointerId: 1, button: 0, clientX: 60, clientY: 30 });
  await fireEvent.pointerUp(svg, { pointerId: 1 });
  expect(onselect).toHaveBeenLastCalledWith(0);
  await rerender({ selected: 0 });
  const user = userEvent.setup();
  await user.keyboard("{Delete}");
  expect(onchange).toHaveBeenLastCalledWith(NO_PLANES);
  expect(onselect).toHaveBeenLastCalledWith(null);
});

test("Delete never reaches the window's shortcuts while the planes show", async () => {
  const { onchange } = open(ONE, "warp", 0);
  const reached = vi.fn();
  window.addEventListener("keydown", reached);
  const user = userEvent.setup();
  await user.keyboard("{Delete}");
  window.removeEventListener("keydown", reached);
  expect(reached).not.toHaveBeenCalled();
  // Warp mode: nothing removed.
  expect(onchange).not.toHaveBeenCalled();
});

test("Enter applies, Esc cancels, a double-click applies", async () => {
  const { oncommit, oncancel, svg } = open(ONE, "warp");
  const user = userEvent.setup();
  await user.keyboard("{Enter}");
  expect(oncommit).toHaveBeenCalledTimes(1);
  await user.keyboard("{Escape}");
  expect(oncancel).toHaveBeenCalledTimes(1);
  await user.dblClick(svg);
  expect(oncommit).toHaveBeenCalledTimes(2);
});
