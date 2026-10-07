import { screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { EditRequest, LayerView } from "../../../src/lib/engine";
import { documentView, layer, open, respond, row, sent } from "./harness";

// Edit > Transform's Distort and Perspective (ADR 0038): Free Transform's box, its corners free.

respond("move_snap_targets", () => ({
  moving: { left: 0, top: 0, right: 100, bottom: 50 },
  others: [],
}));
respond("perform_live", (_, doc) => ({ ...doc, revision: doc.revision + 1 }));
respond("end_gesture", (_, doc) => ({ ...doc, revision: doc.revision + 1 }));

/** Edit > Transform > `entry`, the menu opened by its labels. */
async function transformMenu(user: ReturnType<typeof open>, entry: string) {
  await user.click(screen.getByRole("menuitem", { name: "Edit" }));
  await user.hover(screen.getByText("Transform", { selector: ".label" }));
  await user.click(screen.getByText(entry, { selector: ".dropdown.nested .label" }));
}

/** The box's handle `i` (clockwise from the top left), where it is drawn. */
function handle(i: number): { target: Element; at: [number, number] } {
  const target = document.querySelectorAll(".free-transform .handle")[i];
  const x = Number(target.getAttribute("x")) + 4;
  const y = Number(target.getAttribute("y")) + 4;
  return { target, at: [x, y] };
}

test("Edit > Transform > Distort: a corner dragged sends a map of nine numbers, Enter applies it", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Photo")]));
  await vi.waitFor(() => expect(row("Photo")).toBeInTheDocument());
  await user.click(row("Photo"));
  await transformMenu(user, "Distort");
  await vi.waitFor(() =>
    expect(document.querySelectorAll(".free-transform .handle")).toHaveLength(8),
  );
  const { target, at } = handle(2);
  const svg = document.querySelector(".free-transform") as Element;
  await user.pointer([
    { keys: "[MouseLeft>]", target, coords: { clientX: at[0], clientY: at[1] } },
    { target: svg, coords: { clientX: at[0] - 10, clientY: at[1] + 10 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: at[0] - 10, clientY: at[1] + 10 } },
  ]);
  await vi.waitFor(() => expect(sent("perform_live").length).toBeGreaterThan(0));
  const edit = sent("perform_live").at(-1)?.edit as EditRequest & { matrix: number[] };
  expect(edit.kind).toBe("transformLayers");
  expect(edit.matrix).toHaveLength(9);
  await user.keyboard("{Enter}");
  await vi.waitFor(() => expect(sent("end_gesture")).toHaveLength(1));
});

test("Distort and Perspective are for pixel and vector layers only", async () => {
  const fill: LayerView = { ...layer(1, "Color"), kind: "fill" };
  const user = open(documentView(1, "cat.jpg", [fill]));
  await vi.waitFor(() => expect(row("Color")).toBeInTheDocument());
  await user.click(row("Color"));
  await user.click(screen.getByRole("menuitem", { name: "Edit" }));
  await user.hover(screen.getByText("Transform", { selector: ".label" }));
  for (const name of ["Distort", "Perspective"]) {
    const item = screen.getByText(name, { selector: ".dropdown.nested .label" }).closest(".item");
    expect(item).toHaveAttribute("aria-disabled", "true");
  }
});

test("a vector layer goes in perspective", async () => {
  const shape: LayerView = { ...layer(1, "Rectangle 1"), kind: "vector" };
  const user = open(documentView(1, "cat.jpg", [shape]));
  await vi.waitFor(() => expect(row("Rectangle 1")).toBeInTheDocument());
  await user.click(row("Rectangle 1"));
  await user.click(screen.getByRole("menuitem", { name: "Edit" }));
  await user.hover(screen.getByText("Transform", { selector: ".label" }));
  const item = screen
    .getByText("Perspective", { selector: ".dropdown.nested .label" })
    .closest(".item");
  expect(item).not.toHaveAttribute("aria-disabled", "true");
});
