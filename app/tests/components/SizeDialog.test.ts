import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import SizeDialog from "../../src/lib/SizeDialog.svelte";

/** Image Size (by default) of a 400 × 200 image at 72 ppi. */
function open(mode: "image" | "canvas" = "image") {
  const onapply = vi.fn();
  const onclose = vi.fn();
  render(SizeDialog, { mode, width: 400, height: 200, resolution: 72, onapply, onclose });
  return { onapply, onclose, user: userEvent.setup() };
}

const field = (name: string) => screen.getByRole("spinbutton", { name });

test("a width typed resizes the height with it, and OK applies", async () => {
  const { onapply, user } = open();
  await user.clear(field("Width"));
  await user.type(field("Width"), "200");
  expect(field("Height")).toHaveValue(100);
  expect(screen.getByText("New size: 200 × 100 px")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onapply).toHaveBeenCalledWith(200, 100, [0.5, 0.5], 72);
});

test("the open chain frees the proportions", async () => {
  const { onapply, user } = open();
  await user.click(screen.getByRole("button", { name: "Constrain proportions" }));
  await user.clear(field("Height"));
  await user.type(field("Height"), "50");
  expect(field("Width")).toHaveValue(400);
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onapply).toHaveBeenCalledWith(400, 50, [0.5, 0.5], 72);
});

test("without Resample, a printed size sets the resolution and the pixels stay", async () => {
  const { onapply, user } = open();
  await user.click(screen.getByRole("checkbox", { name: "Resample" }));
  // Pixels cannot change any more: the sizes show in centimeters.
  expect(screen.getByRole("combobox", { name: "Unit" })).toHaveValue("cm");
  await user.selectOptions(screen.getByRole("combobox", { name: "Unit" }), "in");
  await user.clear(field("Width"));
  await user.type(field("Width"), "4");
  expect(field("Resolution")).toHaveValue(100);
  expect(field("Height")).toHaveValue(2);
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onapply).toHaveBeenCalledWith(400, 200, [0.5, 0.5], 100);
});

test("Canvas Size adds relative sizes around the anchor", async () => {
  const { onapply, user } = open("canvas");
  await user.click(screen.getByRole("checkbox", { name: "Relative" }));
  expect(field("Width")).toHaveValue(0);
  await user.type(field("Width"), "50");
  // Canvas Size never links the sides.
  expect(field("Height")).toHaveValue(0);
  await user.click(screen.getAllByRole("radio", { name: "Anchor" })[0]);
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onapply).toHaveBeenCalledWith(450, 200, [0, 0], 72);
});

test("an empty size cannot be applied, and Cancel closes", async () => {
  const { onapply, onclose, user } = open();
  await user.clear(field("Width"));
  expect(screen.getByRole("button", { name: "OK" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onclose).toHaveBeenCalled();
  expect(onapply).not.toHaveBeenCalled();
});
