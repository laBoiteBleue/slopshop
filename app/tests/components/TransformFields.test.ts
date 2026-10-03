import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import type { Matrix } from "../../src/lib/engine";
import TransformFields from "../../src/lib/TransformFields.svelte";

const IDENTITY: Matrix = [1, 0, 0, 1, 0, 0];

function open(matrix: Matrix = IDENTITY, pivot: [number, number] = [0, 0]) {
  const onchange = vi.fn();
  const props = { matrix, pivot, canvas: { width: 200, height: 100 }, onchange };
  const view = render(TransformFields, props);
  return { ...view, props, onchange };
}

/** The fields in the options bar's order: X, Y, W, H, angle, skew. */
const fields = () => screen.getAllByRole("spinbutton") as HTMLInputElement[];
const [X, Y, W, H, ANGLE, SKEW] = [0, 1, 2, 3, 4, 5];

/** Types `value` in the field (one input event, as a paste). */
const enter = (index: number, value: number) =>
  fireEvent.input(fields()[index], { target: { value: String(value) } });

/** The last matrix sent. */
const sent = (onchange: ReturnType<typeof vi.fn>) => onchange.mock.lastCall?.[0] as Matrix;

test("the fields show the transform: position, sizes in percent, angle and skew in degrees", () => {
  // Scaled by 2 horizontally and 0.5 vertically, moved by (30, 40), the reference at the origin.
  open([2, 0, 0, 0.5, 30, 40]);
  expect(fields().map((field) => field.valueAsNumber)).toEqual([30, 40, 200, 50, 0, 0]);
});

test("the angle is shown in degrees", () => {
  // A quarter turn clockwise on screen.
  open([0, 1, -1, 0, 0, 0]);
  expect(fields()[ANGLE].valueAsNumber).toBeCloseTo(90);
});

test("a position typed moves the box and nothing else", async () => {
  const { onchange } = open();
  await enter(X, 25);
  expect(sent(onchange)).toEqual([1, 0, 0, 1, 25, 0]);
  await enter(Y, -10);
  expect(sent(onchange)).toEqual([1, 0, 0, 1, 0, -10]);
});

test("the width and the height change together while they are linked", async () => {
  const { onchange } = open();
  await enter(W, 200);
  expect(sent(onchange)).toEqual([2, 0, 0, 2, 0, 0]);
  await enter(H, 50);
  expect(sent(onchange)).toEqual([0.5, 0, 0, 0.5, 0, 0]);
});

test("the link button frees the width from the height, and links them again", async () => {
  const { onchange } = open();
  const user = userEvent.setup();
  const link = screen.getByRole("button", { name: "Link width and height" });
  expect(link).toHaveAttribute("aria-pressed", "true");
  await user.click(link);
  expect(link).toHaveAttribute("aria-pressed", "false");
  await enter(W, 200);
  expect(sent(onchange)).toEqual([2, 0, 0, 1, 0, 0]);
  await user.click(link);
  await enter(W, 300);
  expect(sent(onchange)).toEqual([3, 0, 0, 3, 0, 0]);
});

test("a flip is a negative scale", async () => {
  const { onchange } = open();
  await enter(W, -100);
  const [a, , , d] = sent(onchange);
  expect(a).toBe(-1);
  expect(d).toBe(-1);
});

test("a scale of zero is refused: it could not be undone", async () => {
  const { onchange } = open();
  await enter(W, 0);
  expect(onchange).not.toHaveBeenCalled();
});

test("values out of range are kept in range", async () => {
  const { onchange } = open();
  await enter(SKEW, 120);
  // 85 degrees at most, the limit before the box would flatten.
  const [, , c, d] = sent(onchange);
  expect(Math.atan(c / d)).toBeCloseTo((85 * Math.PI) / 180);
  await enter(W, 5000);
  expect(sent(onchange)[0]).toBeCloseTo(10);
});

test("an empty field changes nothing, and leaving it shows the transform again", async () => {
  const { onchange } = open();
  const user = userEvent.setup();
  await user.clear(fields()[X]);
  expect(onchange).not.toHaveBeenCalled();
  await user.tab();
  expect(fields()[X].valueAsNumber).toBe(0);
});

test("the fields follow the transform when it changes elsewhere", async () => {
  const { rerender } = open();
  expect(fields()[X].valueAsNumber).toBe(0);
  await rerender({
    matrix: [1, 0, 0, 1, 12, 34],
    pivot: [0, 0],
    canvas: { width: 200, height: 100 },
  });
  expect([fields()[X].valueAsNumber, fields()[Y].valueAsNumber]).toEqual([12, 34]);
});
