import { fireEvent, render } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import PaintTool from "../../src/lib/PaintTool.svelte";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The viewport shows the document at 100%, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x, y],
  toDocument: (x, y) => [x, y],
  docPerCss: 1,
  hand: false,
};

function open(options: { hand?: boolean; size?: number } = {}) {
  const onstroke = vi.fn();
  const { container } = render(PaintTool, {
    mapping: { ...MAPPING, hand: options.hand ?? false },
    size: options.size ?? 20,
    onstroke,
  });
  return {
    onstroke,
    svg: container.querySelector("svg") as SVGSVGElement,
    user: userEvent.setup(),
  };
}

test("a drag sends the start, the samples of the moves, then the end, at full pressure", async () => {
  const { onstroke, svg, user } = open();
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 10, clientY: 10 } },
    { target: svg, coords: { clientX: 20, clientY: 12 } },
    { target: svg, coords: { clientX: 30, clientY: 14 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 30, clientY: 14 } },
  ]);
  expect(onstroke.mock.calls).toEqual([
    [[[10, 10, 1]], "start"],
    [[[20, 12, 1]], "move"],
    [[[30, 14, 1]], "move"],
    [[[30, 14, 1]], "end"],
  ]);
});

test("Shift at the press asks for a straight line from where the last stroke ended", async () => {
  const { onstroke, svg, user } = open();
  await user.keyboard("[ShiftLeft>]");
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: { clientX: 40, clientY: 40 } });
  await user.keyboard("[/ShiftLeft]");
  expect(onstroke.mock.calls).toEqual([
    [[[40, 40, 1]], "line"],
    [[[40, 40, 1]], "end"],
  ]);
});

test("a pen's pressure goes with each sample", async () => {
  const { onstroke, svg } = open();
  const pen = { pointerType: "pen", pointerId: 2 };
  await fireEvent.pointerDown(svg, { ...pen, button: 0, pressure: 0.25, clientX: 5, clientY: 6 });
  await fireEvent.pointerMove(svg, { ...pen, pressure: 0.75, clientX: 8, clientY: 9 });
  await fireEvent.pointerUp(svg, { ...pen, pressure: 0, clientX: 8, clientY: 9 });
  expect(onstroke.mock.calls).toEqual([
    [[[5, 6, 0.25]], "start"],
    [[[8, 9, 0.75]], "move"],
    [[[8, 9, 0]], "end"],
  ]);
});

test("moving without pressing paints nothing, and only the left button paints", async () => {
  const { onstroke, svg, user } = open();
  await user.pointer({ target: svg, coords: { clientX: 10, clientY: 10 } });
  await user.pointer({ keys: "[MouseRight]", target: svg, coords: { clientX: 12, clientY: 12 } });
  await user.pointer({ target: svg, coords: { clientX: 20, clientY: 20 } });
  expect(onstroke).not.toHaveBeenCalled();
});

test("while Space pans the viewport, a drag paints nothing", async () => {
  const { onstroke, svg, user } = open({ hand: true });
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 10, clientY: 10 } },
    { target: svg, coords: { clientX: 20, clientY: 20 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 20, clientY: 20 } },
  ]);
  expect(onstroke).not.toHaveBeenCalled();
});

test("a pointer cancelled mid-stroke ends the stroke, and later moves paint nothing", async () => {
  const { onstroke, svg, user } = open();
  await user.pointer({ keys: "[MouseLeft>]", target: svg, coords: { clientX: 10, clientY: 10 } });
  await fireEvent.pointerCancel(svg, { clientX: 10, clientY: 10 });
  await user.pointer({ target: svg, coords: { clientX: 30, clientY: 30 } });
  expect(onstroke.mock.calls.map(([, phase]) => phase)).toEqual(["start", "end"]);
});
