import { fireEvent, screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
import { calls, documentView, layer, layerNames, open, respond, row, sent } from "./harness";

// The panels below Layers: the dock's tabs, Properties and the Selections panel; the Window
// menu's bars and layout.

/** The layout saved last. */
const saved = () => JSON.parse(localStorage.getItem("slopshop.layout")!);

test("the dock below Layers: tabs unfold their panel, the open one folds it, its edge resizes it", async () => {
  localStorage.clear();
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  const tab = (name: string) => screen.getByRole("tab", { name });
  // Properties at first, empty without an adjustment or fill layer.
  expect(tab("Properties")).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("tabpanel", { name: "Properties" })).toHaveTextContent(
    "Select an adjustment, fill or vector layer",
  );
  await user.click(screen.getByRole("tab", { name: /Selections/ }));
  expect(screen.getByRole("tabpanel", { name: "Selections" })).toHaveTextContent(
    "No saved selection",
  );
  // The open tab folds the dock down to its icons, remembered.
  await user.click(screen.getByRole("tab", { name: "Selections" }));
  expect(screen.queryByRole("tabpanel")).not.toBeInTheDocument();
  expect(saved().dock).toMatchObject({ open: null });
  await user.click(screen.getByRole("tab", { name: /Selections/ }));
  const edge = screen.getByRole("separator", { name: "Resize the panels below Layers" });
  await fireEvent.pointerDown(edge, { pointerId: 1, button: 0, clientY: 500 });
  await fireEvent.pointerMove(edge, { pointerId: 1, clientY: 450 });
  await fireEvent.pointerUp(edge, { pointerId: 1, clientY: 450 });
  expect(saved().dock).toEqual({ open: "selections", height: 330 });
  localStorage.clear();
});

test("selecting an adjustment layer unfolds Properties", async () => {
  localStorage.setItem("slopshop.dock", JSON.stringify({ open: null, height: 280 }));
  const adjustment = {
    ...layer(2, "Levels 1"),
    kind: "adjustment" as const,
    adjustment: { id: "invert", values: [], curves: null, gradient: null },
  } as unknown as LayerView;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), adjustment]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Levels 1", "Cat"]));
  await user.click(row("Cat"));
  // Properties had unfolded for the top layer: the dock is folded again, as it was.
  expect(screen.queryByRole("tabpanel")).not.toBeInTheDocument();
  await user.click(row("Levels 1"));
  expect(screen.getByRole("tabpanel", { name: "Properties" })).toBeInTheDocument();
  localStorage.clear();
});

test("the Selections panel loads, combines, renames and deletes saved selections", async () => {
  localStorage.setItem("slopshop.dock", JSON.stringify({ open: "selections", height: 280 }));
  const user = open({
    ...documentView(1, "cat.jpg", [layer(1, "Cat")]),
    selectionKey: 7,
    savedSelections: [{ id: 4, name: "Hair" }],
  });
  const hair = await screen.findByRole("option", { name: /Hair/ });
  await user.click(hair);
  await user.keyboard("{Shift>}");
  await user.click(hair);
  await user.keyboard("{/Shift}");
  await vi.waitFor(() =>
    expect(sent("load_selection")).toEqual([
      { documentId: 1, id: 4, mode: "replace" },
      { documentId: 1, id: 4, mode: "add" },
    ]),
  );
  await user.dblClick(hair);
  const field = screen.getByRole("textbox", { name: "Rename" });
  await user.clear(field);
  await user.type(field, "Fur{Enter}");
  await user.click(screen.getByRole("option", { name: /Hair/ }));
  await user.keyboard("{Delete}");
  await vi.waitFor(() => {
    expect(sent("rename_saved_selection")).toEqual([{ documentId: 1, id: 4, name: "Fur" }]);
    expect(sent("delete_saved_selection")).toEqual([{ documentId: 1, id: 4 }]);
  });
  // Delete in the panel deleted the saved selection, not a layer.
  expect(sent("perform")).toEqual([]);
  localStorage.clear();
});

test("Window lists the dock's panels, the unfolded one checked; choosing one unfolds it", async () => {
  localStorage.setItem("slopshop.dock", JSON.stringify({ open: "properties", height: 280 }));
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  const entry = (name: string) => screen.getByRole("menuitemradio", { name: new RegExp(name) });
  await user.click(screen.getByRole("menuitem", { name: "Window" }));
  expect(entry("Properties")).toHaveAttribute("aria-checked", "true");
  expect(entry("Selections")).toHaveAttribute("aria-checked", "false");
  await user.click(screen.getByText("Selections", { selector: ".dropdown .label" }));
  expect(screen.getByRole("tabpanel", { name: "Selections" })).toBeInTheDocument();
  // The unfolded one again folds the dock.
  await user.click(screen.getByRole("menuitem", { name: "Window" }));
  await user.click(screen.getByText("Selections", { selector: ".dropdown .label" }));
  expect(screen.queryByRole("tabpanel")).not.toBeInTheDocument();
  expect(saved().dock).toMatchObject({ open: null });
  localStorage.clear();
});

test("Window waits for a document", async () => {
  const user = open();
  await screen.findByText("Open an image or create a document");
  await user.click(screen.getByRole("menuitem", { name: "Window" }));
  expect(screen.getByRole("menuitemradio", { name: /Selections/ })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
});

test("a press on the empty part of the Selections panel's list deselects in the image", async () => {
  localStorage.setItem("slopshop.dock", JSON.stringify({ open: "selections", height: 280 }));
  open({
    ...documentView(1, "cat.jpg", [layer(1, "Cat")]),
    selectionKey: 7,
    savedSelections: [{ id: 4, name: "Hair" }],
  });
  const list = await screen.findByRole("listbox", { name: "Selections" });
  await fireEvent.pointerDown(list, { button: 0 });
  await vi.waitFor(() => expect(sent("deselect")).toEqual([{ documentId: 1 }]));
  localStorage.clear();
});

test("the Selections panel's Last Selection row reselects", async () => {
  localStorage.setItem("slopshop.dock", JSON.stringify({ open: "selections", height: 280 }));
  const user = open({ ...documentView(1, "cat.jpg", [layer(1, "Cat")]), canReselect: true });
  await user.click(await screen.findByRole("button", { name: "Last Selection" }));
  await vi.waitFor(() => expect(sent("reselect")).toEqual([{ documentId: 1 }]));
  localStorage.clear();
});

test("the Selections panel marks the saved selections combined, until the selection changes otherwise", async () => {
  let key = 7;
  respond("load_selection", (_, doc) => ({ ...doc, selectionKey: ++key }));
  respond("deselect", (_, doc) => ({ ...doc, selectionKey: null }));
  localStorage.setItem("slopshop.dock", JSON.stringify({ open: "selections", height: 280 }));
  const user = open({
    ...documentView(1, "cat.jpg", [layer(1, "Cat")]),
    selectionKey: 7,
    savedSelections: [
      { id: 4, name: "Hair" },
      { id: 5, name: "Shirt" },
    ],
  });
  const option = async (name: string) => screen.findByRole("option", { name: new RegExp(name) });
  await user.click(await option("Hair"));
  await user.keyboard("{Shift>}");
  await user.click(await option("Shirt"));
  await user.keyboard("{/Shift}");
  await vi.waitFor(() =>
    expect(screen.getByTitle("Added to the selection")).toHaveTextContent("+"),
  );
  expect(await option("Hair")).toHaveClass("combined");
  // Deselected: the marks go.
  await user.keyboard("{Control>}d{/Control}");
  await vi.waitFor(() => expect(document.querySelectorAll("li.combined")).toHaveLength(0));
  localStorage.clear();
});

test("Properties gives the dock back to the panel it replaced once its layer is left", async () => {
  localStorage.setItem("slopshop.dock", JSON.stringify({ open: "selections", height: 280 }));
  const adjustment = {
    ...layer(2, "Levels 1"),
    kind: "adjustment" as const,
    adjustment: { id: "invert", values: [], curves: null, gradient: null },
  } as unknown as LayerView;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), adjustment]));
  // The adjustment layer is selected at first (the top one): Properties.
  await vi.waitFor(() =>
    expect(screen.getByRole("tabpanel", { name: "Properties" })).toBeInTheDocument(),
  );
  await user.click(row("Cat"));
  expect(screen.getByRole("tabpanel", { name: "Selections" })).toBeInTheDocument();
  // Folded and unfolded by the user meanwhile: Properties stays.
  await user.click(row("Levels 1"));
  expect(screen.getByRole("tabpanel", { name: "Properties" })).toBeInTheDocument();
  await user.click(screen.getByRole("tab", { name: "Properties" }));
  await user.click(screen.getByRole("tab", { name: /Properties/ }));
  await user.click(row("Cat"));
  expect(screen.getByRole("tabpanel", { name: "Properties" })).toBeInTheDocument();
  localStorage.clear();
});

test("Window > Options Bar and Toolbar hide them, saved; Reset Layout brings everything back", async () => {
  localStorage.clear();
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  const choose = async (name: string) => {
    await user.click(screen.getByRole("menuitem", { name: "Window" }));
    await user.click(screen.getByText(name, { selector: ".dropdown .label" }));
  };
  const chrome = () => [...document.querySelectorAll(".chrome")];
  const toolbar = () => chrome()[1];
  const optionsBar = () => chrome()[0];
  await choose("Toolbar");
  expect(toolbar()).toHaveClass("hidden");
  expect(optionsBar()).not.toHaveClass("hidden");
  expect(document.querySelector("main")).toHaveClass("no-toolbar");
  await choose("Options Bar");
  expect(optionsBar()).toHaveClass("hidden");
  expect(saved()).toMatchObject({ toolbar: false, optionsBar: false });
  await user.click(screen.getByRole("menuitem", { name: "Window" }));
  expect(screen.getByRole("menuitemradio", { name: /Toolbar/ })).toHaveAttribute(
    "aria-checked",
    "false",
  );
  await user.keyboard("{Escape}");
  // A tab dragged meanwhile: the reset puts the order back too.
  const tab = screen.getByRole("tab", { name: /Properties/ });
  await fireEvent.pointerDown(tab, { pointerId: 1, button: 0, clientX: 10 });
  await fireEvent.pointerMove(tab, { pointerId: 1, clientX: 100 });
  await fireEvent.pointerUp(tab, { pointerId: 1, clientX: 100 });
  expect(saved().order).toEqual([
    "selections",
    "sources",
    "history",
    "histogram",
    "info",
    "properties",
  ]);
  await choose("Reset Layout");
  expect(toolbar()).not.toHaveClass("hidden");
  expect(optionsBar()).not.toHaveClass("hidden");
  expect(saved()).toMatchObject({ toolbar: true, optionsBar: true });
  expect(saved().order).toEqual([
    "properties",
    "selections",
    "sources",
    "history",
    "histogram",
    "info",
  ]);
  localStorage.clear();
});

test("History lists the steps under the initial state, asked only while shown; a click goes back", async () => {
  respond("history", () => ({
    entries: [
      { kind: "newLayer", detail: null },
      { kind: "filter", detail: "gaussianBlur" },
      { kind: "brush", detail: null },
    ],
    done: 2,
  }));
  respond("go_to_history", (_, doc) => ({ ...doc, revision: doc.revision + 1 }));
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  // Properties shows: History costs nothing.
  expect(calls.some((c) => c.cmd === "history")).toBe(false);
  await user.click(screen.getByRole("tab", { name: /History/ }));
  const panel = screen.getByRole("tabpanel", { name: "History" });
  await vi.waitFor(() => expect(panel).toHaveTextContent("Gaussian Blur"));
  const step = (name: string) => screen.getByRole("button", { name });
  expect(step("Initial State")).toBeInTheDocument();
  expect(step("New Layer")).not.toHaveClass("undone");
  expect(step("Gaussian Blur")).toHaveAttribute("aria-current", "step");
  // Undone (redo's): listed, dimmed.
  expect(step("Brush")).toHaveClass("undone");
  await user.click(step("Initial State"));
  await vi.waitFor(() => expect(sent("go_to_history")).toEqual([{ documentId: 1, done: 0 }]));
  // The document changed: asked again.
  await vi.waitFor(() => expect(sent("history").length).toBeGreaterThan(1));
  // The current step clicked: nothing to do.
  await user.click(step("Gaussian Blur"));
  expect(sent("go_to_history")).toHaveLength(1);
});
