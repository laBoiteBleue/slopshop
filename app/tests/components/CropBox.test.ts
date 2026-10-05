import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import CropBox from "../../src/lib/CropBox.svelte";
import type { CropAspect } from "../../src/lib/crop";
import type { Bounds } from "../../src/lib/engine";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The viewport shows the document at 100%, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x, y],
  toDocument: (x, y) => [x, y],
  docPerCss: 1,
  hand: false,
};

/** A 200 x 100 canvas, where the frame starts. */
const CANVAS: Bounds = { left: 0, top: 0, right: 200, bottom: 100 };

type Point = [number, number];

function open(
  options: { targets?: Bounds[]; hand?: boolean; smartGuides?: boolean; aspect?: CropAspect } = {},
) {
  const onapply = vi.fn();
  const oncancel = vi.fn();
  const { container, rerender } = render(CropBox, {
    mapping: { ...MAPPING, hand: options.hand ?? false },
    canvas: CANVAS,
    targets: options.targets ?? [],
    smartGuides: options.smartGuides ?? true,
    aspect: options.aspect ?? { mode: "free" },
    onapply,
    oncancel,
  });
  const svg = container.querySelector("svg") as SVGSVGElement;
  return {
    onapply,
    oncancel,
    container,
    rerender,
    svg,
    frame: container.querySelector(".frame") as Element,
    handles: [...container.querySelectorAll(".handle")],
    user: userEvent.setup(),
  };
}

type User = ReturnType<typeof userEvent.setup>;

/** Presses on `target` at `from` and moves to `to` on the svg, which has captured the pointer. */
async function press(user: User, target: Element, svg: Element, from: Point, to: Point) {
  await user.pointer([
    { keys: "[MouseLeft>]", target, coords: { clientX: from[0], clientY: from[1] } },
    { target: svg, coords: { clientX: to[0], clientY: to[1] } },
  ]);
}

const release = (user: User, svg: Element, at: Point) =>
  user.pointer({ keys: "[/MouseLeft]", target: svg, coords: { clientX: at[0], clientY: at[1] } });

async function drag(user: User, target: Element, svg: Element, from: Point, to: Point) {
  await press(user, target, svg, from, to);
  await release(user, svg, to);
}

// The handles run clockwise from the top-left corner: 0 top-left, 1 top, 2 top-right, 3 right,
// 4 bottom-right, 5 bottom, 6 bottom-left, 7 left.
const RIGHT = 3;
const BOTTOM_RIGHT = 4;

test("the frame starts on the whole canvas and Enter applies it", async () => {
  const { onapply, user } = open();
  await user.keyboard("{Enter}");
  expect(onapply).toHaveBeenCalledExactlyOnceWith(CANVAS);
});

test("Escape cancels", async () => {
  const { onapply, oncancel, user } = open();
  await user.keyboard("{Escape}");
  expect(oncancel).toHaveBeenCalledOnce();
  expect(onapply).not.toHaveBeenCalled();
});

test("dragging a side handle moves that edge, on whole document pixels", async () => {
  const { onapply, svg, handles, user } = open();
  await drag(user, handles[RIGHT], svg, [200, 50], [150.4, 70]);
  await user.keyboard("{Enter}");
  expect(onapply).toHaveBeenCalledExactlyOnceWith({ left: 0, top: 0, right: 150, bottom: 100 });
});

test("Shift on a corner keeps the proportions the frame had", async () => {
  const { onapply, svg, handles, user } = open();
  await user.keyboard("[ShiftLeft>]");
  await drag(user, handles[BOTTOM_RIGHT], svg, [200, 100], [120, 40]);
  await user.keyboard("[/ShiftLeft]");
  await user.keyboard("{Enter}");
  // The frame was 2:1.
  expect(onapply).toHaveBeenCalledExactlyOnceWith({ left: 0, top: 0, right: 120, bottom: 60 });
});

test("a drag inside moves the frame without resizing it", async () => {
  const { onapply, svg, handles, frame, user } = open();
  await drag(user, handles[BOTTOM_RIGHT], svg, [200, 100], [100, 50]);
  await drag(user, frame, svg, [50, 25], [80, 45]);
  await user.keyboard("{Enter}");
  expect(onapply).toHaveBeenCalledExactlyOnceWith({ left: 30, top: 20, right: 130, bottom: 70 });
});

test("a drag outside draws a new frame from where it began", async () => {
  const { onapply, svg, handles, user } = open();
  await drag(user, handles[BOTTOM_RIGHT], svg, [200, 100], [50, 50]);
  await drag(user, svg, svg, [160, 90], [100, 60]);
  expect(onapply).not.toHaveBeenCalled();
  await user.keyboard("{Enter}");
  expect(onapply).toHaveBeenCalledExactlyOnceWith({ left: 100, top: 60, right: 160, bottom: 90 });
});

test("a click outside the frame, without dragging, applies it", async () => {
  const { onapply, svg, handles, user } = open();
  await drag(user, handles[BOTTOM_RIGHT], svg, [200, 100], [50, 50]);
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: { clientX: 180, clientY: 90 } });
  expect(onapply).toHaveBeenCalledExactlyOnceWith({ left: 0, top: 0, right: 50, bottom: 50 });
});

test("a double-click inside the frame applies it", async () => {
  const { onapply, frame, user } = open();
  await user.pointer({
    keys: "[MouseLeft][MouseLeft]",
    target: frame,
    coords: { clientX: 50, clientY: 50 },
  });
  expect(onapply).toHaveBeenCalledExactlyOnceWith(CANVAS);
});

test("the size of the frame is shown while dragging", async () => {
  const { svg, handles, user } = open();
  await press(user, handles[BOTTOM_RIGHT], svg, [200, 100], [120, 60]);
  expect(screen.getByText("120 × 60 px")).toBeInTheDocument();
  await release(user, svg, [120, 60]);
  expect(screen.queryByText(/ px$/)).not.toBeInTheDocument();
});

test("an edge snaps to what it is near, with a guide, unless Ctrl is held", async () => {
  const targets: Bounds[] = [{ left: 0, top: 0, right: 100, bottom: 100 }];
  const { container, onapply, svg, handles, user } = open({ targets });
  await press(user, handles[RIGHT], svg, [200, 50], [97, 50]);
  expect(container.querySelector(".guide")).toBeInTheDocument();
  await release(user, svg, [97, 50]);
  expect(container.querySelector(".guide")).not.toBeInTheDocument();
  await user.keyboard("{Enter}");
  expect(onapply.mock.lastCall?.[0].right).toBe(100);

  await user.keyboard("[ControlLeft>]");
  await drag(user, handles[RIGHT], svg, [100, 50], [97, 50]);
  await user.keyboard("[/ControlLeft]");
  await user.keyboard("{Enter}");
  expect(onapply.mock.lastCall?.[0].right).toBe(97);
});

test("while Space pans the viewport, a drag changes nothing", async () => {
  const { onapply, svg, handles, user } = open({ hand: true });
  await drag(user, handles[RIGHT], svg, [200, 50], [150, 50]);
  await user.keyboard("{Enter}");
  expect(onapply).toHaveBeenCalledExactlyOnceWith(CANVAS);
});

test("with the smart guides hidden (View > Hide Extras), an edge still snaps, unseen", async () => {
  const targets: Bounds[] = [{ left: 0, top: 0, right: 100, bottom: 100 }];
  const { container, onapply, svg, handles, user } = open({ targets, smartGuides: false });
  await press(user, handles[RIGHT], svg, [200, 50], [97, 50]);
  expect(container.querySelector(".guide")).not.toBeInTheDocument();
  await release(user, svg, [97, 50]);
  await user.keyboard("{Enter}");
  expect(onapply.mock.lastCall?.[0].right).toBe(100);
});

test("a ratio fits the frame to it, centered, and every handle keeps it", async () => {
  const { onapply, svg, handles, user, rerender } = open({
    aspect: { mode: "ratio", width: 1, height: 1 },
  });
  await user.keyboard("{Enter}");
  expect(onapply).toHaveBeenLastCalledWith({ left: 50, top: 0, right: 150, bottom: 100 });
  // A side: the other side follows, centered.
  await drag(user, handles[RIGHT], svg, [150, 50], [130, 50]);
  await user.keyboard("{Enter}");
  expect(onapply).toHaveBeenLastCalledWith({ left: 50, top: 10, right: 130, bottom: 90 });
  // Another ratio chosen: fitted again in the frame.
  await rerender({ aspect: { mode: "ratio", width: 2, height: 1 } });
  await user.keyboard("{Enter}");
  expect(onapply).toHaveBeenLastCalledWith({ left: 50, top: 30, right: 130, bottom: 70 });
});

test("a size fixes the frame: no handles, a press outside puts it there to move it", async () => {
  const { onapply, svg, container, user } = open({
    aspect: { mode: "size", width: 60, height: 40 },
  });
  expect(container.querySelectorAll(".handle")).toHaveLength(0);
  await user.keyboard("{Enter}");
  expect(onapply).toHaveBeenLastCalledWith({ left: 70, top: 30, right: 130, bottom: 70 });
  await drag(user, svg, svg, [40, 30], [50, 40]);
  await user.keyboard("{Enter}");
  expect(onapply).toHaveBeenLastCalledWith({ left: 20, top: 20, right: 80, bottom: 60 });
});

test("with Straighten, a drag draws a line and the image turns to level it", async () => {
  const onstraighten = vi.fn();
  const onapply = vi.fn();
  const { container } = render(CropBox, {
    mapping: MAPPING,
    canvas: CANVAS,
    straighten: true,
    onstraighten,
    onapply,
    oncancel: vi.fn(),
  });
  const svg = container.querySelector("svg") as SVGSVGElement;
  const user = userEvent.setup();
  await press(user, svg, svg, [10, 10], [110, 20]);
  expect(container.querySelector("line.level")).not.toBeNull();
  await release(user, svg, [110, 20]);
  expect(container.querySelector("line.level")).toBeNull();
  expect(onstraighten).toHaveBeenCalledOnce();
  expect(onstraighten.mock.calls[0][0]).toBeCloseTo((-Math.atan2(10, 100) * 180) / Math.PI);
  expect(onapply).not.toHaveBeenCalled();
});

test("the frame can start inside the canvas", async () => {
  const onapply = vi.fn();
  render(CropBox, {
    mapping: MAPPING,
    canvas: CANVAS,
    start: { left: 10, top: 5, right: 190, bottom: 95 },
    onapply,
    oncancel: vi.fn(),
  });
  await userEvent.setup().keyboard("{Enter}");
  expect(onapply).toHaveBeenCalledWith({ left: 10, top: 5, right: 190, bottom: 95 });
});
