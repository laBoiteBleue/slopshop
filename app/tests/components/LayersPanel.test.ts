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
    locks: { transparency: false, pixels: false, position: false, all: false },
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
  const onloadpixels = vi.fn();
  const onfillcolor = vi.fn();
  const onstyle = vi.fn();
  const props = {
    onstyle,
    doc: documentOf(layers),
    onfillcolor,
    onedit,
    onlive: vi.fn(),
    ongestureend: vi.fn(() => Promise.resolve()),
    onloadpixels,
  };
  const view = render(LayersPanel, props);
  return {
    ...view,
    props,
    onedit,
    onfillcolor,
    onstyle,
    onloadpixels,
    user: userEvent.setup(),
  };
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

test("the Effects row's eye hides every effect, each keeping its own; hidden, they are dimmed", async () => {
  const style = withEffect(withEffect(null, "stroke", true), "dropShadow", true);
  const { onedit, user, rerender, props } = open([layer(1, "Logo", { style })]);
  await user.click(within(row("Logo")).getByRole("button", { expanded: false }));
  const effects = screen.getByText("Effects").closest("li")!;
  await user.click(within(effects).getByRole("button", { name: "Hide the effects" }));
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "setLayerStyle",
    id: 1,
    style: { ...style, effectsVisible: false },
  });
  // Hidden: the eye offers to show them, every effect dimmed, the fx mark kept.
  const hidden = { ...style, effectsVisible: false };
  await rerender({ ...props, doc: documentOf([layer(1, "Logo", { style: hidden })]) });
  const eye = within(screen.getByText("Effects").closest("li")!).getByRole("button", {
    name: "Show the effects",
  });
  expect(eye).toHaveAttribute("aria-pressed", "false");
  expect(screen.getByText("Stroke").closest("li")).toHaveClass("off");
  expect(within(row("Logo")).getByText("fx")).toBeInTheDocument();
  await user.click(eye);
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "setLayerStyle",
    id: 1,
    style: { ...style, effectsVisible: true },
  });
});

test("a right-click on the fx mark or the Effects row opens the layer style's menu", async () => {
  const style = withEffect(null, "stroke", true);
  const run = vi.fn();
  const { user, rerender, props } = open([layer(1, "Background"), layer(2, "Logo", { style })]);
  await rerender({
    ...props,
    contextMenu: [{ kind: "command", label: "Duplicate Layer", run: vi.fn() }],
    styleContextMenu: [{ kind: "command", label: "Copy Layer Style", run }],
  });
  await user.click(row("Background"));
  await fireEvent.contextMenu(within(row("Logo")).getByText("fx"));
  // The layer becomes the selection, as a right-click on its row does.
  expect(selectedNames()).toEqual(["Logo"]);
  expect(screen.queryByText("Duplicate Layer")).not.toBeInTheDocument();
  await user.click(screen.getByText("Copy Layer Style"));
  expect(run).toHaveBeenCalledOnce();
  await user.click(within(row("Logo")).getByRole("button", { expanded: false }));
  await fireEvent.contextMenu(screen.getByText("Effects"));
  expect(screen.getByText("Copy Layer Style")).toBeInTheDocument();
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

test("Ctrl+click on a thumbnail loads the layer's pixels as the selection, the layers staying", async () => {
  const masked = layer(6, "Masked", { mask: { enabled: true, contentKey: 6 } });
  const { onloadpixels, user } = open([...LAYERS, masked]);
  const thumb = (name: string, selector = ".thumb") =>
    row(name).querySelector(selector) as HTMLElement;
  await user.keyboard("[ControlLeft>]");
  await user.click(thumb("Sky"));
  expect(onloadpixels).toHaveBeenLastCalledWith(3, "pixels", "replace");
  await user.keyboard("[ShiftLeft>]");
  await user.click(thumb("Sea"));
  expect(onloadpixels).toHaveBeenLastCalledWith(4, "pixels", "add");
  await user.keyboard("[/ShiftLeft]");
  await user.click(thumb("Masked", ".mask-thumb"));
  expect(onloadpixels).toHaveBeenLastCalledWith(6, "mask", "replace");
  await user.keyboard("[/ControlLeft]");
  // The layers selected are as they were: Masked, the top one.
  expect(selectedNames()).toEqual(["Masked"]);
});

const VECTOR_MASK = {
  enabled: true,
  inverted: false,
  feather: 0,
  geometry: {
    kind: "ellipse" as const,
    center: [50, 40] as [number, number],
    radii: [20, 10] as [number, number],
  },
  // The ellipse as anchors (the engine sends them), a diamond here.
  livePath: {
    kind: "path" as const,
    rule: "nonzero" as const,
    subpaths: [
      {
        start: [30, 40] as [number, number],
        segments: [
          [50, 30],
          [70, 40],
          [50, 50],
        ] as [number, number][],
        closed: true,
      },
    ],
  },
};

test("a vector mask shows its thumbnail, targeted by a click, toggled by Shift+click", async () => {
  const masked = layer(6, "Masked", {
    mask: { enabled: true, contentKey: 6 },
    vectorMask: VECTOR_MASK,
  });
  const { user, onedit, component } = open([...LAYERS, masked]);
  const vector = () => row("Masked").querySelector(".vector-mask-thumb") as HTMLElement;
  expect(vector()).toHaveAccessibleName(
    "Vector mask: click to draw and edit it with the Pen, Shift+click to disable or enable it",
  );
  // Its outline in the document, white on gray.
  expect(vector().querySelector("path.shown")).not.toBeNull();
  expect(component.targetsVectorMask()).toBe(false);
  await user.click(vector());
  expect(vector()).toHaveClass("targeted");
  expect(row("Masked").querySelector(".thumb")).not.toHaveClass("targeted");
  expect(component.targetsVectorMask()).toBe(true);
  expect(component.paintsMask()).toBe(false);
  // The pixel mask's thumbnail takes the target back.
  await user.click(row("Masked").querySelector(".mask-thumb") as HTMLElement);
  expect(vector()).not.toHaveClass("targeted");
  expect(component.targetsVectorMask()).toBe(false);
  expect(component.paintsMask()).toBe(true);
  await user.keyboard("[ShiftLeft>]");
  await user.click(vector());
  await user.keyboard("[/ShiftLeft]");
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "setVectorMaskEnabled",
    id: 6,
    enabled: false,
  });
});

test("a disabled vector mask is crossed out; Ctrl+click loads it as the selection", async () => {
  const masked = layer(6, "Masked", { vectorMask: { ...VECTOR_MASK, enabled: false } });
  const { user, onloadpixels, component } = open([...LAYERS, masked]);
  const vector = row("Masked").querySelector(".vector-mask-thumb") as HTMLElement;
  expect(vector).toHaveClass("disabled");
  await user.keyboard("[ControlLeft>]");
  await user.click(vector);
  await user.keyboard("[/ControlLeft]");
  expect(onloadpixels).toHaveBeenLastCalledWith(6, "vectorMask", "replace");
  // Loading does not target it.
  expect(component.targetsVectorMask()).toBe(false);
  // New vector masks are targeted.
  component.targetVectorMasks([6]);
  await tick();
  expect(vector).toHaveClass("targeted");
});

const LOCKED = { transparency: false, pixels: false, position: false, all: false };

test("the lock buttons set a lock on the selected layers, or clear it from all", async () => {
  const layers = [
    layer(1, "Background", { locks: { ...LOCKED, position: true } }),
    layer(5, "Text"),
  ];
  const { user, onedit } = open(layers);
  // Text alone: Lock Position is off, a click sets it.
  const position = screen.getByRole("button", { name: "Lock position" });
  expect(position).toHaveAttribute("aria-pressed", "false");
  await user.click(position);
  expect(onedit).toHaveBeenLastCalledWith(1, { kind: "setLayerLocks", ids: [5], position: true });
  // Both selected: Background has it, Text not: mixed, and a click sets it on both.
  await user.keyboard("[ControlLeft>]");
  await user.click(row("Background"));
  await user.keyboard("[/ControlLeft]");
  expect(position).toHaveAttribute("aria-pressed", "mixed");
  await user.click(position);
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "setLayerLocks",
    ids: [1, 5],
    position: true,
  });
  // Background alone, where it is set: a click clears it.
  await user.click(row("Background"));
  expect(position).toHaveAttribute("aria-pressed", "true");
  await user.click(position);
  expect(onedit).toHaveBeenLastCalledWith(1, {
    kind: "setLayerLocks",
    ids: [1],
    position: false,
  });
  for (const name of ["Lock transparent pixels", "Lock image pixels", "Lock all"]) {
    expect(screen.getByRole("button", { name })).toHaveAttribute("aria-pressed", "false");
  }
});

test("locked rows show Photoshop's padlock, a group's dimmed on its layers", () => {
  open([
    layer(1, "Background", { locks: { ...LOCKED, all: true } }),
    layer(2, "Group", {
      kind: "group",
      locks: { ...LOCKED, pixels: true },
      children: [layer(3, "Sky")],
    }),
    layer(5, "Text"),
  ]);
  const mark = (name: string) => row(name).querySelector(".lock-mark");
  expect(mark("Background")).toHaveAttribute("data-lock", "full");
  expect(mark("Background")).toHaveAttribute("title", "Fully locked");
  expect(mark("Group")).toHaveAttribute("data-lock", "partial");
  expect(mark("Group")).not.toHaveClass("inherited");
  expect(mark("Sky")).toHaveClass("inherited");
  expect(mark("Sky")).toHaveAttribute("title", "Locked by its group");
  expect(mark("Text")).toBeNull();
});

test("Lock All grays the blend mode and the opacity of the selected layer", async () => {
  const { user } = open([
    layer(1, "Background", { locks: { ...LOCKED, all: true } }),
    layer(5, "Text"),
  ]);
  const blend = screen.getByRole("combobox", { name: "Blend mode" });
  expect(blend).toBeEnabled();
  await user.click(row("Background"));
  expect(blend).toBeDisabled();
  expect(screen.getByRole("slider", { name: "Opacity" })).toBeDisabled();
});
