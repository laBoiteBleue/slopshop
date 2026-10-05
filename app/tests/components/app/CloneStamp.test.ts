import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { documentView, layer, open, sent } from "./harness";

// The Clone Stamp (S): Alt+click sets where it takes its pixels, a stroke paints them.

const canvas = () => document.querySelector("svg.paint") as SVGSVGElement;

/** A stroke from (`x`, `y`) to (`x + 10`, `y`), Alt held at the press with `alt`. */
async function stroke(x: number, y: number, alt = false) {
  await fireEvent.pointerDown(canvas(), {
    pointerId: 1,
    button: 0,
    clientX: x,
    clientY: y,
    altKey: alt,
  });
  await fireEvent.pointerMove(canvas(), { pointerId: 1, clientX: x + 10, clientY: y });
  await fireEvent.pointerUp(canvas(), { pointerId: 1, clientX: x + 10, clientY: y });
}

async function openImage() {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), layer(2, "Retouch")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("s");
  return user;
}

test("without a source, a stroke paints nothing and says how to set one", async () => {
  await openImage();
  expect(screen.getByRole("button", { name: "Clone Stamp Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await stroke(100, 50);
  expect(await screen.findByText(/Alt\+click where the Clone Stamp/)).toBeInTheDocument();
  expect(sent("paint_stroke")).toEqual([]);
});

test("Alt+click sets the source; strokes then take the pixels that far, aligned", async () => {
  await openImage();
  await stroke(20, 30, true);
  expect(sent("paint_stroke")).toEqual([]);
  expect(document.querySelector(".clone-source")).not.toBeNull();
  await stroke(120, 40);
  await waitFor(() => expect(sent("paint_stroke").length).toBeGreaterThan(0));
  // From the active layer (Current Layer at first), 100 to the left and 10 up.
  expect(sent("paint_stroke")[0]).toMatchObject({
    request: { target: "layer", layerId: 2, clone: { offset: [-100, -10], sourceLayer: 2 } },
  });
  // Aligned: the next stroke keeps that distance, wherever it starts.
  const before = sent("paint_stroke").length;
  await stroke(200, 90);
  await waitFor(() => expect(sent("paint_stroke").length).toBeGreaterThan(before));
  expect(sent("paint_stroke")[before]).toMatchObject({
    request: { clone: { offset: [-100, -10] } },
  });
});

test("not aligned, each stroke starts from the source; All Layers samples the image", async () => {
  const user = await openImage();
  await user.click(screen.getByRole("checkbox", { name: "Aligned" }));
  await user.selectOptions(screen.getByRole("combobox", { name: "Sample:" }), "All Layers");
  await stroke(20, 30, true);
  await stroke(120, 40);
  await waitFor(() => expect(sent("paint_stroke").length).toBeGreaterThan(0));
  const before = sent("paint_stroke").length;
  await stroke(60, 30);
  await waitFor(() => expect(sent("paint_stroke").length).toBeGreaterThan(before));
  expect(sent("paint_stroke")[before]).toMatchObject({
    request: { clone: { offset: [-40, 0], sourceLayer: null } },
  });
});

test("the Healing Brush (J) shares the source and asks the engine to blend", async () => {
  const user = await openImage();
  await stroke(20, 30, true);
  await user.keyboard("j");
  expect(screen.getByRole("button", { name: "Healing Brush Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(document.querySelector(".clone-source")).not.toBeNull();
  await stroke(120, 40);
  await waitFor(() => expect(sent("paint_stroke").length).toBeGreaterThan(0));
  expect(sent("paint_stroke")[0]).toMatchObject({
    request: { clone: { offset: [-100, -10], sourceLayer: 2, heal: true } },
  });
});
