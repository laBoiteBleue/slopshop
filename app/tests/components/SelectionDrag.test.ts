import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { createRawSnippet } from "svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import SelectionDrag from "../../src/lib/SelectionDrag.svelte";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The viewport shows the document at 100%, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x, y],
  toDocument: (x, y) => [x, y],
  docPerCss: 1,
  hand: false,
};

type Point = [number, number];

/** Where the selection is: the engine says a point is inside it when it is in this box. */
let selected: { left: number; top: number; right: number; bottom: number } | null;
let questions: { documentId: number; x: number; y: number }[];

beforeEach(() => {
  selected = { left: 0, top: 0, right: 100, bottom: 100 };
  questions = [];
  mockIPC((command, args) => {
    if (command !== "selection_bounds_at") return undefined;
    const { x, y } = args as { documentId: number; x: number; y: number };
    questions.push(args as (typeof questions)[number]);
    const inside =
      selected &&
      x >= selected.left &&
      x < selected.right &&
      y >= selected.top &&
      y < selected.bottom;
    return inside ? selected : null;
  });
});

afterEach(() => clearMocks());

function open(options: { enabled?: boolean; hand?: boolean; selectionKey?: number | null } = {}) {
  const onshift = vi.fn();
  const onmove = vi.fn();
  const onclick = vi.fn();
  const toolPress = vi.fn();
  const { container } = render(SelectionDrag, {
    mapping: { ...MAPPING, hand: options.hand ?? false },
    documentId: 3,
    selectionKey: options.selectionKey === undefined ? 1 : options.selectionKey,
    enabled: options.enabled ?? true,
    onshift,
    onmove,
    onclick,
    // The tool underneath: an svg that takes the presses, as the real tools do.
    children: createRawSnippet(() => ({ render: () => '<svg class="tool"></svg>' })),
  });
  const tool = container.querySelector("svg.tool") as SVGSVGElement;
  tool.addEventListener("pointerdown", toolPress);
  const at = ([x, y]: Point) => ({ target: tool, coords: { clientX: x, clientY: y } });
  return {
    onshift,
    onmove,
    onclick,
    toolPress,
    container,
    tool,
    at,
    user: userEvent.setup(),
    /** The pointer rests on `point` until the engine has said whether it is inside. */
    async hover(user: ReturnType<typeof userEvent.setup>, point: Point, over = true) {
      await user.pointer(at(point));
      await waitFor(() => expect(questions.length).toBeGreaterThan(0));
      if (over) {
        await waitFor(() => expect(container.querySelector(".selection-drag")).toHaveClass("over"));
      } else {
        await new Promise((resolve) => setTimeout(resolve, 20));
      }
    },
  };
}

test("hovering asks the engine whether the pixel under the pointer is in the selection, and shows the arrow if so", async () => {
  const { container, user, at } = open();
  await user.pointer(at([20.7, 30.2]));
  await waitFor(() => expect(questions).toEqual([{ documentId: 3, x: 20.5, y: 30.5 }]));
  await waitFor(() => expect(container.querySelector(".selection-drag")).toHaveClass("over"));
  selected = null;
  await user.pointer(at([60, 60]));
  await waitFor(() => expect(container.querySelector(".selection-drag")).not.toHaveClass("over"));
});

test("a press inside the selection drags its outline instead of reaching the tool, and the release moves it", async () => {
  const { onshift, onmove, onclick, toolPress, hover, user, at } = open();
  await hover(user, [20, 20]);
  await user.pointer([
    { keys: "[MouseLeft>]", ...at([20, 20]) },
    at([30, 25]),
    at([35, 28]),
    { keys: "[/MouseLeft]", ...at([35, 28]) },
  ]);
  expect(toolPress).not.toHaveBeenCalled();
  expect(onshift.mock.calls).toEqual([
    [10, 5],
    [15, 8],
  ]);
  expect(onmove).toHaveBeenCalledExactlyOnceWith(15, 8);
  expect(onclick).not.toHaveBeenCalled();
});

test("a click inside the selection, without dragging, is the tool's click at that point", async () => {
  const { onshift, onmove, onclick, hover, user, at } = open();
  await hover(user, [20, 20]);
  await user.pointer({ keys: "[MouseLeft]", ...at([20, 20]) });
  expect(onclick).toHaveBeenCalledExactlyOnceWith(20, 20);
  expect(onshift).not.toHaveBeenCalled();
  expect(onmove).not.toHaveBeenCalled();
});

test("Shift during the drag moves the outline along the nearest multiple of 45 degrees", async () => {
  const { onshift, onmove, hover, user, at } = open();
  await hover(user, [20, 20]);
  await user.pointer([{ keys: "[MouseLeft>]", ...at([20, 20]) }, at([30, 20])]);
  await user.keyboard("[ShiftLeft>]");
  await user.pointer([at([50, 24]), { keys: "[/MouseLeft]", ...at([50, 24]) }]);
  await user.keyboard("[/ShiftLeft]");
  expect(onshift.mock.lastCall).toEqual([30, 0]);
  expect(onmove).toHaveBeenCalledExactlyOnceWith(30, 0);
});

test("Shift or Alt at the press leaves it to the tool", async () => {
  const { onshift, toolPress, hover, user, at } = open();
  await hover(user, [20, 20]);
  await user.keyboard("[ShiftLeft>]");
  await user.pointer({ keys: "[MouseLeft]", ...at([20, 20]) });
  await user.keyboard("[/ShiftLeft]");
  await user.keyboard("[AltLeft>]");
  await user.pointer({ keys: "[MouseLeft]", ...at([20, 20]) });
  await user.keyboard("[/AltLeft]");
  expect(toolPress).toHaveBeenCalledTimes(2);
  expect(onshift).not.toHaveBeenCalled();
});

test("a press outside the selection reaches the tool", async () => {
  selected = { left: 0, top: 0, right: 50, bottom: 50 };
  const { onshift, onclick, toolPress, hover, user, at } = open();
  await hover(user, [80, 80], false);
  await user.pointer([
    { keys: "[MouseLeft>]", ...at([80, 80]) },
    at([90, 90]),
    { keys: "[/MouseLeft]", ...at([90, 90]) },
  ]);
  expect(toolPress).toHaveBeenCalledOnce();
  expect(onshift).not.toHaveBeenCalled();
  expect(onclick).not.toHaveBeenCalled();
});

test("a cancelled pointer ends the drag back where it started", async () => {
  const { onmove, hover, user, at, tool } = open();
  await hover(user, [20, 20]);
  await user.pointer([{ keys: "[MouseLeft>]", ...at([20, 20]) }, at([40, 40])]);
  tool.dispatchEvent(new PointerEvent("pointercancel", { bubbles: true, pointerId: 1 }));
  expect(onmove).toHaveBeenCalledExactlyOnceWith(0, 0);
});

test.each([
  ["the tool or mode does not allow it", { enabled: false }],
  ["there is no selection", { selectionKey: null }],
  ["Space pans the viewport", { hand: true }],
])("when %s, nothing is asked and the tool gets the press", async (_, options) => {
  const { onshift, toolPress, container, user, at } = open(options);
  await user.pointer([
    { keys: "[MouseLeft>]", ...at([20, 20]) },
    at([30, 30]),
    { keys: "[/MouseLeft]", ...at([30, 30]) },
  ]);
  expect(questions).toEqual([]);
  expect(toolPress).toHaveBeenCalledOnce();
  expect(onshift).not.toHaveBeenCalled();
  expect(container.querySelector(".selection-drag")).not.toHaveClass("over");
});
