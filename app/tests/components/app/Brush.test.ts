import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { documentView, layer, open, sent } from "./harness";

// The Brush's options as the engine gets them.

test("Pencil paints hard pixels: the stroke says so", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("b");
  await user.click(screen.getByRole("checkbox", { name: "Pencil" }));
  const canvas = document.querySelector("svg.paint") as SVGSVGElement;
  await fireEvent.pointerDown(canvas, { pointerId: 1, button: 0, clientX: 10, clientY: 10 });
  await fireEvent.pointerUp(canvas, { pointerId: 1, clientX: 10, clientY: 10 });
  await waitFor(() => expect(sent("paint_stroke").length).toBeGreaterThan(0));
  expect(sent("paint_stroke")[0]).toMatchObject({ request: { brush: { pencil: true } } });
});
