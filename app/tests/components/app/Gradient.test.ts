import { screen, waitFor } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { documentView, layer, open, respond, sent } from "./harness";

// The Gradient tool (G): a line drawn on the image lays the gradient along it.

respond("paint_gradient", (_args, doc) => ({ ...doc, revision: doc.revision + 1 }));

test("G picks the Gradient: a line drawn lays the foreground to the background as paint", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("g");
  expect(screen.getByRole("button", { name: "Gradient Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  const svg = document.querySelector("svg.gradient-tool") as SVGSVGElement;
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 10, clientY: 20 } },
    { target: svg, coords: { clientX: 210, clientY: 20 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 210, clientY: 20 } },
  ]);
  await waitFor(() => expect(sent("paint_gradient")).toHaveLength(1));
  expect(sent("paint_gradient")[0]).toEqual({
    documentId: 1,
    layerId: 1,
    target: "layer",
    gradient: {
      // The default colors: black to white.
      stops: [
        [0, 0, 0, 0],
        [4096, 255, 255, 255],
      ],
      alpha: [1, 1],
      shape: "linear",
      from: [10, 20],
      to: [210, 20],
    },
    opacity: 1,
  });
});

test("the options bar's gradient, shape and Reverse are sent; in Quick Mask, its mask", async () => {
  const user = open({ ...documentView(1, "cat.jpg", [layer(1, "Cat")]), quickMask: true });
  await screen.findByText("cat.jpg");
  await user.keyboard("g");
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Gradient" }),
    "Foreground to Transparent",
  );
  await user.click(screen.getByRole("button", { name: "Radial Gradient" }));
  await user.click(screen.getByRole("checkbox", { name: "Reverse" }));
  const svg = document.querySelector("svg.gradient-tool") as SVGSVGElement;
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 50, clientY: 50 } },
    { target: svg, coords: { clientX: 90, clientY: 50 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 90, clientY: 50 } },
  ]);
  await waitFor(() => expect(sent("paint_gradient")).toHaveLength(1));
  expect(sent("paint_gradient")[0]).toMatchObject({
    target: "quickMask",
    layerId: 0,
    gradient: { alpha: [0, 1], shape: "radial" },
  });
});
