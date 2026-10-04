import { fireEvent, screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
import { documentView, layer, layerNames, open, respond, row, sent } from "./harness";

// The panels below Layers: the dock's tabs, Properties and the Selections panel.

test("the dock below Layers: tabs unfold their panel, the open one folds it, its edge resizes it", async () => {
  localStorage.clear();
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  const tab = (name: string) => screen.getByRole("tab", { name });
  // Properties at first, empty without an adjustment or fill layer.
  expect(tab("Properties")).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("tabpanel", { name: "Properties" })).toHaveTextContent(
    "Select an adjustment or fill layer",
  );
  await user.click(screen.getByRole("tab", { name: /Selections/ }));
  expect(screen.getByRole("tabpanel", { name: "Selections" })).toHaveTextContent(
    "No saved selection",
  );
  // The open tab folds the dock down to its icons, remembered.
  await user.click(screen.getByRole("tab", { name: "Selections" }));
  expect(screen.queryByRole("tabpanel")).not.toBeInTheDocument();
  expect(JSON.parse(localStorage.getItem("slopshop.dock")!)).toMatchObject({ open: null });
  await user.click(screen.getByRole("tab", { name: /Selections/ }));
  const edge = screen.getByRole("separator", { name: "Resize the panels below Layers" });
  await fireEvent.pointerDown(edge, { pointerId: 1, button: 0, clientY: 500 });
  await fireEvent.pointerMove(edge, { pointerId: 1, clientY: 450 });
  await fireEvent.pointerUp(edge, { pointerId: 1, clientY: 450 });
  expect(JSON.parse(localStorage.getItem("slopshop.dock")!)).toEqual({
    open: "selections",
    height: 330,
  });
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
  // Folded meanwhile.
  await user.click(screen.getByRole("tab", { name: "Properties" }));
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
  expect(JSON.parse(localStorage.getItem("slopshop.dock")!)).toMatchObject({ open: null });
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
