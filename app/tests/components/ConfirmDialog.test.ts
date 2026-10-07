import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import ConfirmDialog from "../../src/lib/ConfirmDialog.svelte";

test("the question, what it touches, then the action or Cancel", async () => {
  HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
    this.open = true;
  };
  const onconfirm = vi.fn();
  const onclose = vi.fn();
  render(ConfirmDialog, {
    title: "Delete Source",
    message: "Two layers go.",
    items: ["A", "B"],
    action: "Delete",
    onconfirm,
    onclose,
  });
  const user = userEvent.setup();
  expect(screen.getByRole("dialog", { name: "Delete Source" })).toHaveTextContent("Two layers go.");
  expect(screen.getAllByRole("listitem")).toHaveLength(2);
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onclose).toHaveBeenCalled();
  expect(onconfirm).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Delete" }));
  expect(onconfirm).toHaveBeenCalled();
});
