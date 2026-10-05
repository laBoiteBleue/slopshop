import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { documentView, layer, open, respond, sent } from "./harness";

// The Paint Bucket (G): a click fills the region of a similar color with the foreground color.

respond("paint_bucket", (_args, doc) => ({ ...doc, revision: doc.revision + 1 }));

const surface = () => document.querySelector(".bucket") as HTMLElement;

test("Shift+G picks the Paint Bucket after the Gradient: a click fills the active layer's region with the foreground", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), layer(2, "Paint")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("{Shift>}g{/Shift}");
  expect(screen.getByRole("button", { name: "Paint Bucket Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await fireEvent.pointerDown(surface(), { button: 0, clientX: 30.6, clientY: 20.2 });
  await waitFor(() => expect(sent("paint_bucket")).toHaveLength(1));
  expect(sent("paint_bucket")[0]).toEqual({
    documentId: 1,
    x: 30,
    y: 20,
    tolerance: 32,
    contiguous: true,
    antiAlias: true,
    // The active layer is sampled, as the Magic Wand does without Sample All Layers.
    sampleLayer: 2,
    layerId: 2,
    target: "layer",
    color: [0, 0, 0],
    opacity: 1,
    task: expect.any(Number),
  });
});

test("Sample All Layers samples the image as shown; outside the canvas, nothing", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("{Shift>}g{/Shift}");
  await user.click(screen.getByRole("checkbox", { name: "Sample All Layers" }));
  await fireEvent.pointerDown(surface(), { button: 0, clientX: 500, clientY: 20 });
  await fireEvent.pointerDown(surface(), { button: 0, clientX: 10, clientY: 10 });
  await waitFor(() => expect(sent("paint_bucket")).toHaveLength(1));
  expect(sent("paint_bucket")[0]).toMatchObject({ x: 10, y: 10, sampleLayer: null });
});

test("in Quick Mask, the bucket fills the mask", async () => {
  const user = open({ ...documentView(1, "cat.jpg", [layer(1, "Cat")]), quickMask: true });
  await screen.findByText("cat.jpg");
  await user.keyboard("{Shift>}g{/Shift}");
  await fireEvent.pointerDown(surface(), { button: 0, clientX: 10, clientY: 10 });
  await waitFor(() => expect(sent("paint_bucket")).toHaveLength(1));
  expect(sent("paint_bucket")[0]).toMatchObject({ target: "quickMask", layerId: 0 });
});
