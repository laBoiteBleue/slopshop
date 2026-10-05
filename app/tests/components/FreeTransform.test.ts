import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import FreeTransform from "../../src/lib/FreeTransform.svelte";
import type { Matrix } from "../../src/lib/engine";
import type { ViewMapping } from "../../src/lib/Viewport.svelte";

/** The viewport shows the document at 100%, from the window's corner. */
const MAPPING: ViewMapping = {
  toViewport: (x, y) => [x, y],
  toDocument: (x, y) => [x, y],
  docPerCss: 1,
  hand: false,
};

function open(
  mapping: ViewMapping = MAPPING,
  more: { distortable?: boolean; mode?: "free" | "distort" | "perspective" } = {},
) {
  const onchange = vi.fn();
  const oncommit = vi.fn();
  const oncancel = vi.fn();
  const { container } = render(FreeTransform, {
    mapping,
    box: { left: 0, top: 0, right: 100, bottom: 50 },
    matrix: [1, 0, 0, 1, 0, 0] as Matrix,
    pivot: [50, 25] as [number, number],
    onchange,
    oncommit,
    oncancel,
    ...more,
  });
  const svg = container.querySelector("svg") as SVGSVGElement;
  return {
    onchange,
    oncommit,
    oncancel,
    svg,
    body: container.querySelector(".body") as Element,
    handles: [...container.querySelectorAll(".handle")],
    user: userEvent.setup(),
  };
}

/** The last matrix sent, rounded. */
const last = (fn: ReturnType<typeof vi.fn>) =>
  (fn.mock.lastCall?.[0] as Matrix).map((v) => Math.round(v * 1000) / 1000 + 0);

/** A drag from `from` on `target` to `to`, the moves landing on the svg (captured). */
async function drag(
  user: ReturnType<typeof userEvent.setup>,
  target: Element,
  svg: Element,
  from: [number, number],
  to: [number, number],
) {
  await user.pointer([
    { keys: "[MouseLeft>]", target, coords: { clientX: from[0], clientY: from[1] } },
    { target: svg, coords: { clientX: to[0], clientY: to[1] } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: to[0], clientY: to[1] } },
  ]);
}

test("a drag inside moves, with the move shown by the pointer", async () => {
  const { onchange, svg, body, user } = open();
  await drag(user, body, svg, [50, 25], [70, 35]);
  expect(last(onchange)).toEqual([1, 0, 0, 1, 20, 10]);
});

test("Shift keeps a move along one axis", async () => {
  const { onchange, svg, body, user } = open();
  await user.keyboard("[ShiftLeft>]");
  await drag(user, body, svg, [50, 25], [70, 31]);
  await user.keyboard("[/ShiftLeft]");
  expect(last(onchange)).toEqual([1, 0, 0, 1, 20, 0]);
});

test("a corner handle scales, keeping the proportions, the scale shown while dragging", async () => {
  const { onchange, svg, handles, user } = open();
  await user.pointer([
    { keys: "[MouseLeft>]", target: handles[4], coords: { clientX: 100, clientY: 50 } },
    { target: svg, coords: { clientX: 200, clientY: 100 } },
  ]);
  expect(screen.getByText("W 200.0 % · H 200.0 %")).toBeInTheDocument();
  await user.pointer({ keys: "[/MouseLeft]", target: svg });
  expect(last(onchange)).toEqual([2, 0, 0, 2, 0, 0]);
  expect(screen.queryByText(/^W /)).not.toBeInTheDocument();
});

test("Ctrl and a side handle skew", async () => {
  const { onchange, svg, handles, user } = open();
  await user.keyboard("[ControlLeft>]");
  await drag(user, handles[1], svg, [50, 0], [60, 0]);
  await user.keyboard("[/ControlLeft]");
  // The top slides right by 10, the bottom stays.
  expect(last(onchange)).toEqual([1, 0, -0.2, 1, 10, 0]);
});

test("a drag outside rotates about the pivot; a click outside applies", async () => {
  const { onchange, oncommit, svg, user } = open();
  await drag(user, svg, svg, [150, 25], [50, 125]);
  expect(last(onchange)).toEqual([0, 1, -1, 0, 75, -25]);
  expect(oncommit).not.toHaveBeenCalled();
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: { clientX: 300, clientY: 300 } });
  expect(oncommit).toHaveBeenCalledOnce();
});

test("Enter or a double-click inside applies, Escape cancels", async () => {
  const { oncommit, oncancel, body, user } = open();
  await user.keyboard("{Enter}");
  await user.dblClick(body);
  expect(oncommit).toHaveBeenCalledTimes(2);
  await user.keyboard("{Escape}");
  expect(oncancel).toHaveBeenCalledOnce();
});

test("the right-click menu turns the box about the pivot", async () => {
  const { onchange, svg, user } = open();
  await user.pointer({ keys: "[MouseRight]", target: svg, coords: { clientX: 10, clientY: 10 } });
  await user.click(screen.getByText("Flip Horizontal"));
  expect(last(onchange)).toEqual([-1, 0, 0, 1, 100, 0]);
});

test("at 200%, moves and scales keep the box's edges on whole pixels", async () => {
  const zoomed: ViewMapping = {
    toViewport: (x, y) => [x * 2, y * 2],
    toDocument: (x, y) => [x / 2, y / 2],
    docPerCss: 0.5,
    hand: false,
  };
  const { onchange, svg, body, handles, user } = open(zoomed);
  // 21 × 11 CSS pixels: 10.5 × 5.5 document pixels, landing on 11 × 6.
  await drag(user, body, svg, [100, 50], [121, 61]);
  expect(last(onchange)).toEqual([1, 0, 0, 1, 11, 6]);
  // The right side to x = 120.5 + 11: the right edge lands on a whole pixel.
  await drag(user, handles[3], svg, [222, 62], [263, 62]);
  const [a, , , , e] = last(onchange);
  expect(a * 100 + e).toBe(Math.round(a * 100 + e));
  expect(e).toBe(11);
});

/** Where the last map sent puts point (`x`, `y`): six numbers or nine. */
function placedBy(fn: ReturnType<typeof vi.fn>, x: number, y: number): [number, number] {
  const m = fn.mock.lastCall?.[0] as number[];
  if (m.length === 6) return [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
  const w = m[6] * x + m[7] * y + m[8];
  return [(m[0] * x + m[2] * y + m[4]) / w, (m[1] * x + m[3] * y + m[5]) / w];
}

const near = (p: [number, number], q: [number, number]) => {
  expect(p[0]).toBeCloseTo(q[0], 6);
  expect(p[1]).toBeCloseTo(q[1], 6);
};

test("Ctrl and a corner place it freely: the box goes to that quad (Distort)", async () => {
  const { onchange, svg, handles, user } = open(MAPPING, { distortable: true });
  await user.keyboard("[ControlLeft>]");
  await drag(user, handles[2], svg, [100, 0], [90, 10]);
  await user.keyboard("[/ControlLeft]");
  expect(onchange.mock.lastCall?.[0]).toHaveLength(9);
  near(placedBy(onchange, 100, 0), [90, 10]);
  near(placedBy(onchange, 0, 0), [0, 0]);
  near(placedBy(onchange, 100, 50), [100, 50]);
  // Free now: a plain corner drag distorts too, the box drawn through the corners.
  await drag(user, handles[6], svg, [0, 50], [5, 60]);
  near(placedBy(onchange, 0, 50), [5, 60]);
  near(placedBy(onchange, 100, 0), [90, 10]);
});

test("Alt+Shift+Ctrl and a corner move its pair the other way (Perspective)", async () => {
  const { onchange, svg, handles, user } = open(MAPPING, { distortable: true });
  await user.keyboard("[ControlLeft>][AltLeft>][ShiftLeft>]");
  await drag(user, handles[2], svg, [100, 0], [90, 0]);
  await user.keyboard("[/ShiftLeft][/AltLeft][/ControlLeft]");
  // The top narrowed symmetrically.
  near(placedBy(onchange, 100, 0), [90, 0]);
  near(placedBy(onchange, 0, 0), [10, 0]);
  near(placedBy(onchange, 100, 50), [100, 50]);
});

test("Edit > Transform > Perspective: a corner's plain drag", async () => {
  const { onchange, svg, handles, user } = open(MAPPING, {
    distortable: true,
    mode: "perspective",
  });
  await drag(user, handles[4], svg, [100, 50], [100, 70]);
  near(placedBy(onchange, 100, 50), [100, 70]);
  near(placedBy(onchange, 100, 0), [100, -20]);
});

test("a corner that would fold the box stays where it was", async () => {
  const { onchange, svg, handles, user } = open(MAPPING, { distortable: true, mode: "distort" });
  await drag(user, handles[0], svg, [0, 0], [10, 5]);
  near(placedBy(onchange, 0, 0), [10, 5]);
  // Across the opposite corner: a bow tie, refused.
  await drag(user, handles[0], svg, [10, 5], [150, 80]);
  near(placedBy(onchange, 0, 0), [10, 5]);
});

test("in perspective, a drag inside moves the quad and a side handle its two corners", async () => {
  const { onchange, svg, body, handles, user } = open(MAPPING, {
    distortable: true,
    mode: "distort",
  });
  await drag(user, handles[2], svg, [100, 0], [90, 10]);
  await drag(user, body, svg, [50, 25], [60, 30]);
  near(placedBy(onchange, 100, 0), [100, 15]);
  near(placedBy(onchange, 0, 0), [10, 5]);
  // The bottom side's handle, where it is now (the quad's bottom middle), dragged down.
  await drag(user, handles[5], svg, [60, 55], [60, 65]);
  near(placedBy(onchange, 0, 50), [10, 65]);
  near(placedBy(onchange, 100, 50), [110, 65]);
  near(placedBy(onchange, 0, 0), [10, 5]);
});

test("layers that cannot be put in perspective: Ctrl and a corner still scale", async () => {
  const { onchange, svg, handles, user } = open();
  await user.keyboard("[ControlLeft>]");
  await drag(user, handles[4], svg, [100, 50], [200, 100]);
  await user.keyboard("[/ControlLeft]");
  expect(onchange.mock.lastCall?.[0]).toHaveLength(6);
});
