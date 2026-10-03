import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import type { TrimSettings } from "../../src/lib/engine";
import TrimDialog from "../../src/lib/TrimDialog.svelte";

const ALL: TrimSettings = {
  basis: "transparent",
  top: true,
  bottom: true,
  left: true,
  right: true,
};

function open(settings: TrimSettings = ALL) {
  const onapply = vi.fn();
  const onclose = vi.fn();
  render(TrimDialog, { settings, onapply, onclose });
  return { onapply, onclose, user: userEvent.setup() };
}

test("what a margin is and the sides it comes off, applied by OK", async () => {
  const { onapply, user } = open();
  await user.click(screen.getByRole("radio", { name: "Top Left Pixel Color" }));
  await user.click(screen.getByRole("checkbox", { name: "Left" }));
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onapply).toHaveBeenCalledWith({ ...ALL, basis: "topLeft", left: false });
});

test("the last settings are shown again", () => {
  open({ ...ALL, basis: "bottomRight", top: false });
  expect(screen.getByRole("radio", { name: "Bottom Right Pixel Color" })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Top" })).not.toBeChecked();
});

test("without a side, OK waits", async () => {
  const { user } = open();
  for (const side of ["Top", "Bottom", "Left", "Right"]) {
    await user.click(screen.getByRole("checkbox", { name: side }));
  }
  expect(screen.getByRole("button", { name: "OK" })).toBeDisabled();
});
