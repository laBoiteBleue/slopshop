import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import OptionsBar from "../../src/lib/OptionsBar.svelte";
import type { ToolId } from "../../src/lib/tools";

function open(tool: ToolId, more: Record<string, unknown> = {}) {
  const paint = () => ({
    size: 20,
    hardness: 1,
    opacity: 1,
    flow: 1,
    pressureSize: false,
    pressureOpacity: false,
  });
  const props = {
    tool,
    autoSelect: false,
    selectionMode: "replace" as const,
    feather: 0,
    antiAlias: true,
    wand: { tolerance: 32, contiguous: true, sampleAll: false },
    quick: { size: 30, sampleAll: false, objectRefine: false },
    brush: paint(),
    eraser: paint(),
    ...more,
  };
  render(OptionsBar, props);
  return { props, user: userEvent.setup() };
}

const check = (name: string) => screen.getByRole("checkbox", { name });
const mode = (name: RegExp) => screen.getByRole("button", { name });

test("each tool shows its own options", () => {
  open("move");
  expect(check("Auto-Select")).toBeInTheDocument();
  expect(screen.queryByText("Feather:")).not.toBeInTheDocument();
});

test("the selection modes, without intersection for Quick Selection", async () => {
  const { user } = open("marquee");
  expect(mode(/New selection/)).toHaveAttribute("aria-pressed", "true");
  await user.click(mode(/Add to selection/));
  expect(mode(/Add to selection/)).toHaveAttribute("aria-pressed", "true");
  expect(mode(/New selection/)).toHaveAttribute("aria-pressed", "false");
  expect(screen.getByText("Feather:")).toBeInTheDocument();
  // The Rectangular Marquee has no anti-aliasing to choose.
  expect(screen.queryByRole("checkbox", { name: "Anti-alias" })).not.toBeInTheDocument();
});

test("Quick Selection has no intersection", () => {
  open("quickSelection");
  expect(screen.queryByRole("button", { name: /Intersect/ })).not.toBeInTheDocument();
  expect(screen.getByText("Size:")).toBeInTheDocument();
});

test("the Magic Wand's checkboxes change its settings", async () => {
  const { props, user } = open("wand");
  await user.click(check("Contiguous"));
  await user.click(check("Sample All Layers"));
  expect(props.wand).toEqual({ tolerance: 32, contiguous: false, sampleAll: true });
});

test("the Brush and the Eraser keep their own settings", async () => {
  const { props, user } = open("eraser");
  await user.click(check("Pressure: size"));
  expect(props.eraser.pressureSize).toBe(true);
  expect(props.brush.pressureSize).toBe(false);
  expect(screen.getByText("Opacity:")).toBeInTheDocument();
  expect(screen.getByText("Flow:")).toBeInTheDocument();
});

test("Quick Mask is shown whatever the tool, with Add / Remove for the Brush's colors", async () => {
  const onquickmask = vi.fn();
  render(OptionsBar, {
    tool: "move",
    autoSelect: false,
    selectionMode: "replace",
    feather: 0,
    antiAlias: true,
    wand: { tolerance: 32, contiguous: true, sampleAll: false },
    quick: { size: 30, sampleAll: false, objectRefine: false },
    brush: {
      size: 20,
      hardness: 1,
      opacity: 1,
      flow: 1,
      pressureSize: false,
      pressureOpacity: false,
    },
    eraser: {
      size: 20,
      hardness: 1,
      opacity: 1,
      flow: 1,
      pressureSize: false,
      pressureOpacity: false,
    },
    quickMask: { action: null },
    quickMaskOpacity: 50,
    onquickmask,
  });
  const user = userEvent.setup();
  expect(screen.getByRole("status")).toHaveTextContent("Quick Mask");
  // A gray paints some of either: neither is on.
  expect(screen.getByRole("button", { name: "Add" })).toHaveAttribute("aria-pressed", "false");
  expect(screen.getByRole("button", { name: "Remove" })).toHaveAttribute("aria-pressed", "false");
  await user.click(screen.getByRole("button", { name: "Add" }));
  await user.click(screen.getByRole("button", { name: "Remove" }));
  expect(onquickmask.mock.calls).toEqual([["add"], ["remove"]]);
  expect(screen.getByText("Overlay opacity:")).toBeInTheDocument();
  // The tool's own options follow.
  expect(check("Auto-Select")).toBeInTheDocument();
});

test("without Quick Mask, nothing of it", () => {
  open("brush");
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Add" })).not.toBeInTheDocument();
});

test("the Move tool's align and distribute buttons, grayed until enough layers are selected", async () => {
  const onalign = vi.fn();
  const ondistribute = vi.fn();
  const { user } = open("move", { alignable: true, distributable: false, onalign, ondistribute });
  await user.click(screen.getByRole("button", { name: "Align horizontal centers" }));
  expect(onalign).toHaveBeenCalledWith("horizontalCenters");
  const spacing = screen.getByRole("button", { name: "Distribute vertical spacing" });
  expect(spacing).toBeDisabled();
  await user.click(spacing);
  expect(ondistribute).not.toHaveBeenCalled();
});
