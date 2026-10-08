import { screen, waitFor, within } from "@testing-library/svelte";
import { expect, test } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
import { documentView, layer, open, row, sent } from "./harness";

// The Pen (P) and Direct Selection (A), ADR 0041: a path drawn on the image becomes a vector
// layer; its anchors are dragged again, live, one undo entry a drag.

type User = ReturnType<typeof open>;

function click(user: User, svg: Element, x: number, y: number) {
  return user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: x, clientY: y } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: x, clientY: y } },
  ]);
}

/** A vector layer of a triangle drawn with the Pen. */
const triangle = {
  ...layer(2, "Shape 1"),
  kind: "vector" as const,
  shape: {
    geometry: {
      kind: "path",
      evenOdd: false,
      subpaths: [
        {
          start: [10, 10],
          closed: true,
          segments: [
            { kind: "line", to: [90, 10] },
            { kind: "line", to: [50, 70] },
            { kind: "line", to: [10, 10] },
          ],
        },
      ],
    },
    fill: [0, 0, 0, 1],
    stroke: null,
  },
} as LayerView;

test("P picks the Pen: anchors clicked and the path closed add a filled vector layer", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("p");
  expect(screen.getByRole("button", { name: "Pen Tool" })).toHaveAttribute("aria-pressed", "true");
  // The shape tools' fill and stroke in the options bar.
  expect(screen.getByRole("checkbox", { name: "Fill" })).toBeChecked();
  const svg = document.querySelector("svg.pen-tool") as SVGSVGElement;
  await click(user, svg, 10, 10);
  await click(user, svg, 90, 10);
  await click(user, svg, 50, 70);
  await click(user, svg, 10, 10);
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("perform")[0]).toMatchObject({
    documentId: 1,
    edit: { kind: "addShape", name: "Shape 1", shape: triangle.shape, parent: null, index: 1 },
  });
});

test("A drags an anchor of the active path layer: live, then one undo entry", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), triangle]));
  await waitFor(() => expect(row("Shape 1")).toBeInTheDocument());
  await user.click(row("Shape 1"));
  await user.keyboard("a");
  expect(screen.getByRole("button", { name: "Direct Selection Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  const svg = document.querySelector("svg.direct-selection") as SVGSVGElement;
  expect(svg.querySelectorAll("rect.anchor")).toHaveLength(3);
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 50, clientY: 70 } },
    { target: svg, coords: { clientX: 50, clientY: 90 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 50, clientY: 90 } },
  ]);
  await waitFor(() => expect(sent("end_gesture")).toHaveLength(1));
  expect(sent("perform_live").at(-1)).toMatchObject({
    documentId: 1,
    replace: true,
    edit: {
      kind: "setShape",
      id: 2,
      shape: {
        geometry: {
          kind: "path",
          subpaths: [
            {
              segments: [
                { kind: "line", to: [90, 10] },
                { kind: "line", to: [50, 90] },
                { kind: "line", to: [10, 10] },
              ],
            },
          ],
        },
        fill: [0, 0, 0, 1],
      },
    },
  });
});

test("A on a rectangle asks to turn it into a path; Continue does", async () => {
  const rectangle = {
    ...layer(2, "Rectangle 1"),
    kind: "vector" as const,
    shape: {
      geometry: { kind: "rectangle", rect: [10, 10, 50, 30], radii: [0, 0, 0, 0] },
      fill: [1, 0, 0, 1],
      stroke: null,
    },
  } as LayerView;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), rectangle]));
  await waitFor(() => expect(row("Rectangle 1")).toBeInTheDocument());
  await user.click(row("Rectangle 1"));
  await user.keyboard("a");
  const svg = document.querySelector("svg.direct-selection") as SVGSVGElement;
  await click(user, svg, 10, 10);
  const dialog = await screen.findByRole("dialog", { name: "Turn into a Path" });
  await user.click(within(dialog).getByRole("button", { name: "Continue" }));
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("perform")[0]).toMatchObject({
    edit: {
      kind: "setShape",
      id: 2,
      shape: {
        geometry: {
          kind: "path",
          subpaths: [
            {
              start: [10, 10],
              closed: true,
              segments: [
                { kind: "line", to: [50, 10] },
                { kind: "line", to: [50, 30] },
                { kind: "line", to: [10, 30] },
                { kind: "line", to: [10, 10] },
              ],
            },
          ],
        },
        fill: [1, 0, 0, 1],
      },
    },
  });
});
