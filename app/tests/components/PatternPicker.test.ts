import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import PatternPicker from "../../src/lib/PatternPicker.svelte";

beforeEach(() => {
  HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
    this.open = true;
  };
  mockIPC((cmd) => {
    if (cmd === "list_patterns") {
      return [
        { id: "builtin:checkers", name: "" },
        { id: "file:pattern-1.png", name: "Bricks" },
      ];
    }
    if (cmd === "pattern_thumbnail") {
      // A 1 × 1 thumbnail.
      const bytes = new Uint8Array([1, 0, 0, 0, 1, 0, 0, 0, 255, 0, 0, 255]);
      return bytes.buffer;
    }
    return null;
  });
});

afterEach(() => clearMocks());

test("the library's patterns are shown, named, and a click picks one", async () => {
  const onpick = vi.fn();
  const onclose = vi.fn();
  render(PatternPicker, { title: "Pattern", onpick, onclose });
  const user = userEvent.setup();
  const bricks = await screen.findByRole("option", { name: "Bricks" });
  expect(screen.getByRole("option", { name: "Checkers" })).toBeInTheDocument();
  await user.click(bricks);
  expect(onpick).toHaveBeenCalledWith({ id: "file:pattern-1.png", name: "Bricks" }, "Bricks");
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onclose).toHaveBeenCalled();
});
