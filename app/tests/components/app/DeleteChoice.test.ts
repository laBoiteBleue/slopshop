import { screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import { documentView, layer, open, respond, sent } from "./harness";

// Delete with a selection on a pixel layer (ADR 0045): the choice of transparency, the
// background or foreground color, or generative fill.

respond("ai_runtime", () => "directml");
respond("fill", (_args, doc) => ({ ...doc, revision: doc.revision + 1 }));
let generativeFails = false;
respond("ai_generative_fill", (_args, doc) => {
  if (generativeFails) {
    generativeFails = false;
    throw { code: "notInstalled", detail: "erase" };
  }
  return { ...doc, revision: doc.revision + 1 };
});
respond("ai_components", () => [
  {
    id: "flux2-klein-4b",
    downloadSize: 7919230622,
    installedSize: 7919230622,
    installed: false,
    licenses: [{ name: "Apache-2.0", url: "https://example.org", commercial: true, accept: false }],
  },
]);

const selected = () => ({ ...documentView(1, "cat.jpg", [layer(1, "Cat")]), selectionKey: 7 });

async function choose(name: string) {
  const user = open(selected());
  await screen.findByText("cat.jpg");
  await user.keyboard("{Delete}");
  await screen.findByRole("dialog", { name: "Delete Selection" });
  await user.click(screen.getByRole("radio", { name }));
  await user.click(screen.getByRole("button", { name: "OK" }));
  return user;
}

test("Delete with a selection asks what replaces it, and Cancel does nothing", async () => {
  const user = open(selected());
  await screen.findByText("cat.jpg");
  await user.keyboard("{Delete}");
  expect(await screen.findByRole("dialog", { name: "Delete Selection" })).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("dialog", { name: "Delete Selection" })).not.toBeInTheDocument();
  expect(sent("fill")).toHaveLength(0);
  expect(sent("ai_generative_fill")).toHaveLength(0);
});

test("Transparency erases the selection, as Delete did", async () => {
  await choose("Transparency");
  await vi.waitFor(() => expect(sent("fill")).toHaveLength(1));
  expect(sent("fill")[0]).toMatchObject({
    documentId: 1,
    layerId: 1,
    target: "layer",
    color: null,
  });
});

test("the background and foreground colors paint the selection", async () => {
  // Black in front, white behind by default.
  await choose("Background Color");
  await vi.waitFor(() => expect(sent("fill")).toHaveLength(1));
  expect(sent("fill")[0]).toMatchObject({ target: "layer", color: [1, 1, 1], opacity: 1 });
  document.body.innerHTML = "";
  await choose("Foreground Color");
  await vi.waitFor(() => expect(sent("fill")).toHaveLength(2));
  expect(sent("fill")[1]).toMatchObject({ target: "layer", color: [0, 0, 0] });
});

test("Generative Fill runs the AI task on the layer", async () => {
  await choose("Generative Fill");
  await vi.waitFor(() => expect(sent("ai_generative_fill")).toHaveLength(1));
  expect(sent("ai_generative_fill")[0]).toEqual({
    documentId: 1,
    layerId: 1,
    task: expect.any(Number),
  });
  expect(sent("fill")).toHaveLength(0);
});

test("on first use, Generative Fill offers to download its model", async () => {
  generativeFails = true;
  await choose("Generative Fill");
  await vi.waitFor(() => expect(sent("ai_components")).toHaveLength(1));
  expect(sent("ai_components")[0]).toEqual({ feature: "erase" });
  expect(await screen.findByText(/FLUX\.2 \[klein\] 4B: generative fill/)).toBeInTheDocument();
});

test("on a mask, Delete paints the background gray at once, without asking", async () => {
  const user = open({ ...selected(), quickMask: true });
  await screen.findByText("cat.jpg");
  await user.keyboard("{Delete}");
  await vi.waitFor(() => expect(sent("fill")).toHaveLength(1));
  expect(sent("fill")[0]).toMatchObject({ target: "quickMask" });
  expect(screen.queryByRole("dialog", { name: "Delete Selection" })).not.toBeInTheDocument();
});
