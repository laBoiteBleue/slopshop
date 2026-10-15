import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { expect, test } from "vitest";
import type { LayerView, VectorMaskView } from "../../../src/lib/engine";
import { documentView, layer, open, respond, row, sent } from "./harness";

// Vector masks (ADR 0041): Layer > Vector Mask, their thumbnail in the Layers panel, the Pen
// drawing in a targeted one, Ctrl+Enter, and their feather in Properties.

respond("make_selection", (_, doc) => ({ ...doc }));

const square = {
  kind: "path" as const,
  rule: "nonzero" as const,
  subpaths: [
    {
      start: [0, 0] as [number, number],
      segments: [
        [100, 0],
        [100, 100],
        [0, 100],
      ] as [number, number][],
      closed: true,
    },
  ],
};

const vectorMask = (changes: Partial<VectorMaskView> = {}): VectorMaskView => ({
  enabled: true,
  inverted: false,
  feather: 0,
  geometry: square,
  livePath: null,
  ...changes,
});

const masked = (id: number, name: string, changes: Partial<VectorMaskView> = {}): LayerView => ({
  ...layer(id, name),
  vectorMask: vectorMask(changes),
});

const shapeLayer = (id: number, name: string): LayerView =>
  ({
    ...layer(id, name),
    kind: "vector",
    shape: { geometry: square, fill: [0, 0, 0, 1], stroke: null },
  }) as LayerView;

/** Open Layer > Vector Mask and click `entry` in it. */
async function vectorMaskMenu(user: ReturnType<typeof open>, entry: string) {
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  await user.hover(screen.getByText("Vector Mask", { selector: ".label" }));
  const nested = document.querySelector(".dropdown.nested") as HTMLElement;
  const item = within(nested).getByText(entry, { selector: ".label" }).closest("[role=menuitem]");
  if (item?.getAttribute("aria-disabled") === "true") throw new Error(`${entry} is disabled`);
  await user.click(item as HTMLElement);
}

test("Layer > Vector Mask > Reveal All and Hide All mask the selected layers", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Background"), layer(2, "Cat")]));
  await waitFor(() => expect(row("Cat")).toBeInTheDocument());
  await user.keyboard("[ControlLeft>]");
  await user.click(row("Background"));
  await user.keyboard("[/ControlLeft]");
  await vectorMaskMenu(user, "Reveal All");
  await waitFor(() =>
    expect(sent("perform")).toEqual([
      { documentId: 1, edit: { kind: "addVectorMask", ids: [1, 2], mask: "revealAll" } },
    ]),
  );
  // Disable, Delete and Current Path need a vector mask, or a path to take.
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  await user.hover(screen.getByText("Vector Mask", { selector: ".label" }));
  const nested = document.querySelector(".dropdown.nested") as HTMLElement;
  for (const name of ["Disable", "Delete", "Current Path"]) {
    expect(
      within(nested).getByText(name, { selector: ".label" }).closest("[role=menuitem]"),
    ).toHaveAttribute("aria-disabled", "true");
  }
});

test("Current Path masks the active layer with the other selected vector layer's path", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), shapeLayer(2, "Shape 1")]));
  await waitFor(() => expect(row("Shape 1")).toBeInTheDocument());
  // Shape 1 selected, then Cat added: Cat is active.
  await user.keyboard("[ControlLeft>]");
  await user.click(row("Cat"));
  await user.keyboard("[/ControlLeft]");
  await vectorMaskMenu(user, "Current Path");
  await waitFor(() =>
    expect(sent("perform")).toEqual([
      {
        documentId: 1,
        edit: { kind: "addVectorMask", ids: [1], mask: "currentPath", from: 2 },
      },
    ]),
  );
});

test("Disable then Delete act on the selected layers' vector masks", async () => {
  const user = open(documentView(1, "cat.jpg", [masked(1, "Cat")]));
  await waitFor(() => expect(row("Cat")).toBeInTheDocument());
  expect(row("Cat").querySelector(".vector-mask-thumb")).not.toBeNull();
  await vectorMaskMenu(user, "Disable");
  await vectorMaskMenu(user, "Delete");
  await waitFor(() =>
    expect(sent("perform")).toEqual([
      { documentId: 1, edit: { kind: "setVectorMaskEnabled", id: 1, enabled: false } },
      { documentId: 1, edit: { kind: "removeVectorMask", id: 1 } },
    ]),
  );
});

function click(user: ReturnType<typeof open>, ...points: [number, number][]) {
  const svg = document.querySelector("svg.pen-tool") as SVGSVGElement;
  return user.pointer(
    points.map(([x, y]) => ({
      keys: "[MouseLeft]",
      target: svg,
      coords: { clientX: x, clientY: y },
    })),
  );
}

test("with its vector mask targeted, the Pen adds to the mask's path and Ctrl+Enter selects it", async () => {
  const user = open(documentView(1, "cat.jpg", [masked(1, "Cat")]));
  await waitFor(() => expect(row("Cat")).toBeInTheDocument());
  await user.click(row("Cat").querySelector(".vector-mask-thumb") as HTMLElement);
  await user.keyboard("p");
  // A triangle away from the square: a new part of the mask, no new layer.
  await click(user, [200, 200], [300, 200], [250, 280], [201, 200]);
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("perform")[0]).toEqual({
    documentId: 1,
    edit: {
      kind: "setVectorMaskPath",
      id: 1,
      path: {
        kind: "path",
        rule: "nonzero",
        subpaths: [
          ...square.subpaths,
          {
            start: [200, 200],
            segments: [
              [300, 200],
              [250, 280],
              [200, 200],
            ],
            closed: true,
          },
        ],
      },
    },
  });
  await user.keyboard("{Control>}{Enter}{/Control}");
  await waitFor(() =>
    expect(sent("make_selection")).toEqual([
      { documentId: 1, path: null, layerId: 1, vectorMask: true },
    ]),
  );
});

test("the vector mask's feather is set in Properties", async () => {
  const user = open(documentView(1, "cat.jpg", [masked(1, "Cat", { feather: 2 })]));
  await waitFor(() => expect(row("Cat")).toBeInTheDocument());
  const panel = await screen.findByRole("region", { name: "Vector Mask" });
  const feather = within(panel).getByRole("spinbutton", { name: "Feather" });
  expect(feather).toHaveValue(2);
  await user.clear(feather);
  await user.type(feather, "12.5");
  await fireEvent.change(feather);
  await waitFor(() =>
    expect(sent("perform")).toContainEqual({
      documentId: 1,
      edit: { kind: "setVectorMaskFeather", id: 1, feather: 12.5 },
    }),
  );
});
