import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import App from "../../src/App.svelte";
import type { DocumentView, EditRequest, LayerView } from "../../src/lib/engine";

// The whole UI on a fake engine: a few documents in memory behind the IPC, answering the
// commands these tests go through (anything else answers nothing).

class FixedSizeObserver {
  constructor(private readonly callback: ResizeObserverCallback) {}
  observe(target: Element) {
    // With its target: Svelte's size bindings (the selection outline's) key entries by it.
    const entry = {
      target,
      contentRect: { width: 200, height: 100 },
    } as unknown as ResizeObserverEntry;
    this.callback([entry], this as unknown as ResizeObserver);
  }
  unobserve() {}
  disconnect() {}
}

function layer(id: number, name: string): LayerView {
  return {
    id,
    name,
    visible: true,
    opacity: 1,
    kind: "raster",
    swatch: [0, 0, 0, 0],
    blendMode: "normal",
    contentKey: id,
    hasAlpha: false,
    mask: null,
    children: [],
    passThrough: false,
    clipped: false,
    transform: [1, 0, 0, 1, 0, 0],
    painted: false,
    entries: [],
    adjustment: null,
  };
}

function documentView(id: number, name: string | null, layers: LayerView[]): DocumentView {
  return {
    id,
    name,
    width: 400,
    height: 300,
    workingSpace: "srgb",
    blendSpace: "perceptual",
    resolution: 72,
    revision: 0,
    canUndo: true,
    canRedo: false,
    layers,
    warnings: [],
    path: null,
    dirty: false,
    selectionKey: null,
    canReselect: false,
    quickMask: false,
    quickMaskOpacity: 50,
  } as DocumentView;
}

/** A 1 × 1 frame of `documentId` at 100% (see `parseFrame` in engine.ts). */
function frame(documentId: number): ArrayBuffer {
  const buffer = new ArrayBuffer(56 + 4);
  const view = new DataView(buffer);
  view.setUint32(0, 2, true);
  view.setUint32(4, 1, true);
  view.setUint32(8, 1, true);
  view.setFloat64(24, 1, true);
  view.setUint32(36, documentId, true);
  return buffer;
}

let documents: DocumentView[];
/** Every command the UI sent, with its arguments. */
let calls: { cmd: string; args: Record<string, unknown> }[];

const sent = (cmd: string) => calls.filter((c) => c.cmd === cmd).map((c) => c.args);

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", FixedSizeObserver);
  documents = [];
  calls = [];
  mockIPC(
    (cmd, payload) => {
      const args = (payload ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args });
      const find = () => documents.find((d) => d.id === args.documentId) as DocumentView;
      switch (cmd) {
        case "presenter_mode":
          return "frames";
        case "documents":
          return documents;
        case "openings":
        case "open_failures":
        case "recent_files":
          return [];
        case "clipboard_contents":
          return "nothing";
        case "render_view":
          return frame(args.documentId as number);
        case "view":
          return { zoom: 1, origin: [0, 0], fit: true };
        case "new_document": {
          const doc = documentView(documents.length + 1, args.name as string | null, [
            layer(1, args.layerName as string),
          ]);
          documents.push(doc);
          return doc;
        }
        case "set_quick_mask": {
          const doc = find();
          doc.quickMask = args.on as boolean;
          doc.quickMaskOpacity = args.opacity as number;
          return { ...doc };
        }
        case "selection_bounds":
          return { left: 10, top: 20, right: 110, bottom: 70 };
        case "undo":
          return { ...find(), revision: find().revision + 1, canUndo: false };
        case "perform": {
          const edit = args.edit as EditRequest;
          const doc = find();
          if (edit.kind === "removeLayer") {
            doc.layers = doc.layers.filter((l) => l.id !== edit.id);
          }
          return { ...doc, revision: doc.revision + 1 };
        }
      }
      return null;
    },
    { shouldMockEvents: true },
  );
  mockWindows("main");
});
afterEach(async () => {
  // Unmounted first: the app stops listening to the engine's events (some once a promise
  // settles), which must still be mocked then.
  cleanup();
  await new Promise((resolve) => setTimeout(resolve, 0));
  clearMocks();
  vi.unstubAllGlobals();
});

function open(...docs: DocumentView[]) {
  documents = docs;
  render(App);
  return userEvent.setup();
}

/** The names in the Layers panel, top to bottom. */
const layerNames = () =>
  [...document.querySelectorAll("li .name")].map((el) => el.textContent?.trim());

test("it starts on the welcome page, with the menus", async () => {
  open();
  expect(await screen.findByText("Open an image or create a document")).toBeInTheDocument();
  expect(screen.getByRole("menuitem", { name: "File" })).toBeInTheDocument();
});

test("documents open at startup are tabs; the last one shows its layers and is drawn", async () => {
  open(
    documentView(1, "cat.jpg", [layer(1, "Cat")]),
    documentView(2, "dog.png", [layer(1, "Background"), layer(2, "Dog")]),
  );
  expect(await screen.findByText("cat.jpg")).toBeInTheDocument();
  expect(screen.getByText("dog.png")).toBeInTheDocument();
  await vi.waitFor(() => expect(layerNames()).toEqual(["Dog", "Background"]));
  await vi.waitFor(() =>
    expect(sent("render_view")).toContainEqual(expect.objectContaining({ documentId: 2 })),
  );
});

test("File > New creates an untitled document in a new tab", async () => {
  const user = open();
  await user.click(await screen.findByRole("button", { name: "New document…" }));
  const dialog = screen.getByRole("dialog", { name: "New" });
  await user.selectOptions(within(dialog).getByRole("combobox", { name: "Preset" }), "hd");
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(sent("new_document")).toHaveLength(1));
  expect(sent("new_document")[0]).toMatchObject({ name: null, width: 1920, height: 1080 });
  await vi.waitFor(() => expect(layerNames()).toHaveLength(1));
});

test("Ctrl+Z undoes in the active document", async () => {
  const user = open(documentView(4, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("{Control>}z{/Control}");
  await vi.waitFor(() => expect(sent("undo")).toEqual([{ documentId: 4 }]));
});

test("a tool's letter picks it in the toolbar", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("b");
  expect(screen.getByRole("button", { name: "Brush Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("Delete removes the selected layer, without a selection of pixels", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Background"), layer(2, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Background"]));
  await user.keyboard("{Delete}");
  await vi.waitFor(() =>
    expect(sent("perform")).toContainEqual({
      documentId: 1,
      edit: { kind: "removeLayer", id: 2 },
    }),
  );
  await vi.waitFor(() => expect(layerNames()).toEqual(["Background"]));
});

test("the panels open at the width saved last, and their left edge resizes them", async () => {
  localStorage.setItem("slopshop.panelWidth", "420");
  open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  const edge = await screen.findByRole("separator", { name: "Resize the panels" });
  const main = document.querySelector("main") as HTMLElement;
  expect(main.style.getPropertyValue("--panel-width")).toBe("420px");
  await fireEvent.pointerDown(edge, { pointerId: 1, button: 0, clientX: 600 });
  await fireEvent.pointerMove(edge, { pointerId: 1, clientX: 550 });
  await fireEvent.pointerUp(edge, { pointerId: 1, clientX: 550 });
  expect(main.style.getPropertyValue("--panel-width")).toBe("470px");
  expect(localStorage.getItem("slopshop.panelWidth")).toBe("470");
  localStorage.clear();
});

/** The Layers panel's row of the layer named `name`. */
const row = (name: string) => screen.getByText(name, { selector: "li .name" }).closest("li")!;

test("the Layer menu: New, the fill and adjustment layers, then the commands on the layers", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  const labels = [...document.querySelectorAll(".dropdown > .item > .label")].map(
    (el) => el.textContent,
  );
  expect(labels.slice(0, 3)).toEqual(["New", "New Fill Layer", "New Adjustment Layer"]);
  expect(labels).toContain("Arrange");
  await user.hover(screen.getByText("New", { selector: ".label" }));
  const nested = [...document.querySelectorAll(".dropdown.nested .label")].map(
    (el) => el.textContent,
  );
  expect(nested).toEqual(["Layer", "Group", "Layer via Copy", "Layer via Cut"]);
});

test("Ctrl+] brings the selected layer forward, and does nothing at the top", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Background"), layer(2, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Background"]));
  await user.keyboard("{Control>}]{/Control}");
  await user.click(row("Background"));
  await user.keyboard("{Control>}]{/Control}");
  await vi.waitFor(() =>
    expect(sent("perform")).toEqual([
      { documentId: 1, edit: { kind: "arrangeLayers", ids: [1], arrange: "forward" } },
    ]),
  );
});

test("Shift+Ctrl+G ungroups every selected group", async () => {
  const group = (id: number, name: string, inner: LayerView) => ({
    ...layer(id, name),
    kind: "group" as const,
    children: [inner],
  });
  const user = open(
    documentView(1, "cat.jpg", [
      group(1, "Back", layer(2, "Sky")),
      group(3, "Front", layer(4, "Cat")),
    ]),
  );
  await vi.waitFor(() => expect(layerNames()).toContain("Back"));
  await user.click(row("Front"));
  await user.keyboard("[ControlLeft>]");
  await user.click(row("Back"));
  await user.keyboard("[/ControlLeft]");
  await user.keyboard("{Shift>}{Control>}g{/Control}{/Shift}");
  await vi.waitFor(() =>
    expect(sent("perform")).toEqual([{ documentId: 1, edit: { kind: "ungroup", ids: [1, 3] } }]),
  );
});

test("the clipping command says Release only when every selected layer is clipped", async () => {
  const user = open(
    documentView(1, "cat.jpg", [
      layer(1, "Background"),
      { ...layer(2, "Shade"), clipped: true },
      layer(3, "Cat"),
    ]),
  );
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Shade", "Background"]));
  await user.click(row("Shade"));
  await user.keyboard("[ControlLeft>]");
  await user.click(row("Cat"));
  await user.keyboard("[/ControlLeft]");
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  expect(screen.getByText("Create Clipping Mask")).toBeInTheDocument();
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  await user.click(row("Shade"));
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  expect(screen.getByText("Release Clipping Mask")).toBeInTheDocument();
});

/** The labels of the open menu's first level, separators as "—". */
const menuLabels = () =>
  [...document.querySelectorAll(".dropdown:not(.nested) > *")].map((el) =>
    el.classList.contains("separator") ? "—" : el.querySelector(".label")?.textContent,
  );

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
