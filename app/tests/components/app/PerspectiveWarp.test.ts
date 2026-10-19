import { fireEvent, screen } from "@testing-library/svelte";
import { beforeEach, expect, test, vi } from "vitest";
import type { EditRequest, LayerView, WarpGrid, WarpPlanes } from "../../../src/lib/engine";
import { identityGrid } from "../../../src/lib/warpGrid";
import { documentView, layer, open, respond, row, sent } from "./harness";

// Edit > Perspective Warp (ADR 0044): planes drawn over the image in Layout mode, their corners
// moved in Warp mode (the layer warped live), Enter applying them. The view is at 100 %, the
// image at the viewport's corner: CSS pixels are document pixels.

/** A layer without a warp: no plane yet, its 90 × 60 pixels at the image's corner. */
const NONE: WarpPlanes = {
  frame: [0, 0, 90, 60],
  toDocument: [1, 0, 0, 1, 0, 0, 0, 0, 1],
  layout: [],
  warped: [],
  planes: [],
  grid: false,
};
/** A layer warped by one plane, its top left corner moved. */
const WARPED: WarpPlanes = {
  ...NONE,
  layout: [
    [10, 10],
    [80, 10],
    [80, 50],
    [10, 50],
  ],
  warped: [
    [14, 12],
    [80, 10],
    [80, 50],
    [10, 50],
  ],
  planes: [[0, 1, 2, 3]],
};
let planes = NONE;
let grid: WarpGrid | null = null;
beforeEach(() => {
  planes = NONE;
  grid = null;
});
respond("warp_planes", () => planes);
respond("warp_grid", () => grid);
respond("perform_live", (_, doc) => ({ ...doc, revision: doc.revision + 1 }));
respond("end_gesture", (_, doc) => ({ ...doc, revision: doc.revision + 1 }));
respond("cancel_gesture", (_, doc) => doc);

/** Edit > Perspective Warp on the photo layer. */
async function perspectiveWarp() {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Photo")]));
  await vi.waitFor(() => expect(row("Photo")).toBeInTheDocument());
  await user.click(row("Photo"));
  await user.click(screen.getByRole("menuitem", { name: "Edit" }));
  await user.click(screen.getByText("Perspective Warp", { selector: ".label" }));
  return user;
}

const overlay = () => document.querySelector(".perspective-warp");

/** A drag on the image from (`x`, `y`) to (`tx`, `ty`), in document pixels. */
async function drag(svg: Element, [x, y]: number[], [tx, ty]: number[]) {
  await fireEvent.pointerDown(svg, { pointerId: 1, button: 0, clientX: x, clientY: y });
  await fireEvent.pointerMove(svg, { pointerId: 1, clientX: tx, clientY: ty });
  await fireEvent.pointerUp(svg, { pointerId: 1 });
}

const lastEdit = () =>
  sent("perform_live").at(-1)?.edit as Extract<EditRequest, { kind: "perspectiveWarpLayer" }>;

test("Layout mode draws a plane, Warp mode warps the layer live, Enter applies it", async () => {
  const user = await perspectiveWarp();
  await vi.waitFor(() => expect(overlay()).not.toBeNull());
  expect(sent("warp_planes").at(-1)).toMatchObject({ documentId: 1, layerId: 1 });
  expect(screen.getByRole("button", { name: "Layout" })).toHaveAttribute("aria-pressed", "true");
  expect(screen.getByRole("button", { name: "Warp" })).toBeDisabled();
  // A plane drawn: laid out, nothing warped yet.
  await drag(overlay() as Element, [10, 10], [80, 50]);
  await vi.waitFor(() =>
    expect(document.querySelectorAll(".perspective-warp .pin")).toHaveLength(4),
  );
  expect(sent("perform_live")).toHaveLength(0);
  // Warp mode: the plane's warp shows, then follows its corner's drag.
  await user.click(screen.getByRole("button", { name: "Warp" }));
  await vi.waitFor(() => expect(sent("perform_live").length).toBeGreaterThan(0));
  expect(lastEdit()).toMatchObject({ kind: "perspectiveWarpLayer", id: 1 });
  await drag(overlay() as Element, [10, 10], [16, 13]);
  await vi.waitFor(() => expect(lastEdit().planes?.warped[0]).toEqual([16, 13]));
  expect(sent("perform_live").at(-1)).toMatchObject({ replace: true });
  expect(lastEdit().planes?.layout[0]).toEqual([10, 10]);
  await user.keyboard("{Enter}");
  await vi.waitFor(() => expect(sent("end_gesture")).toHaveLength(1));
  expect(lastEdit().planes?.planes).toEqual([[0, 1, 2, 3]]);
  expect(overlay()).toBeNull();
});

test("a layer warped by planes shows them in Warp mode; Layout mode shows it unwarped", async () => {
  planes = WARPED;
  const user = await perspectiveWarp();
  await vi.waitFor(() => expect(overlay()).not.toBeNull());
  expect(screen.getByRole("button", { name: "Warp" })).toHaveAttribute("aria-pressed", "true");
  // The moved corner where it went.
  const pin = document.querySelector(".perspective-warp .pin") as Element;
  expect([pin.getAttribute("cx"), pin.getAttribute("cy")]).toEqual(["14", "12"]);
  await user.click(screen.getByRole("button", { name: "Layout" }));
  await vi.waitFor(() => expect(sent("perform_live").length).toBeGreaterThan(0));
  expect(lastEdit()).toEqual({ kind: "perspectiveWarpLayer", id: 1 });
  // Unchanged: Enter changes nothing.
  await user.keyboard("{Enter}");
  await vi.waitFor(() => expect(sent("cancel_gesture")).toHaveLength(1));
  expect(sent("end_gesture")).toHaveLength(0);
});

test("Esc cancels a perspective warp", async () => {
  planes = WARPED;
  const user = await perspectiveWarp();
  await vi.waitFor(() => expect(overlay()).not.toBeNull());
  await drag(overlay() as Element, [80, 50], [85, 55]);
  await vi.waitFor(() => expect(sent("perform_live").length).toBeGreaterThan(0));
  await user.keyboard("{Escape}");
  await vi.waitFor(() => expect(sent("cancel_gesture")).toHaveLength(1));
  expect(overlay()).toBeNull();
});

test("a layer warped by Warp's grid is not given planes: a message says why", async () => {
  planes = { ...NONE, grid: true };
  await perspectiveWarp();
  await vi.waitFor(() =>
    expect(document.querySelector(".export-result.error")).toHaveTextContent(
      "This layer is warped by Edit > Transform > Warp",
    ),
  );
  expect(overlay()).toBeNull();
});

test("Edit > Transform > Warp on a layer warped by planes says why it does not open", async () => {
  grid = {
    style: null,
    bend: 0,
    vertical: false,
    warped: true,
    perspective: true,
    columns: 3,
    rows: 3,
    frame: [0, 0, 90, 60],
    points: identityGrid([0, 0, 90, 60], 3, 3).points,
    toDocument: [1, 0, 0, 1, 0, 0, 0, 0, 1],
  };
  const user = open(documentView(1, "cat.jpg", [layer(1, "Photo")]));
  await vi.waitFor(() => expect(row("Photo")).toBeInTheDocument());
  await user.click(row("Photo"));
  await user.click(screen.getByRole("menuitem", { name: "Edit" }));
  await user.hover(screen.getByText("Transform", { selector: ".label" }));
  await user.click(screen.getByText("Warp", { selector: ".dropdown.nested .label" }));
  await vi.waitFor(() =>
    expect(document.querySelector(".export-result.error")).toHaveTextContent(
      "This layer has a perspective warp",
    ),
  );
  expect(document.querySelector(".warp-grid")).toBeNull();
});

test("Perspective Warp is for pixel layers only", async () => {
  const fill: LayerView = { ...layer(1, "Color"), kind: "fill" };
  const user = open(documentView(1, "cat.jpg", [fill]));
  await vi.waitFor(() => expect(row("Color")).toBeInTheDocument());
  await user.click(row("Color"));
  await user.click(screen.getByRole("menuitem", { name: "Edit" }));
  const item = screen.getByText("Perspective Warp", { selector: ".label" }).closest(".item");
  expect(item).toHaveAttribute("aria-disabled", "true");
});
