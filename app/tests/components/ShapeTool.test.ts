import { render } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import ShapeTool from "../../src/lib/ShapeTool.svelte";
import { defaultShapeOptions, type ShapeKind } from "../../src/lib/shapes";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The document at 100%, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x, y],
  toDocument: (x, y) => [x, y],
  docPerCss: 1,
  hand: false,
};

function open(kind: ShapeKind, hand = false) {
  const onshape = vi.fn();
  const { container } = render(ShapeTool, {
    mapping: { ...MAPPING, hand },
    kind,
    options: { ...defaultShapeOptions("#000000"), radius: 4 },
    onshape,
  });
  const svg = container.querySelector("svg") as SVGSVGElement;
  return { onshape, container, svg, user: userEvent.setup() };
}

test("a drag shows the shape's outline, and the release draws it", async () => {
  const { onshape, container, svg, user } = open("rectangle");
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 10, clientY: 20 } },
    { target: svg, coords: { clientX: 110, clientY: 70 } },
  ]);
  expect(container.querySelector("path.outline")).not.toBeNull();
  expect(container.querySelector("text.readout")).toHaveTextContent("100");
  await user.pointer({ keys: "[/MouseLeft]", target: svg, coords: { clientX: 110, clientY: 70 } });
  expect(onshape).toHaveBeenCalledWith({
    kind: "rectangle",
    rect: [10, 20, 110, 70],
    radii: [4, 4, 4, 4],
  });
  expect(container.querySelector("path.outline")).toBeNull();
});

test("Shift draws a circle; a line keeps to 45° steps", async () => {
  const ellipse = open("ellipse");
  await ellipse.user.pointer([
    { keys: "[MouseLeft>]", target: ellipse.svg, coords: { clientX: 0, clientY: 0 } },
    { target: ellipse.svg, coords: { clientX: 40, clientY: 10 } },
  ]);
  await ellipse.user.keyboard("[ShiftLeft>]");
  await ellipse.user.pointer([
    { target: ellipse.svg, coords: { clientX: 40, clientY: 12 } },
    { keys: "[/MouseLeft]", target: ellipse.svg, coords: { clientX: 40, clientY: 12 } },
  ]);
  expect(ellipse.onshape).toHaveBeenCalledWith({
    kind: "ellipse",
    center: [20, 20],
    radii: [20, 20],
  });
  const line = open("line");
  await line.user.pointer([
    { keys: "[MouseLeft>]", target: line.svg, coords: { clientX: 0, clientY: 0 } },
    { target: line.svg, coords: { clientX: 100, clientY: 6 } },
  ]);
  await line.user.keyboard("[ShiftLeft>]");
  await line.user.pointer([
    { target: line.svg, coords: { clientX: 100, clientY: 7 } },
    { keys: "[/MouseLeft]", target: line.svg, coords: { clientX: 100, clientY: 7 } },
  ]);
  const [geometry] = line.onshape.mock.calls[0];
  expect(geometry.kind).toBe("line");
  expect(geometry.to[1]).toBeCloseTo(0);
});

test("a click, Escape or a drag while panning draws nothing", async () => {
  const { onshape, svg, user } = open("polygon");
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 10, clientY: 10 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 11, clientY: 10 } },
  ]);
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 10, clientY: 10 } },
    { target: svg, coords: { clientX: 80, clientY: 80 } },
  ]);
  await user.keyboard("{Escape}");
  await user.pointer({ keys: "[/MouseLeft]", target: svg, coords: { clientX: 80, clientY: 80 } });
  expect(onshape).not.toHaveBeenCalled();
  const panning = open("rectangle", true);
  expect(panning.svg).toHaveClass("hand");
});
