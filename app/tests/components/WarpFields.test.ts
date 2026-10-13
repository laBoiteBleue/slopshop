import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import WarpFields from "../../src/lib/WarpFields.svelte";
import type { WarpSettings } from "../../src/lib/warp";

function open(settings: WarpSettings) {
  const onchange = vi.fn();
  render(WarpFields, { settings, onchange });
  return { onchange, user: userEvent.setup() };
}

test("a style chosen from None starts at a bend of 50 %", async () => {
  const { onchange, user } = open({ style: null, bend: 10, vertical: false });
  // None: no orientation nor bend to set.
  expect(screen.getByRole("combobox", { name: "Orientation" })).toBeDisabled();
  await user.selectOptions(screen.getByRole("combobox", { name: "Warp" }), "Flag");
  expect(onchange).toHaveBeenLastCalledWith({ style: "flag", bend: 50, vertical: false });
});

test("another style keeps the bend; the orientation and None are sent", async () => {
  const { onchange, user } = open({ style: "arc", bend: -30, vertical: false });
  await user.selectOptions(screen.getByRole("combobox", { name: "Warp" }), "Twist");
  expect(onchange).toHaveBeenLastCalledWith({ style: "twist", bend: -30, vertical: false });
  await user.selectOptions(screen.getByRole("combobox", { name: "Orientation" }), "Vertical");
  expect(onchange).toHaveBeenLastCalledWith({ style: "arc", bend: -30, vertical: true });
  await user.selectOptions(screen.getByRole("combobox", { name: "Warp" }), "None");
  expect(onchange).toHaveBeenLastCalledWith({ style: null, bend: -30, vertical: false });
});

test("the bend typed is sent, within -100 to 100 %", async () => {
  const { onchange } = open({ style: "wave", bend: 50, vertical: false });
  const field = screen.getByRole("spinbutton") as HTMLInputElement;
  expect(field.valueAsNumber).toBe(50);
  await fireEvent.input(field, { target: { value: "75" } });
  expect(onchange).toHaveBeenLastCalledWith({ style: "wave", bend: 75, vertical: false });
});
