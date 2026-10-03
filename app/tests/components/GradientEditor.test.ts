import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import GradientEditor from "../../src/lib/GradientEditor.svelte";

const RED_TO_BLUE = [
  [0, 255, 0, 0],
  [4096, 0, 0, 255],
];

function open(stops = RED_TO_BLUE) {
  const onlive = vi.fn();
  const onend = vi.fn();
  const onapply = vi.fn();
  render(GradientEditor, { stops, onlive, onend, onapply });
  return { onlive, onend, onapply, user: userEvent.setup() };
}

const stop = (n: number) => screen.getByRole("button", { name: new RegExp(`stop ${n} `) });

test("a selected stop's color and location are edited, one change each", async () => {
  const { onapply, user } = open();
  expect(screen.getByLabelText("Location (%)")).toBeDisabled();
  await user.click(stop(2));
  const location = screen.getByLabelText("Location (%)");
  expect(location).toHaveValue(100);
  await user.clear(location);
  await user.type(location, "60");
  await user.tab();
  expect(onapply).toHaveBeenLastCalledWith([
    [0, 255, 0, 0],
    [2458, 0, 0, 255],
  ]);
  await fireEvent.change(screen.getByLabelText("Color"), { target: { value: "#00ff00" } });
  expect(onapply).toHaveBeenLastCalledWith([
    [0, 255, 0, 0],
    [2458, 0, 255, 0],
  ]);
});

test("a click under the gradient adds a stop of its color there", async () => {
  const { onapply } = open();
  const track = document.querySelector(".track") as HTMLElement;
  // jsdom lays nothing out: a 100-pixel track at the origin.
  track.getBoundingClientRect = () => ({ left: 0, width: 100, top: 0, bottom: 16 }) as DOMRect;
  await fireEvent.pointerDown(track, { button: 0, clientX: 50 });
  expect(onapply).toHaveBeenLastCalledWith([
    [0, 255, 0, 0],
    [2048, 128, 0, 128],
    [4096, 0, 0, 255],
  ]);
});

test("Delete removes the selected stop, never below two", async () => {
  const { onapply, user } = open([
    [0, 0, 0, 0],
    [2000, 9, 9, 9],
    [4096, 255, 255, 255],
  ]);
  await user.click(stop(2));
  await user.click(screen.getByRole("button", { name: "Delete" }));
  expect(onapply).toHaveBeenLastCalledWith([
    [0, 0, 0, 0],
    [4096, 255, 255, 255],
  ]);
  document.body.innerHTML = "";
  open();
  await user.click(stop(1));
  expect(screen.getByRole("button", { name: "Delete" })).toBeDisabled();
});
