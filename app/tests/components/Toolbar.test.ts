import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import Toolbar from "../../src/lib/Toolbar.svelte";
import type { ToolId } from "../../src/lib/tools";

afterEach(() => vi.useRealTimers());

function open(tool: ToolId = "move", choices: Record<string, ToolId> = {}) {
  const onselect = vi.fn();
  const onpickcolor = vi.fn();
  const colors = { foreground: "#ff0000", background: "#0000ff" };
  render(Toolbar, { tool, choices, onselect, onpickcolor, colors });
  return { onselect, onpickcolor };
}

const tool = (name: string) => screen.getByRole("button", { name });

test("a click picks the tool its button shows; the active one is pressed", async () => {
  const user = userEvent.setup();
  const { onselect } = open("move", { M: "ellipse" });
  expect(tool("Move Tool")).toHaveAttribute("aria-pressed", "true");
  // The slot shows the variant chosen last.
  await user.click(tool("Elliptical Marquee Tool"));
  expect(onselect).toHaveBeenCalledWith("ellipse");
});

test("a right-click lists a slot's variants, and one is picked", async () => {
  const user = userEvent.setup();
  const { onselect } = open();
  await user.pointer({ keys: "[MouseRight]", target: tool("Lasso Tool") });
  const variants = screen.getAllByRole("menuitemradio");
  expect(variants.map((v) => v.textContent?.trim())).toEqual([
    "Lasso Tool L",
    "Polygonal Lasso Tool L",
  ]);
  await user.click(screen.getByRole("menuitemradio", { name: /Polygonal Lasso Tool/ }));
  expect(onselect).toHaveBeenCalledWith("polygonalLasso");
  expect(screen.queryByRole("menuitemradio")).not.toBeInTheDocument();
});

test("a long press lists the variants without picking the tool shown", async () => {
  vi.useFakeTimers();
  const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
  const { onselect } = open();
  await user.pointer({ keys: "[MouseLeft>]", target: tool("Rectangular Marquee Tool") });
  vi.advanceTimersByTime(400);
  await user.pointer({ keys: "[/MouseLeft]", target: tool("Rectangular Marquee Tool") });
  expect(screen.getAllByRole("menuitemradio")).toHaveLength(2);
  expect(onselect).not.toHaveBeenCalled();
});

test("Escape closes the variants", async () => {
  const user = userEvent.setup();
  open();
  await user.pointer({ keys: "[MouseRight]", target: tool("Lasso Tool") });
  await user.keyboard("{Escape}");
  expect(screen.queryByRole("menuitemradio")).not.toBeInTheDocument();
});

test("the color squares open the picker, swap and reset the colors", async () => {
  const user = userEvent.setup();
  const { onpickcolor } = open();
  await user.click(screen.getByRole("button", { name: "Set foreground color" }));
  expect(onpickcolor).toHaveBeenCalledWith("foreground");
  const foreground = screen.getByRole("button", { name: "Set foreground color" });
  const background = screen.getByRole("button", { name: "Set background color" });
  await user.click(screen.getByRole("button", { name: /Switch foreground and background/ }));
  expect(foreground).toHaveStyle({ background: "#0000ff" });
  expect(background).toHaveStyle({ background: "#ff0000" });
  await user.click(screen.getByRole("button", { name: /Default foreground and background/ }));
  expect(foreground).toHaveStyle({ background: "#000000" });
  expect(background).toHaveStyle({ background: "#ffffff" });
});
