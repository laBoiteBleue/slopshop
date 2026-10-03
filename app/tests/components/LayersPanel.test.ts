import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { render, screen, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { DocumentView, LayerView } from "../../src/lib/engine";
import LayersPanel from "../../src/lib/LayersPanel.svelte";

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
  const props = {
    doc: documentOf(layers),
    onedit,
    onlive: vi.fn(),
    ongestureend: vi.fn(() => Promise.resolve()),
  };
  const view = render(LayersPanel, props);
  return { ...view, props, onedit, user: userEvent.setup() };
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
