import { screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import { documentView, layer, open, sent } from "./harness";

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
