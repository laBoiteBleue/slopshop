import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import SelectionsPanel from "../../src/lib/SelectionsPanel.svelte";

function open(
  saved = [
    { id: 1, name: "Hair" },
    { id: 2, name: "Shirt" },
  ],
  selected = true,
) {
  const props = {
    saved,
    selected,
    onload: vi.fn(),
    onsave: vi.fn(),
    onreplace: vi.fn(),
    onrename: vi.fn(),
    ondelete: vi.fn(),
  };
  render(SelectionsPanel, props);
  return { ...props, user: userEvent.setup() };
}

const row = (name: string) => screen.getByRole("option", { name: new RegExp(name) });

test("a click loads a saved selection; Shift, Alt and both combine it", async () => {
  const { onload, user } = open();
  await user.click(row("Hair"));
  await user.keyboard("{Shift>}");
  await user.click(row("Shirt"));
  await user.keyboard("{/Shift}{Alt>}");
  await user.click(row("Hair"));
  await user.keyboard("{Shift>}");
  await user.click(row("Shirt"));
  await user.keyboard("{/Shift}{/Alt}");
  expect(onload.mock.calls).toEqual([
    [1, "replace"],
    [2, "add"],
    [1, "subtract"],
    [2, "intersect"],
  ]);
});

test("a double-click renames in place; Escape keeps the name", async () => {
  const { onrename, user } = open();
  await user.dblClick(row("Hair"));
  const field = screen.getByRole("textbox", { name: "Rename" });
  await user.clear(field);
  await user.type(field, "Hair and beard{Enter}");
  expect(onrename).toHaveBeenCalledWith(1, "Hair and beard");
  await user.dblClick(row("Shirt"));
  await user.type(screen.getByRole("textbox", { name: "Rename" }), "x{Escape}");
  expect(onrename).toHaveBeenCalledOnce();
});

test("the right-click menu combines, replaces, renames and deletes", async () => {
  const { onload, onreplace, ondelete, user } = open();
  await fireEvent.contextMenu(row("Shirt"), { clientX: 5, clientY: 5 });
  await user.click(screen.getByText("Intersect with Selection"));
  await fireEvent.contextMenu(row("Shirt"), { clientX: 5, clientY: 5 });
  await user.click(screen.getByText("Replace with Current Selection"));
  await fireEvent.contextMenu(row("Hair"), { clientX: 5, clientY: 5 });
  await user.click(screen.getByText("Delete Saved Selection"));
  expect(onload).toHaveBeenCalledWith(2, "intersect");
  expect(onreplace).toHaveBeenCalledWith(2);
  expect(ondelete).toHaveBeenCalledWith(1);
});

test("+ saves the current selection; the trash and Delete remove the row last clicked", async () => {
  const { onsave, ondelete, user } = open();
  expect(screen.getByRole("button", { name: "Delete Saved Selection" })).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Save the current selection" }));
  expect(onsave).toHaveBeenCalledOnce();
  await user.click(row("Shirt"));
  await user.click(screen.getByRole("button", { name: "Delete Saved Selection" }));
  expect(ondelete).toHaveBeenLastCalledWith(2);
  await user.click(row("Hair"));
  await user.keyboard("{Delete}");
  expect(ondelete).toHaveBeenLastCalledWith(1);
});

test("without a selection, nothing to save or replace with; without saved ones, a hint", async () => {
  open([], false);
  expect(screen.getByText(/No saved selection/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Save the current selection" })).toBeDisabled();
});

test("a press on the empty part of the list deselects the row, as in the Layers panel", async () => {
  const { ondelete, user } = open();
  await user.click(row("Shirt"));
  expect(row("Shirt")).toHaveAttribute("aria-selected", "true");
  await fireEvent.pointerDown(screen.getByRole("listbox"), { button: 0 });
  expect(row("Shirt")).toHaveAttribute("aria-selected", "false");
  expect(screen.getByRole("button", { name: "Delete Saved Selection" })).toBeDisabled();
  await user.keyboard("{Delete}");
  expect(ondelete).not.toHaveBeenCalled();
});

test("a press anywhere but on a row deselects it (the image, another panel); the trash keeps it", async () => {
  const { ondelete, user } = open();
  await user.click(row("Shirt"));
  // The trash acts on the row clicked last.
  await user.click(screen.getByRole("button", { name: "Delete Saved Selection" }));
  expect(ondelete).toHaveBeenLastCalledWith(2);
  await user.click(row("Hair"));
  await fireEvent.pointerDown(document.body, { button: 0 });
  expect(row("Hair")).toHaveAttribute("aria-selected", "false");
  expect(screen.getByRole("button", { name: "Delete Saved Selection" })).toBeDisabled();
});

test("a press on the empty part of the list deselects in the image; elsewhere, not", async () => {
  const ondeselect = vi.fn();
  render(SelectionsPanel, {
    saved: [{ id: 1, name: "Hair" }],
    selected: true,
    onload: vi.fn(),
    onsave: vi.fn(),
    onreplace: vi.fn(),
    onrename: vi.fn(),
    ondelete: vi.fn(),
    ondeselect,
  });
  await fireEvent.pointerDown(screen.getByRole("option", { name: /Hair/ }), { button: 0 });
  await fireEvent.pointerDown(document.body, { button: 0 });
  expect(ondeselect).not.toHaveBeenCalled();
  await fireEvent.pointerDown(screen.getByRole("listbox"), { button: 0 });
  expect(ondeselect).toHaveBeenCalledOnce();
});
