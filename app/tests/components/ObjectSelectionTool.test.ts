import { act, fireEvent, render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import ObjectSelectionTool, { type ObjectHover } from "../../src/lib/ObjectSelectionTool.svelte";
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

/** An object found under the pointer: a 2 x 2 mask, all inside, over a 40 x 20 region. */
const OBJECT: ObjectHover = {
  side: 2,
  mask: new Uint8Array([255, 255, 255, 255]),
  region: [10, 20, 40, 20],
};

function open(
  options: {
    mode?: SelectionMode;
    hand?: boolean;
    busy?: boolean;
    onhover?: (...args: unknown[]) => Promise<ObjectHover | null>;
  } = {},
) {
  const onhover = vi.fn(options.onhover ?? (async () => null));
  const onselect = vi.fn();
  const { container } = render(ObjectSelectionTool, {
    mapping: { ...MAPPING, hand: options.hand ?? false },
    mode: options.mode ?? "replace",
    busy: options.busy ?? false,
    onhover,
    onselect,
  });
  const svg = container.querySelector("svg") as SVGSVGElement;
  const user = userEvent.setup();
  const at = ([x, y]: Point) => ({ target: svg, coords: { clientX: x, clientY: y } });
  return { onhover, onselect, container, svg, user, at };
}

afterEach(() => vi.useRealTimers());

test("a click selects the point under the pointer, with no mode of its own", async () => {
  const { onselect, user, at } = open();
  await user.pointer({ keys: "[MouseLeft]", ...at([30, 40]) });
  expect(onselect).toHaveBeenCalledExactlyOnceWith([30, 40], null, null, expect.any(Array));
});

test("a drag selects the box between the press and the release, whatever the direction", async () => {
  const { onselect, user, at } = open();
  await user.pointer([
    { keys: "[MouseLeft>]", ...at([60, 50]) },
    at([20, 10]),
    { keys: "[/MouseLeft]", ...at([20, 10]) },
  ]);
  expect(onselect).toHaveBeenCalledExactlyOnceWith(null, [20, 10, 60, 50], null, expect.any(Array));
});

test.each([
  ["[ShiftLeft>]", "[/ShiftLeft]", "add"],
  ["[AltLeft>]", "[/AltLeft]", "subtract"],
  ["[ShiftLeft>][AltLeft>]", "[/AltLeft][/ShiftLeft]", "intersect"],
])("keys held at the release (%s) ask for the mode %s", async (down, up, mode) => {
  const { onselect, user, at } = open();
  await user.keyboard(down);
  await user.pointer({ keys: "[MouseLeft]", ...at([30, 40]) });
  await user.keyboard(up);
  expect(onselect.mock.lastCall?.[2]).toBe(mode);
});

test("a press that moves less than a few pixels is still a click", async () => {
  const { onselect, user, at } = open();
  await user.pointer([
    { keys: "[MouseLeft>]", ...at([30, 40]) },
    at([32, 41]),
    { keys: "[/MouseLeft]", ...at([32, 41]) },
  ]);
  expect(onselect).toHaveBeenCalledOnce();
  expect(onselect.mock.lastCall?.[0]).toEqual([32, 41]);
  expect(onselect.mock.lastCall?.[1]).toBeNull();
});

test("the object under the pointer is asked for at the pointer's document point, and lit up", async () => {
  const { onhover, container, user, at } = open({ onhover: async () => OBJECT });
  await user.pointer(at([25, 35]));
  expect(onhover).toHaveBeenCalledExactlyOnceWith(25, 35, expect.any(Array));
  await waitFor(() => expect(container.querySelector("canvas.highlight")).toBeInTheDocument());
  expect(container.querySelector("path.edge")).toBeInTheDocument();
});

test("one hover is asked at a time, and the latest position is asked next", async () => {
  let release: (found: ObjectHover | null) => void = () => {};
  const { onhover, user, at } = open({
    onhover: () => new Promise((resolve) => (release = resolve)),
  });
  await user.pointer(at([10, 10]));
  await user.pointer(at([20, 20]));
  await user.pointer(at([30, 30]));
  expect(onhover).toHaveBeenCalledTimes(1);
  await act(() => release(null));
  await waitFor(() => expect(onhover).toHaveBeenCalledTimes(2));
  expect(onhover.mock.lastCall?.slice(0, 2)).toEqual([30, 30]);
});

test("pressing drops the highlight, and a drag shows its box", async () => {
  const { container, user, at } = open({ onhover: async () => OBJECT });
  await user.pointer(at([25, 35]));
  await waitFor(() => expect(container.querySelector("canvas.highlight")).toBeInTheDocument());
  await user.pointer([{ keys: "[MouseLeft>]", ...at([25, 35]) }, at([60, 70])]);
  expect(container.querySelector("canvas.highlight")).not.toBeInTheDocument();
  expect(container.querySelector("rect.box")).toBeInTheDocument();
  await user.pointer({ keys: "[/MouseLeft]", ...at([60, 70]) });
  expect(container.querySelector("rect.box")).not.toBeInTheDocument();
});

test("while a selection is being made, nothing is asked or selected", async () => {
  const { onhover, onselect, user, at } = open({ busy: true });
  await user.pointer(at([25, 35]));
  await user.pointer({ keys: "[MouseLeft]", ...at([30, 40]) });
  expect(onhover).not.toHaveBeenCalled();
  expect(onselect).not.toHaveBeenCalled();
});

test("while Space pans the viewport, nothing is asked or selected", async () => {
  const { onhover, onselect, user, at } = open({ hand: true });
  await user.pointer(at([25, 35]));
  await user.pointer({ keys: "[MouseLeft]", ...at([30, 40]) });
  expect(onhover).not.toHaveBeenCalled();
  expect(onselect).not.toHaveBeenCalled();
});

test("a spinner by the pointer shows when a hover takes a moment", async () => {
  vi.useFakeTimers();
  const { container, svg } = open({ onhover: () => new Promise(() => {}) });
  await fireEvent.pointerMove(svg, { clientX: 25, clientY: 35 });
  expect(container.querySelector(".spinner")).not.toBeInTheDocument();
  await act(() => vi.advanceTimersByTime(200));
  expect(container.querySelector(".spinner")).toBeInTheDocument();
});

test("a badge by the pointer tells the mode the next selection would take", async () => {
  const { user, at } = open({ mode: "add" });
  await user.pointer(at([10, 10]));
  expect(screen.getByText("+")).toBeInTheDocument();
  await user.keyboard("[AltLeft>]");
  await user.pointer(at([11, 11]));
  expect(screen.getByText("−")).toBeInTheDocument();
});
