import { fireEvent, screen, within } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
import { withEffect, withEffectsVisible } from "../../../src/lib/layerStyle";
import { forgetContourPresets } from "../../../src/lib/styleContours";
import { documentView, layer, layerNames, open, respond, row, sent } from "./harness";

// Layer > Layer Style's commands on styles (Copy, Paste, Clear, Hide or Show All Effects, Scale
// Effects), also on the right-click menu of a layer's fx mark.

respond("paste_layer_style", (_args, doc) => ({ ...doc, revision: doc.revision + 1 }));

const CONE: [number, number, boolean][] = [
  [0, 0, false],
  [128, 255, true],
  [255, 0, false],
];
respond("contour_presets", () => [
  {
    id: "linear",
    contour: {
      points: [
        [0, 0, false],
        [255, 255, false],
      ],
      samples: [0, 1],
    },
  },
  { id: "cone", contour: { points: CONE, samples: [0, 1, 0] } },
]);

const shadow = withEffect(null, "dropShadow", true);

function styled(id: number, name: string, style = shadow): LayerView {
  return { ...layer(id, name), style };
}

/** The labels of the open Layer Style submenu, separators as "—". */
const styleMenuLabels = () =>
  [...document.querySelectorAll(".dropdown.nested > *")].map((el) =>
    el.classList.contains("separator") ? "—" : el.querySelector(".label")?.textContent,
  );

/** The item of the Layer Style submenu labeled `label`, the submenu opened. */
async function styleItem(user: ReturnType<typeof open>, label: string) {
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  await user.hover(screen.getByText("Layer Style", { selector: ".label" }));
  return screen.getByText(label, { selector: ".dropdown.nested .label" }).closest(".item")!;
}

test("Layer > Layer Style lists the effects, then the commands on styles, as Photoshop", async () => {
  const user = open(documentView(1, "cat.jpg", [styled(1, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat"]));
  await styleItem(user, "Copy Layer Style");
  expect(styleMenuLabels().slice(-9)).toEqual([
    "Drop Shadow…",
    "—",
    "Copy Layer Style",
    "Paste Layer Style",
    "Clear Layer Style",
    "—",
    "Global Light…",
    "Hide All Effects",
    "Scale Effects…",
  ]);
});

test("Copy Layer Style, then Paste Layer Style onto every selected layer", async () => {
  const user = open(
    documentView(1, "cat.jpg", [layer(1, "Background"), layer(2, "Sky"), styled(3, "Cat")]),
  );
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Sky", "Background"]));
  // Nothing copied yet: nothing to paste.
  expect((await styleItem(user, "Paste Layer Style")).classList.contains("disabled")).toBe(true);
  await user.click(screen.getByText("Copy Layer Style", { selector: ".label" }));
  await vi.waitFor(() => expect(sent("copy_layer_style")).toEqual([{ documentId: 1, layerId: 3 }]));
  await user.click(row("Sky"));
  await user.keyboard("[ControlLeft>]");
  await user.click(row("Background"));
  await user.keyboard("[/ControlLeft]");
  // A layer without a style has none to copy.
  expect((await styleItem(user, "Copy Layer Style")).classList.contains("disabled")).toBe(true);
  await user.click(screen.getByText("Paste Layer Style", { selector: ".label" }));
  await vi.waitFor(() =>
    expect(sent("paste_layer_style")).toEqual([{ documentId: 1, ids: [1, 2] }]),
  );
});

test("Hide All Effects hides every layer's; once all are hidden, the command shows them", async () => {
  const hidden = withEffectsVisible(shadow, false);
  const user = open(documentView(1, "cat.jpg", [styled(1, "Sky"), styled(2, "Cat", hidden)]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Sky"]));
  await user.click(await styleItem(user, "Hide All Effects"));
  await vi.waitFor(() =>
    expect(sent("perform")).toContainEqual({
      documentId: 1,
      edit: { kind: "setAllEffectsVisible", visible: false },
    }),
  );
  await user.keyboard("{Escape}");
});

test("with every layer's effects hidden, Show All Effects shows them; without effects, neither", async () => {
  const hidden = withEffectsVisible(shadow, false);
  // The last document opened is the active one.
  const user = open(
    documentView(2, "plain.jpg", [layer(1, "Plain")]),
    documentView(1, "cat.jpg", [styled(1, "Sky", hidden), styled(2, "Cat", hidden)]),
  );
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Sky"]));
  await user.click(await styleItem(user, "Show All Effects"));
  await vi.waitFor(() =>
    expect(sent("perform")).toContainEqual({
      documentId: 1,
      edit: { kind: "setAllEffectsVisible", visible: true },
    }),
  );
  await user.click(screen.getByText("plain.jpg"));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Plain"]));
  expect((await styleItem(user, "Hide All Effects")).classList.contains("disabled")).toBe(true);
});

test("Scale Effects scales the selected styles live; OK keeps it, Cancel takes it back", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Background"), styled(2, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Background"]));
  await user.click(await styleItem(user, "Scale Effects…"));
  const dialog = await screen.findByRole("dialog", { name: "Scale Layer Effects" });
  const field = within(dialog).getByRole("spinbutton", { name: "Scale:" });
  await user.clear(field);
  await user.type(field, "50");
  await vi.waitFor(() =>
    expect(sent("perform_live").at(-1)).toEqual({
      documentId: 1,
      edit: { kind: "scaleLayerStyle", ids: [2], scale: 50 },
      replace: true,
    }),
  );
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(sent("end_gesture")).toEqual([{ documentId: 1 }]));
  // Again, cancelled.
  await user.click(await styleItem(user, "Scale Effects…"));
  await user.click(
    within(await screen.findByRole("dialog", { name: "Scale Layer Effects" })).getByRole("button", {
      name: "Cancel",
    }),
  );
  await vi.waitFor(() => expect(sent("cancel_gesture")).toEqual([{ documentId: 1 }]));
  // Not offered for a layer without a style.
  await user.click(row("Background"));
  expect((await styleItem(user, "Scale Effects…")).classList.contains("disabled")).toBe(true);
});

test("a right-click on a layer's fx mark offers Layer Style's commands", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Background"), styled(2, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Background"]));
  await user.click(row("Background"));
  await fireEvent.contextMenu(within(row("Cat")).getByText("fx"));
  await user.click(await screen.findByText("Clear Layer Style"));
  await vi.waitFor(() =>
    expect(sent("perform")).toContainEqual({
      documentId: 1,
      edit: { kind: "setLayerStyle", id: 2, style: null },
    }),
  );
});

test("Global Light sets the document's light live; OK keeps it as one undo entry", async () => {
  const doc = {
    ...documentView(1, "cat.jpg", [styled(1, "Cat")]),
    globalLight: [90, 30] as [number, number],
  };
  const user = open(doc);
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat"]));
  await user.click(await styleItem(user, "Global Light…"));
  const dialog = await screen.findByRole("dialog", { name: "Global Light" });
  const angle = within(dialog).getByRole("spinbutton", { name: "Angle:" });
  expect(angle).toHaveValue(90);
  await user.clear(angle);
  await user.type(angle, "45");
  await vi.waitFor(() =>
    expect(sent("perform_live").at(-1)).toEqual({
      documentId: 1,
      edit: { kind: "setGlobalLight", angle: 45, altitude: 30 },
      replace: true,
    }),
  );
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(sent("end_gesture")).toEqual([{ documentId: 1 }]));
});

test("in Layer Style, a shadow's angle on the global light moves the document's light with it", async () => {
  const user = open(documentView(1, "cat.jpg", [styled(1, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat"]));
  await user.click(await styleItem(user, "Drop Shadow…"));
  const dialog = await screen.findByRole("dialog", { name: "Layer Style" });
  const settings = within(dialog).getByRole("region", { name: "Drop Shadow" });
  const angle = within(settings).getAllByRole("spinbutton")[1];
  await user.clear(angle);
  await user.type(angle, "30");
  await user.tab();
  await vi.waitFor(() =>
    expect(sent("perform_live").at(-1)).toEqual({
      documentId: 1,
      edit: {
        kind: "batch",
        edits: [
          { kind: "setGlobalLight", angle: 30, altitude: 30 },
          expect.objectContaining({
            kind: "setLayerStyle",
            style: expect.objectContaining({
              dropShadow: expect.objectContaining({ angle: 30, useGlobalLight: true }),
            }),
          }),
        ],
      },
      replace: false,
    }),
  );
});

test("in Layer Style, a shadow's contour picked from the engine's presets is sent live", async () => {
  // Asked once by the app (the dialogs of the tests before asked already).
  forgetContourPresets();
  const user = open(documentView(1, "cat.jpg", [styled(1, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat"]));
  await user.click(await styleItem(user, "Drop Shadow…"));
  const dialog = await screen.findByRole("dialog", { name: "Layer Style" });
  const settings = within(dialog).getByRole("region", { name: "Drop Shadow" });
  await user.click(within(settings).getByRole("button", { name: "Contour" }));
  await user.click(await within(settings).findByRole("option", { name: "Cone" }));
  await vi.waitFor(() =>
    expect(sent("perform_live").at(-1)).toEqual({
      documentId: 1,
      edit: expect.objectContaining({
        kind: "setLayerStyle",
        style: expect.objectContaining({
          dropShadow: expect.objectContaining({
            contour: expect.objectContaining({ points: CONE }),
          }),
        }),
      }),
      replace: expect.any(Boolean),
    }),
  );
  expect(sent("contour_presets")).toHaveLength(1);
});
