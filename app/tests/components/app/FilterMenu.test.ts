// The Filter menu (ADR 0034): filters on the active pixel layer, Repeat (Ctrl+F), and filter
// entries edited again from the Layers panel.
import { screen, within } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
import { documentView, layer, open, row, sent } from "./harness";

async function blurDialog(layers: LayerView[] = [layer(1, "Photo")]) {
  const user = open(documentView(1, "photo.jpg", layers));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Blur", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Gaussian Blur…" }));
  await screen.findByRole("dialog", { name: "Gaussian Blur" });
  return user;
}

const radius = () => screen.getByRole("spinbutton", { name: "Radius" });

/**
 * What OK applied: the edit replacing the dialog's gesture, or, when the canvas showed it
 * already, its last live edit kept as the undo entry (the gesture ended).
 */
function applied() {
  const replaced = sent("replace_gesture").at(-1)?.edit;
  if (replaced) return replaced;
  return sent("end_gesture").length > 0 ? sent("perform_live").at(-1)?.edit : undefined;
}

test("the Filter menu: Repeat and its settings grayed until a filter is applied, then Blur", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  expect(screen.getByRole("menuitem", { name: /Repeat Last Filter/ })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  expect(screen.getByRole("menuitem", { name: /Last Filter Settings/ })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  expect(screen.getByText("Blur", { selector: ".label" })).toBeInTheDocument();
});

test("Blur > Motion Blur: an angle and a distance", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Blur", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Motion Blur…" }));
  await screen.findByRole("dialog", { name: "Motion Blur" });
  const angle = screen.getByRole("spinbutton", { name: "Angle" });
  await user.clear(angle);
  await user.type(angle, "-30");
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "motionBlur",
      values: [-30, 10],
    }),
  );
});

test("Noise > Add Noise: Gaussian and monochromatic, a new seed each time it is applied", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Noise", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Add Noise…" }));
  const dialog = await screen.findByRole("dialog", { name: "Add Noise" });
  expect(within(dialog).getByRole("radio", { name: "Uniform" })).toBeChecked();
  await user.click(within(dialog).getByRole("radio", { name: "Gaussian" }));
  await user.click(within(dialog).getByRole("checkbox", { name: "Monochromatic" }));
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toMatchObject({
      kind: "applyFilter",
      filter: "addNoise",
    }),
  );
  const first = applied() as { values: number[] };
  expect(first.values.slice(0, 3)).toEqual([12.5, 1, 1]);
  expect(Number.isInteger(first.values[3])).toBe(true);
  // Repeat: the same settings, another grain.
  await user.keyboard("{Control>}f{/Control}");
  await vi.waitFor(() =>
    expect(sent("perform").at(-1)?.edit).toMatchObject({ filter: "addNoise" }),
  );
  const again = sent("perform").at(-1)?.edit as { values: number[] };
  expect(again.values.slice(0, 3)).toEqual([12.5, 1, 1]);
  expect(again.values[3]).not.toBe(first.values[3]);
});

test("Noise > Dust & Scratches: a radius and a threshold", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Noise", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Dust & Scratches…" }));
  await screen.findByRole("dialog", { name: "Dust & Scratches" });
  const threshold = screen.getByRole("spinbutton", { name: "Threshold" });
  await user.clear(threshold);
  await user.type(threshold, "12");
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "dustAndScratches",
      values: [1, 12],
    }),
  );
});

test("Sharpen > Unsharp Mask and Other > High Pass apply to the active layer", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Sharpen", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Unsharp Mask…" }));
  await screen.findByRole("dialog", { name: "Unsharp Mask" });
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "unsharpMask",
      values: [100, 1, 0],
    }),
  );
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Other", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "High Pass…" }));
  await screen.findByRole("dialog", { name: "High Pass" });
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "highPass",
      values: [10],
    }),
  );
});

test("Gaussian Blur shows live on the active layer, OK is one undo entry, Ctrl+F repeats it", async () => {
  const user = await blurDialog();
  await vi.waitFor(() =>
    expect(sent("perform_live").at(-1)).toEqual({
      documentId: 1,
      edit: { kind: "applyFilter", id: 1, filter: "gaussianBlur", values: [1] },
      replace: true,
    }),
  );
  await user.clear(radius());
  await user.type(radius(), "6");
  await vi.waitFor(() => expect(sent("perform_live").at(-1)?.edit).toMatchObject({ values: [6] }));
  await user.click(screen.getByRole("button", { name: "OK" }));
  const blur = { kind: "applyFilter", id: 1, filter: "gaussianBlur", values: [6] };
  // The canvas shows it already: kept as it is (what was computed of it too), one undo entry.
  await vi.waitFor(() => expect(sent("end_gesture")).toEqual([{ documentId: 1 }]));
  expect(sent("replace_gesture")).toHaveLength(0);
  expect(sent("perform_live").at(-1)?.edit).toEqual(blur);
  // Repeat: the same filter, a new entry.
  await user.keyboard("{Control>}f{/Control}");
  await vi.waitFor(() => expect(sent("perform").at(-1)?.edit).toEqual(blur));
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  expect(screen.getByRole("menuitem", { name: /Repeat Gaussian Blur/ })).not.toHaveAttribute(
    "aria-disabled",
    "true",
  );
});

test("Preview off takes the blur off the canvas; Cancel leaves no undo entry", async () => {
  const user = await blurDialog();
  await user.click(screen.getByRole("checkbox", { name: "Preview" }));
  await vi.waitFor(() => expect(sent("cancel_gesture").length).toBeGreaterThan(0));
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(sent("replace_gesture")).toHaveLength(0);
});

test("OK with Preview off applies the filter, which the canvas did not show", async () => {
  const user = await blurDialog();
  await user.click(screen.getByRole("checkbox", { name: "Preview" }));
  await vi.waitFor(() => expect(sent("cancel_gesture").length).toBeGreaterThan(0));
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(sent("replace_gesture").at(-1)?.edit).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "gaussianBlur",
      values: [1],
    }),
  );
  expect(sent("end_gesture")).toHaveLength(0);
});

test("filters are grayed on a hidden layer", async () => {
  const hidden = { ...layer(1, "Photo"), visible: false };
  const user = open(documentView(1, "photo.jpg", [hidden]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Blur", { selector: ".label" }));
  expect(screen.getByRole("menuitem", { name: "Gaussian Blur…" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
});

test("a filter entry is edited again with its icon: its radius, live, then one undo entry", async () => {
  const photo: LayerView = {
    ...layer(1, "Photo"),
    entries: [
      {
        kind: "filter",
        adjustment: null,
        count: 1,
        hidden: false,
        steps: [],
        filter: "gaussianBlur",
        filterSteps: [{ id: "gaussianBlur", values: [3] }],
      },
    ],
  };
  const user = open(documentView(1, "photo.jpg", [photo]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(within(row("Photo")).getByRole("button", { expanded: false }));
  expect(screen.getByText("Gaussian Blur", { selector: ".entry-name" })).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Edit Settings…" }));
  await screen.findByRole("dialog", { name: "Gaussian Blur" });
  expect(radius()).toHaveValue(3);
  await user.clear(radius());
  await user.type(radius(), "8");
  const edit = {
    kind: "setStackEntry",
    id: 1,
    index: 0,
    hidden: false,
    filters: [{ filter: "gaussianBlur", values: [8] }],
  };
  await vi.waitFor(() => expect(sent("perform_live").at(-1)?.edit).toEqual(edit));
  await user.click(screen.getByRole("button", { name: "OK" }));
  // Shown already: the gesture is the undo entry.
  await vi.waitFor(() => expect(sent("end_gesture")).toEqual([{ documentId: 1 }]));
  expect(sent("replace_gesture")).toHaveLength(0);
});

test("Other > Maximum and Minimum, Noise > Median, Blur > Box Blur: a radius each", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Blur", { selector: ".label" }));
  expect(screen.getByRole("menuitem", { name: "Box Blur…" })).toBeInTheDocument();
  await user.hover(screen.getByText("Noise", { selector: ".label" }));
  expect(screen.getByRole("menuitem", { name: "Median…" })).toBeInTheDocument();
  await user.hover(screen.getByText("Other", { selector: ".label" }));
  expect(screen.getByRole("menuitem", { name: "Minimum…" })).toBeInTheDocument();
  await user.click(screen.getByRole("menuitem", { name: "Maximum…" }));
  await screen.findByRole("dialog", { name: "Maximum" });
  await user.clear(radius());
  await user.type(radius(), "4");
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({ kind: "applyFilter", id: 1, filter: "maximum", values: [4] }),
  );
});

test("Stylize > Solarize applies at once; Pixelate > Mosaic opens its dialog", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Stylize", { selector: ".label" }));
  expect(screen.getByRole("menuitem", { name: "Emboss…" })).toBeInTheDocument();
  expect(screen.getByRole("menuitem", { name: "Find Edges" })).toBeInTheDocument();
  await user.click(screen.getByRole("menuitem", { name: "Solarize" }));
  await vi.waitFor(() =>
    expect(sent("perform").at(-1)?.edit).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "solarize",
      values: [],
    }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Pixelate", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Mosaic…" }));
  await screen.findByRole("dialog", { name: "Mosaic" });
});

test("Distort > Twirl: an angle sent with the filter", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Distort", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Twirl…" }));
  await screen.findByRole("dialog", { name: "Twirl" });
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({ kind: "applyFilter", id: 1, filter: "twirl", values: [50] }),
  );
});

/** Filter > Distort > `name`…: its dialog on a pixel layer. */
async function distortDialog(name: string) {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Distort", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: `${name}…` }));
  return { user, dialog: await screen.findByRole("dialog", { name }) };
}

test("Distort > Ripple: an amount and a size", async () => {
  const { user, dialog } = await distortDialog("Ripple");
  expect(within(dialog).getByRole("radio", { name: "Medium" })).toBeChecked();
  const amount = within(dialog).getByRole("spinbutton", { name: "Amount" });
  await user.clear(amount);
  await user.type(amount, "-450");
  await user.click(within(dialog).getByRole("radio", { name: "Large" }));
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({ kind: "applyFilter", id: 1, filter: "ripple", values: [-450, 2] }),
  );
});

test("Distort > Wave: Photoshop's defaults, Triangle, a seed drawn anew by Randomize", async () => {
  const { user, dialog } = await distortDialog("Wave");
  const generators = within(dialog).getByRole("spinbutton", { name: "Number of Generators" });
  expect(generators).toHaveValue(5);
  await user.click(within(dialog).getByRole("radio", { name: "Triangle" }));
  await user.click(within(dialog).getByRole("button", { name: "Randomize" }));
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(applied()).toMatchObject({ kind: "applyFilter", filter: "wave" }));
  const { values } = applied() as { values: number[] };
  expect(values.slice(0, 9)).toEqual([5, 10, 120, 5, 35, 100, 100, 1, 1]);
  expect(Number.isInteger(values[9])).toBe(true);
});

test("Distort > ZigZag and Spherize's modes", async () => {
  const { user, dialog } = await distortDialog("ZigZag");
  expect(within(dialog).getByRole("radio", { name: "Pond Ripples" })).toBeChecked();
  const ridges = within(dialog).getByRole("spinbutton", { name: "Ridges" });
  await user.clear(ridges);
  await user.type(ridges, "12");
  await user.click(within(dialog).getByRole("radio", { name: "Out From Center" }));
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "zigZag",
      values: [10, 12, 1],
    }),
  );
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Distort", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Spherize…" }));
  const sphere = await screen.findByRole("dialog", { name: "Spherize" });
  expect(within(sphere).getByRole("radio", { name: "Normal" })).toBeChecked();
  await user.click(within(sphere).getByRole("radio", { name: "Vertical Only" }));
  await user.click(within(sphere).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({ kind: "applyFilter", id: 1, filter: "spherize", values: [100, 2] }),
  );
});

test("Blur > Average applies at once, without a dialog", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Blur", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Average" }));
  await vi.waitFor(() =>
    expect(sent("perform").at(-1)?.edit).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "average",
      values: [],
    }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

test("Blur > Radial Blur: Zoom, Best, and its center picked in the square", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Blur", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Radial Blur…" }));
  const dialog = await screen.findByRole("dialog", { name: "Radial Blur" });
  expect(within(dialog).getByRole("radio", { name: "Spin" })).toBeChecked();
  expect(within(dialog).getByRole("radio", { name: "Good" })).toBeChecked();
  await user.click(within(dialog).getByRole("radio", { name: "Zoom" }));
  await user.click(within(dialog).getByRole("radio", { name: "Best" }));
  const amount = within(dialog).getByRole("spinbutton", { name: "Amount" });
  await user.clear(amount);
  await user.type(amount, "40");
  // The square is 120 pixels at (0, 0) here: a click a quarter across and three quarters down.
  const square = within(dialog).getByRole("slider", { name: "Blur Center" });
  square.getBoundingClientRect = () =>
    ({ left: 0, top: 0, width: 120, height: 120, right: 120, bottom: 120 }) as DOMRect;
  await user.pointer({ keys: "[MouseLeft]", target: square, coords: { clientX: 30, clientY: 90 } });
  // Then a nudge to the right with the keyboard.
  square.focus();
  await user.keyboard("{ArrowRight}");
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "radialBlur",
      values: [40, 1, 2, 26, 75],
    }),
  );
});

test("Blur > Surface Blur: a radius and a threshold", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Blur", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Surface Blur…" }));
  await screen.findByRole("dialog", { name: "Surface Blur" });
  const threshold = screen.getByRole("spinbutton", { name: "Threshold" });
  await user.clear(threshold);
  await user.type(threshold, "30");
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "surfaceBlur",
      values: [5, 30],
    }),
  );
});

test("Blur > Smart Blur: High quality, Edge Only", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Blur", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Smart Blur…" }));
  const dialog = await screen.findByRole("dialog", { name: "Smart Blur" });
  expect(within(dialog).getByRole("radio", { name: "Low" })).toBeChecked();
  expect(within(dialog).getByRole("radio", { name: "Normal" })).toBeChecked();
  await user.click(within(dialog).getByRole("radio", { name: "High" }));
  await user.click(within(dialog).getByRole("radio", { name: "Edge Only" }));
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "smartBlur",
      values: [3, 25, 2, 1],
    }),
  );
});

test("Image > Adjustments > Shadows/Highlights: on every pixel layer shown, More Options, not repeated", async () => {
  const hidden = { ...layer(3, "Hidden"), visible: false };
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo"), layer(2, "Sky"), hidden]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Image" }));
  await user.hover(screen.getByText("Adjustments", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Shadows/Highlights…" }));
  const dialog = within(await screen.findByRole("dialog", { name: "Shadows/Highlights" }));
  const settings = (values: number[]) => ({
    kind: "batch",
    edits: [1, 2].map((id) => ({ kind: "applyFilter", id, filter: "shadowsHighlights", values })),
  });
  // Shown live on both layers shown (not the hidden one), Photoshop's defaults.
  await vi.waitFor(() =>
    expect(sent("perform_live").at(-1)?.edit).toEqual(settings([35, 50, 30, 0, 50, 30, 20, 0])),
  );
  // The two Amounts at first; Show More Options adds Tone, Radius, Color and Midtone.
  expect(dialog.getAllByRole("spinbutton")).toHaveLength(2);
  expect(dialog.queryByRole("spinbutton", { name: "Midtone" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("checkbox", { name: "Show More Options" }));
  expect(dialog.getAllByRole("spinbutton")).toHaveLength(8);
  const midtone = dialog.getByRole("spinbutton", { name: "Midtone" });
  await user.clear(midtone);
  await user.type(midtone, "40");
  const chosen = settings([35, 50, 30, 0, 50, 30, 20, 40]);
  await vi.waitFor(() => expect(sent("perform_live").at(-1)?.edit).toEqual(chosen));
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(applied()).toEqual(chosen));
  // Not a filter of the Filter menu: Repeat stays grayed.
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  expect(screen.getByRole("menuitem", { name: /Repeat Last Filter/ })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
});
