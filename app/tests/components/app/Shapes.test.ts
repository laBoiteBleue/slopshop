import { screen, waitFor, within } from "@testing-library/svelte";
import { expect, test } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
import { documentView, layer, open, row, sent } from "./harness";

// The shape tools (U, ADR 0041): a drag on the image adds a vector layer of the shape.

function drag(user: ReturnType<typeof open>, from: [number, number], to: [number, number]) {
  const svg = document.querySelector("svg.shape-tool") as SVGSVGElement;
  return user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: from[0], clientY: from[1] } },
    { target: svg, coords: { clientX: to[0], clientY: to[1] } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: to[0], clientY: to[1] } },
  ]);
}

test("U picks the Rectangle: a drag adds a filled vector layer above the active one", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("u");
  expect(screen.getByRole("button", { name: "Rectangle Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await drag(user, [10, 20], [110, 70]);
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("perform")[0]).toMatchObject({
    documentId: 1,
    edit: {
      kind: "addShape",
      name: "Rectangle 1",
      shape: {
        geometry: { kind: "rectangle", rect: [10, 20, 110, 70], radii: [0, 0, 0, 0] },
        // The foreground color, no stroke.
        fill: [0, 0, 0, 1],
        stroke: null,
      },
      parent: null,
      index: 1,
    },
  });
});

test("Shift+U goes to the Ellipse; the options bar's stroke is drawn, the fill left out", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("u");
  await user.keyboard("{Shift>}u{/Shift}");
  expect(screen.getByRole("button", { name: "Ellipse Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await user.click(screen.getByRole("checkbox", { name: "Fill" }));
  await user.click(screen.getByRole("checkbox", { name: "Stroke" }));
  await user.selectOptions(screen.getByRole("combobox", { name: "Stroke position" }), "Outside");
  await drag(user, [0, 0], [40, 20]);
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("perform")[0]).toMatchObject({
    edit: {
      kind: "addShape",
      name: "Ellipse 1",
      shape: {
        geometry: { kind: "ellipse", center: [20, 10], radii: [20, 10] },
        fill: null,
        stroke: { color: [0, 0, 0, 1], width: 3, align: "outside" },
      },
    },
  });
});

test("a swatch opens the color picker; the color chosen fills the next shape", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("u");
  await user.click(screen.getByRole("button", { name: "Fill color" }));
  const dialog = await screen.findByRole("dialog");
  const hex = within(dialog).getByRole("textbox");
  await user.clear(hex);
  await user.type(hex, "ff0000");
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await drag(user, [0, 0], [30, 30]);
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("perform")[0]).toMatchObject({ edit: { shape: { fill: [1, 0, 0, 1] } } });
});

test("a vector layer's colors are changed in Properties, its shape kept", async () => {
  const shape = {
    ...layer(2, "Ellipse 1"),
    kind: "vector" as const,
    shape: {
      geometry: { kind: "ellipse", center: [20, 10], radii: [20, 10] },
      fill: [0, 0, 0, 1],
      stroke: null,
    },
  } as LayerView;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), shape]));
  await waitFor(() => expect(row("Ellipse 1")).toBeInTheDocument());
  await user.click(row("Ellipse 1"));
  const panel = screen.getByRole("tabpanel", { name: "Properties" });
  await user.click(within(panel).getByRole("button", { name: "Fill color" }));
  const dialog = await screen.findByRole("dialog");
  const hex = within(dialog).getByRole("textbox");
  await user.clear(hex);
  await user.type(hex, "0000ff");
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("perform")[0]).toMatchObject({
    documentId: 1,
    edit: {
      kind: "setShape",
      id: 2,
      shape: {
        geometry: { kind: "ellipse", center: [20, 10], radii: [20, 10] },
        fill: [0, 0, 1, 1],
        stroke: null,
      },
    },
  });
});
