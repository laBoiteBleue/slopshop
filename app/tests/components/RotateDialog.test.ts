import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import RotateDialog from "../../src/lib/RotateDialog.svelte";

function open(angle = 0, clockwise = true) {
  const onapply = vi.fn();
  const onclose = vi.fn();
  render(RotateDialog, { angle, clockwise, onapply, onclose });
  return { onapply, onclose, user: userEvent.setup() };
}

const angle = () => screen.getByRole("spinbutton", { name: "Angle:" });

test("an angle and its direction, applied by OK", async () => {
  const { onapply, user } = open();
  await user.clear(angle());
  await user.type(angle(), "30");
  await user.click(screen.getByRole("radio", { name: "°CCW" }));
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onapply).toHaveBeenCalledWith(30, false);
});

test("the last angle and direction are shown again", () => {
  open(12.5, false);
  expect(angle()).toHaveValue(12.5);
  expect(screen.getByRole("radio", { name: "°CCW" })).toBeChecked();
});

test("the slider sets the angle, its sign the direction", async () => {
  const { onapply, user } = open();
  await fireEvent.input(screen.getByRole("slider", { name: "Angle:" }), {
    target: { value: "-45" },
  });
  expect(angle()).toHaveValue(45);
  expect(screen.getByRole("radio", { name: "°CCW" })).toBeChecked();
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onapply).toHaveBeenCalledWith(45, false);
});

test("a whole turn or more is refused; Cancel closes", async () => {
  const { onapply, onclose, user } = open();
  await user.clear(angle());
  await user.type(angle(), "400");
  expect(screen.getByRole("button", { name: "OK" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onclose).toHaveBeenCalled();
  expect(onapply).not.toHaveBeenCalled();
});
