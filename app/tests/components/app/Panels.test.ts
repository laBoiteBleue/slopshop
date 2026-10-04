import { fireEvent, screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
import { documentView, layer, layerNames, open, row, sent } from "./harness";

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
