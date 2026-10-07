import { screen, within } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { DocumentView, LayerView } from "../../../src/lib/engine";
import { documentView, layer, layerNames, open, row, sent } from "./harness";

// The Sources panel and Layer > Make Source Unique (ADR 0040).

const showing = (id: number, name: string, source: number): LayerView => ({
  ...layer(id, name),
  source,
});

function photos(): DocumentView {
  return {
    ...documentView(1, "photo.jpg", [
      showing(1, "Photo", 7),
      showing(2, "Photo copy", 7),
      showing(3, "Pasted", 8),
    ]),
    sources: [
      { id: 7, name: "photo.jpg", width: 400, height: 300, layers: [1, 2] },
      { id: 8, name: "", width: 40, height: 30, layers: [3] },
    ],
  };
}

test("the Sources panel lists the document's sources; a click selects their layers", async () => {
  localStorage.clear();
  const user = open(photos());
  await vi.waitFor(() => expect(layerNames()).toEqual(["Pasted", "Photo copy", "Photo"]));
  await user.click(screen.getByRole("tab", { name: /Sources/ }));
  const panel = screen.getByRole("tabpanel", { name: "Sources" });
  const source = (name: string) => screen.getByRole("option", { name: new RegExp(name) });
  expect(panel).toHaveTextContent("photo.jpg");
  // Pasted pixels have no name of their own: the layer's.
  expect(source("Pasted")).toHaveTextContent("1 layer");
  await user.click(source("photo.jpg"));
  expect(row("Photo")).toHaveClass("selected");
  expect(row("Photo copy")).toHaveClass("selected");
  expect(row("Pasted")).not.toHaveClass("selected");
  // A new layer showing it, above the active layer.
  await user.pointer({ keys: "[MouseRight]", target: source("photo.jpg") });
  await user.click(screen.getByRole("menuitem", { name: "New Layer from Source" }));
  await vi.waitFor(() =>
    expect(sent("perform").at(-1)?.edit).toEqual({
      kind: "addSourceLayer",
      source: 7,
      name: "photo.jpg",
      parent: null,
      index: 2,
    }),
  );
});

test("Layer > Make Source Unique: on layers sharing their source, grayed otherwise", async () => {
  const user = open(photos());
  await vi.waitFor(() => expect(layerNames()).toContain("Pasted"));
  const makeUnique = () => screen.getByRole("menuitem", { name: "Make Source Unique" });
  await user.click(row("Pasted"));
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  expect(makeUnique()).toHaveAttribute("aria-disabled", "true");
  await user.keyboard("{Escape}");
  await user.click(row("Photo copy"));
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  await user.click(makeUnique());
  await vi.waitFor(() =>
    expect(sent("perform").at(-1)?.edit).toEqual({ kind: "makeUnique", ids: [2] }),
  );
});

test("deleting a source asks first, listing the layers that go with it", async () => {
  localStorage.clear();
  const user = open(photos());
  await vi.waitFor(() => expect(layerNames()).toContain("Pasted"));
  await user.click(screen.getByRole("tab", { name: /Sources/ }));
  const source = screen.getByRole("option", { name: /photo\.jpg/ });
  // Cancel: nothing sent.
  await user.pointer({ keys: "[MouseRight]", target: source });
  await user.click(screen.getByRole("menuitem", { name: "Delete Source…" }));
  let dialog = await screen.findByRole("dialog", { name: "Delete Source" });
  expect(dialog).toHaveTextContent('2 layers show "photo.jpg"');
  expect(
    within(dialog)
      .getAllByRole("listitem")
      .map((li) => li.textContent),
  ).toEqual(["Photo", "Photo copy"]);
  await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(sent("perform")).toEqual([]);
  // Delete: one edit.
  await user.pointer({ keys: "[MouseRight]", target: source });
  await user.click(screen.getByRole("menuitem", { name: "Delete Source…" }));
  dialog = await screen.findByRole("dialog", { name: "Delete Source" });
  await user.click(within(dialog).getByRole("button", { name: "Delete" }));
  await vi.waitFor(() =>
    expect(sent("perform").at(-1)?.edit).toEqual({ kind: "deleteSource", source: 7 }),
  );
});
