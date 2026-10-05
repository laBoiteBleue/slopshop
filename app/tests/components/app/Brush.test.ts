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

test("Dodge (O) lightens the layer where it paints; Alt burns; not on a mask", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("o");
  const canvas = () => document.querySelector("svg.paint") as SVGSVGElement;
  await fireEvent.pointerDown(canvas(), { pointerId: 1, button: 0, clientX: 10, clientY: 10 });
  await fireEvent.pointerUp(canvas(), { pointerId: 1, clientX: 10, clientY: 10 });
  await waitFor(() => expect(sent("paint_stroke").length).toBeGreaterThan(0));
  expect(sent("paint_stroke")[0]).toMatchObject({
    request: {
      layerId: 1,
      clone: {
        offset: [0, 0],
        sourceLayer: 1,
        tone: { burn: false, range: "midtones", exposure: 0.5 },
      },
    },
  });
  const before = sent("paint_stroke").length;
  await fireEvent.pointerDown(canvas(), {
    pointerId: 1,
    button: 0,
    clientX: 30,
    clientY: 10,
    altKey: true,
  });
  await fireEvent.pointerUp(canvas(), { pointerId: 1, clientX: 30, clientY: 10 });
  await waitFor(() => expect(sent("paint_stroke").length).toBeGreaterThan(before));
  expect(sent("paint_stroke")[before]).toMatchObject({
    request: { clone: { tone: { burn: true } } },
  });
});

test("Dodge and Burn refuse a mask", async () => {
  const user = open({ ...documentView(1, "cat.jpg", [layer(1, "Cat")]), quickMask: true });
  await screen.findByText("cat.jpg");
  await user.keyboard("o");
  const canvas = document.querySelector("svg.paint") as SVGSVGElement;
  await fireEvent.pointerDown(canvas, { pointerId: 1, button: 0, clientX: 10, clientY: 10 });
  await fireEvent.pointerUp(canvas, { pointerId: 1, clientX: 10, clientY: 10 });
  expect(await screen.findByText(/not a mask/)).toBeInTheDocument();
  expect(sent("paint_stroke")).toEqual([]);
});

test("Blur and Sharpen paint the layer through a filter, by their strength", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.click(screen.getByRole("button", { name: "Blur Tool" }));
  const canvas = () => document.querySelector("svg.paint") as SVGSVGElement;
  await fireEvent.pointerDown(canvas(), { pointerId: 1, button: 0, clientX: 10, clientY: 10 });
  await fireEvent.pointerUp(canvas(), { pointerId: 1, clientX: 10, clientY: 10 });
  await waitFor(() => expect(sent("paint_stroke").length).toBeGreaterThan(0));
  expect(sent("paint_stroke")[0]).toMatchObject({
    request: {
      clone: { offset: [0, 0], sourceLayer: 1, filter: { sharpen: false, strength: 0.5 } },
    },
  });
  expect(screen.getByText("Strength:")).toBeInTheDocument();
});
