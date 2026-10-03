import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import ModifyDialog from "../../src/lib/ModifyDialog.svelte";
import type { SelectionModify } from "../../src/lib/engine";

/** Select > Modify of the given kind, last used with `value` pixels, accepting up to `max`. */
function open(kind: SelectionModify = "expand", value = 10, max = 500) {
  const onapply = vi.fn();
  const onclose = vi.fn();
  render(ModifyDialog, { kind, value, max, onapply, onclose });
  return { onapply, onclose, user: userEvent.setup() };
}

const amount = (name: string) => screen.getByRole("spinbutton", { name });
const ok = () => screen.getByRole("button", { name: "OK" });

test("each kind has its own title and label, and starts at the value used last", () => {
  open("feather", 25);
  expect(screen.getByRole("dialog", { name: "Feather Selection" })).toBeInTheDocument();
  expect(amount("Feather Radius:")).toHaveValue(25);
  expect(screen.getByText("pixels")).toBeInTheDocument();
});

test("a number typed is applied by OK or by Enter", async () => {
  const { onapply, user } = open("expand", 10);
  await user.clear(amount("Expand By:"));
  await user.type(amount("Expand By:"), "42");
  await user.click(ok());
  expect(onapply).toHaveBeenLastCalledWith(42);
  await user.type(amount("Expand By:"), "{Enter}");
  expect(onapply).toHaveBeenCalledTimes(2);
});

test("an empty, null or too large amount cannot be applied", async () => {
  const { onapply, user } = open("border", 10, 100);
  const field = amount("Width:");
  await user.clear(field);
  expect(ok()).toBeDisabled();
  await user.type(field, "0");
  expect(ok()).toBeDisabled();
  await user.clear(field);
  await user.type(field, "101");
  expect(ok()).toBeDisabled();
  // Enter in the field does not get around it.
  await user.type(field, "{Enter}");
  expect(onapply).not.toHaveBeenCalled();
  await user.clear(field);
  await user.type(field, "100");
  expect(ok()).toBeEnabled();
});

test("the slider is logarithmic, so that small amounts are as easy to set as large ones", async () => {
  const { onapply, user } = open("smooth", 1, 10000);
  // Half way along a logarithmic slider from 1 to 10000: 100.
  await fireEvent.input(screen.getByRole("slider", { name: "Sample Radius:" }), {
    target: { value: "500" },
  });
  expect(amount("Sample Radius:")).toHaveValue(100);
  await user.click(ok());
  expect(onapply).toHaveBeenCalledWith(100);
});

test("Cancel and Escape close without applying, and the app's keys wait meanwhile", async () => {
  const { onapply, onclose, user } = open();
  const appKeys = vi.fn();
  window.addEventListener("keydown", appKeys);
  await user.keyboard("v");
  window.removeEventListener("keydown", appKeys);
  expect(appKeys).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  screen.getByRole("dialog").dispatchEvent(new Event("cancel", { cancelable: true }));
  expect(onclose).toHaveBeenCalledTimes(2);
  expect(onapply).not.toHaveBeenCalled();
});

test("every valid amount is previewed as it changes, the first one included", async () => {
  const onpreview = vi.fn();
  render(ModifyDialog, {
    kind: "contract",
    value: 10,
    max: 100,
    onapply: vi.fn(),
    onclose: vi.fn(),
    onpreview,
  });
  const user = userEvent.setup();
  expect(onpreview).toHaveBeenLastCalledWith(10);
  const field = amount("Contract By:");
  await user.clear(field);
  await user.type(field, "7");
  expect(onpreview).toHaveBeenLastCalledWith(7);
  // Invalid amounts are not shown.
  await user.type(field, "00");
  expect(onpreview).toHaveBeenLastCalledWith(70);
  expect(onpreview).not.toHaveBeenCalledWith(700);
});
