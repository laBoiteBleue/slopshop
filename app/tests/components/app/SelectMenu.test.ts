import { fireEvent, screen, within } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import { calls, documentView, layer, layerNames, menuLabels, open, respond, sent } from "./harness";

// The Select menu and what it opens: Grow and Similar, Transform Selection, Color Range,
// Modify, Quick Mask, saved selections, Select and Mask.

respond("set_quick_mask", (args, doc) => {
  doc.quickMask = args.on as boolean;
  doc.quickMaskOpacity = args.opacity as number;
  return { ...doc };
});
respond("save_selection", (args, doc) => {
  doc.savedSelections = [...doc.savedSelections, { id: 9, name: args.name as string }];
  return { ...doc };
});
for (const cmd of [
  "refine_open",
  "refine_view",
  "refine_preview",
  "refine_close",
  "refine_output",
  "refine_brush",
  "ai_refine_base",
]) {
  respond(cmd, (_, doc) => ({ ...doc }));
}
respond("selection_bounds", () => ({ left: 10, top: 20, right: 110, bottom: 70 }));

test("the Select menu: the basics, then by subject and color, Modify, Grow and Similar", async () => {
  const user = open({ ...documentView(1, "cat.jpg", [layer(1, "Cat")]), selectionKey: 7 });
  await screen.findByText("cat.jpg");
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  expect(menuLabels()).toEqual([
    "All",
    "Deselect",
    "Reselect",
    "Inverse",
    "—",
    "Select Subject",
    "Color Range…",
    "Select and Mask…",
    "—",
    "Modify",
    "—",
    "Grow",
    "Similar",
    "Transform Selection",
    "Quick Mask Mode",
    "—",
    "Save Selection…",
    "Load Selection",
    "—",
    "All Layers",
    "Deselect Layers",
  ]);
});

test("Select > Grow and Similar use the Magic Wand's tolerance on its sampled layer", async () => {
  const user = open({ ...documentView(1, "cat.jpg", [layer(1, "Cat")]), selectionKey: 7 });
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat"]));
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  await user.click(screen.getByText("Grow", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  await user.click(screen.getByText("Similar", { selector: ".label" }));
  await vi.waitFor(() =>
    expect(sent("grow_selection")).toEqual([
      { documentId: 1, tolerance: 32, contiguous: true, antiAlias: true, layerId: 1 },
      { documentId: 1, tolerance: 32, contiguous: false, antiAlias: true, layerId: 1 },
    ]),
  );
});

test("Grow and Similar wait for a selection", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  const grow = screen.getByText("Grow", { selector: ".label" }).closest("[role=menuitem]");
  expect(grow).toHaveAttribute("aria-disabled", "true");
});

test("Photoshop's selection shortcuts: All, Deselect, Reselect, Inverse", async () => {
  const user = open({
    ...documentView(1, "cat.jpg", [layer(1, "Cat")]),
    selectionKey: 7,
    canReselect: true,
  });
  await screen.findByText("cat.jpg");
  await user.keyboard("{Control>}a{/Control}");
  await user.keyboard("{Control>}d{/Control}");
  await user.keyboard("{Shift>}{Control>}d{/Control}{/Shift}");
  await user.keyboard("{Shift>}{Control>}i{/Control}{/Shift}");
  await vi.waitFor(() =>
    expect(
      calls
        .map((c) => c.cmd)
        .filter((c) => ["select_all", "deselect", "reselect", "invert_selection"].includes(c)),
    ).toEqual(["select_all", "deselect", "reselect", "invert_selection"]),
  );
});

test("Select > Transform Selection turns the selection's outline, then Enter resamples it", async () => {
  const user = open({ ...documentView(1, "cat.jpg", [layer(1, "Cat")]), selectionKey: 7 });
  await screen.findByText("cat.jpg");
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  await user.click(screen.getByText("Transform Selection", { selector: ".label" }));
  await vi.waitFor(() => expect(document.querySelectorAll(".handle")).toHaveLength(8));
  const svg = document.querySelector(".handle")!.closest("svg")!;
  await user.pointer({ keys: "[MouseRight]", target: svg, coords: { clientX: 5, clientY: 5 } });
  await user.click(screen.getByText("Flip Horizontal"));
  // Live: only the outline moves, nothing is sent to the layers.
  expect(sent("perform_live")).toEqual([]);
  await user.keyboard("{Enter}");
  // Flipped about the box's center (x = 60).
  await vi.waitFor(() =>
    expect(sent("transform_selection")).toEqual([{ documentId: 1, matrix: [-1, 0, 0, 1, 120, 0] }]),
  );
  expect(document.querySelectorAll(".handle")).toHaveLength(0);
});

test("Escape leaves the selection as it was", async () => {
  const user = open({ ...documentView(1, "cat.jpg", [layer(1, "Cat")]), selectionKey: 7 });
  await screen.findByText("cat.jpg");
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  await user.click(screen.getByText("Transform Selection", { selector: ".label" }));
  await vi.waitFor(() => expect(document.querySelectorAll(".handle")).toHaveLength(8));
  await user.keyboard("{Escape}");
  await vi.waitFor(() => expect(document.querySelectorAll(".handle")).toHaveLength(0));
  expect(sent("transform_selection")).toEqual([]);
  expect(sent("cancel_gesture")).toEqual([]);
});

test("Color Range samples the active layer, and keeps its settings for the next time", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat"]));
  const openColorRange = async () => {
    await user.click(screen.getByRole("menuitem", { name: "Select" }));
    await user.click(screen.getByText("Color Range…", { selector: ".label" }));
  };
  await openColorRange();
  await user.click(screen.getByRole("checkbox", { name: "Invert" }));
  await user.click(screen.getByRole("checkbox", { name: "Localized" }));
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(sent("color_range")).toHaveLength(1));
  expect(sent("color_range")[0]).toMatchObject({
    documentId: 1,
    request: { invert: true, localized: 100, layerId: 1 },
  });
  await openColorRange();
  expect(screen.getByRole("checkbox", { name: "Invert" })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "Localized" })).toBeChecked();
});

test("Select > Modify shows each amount live; OK applies it, Cancel takes it back", async () => {
  const user = open({ ...documentView(1, "cat.jpg", [layer(1, "Cat")]), selectionKey: 7 });
  await screen.findByText("cat.jpg");
  const expand = async () => {
    await user.click(screen.getByRole("menuitem", { name: "Select" }));
    await user.hover(screen.getByText("Modify", { selector: ".label" }));
    await user.click(screen.getByText("Expand…", { selector: ".label" }));
  };
  await expand();
  const field = screen.getByRole("spinbutton", { name: "Expand By:" });
  await user.clear(field);
  await user.type(field, "25");
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(sent("modify_selection").at(-1)).toEqual({
      documentId: 1,
      kind: "expand",
      amount: 25,
      live: false,
    }),
  );
  const shown = sent("modify_selection").filter((a) => a.live);
  expect(shown[0]).toMatchObject({ amount: 10 });
  expect(shown.at(-1)).toMatchObject({ amount: 25 });
  expect(sent("cancel_gesture")).toEqual([]);
  // Opened again at 25; Cancel takes the preview back.
  await expand();
  expect(screen.getByRole("spinbutton", { name: "Expand By:" })).toHaveValue(25);
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  await vi.waitFor(() => expect(sent("cancel_gesture")).toEqual([{ documentId: 1 }]));
});

test("Q enters Quick Mask: named in the tab and the options bar, with its own Add / Remove colors", async () => {
  localStorage.clear();
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  const foreground = () => screen.getByRole("button", { name: "Set foreground color" });
  // A drawing color, kept aside while Quick Mask is on.
  await user.keyboard("x");
  const drawing = foreground().getAttribute("style");
  await user.keyboard("q");
  await vi.waitFor(() =>
    expect(sent("set_quick_mask")).toEqual([{ documentId: 1, on: true, opacity: 50 }]),
  );
  await vi.waitFor(() => expect(screen.getByText("(Quick Mask)")).toBeInTheDocument());
  expect(screen.getByRole("status")).toHaveTextContent("Quick Mask");
  const add = screen.getByRole("button", { name: "Add" });
  const remove = screen.getByRole("button", { name: "Remove" });
  // Photoshop's default colors: black, the Brush removes.
  expect(remove).toHaveAttribute("aria-pressed", "true");
  await user.click(add);
  expect(add).toHaveAttribute("aria-pressed", "true");
  // X swaps the pair, D resets it.
  await user.keyboard("x");
  expect(remove).toHaveAttribute("aria-pressed", "true");
  await user.keyboard("x");
  await user.keyboard("d");
  expect(remove).toHaveAttribute("aria-pressed", "true");
  // Leaving it: the drawing colors as they were.
  await user.keyboard("q");
  await vi.waitFor(() => expect(screen.queryByText("(Quick Mask)")).not.toBeInTheDocument());
  expect(screen.queryByRole("button", { name: "Add" })).not.toBeInTheDocument();
  expect(foreground().getAttribute("style")).toBe(drawing);
});

test("Quick Mask's overlay opacity is sent as it changes, and kept for next time", async () => {
  localStorage.clear();
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("q");
  const label = await screen.findByText("Overlay opacity:");
  const field = label.parentElement!.querySelector("input[type=number]") as HTMLInputElement;
  await user.clear(field);
  await user.type(field, "80{Enter}");
  await vi.waitFor(() =>
    expect(sent("set_quick_mask").at(-1)).toEqual({ documentId: 1, on: true, opacity: 80 }),
  );
  expect(localStorage.getItem("slopshop.quickMaskOpacity")).toBe("80");
  localStorage.clear();
});

test("Select > Save Selection names it; Load Selection lists the saved ones and loads one", async () => {
  const user = open({
    ...documentView(1, "cat.jpg", [layer(1, "Cat")]),
    selectionKey: 7,
    savedSelections: [{ id: 4, name: "Hair" }],
  });
  await screen.findByText("cat.jpg");
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  await user.click(screen.getByText("Save Selection…", { selector: ".label" }));
  const dialog = screen.getByRole("dialog", { name: "Save Selection" });
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(sent("save_selection")).toEqual([{ documentId: 1, name: "Selection 1", replace: null }]),
  );
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  await user.hover(screen.getByText("Load Selection", { selector: ".label" }));
  const nested = () =>
    [...document.querySelectorAll(".dropdown.nested .label")].map((el) => el.textContent);
  await vi.waitFor(() => expect(nested()).toEqual(["Hair", "Selection 1"]));
  await user.click(screen.getByText("Hair", { selector: ".label" }));
  await vi.waitFor(() =>
    expect(sent("load_selection")).toEqual([{ documentId: 1, id: 4, mode: "replace" }]),
  );
});

test("Load Selection waits for a saved selection", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  const load = screen
    .getByText("Load Selection", { selector: ".label" })
    .closest("[role=menuitem]");
  expect(load).toHaveAttribute("aria-disabled", "true");
  const save = screen
    .getByText("Save Selection…", { selector: ".label" })
    .closest("[role=menuitem]");
  expect(save).toHaveAttribute("aria-disabled", "true");
});

test("Select and Mask opens on the selection, shows it live, and outputs it", async () => {
  const user = open({ ...documentView(1, "cat.jpg", [layer(1, "Cat")]), selectionKey: 7 });
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat"]));
  const selectAndMask = async () => {
    await user.click(screen.getByRole("menuitem", { name: "Select" }));
    await user.click(screen.getByText("Select and Mask…", { selector: ".label" }));
    await screen.findByText("Select and Mask", { selector: "header" });
  };
  await selectAndMask();
  expect(sent("refine_open")).toEqual([{ documentId: 1 }]);
  await vi.waitFor(() => {
    expect(sent("refine_view")).toEqual([{ documentId: 1, view: "overlay" }]);
    expect(sent("refine_preview")[0]).toMatchObject({ documentId: 1, live: true });
  });
  // Cancel: the selection as it was.
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  await vi.waitFor(() => expect(sent("refine_close")).toEqual([{ documentId: 1 }]));
  // Output to a new layer with a mask, on the active layer.
  await selectAndMask();
  await user.selectOptions(screen.getByRole("combobox", { name: "Output To:" }), "newLayer");
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(sent("refine_output")).toEqual([
      {
        documentId: 1,
        settings: { smooth: 0, feather: 0, contrast: 0, shift: 0 },
        layerId: 1,
        newLayer: true,
        nameFormat: "{name} copy",
      },
    ]),
  );
  // The output is kept for the next time; to the selection: the last preview, not live.
  await selectAndMask();
  await user.selectOptions(screen.getByRole("combobox", { name: "Output To:" }), "selection");
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(sent("refine_preview").at(-1)).toMatchObject({ documentId: 1, live: false }),
  );
});

test("Select and Mask's refine-edge brush: a stroke sent on release, then edge detection", async () => {
  const user = open({ ...documentView(1, "cat.jpg", [layer(1, "Cat")]), selectionKey: 7 });
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat"]));
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  await user.click(screen.getByText("Select and Mask…", { selector: ".label" }));
  await user.click(await screen.findByRole("button", { name: "Refine Edge Brush" }));
  const canvas = document.querySelector("svg.paint") as SVGSVGElement;
  expect(canvas).not.toBeNull();
  await fireEvent.pointerDown(canvas, { pointerId: 1, button: 0, clientX: 10, clientY: 10 });
  await fireEvent.pointerMove(canvas, { pointerId: 1, clientX: 20, clientY: 10 });
  expect(sent("refine_brush")).toEqual([]);
  await fireEvent.pointerUp(canvas, { pointerId: 1, clientX: 30, clientY: 10 });
  await vi.waitFor(() => expect(sent("ai_refine_base")).toHaveLength(1));
  const [stroke] = sent("refine_brush");
  expect(stroke).toMatchObject({ documentId: 1, size: 40, erase: false });
  expect((stroke.samples as number[][]).length).toBe(3);
  expect(sent("ai_refine_base")[0]).toMatchObject({ documentId: 1, radius: 16 });
  await vi.waitFor(() =>
    expect(sent("refine_preview").at(-1)).toMatchObject({ documentId: 1, live: true }),
  );
  // Alt erases.
  await fireEvent.pointerDown(canvas, {
    pointerId: 2,
    button: 0,
    clientX: 10,
    clientY: 10,
    altKey: true,
  });
  await fireEvent.pointerUp(canvas, { pointerId: 2, clientX: 10, clientY: 10 });
  await vi.waitFor(() => expect(sent("refine_brush")[1]).toMatchObject({ erase: true }));
});

test("Color Range shows its progress and the cancel button while it computes", async () => {
  let finish: (() => void) | null = null;
  respond("color_range", (_, doc) => new Promise((resolve) => (finish = () => resolve(doc))));
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat"]));
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  await user.click(screen.getByText("Color Range…", { selector: ".label" }));
  await user.click(screen.getByRole("checkbox", { name: "Invert" }));
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(sent("color_range")).toHaveLength(1));
  const task = sent("color_range")[0].task as number;
  expect(typeof task).toBe("number");
  // After a moment, the card with its cancel button.
  const cancel = await screen.findByRole("button", { name: "Cancel (Esc)" }, { timeout: 2000 });
  expect(screen.getByText(/Selecting the color range/)).toBeInTheDocument();
  await user.click(cancel);
  await vi.waitFor(() => expect(sent("ai_cancel")).toEqual([{ task }]));
  finish!();
});
