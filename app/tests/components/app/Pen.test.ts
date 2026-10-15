import { screen, waitFor, within } from "@testing-library/svelte";
import { expect, test } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
import type { PathGeometry } from "../../../src/lib/pen";
import { documentView, layer, open, respond, row, sent } from "./harness";

// The Pen (P, ADR 0041): clicks on the image build a path, finished into a vector layer;
// Ctrl+Enter makes a selection of a path.

respond("make_selection", (_, doc) => ({ ...doc }));

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

test("P picks the Pen: a closed path adds a filled vector layer named Shape 1", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("p");
  expect(screen.getByRole("button", { name: "Pen Tool" })).toHaveAttribute("aria-pressed", "true");
  // Fill and stroke from the options bar, as the shape tools'.
  await user.click(screen.getByRole("checkbox", { name: "Stroke" }));
  await click(user, [10, 10], [110, 10], [60, 90], [11, 10]);
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("perform")[0]).toMatchObject({
    documentId: 1,
    edit: {
      kind: "addShape",
      name: "Shape 1",
      shape: {
        geometry: {
          kind: "path",
          rule: "nonzero",
          subpaths: [
            {
              start: [10, 10],
              segments: [
                [110, 10],
                [60, 90],
                [10, 10],
              ],
              closed: true,
            },
          ],
        },
        fill: [0, 0, 0, 1],
        stroke: { width: 3, align: "inside" },
      },
      parent: null,
      index: 1,
    },
  });
});

test("Ctrl+Enter selects the path being drawn, else the active vector layer's", async () => {
  const shape = {
    ...layer(2, "Shape 1"),
    kind: "vector" as const,
    shape: {
      geometry: {
        kind: "path",
        rule: "nonzero",
        subpaths: [
          {
            start: [0, 0],
            segments: [
              [10, 0],
              [0, 10],
            ],
            closed: true,
          },
        ],
      },
      fill: [0, 0, 0, 1],
      stroke: null,
    },
  } as LayerView;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), shape]));
  await waitFor(() => expect(row("Shape 1")).toBeInTheDocument());
  await user.keyboard("p");
  await click(user, [10, 10], [50, 10], [50, 50]);
  await user.keyboard("{Control>}{Enter}{/Control}");
  await waitFor(() => expect(sent("make_selection")).toHaveLength(1));
  expect(sent("make_selection")[0]).toEqual({
    documentId: 1,
    path: {
      kind: "path",
      rule: "nonzero",
      subpaths: [
        {
          start: [10, 10],
          segments: [
            [50, 10],
            [50, 50],
          ],
          closed: false,
        },
      ],
    },
    layerId: null,
    vectorMask: false,
  });
  // No layer was added.
  expect(sent("perform")).toHaveLength(0);
  // No path drawn: the active layer's, when it is a vector layer.
  await user.click(row("Shape 1"));
  await user.keyboard("{Control>}{Enter}{/Control}");
  await waitFor(() => expect(sent("make_selection")).toHaveLength(2));
  expect(sent("make_selection")[1]).toEqual({
    documentId: 1,
    path: null,
    layerId: 2,
    vectorMask: false,
  });
  // A pixel layer has no path: nothing is sent.
  await user.click(row("Cat"));
  await user.keyboard("{Control>}{Enter}{/Control}");
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(sent("make_selection")).toHaveLength(2);
});

test("changing tool finishes the open path into a layer", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("p");
  await click(user, [10, 10], [60, 40]);
  await user.keyboard("v");
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("perform")[0]).toMatchObject({
    edit: {
      kind: "addShape",
      name: "Shape 1",
      shape: {
        geometry: {
          kind: "path",
          subpaths: [{ start: [10, 10], segments: [[60, 40]], closed: false }],
        },
      },
    },
  });
});

// Editing a vector layer's path with the Pen (Photoshop's Pen and Direct Selection gestures).

/** A vector layer drawn by the Pen: a closed triangle, its third anchor smooth. */
function pathLayer(id: number, name: string, extra: Partial<LayerView> = {}): LayerView {
  return {
    ...layer(id, name),
    kind: "vector",
    shape: {
      geometry: {
        kind: "path",
        rule: "nonzero",
        subpaths: [
          {
            start: [10, 10],
            segments: [
              [110, 10],
              [110, 10, 110, 110, 60, 110],
              [10, 110, 10, 10, 10, 10],
            ],
            closed: true,
          },
        ],
      },
      fill: [0, 0, 0, 1],
      stroke: null,
    },
    ...extra,
  } as LayerView;
}

const svg = () => document.querySelector("svg.pen-tool") as SVGSVGElement;

/** The app on a document with `shape` above a pixel layer, the shape selected, the Pen on. */
async function editing(shape: LayerView) {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), shape]));
  await waitFor(() => expect(row(shape.name)).toBeInTheDocument());
  await user.click(row(shape.name));
  await user.keyboard("p");
  return user;
}

function drag(user: ReturnType<typeof open>, from: [number, number], to: [number, number]) {
  return user.pointer([
    { keys: "[MouseLeft>]", target: svg(), coords: { clientX: from[0], clientY: from[1] } },
    { target: svg(), coords: { clientX: (from[0] + to[0]) / 2, clientY: (from[1] + to[1]) / 2 } },
    { target: svg(), coords: { clientX: to[0], clientY: to[1] } },
    { keys: "[/MouseLeft]", target: svg(), coords: { clientX: to[0], clientY: to[1] } },
  ]);
}

/** The segments of the path sent by the last `setShape`. */
function sentPath() {
  const edits = sent("perform").map((a) => a.edit as Record<string, unknown>);
  const last = edits.at(-1) as { kind: string; id: number; shape: { geometry: PathGeometry } };
  expect(last.kind).toBe("setShape");
  return last;
}

test("the active vector layer's anchors show; Ctrl-drag moves one, in one edit", async () => {
  const user = await editing(pathLayer(2, "Shape 1"));
  await waitFor(() => expect(screen.getAllByTestId("pen-layer-anchor")).toHaveLength(3));
  await user.keyboard("{Control>}");
  await drag(user, [110, 10], [130, 20]);
  await user.keyboard("{/Control}");
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  const edit = sentPath();
  expect(edit.id).toBe(2);
  expect(edit.shape.geometry.subpaths[0]).toEqual({
    start: [10, 10],
    segments: [
      [130, 20],
      [130, 20, 110, 110, 60, 110],
      [10, 110, 10, 10, 10, 10],
    ],
    closed: true,
  });
  // The fill kept.
  expect(sent("perform")[0]).toMatchObject({ edit: { shape: { fill: [0, 0, 0, 1] } } });
});

test("a click on a segment adds an anchor, on an anchor deletes it", async () => {
  const user = await editing(pathLayer(2, "Shape 1"));
  await waitFor(() => expect(screen.getAllByTestId("pen-layer-anchor")).toHaveLength(3));
  await click(user, [60, 11]);
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sentPath().shape.geometry.subpaths[0].segments[0]).toEqual([60, 10]);
  expect(sentPath().shape.geometry.subpaths[0].segments).toHaveLength(4);
  // No new layer was started.
  expect(sent("perform").some((a) => (a.edit as { kind: string }).kind === "addShape")).toBe(false);
  await click(user, [110, 10]);
  await waitFor(() => expect(sent("perform")).toHaveLength(2));
  expect(sentPath().shape.geometry.subpaths[0]).toEqual({
    start: [10, 10],
    segments: [
      [10, 10, 110, 110, 60, 110],
      [10, 110, 10, 10, 10, 10],
    ],
    closed: true,
  });
});

test("Alt+click on a smooth anchor makes it a corner", async () => {
  const user = await editing(pathLayer(2, "Shape 1"));
  await waitFor(() => expect(screen.getAllByTestId("pen-layer-anchor")).toHaveLength(3));
  await user.keyboard("{Alt>}");
  await click(user, [60, 110]);
  await user.keyboard("{/Alt}");
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sentPath().shape.geometry.subpaths[0].segments).toEqual([
    [110, 10],
    [60, 110],
    [10, 10],
  ]);
});

test("a click on an open path's end continues it", async () => {
  const open_ = pathLayer(2, "Path 1", {});
  open_.shape!.geometry = {
    kind: "path",
    rule: "nonzero",
    subpaths: [{ start: [10, 10], segments: [[110, 10]], closed: false }],
  };
  const user = await editing(open_);
  await waitFor(() => expect(screen.getAllByTestId("pen-layer-anchor")).toHaveLength(2));
  // From its first anchor: drawn the other way.
  await click(user, [10, 10], [10, 90]);
  await user.keyboard("{Enter}");
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sentPath().shape.geometry.subpaths[0]).toEqual({
    start: [110, 10],
    segments: [
      [10, 10],
      [10, 90],
    ],
    closed: false,
  });
});

test("a live shape asks to be turned into a path first", async () => {
  const rect = pathLayer(2, "Rectangle 1");
  const livePath = rect.shape!.geometry;
  rect.shape!.geometry = { kind: "rectangle", rect: [10, 10, 110, 110], radii: [0, 0, 0, 0] };
  rect.livePath = livePath;
  const user = await editing(rect);
  await waitFor(() => expect(screen.getAllByTestId("pen-layer-anchor")).toHaveLength(3));
  await user.keyboard("{Control>}");
  await drag(user, [110, 10], [130, 20]);
  await user.keyboard("{/Control}");
  const dialog = await screen.findByRole("dialog", { name: "Turn into a Path" });
  expect(sent("perform")).toHaveLength(0);
  await user.click(within(dialog).getByRole("button", { name: "Turn into a Path" }));
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sentPath()).toMatchObject({ id: 2, shape: { geometry: livePath, fill: [0, 0, 0, 1] } });
});

test("a layer in perspective is not edited: a message says so", async () => {
  const user = await editing(
    pathLayer(2, "Shape 1", { perspective: [1, 0, 0, 1, 0, 0, 0.001, 0, 1] }),
  );
  expect(screen.queryAllByTestId("pen-layer-anchor")).toHaveLength(0);
  await user.keyboard("{Control>}");
  await click(user, [110, 10]);
  await user.keyboard("{/Control}");
  expect(
    await screen.findByText("The path of a layer in perspective cannot be edited."),
  ).toBeInTheDocument();
  expect(sent("perform")).toHaveLength(0);
});

test("Ctrl-drag on a handle turns a smooth anchor's two; on a segment moves the path", async () => {
  const user = await editing(pathLayer(2, "Shape 1"));
  await waitFor(() => expect(screen.getAllByTestId("pen-layer-anchor")).toHaveLength(3));
  await user.keyboard("{Control>}");
  // The third anchor's outgoing handle, (10, 110), dragged up to (10, 60).
  await drag(user, [10, 110], [10, 60]);
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  const [, curve, closing] = sentPath().shape.geometry.subpaths[0].segments;
  expect(closing.slice(0, 2)).toEqual([10, 60]);
  // The incoming one opposite, 50 px long as it was.
  expect(curve[2]).toBeCloseTo(60 + 50 / Math.SQRT2);
  expect(curve[3]).toBeCloseTo(110 + 50 / Math.SQRT2);
  // A segment: the whole path moves (from the layer's path: the mock keeps it).
  await drag(user, [60, 10], [70, 30]);
  await user.keyboard("{/Control}");
  await waitFor(() => expect(sent("perform")).toHaveLength(2));
  expect(sentPath().shape.geometry.subpaths[0].start).toEqual([20, 30]);
});

test("over the path, the pointer shows what a click does: + adds, − deletes", async () => {
  const user = await editing(pathLayer(2, "Shape 1"));
  await waitFor(() => expect(screen.getAllByTestId("pen-layer-anchor")).toHaveLength(3));
  await user.pointer({ target: svg(), coords: { clientX: 60, clientY: 11 } });
  expect(screen.getByTestId("pen-hint-add")).toBeInTheDocument();
  await user.pointer({ target: svg(), coords: { clientX: 110, clientY: 11 } });
  expect(screen.getByTestId("pen-hint-delete")).toBeInTheDocument();
  await user.pointer({ target: svg(), coords: { clientX: 60, clientY: 60 } });
  expect(screen.queryByTestId(/pen-hint/)).toBeNull();
});
