import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { documentView, layer, open, respond, row, sent } from "./harness";

// The Move tool on the image: layers picked under the pointer, in the Layers panel's selection.

// Bottom to top: Sky, Tree, Bird; each shows its pixels in a band of 100 document pixels (the
// view is at 100%, the image at the viewport's corner: CSS pixels are document pixels).
respond("layer_at", (args) => {
  const x = args.x as number;
  return x < 100 ? 1 : x < 200 ? 2 : x < 300 ? 3 : null;
});
respond("move_snap_targets", () => ({ moving: null, others: [] }));
respond("perform_live", (_args, doc) => doc);
respond("end_gesture", (_args, doc) => doc);
respond("cancel_gesture", (_args, doc) => doc);

const selected = () =>
  [...document.querySelectorAll("li.selected .name")].map((el) => el.textContent?.trim());

async function openImage() {
  open(documentView(1, "trees.jpg", [layer(1, "Sky"), layer(2, "Tree"), layer(3, "Bird")]));
  await screen.findByText("trees.jpg");
  const area = document.querySelector(".viewport") as HTMLElement;
  await waitFor(() => expect(sent("render_view").length).toBeGreaterThan(0));
  return area;
}

/** A press at (`x`, 10) with `keys`, a drag by `dx`, then the release. */
async function press(area: HTMLElement, x: number, keys: { shiftKey?: boolean } = {}, dx = 0) {
  await fireEvent.pointerDown(area, { pointerId: 1, button: 0, clientX: x, clientY: 10, ...keys });
  // The engine answers which layer is there before the release.
  await new Promise((resolve) => setTimeout(resolve, 0));
  if (dx !== 0) {
    await fireEvent.pointerMove(area, { pointerId: 1, clientX: x + dx, clientY: 10, ...keys });
  }
  await fireEvent.pointerUp(area, { pointerId: 1, clientX: x + dx, clientY: 10, ...keys });
  await new Promise((resolve) => setTimeout(resolve, 0));
}

test("a click on a layer's pixels selects it alone; Shift+click adds or removes one", async () => {
  const area = await openImage();
  expect(selected()).toEqual(["Bird"]);
  await press(area, 150);
  await waitFor(() => expect(selected()).toEqual(["Tree"]));
  await press(area, 50, { shiftKey: true });
  await waitFor(() => expect(selected()).toEqual(["Tree", "Sky"]));
  // The layer clicked last is the active one.
  expect(row("Sky")).toHaveAttribute("aria-current", "true");
  // Shift+click again takes it out, and nothing moves.
  await press(area, 50, { shiftKey: true }, 20);
  await waitFor(() => expect(selected()).toEqual(["Tree"]));
  expect(sent("perform_live")).toEqual([]);
});

test("a drag from a layer of a multiple selection moves them all; a click selects it alone", async () => {
  const area = await openImage();
  await press(area, 150);
  await press(area, 50, { shiftKey: true });
  await waitFor(() => expect(selected()).toEqual(["Tree", "Sky"]));
  await press(area, 150, {}, 30);
  await waitFor(() => expect(sent("perform_live").length).toBeGreaterThan(0));
  expect(sent("perform_live").at(-1)).toMatchObject({
    edit: { kind: "translateLayers", ids: [1, 2], dx: 30, dy: 0 },
  });
  expect(selected()).toEqual(["Tree", "Sky"]);
  expect(row("Tree")).toHaveAttribute("aria-current", "true");
  await press(area, 50);
  await waitFor(() => expect(selected()).toEqual(["Sky"]));
});

test("Shift pressed during a drag keeps it on one axis", async () => {
  const area = await openImage();
  await fireEvent.pointerDown(area, { pointerId: 1, button: 0, clientX: 150, clientY: 10 });
  await new Promise((resolve) => setTimeout(resolve, 0));
  await fireEvent.pointerMove(area, { pointerId: 1, clientX: 190, clientY: 25, shiftKey: true });
  await fireEvent.pointerUp(area, { pointerId: 1, clientX: 190, clientY: 25 });
  await waitFor(() => expect(sent("perform_live").length).toBeGreaterThan(0));
  expect(sent("perform_live").at(-1)).toMatchObject({
    edit: { kind: "translateLayers", ids: [2], dx: 40, dy: 0 },
  });
});
