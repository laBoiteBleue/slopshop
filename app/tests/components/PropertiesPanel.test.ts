import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import { ADJUSTMENT_PARAMS, type AdjustmentId, type LayerView } from "../../src/lib/engine";
import PropertiesPanel from "../../src/lib/PropertiesPanel.svelte";

const padded = (values: number[]) => [
  ...values,
  ...Array<number>(ADJUSTMENT_PARAMS - values.length).fill(0),
];

function adjustmentLayer(id: AdjustmentId, values: number[]): LayerView {
  return {
    id: 7,
    name: id,
    visible: true,
    opacity: 1,
    kind: "adjustment",
    swatch: [0, 0, 0, 0],
    blendMode: "normal",
    contentKey: 0,
    hasAlpha: false,
    mask: null,
    children: [],
    passThrough: false,
    clipped: false,
    transform: [1, 0, 0, 1, 0, 0],
    painted: false,
    entries: [],
    adjustment: { id, values: padded(values), curves: null, curveSamples: null },
  };
}

function open(id: AdjustmentId, values: number[]) {
  const onedit = vi.fn();
  const onlive = vi.fn();
  const ongestureend = vi.fn();
  render(PropertiesPanel, {
    documentId: 1,
    layer: adjustmentLayer(id, values),
    onedit,
    onlive,
    ongestureend,
  });
  return { onedit, onlive, ongestureend, user: userEvent.setup() };
}

/** The values sent by the last call of `fn`. */
const sent = (fn: ReturnType<typeof vi.fn>) =>
  (fn.mock.lastCall?.[1] as { values: number[] }).values;
const number = (name: string) => screen.getByRole("spinbutton", { name });

async function typeInto(user: ReturnType<typeof userEvent.setup>, name: string, text: string) {
  await user.clear(number(name));
  await user.type(number(name), text);
  await user.tab();
}

test("Levels are shown 0–255, stored 0–1, the input black kept below the input white", async () => {
  const { onedit, user } = open("levels", [0, 100 / 255, 1, 0, 1]);
  expect(number("Input white")).toHaveValue(100);
  await typeInto(user, "Input black", "50");
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "setAdjustment",
    id: 7,
    adjustment: "levels",
    values: padded([50 / 255, 100 / 255, 1, 0, 1]),
  });
  await typeInto(user, "Input black", "200");
  expect(sent(onedit)[0]).toBeCloseTo(98 / 255);
});

test("Levels' Channel menu edits each channel's own settings", async () => {
  const identity = [0, 1, 1, 0, 1];
  const { onedit, user } = open("levels", [...identity, ...identity, ...identity, ...identity]);
  const channel = screen.getByRole("combobox", { name: "Channel" });
  expect(channel).toHaveValue("0");
  await user.selectOptions(channel, "Green");
  await typeInto(user, "Input black", "30");
  // Green's input black (values 10–14), the composite and the others unchanged.
  expect(sent(onedit).slice(0, 15)).toEqual([...identity, ...identity, 30 / 255, 1, 1, 0, 1]);
  // Green's input white stays above its input black.
  await typeInto(user, "Input white", "10");
  expect(sent(onedit)[11]).toBeCloseTo(32 / 255);
  expect(sent(onedit)[1]).toBe(1);
});

test("a slider applies live during the drag, one undo entry at its end", async () => {
  const { onlive, ongestureend, onedit } = open("hueSaturation", []);
  const slider = screen.getByRole("slider", { name: "Hue" });
  await fireEvent.input(slider, { target: { value: "30" } });
  await fireEvent.input(slider, { target: { value: "45" } });
  expect(onlive).toHaveBeenCalledTimes(2);
  expect(sent(onlive)[0]).toBe(45);
  await fireEvent.change(slider);
  expect(ongestureend).toHaveBeenCalledWith(1);
  expect(onedit).not.toHaveBeenCalled();
});

test("Color Balance's tone chooses which values the sliders edit", async () => {
  const { onedit, user } = open("colorBalance", [0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
  await user.selectOptions(screen.getByRole("combobox", { name: "Tone" }), "Highlights");
  await typeInto(user, "Cyan / Red", "20");
  expect(sent(onedit)[6]).toBe(20);
  expect(sent(onedit)[0]).toBe(0);
  await user.click(screen.getByRole("checkbox", { name: "Preserve Luminosity" }));
  expect(sent(onedit)[9]).toBe(0);
});

test("Monochrome hides the Channel Mixer's output channel", async () => {
  const { user } = open("channelMixer", [100, 0, 0, 0, 0, 100, 0, 0, 0, 0, 100, 0, 0]);
  expect(screen.getByRole("combobox", { name: "Output Channel" })).toBeInTheDocument();
  await user.click(screen.getByRole("checkbox", { name: "Monochrome" }));
  expect(screen.queryByRole("combobox", { name: "Output Channel" })).not.toBeInTheDocument();
});

test("Black & White's tint sliders wait for Tint", () => {
  open("blackWhite", [40, 60, 40, 60, 20, 80, 0, 42, 20]);
  expect(screen.getByRole("slider", { name: "Hue" })).toBeDisabled();
  expect(screen.getByRole("slider", { name: "Reds" })).toBeEnabled();
});

test("Reset puts back a new layer's settings; Invert has none", async () => {
  const { onedit, user } = open("exposure", [2, 0.1, 1.5]);
  await user.click(screen.getByRole("button", { name: "Reset" }));
  expect(sent(onedit)).toEqual(padded([0, 0, 1]));
  document.body.innerHTML = "";
  open("invert", []);
  expect(screen.getByText("No settings")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Reset" })).not.toBeInTheDocument();
});

test("a fill layer shows its color; a click on it asks for another", async () => {
  const onfillcolor = vi.fn();
  const fill: LayerView = {
    ...adjustmentLayer("levels", []),
    kind: "fill",
    swatch: [0, 0.5, 1, 1],
    adjustment: null,
  };
  render(PropertiesPanel, {
    documentId: 1,
    layer: fill,
    onedit: vi.fn(),
    onlive: vi.fn(),
    ongestureend: vi.fn(),
    onfillcolor,
  });
  expect(screen.getByText("Solid Color")).toBeInTheDocument();
  const swatch = screen.getByRole("button", { name: "Color Picker (Solid Color)" });
  expect(swatch).toHaveStyle({ background: "#0080ff" });
  await userEvent.setup().click(swatch);
  expect(onfillcolor).toHaveBeenCalledWith(fill);
});
