import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test } from "vitest";
import SliderField from "../../src/lib/SliderField.svelte";

/** The label (a scrubby slider); the dropped-down slider comes after it. */
const label = () => screen.getAllByRole("slider")[0];
const number = () => screen.getByRole("spinbutton");

test("a value typed is kept within the range and on the steps", async () => {
  render(SliderField, { label: "Tolerance:", value: 32, min: 0, max: 255 });
  const user = userEvent.setup();
  expect(number()).toHaveValue(32);
  await user.clear(number());
  await user.type(number(), "400");
  expect(label()).toHaveAttribute("aria-valuenow", "255");
  await user.clear(number());
  await user.type(number(), "12.6");
  expect(label()).toHaveAttribute("aria-valuenow", "13");
});

test("a share is shown in percent", async () => {
  render(SliderField, { label: "Opacity:", value: 0.25, min: 0, max: 100, factor: 100 });
  expect(number()).toHaveValue(25);
});

test("dragging the label sideways changes the value, ten times faster with Shift", async () => {
  render(SliderField, { label: "Tolerance:", value: 32, min: 0, max: 255 });
  const user = userEvent.setup();
  await user.pointer([
    { keys: "[MouseLeft>]", target: label(), coords: { clientX: 100 } },
    { target: label(), coords: { clientX: 110 } },
    { keys: "[/MouseLeft]", target: label() },
  ]);
  expect(label()).toHaveAttribute("aria-valuenow", "42");
  await user.keyboard("[ShiftLeft>]");
  await user.pointer([
    { keys: "[MouseLeft>]", target: label(), coords: { clientX: 100 } },
    { target: label(), coords: { clientX: 98 } },
    { keys: "[/MouseLeft]", target: label() },
  ]);
  await user.keyboard("[/ShiftLeft]");
  expect(label()).toHaveAttribute("aria-valuenow", "22");
});

test("the arrow drops a slider, logarithmic for sizes, closed by Escape", async () => {
  render(SliderField, { label: "Size:", value: 10, min: 1, max: 10000, log: true });
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Size:" }));
  const slider = document.querySelector(".popup input") as HTMLInputElement;
  // Half way along a logarithmic slider from 1 to 10000: 100.
  await fireEvent.input(slider, { target: { value: "500" } });
  expect(label()).toHaveAttribute("aria-valuenow", "100");
  await user.keyboard("{Escape}");
  expect(document.querySelector(".popup")).toBeNull();
});
