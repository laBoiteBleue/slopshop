import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { tick } from "svelte";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { DocumentView, LayerView } from "../../src/lib/engine";
import LayersPanel from "../../src/lib/LayersPanel.svelte";
import { PLAIN, withEffect } from "../../src/lib/layerStyle";
import { LayersUi } from "../../src/lib/layersUi.svelte";

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
    guides: [],
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

test("among several selected layers, the active one is marked", async () => {
  const { user } = open();
  const active = () => document.querySelector("li[aria-current='true'] .name")?.textContent;
  const marked = () =>
    [...document.querySelectorAll("li.active .name")].map((el) => el.textContent);
  expect(active()).toBe("Text");
  // Alone, nothing to tell apart.
  expect(marked()).toEqual([]);
  await user.click(row("Sea"));
  await user.keyboard("[ControlLeft>]");
  await user.click(row("Background"));
  await user.keyboard("[/ControlLeft]");
  expect(active()).toBe("Background");
  expect(marked()).toEqual(["Background"]);
});

test("layers picked in the image follow the panel's rules, the groups unfolding", async () => {
  const { component, user } = open();
  await user.click(within(row("Group")).getByTitle("Collapse group"));
  expect(screen.queryByText("Sea")).toBeNull();
  const pick = async (hit: number, add: boolean) => {
    const picked = component.pickInImage(hit, add);
    await tick();
    return picked;
  };
  // A layer alone, inside a folded group.
  expect(await pick(4, false)).toEqual({ collapse: false, moves: true });
  expect(selectedNames()).toEqual(["Sea"]);
  // Shift adds one, active; again takes it out, without moving.
  expect(await pick(1, true)).toEqual({ collapse: false, moves: true });
  expect(selectedNames()).toEqual(["Sea", "Background"]);
  expect(await pick(5, true)).toEqual({ collapse: false, moves: true });
  expect(await pick(5, true)).toEqual({ collapse: false, moves: false });
  expect(selectedNames()).toEqual(["Sea", "Background"]);
  // A press on one of them keeps them all, for a drag.
  expect(await pick(4, false)).toEqual({ collapse: true, moves: true });
  expect(selectedNames()).toEqual(["Sea", "Background"]);
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

test("a panel made again for the same document keeps its selection, folds and scroll", async () => {
  const ui = new LayersUi();
  const props = {
    doc: documentOf(LAYERS),
    ui,
    onedit: vi.fn(() => Promise.resolve()),
    onlive: vi.fn(),
    ongestureend: vi.fn(() => Promise.resolve()),
  };
  const first = render(LayersPanel, props);
  const user = userEvent.setup();
  await user.click(row("Background"));
  await user.click(within(row("Group")).getByRole("button", { expanded: true }));
  document.querySelector("ul")!.scrollTop = 40;
  await fireEvent.scroll(document.querySelector("ul")!);
  first.unmount();
  // Another tab meanwhile, then this one again.
  render(LayersPanel, props);
  expect(selectedNames()).toEqual(["Background"]);
  expect(screen.queryByText("Sky")).not.toBeInTheDocument();
  expect(within(row("Group")).getByRole("button", { expanded: false })).toBeInTheDocument();
  expect(document.querySelector("ul")!.scrollTop).toBe(40);
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

test("a layer's stack unfolds below it: each entry has an eye, an edit icon when it has settings", async () => {
  const levels = {
    id: "levels" as const,
    values: [0, 1, 1, 0, 1],
    curves: null,
    curveSamples: null,
    gradient: null,
  };
  const none = { filter: null, filterSteps: [] };
  const entries = [
    { kind: "paint" as const, adjustment: null, count: 1, hidden: false, steps: [], ...none },
    {
      kind: "effect" as const,
      adjustment: "invert" as const,
      count: 1,
      hidden: false,
      steps: [{ ...levels, id: "invert" as const, values: [] }],
      ...none,
    },
    {
      kind: "effect" as const,
      adjustment: "levels" as const,
      count: 2,
      hidden: true,
      steps: [levels, levels],
      ...none,
    },
  ];
  const onentryedit = vi.fn();
  const { onedit, user, rerender, props } = open([
    layer(1, "Background"),
    layer(2, "Photo", { entries }),
  ]);
  await rerender({ ...props, onentryedit });
  await user.click(within(row("Photo")).getByRole("button", { expanded: false }));
  const entry = (name: string) => screen.getByText(name).closest("li")!;
  // Newest on top; a hidden entry dimmed, its eye offering to show it.
  expect(
    [...document.querySelectorAll("li.entry .entry-name")].map((el) => el.textContent?.trim()),
  ).toEqual(["Levels ×2", "Invert", "Paint"]);
  expect(entry("Levels ×2")).toHaveClass("off");
  await user.click(within(entry("Levels ×2")).getByRole("button", { name: "Show" }));
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "setStackEntry",
    id: 2,
    index: 2,
    hidden: false,
  });
  await user.click(within(entry("Paint")).getByRole("button", { name: "Hide" }));
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "setStackEntry",
    id: 2,
    index: 0,
    hidden: true,
  });
  // Settings to edit: Levels, not Invert nor paint.
  expect(within(entry("Invert")).queryByRole("button", { name: "Edit Settings…" })).toBeNull();
  expect(within(entry("Paint")).queryByRole("button", { name: "Edit Settings…" })).toBeNull();
  await user.click(within(entry("Levels ×2")).getByRole("button", { name: "Edit Settings…" }));
  expect(onentryedit).toHaveBeenLastCalledWith(expect.objectContaining({ id: 2 }), 2);
  onentryedit.mockClear();
  // A double-click edits nothing: only the icon and the right-click menu do.
  await user.dblClick(entry("Levels ×2"));
  expect(onentryedit).not.toHaveBeenCalled();
  // The right-click menu: edit, show or hide, delete.
  await user.pointer({ keys: "[MouseRight]", target: entry("Levels ×2") });
  const menu = screen.getByRole("menu");
  expect(
    within(menu)
      .getAllByRole("menuitem")
      .map((el) => el.textContent?.trim()),
  ).toEqual(["Edit Settings…", "Show", "Delete"]);
});

test("a double-click on a pixel layer's row opens Layer Style; Fill sets its Fill Opacity", async () => {
  const style = withEffect(null, "stroke", true);
  const { onedit, onstyle, user } = open([layer(1, "Background"), layer(2, "Logo", { style })]);
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
    style: { ...style, fillOpacity: 0.3 },
  });
});

test("Fill shows only where it means something: an effect, or a Fill already set", async () => {
  const { user } = open([
    layer(1, "Plain"),
    layer(2, "Styled", { style: withEffect(null, "dropShadow", false) }),
    layer(3, "Faded", { style: { ...PLAIN, fillOpacity: 0.4 } }),
    layer(4, "Curves", { kind: "adjustment", style: withEffect(null, "stroke", true) }),
  ]);
  const fill = () => screen.queryByRole("spinbutton", { name: "Fill" });
  await user.click(row("Plain"));
  expect(fill()).toBeNull();
  // Opacity stays, whatever the layer.
  expect(screen.getByRole("slider", { name: "Opacity" })).toBeInTheDocument();
  await user.click(row("Styled"));
  expect(fill()).toHaveValue(100);
  await user.click(row("Faded"));
  expect(fill()).toHaveValue(40);
  await user.click(row("Curves"));
  expect(fill()).toBeNull();
});

test("an effect's trash deletes it, its settings with it", async () => {
  const style = withEffect(withEffect(null, "stroke", true), "dropShadow", true);
  const { onedit, user } = open([layer(1, "Background"), layer(2, "Logo", { style })]);
  await user.click(within(row("Logo")).getByRole("button", { expanded: false }));
  await user.click(
    within(screen.getByText("Drop Shadow").closest("li")!).getByRole("button", {
      name: "Delete the effect",
    }),
  );
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "setLayerStyle",
    id: 2,
    style: { ...style, dropShadow: null },
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

test("a layer being baked shows its thumbnail, dimmed, in place of its kind's", () => {
  open([layer(1, "Background"), layer(2, "Merged", { kind: "group", baking: true })]);
  const thumb = row("Merged").querySelector(".thumb.baking");
  expect(thumb?.querySelector("canvas")).not.toBeNull();
  expect(row("Merged").querySelector(".fold")).toBeNull();
});
