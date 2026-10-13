import { screen, waitFor } from "@testing-library/svelte";
import { expect, test } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
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
  });
  // No layer was added.
  expect(sent("perform")).toHaveLength(0);
  // No path drawn: the active layer's, when it is a vector layer.
  await user.click(row("Shape 1"));
  await user.keyboard("{Control>}{Enter}{/Control}");
  await waitFor(() => expect(sent("make_selection")).toHaveLength(2));
  expect(sent("make_selection")[1]).toEqual({ documentId: 1, path: null, layerId: 2 });
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
