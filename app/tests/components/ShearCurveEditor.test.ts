import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import ShearCurveEditor from "../../src/lib/ShearCurveEditor.svelte";
import { OFF_GRID, STRAIGHT, type ShearPoint } from "../../src/lib/shearCurve";

/**
 * The editor as Shear's dialog hosts it: each curve it sends comes back as its points. jsdom
 * lays nothing out: the grid is 200 × 100 pixels from the window's corner, so that a pointer at
 * (x, y) is the offset `x - 100` % and the row `y` %.
 */
function open(points: ShearPoint[] = STRAIGHT) {
  const onchange = vi.fn((next: ShearPoint[]) => {
    void view.rerender({ points: next, onchange });
  });
  const view = render(ShearCurveEditor, { points, onchange });
  const grid = screen.getByRole("application", { name: "Shear curve" });
  vi.spyOn(grid, "getBoundingClientRect").mockReturnValue({
    left: 0,
    top: 0,
    width: 200,
    height: 100,
    right: 200,
    bottom: 100,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  });
  return { onchange, grid, user: userEvent.setup() };
}

/** A drag on the grid through `path` (pointer positions). */
async function drag(
  user: ReturnType<typeof userEvent.setup>,
  grid: Element,
  ...path: [number, number][]
) {
  const [first, ...rest] = path;
  const at = ([x, y]: [number, number]) => ({ clientX: x, clientY: y });
  await user.pointer([
    { keys: "[MouseLeft>]", target: grid, coords: at(first) },
    ...rest.map((p) => ({ target: grid, coords: at(p) })),
    { keys: "[/MouseLeft]", target: grid, coords: at(path[path.length - 1]) },
  ]);
}

const last = (fn: ReturnType<typeof vi.fn>) => fn.mock.lastCall?.[0] as ShearPoint[];

test("the straight line's two points, and Default", async () => {
  const bent: ShearPoint[] = [
    [0, 20],
    [50, -30],
    [100, 0],
  ];
  const { onchange, user } = open(bent);
  const points = screen.getAllByRole("slider");
  expect(points).toHaveLength(3);
  expect(points[1]).toHaveAttribute("aria-valuenow", "-30");
  await user.click(screen.getByRole("button", { name: "Default" }));
  expect(last(onchange)).toEqual(STRAIGHT);
  expect(screen.getAllByRole("slider")).toHaveLength(2);
});

test("a press on the grid adds a point there and drags it", async () => {
  const { onchange, grid, user } = open();
  await drag(user, grid, [130, 40], [150, 45]);
  expect(onchange.mock.calls[0][0]).toEqual([
    [0, 0],
    [40, 30],
    [100, 0],
  ]);
  expect(last(onchange)).toEqual([
    [0, 0],
    [45, 50],
    [100, 0],
  ]);
  expect(screen.getAllByRole("slider")).toHaveLength(3);
});

test("the top point moves along its row only", async () => {
  const { onchange, grid, user } = open();
  await drag(user, grid, [100, 0], [60, 30]);
  expect(last(onchange)).toEqual([
    [0, -40],
    [100, 0],
  ]);
});

test("a point dragged off the grid is removed, back on it again", async () => {
  const start: ShearPoint[] = [
    [0, 0],
    [50, 20],
    [100, 0],
  ];
  const { onchange, grid, user } = open(start);
  await user.pointer({ keys: "[MouseLeft>]", target: grid, coords: { clientX: 120, clientY: 50 } });
  await user.pointer({ target: grid, coords: { clientX: 200 + OFF_GRID + 5, clientY: 50 } });
  expect(last(onchange)).toEqual([
    [0, 0],
    [100, 0],
  ]);
  await user.pointer({ target: grid, coords: { clientX: 180, clientY: 50 } });
  expect(last(onchange)).toEqual([
    [0, 0],
    [50, 80],
    [100, 0],
  ]);
  await user.pointer({ keys: "[/MouseLeft]", target: grid });
  // An end stays, held at the grid's edge.
  await drag(user, grid, [100, 100], [-50, 100]);
  expect(last(onchange)).toEqual([
    [0, 0],
    [50, 80],
    [100, -100],
  ]);
});

test("a point focused takes the arrows and Delete", async () => {
  const start: ShearPoint[] = [
    [0, 0],
    [50, 20],
    [100, 0],
  ];
  const { onchange } = open(start);
  const middle = screen.getByRole("slider", { name: "Point 2" });
  await fireEvent.keyDown(middle, { key: "ArrowRight", shiftKey: true });
  expect(last(onchange)[1]).toEqual([50, 30]);
  await fireEvent.keyDown(screen.getByRole("slider", { name: "Point 2" }), { key: "Delete" });
  expect(last(onchange)).toEqual([
    [0, 0],
    [100, 0],
  ]);
});
