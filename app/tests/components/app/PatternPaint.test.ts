import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { documentView, layer, open, respond, sent } from "./harness";

// Patterns painted (ADR 0042): Edit > Fill > Pattern and the Pattern Stamp (S, with the Clone
// Stamp).

respond("list_patterns", () => [
  { id: "builtin:checkers", name: "" },
  { id: "builtin:dots", name: "" },
]);
respond("pattern_thumbnail", () => new Uint8Array([1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 255]).buffer);
respond("load_pattern", () => 77);

const canvas = () => document.querySelector("svg.paint") as SVGSVGElement;

/** A stroke from (`x`, `y`) to (`x + 10`, `y`). */
async function stroke(x: number, y: number) {
  await fireEvent.pointerDown(canvas(), { pointerId: 1, button: 0, clientX: x, clientY: y });
  await fireEvent.pointerMove(canvas(), { pointerId: 1, clientX: x + 10, clientY: y });
  await fireEvent.pointerUp(canvas(), { pointerId: 1, clientX: x + 10, clientY: y });
}

test("Edit > Fill > Pattern paints the pattern chosen, loaded for the document", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("{Shift>}{F5}{/Shift}");
  let dialog = await screen.findByRole("dialog", { name: "Fill" });
  await user.selectOptions(within(dialog).getByRole("combobox", { name: "Contents:" }), "pattern");
  // The first generated pattern at first; its swatch opens the picker.
  await user.click(within(dialog).getByRole("button", { name: "Pattern: Checkers" }));
  await user.click(await screen.findByRole("option", { name: "Dots" }));
  dialog = await screen.findByRole("dialog", { name: "Fill" });
  expect(within(dialog).getByRole("combobox", { name: "Contents:" })).toHaveValue("pattern");
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await waitFor(() => expect(sent("fill")).toHaveLength(1));
  expect(sent("load_pattern")[0]).toEqual({
    documentId: 1,
    pattern: "builtin:dots",
    sourceName: "Dots",
  });
  expect(sent("fill")[0]).toMatchObject({
    documentId: 1,
    layerId: 1,
    target: "layer",
    color: null,
    opacity: 1,
    stroke: null,
    pattern: 77,
  });
  // The pattern comes back next time.
  await user.keyboard("{Shift>}{F5}{/Shift}");
  dialog = await screen.findByRole("dialog", { name: "Fill" });
  expect(within(dialog).getByRole("button", { name: "Pattern: Dots" })).toBeInTheDocument();
});

test("the Pattern Stamp paints its pattern, anchored at the origin or where a stroke starts", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  // S picks the Clone Stamp, Shift+S its variant.
  await user.keyboard("s{Shift>}s{/Shift}");
  expect(screen.getByRole("button", { name: "Pattern Stamp Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await user.click(screen.getByRole("button", { name: "Pattern painted: Checkers" }));
  await user.click(await screen.findByRole("option", { name: "Dots" }));
  expect(screen.getByRole("button", { name: "Pattern painted: Dots" })).toBeInTheDocument();
  await stroke(120, 40);
  await waitFor(() => expect(sent("paint_stroke").length).toBeGreaterThan(0));
  expect(sent("load_pattern")[0]).toMatchObject({ documentId: 1, pattern: "builtin:dots" });
  // Aligned (the default): from the document's origin.
  expect(sent("paint_stroke")[0]).toMatchObject({
    request: { target: "layer", layerId: 1, pattern: { source: 77, origin: [0, 0] } },
  });
  // Not aligned: from where each stroke starts.
  await user.click(screen.getByRole("checkbox", { name: "Aligned" }));
  const before = sent("paint_stroke").length;
  await stroke(60, 30);
  await waitFor(() => expect(sent("paint_stroke").length).toBeGreaterThan(before));
  const request = (sent("paint_stroke")[before] as { request: Record<string, unknown> }).request;
  const [x, y] = (request.samples as number[][])[0];
  expect(request.pattern).toEqual({ source: 77, origin: [x, y] });
  // Loaded once for the document.
  expect(sent("load_pattern")).toHaveLength(1);
});
