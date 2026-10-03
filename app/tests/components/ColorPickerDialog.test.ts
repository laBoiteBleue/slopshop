import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import ColorPickerDialog from "../../src/lib/ColorPickerDialog.svelte";
import type { Rgb } from "../../src/lib/colorModel";

/** The number fields, in order: H S B, R G B, L a b. */
const NAMES = ["H", "S", "B", "R", "G", "Blue", "L", "a", "b"] as const;

function open(color = "#ff0000", sample?: { at: () => Promise<Rgb | null> }) {
  const onapply = vi.fn();
  const onclose = vi.fn();
  render(ColorPickerDialog, {
    title: "Color Picker (Foreground Color)",
    color,
    onapply,
    onclose,
    sample: sample && { at: sample.at, probe: () => true },
  });
  return { onapply, onclose, user: userEvent.setup() };
}

const field = (name: (typeof NAMES)[number]) =>
  screen.getAllByRole("spinbutton")[NAMES.indexOf(name)];
const hexField = () => screen.getByRole("textbox");

test("the fields show the color in HSB, RGB, Lab and hexadecimal", () => {
  open("#ff0000");
  expect(field("H")).toHaveValue(0);
  expect(field("S")).toHaveValue(100);
  expect(field("B")).toHaveValue(100);
  expect(field("R")).toHaveValue(255);
  expect(field("G")).toHaveValue(0);
  expect(field("L")).toHaveValue(54);
  expect(hexField()).toHaveValue("ff0000");
});

test("a component typed changes the others, and OK applies the color", async () => {
  const { onapply, user } = open("#ff0000");
  await user.clear(field("G"));
  await user.type(field("G"), "255");
  expect(hexField()).toHaveValue("ffff00");
  expect(field("H")).toHaveValue(60);
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onapply).toHaveBeenCalledWith("#ffff00");
});

test("a hexadecimal color is taken when valid, put back when not", async () => {
  const { user } = open("#ff0000");
  await user.clear(hexField());
  await user.type(hexField(), "336699");
  await user.tab();
  expect(field("R")).toHaveValue(51);
  expect(field("B")).toHaveValue(60);
  await user.clear(hexField());
  await user.type(hexField(), "nope");
  await user.tab();
  expect(hexField()).toHaveValue("336699");
});

test("a gray keeps its hue, so that saturation brings the color back", async () => {
  const { user } = open("#0080ff");
  const hue = Number((field("H") as HTMLInputElement).value);
  await user.clear(field("S"));
  await user.type(field("S"), "0");
  expect(field("H")).toHaveValue(hue);
  await user.clear(field("S"));
  await user.type(field("S"), "100");
  expect(hexField()).toHaveValue("0080ff");
});

test("the current color takes the color back; Cancel and Escape close", async () => {
  const { onclose, user } = open("#ff0000");
  await user.clear(field("R"));
  await user.type(field("R"), "0");
  await user.click(screen.getByRole("button", { name: "Back to the current color" }));
  expect(hexField()).toHaveValue("ff0000");
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  await user.keyboard("{Escape}");
  expect(onclose).toHaveBeenCalledTimes(2);
});

test("a click on the image takes the color shown there (the eyedropper)", async () => {
  const at = vi.fn(async (): Promise<Rgb> => [0, 0, 1]);
  const { user } = open("#ff0000", { at });
  const blocker = document.querySelector(".blocker") as HTMLElement;
  await user.pointer({ keys: "[MouseLeft]", target: blocker });
  expect(at).toHaveBeenCalled();
  await vi.waitFor(() => expect(hexField()).toHaveValue("0000ff"));
});
