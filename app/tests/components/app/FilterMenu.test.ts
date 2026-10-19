// The Filter menu (ADR 0034): filters on the active pixel layer, Repeat (Ctrl+F), and filter
// entries edited again from the Layers panel.
import { fireEvent, screen, within } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
import { documentView, layer, open, respond, row, sent } from "./harness";

// Displace's Choose a File…: a file picked, opened as a map.
respond("plugin:dialog|open", () => "C:/maps/ripples.psd");
respond("load_displacement_map", (args) => ({ id: 77, name: `map of ${String(args.path)}` }));

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
  const dialog = await screen.findByRole("dialog", { name: "Maximum" });
  await user.clear(radius());
  await user.type(radius(), "4");
  // Preserve: Squareness at first, Roundness the disk around.
  expect(within(dialog).getByRole("radio", { name: "Squareness" })).toBeChecked();
  await user.click(within(dialog).getByRole("radio", { name: "Roundness" }));
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({ kind: "applyFilter", id: 1, filter: "maximum", values: [4, 1] }),
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

test("Distort > Shear: the curve's points after Undefined Areas, Default straightens it", async () => {
  const { user, dialog } = await distortDialog("Shear");
  expect(within(dialog).getByRole("radio", { name: "Wrap Around" })).toBeChecked();
  expect(within(dialog).getAllByRole("slider")).toHaveLength(2);
  await fireEvent.keyDown(within(dialog).getByRole("slider", { name: "Point 1" }), {
    key: "ArrowRight",
    shiftKey: true,
  });
  await fireEvent.keyDown(within(dialog).getByRole("slider", { name: "Point 2" }), {
    key: "ArrowLeft",
  });
  await user.click(within(dialog).getByRole("radio", { name: "Repeat Edge Pixels" }));
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "shear",
      values: [1, 0, 10, 100, -1],
    }),
  );
  // Opened again with those settings; Default straightens the line.
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Distort", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Shear…" }));
  const again = await screen.findByRole("dialog", { name: "Shear" });
  expect(within(again).getByRole("slider", { name: "Point 1" })).toHaveAttribute(
    "aria-valuenow",
    "10",
  );
  await user.click(within(again).getByRole("button", { name: "Default" }));
  await user.click(within(again).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "shear",
      values: [1, 0, 0, 100, 0],
    }),
  );
});

test("Distort > Displace: a map of the Sources panel or a file, its scales and modes", async () => {
  const doc = {
    ...documentView(1, "photo.jpg", [layer(1, "Photo")]),
    sources: [
      { id: 5, kind: "image" as const, name: "clouds.png", width: 9, height: 9, layers: [1] },
    ],
  };
  const user = open(doc);
  await screen.findByText("Photo", { selector: "li .name" });
  const displace = async () => {
    await user.click(screen.getByRole("menuitem", { name: "Filter" }));
    await user.hover(screen.getByText("Distort", { selector: ".label" }));
    await user.click(screen.getByRole("menuitem", { name: "Displace…" }));
    return screen.findByRole("dialog", { name: "Displace" });
  };
  let dialog = await displace();
  expect(within(dialog).getByRole("radio", { name: "Stretch to Fit" })).toBeChecked();
  expect(within(dialog).getByRole("radio", { name: "Repeat Edge Pixels" })).toBeChecked();
  // Nothing shown nor applied without a map.
  expect(within(dialog).getByRole("button", { name: "OK" })).toBeDisabled();
  expect(sent("perform_live")).toEqual([]);
  await user.selectOptions(within(dialog).getByRole("combobox", { name: "Map" }), "5");
  const horizontal = within(dialog).getByRole("spinbutton", { name: "Horizontal Scale" });
  await user.clear(horizontal);
  await user.type(horizontal, "-50");
  await user.click(within(dialog).getByRole("radio", { name: "Tile" }));
  expect(sent("perform_live").at(-1)?.edit).toMatchObject({ filter: "displace", map: 5 });
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "displace",
      values: [-50, 10, 1, 1],
      map: 5,
    }),
  );
  // Again: the map chosen last at first; another opened from a file.
  dialog = await displace();
  expect(within(dialog).getByRole("combobox", { name: "Map" })).toHaveValue("5");
  await user.click(within(dialog).getByRole("button", { name: "Choose a File…" }));
  await vi.waitFor(() =>
    expect(sent("load_displacement_map")).toEqual([{ documentId: 1, path: "C:/maps/ripples.psd" }]),
  );
  await vi.waitFor(() =>
    expect(within(dialog).getByRole("combobox", { name: "Map" })).toHaveValue("77"),
  );
  expect(within(dialog).getByRole("option", { name: "map of C:/maps/ripples.psd" })).toBeTruthy();
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(applied()).toMatchObject({ filter: "displace", map: 77 }));
  // Repeat: the same map.
  await user.keyboard("{Control>}f{/Control}");
  await vi.waitFor(() =>
    expect(sent("perform").at(-1)?.edit).toMatchObject({ filter: "displace", map: 77 }),
  );
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

test("Render > Clouds applies at once with the colors of the moment; Ctrl+F draws other clouds", async () => {
  localStorage.clear();
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  // White in front, black behind.
  await user.keyboard("x");
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Render", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Clouds" }));
  await vi.waitFor(() =>
    expect(sent("perform").at(-1)?.edit).toMatchObject({
      kind: "applyFilter",
      id: 1,
      filter: "clouds",
    }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  const first = (sent("perform").at(-1)?.edit as { values: number[] }).values;
  expect(first.slice(0, 2)).toEqual([0xffffff, 0x000000]);
  expect(Number.isInteger(first[2])).toBe(true);
  // Repeated with the colors of the moment (black in front again) and a seed of its own.
  await user.keyboard("x");
  const random = vi.spyOn(Math, "random").mockReturnValue(0.25);
  await user.keyboard("{Control>}f{/Control}");
  await vi.waitFor(() =>
    expect(sent("perform").at(-1)?.edit).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "clouds",
      values: [0x000000, 0xffffff, 2 ** 22],
    }),
  );
  random.mockRestore();
});

test("Render > Difference Clouds applies at once; Fibers: Variance, Strength, Randomize", async () => {
  localStorage.clear();
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Render", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Difference Clouds" }));
  await vi.waitFor(() =>
    expect(sent("perform").at(-1)?.edit).toMatchObject({
      kind: "applyFilter",
      filter: "differenceClouds",
    }),
  );
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Render", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Fibers…" }));
  const dialog = await screen.findByRole("dialog", { name: "Fibers" });
  expect(within(dialog).getByRole("spinbutton", { name: "Variance" })).toHaveValue(16);
  const strength = within(dialog).getByRole("spinbutton", { name: "Strength" });
  expect(strength).toHaveValue(4);
  await user.clear(strength);
  await user.type(strength, "20");
  const random = vi.spyOn(Math, "random").mockReturnValue(0.5);
  await user.click(within(dialog).getByRole("button", { name: "Randomize" }));
  random.mockRestore();
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "fibers",
      values: [16, 20, 0x000000, 0xffffff, 2 ** 23],
    }),
  );
});

/** Filter > Stylize > `name`…: its dialog on a photo layer. */
async function stylizeDialog(name: string) {
  localStorage.clear();
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Stylize", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: `${name}…` }));
  const dialog = await screen.findByRole("dialog", { name });
  return { user, dialog };
}

test("Stylize > Diffuse: Anisotropic; Wind: Blast from the left, a seed each time", async () => {
  const { user, dialog } = await stylizeDialog("Diffuse");
  expect(within(dialog).getByRole("radio", { name: "Normal" })).toBeChecked();
  await user.click(within(dialog).getByRole("radio", { name: "Anisotropic" }));
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(applied()).toMatchObject({ filter: "diffuse" }));
  const diffuse = (applied() as { values: number[] }).values;
  expect(diffuse[0]).toBe(3);
  expect(Number.isInteger(diffuse[1])).toBe(true);

  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Stylize", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Wind…" }));
  const wind = await screen.findByRole("dialog", { name: "Wind" });
  expect(within(wind).getByRole("radio", { name: "Wind" })).toBeChecked();
  expect(within(wind).getByRole("radio", { name: "From the Right" })).toBeChecked();
  await user.click(within(wind).getByRole("radio", { name: "Blast" }));
  await user.click(within(wind).getByRole("radio", { name: "From the Left" }));
  await user.click(within(wind).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(applied()).toMatchObject({ filter: "wind" }));
  expect((applied() as { values: number[] }).values.slice(0, 2)).toEqual([1, 1]);
});

test("Stylize > Tiles: its number, offset and fill, the colors of the moment", async () => {
  const { user, dialog } = await stylizeDialog("Tiles");
  const number = within(dialog).getByRole("spinbutton", { name: "Number of Tiles" });
  expect(number).toHaveValue(10);
  expect(within(dialog).getByRole("spinbutton", { name: "Maximum Offset" })).toHaveValue(10);
  expect(within(dialog).getByRole("radio", { name: "Background Color" })).toBeChecked();
  await user.clear(number);
  await user.type(number, "25");
  await user.click(within(dialog).getByRole("radio", { name: "Inverse Image" }));
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(applied()).toMatchObject({ filter: "tiles" }));
  expect((applied() as { values: number[] }).values.slice(0, 5)).toEqual([
    25, 10, 2, 0x000000, 0xffffff,
  ]);
});

test("Stylize > Trace Contour: a level, Lower; Oil Paint: its brush, lighting off", async () => {
  const { user, dialog } = await stylizeDialog("Trace Contour");
  const level = within(dialog).getByRole("spinbutton", { name: "Level" });
  expect(level).toHaveValue(128);
  expect(within(dialog).getByRole("radio", { name: "Upper" })).toBeChecked();
  await user.clear(level);
  await user.type(level, "90");
  await user.click(within(dialog).getByRole("radio", { name: "Lower" }));
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "traceContour",
      values: [90, 0],
    }),
  );

  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Stylize", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Oil Paint…" }));
  const oil = await screen.findByRole("dialog", { name: "Oil Paint" });
  expect(within(oil).getByText("Brush")).toBeInTheDocument();
  const stylization = within(oil).getByRole("spinbutton", { name: "Stylization" });
  expect(stylization).toHaveValue(4);
  await user.clear(stylization);
  await user.type(stylization, "7.5");
  await user.click(within(oil).getByRole("checkbox", { name: "Lighting" }));
  await user.click(within(oil).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "oilPaint",
      values: [7.5, 5, 1, 2, 0, -60, 1],
    }),
  );
});

/** Filter > Pixelate > `name` (`…` for a dialog) on a photo layer. */
async function pixelate(name: string) {
  localStorage.clear();
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Pixelate", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name }));
  return user;
}

test("Pixelate > Facet and Fragment apply at once, with no dialog", async () => {
  const user = await pixelate("Facet");
  await vi.waitFor(() =>
    expect(sent("perform").at(-1)?.edit).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "facet",
      values: [],
    }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Pixelate", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Fragment" }));
  await vi.waitFor(() =>
    expect(sent("perform").at(-1)?.edit).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "fragment",
      values: [],
    }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

test("Pixelate > Color Halftone: Max. Radius and the screen angles of three channels", async () => {
  const user = await pixelate("Color Halftone…");
  const dialog = await screen.findByRole("dialog", { name: "Color Halftone" });
  expect(within(dialog).getByText("Screen Angles")).toBeInTheDocument();
  const radius = within(dialog).getByRole("spinbutton", { name: "Max. Radius" });
  expect(radius).toHaveValue(8);
  expect(within(dialog).getByRole("spinbutton", { name: "Channel 1" })).toHaveValue(108);
  expect(within(dialog).getByRole("spinbutton", { name: "Channel 3" })).toHaveValue(90);
  const second = within(dialog).getByRole("spinbutton", { name: "Channel 2" });
  expect(second).toHaveValue(162);
  await user.clear(radius);
  await user.type(radius, "20");
  await user.clear(second);
  await user.type(second, "-45");
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "colorHalftone",
      values: [20, 108, -45, 90],
    }),
  );
});

test("Pixelate > Crystallize and Pointillize: a cell size, a seed; Pointillize's background", async () => {
  const user = await pixelate("Crystallize…");
  const dialog = await screen.findByRole("dialog", { name: "Crystallize" });
  const cell = within(dialog).getByRole("spinbutton", { name: "Cell Size" });
  expect(cell).toHaveValue(10);
  await user.clear(cell);
  await user.type(cell, "40");
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(applied()).toMatchObject({ filter: "crystallize" }));
  const crystals = (applied() as { values: number[] }).values;
  expect(crystals[0]).toBe(40);
  expect(Number.isInteger(crystals[1])).toBe(true);

  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Pixelate", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Pointillize…" }));
  const dots = await screen.findByRole("dialog", { name: "Pointillize" });
  expect(within(dots).getByRole("spinbutton", { name: "Cell Size" })).toHaveValue(5);
  await user.click(within(dots).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(applied()).toMatchObject({ filter: "pointillize" }));
  // The background color of the moment (white by default).
  expect((applied() as { values: number[] }).values.slice(0, 2)).toEqual([5, 0xffffff]);
});

test("Pixelate > Mezzotint: Fine Dots at first, its ten types", async () => {
  const user = await pixelate("Mezzotint…");
  const dialog = await screen.findByRole("dialog", { name: "Mezzotint" });
  expect(within(dialog).getAllByRole("radio")).toHaveLength(10);
  expect(within(dialog).getByRole("radio", { name: "Fine Dots" })).toBeChecked();
  await user.click(within(dialog).getByRole("radio", { name: "Long Strokes" }));
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(applied()).toMatchObject({ filter: "mezzotint" }));
  const values = (applied() as { values: number[] }).values;
  expect(values[0]).toBe(9);
  expect(Number.isInteger(values[1])).toBe(true);
});

/** Filter > `submenu` > `name`, on a pixel layer. */
async function filterItem(submenu: string, name: string) {
  localStorage.clear();
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText(submenu, { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name }));
  return user;
}

test("Other > Custom: its 5 x 5 weights in a grid, Scale and Offset", async () => {
  const user = await filterItem("Other", "Custom…");
  const dialog = await screen.findByRole("dialog", { name: "Custom" });
  // Photoshop's sharpening kernel at first.
  const cell = (row: number, column: number) =>
    within(dialog).getByRole("spinbutton", { name: `Row ${row}, column ${column}` });
  expect(cell(3, 3)).toHaveValue(5);
  expect(cell(2, 3)).toHaveValue(-1);
  expect(cell(1, 1)).toHaveValue(0);
  await user.clear(cell(3, 3));
  await user.type(cell(3, 3), "9");
  await user.clear(cell(1, 5));
  await user.type(cell(1, 5), "-2");
  const scale = within(dialog).getByRole("spinbutton", { name: "Scale" });
  await user.clear(scale);
  await user.type(scale, "3");
  const offset = within(dialog).getByRole("spinbutton", { name: "Offset" });
  await user.clear(offset);
  await user.type(offset, "10");
  // A weight beyond 999: OK grayed.
  await user.clear(cell(5, 5));
  await user.type(cell(5, 5), "1000");
  expect(within(dialog).getByRole("button", { name: "OK" })).toBeDisabled();
  await user.clear(cell(5, 5));
  await user.type(cell(5, 5), "0");
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  const weights = [
    ...[0, 0, 0, 0, -2],
    ...[0, 0, -1, 0, 0],
    ...[0, -1, 9, -1, 0],
    ...[0, 0, -1, 0, 0],
    ...[0, 0, 0, 0, 0],
  ];
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "custom",
      values: [...weights, 3, 10],
    }),
  );
});

test("Other > HSB/HSL: RGB to HSB at first, HSL to RGB sent", async () => {
  const user = await filterItem("Other", "HSB/HSL…");
  const dialog = await screen.findByRole("dialog", { name: "HSB/HSL" });
  const input = within(dialog).getByRole("group", { name: "Input Mode" });
  const order = within(dialog).getByRole("group", { name: "Row Order" });
  expect(within(input).getByRole("radio", { name: "RGB" })).toBeChecked();
  expect(within(order).getByRole("radio", { name: "HSB" })).toBeChecked();
  await user.click(within(input).getByRole("radio", { name: "HSL" }));
  await user.click(within(order).getByRole("radio", { name: "RGB" }));
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({ kind: "applyFilter", id: 1, filter: "hsbHsl", values: [2, 0] }),
  );
});

test("Noise > Despeckle applies at once; Reduce Noise: its four settings", async () => {
  const user = await filterItem("Noise", "Despeckle");
  await vi.waitFor(() =>
    expect(sent("perform").at(-1)?.edit).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "despeckle",
      values: [],
    }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Noise", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Reduce Noise…" }));
  const dialog = await screen.findByRole("dialog", { name: "Reduce Noise" });
  const field = (name: string) => within(dialog).getByRole("spinbutton", { name });
  expect(field("Strength")).toHaveValue(6);
  expect(field("Preserve Details")).toHaveValue(60);
  await user.clear(field("Strength"));
  await user.type(field("Strength"), "9");
  await user.clear(field("Reduce Color Noise"));
  await user.type(field("Reduce Color Noise"), "80");
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "reduceNoise",
      values: [9, 60, 80, 25],
    }),
  );
});

test("Sharpen > Smart Sharpen: Lens Blur at first, Motion Blur with its angle", async () => {
  const user = await filterItem("Sharpen", "Smart Sharpen…");
  const dialog = await screen.findByRole("dialog", { name: "Smart Sharpen" });
  expect(within(dialog).getByRole("radio", { name: "Lens Blur" })).toBeChecked();
  const field = (name: string) => within(dialog).getByRole("spinbutton", { name });
  expect(field("Amount")).toHaveValue(200);
  await user.clear(field("Radius"));
  await user.type(field("Radius"), "2.5");
  await user.click(within(dialog).getByRole("radio", { name: "Motion Blur" }));
  await user.clear(field("Angle"));
  await user.type(field("Angle"), "45");
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(applied()).toEqual({
      kind: "applyFilter",
      id: 1,
      filter: "smartSharpen",
      values: [200, 2.5, 10, 2, 45],
    }),
  );
});
