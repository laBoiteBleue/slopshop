import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import PenTool from "../../src/lib/PenTool.svelte";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The viewport shows the document at 100%, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x, y],
  toDocument: (x, y) => [x, y],
  docPerCss: 1,
  hand: false,
};

type Point = [number, number];

function open() {
  const onpath = vi.fn();
  const onmakeselection = vi.fn();
  const { container } = render(PenTool, { mapping: MAPPING, onpath, onmakeselection });
  const svg = container.querySelector("svg") as SVGSVGElement;
  const user = userEvent.setup();
  const at = ([x, y]: Point) => ({ target: svg, coords: { clientX: x, clientY: y } });
  return {
    onpath,
    onmakeselection,
    user,
    at,
    async click(...points: Point[]) {
      for (const point of points) await user.pointer({ keys: "[MouseLeft]", ...at(point) });
    },
    async drag(from: Point, to: Point) {
      await user.pointer([
        { keys: "[MouseLeft>]", ...at(from) },
        { ...at([(from[0] + to[0]) / 2, (from[1] + to[1]) / 2]) },
        { ...at(to) },
        { keys: "[/MouseLeft]", ...at(to) },
      ]);
    },
  };
}

const anchors = () => screen.queryAllByTestId("pen-anchor");

test("clicks add corner anchors; a click on the first one closes the path into a layer", async () => {
  const { click, onpath } = open();
  await click([10, 10], [100, 10], [100, 80]);
  expect(anchors()).toHaveLength(3);
  expect(onpath).not.toHaveBeenCalled();
  // Within a few pixels of the first anchor.
  await click([12, 11]);
  expect(onpath).toHaveBeenCalledExactlyOnceWith({
    kind: "path",
    rule: "nonzero",
    subpaths: [
      {
        start: [10, 10],
        segments: [
          [100, 10],
          [100, 80],
          [10, 10],
        ],
        closed: true,
      },
    ],
  });
  // Done: nothing drawn any more.
  expect(anchors()).toHaveLength(0);
});

test("a drag makes a smooth anchor; Enter finishes the open path", async () => {
  const { click, drag, user, onpath } = open();
  await click([0, 0]);
  await drag([50, 0], [60, 10]);
  await user.keyboard("{Enter}");
  expect(onpath).toHaveBeenCalledExactlyOnceWith({
    kind: "path",
    rule: "nonzero",
    subpaths: [{ start: [0, 0], segments: [[0, 0, 40, -10, 50, 0]], closed: false }],
  });
});

test("Alt during the drag moves the outgoing handle alone", async () => {
  const { click, user, at, onpath } = open();
  await click([0, 0]);
  await user.pointer([{ keys: "[MouseLeft>]", ...at([50, 0]) }, { ...at([60, 0]) }]);
  await user.keyboard("{Alt>}");
  await user.pointer([{ ...at([50, 20]) }, { keys: "[/MouseLeft]", ...at([50, 20]) }]);
  await user.keyboard("{/Alt}");
  await click([100, 0]);
  await user.keyboard("{Enter}");
  const segments = onpath.mock.calls[0][0].subpaths[0].segments;
  // The incoming handle stayed where the symmetric drag left it (40, 0); the outgoing one
  // followed the pointer.
  expect(segments).toEqual([
    [0, 0, 40, 0, 50, 0],
    [50, 20, 100, 0, 100, 0],
  ]);
});

test("Shift keeps the next anchor on a 45° step", async () => {
  const { click, user, at, onpath } = open();
  await click([0, 0]);
  await user.keyboard("{Shift>}");
  await user.pointer({ keys: "[MouseLeft]", ...at([100, 8]) });
  await user.keyboard("{/Shift}{Enter}");
  const [x, y] = onpath.mock.calls[0][0].subpaths[0].segments[0];
  expect(y).toBeCloseTo(0);
  expect(x).toBeCloseTo(Math.hypot(100, 8));
});

test("a rubber band follows the pointer from the last anchor", async () => {
  const { click, user, at } = open();
  expect(screen.queryByTestId("pen-band")).toBeNull();
  await click([10, 10]);
  await user.pointer(at([40, 30]));
  expect(screen.getByTestId("pen-band")).toHaveAttribute("d", "M10.00 10.00 L40.00 30.00");
});

test("Backspace removes the last anchor, Esc drops the path, a lone anchor finishes nothing", async () => {
  const { click, user, onpath } = open();
  await click([0, 0], [10, 0], [10, 10]);
  await user.keyboard("{Backspace}");
  expect(anchors()).toHaveLength(2);
  await user.keyboard("{Escape}");
  expect(anchors()).toHaveLength(0);
  await click([5, 5]);
  await user.keyboard("{Enter}");
  expect(onpath).not.toHaveBeenCalled();
});

test("Ctrl+Enter makes a selection of the path being drawn, or of the layer's without one", async () => {
  const { click, user, onmakeselection, onpath } = open();
  await click([0, 0], [10, 0], [10, 10]);
  await user.keyboard("{Control>}{Enter}{/Control}");
  expect(onmakeselection).toHaveBeenCalledExactlyOnceWith({
    kind: "path",
    rule: "nonzero",
    subpaths: [
      {
        start: [0, 0],
        segments: [
          [10, 0],
          [10, 10],
        ],
        closed: false,
      },
    ],
  });
  // The path became the selection, not a layer.
  expect(onpath).not.toHaveBeenCalled();
  expect(anchors()).toHaveLength(0);
  await user.keyboard("{Control>}{Enter}{/Control}");
  expect(onmakeselection).toHaveBeenLastCalledWith(null);
});
