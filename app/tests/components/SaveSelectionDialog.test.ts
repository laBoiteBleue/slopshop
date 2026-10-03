import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import SaveSelectionDialog from "../../src/lib/SaveSelectionDialog.svelte";

function open(saved = [{ id: 3, name: "Selection 1" }]) {
  const onapply = vi.fn();
  const onclose = vi.fn();
  render(SaveSelectionDialog, { saved, onapply, onclose });
  return { onapply, onclose, user: userEvent.setup() };
}

const field = () => screen.getByRole("textbox", { name: "Name:" });

test("it proposes the first free name, and OK saves a new selection under it", async () => {
  const { onapply, user } = open();
  expect(screen.getByRole("dialog", { name: "Save Selection" })).toBeInTheDocument();
  expect(field()).toHaveValue("Selection 2");
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onapply).toHaveBeenCalledWith("Selection 2", null);
});

test("a name already used says so, and replaces that saved selection", async () => {
  const { onapply, user } = open();
  await user.clear(field());
  await user.type(field(), "Selection 1");
  expect(screen.getByRole("status")).toHaveTextContent(
    "“Selection 1” will be replaced by the current selection.",
  );
  await user.click(screen.getByRole("button", { name: "Replace" }));
  expect(onapply).toHaveBeenCalledWith("Selection 1", 3);
});

test("a name is needed; Enter saves, Escape and Cancel close", async () => {
  const { onapply, onclose, user } = open([]);
  await user.clear(field());
  expect(screen.getByRole("button", { name: "OK" })).toBeDisabled();
  await user.type(field(), " Hair {Enter}");
  expect(onapply).toHaveBeenCalledWith("Hair", null);
  // Escape: the dialog's cancel event (jsdom does not raise it from the key).
  screen.getByRole("dialog").dispatchEvent(new Event("cancel", { cancelable: true }));
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onclose).toHaveBeenCalledTimes(2);
});
