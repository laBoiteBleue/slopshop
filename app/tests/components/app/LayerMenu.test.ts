import { screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
import { documentView, layer, layerNames, open, respond, row, sent } from "./harness";

// The Layer menu and its shortcuts, on the selected layers.

respond("new_layer_from_visible", (args, doc) => {
  doc.layers = [...doc.layers, layer(99, args.name as string)];
  return { ...doc, revision: doc.revision + 1 };
});

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

test("Layer > Align > Left Edges aligns the selected layers, Distribute waits for three", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Background"), layer(2, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Background"]));
  await user.keyboard("[ControlLeft>]");
  await user.click(row("Background"));
  await user.keyboard("[/ControlLeft]");
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  const distribute = screen.getByText("Distribute", { selector: ".label" }).closest("li");
  expect(distribute).toHaveAttribute("aria-disabled", "true");
  await user.hover(screen.getByText("Align", { selector: ".label" }));
  await user.click(screen.getByText("Left Edges"));
  await vi.waitFor(() =>
    expect(sent("perform")).toEqual([
      { documentId: 1, edit: { kind: "alignLayers", ids: [1, 2], align: "left" } },
    ]),
  );
});

test("Alt+Shift+Ctrl+E stamps the visible layers into a new layer on top, selected", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Background"), layer(2, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Background"]));
  await user.click(row("Background"));
  await user.keyboard("{Alt>}{Shift>}{Control>}e{/Control}{/Shift}{/Alt}");
  await vi.waitFor(() =>
    expect(sent("new_layer_from_visible")).toEqual([{ documentId: 1, name: "Layer 3" }]),
  );
  await vi.waitFor(() => expect(layerNames()).toEqual(["Layer 3", "Cat", "Background"]));
  expect(row("Layer 3")).toHaveClass("selected");
});
