import { fireEvent, render, screen, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test } from "vitest";
import BrushPicker from "../../src/lib/BrushPicker.svelte";

const STORAGE_KEY = "slopshop.brushPresets";

beforeEach(() => localStorage.clear());

function open(size = 20, hardness = 0.5) {
  const view = render(BrushPicker, { size, hardness });
  return { ...view, user: userEvent.setup() };
}

const trigger = () => screen.getByRole("button", { name: "Brush preset picker" });
const panel = () => screen.getByRole("dialog", { name: "Brush preset picker" });
/** The size (px) and hardness (%) fields of the panel. */
const sizeField = () => within(panel()).getAllByRole("spinbutton")[0] as HTMLInputElement;
const hardnessField = () => within(panel()).getAllByRole("spinbutton")[1] as HTMLInputElement;
/** A preset of the grid, by its tooltip (its name is only its size). */
const preset = (title: string) => within(panel()).getByTitle(title);
const stored = () => JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "[]");

test("the button shows the brush's size, and drops the panel down when clicked", async () => {
  const { user } = open(42);
  expect(trigger()).toHaveTextContent("42");
  expect(trigger()).toHaveAttribute("aria-expanded", "false");
  expect(screen.queryByRole("dialog")).toBeNull();
  await user.click(trigger());
  expect(trigger()).toHaveAttribute("aria-expanded", "true");
  expect(sizeField()).toHaveValue(42);
  expect(hardnessField()).toHaveValue(50);
});

test("Escape, a click elsewhere and a second click on the button close the panel", async () => {
  const { user } = open();
  await user.click(trigger());
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("dialog")).toBeNull();
  await user.click(trigger());
  await user.pointer({ keys: "[MouseLeft]", target: document.body });
  expect(screen.queryByRole("dialog")).toBeNull();
  await user.click(trigger());
  await user.click(trigger());
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("a click inside the panel keeps it open", async () => {
  const { user } = open();
  await user.click(trigger());
  await user.click(preset("Hard Round 30 px"));
  expect(panel()).toBeInTheDocument();
});

test("picking a round brush sets the size and the hardness", async () => {
  const { user } = open(20, 0.5);
  await user.click(trigger());
  await user.click(preset("Soft Round 100 px"));
  expect(trigger()).toHaveTextContent("100");
  expect(sizeField()).toHaveValue(100);
  expect(hardnessField()).toHaveValue(0);
  await user.click(preset("Hard Round 5 px"));
  expect(sizeField()).toHaveValue(5);
  expect(hardnessField()).toHaveValue(100);
});

test("the size and hardness typed in the panel change the brush, within their ranges", async () => {
  const { user } = open(20, 0.5);
  await user.click(trigger());
  await fireEvent.input(sizeField(), { target: { value: "75" } });
  expect(trigger()).toHaveTextContent("75");
  await fireEvent.input(sizeField(), { target: { value: "99999" } });
  expect(sizeField()).toHaveValue(5000);
  await fireEvent.input(hardnessField(), { target: { value: "80" } });
  expect(hardnessField()).toHaveValue(80);
  await fireEvent.input(hardnessField(), { target: { value: "-5" } });
  expect(hardnessField()).toHaveValue(0);
});

test("+ saves the current brush, once, and keeps it for next time", async () => {
  const { user, unmount } = open(37, 0.25);
  await user.click(trigger());
  await user.click(screen.getByRole("button", { name: "Save the current brush as a preset" }));
  await user.click(screen.getByRole("button", { name: "Save the current brush as a preset" }));
  expect(stored()).toEqual([{ size: 37, hardness: 0.25 }]);
  expect(preset("37 px, hardness 25% (right-click: remove)")).toBeInTheDocument();
  // Another session: the saved brush is back.
  unmount();
  const next = open(10, 1);
  await next.user.click(trigger());
  await next.user.click(preset("37 px, hardness 25% (right-click: remove)"));
  expect(sizeField()).toHaveValue(37);
  expect(hardnessField()).toHaveValue(25);
});

test("a right-click removes a saved brush", async () => {
  localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify([
      { size: 11, hardness: 0.1 },
      { size: 22, hardness: 0.2 },
    ]),
  );
  const { user } = open();
  await user.click(trigger());
  await user.pointer({
    keys: "[MouseRight]",
    target: preset("11 px, hardness 10% (right-click: remove)"),
  });
  expect(within(panel()).queryByTitle(/^11 px, hardness/)).toBeNull();
  expect(preset("22 px, hardness 20% (right-click: remove)")).toBeInTheDocument();
  expect(stored()).toEqual([{ size: 22, hardness: 0.2 }]);
});

test("saved brushes that make no sense are ignored", async () => {
  localStorage.setItem(
    STORAGE_KEY,
    JSON.stringify([
      { size: 0, hardness: 0.5 },
      { size: 10, hardness: 2 },
      { size: "big", hardness: 0.5 },
      null,
      { size: 12, hardness: 0.5 },
    ]),
  );
  const { user } = open();
  await user.click(trigger());
  expect(within(panel()).getAllByTitle(/right-click: remove/)).toHaveLength(1);
  expect(preset("12 px, hardness 50% (right-click: remove)")).toBeInTheDocument();
});

test("unreadable saved brushes leave only the round brushes", async () => {
  localStorage.setItem(STORAGE_KEY, "{not json");
  const { user } = open();
  await user.click(trigger());
  expect(within(panel()).queryAllByTitle(/right-click: remove/)).toHaveLength(0);
  expect(preset("Hard Round 1 px")).toBeInTheDocument();
});
