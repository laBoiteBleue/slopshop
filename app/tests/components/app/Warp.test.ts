import { fireEvent, screen } from "@testing-library/svelte";
import { beforeEach, expect, test, vi } from "vitest";
import type { EditRequest, LayerView, WarpGrid } from "../../../src/lib/engine";
import { identityGrid } from "../../../src/lib/warpGrid";
import { documentView, layer, open, respond, row, sent } from "./harness";

// Edit > Transform > Warp (ADR 0044): the layer's grid over the image, a style in the options
// bar or drags on the grid warping it live, Enter applying it. The view is at 100 %, the image
// at the viewport's corner: CSS pixels are document pixels.

/** A layer without a warp: a plain 3 × 3 grid over its 90 × 60 pixels, at the image's corner. */
const GRID: WarpGrid = {
  style: null,
  bend: 0,
  vertical: false,
  warped: false,
  perspective: false,
  columns: 3,
  rows: 3,
  frame: [0, 0, 90, 60],
  points: identityGrid([0, 0, 90, 60], 3, 3).points,
  toDocument: [1, 0, 0, 1, 0, 0, 0, 0, 1],
};
let grid = GRID;
beforeEach(() => {
  grid = GRID;
});
respond("warp_grid", () => grid);
respond("perform_live", (_, doc) => ({ ...doc, revision: doc.revision + 1 }));
respond("end_gesture", (_, doc) => ({ ...doc, revision: doc.revision + 1 }));
respond("cancel_gesture", (_, doc) => doc);

/** Edit > Transform > Warp, the menu opened by its labels. */
async function warpMenu(user: ReturnType<typeof open>) {
  await user.click(screen.getByRole("menuitem", { name: "Edit" }));
  await user.hover(screen.getByText("Transform", { selector: ".label" }));
  await user.click(screen.getByText("Warp", { selector: ".dropdown.nested .label" }));
}

test("Edit > Transform > Warp: the grid shows, a style warps the layer live, Enter applies it", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Photo")]));
  await vi.waitFor(() => expect(row("Photo")).toBeInTheDocument());
  await user.click(row("Photo"));
  await warpMenu(user);
  await vi.waitFor(() => expect(document.querySelectorAll(".warp-grid .line")).toHaveLength(8));
  expect(sent("warp_grid").at(-1)).toMatchObject({ documentId: 1, layerId: 1 });
  await user.selectOptions(screen.getByRole("combobox", { name: "Warp" }), "Arc");
  await vi.waitFor(() => expect(sent("perform_live").length).toBeGreaterThan(0));
  const edit = sent("perform_live").at(-1)?.edit as EditRequest;
  expect(edit).toEqual({ kind: "warpLayer", id: 1, style: "arc", bend: 0.5, vertical: false });
  // The grid is asked for again once warped.
  await vi.waitFor(() => expect(sent("warp_grid").length).toBeGreaterThan(1));
  await user.keyboard("{Enter}");
  await vi.waitFor(() => expect(sent("end_gesture")).toHaveLength(1));
  expect(document.querySelector(".warp-grid")).toBeNull();
});

test("Esc cancels a warp; unchanged, Enter changes nothing", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Photo")]));
  await vi.waitFor(() => expect(row("Photo")).toBeInTheDocument());
  await user.click(row("Photo"));
  await warpMenu(user);
  await vi.waitFor(() => expect(document.querySelector(".warp-grid")).not.toBeNull());
  await user.selectOptions(screen.getByRole("combobox", { name: "Warp" }), "Bulge");
  await user.keyboard("{Escape}");
  await vi.waitFor(() => expect(sent("cancel_gesture")).toHaveLength(1));
  expect(document.querySelector(".warp-grid")).toBeNull();
  await warpMenu(user);
  await vi.waitFor(() => expect(document.querySelector(".warp-grid")).not.toBeNull());
  await user.keyboard("{Enter}");
  await vi.waitFor(() => expect(sent("cancel_gesture")).toHaveLength(2));
  expect(sent("end_gesture")).toHaveLength(0);
});

test("Warp is for pixel layers only", async () => {
  const fill: LayerView = { ...layer(1, "Color"), kind: "fill" };
  const user = open(documentView(1, "cat.jpg", [fill]));
  await vi.waitFor(() => expect(row("Color")).toBeInTheDocument());
  await user.click(row("Color"));
  await user.click(screen.getByRole("menuitem", { name: "Edit" }));
  await user.hover(screen.getByText("Transform", { selector: ".label" }));
  const item = screen.getByText("Warp", { selector: ".dropdown.nested .label" }).closest(".item");
  expect(item).toHaveAttribute("aria-disabled", "true");
});

/** Edit > Transform > Warp on the photo layer, once its grid shows. */
async function warpPhoto() {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Photo")]));
  await vi.waitFor(() => expect(row("Photo")).toBeInTheDocument());
  await user.click(row("Photo"));
  await warpMenu(user);
  await vi.waitFor(() => expect(document.querySelector(".warp-grid")).not.toBeNull());
  return { user, svg: document.querySelector(".warp-grid") as Element };
}

/** A drag on the grid from (`x`, `y`) to (`tx`, `ty`), in document pixels. */
async function drag(svg: Element, [x, y]: number[], [tx, ty]: number[]) {
  await fireEvent.pointerDown(svg, { pointerId: 1, button: 0, clientX: x, clientY: y });
  await fireEvent.pointerMove(svg, { pointerId: 1, clientX: tx, clientY: ty });
  await fireEvent.pointerUp(svg, { pointerId: 1 });
}

test("dragging the grid's corner sends the grid edited point by point, as Custom", async () => {
  const { user, svg } = await warpPhoto();
  await drag(svg, [0, 0], [6, 4]);
  await vi.waitFor(() => expect(sent("perform_live").length).toBeGreaterThan(0));
  const last = sent("perform_live").at(-1);
  expect(last).toMatchObject({ replace: true });
  const edit = last?.edit as Extract<EditRequest, { kind: "warpLayer" }>;
  expect(edit).toMatchObject({ kind: "warpLayer", id: 1, grid: { columns: 3, rows: 3 } });
  expect(edit.grid?.points[0]).toEqual([6, 4]);
  expect(screen.getByRole("combobox", { name: "Warp" })).toHaveValue("custom");
  await user.keyboard("{Enter}");
  await vi.waitFor(() => expect(sent("end_gesture")).toHaveLength(1));
});

test("a grid size: plain without a warp; a warped grid keeps its shape at the new size", async () => {
  const { user } = await warpPhoto();
  await user.selectOptions(screen.getByRole("combobox", { name: "Grid" }), "5 × 5");
  // Nothing is warped yet: a finer plain grid, nothing sent.
  await vi.waitFor(() => expect(document.querySelectorAll(".warp-grid .anchor")).toHaveLength(36));
  expect(sent("perform_live")).toHaveLength(0);
  // Warped by a style, then 4 × 4: the grid is sent, Custom.
  grid = { ...GRID, style: "arc", bend: 0.5, warped: true };
  await user.selectOptions(screen.getByRole("combobox", { name: "Warp" }), "Arc");
  await vi.waitFor(() => expect(sent("warp_grid").length).toBeGreaterThan(1));
  await user.selectOptions(screen.getByRole("combobox", { name: "Grid" }), "4 × 4");
  await vi.waitFor(() =>
    expect((sent("perform_live").at(-1)?.edit as { grid?: unknown }).grid).toBeDefined(),
  );
  const edit = sent("perform_live").at(-1)?.edit as Extract<EditRequest, { kind: "warpLayer" }>;
  expect(edit.grid?.columns).toBe(4);
  expect(edit.grid?.points).toHaveLength(13 * 13);
  expect(screen.getByRole("combobox", { name: "Warp" })).toHaveValue("custom");
});

test("Split Warp Crosswise, then a click on the grid: a row and a column more", async () => {
  const { user, svg } = await warpPhoto();
  await user.click(screen.getByRole("button", { name: "Split Warp Crosswise" }));
  await fireEvent.pointerDown(svg, { pointerId: 1, button: 0, clientX: 45, clientY: 25 });
  await vi.waitFor(() => expect(sent("perform_live").length).toBeGreaterThan(0));
  const edit = sent("perform_live").at(-1)?.edit as Extract<EditRequest, { kind: "warpLayer" }>;
  expect([edit.grid?.columns, edit.grid?.rows]).toEqual([4, 4]);
  // One split per click: the mode is off again; the grid has 4 × 4 patches.
  expect(screen.getByRole("button", { name: "Split Warp Crosswise" })).toHaveAttribute(
    "aria-pressed",
    "false",
  );
  expect(screen.getByRole("combobox", { name: "Grid" })).toHaveValue("4");
});

test("Warp on a layer warped point by point shows its grid again, as Custom", async () => {
  const custom = identityGrid([0, 0, 90, 60], 4, 4);
  custom.points[0] = [-3, -3];
  grid = { ...GRID, warped: true, columns: 4, rows: 4, points: custom.points };
  await warpPhoto();
  expect(screen.getByRole("combobox", { name: "Warp" })).toHaveValue("custom");
  expect(screen.getByRole("combobox", { name: "Grid" })).toHaveValue("4");
  const anchor = document.querySelector(".warp-grid .anchor") as Element;
  expect([anchor.getAttribute("x"), anchor.getAttribute("y")]).toEqual(["-6.5", "-6.5"]);
});
