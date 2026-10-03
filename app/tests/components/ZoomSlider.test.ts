import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import ZoomSlider from "../../src/lib/ZoomSlider.svelte";

function open(zoom: number | null = 1) {
  const onzoom = vi.fn(() => Promise.resolve());
  const onstep = vi.fn();
  const onfit = vi.fn();
  const props = { zoom, hint: "Wheel: zoom", onzoom, onstep, onfit };
  const view = render(ZoomSlider, props);
  return { ...view, props, onzoom, onstep, onfit, user: userEvent.setup() };
}

const slider = () => screen.getByRole("slider") as HTMLInputElement;

/** The slider moved to the logarithmic position `value` (log2 of the zoom). */
const slide = (value: number) => fireEvent.input(slider(), { target: { value: String(value) } });

test("the zoom is shown as a percentage, and the slider is on its logarithm", () => {
  open(2);
  expect(screen.getByText(/200/)).toBeInTheDocument();
  expect(slider().valueAsNumber).toBeCloseTo(1);
  expect(slider()).toHaveAttribute("aria-valuetext", expect.stringContaining("200"));
});

test("before the first frame, nothing is shown and everything is disabled", () => {
  open(null);
  expect(slider()).toBeDisabled();
  expect(screen.getByRole("button", { name: "100%" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Fit on Screen" })).toBeDisabled();
});

test("the tooltip of the percentage is the hint", () => {
  open(1);
  expect(screen.getByTitle("Wheel: zoom")).toHaveTextContent(/100/);
});

test("sliding asks for the zoom of the slider's position, and the slider keeps the user's value", async () => {
  const { onzoom } = open(1);
  await slide(3);
  expect(onzoom).toHaveBeenLastCalledWith(8);
  // The view has not followed (zoom is still 1): the slider and the readout show the user's value.
  expect(slider().valueAsNumber).toBeCloseTo(3);
  expect(screen.getByText(/800/)).toBeInTheDocument();
});

test("the slider follows the view again once the last request is answered", async () => {
  const { onzoom } = open(1);
  await slide(3);
  await fireEvent.change(slider());
  await vi.waitFor(() => expect(slider().valueAsNumber).toBeCloseTo(0));
  expect(onzoom).toHaveBeenCalledOnce();
});

test("the slider keeps the user's value while the pointer is held", async () => {
  open(1);
  await fireEvent.pointerDown(slider());
  await slide(2);
  await fireEvent.change(slider());
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(slider().valueAsNumber).toBeCloseTo(2);
  await fireEvent.pointerUp(window);
  await vi.waitFor(() => expect(slider().valueAsNumber).toBeCloseTo(0));
});

test("the arrow and page keys step through the zoom presets instead of nudging the slider", async () => {
  const { user, onstep, onzoom } = open(1);
  slider().focus();
  await user.keyboard("{ArrowRight}{PageUp}{ArrowUp}");
  expect(onstep.mock.calls).toEqual([[true], [true], [true]]);
  await user.keyboard("{ArrowLeft}{PageDown}{ArrowDown}");
  expect(onstep.mock.calls.slice(3)).toEqual([[false], [false], [false]]);
  expect(onzoom).not.toHaveBeenCalled();
});

test("the 100% button zooms to 1 and the fit button shows the whole image", async () => {
  const { user, onzoom, onfit } = open(0.25);
  await user.click(screen.getByRole("button", { name: "100%" }));
  expect(onzoom).toHaveBeenCalledExactlyOnceWith(1);
  await user.click(screen.getByRole("button", { name: "Fit on Screen" }));
  expect(onfit).toHaveBeenCalledOnce();
});

test("the slider spans the engine's zoom range", () => {
  open(1);
  expect(2 ** Number(slider().min)).toBeCloseTo(0.001);
  expect(2 ** Number(slider().max)).toBeCloseTo(64);
});
