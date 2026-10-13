import { screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { EditRequest, LayerView } from "../../../src/lib/engine";
import { documentView, layer, open, respond, row, sent } from "./harness";

// Edit > Transform > Warp (ADR 0044): the layer's grid over the image, a style in the options
// bar warping it live, Enter applying it.

const GRID = {
  style: null,
  bend: 0,
  vertical: false,
  lines: [
    [
      [0, 0],
      [100, 0],
    ],
    [
      [0, 0],
      [0, 50],
    ],
  ],
};
respond("warp_grid", () => GRID);
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
  await vi.waitFor(() => expect(document.querySelectorAll(".warp-grid .line")).toHaveLength(2));
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
