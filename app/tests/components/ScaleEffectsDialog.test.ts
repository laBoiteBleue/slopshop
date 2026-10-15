import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import ScaleEffectsDialog from "../../src/lib/ScaleEffectsDialog.svelte";

function open() {
  const onchange = vi.fn();
  const onok = vi.fn();
  const oncancel = vi.fn();
  render(ScaleEffectsDialog, { onchange, onok, oncancel });
  return { onchange, onok, oncancel, user: userEvent.setup() };
}

const field = () => screen.getByRole("spinbutton", { name: "Scale:" });

test("it opens at 100 %; each scale typed or slid is sent, OK keeps it", async () => {
  const { onchange, onok, user } = open();
  expect(screen.getByRole("dialog", { name: "Scale Layer Effects" })).toBeInTheDocument();
  expect(field()).toHaveValue(100);
  await user.clear(field());
  await user.type(field(), "250");
  expect(onchange).toHaveBeenLastCalledWith(250);
  await fireEvent.input(screen.getByRole("slider", { name: "Scale:" }), {
    target: { value: "40" },
  });
  expect(onchange).toHaveBeenLastCalledWith(40);
  expect(field()).toHaveValue(40);
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onok).toHaveBeenCalledOnce();
});

test("a scale out of 1 to 1000 % is not sent nor applied; Cancel and Esc take it back", async () => {
  const { onchange, onok, oncancel, user } = open();
  await user.clear(field());
  await user.type(field(), "2000");
  expect(onchange).not.toHaveBeenCalledWith(2000);
  expect(screen.getByRole("button", { name: "OK" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(oncancel).toHaveBeenCalledOnce();
  await fireEvent(screen.getByRole("dialog"), new Event("cancel"));
  expect(oncancel).toHaveBeenCalledTimes(2);
  expect(onok).not.toHaveBeenCalled();
});
