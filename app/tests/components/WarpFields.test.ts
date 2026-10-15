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

test("Custom is a choice of its own, without bend nor orientation", async () => {
  const { onchange, user } = open({ style: "custom", bend: 50, vertical: false });
  expect(screen.getByRole("combobox", { name: "Warp" })).toHaveValue("custom");
  expect(screen.getByRole("combobox", { name: "Orientation" })).toBeDisabled();
  // A style chosen from Custom starts at Photoshop's bend.
  await user.selectOptions(screen.getByRole("combobox", { name: "Warp" }), "Bulge");
  expect(onchange).toHaveBeenLastCalledWith({ style: "bulge", bend: 50, vertical: false });
  await user.selectOptions(screen.getByRole("combobox", { name: "Warp" }), "Custom");
  expect(onchange).toHaveBeenLastCalledWith({ style: "custom", bend: 50, vertical: false });
});

test("the grid's sizes: Default, 4 × 4, 5 × 5; another grid shows Custom", async () => {
  const onchange = vi.fn();
  const onsize = vi.fn();
  const settings: WarpSettings = { style: null, bend: 50, vertical: false };
  const { rerender } = render(WarpFields, { settings, size: 3, onchange, onsize });
  const user = userEvent.setup();
  const grid = screen.getByRole("combobox", { name: "Grid" });
  expect(grid).toHaveValue("3");
  expect([...grid.querySelectorAll("option")].map((o) => o.textContent)).toEqual([
    "Default",
    "4 × 4",
    "5 × 5",
  ]);
  await user.selectOptions(grid, "5 × 5");
  expect(onsize).toHaveBeenLastCalledWith(5);
  await rerender({ settings, size: null, onchange, onsize });
  expect(grid).toHaveValue("custom");
});

test("Split Warp's three modes toggle, and are off at the most patches", async () => {
  const onchange = vi.fn();
  const onsplit = vi.fn();
  const settings: WarpSettings = { style: null, bend: 50, vertical: false };
  const { rerender } = render(WarpFields, { settings, onchange, onsplit });
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "Split Warp Crosswise" }));
  expect(onsplit).toHaveBeenLastCalledWith("crosswise");
  await rerender({ settings, onchange, onsplit, split: "crosswise" });
  const crosswise = screen.getByRole("button", { name: "Split Warp Crosswise" });
  expect(crosswise).toHaveAttribute("aria-pressed", "true");
  await user.click(crosswise);
  expect(onsplit).toHaveBeenLastCalledWith(null);
  await user.click(screen.getByRole("button", { name: "Split Warp Horizontally" }));
  expect(onsplit).toHaveBeenLastCalledWith("horizontal");
  await rerender({ settings, onchange, onsplit, splittable: false });
  expect(screen.getByRole("button", { name: "Split Warp Vertically" })).toBeDisabled();
});

test("the Warp menu lists None, Custom and Photoshop's fifteen styles", async () => {
  const { onchange, user } = open({ style: null, bend: 50, vertical: false });
  const menu = screen.getByRole("combobox", { name: "Warp" });
  const labels = [...menu.querySelectorAll("option")].map((o) => o.textContent);
  expect(labels).toHaveLength(17);
  expect(labels.slice(0, 6)).toEqual(["None", "Custom", "Arc", "Arc Lower", "Arc Upper", "Arch"]);
  expect(labels.at(-1)).toBe("Twist");
  await user.selectOptions(menu, "Fisheye");
  expect(onchange).toHaveBeenLastCalledWith({ style: "fisheye", bend: 50, vertical: false });
});
