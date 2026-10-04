import { fireEvent, screen, within } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import { documentView, layer, layerNames, open, sent } from "./harness";

// The app's shell: the welcome page, tabs, File > New, undo, the toolbar's letters, Delete
// and the panels' width.

test("it starts on the welcome page, with the menus", async () => {
  open();
  expect(await screen.findByText("Open an image or create a document")).toBeInTheDocument();
  expect(screen.getByRole("menuitem", { name: "File" })).toBeInTheDocument();
});

test("documents open at startup are tabs; the last one shows its layers and is drawn", async () => {
  open(
    documentView(1, "cat.jpg", [layer(1, "Cat")]),
    documentView(2, "dog.png", [layer(1, "Background"), layer(2, "Dog")]),
  );
  expect(await screen.findByText("cat.jpg")).toBeInTheDocument();
  expect(screen.getByText("dog.png")).toBeInTheDocument();
  await vi.waitFor(() => expect(layerNames()).toEqual(["Dog", "Background"]));
  await vi.waitFor(() =>
    expect(sent("render_view")).toContainEqual(expect.objectContaining({ documentId: 2 })),
  );
});

test("File > New creates an untitled document in a new tab", async () => {
  const user = open();
  await user.click(await screen.findByRole("button", { name: "New document…" }));
  const dialog = screen.getByRole("dialog", { name: "New" });
  await user.selectOptions(within(dialog).getByRole("combobox", { name: "Preset" }), "hd");
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(sent("new_document")).toHaveLength(1));
  expect(sent("new_document")[0]).toMatchObject({ name: null, width: 1920, height: 1080 });
  await vi.waitFor(() => expect(layerNames()).toHaveLength(1));
});

test("Ctrl+Z undoes in the active document", async () => {
  const user = open(documentView(4, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("{Control>}z{/Control}");
  await vi.waitFor(() => expect(sent("undo")).toEqual([{ documentId: 4 }]));
});

test("a tool's letter picks it in the toolbar", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("b");
  expect(screen.getByRole("button", { name: "Brush Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("Delete removes the selected layer, without a selection of pixels", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Background"), layer(2, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Background"]));
  await user.keyboard("{Delete}");
  await vi.waitFor(() =>
    expect(sent("perform")).toContainEqual({
      documentId: 1,
      edit: { kind: "removeLayer", id: 2 },
    }),
  );
  await vi.waitFor(() => expect(layerNames()).toEqual(["Background"]));
});

test("the panels open at the width saved last, and their left edge resizes them", async () => {
  localStorage.setItem("slopshop.panelWidth", "420");
  open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  const edge = await screen.findByRole("separator", { name: "Resize the panels" });
  const main = document.querySelector("main") as HTMLElement;
  expect(main.style.getPropertyValue("--panel-width")).toBe("420px");
  await fireEvent.pointerDown(edge, { pointerId: 1, button: 0, clientX: 600 });
  await fireEvent.pointerMove(edge, { pointerId: 1, clientX: 550 });
  await fireEvent.pointerUp(edge, { pointerId: 1, clientX: 550 });
  expect(main.style.getPropertyValue("--panel-width")).toBe("470px");
  expect(localStorage.getItem("slopshop.panelWidth")).toBe("470");
  localStorage.clear();
});
