import { screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { EditRequest } from "../../../src/lib/engine";
import { documentView, layer, layerNames, open, respond, row, sent } from "./harness";

// Layer locks (ADR 0046): Layer > Lock Layers, its shortcuts, and what a lock refuses.

respond("perform", (args, doc) => {
  const edit = args.edit as EditRequest;
  // The engine refuses to move a locked layer, in words the UI recognizes.
  if (edit.kind === "arrangeLayers") throw "layer 1 is locked: position";
  return { ...doc, revision: doc.revision + 1 };
});

test("Layer > Lock Layers sets the locks chosen on the selected layers", async () => {
  const user = open(
    documentView(1, "cat.jpg", [
      { ...layer(1, "Background"), locks: { ...layer(1, "").locks, position: true } },
      layer(2, "Cat"),
    ]),
  );
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Background"]));
  await user.click(row("Background"));
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  await user.click(screen.getByText("Lock Layers…"));
  const dialog = screen.getByRole("dialog", { name: "Lock Layers" });
  // It starts from the layer's locks.
  expect(screen.getByRole("checkbox", { name: "Position" })).toBeChecked();
  expect(screen.getByRole("checkbox", { name: "All" })).not.toBeChecked();
  await user.click(screen.getByRole("checkbox", { name: "All" }));
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(dialog).not.toBeInTheDocument();
  await vi.waitFor(() =>
    expect(sent("perform")).toEqual([
      {
        documentId: 1,
        edit: {
          kind: "setLayerLocks",
          ids: [1],
          transparency: false,
          pixels: false,
          position: true,
          all: true,
        },
      },
    ]),
  );
});

test("/ toggles Lock Transparent Pixels, Ctrl+/ opens Lock Layers", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat"]));
  await user.keyboard("/");
  await vi.waitFor(() =>
    expect(sent("perform")).toEqual([
      { documentId: 1, edit: { kind: "setLayerLocks", ids: [1], transparency: true } },
    ]),
  );
  await user.keyboard("{Control>}/{/Control}");
  expect(await screen.findByRole("dialog", { name: "Lock Layers" })).toBeInTheDocument();
});

test("an edit a lock refuses is said in the interface's language", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Background"), layer(2, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Background"]));
  await user.click(row("Background"));
  await user.keyboard("{Control>}]{/Control}");
  expect(
    await screen.findByText("Could not move: the layer's position is locked."),
  ).toBeInTheDocument();
});
