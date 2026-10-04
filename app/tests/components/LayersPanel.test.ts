import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { render, screen, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { DocumentView, LayerView } from "../../src/lib/engine";
import LayersPanel from "../../src/lib/LayersPanel.svelte";
import { PLAIN, withEffect } from "../../src/lib/layerStyle";

// Thumbnails ask the engine: their answers never come (nothing to draw in jsdom).
beforeEach(() => mockIPC(() => new Promise(() => {})));
afterEach(() => clearMocks());

function layer(id: number, name: string, changes: Partial<LayerView> = {}): LayerView {
  return {
    id,
    name,
    visible: true,
    opacity: 1,
    kind: "raster",
    swatch: [0, 0, 0, 0],
    blendMode: "normal",
    contentKey: id,
    hasAlpha: true,
    mask: null,
    children: [],
    passThrough: false,
    clipped: false,
    transform: [1, 0, 0, 1, 0, 0],
    painted: false,
    entries: [],
    adjustment: null,
    ...changes,
  };
}

function documentOf(layers: LayerView[]): DocumentView {
  return {
    id: 1,
    name: null,
    width: 100,
    height: 100,
    workingSpace: "srgb",
    blendSpace: "perceptual",
    resolution: 72,
    revision: 0,
    canUndo: false,
    canRedo: false,
    layers,
    warnings: [],
    path: null,
    dirty: false,
    selectionKey: null,
    canReselect: false,
    quickMask: false,
    quickMaskOpacity: 50,
    savedSelections: [],
  } as DocumentView;
}

/** Bottom to top: Background, a group holding Sky and Sea, then Text. */
const LAYERS = [
  layer(1, "Background"),
  layer(2, "Group", {
    kind: "group",
    children: [layer(3, "Sky"), layer(4, "Sea")],
  }),
  layer(5, "Text"),
];

function open(layers = LAYERS) {
  const onedit = vi.fn(() => Promise.resolve());
  const onfillcolor = vi.fn();
  const onstyle = vi.fn();
  const props = {
    onstyle,
    doc: documentOf(layers),
    onfillcolor,
    onedit,
    onlive: vi.fn(),
    ongestureend: vi.fn(() => Promise.resolve()),
  };
  const view = render(LayersPanel, props);
  return { ...view, props, onedit, onfillcolor, onstyle, user: userEvent.setup() };
}

/** The row of the layer named `name`. */
const row = (name: string) => {
  const element = screen.getByText(name).closest("li");
  if (!element) throw new Error(`no row for ${name}`);
  return element;
};
const selectedNames = () =>
  [...document.querySelectorAll("li.selected .name")].map((el) => el.textContent?.trim());

test("layers are listed top to bottom, the top one selected", () => {
  open();
  const names = [...document.querySelectorAll("li .name")].map((el) => el.textContent?.trim());
  expect(names).toEqual(["Text", "Group", "Sea", "Sky", "Background"]);
  expect(selectedNames()).toEqual(["Text"]);
});

test("a click selects a layer, Ctrl+click adds one, Shift+click a range", async () => {
  const { user } = open();
  await user.click(row("Sea"));
  expect(selectedNames()).toEqual(["Sea"]);
  await user.keyboard("[ControlLeft>]");
  await user.click(row("Background"));
  await user.keyboard("[/ControlLeft]");
  expect(selectedNames()).toEqual(["Sea", "Background"]);
  await user.click(row("Text"));
  await user.keyboard("[ShiftLeft>]");
  await user.click(row("Sky"));
  await user.keyboard("[/ShiftLeft]");
  expect(selectedNames()).toEqual(["Text", "Group", "Sea", "Sky"]);
});

test("the eye hides its layer, or the whole selection it is part of, in one edit", async () => {
  const { onedit, user } = open();
  await user.click(within(row("Sky")).getByTitle("Hide layer"));
  expect(onedit).toHaveBeenLastCalledWith(1, { kind: "setLayerVisible", id: 3, visible: false });
  await user.keyboard("[ControlLeft>]");
  await user.click(row("Background"));
  await user.keyboard("[/ControlLeft]");
  await user.click(within(row("Background")).getByTitle("Hide layer"));
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "batch",
    edits: [
      { kind: "setLayerVisible", id: 1, visible: false },
      { kind: "setLayerVisible", id: 5, visible: false },
    ],
  });
});

test("a group folds away its layers", async () => {
  const { user } = open();
  await user.click(within(row("Group")).getByRole("button", { expanded: true }));
  expect(screen.queryByText("Sky")).not.toBeInTheDocument();
  expect(screen.queryByText("Sea")).not.toBeInTheDocument();
});

test("a double-click renames, Enter commits, Escape cancels", async () => {
  const { onedit, user } = open();
  await user.dblClick(screen.getByText("Sea"));
  const field = screen.getByDisplayValue("Sea");
  await user.clear(field);
  await user.type(field, "Ocean{Enter}");
  expect(onedit).toHaveBeenLastCalledWith(1, { kind: "renameLayer", id: 4, name: "Ocean" });
  onedit.mockClear();
  await user.dblClick(screen.getByText("Sky"));
  await user.type(screen.getByDisplayValue("Sky"), "Cloud{Escape}");
  expect(onedit).not.toHaveBeenCalled();
  expect(screen.getByText("Sky")).toBeInTheDocument();
});

test("the delete button removes a group with its layers in one edit", async () => {
  const { onedit, user } = open();
  await user.click(row("Group"));
  await user.keyboard("[ShiftLeft>]");
  await user.click(row("Sky"));
  await user.keyboard("[/ShiftLeft]");
  await user.click(screen.getByTitle("Delete layers"));
  expect(onedit).toHaveBeenLastCalledWith(1, { kind: "removeLayer", id: 2 });
});

test("the + button adds an empty layer above the active one, and no color is asked", async () => {
  const { onedit, user } = open();
  await user.click(row("Sea"));
  await user.click(screen.getByTitle("New layer"));
  expect(onedit).toHaveBeenLastCalledWith(
    1,
    expect.objectContaining({ kind: "addEmptyLayer", parent: 2 }),
  );
  expect(document.querySelector('input[type="color"]')).toBeNull();
});

test("the blend mode and the opacity field apply to every selected layer", async () => {
  const { onedit, user } = open();
  await user.keyboard("[ControlLeft>]");
  await user.click(row("Background"));
  await user.keyboard("[/ControlLeft]");
  await user.selectOptions(screen.getByRole("combobox", { name: "Blend mode" }), "multiply");
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "batch",
    edits: [
      { kind: "setLayerBlendMode", id: 1, mode: "multiply" },
      { kind: "setLayerBlendMode", id: 5, mode: "multiply" },
    ],
  });
  const opacity = screen.getByRole("spinbutton", { name: "Opacity" });
  await user.clear(opacity);
  await user.type(opacity, "40");
  // The field commits on `change`: when it loses the focus.
  await user.tab();
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "batch",
    edits: [
      { kind: "setLayerOpacity", id: 1, opacity: 0.4 },
      { kind: "setLayerOpacity", id: 5, opacity: 0.4 },
    ],
  });
});

test("layers added to the document become the selection", async () => {
  const { rerender, props } = open();
  await rerender({ ...props, doc: documentOf([...LAYERS, layer(6, "Pasted")]) });
  expect(selectedNames()).toEqual(["Pasted"]);
});

test("hidden layers (Image > Adjustments' previews) are never selected, and the selection stays", async () => {
  const { rerender, props, user } = open();
  await user.click(row("Sea"));
  // The previews arrive already hidden, then go: the selection does not move.
  const preview = layer(6, "levels", { kind: "adjustment", clipped: true });
  await rerender({ ...props, doc: documentOf([...LAYERS, preview]), hidden: [6] });
  expect(selectedNames()).toEqual(["Sea"]);
  expect(screen.queryByText("levels")).not.toBeInTheDocument();
  await rerender({ ...props, doc: documentOf(LAYERS), hidden: [] });
  expect(selectedNames()).toEqual(["Sea"]);
});

test("a new fill layer goes above the active layer, in its group, selected once added", async () => {
  const { component, onedit, props, rerender, user } = open();
  await user.click(row("Sky"));
  component.addFill("#ff0000");
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "addFillLayer",
    name: "Color Fill 1",
    color: [1, 0, 0, 1],
    parent: 2,
    index: 1,
  });
  const fill = layer(6, "Color Fill 1", { kind: "fill" });
  const group = { ...LAYERS[1], children: [layer(3, "Sky"), fill, layer(4, "Sea")] };
  await rerender({ ...props, doc: documentOf([LAYERS[0], group, LAYERS[2]]) });
  await vi.waitFor(() => expect(selectedNames()).toEqual(["Color Fill 1"]));
});

test("a double-click on a fill layer's thumbnail asks for its color, not on a pixel layer's", async () => {
  const fill = layer(6, "Color Fill 1", { kind: "fill", swatch: [1, 0, 0, 1] });
  const { onfillcolor, user } = open([...LAYERS, fill]);
  await user.dblClick(row("Text").querySelector(".thumb")!);
  expect(onfillcolor).not.toHaveBeenCalled();
  await user.dblClick(row("Color Fill 1").querySelector(".thumb")!);
  expect(onfillcolor).toHaveBeenCalledWith(fill);
});

test("a styled layer shows fx; its effects unfold below it, each eye turns one off", async () => {
  const style = withEffect(withEffect(null, "stroke", true), "dropShadow", true);
  const { onedit, user } = open([layer(1, "Background"), layer(2, "Logo", { style })]);
  expect(within(row("Logo")).getByText("fx")).toBeInTheDocument();
  expect(within(row("Background")).queryByText("fx")).not.toBeInTheDocument();
  await user.click(within(row("Logo")).getByRole("button", { expanded: false }));
  expect(screen.getByText("Effects")).toBeInTheDocument();
  expect(screen.getByText("Stroke")).toBeInTheDocument();
  await user.click(
    within(screen.getByText("Drop Shadow").closest("li")!).getByRole("button", {
      name: "Hide the effect",
    }),
  );
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "setLayerStyle",
    id: 2,
    style: { ...style, dropShadow: { ...style.dropShadow, enabled: false } },
  });
});

test("a double-click on a pixel layer's row opens Layer Style; Fill sets its Fill Opacity", async () => {
  const { onedit, onstyle, user } = open([layer(1, "Background"), layer(2, "Logo")]);
  await user.click(row("Logo"));
  await user.dblClick(row("Logo"));
  expect(onstyle).toHaveBeenCalledWith(expect.objectContaining({ id: 2 }), "blending");
  const fill = screen.getByRole("spinbutton", { name: "Fill" });
  await user.clear(fill);
  await user.type(fill, "30");
  await user.tab();
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "setLayerStyle",
    id: 2,
    style: { ...PLAIN, fillOpacity: 0.3 },
  });
});

test("a double-click on a group's row opens Layer Style, not on an adjustment layer's", async () => {
  const curves = layer(6, "Curves", { kind: "adjustment" });
  const { onstyle, user } = open([...LAYERS, curves]);
  await user.dblClick(row("Group"));
  expect(onstyle).toHaveBeenCalledWith(expect.objectContaining({ id: 2 }), "blending");
  onstyle.mockClear();
  await user.dblClick(row("Curves"));
  expect(onstyle).not.toHaveBeenCalled();
});
