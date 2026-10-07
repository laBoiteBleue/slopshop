import { screen, waitFor, within } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
import { documentView, layer, open, respond, row, sent } from "./harness";

// Patterns (ADR 0042): pattern fill layers from the library, Edit > Define Pattern.

respond("list_patterns", () => [
  { id: "builtin:checkers", name: "" },
  { id: "builtin:dots", name: "" },
  { id: "file:pattern-1.png", name: "Pattern 1" },
]);
respond("pattern_thumbnail", () => new Uint8Array([1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 255]).buffer);
respond("add_pattern_fill", (_args, doc) => ({ ...doc, revision: doc.revision + 1 }));
respond("layer_thumbnail", () => new Uint8Array([1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 255]).buffer);
respond("replace_pattern", (_args, doc) => ({ ...doc, revision: doc.revision + 1 }));
respond("load_pattern", () => 77);
respond("define_pattern", (args) => ({
  id: "file:pattern-2.png",
  name: (args as { name: string }).name,
}));

async function menu(user: ReturnType<typeof open>, top: string, ...path: string[]) {
  await user.click(screen.getByRole("menuitem", { name: top }));
  for (const [i, name] of path.entries()) {
    if (i < path.length - 1) await user.hover(screen.getByText(name, { selector: ".label" }));
    else
      await user.click(
        screen.getByText(name, {
          selector: i === 0 ? ".dropdown .label" : ".dropdown.nested .label",
        }),
      );
  }
}

test("Layer > New Fill Layer > Pattern adds the pattern chosen above the active layer", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await menu(user, "Layer", "New Fill Layer", "Pattern…");
  await user.click(await screen.findByRole("option", { name: "Dots" }));
  await waitFor(() => expect(sent("add_pattern_fill")).toHaveLength(1));
  expect(sent("add_pattern_fill")[0]).toEqual({
    documentId: 1,
    pattern: "builtin:dots",
    sourceName: "Dots",
    name: "Pattern Fill 1",
    parent: null,
    index: 1,
  });
});

test("Edit > Define Pattern names the next pattern and adds it to the library", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await menu(user, "Edit", "Define Pattern…");
  const field = await screen.findByRole("textbox", { name: "Name" });
  // "Pattern 1" is taken.
  expect(field).toHaveValue("Pattern 2");
  await user.click(screen.getByRole("button", { name: "OK" }));
  await waitFor(() => expect(sent("define_pattern")).toHaveLength(1));
  expect(sent("define_pattern")[0]).toEqual({ documentId: 1, name: "Pattern 2" });
});

test("a pattern fill's scale and angle are changed in Properties, its pattern replaced", async () => {
  const fill = {
    ...layer(2, "Pattern Fill 1"),
    kind: "patternFill" as const,
    pattern: { source: 5, scale: 1, angle: 0 },
  } as LayerView;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), fill]));
  await vi.waitFor(() => expect(row("Pattern Fill 1")).toBeInTheDocument());
  await user.click(row("Pattern Fill 1"));
  const panel = screen.getByRole("tabpanel", { name: "Properties" });
  const scale = within(panel).getByRole("spinbutton", { name: "Scale" });
  await user.clear(scale);
  await user.type(scale, "250");
  await user.tab();
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("perform")[0]).toMatchObject({
    edit: { kind: "setPatternFill", id: 2, scale: 2.5, angle: 0 },
  });
  await user.click(within(panel).getByRole("button", { name: "Choose another pattern" }));
  await user.click(await screen.findByRole("option", { name: "Checkers" }));
  await waitFor(() => expect(sent("replace_pattern")).toHaveLength(1));
  expect(sent("replace_pattern")[0]).toEqual({
    documentId: 1,
    layerId: 2,
    pattern: "builtin:checkers",
    sourceName: "Checkers",
  });
});

test("Layer > Layer Style > Pattern Overlay asks for the pattern, then sends it live", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await vi.waitFor(() => expect(row("Cat")).toBeInTheDocument());
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  await user.hover(screen.getByText("Layer Style", { selector: ".label" }));
  await user.click(screen.getByText("Pattern Overlay…", { selector: ".label" }));
  // The picker first, the dialog behind it.
  await user.click(await screen.findByRole("option", { name: "Dots" }));
  await waitFor(() => expect(sent("load_pattern")).toHaveLength(1));
  expect(sent("load_pattern")[0]).toEqual({
    documentId: 1,
    pattern: "builtin:dots",
    sourceName: "Dots",
  });
  await vi.waitFor(() =>
    expect(sent("perform_live")).toContainEqual(
      expect.objectContaining({
        edit: expect.objectContaining({
          kind: "setLayerStyle",
          id: 1,
          style: expect.objectContaining({
            patternOverlay: expect.objectContaining({ enabled: true, source: 77, scale: 1 }),
          }),
        }),
      }),
    ),
  );
  const dialog = await screen.findByRole("dialog", { name: "Layer Style" });
  expect(within(dialog).getByRole("checkbox", { name: "Pattern Overlay" })).toBeChecked();
});
