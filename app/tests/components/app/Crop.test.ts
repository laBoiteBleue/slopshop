import { screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import { insetAfterTurn } from "../../../src/lib/crop";
import { documentView, layer, open, respond, sent } from "./harness";

// The Crop tool's options: a ratio or a size from the options bar shapes the frame.

test("a ratio chosen in the options bar fits the frame, and Enter crops to it", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("c");
  await vi.waitFor(() => expect(document.querySelectorAll(".handle")).toHaveLength(8));
  await user.selectOptions(screen.getByRole("combobox", { name: "Ratio or size" }), "1 : 1");
  await user.keyboard("{Enter}");
  // The 400 × 300 canvas: the largest square in it, centered.
  await vi.waitFor(() =>
    expect(sent("perform").map((a) => a.edit)).toContainEqual(
      expect.objectContaining({ kind: "crop", x: 50, y: 0, width: 300, height: 300 }),
    ),
  );
});

test("a size fixes the frame, its handles gone", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("c");
  await user.selectOptions(screen.getByRole("combobox", { name: "Ratio or size" }), "Size (px)");
  expect(document.querySelectorAll(".handle")).toHaveLength(0);
});

test("Straighten turns the image to level the line drawn, the frame inside it", async () => {
  respond("perform", (args, doc) => {
    const edit = args.edit as { kind: string };
    // The engine grows the canvas to hold the turned image.
    if (edit.kind === "rotateImageBy") Object.assign(doc, { width: 428, height: 339 });
    return { ...doc, revision: doc.revision + 1 };
  });
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("c");
  await vi.waitFor(() => expect(document.querySelectorAll(".handle")).toHaveLength(8));
  await user.click(screen.getByRole("button", { name: "Straighten" }));
  const svg = document.querySelector("svg.crop") as SVGSVGElement;
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 0, clientY: 0 } },
    { target: svg, coords: { clientX: 100, clientY: 10 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 100, clientY: 10 } },
  ]);
  const degrees = (-Math.atan2(10, 100) * 180) / Math.PI;
  await vi.waitFor(() =>
    expect(sent("perform").map((a) => a.edit)).toContainEqual({ kind: "rotateImageBy", degrees }),
  );
  // Straighten turns itself off; the frame starts inside the turned image.
  expect(screen.getByRole("button", { name: "Straighten" })).toHaveAttribute(
    "aria-pressed",
    "false",
  );
  const inside = insetAfterTurn(400, 300, degrees, { width: 428, height: 339 });
  await vi.waitFor(() => expect(document.querySelectorAll(".handle")).toHaveLength(8));
  await user.keyboard("{Enter}");
  await vi.waitFor(() =>
    expect(sent("perform").map((a) => a.edit)).toContainEqual(
      expect.objectContaining({
        kind: "crop",
        x: inside.left,
        y: inside.top,
        width: inside.right - inside.left,
        height: inside.bottom - inside.top,
      }),
    ),
  );
});
