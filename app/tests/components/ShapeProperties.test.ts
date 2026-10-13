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
    locks: { transparency: false, pixels: false, position: false, all: false },
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

test("a path drawn with the Pen: its fill and stroke, its geometry kept", async () => {
  const geometry = {
    kind: "path" as const,
    rule: "nonzero" as const,
    subpaths: [
      {
        start: [0, 0] as [number, number],
        segments: [
          [10, 0],
          [10, 5, 5, 10, 0, 10],
        ],
        closed: false,
      },
    ],
  };
  const { onedit, user } = open({ geometry, fill: [1, 0, 0, 1], stroke: null });
  expect(screen.getByText("Path")).toBeInTheDocument();
  // No live parameters: no radius nor sides.
  expect(screen.queryByRole("spinbutton", { name: "Radius:" })).toBeNull();
  expect(screen.queryByRole("spinbutton", { name: "Sides:" })).toBeNull();
  await user.click(screen.getByRole("checkbox", { name: "Fill" }));
  expect(sentShape(onedit)).toEqual({ geometry, fill: null, stroke: null });
});

test("W, H, X and Y: the shape's bounds in the document, a typed value reshapes it", async () => {
  const { onedit, user } = open({
    ...RECTANGLE,
    geometry: { kind: "rectangle", rect: [0, 0, 100, 40], radii: [4, 4, 4, 4] },
  });
  const field = (name: string) => screen.getByRole("spinbutton", { name });
  expect(field("W")).toHaveValue(100);
  expect(field("H")).toHaveValue(40);
  expect(field("X")).toHaveValue(0);
  expect(field("Y")).toHaveValue(0);
  await user.clear(field("W"));
  await user.type(field("W"), "50");
  await user.tab();
  expect(sentShape(onedit).geometry).toEqual({
    kind: "rectangle",
    rect: [0, 0, 50, 40],
    radii: [4, 4, 4, 4],
  });
  await user.clear(field("X"));
  await user.type(field("X"), "12");
  await user.tab();
  expect(sentShape(onedit).geometry).toEqual({
    kind: "rectangle",
    rect: [12, 0, 112, 40],
    radii: [4, 4, 4, 4],
  });
  // A size of zero changes nothing.
  const sent = onedit.mock.calls.length;
  await user.clear(field("H"));
  await user.type(field("H"), "0");
  await user.tab();
  expect(onedit).toHaveBeenCalledTimes(sent);
});

test("the bounds through the layer's groups, none for a rotated layer", () => {
  const onedit = vi.fn();
  const layer = vectorLayer(RECTANGLE);
  const group: LayerView = {
    ...vectorLayer(RECTANGLE),
    id: 3,
    kind: "group",
    shape: null,
    transform: [1, 0, 0, 1, 30, 5],
    children: [layer],
  };
  const { unmount } = render(PropertiesPanel, {
    documentId: 1,
    layer,
    layers: [group],
    onedit,
    onlive: vi.fn(),
    ongestureend: vi.fn(),
  });
  expect(screen.getByRole("spinbutton", { name: "X" })).toHaveValue(30);
  expect(screen.getByRole("spinbutton", { name: "Y" })).toHaveValue(5);
  unmount();
  render(PropertiesPanel, {
    documentId: 1,
    layer: { ...layer, transform: [0, 1, -1, 0, 0, 0] },
    onedit,
    onlive: vi.fn(),
    ongestureend: vi.fn(),
  });
  expect(screen.queryByRole("spinbutton", { name: "W" })).toBeNull();
});

test("a rectangle's corners unlinked, each its own radius, linked again", async () => {
  const { onedit, user } = open(RECTANGLE);
  const link = screen.getByRole("button", { name: "Link corner radii" });
  expect(link).toHaveAttribute("aria-pressed", "true");
  await user.click(link);
  expect(link).toHaveAttribute("aria-pressed", "false");
  expect(screen.queryByRole("spinbutton", { name: "Radius:" })).toBeNull();
  const corner = screen.getByRole("spinbutton", { name: "Bottom right radius" });
  await user.clear(corner);
  await user.type(corner, "8");
  await user.tab();
  expect(sentShape(onedit).geometry).toEqual({
    kind: "rectangle",
    rect: [0, 0, 100, 40],
    radii: [0, 0, 8, 0],
  });
});

test("corners rounded differently show each radius; linking gives them the first one's", async () => {
  const { onedit, user } = open({
    ...RECTANGLE,
    geometry: { kind: "rectangle", rect: [0, 0, 100, 40], radii: [6, 2, 3, 4] },
  });
  expect(screen.getByRole("spinbutton", { name: "Top left radius" })).toHaveValue(6);
  expect(screen.getByRole("spinbutton", { name: "Top right radius" })).toHaveValue(2);
  expect(screen.getByRole("spinbutton", { name: "Bottom right radius" })).toHaveValue(3);
  expect(screen.getByRole("spinbutton", { name: "Bottom left radius" })).toHaveValue(4);
  const link = screen.getByRole("button", { name: "Link corner radii" });
  expect(link).toHaveAttribute("aria-pressed", "false");
  await user.click(link);
  expect(sentShape(onedit).geometry).toEqual({
    kind: "rectangle",
    rect: [0, 0, 100, 40],
    radii: [6, 6, 6, 6],
  });
});
