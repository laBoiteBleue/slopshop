import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import MarqueeTool from "../../src/lib/MarqueeTool.svelte";
import type { SelectionMode } from "../../src/lib/engine";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The viewport shows the document at 100%, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x, y],
  toDocument: (x, y) => [x, y],
  docPerCss: 1,
  hand: false,
};

function open(
  options: { kind?: "rectangle" | "ellipse"; mode?: SelectionMode; hand?: boolean } = {},
) {
  const onselect = vi.fn();
  const ondeselect = vi.fn();
  const { container } = render(MarqueeTool, {
    mapping: { ...MAPPING, hand: options.hand ?? false },
    kind: options.kind ?? "rectangle",
    mode: options.mode ?? "replace",
    onselect,
    ondeselect,
  });
  return {
    onselect,
    ondeselect,
    svg: container.querySelector("svg") as SVGSVGElement,
    user: userEvent.setup(),
  };
}

type Point = [number, number];

/** Presses at `from`, moves to `to` and holds there (the button stays down). */
async function press(
  user: ReturnType<typeof userEvent.setup>,
  svg: Element,
  from: Point,
  to: Point,
) {
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: from[0], clientY: from[1] } },
    { target: svg, coords: { clientX: to[0], clientY: to[1] } },
  ]);
}

const release = (user: ReturnType<typeof userEvent.setup>, svg: Element, at: Point) =>
  user.pointer({ keys: "[/MouseLeft]", target: svg, coords: { clientX: at[0], clientY: at[1] } });

async function drag(
  user: ReturnType<typeof userEvent.setup>,
  svg: Element,
  from: Point,
  to: Point,
) {
  await press(user, svg, from, to);
  await release(user, svg, to);
}

test("a drag selects the rectangle between the press and the release, whatever the direction", async () => {
  const { onselect, svg, user } = open();
  await drag(user, svg, [40, 30], [10, 5]);
  expect(onselect).toHaveBeenCalledExactlyOnceWith(
    { kind: "rectangle", left: 10, top: 5, right: 40, bottom: 30 },
    null,
  );
});

test("the Elliptical Marquee sends an ellipse in the same box", async () => {
  const { onselect, svg, user } = open({ kind: "ellipse" });
  await drag(user, svg, [10, 10], [50, 30]);
  expect(onselect).toHaveBeenCalledExactlyOnceWith(
    { kind: "ellipse", left: 10, top: 10, right: 50, bottom: 30 },
    null,
  );
});

test("the box lands on whole document pixels", async () => {
  const { onselect, svg, user } = open();
  await drag(user, svg, [10.4, 10.6], [30.6, 20.4]);
  expect(onselect.mock.lastCall?.[0]).toMatchObject({ left: 10, top: 11, right: 31, bottom: 20 });
});

test("the size is shown while dragging and gone after", async () => {
  const { svg, user } = open();
  await press(user, svg, [10, 10], [40, 30]);
  expect(screen.getByText("30 × 20 px")).toBeInTheDocument();
  await release(user, svg, [40, 30]);
  expect(screen.queryByText(/ px$/)).not.toBeInTheDocument();
});

test.each([
  ["[ShiftLeft>]", "[/ShiftLeft]", "add"],
  ["[AltLeft>]", "[/AltLeft]", "subtract"],
  ["[ShiftLeft>][AltLeft>]", "[/AltLeft][/ShiftLeft]", "intersect"],
])("keys held at the press (%s) ask for the mode %s", async (down, up, mode) => {
  const { onselect, svg, user } = open();
  await user.keyboard(down);
  await drag(user, svg, [10, 10], [40, 30]);
  await user.keyboard(up);
  expect(onselect).toHaveBeenCalledExactlyOnceWith(expect.anything(), mode);
});

test("without keys, the options bar's mode applies, and a badge tells the mode that keys would give", async () => {
  const { svg, user } = open({ mode: "subtract" });
  await user.pointer({ target: svg, coords: { clientX: 20, clientY: 20 } });
  expect(screen.getByText("−")).toBeInTheDocument();
  await user.keyboard("[ShiftLeft>]");
  await user.pointer({ target: svg, coords: { clientX: 21, clientY: 21 } });
  expect(screen.getByText("+")).toBeInTheDocument();
  expect(screen.queryByText("−")).not.toBeInTheDocument();
});

test("Shift pressed after the press makes a square, Alt draws from the center", async () => {
  const { onselect, svg, user } = open();
  await press(user, svg, [50, 50], [70, 60]);
  await user.keyboard("[ShiftLeft>]");
  await user.pointer({ target: svg, coords: { clientX: 70, clientY: 61 } });
  await release(user, svg, [70, 61]);
  await user.keyboard("[/ShiftLeft]");
  expect(onselect.mock.lastCall?.[0]).toMatchObject({ left: 50, top: 50, right: 70, bottom: 70 });
  // The keys did not ask for a mode: they shaped the drag only.
  expect(onselect.mock.lastCall?.[1]).toBeNull();

  await press(user, svg, [50, 50], [70, 60]);
  await user.keyboard("[AltLeft>]");
  await user.pointer({ target: svg, coords: { clientX: 70, clientY: 61 } });
  await release(user, svg, [70, 61]);
  await user.keyboard("[/AltLeft]");
  expect(onselect.mock.lastCall?.[0]).toMatchObject({ left: 30, top: 39, right: 70, bottom: 61 });
  expect(onselect.mock.lastCall?.[1]).toBeNull();
});

test("a click without dragging deselects, but not with a key that asks for a mode", async () => {
  const { onselect, ondeselect, svg, user } = open();
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: { clientX: 20, clientY: 20 } });
  expect(ondeselect).toHaveBeenCalledOnce();

  await user.keyboard("[ShiftLeft>]");
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: { clientX: 20, clientY: 20 } });
  await user.keyboard("[/ShiftLeft]");
  expect(ondeselect).toHaveBeenCalledOnce();
  expect(onselect).not.toHaveBeenCalled();
});

test("a drag with no width or no height selects nothing", async () => {
  const { onselect, ondeselect, svg, user } = open();
  await drag(user, svg, [10, 10], [60, 10]);
  expect(onselect).not.toHaveBeenCalled();
  expect(ondeselect).not.toHaveBeenCalled();
});

test("while Space pans the viewport, a drag selects nothing", async () => {
  const { onselect, ondeselect, svg, user } = open({ hand: true });
  await drag(user, svg, [10, 10], [40, 30]);
  expect(onselect).not.toHaveBeenCalled();
  expect(ondeselect).not.toHaveBeenCalled();
});
