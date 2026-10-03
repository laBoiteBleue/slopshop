import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import QuickSelectionTool from "../../src/lib/QuickSelectionTool.svelte";
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

function open(options: { mode?: SelectionMode; hand?: boolean; size?: number } = {}) {
  const onstroke = vi.fn();
  const { container } = render(QuickSelectionTool, {
    mapping: { ...MAPPING, hand: options.hand ?? false },
    mode: options.mode ?? "add",
    size: options.size ?? 20,
    busy: false,
    onstroke,
  });
  const svg = container.querySelector("svg") as SVGSVGElement;
  const user = userEvent.setup();
  const at = ([x, y]: Point) => ({ target: svg, coords: { clientX: x, clientY: y } });
  return { onstroke, svg, user, at };
}

/** The phases sent, in order. */
const phases = (fn: ReturnType<typeof vi.fn>) => fn.mock.calls.map((call) => call[3]);

test("a stroke sends a point every half brush while it goes, then all its points at the end", async () => {
  const { onstroke, user, at } = open({ size: 20 });
  await user.pointer([
    { keys: "[MouseLeft>]", ...at([10, 10]) },
    at([14, 10]),
    at([25, 10]),
    at([30, 10]),
    at([40, 10]),
    { keys: "[/MouseLeft]", ...at([40, 10]) },
  ]);
  // The moves 4 px and 5 px from the last point are closer than 10 px (half the brush).
  expect(phases(onstroke)).toEqual(["move", "move", "move", "end"]);
  expect(onstroke.mock.calls[0][0]).toEqual([[10, 10]]);
  expect(onstroke.mock.lastCall?.[0]).toEqual([
    [10, 10],
    [25, 10],
    [40, 10],
  ]);
});

test("a click is a stroke of one point", async () => {
  const { onstroke, user, at } = open();
  await user.pointer({ keys: "[MouseLeft]", ...at([15, 15]) });
  expect(phases(onstroke)).toEqual(["move", "end"]);
  expect(onstroke.mock.lastCall?.[0]).toEqual([[15, 15]]);
});

test.each([
  ["[ShiftLeft>]", "[/ShiftLeft]", "add"],
  ["[AltLeft>]", "[/AltLeft]", "subtract"],
  ["[ShiftLeft>][AltLeft>]", "[/AltLeft][/ShiftLeft]", "intersect"],
])("keys held at the start of the stroke (%s) ask for the mode %s", async (down, up, mode) => {
  const { onstroke, user, at } = open();
  await user.keyboard(down);
  await user.pointer([
    { keys: "[MouseLeft>]", ...at([10, 10]) },
    at([40, 10]),
    { keys: "[/MouseLeft]", ...at([40, 10]) },
  ]);
  await user.keyboard(up);
  expect(onstroke.mock.calls.map((call) => call[1])).toEqual([mode, mode, mode]);
});

test("without keys the stroke asks for no mode of its own", async () => {
  const { onstroke, user, at } = open({ mode: "subtract" });
  await user.pointer({ keys: "[MouseLeft]", ...at([10, 10]) });
  expect(onstroke.mock.calls.map((call) => call[1])).toEqual([null, null]);
});

test("Escape drops the stroke under way, and its release sends nothing more", async () => {
  const { onstroke, user, at } = open();
  await user.pointer([{ keys: "[MouseLeft>]", ...at([10, 10]) }, at([40, 10])]);
  await user.keyboard("{Escape}");
  expect(onstroke.mock.lastCall?.slice(0, 2)).toEqual([[], null]);
  expect(onstroke.mock.lastCall?.[3]).toBe("cancel");
  const calls = onstroke.mock.calls.length;
  await user.pointer({ keys: "[/MouseLeft]", ...at([40, 10]) });
  expect(onstroke).toHaveBeenCalledTimes(calls);
});

test("Escape without a stroke does nothing", async () => {
  const { onstroke, user } = open();
  await user.keyboard("{Escape}");
  expect(onstroke).not.toHaveBeenCalled();
});

test("a badge by the brush tells the mode the next stroke would take", async () => {
  const { user, at } = open({ mode: "add" });
  await user.pointer(at([10, 10]));
  expect(screen.getByText("+")).toBeInTheDocument();
  await user.keyboard("[AltLeft>]");
  await user.pointer(at([11, 11]));
  expect(screen.getByText("−")).toBeInTheDocument();
});

test("while Space pans the viewport, a drag selects nothing", async () => {
  const { onstroke, user, at } = open({ hand: true });
  await user.pointer([
    { keys: "[MouseLeft>]", ...at([10, 10]) },
    at([40, 10]),
    { keys: "[/MouseLeft]", ...at([40, 10]) },
  ]);
  expect(onstroke).not.toHaveBeenCalled();
});

test("a cancelled pointer drops the stroke without ending it", async () => {
  const { onstroke, svg, user, at } = open();
  await user.pointer({ keys: "[MouseLeft>]", ...at([10, 10]) });
  await fireEvent.pointerCancel(svg);
  const calls = onstroke.mock.calls.length;
  await user.pointer({ keys: "[/MouseLeft]", ...at([10, 10]) });
  expect(onstroke).toHaveBeenCalledTimes(calls);
});
