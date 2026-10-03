import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import NewDocumentDialog from "../../src/lib/NewDocumentDialog.svelte";

// The dialog keeps the last settings for the session (module state, as Photoshop's): tests that
// press OK change what the next ones start with, so they come last.

function open(clipboard: [number, number] | null = null) {
  const oncreate = vi.fn();
  const onclose = vi.fn();
  render(NewDocumentDialog, { clipboard, oncreate, onclose });
  return { oncreate, onclose, user: userEvent.setup() };
}

const field = (name: string) => screen.getByRole("spinbutton", { name });
const list = (name: string) => screen.getByRole("combobox", { name });

test("the clipboard's size comes first, chosen", () => {
  open([800, 600]);
  expect(list("Preset")).toHaveValue("clipboard");
  expect(field("Width")).toHaveValue(800);
  expect(field("Height")).toHaveValue(600);
});

test("a preset sets the size, the orientation turns it, typing makes it custom", async () => {
  const { user } = open();
  await user.selectOptions(list("Preset"), "hd");
  expect(field("Width")).toHaveValue(1920);
  expect(field("Height")).toHaveValue(1080);
  expect(field("Resolution")).toHaveValue(72);
  await user.click(screen.getByRole("radio", { name: "Portrait" }));
  expect(field("Width")).toHaveValue(1080);
  expect(field("Height")).toHaveValue(1920);
  // Turned, it is still the preset.
  expect(list("Preset")).toHaveValue("hd");
  await user.clear(field("Width"));
  await user.type(field("Width"), "1000");
  expect(list("Preset")).toHaveValue("custom");
});

test("OK creates the document, named or untitled; its settings come back next time", async () => {
  const { oncreate, user } = open();
  await user.selectOptions(list("Preset"), "square");
  await user.selectOptions(list("Background Contents"), "transparent");
  await user.type(screen.getByRole("textbox", { name: "Name" }), "  Poster ");
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(oncreate).toHaveBeenCalledWith({
    name: "Poster",
    width: 1080,
    height: 1080,
    background: "transparent",
    resolution: 72,
  });
  cleanup();
  const again = open();
  expect(list("Preset")).toHaveValue("square");
  expect(list("Background Contents")).toHaveValue("transparent");
  await again.user.click(screen.getByRole("button", { name: "OK" }));
  expect(again.oncreate).toHaveBeenCalledWith(expect.objectContaining({ name: null }));
});

test("a size in centimeters keeps its length when the resolution changes", async () => {
  const { oncreate, user } = open();
  await user.selectOptions(list("Preset"), "a4");
  await user.selectOptions(screen.getAllByRole("combobox", { name: "Unit" })[0], "cm");
  expect(field("Width")).toHaveValue(21);
  await user.clear(field("Resolution"));
  await user.type(field("Resolution"), "150");
  expect(field("Width")).toHaveValue(21);
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(oncreate).toHaveBeenCalledWith(
    expect.objectContaining({ width: 1240, height: 1754, resolution: 150 }),
  );
});
