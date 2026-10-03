import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import LassoTool from "../../src/lib/LassoTool.svelte";
import type { SelectionMode } from "../../src/lib/engine";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The viewport shows the document at 100%, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x, y],
  toDocument: (x, y) => [x, y],
  docPerCss: 1,
  hand: false,
};

type Point = [number, number];

function open(options: { polygonal?: boolean; mode?: SelectionMode; hand?: boolean } = {}) {
  const onselect = vi.fn();
  const ondeselect = vi.fn();
  const { container } = render(LassoTool, {
    mapping: { ...MAPPING, hand: options.hand ?? false },
    polygonal: options.polygonal ?? false,
    mode: options.mode ?? "replace",
    onselect,
    ondeselect,
  });
  const svg = container.querySelector("svg") as SVGSVGElement;
  const user = userEvent.setup();
  const at = ([x, y]: Point) => ({ target: svg, coords: { clientX: x, clientY: y } });
  return {
    onselect,
    ondeselect,
    svg,
    user,
    /** Freehand: press at the first point, move through the others, release at the last. */
    async draw(points: Point[]) {
      await user.pointer([
        { keys: "[MouseLeft>]", ...at(points[0]) },
        ...points.slice(1).map(at),
        { keys: "[/MouseLeft]", ...at(points[points.length - 1]) },
      ]);
    },
    /** Polygonal: a click on each point. */
    async click(...points: Point[]) {
      for (const point of points) await user.pointer({ keys: "[MouseLeft]", ...at(point) });
    },
    at,
  };
}

test("a freehand drag selects the polygon through the points it went by", async () => {
  const { onselect, draw } = open();
  await draw([
    [10, 10],
    [50, 10],
    [50, 50],
  ]);
  expect(onselect).toHaveBeenCalledExactlyOnceWith(
    {
      kind: "polygon",
      points: [
        [10, 10],
        [50, 10],
        [50, 50],
      ],
    },
    null,
  );
});

test("a freehand drag of fewer than three points selects nothing", async () => {
  const { onselect, ondeselect, draw } = open();
  await draw([
    [10, 10],
    [50, 10],
  ]);
  expect(onselect).not.toHaveBeenCalled();
  expect(ondeselect).not.toHaveBeenCalled();
});

test("a click without drawing deselects, unless a key asks for a mode", async () => {
  const { onselect, ondeselect, click, user } = open();
  await click([20, 20]);
  expect(ondeselect).toHaveBeenCalledOnce();

  await user.keyboard("[ShiftLeft>]");
  await click([20, 20]);
  await user.keyboard("[/ShiftLeft]");
  expect(ondeselect).toHaveBeenCalledOnce();
  expect(onselect).not.toHaveBeenCalled();
});

test.each([
  ["[ShiftLeft>]", "[/ShiftLeft]", "add"],
  ["[AltLeft>]", "[/AltLeft]", "subtract"],
  ["[ShiftLeft>][AltLeft>]", "[/AltLeft][/ShiftLeft]", "intersect"],
])("keys held at the first press (%s) ask for the mode %s", async (down, up, mode) => {
  const { onselect, draw, user } = open();
  await user.keyboard(down);
  await draw([
    [10, 10],
    [50, 10],
    [50, 50],
  ]);
  await user.keyboard(up);
  expect(onselect).toHaveBeenCalledExactlyOnceWith(expect.anything(), mode);
});

test("the badge by the pointer tells the mode of the next shape", async () => {
  const { user, at } = open({ mode: "intersect" });
  await user.pointer(at([20, 20]));
  expect(screen.getByText("×")).toBeInTheDocument();
  await user.keyboard("[AltLeft>]");
  await user.pointer(at([21, 21]));
  expect(screen.getByText("−")).toBeInTheDocument();
});

test("the Polygonal Lasso adds a corner at each click and Enter closes the shape", async () => {
  const { onselect, click, user } = open({ polygonal: true });
  await click([10, 10], [50, 10], [50, 50]);
  expect(onselect).not.toHaveBeenCalled();
  await user.keyboard("{Enter}");
  expect(onselect).toHaveBeenCalledExactlyOnceWith(
    {
      kind: "polygon",
      points: [
        [10, 10],
        [50, 10],
        [50, 50],
      ],
    },
    null,
  );
});

test("a click on the first corner closes the polygon without adding a corner", async () => {
  const { onselect, click } = open({ polygonal: true });
  await click([10, 10], [50, 10], [50, 50], [12, 12]);
  expect(onselect).toHaveBeenCalledOnce();
  expect(onselect.mock.lastCall?.[0].points).toEqual([
    [10, 10],
    [50, 10],
    [50, 50],
  ]);
});

test("a double-click closes the polygon, the corner of its two presses counted once", async () => {
  const { onselect, click, user, at } = open({ polygonal: true });
  await click([10, 10], [50, 10]);
  await user.pointer({ keys: "[MouseLeft][MouseLeft]", ...at([50, 50]) });
  expect(onselect).toHaveBeenCalledOnce();
  expect(onselect.mock.lastCall?.[0].points).toEqual([
    [10, 10],
    [50, 10],
    [50, 50],
  ]);
});

test("Backspace removes the last corner and Escape drops the shape", async () => {
  const { onselect, click, user } = open({ polygonal: true });
  await click([10, 10], [50, 10], [50, 50], [10, 50]);
  await user.keyboard("{Backspace}{Enter}");
  expect(onselect.mock.lastCall?.[0].points).toEqual([
    [10, 10],
    [50, 10],
    [50, 50],
  ]);

  await click([10, 10], [50, 10], [50, 50]);
  await user.keyboard("{Escape}{Enter}");
  expect(onselect).toHaveBeenCalledOnce();
});

test("Shift places a corner along the nearest multiple of 45 degrees", async () => {
  const { onselect, click, user } = open({ polygonal: true });
  await click([10, 10]);
  await user.keyboard("[ShiftLeft>]");
  // Almost horizontal: the corner goes straight right.
  await click([60, 14]);
  await user.keyboard("[/ShiftLeft]");
  await click([60, 60]);
  await user.keyboard("{Enter}");
  const [first, second, third] = onselect.mock.lastCall?.[0].points as Point[];
  expect(first).toEqual([10, 10]);
  expect(second[1]).toBeCloseTo(10);
  expect(second[0]).toBeGreaterThan(50);
  expect(third).toEqual([60, 60]);
});

test("while Space pans the viewport, nothing is drawn", async () => {
  const { onselect, ondeselect, draw } = open({ hand: true });
  await draw([
    [10, 10],
    [50, 10],
    [50, 50],
  ]);
  expect(onselect).not.toHaveBeenCalled();
  expect(ondeselect).not.toHaveBeenCalled();
});
