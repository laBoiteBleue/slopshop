import { render } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import GradientTool from "../../src/lib/GradientTool.svelte";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The document at 100%, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x, y],
  toDocument: (x, y) => [x, y],
  docPerCss: 1,
  hand: false,
};

function open(hand = false) {
  const ongradient = vi.fn();
  const { container } = render(GradientTool, { mapping: { ...MAPPING, hand }, ongradient });
  const svg = container.querySelector("svg") as SVGSVGElement;
  return { ongradient, container, svg, user: userEvent.setup() };
}

test("a drag draws the line, and the release lays the gradient along it", async () => {
  const { ongradient, container, svg, user } = open();
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 10, clientY: 20 } },
    { target: svg, coords: { clientX: 110, clientY: 30 } },
  ]);
  expect(container.querySelectorAll("circle.end")).toHaveLength(2);
  await user.pointer({ keys: "[/MouseLeft]", target: svg, coords: { clientX: 110, clientY: 30 } });
  expect(ongradient).toHaveBeenCalledWith([10, 20], [110, 30]);
  expect(container.querySelector("line")).toBeNull();
});

test("Shift keeps the line at 45°; a click or Escape lays nothing", async () => {
  const { ongradient, svg, user } = open();
  await user.keyboard("[ShiftLeft>]");
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 0, clientY: 0 } },
    { target: svg, coords: { clientX: 100, clientY: 8 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 100, clientY: 8 } },
  ]);
  await user.keyboard("[/ShiftLeft]");
  const [from, to] = ongradient.mock.calls[0];
  expect(from).toEqual([0, 0]);
  expect(to[1]).toBeCloseTo(0);
  ongradient.mockClear();
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 5, clientY: 5 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 5, clientY: 5 } },
  ]);
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 5, clientY: 5 } },
    { target: svg, coords: { clientX: 60, clientY: 5 } },
  ]);
  await user.keyboard("{Escape}");
  await user.pointer({ keys: "[/MouseLeft]", target: svg, coords: { clientX: 60, clientY: 5 } });
  expect(ongradient).not.toHaveBeenCalled();
});
