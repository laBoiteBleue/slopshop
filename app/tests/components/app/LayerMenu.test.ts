import { emit } from "@tauri-apps/api/event";
import { screen, within } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
import { documentView, layer, layerNames, open, respond, row, sent } from "./harness";

// The Layer menu and its shortcuts, on the selected layers.

respond("bake_layers", (args, doc) => {
  // New Layer from Visible: its preview at once, a group shown as the layer it becomes.
  const request = args.request as { kind: string; name?: string };
  if (request.kind === "visible") {
    const preview = { ...layer(99, request.name ?? ""), kind: "group" as const };
    doc.layers = [...doc.layers, { ...preview, baking: true, children: doc.layers }];
  }
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

test("Alt+Shift+Ctrl+E shows the new layer at once, its pixels following", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Background"), layer(2, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Background"]));
  await user.click(row("Background"));
  await user.keyboard("{Alt>}{Shift>}{Control>}e{/Control}{/Shift}{/Alt}");
  await vi.waitFor(() =>
    expect(sent("bake_layers")).toEqual([
      { documentId: 1, request: { kind: "visible", name: "Layer 3" } },
    ]),
  );
  // At once: listed as the layer it becomes (no folder, its layers not listed), selected.
  await vi.waitFor(() => expect(layerNames()).toEqual(["Layer 3", "Cat", "Background"]));
  expect(row("Layer 3")).toHaveClass("selected");
  expect(within(row("Layer 3")).getByTitle("Computing its pixels…")).toBeInTheDocument();
  // The pixels come: the document is sent again.
  const done = documentView(1, "cat.jpg", [
    layer(1, "Background"),
    layer(2, "Cat"),
    layer(99, "Layer 3"),
  ]);
  await emit("document-updated", { ...done, revision: 5 });
  await vi.waitFor(() =>
    expect(within(row("Layer 3")).queryByTitle("Computing its pixels…")).toBeNull(),
  );
  expect(layerNames()).toEqual(["Layer 3", "Cat", "Background"]);
});

test("Ctrl+E merges the selected layers; with one, the menu says Merge Down", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Background"), layer(2, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Background"]));
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  await user.hover(screen.getByText("Bake to Pixels", { selector: ".label" }));
  expect(screen.getByText("Merge Down")).toBeInTheDocument();
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  await user.keyboard("[ControlLeft>]");
  await user.click(row("Background"));
  await user.keyboard("[/ControlLeft]");
  await user.keyboard("{Control>}e{/Control}");
  await vi.waitFor(() =>
    expect(sent("bake_layers")).toEqual([
      { documentId: 1, request: { kind: "merge", ids: [1, 2] } },
    ]),
  );
});

test("Layer > Layer Style > Drop Shadow turns it on live; OK keeps it as one undo entry", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat"]));
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  await user.hover(screen.getByText("Layer Style", { selector: ".label" }));
  await user.click(screen.getByText("Drop Shadow…", { selector: ".label" }));
  const dialog = await screen.findByRole("dialog", { name: "Layer Style" });
  await vi.waitFor(() =>
    expect(sent("perform_live")).toContainEqual(
      expect.objectContaining({
        edit: expect.objectContaining({
          kind: "setLayerStyle",
          id: 1,
          style: expect.objectContaining({
            dropShadow: expect.objectContaining({ enabled: true, mode: "multiply" }),
          }),
        }),
      }),
    ),
  );
  expect(within(dialog).getByRole("checkbox", { name: "Drop Shadow" })).toBeChecked();
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(sent("end_gesture")).toEqual([{ documentId: 1 }]));
  expect(sent("cancel_gesture")).toEqual([]);
});

test("Layer > Layer Style is open to groups, not to adjustment layers", async () => {
  const group = { ...layer(1, "Back"), kind: "group" as const, children: [layer(2, "Sky")] };
  const curves = { ...layer(3, "Curves"), kind: "adjustment" as const };
  const user = open(documentView(1, "cat.jpg", [group, curves]));
  await vi.waitFor(() => expect(layerNames()).toContain("Back"));
  const styleItem = async () => {
    await user.click(screen.getByRole("menuitem", { name: "Layer" }));
    const label = screen.getByText("Layer Style", { selector: ".label" });
    return label.closest(".item") as HTMLElement;
  };
  await user.click(row("Back"));
  expect((await styleItem()).classList.contains("disabled")).toBe(false);
  await user.keyboard("{Escape}");
  await user.click(row("Curves"));
  expect((await styleItem()).classList.contains("disabled")).toBe(true);
});

test("Layer > New Fill Layer > Gradient adds the drawing colors' gradient across the document", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  await user.hover(screen.getByText("New Fill Layer", { selector: ".label" }));
  await user.click(screen.getByText("Gradient", { selector: ".dropdown.nested .label" }));
  await vi.waitFor(() => expect(sent("perform")).toHaveLength(1));
  const { edit } = sent("perform")[0] as {
    edit: { kind: string; name: string; gradient: Record<string, unknown> };
  };
  expect(edit.kind).toBe("addGradientFill");
  expect(edit.name).toBe("Gradient Fill 1");
  // Black to white, from the bottom of the 400 × 300 document to its top.
  expect(edit.gradient).toEqual({
    stops: [
      [0, 0, 0, 0],
      [4096, 255, 255, 255],
    ],
    alpha: [1, 1],
    shape: "linear",
    from: [expect.closeTo(200, 6), expect.closeTo(300, 6)],
    to: [expect.closeTo(200, 6), expect.closeTo(0, 6)],
  });
});

test("Layer > Duplicate Layer makes a linked copy, Duplicate as Independent Copy one of its own", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat"]));
  await user.click(row("Cat"));
  for (const name of ["Duplicate Layer", "Duplicate as Independent Copy"]) {
    await user.click(screen.getByRole("menuitem", { name: "Layer" }));
    await user.click(screen.getByText(name, { selector: ".label" }));
  }
  await vi.waitFor(() => expect(sent("perform")).toHaveLength(2));
  expect(sent("perform").map((p) => (p as { edit: unknown }).edit)).toEqual([
    { kind: "duplicateLayers", ids: [1], nameFormat: "{name} copy", independent: false },
    { kind: "duplicateLayers", ids: [1], nameFormat: "{name} copy", independent: true },
  ]);
});

test("linked layers show a link mark telling how many change with them", async () => {
  const linked = (id: number, name: string): LayerView => ({ ...layer(id, name), linked: 1 });
  open(documentView(1, "cat.jpg", [linked(1, "Logo"), linked(2, "Logo copy"), layer(3, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Logo copy", "Logo"]));
  const marks = screen.getAllByLabelText(/^Linked to 1 other layers/);
  expect(marks).toHaveLength(2);
});
