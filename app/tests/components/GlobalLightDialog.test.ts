import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import GlobalLightDialog from "../../src/lib/GlobalLightDialog.svelte";

function open(light: [number, number] = [120, 30]) {
  const onchange = vi.fn();
  const onok = vi.fn();
  const oncancel = vi.fn();
  render(GlobalLightDialog, { light, onchange, onok, oncancel });
  return { onchange, onok, oncancel, user: userEvent.setup() };
}

const angle = () => screen.getByRole("spinbutton", { name: "Angle:" });
const altitude = () => screen.getByRole("spinbutton", { name: "Altitude:" });

test("it shows the document's light; each angle or altitude is sent, OK keeps it", async () => {
  const { onchange, onok, user } = open([90, 45]);
  expect(screen.getByRole("dialog", { name: "Global Light" })).toBeInTheDocument();
  expect(angle()).toHaveValue(90);
  expect(altitude()).toHaveValue(45);
  await user.clear(angle());
  await user.type(angle(), "30");
  expect(onchange).toHaveBeenLastCalledWith([30, 45]);
  await fireEvent.input(screen.getByRole("slider", { name: "Angle:" }), {
    target: { value: "-60" },
  });
  expect(onchange).toHaveBeenLastCalledWith([-60, 45]);
  await fireEvent.input(screen.getByRole("slider", { name: "Altitude:" }), {
    target: { value: "70" },
  });
  expect(onchange).toHaveBeenLastCalledWith([-60, 70]);
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onok).toHaveBeenCalledOnce();
});

test("an altitude above 90 or an angle beyond 180 is not sent; Cancel takes it back", async () => {
  const { onchange, onok, oncancel, user } = open();
  await user.clear(altitude());
  await user.type(altitude(), "95");
  expect(onchange).not.toHaveBeenCalledWith([120, 95]);
  expect(screen.getByRole("button", { name: "OK" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(oncancel).toHaveBeenCalledOnce();
  expect(onok).not.toHaveBeenCalled();
});
