import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import type { LayerView } from "../../src/lib/engine";
import PropertiesPanel from "../../src/lib/PropertiesPanel.svelte";
import type { Shape } from "../../src/lib/shapes";

function vectorLayer(shape: Shape): LayerView {
  return {
    id: 7,
    name: "Shape 1",
    visible: true,
    opacity: 1,
    kind: "vector",
    swatch: [0, 0, 0, 1],
    blendMode: "normal",
    contentKey: 0,
    hasAlpha: true,
    mask: null,
    children: [],
    passThrough: false,
    clipped: false,
    transform: [1, 0, 0, 1, 0, 0],
    painted: false,
    entries: [],
    adjustment: null,
    shape,
  };
}

const RECTANGLE: Shape = {
  geometry: { kind: "rectangle", rect: [0, 0, 100, 40], radii: [0, 0, 0, 0] },
  fill: [1, 0, 0, 1],
  stroke: null,
};

function open(shape: Shape) {
  const onedit = vi.fn();
  const onshapecolor = vi.fn();
  render(PropertiesPanel, {
    documentId: 1,
    layer: vectorLayer(shape),
    onedit,
    onlive: vi.fn(),
    ongestureend: vi.fn(),
    onshapecolor,
  });
  return { onedit, onshapecolor, user: userEvent.setup() };
}

/** The shape sent by the last edit. */
const sentShape = (onedit: ReturnType<typeof vi.fn>) => {
  const [documentId, edit] = onedit.mock.lastCall as [
    number,
    { kind: string; id: number; shape: Shape },
  ];
  expect(documentId).toBe(1);
  expect(edit.kind).toBe("setShape");
  expect(edit.id).toBe(7);
  return edit.shape;
};

test("a rectangle's fill and stroke turned on and off, its corners rounded", async () => {
  const { onedit, onshapecolor, user } = open(RECTANGLE);
  expect(screen.getByText("Rectangle")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Fill color" }));
  expect(onshapecolor).toHaveBeenCalledWith(expect.objectContaining({ id: 7 }), "fill");
  // A stroke turned on takes the fill's color, 3 pixels inside.
  await user.click(screen.getByRole("checkbox", { name: "Stroke" }));
  expect(sentShape(onedit).stroke).toEqual({
    color: [1, 0, 0, 1],
    width: 3,
    align: "inside",
    cap: "butt",
    join: "miter",
    dashes: [],
  });
  await user.click(screen.getByRole("checkbox", { name: "Fill" }));
  expect(sentShape(onedit).fill).toBeNull();
  // The radius, never past half the shorter side.
  const radius = screen.getByRole("spinbutton", { name: "Radius:" });
  await user.clear(radius);
  await user.type(radius, "50");
  await user.tab();
  expect(sentShape(onedit).geometry).toEqual({
    kind: "rectangle",
    rect: [0, 0, 100, 40],
    radii: [20, 20, 20, 20],
  });
});

test("a stroke's width and position", async () => {
  const { onedit, user } = open({
    ...RECTANGLE,
    stroke: {
      color: [0, 0, 1, 1],
      width: 2,
      align: "center",
      cap: "butt",
      join: "miter",
      dashes: [],
    },
  });
  const width = screen.getByRole("spinbutton", { name: "Width:" });
  await user.clear(width);
  await user.type(width, "7.5");
  await user.tab();
  expect(sentShape(onedit).stroke?.width).toBe(7.5);
  await user.selectOptions(screen.getByRole("combobox", { name: "Stroke position" }), "Outside");
  expect(sentShape(onedit).stroke?.align).toBe("outside");
});

test("a polygon's sides and star; a line has its stroke only", async () => {
  const { onedit, user } = open({
    geometry: { kind: "polygon", center: [0, 0], radius: 10, sides: 5, star: null, rotation: 90 },
    fill: [0, 0, 0, 1],
    stroke: null,
  });
  const sides = screen.getByRole("spinbutton", { name: "Sides:" });
  await user.clear(sides);
  await user.type(sides, "8");
  await user.tab();
  expect(sentShape(onedit).geometry).toMatchObject({ sides: 8 });
  await user.click(screen.getByRole("checkbox", { name: "Star" }));
  expect(sentShape(onedit).geometry).toMatchObject({ star: 0.5 });
});

test("a line shows its stroke's color and width, nothing to fill", () => {
  open({
    geometry: { kind: "line", from: [0, 0], to: [10, 10] },
    fill: null,
    stroke: {
      color: [0, 0, 0, 1],
      width: 1,
      align: "center",
      cap: "butt",
      join: "miter",
      dashes: [],
    },
  });
  expect(screen.queryByRole("checkbox", { name: "Fill" })).toBeNull();
  expect(screen.queryByRole("combobox", { name: "Stroke position" })).toBeNull();
  expect(screen.getByRole("button", { name: "Stroke color" })).toBeInTheDocument();
  expect(screen.getByRole("spinbutton", { name: "Width:" })).toHaveValue(1);
});
