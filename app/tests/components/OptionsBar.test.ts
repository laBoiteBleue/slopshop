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

test("the Eyedropper's Sample Size and Sample change its settings", async () => {
  const eyedropper = { sample: "all", size: 1 };
  const { user } = open("eyedropper", { eyedropper });
  const size = screen.getByRole("combobox", { name: "Sample Size:" });
  expect([...size.querySelectorAll("option")].map((o) => o.textContent?.trim())).toEqual([
    "Point Sample",
    "3 by 3 Average",
    "5 by 5 Average",
    "11 by 11 Average",
    "31 by 31 Average",
    "51 by 51 Average",
    "101 by 101 Average",
  ]);
  await user.selectOptions(size, "5 by 5 Average");
  await user.selectOptions(screen.getByRole("combobox", { name: "Sample:" }), "Current Layer");
  expect(eyedropper).toEqual({ sample: "layer", size: 5 });
});

test("Crop's ratio or size: presets, the canvas's ratio, a size, swapped", async () => {
  const { user } = open("crop", { canvasSize: { width: 400, height: 300 } });
  const preset = screen.getByRole("combobox", { name: "Ratio or size" });
  expect(preset).toHaveValue("free");
  expect(screen.queryByText("W:")).not.toBeInTheDocument();
  await user.selectOptions(preset, "16 : 9");
  expect(preset).toHaveValue("16:9");
  await user.selectOptions(preset, "Original Ratio");
  expect(preset).toHaveValue("original");
  await user.selectOptions(preset, "Size (px)");
  // W, then H.
  const fields = () => screen.getAllByRole("spinbutton").map((f) => (f as HTMLInputElement).value);
  expect(fields()).toEqual(["400", "300"]);
  await user.click(screen.getByRole("button", { name: "Swap the width and the height" }));
  expect(fields()).toEqual(["300", "400"]);
  // Straighten, a switch.
  const straighten = screen.getByRole("button", { name: "Straighten" });
  expect(straighten).toHaveAttribute("aria-pressed", "false");
  await user.click(straighten);
  expect(straighten).toHaveAttribute("aria-pressed", "true");
});

test("the Paint Bucket's opacity and region settings", async () => {
  const bucket = { opacity: 1, tolerance: 32, contiguous: true, antiAlias: true, sampleAll: false };
  const { user } = open("paintBucket", { bucket });
  await user.click(check("Contiguous"));
  await user.click(check("Anti-alias"));
  await user.click(check("Sample All Layers"));
  expect(bucket).toEqual({
    opacity: 1,
    tolerance: 32,
    contiguous: false,
    antiAlias: false,
    sampleAll: true,
  });
  expect(screen.getByText("Opacity:")).toBeInTheDocument();
  expect(screen.getByText("Tolerance:")).toBeInTheDocument();
});

test("the Brush and the Eraser keep their own settings", async () => {
  const { props, user } = open("eraser");
  await user.click(check("Pressure: size"));
  expect(props.eraser.pressureSize).toBe(true);
  expect(props.brush.pressureSize).toBe(false);
  expect(screen.getByText("Opacity:")).toBeInTheDocument();
  expect(screen.getByText("Flow:")).toBeInTheDocument();
});

test("Quick Mask is said whatever the tool, with its overlay's opacity", () => {
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
    quickMask: true,
    quickMaskOpacity: 50,
  });
  expect(screen.getByRole("status")).toHaveTextContent("Quick Mask");
  expect(screen.getByText("Overlay opacity:")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Add" })).not.toBeInTheDocument();
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
