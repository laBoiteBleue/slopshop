import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import ContextMenu from "../../src/lib/ContextMenu.svelte";
import type { MenuItem } from "../../src/lib/MenuBar.svelte";

function open() {
  const run = { copy: vi.fn(), paste: vi.fn(), del: vi.fn() };
  const items: MenuItem[] = [
    { kind: "command", label: "Copy", run: run.copy },
    { kind: "command", label: "Paste", disabled: true, run: run.paste },
    { kind: "separator" },
    { kind: "command", label: "Delete", shortcut: "Del", run: run.del },
  ];
  const onclose = vi.fn();
  render(ContextMenu, { x: 10, y: 20, items, onclose });
  return { run, onclose, user: userEvent.setup() };
}

const item = (name: string) => screen.getByText(name).closest("li") as HTMLElement;

test("a command runs and closes the menu; a disabled one does nothing", async () => {
  const { run, onclose, user } = open();
  await user.click(item("Paste"));
  expect(run.paste).not.toHaveBeenCalled();
  expect(onclose).not.toHaveBeenCalled();
  await user.click(item("Delete"));
  expect(onclose).toHaveBeenCalledOnce();
  expect(run.del).toHaveBeenCalledOnce();
});

test("a right-button release on an entry runs it too (press, drag, release)", async () => {
  const { run, user } = open();
  await user.pointer({ keys: "[MouseRight]", target: item("Copy") });
  expect(run.copy).toHaveBeenCalledOnce();
});

test("arrows skip disabled entries and separators, Enter runs, Escape closes", async () => {
  const { run, onclose, user } = open();
  await user.keyboard("{ArrowDown}{ArrowDown}");
  expect(item("Delete")).toHaveClass("current");
  await user.keyboard("{ArrowDown}");
  expect(item("Copy")).toHaveClass("current");
  await user.keyboard("{Enter}");
  expect(run.copy).toHaveBeenCalledOnce();
  await user.keyboard("{Escape}");
  expect(onclose).toHaveBeenCalledTimes(2);
});

test("a press outside closes it", async () => {
  const { onclose, user } = open();
  await user.click(document.body);
  expect(onclose).toHaveBeenCalledOnce();
});

test("a choice shows its check mark and says it is checked", () => {
  const items: MenuItem[] = [
    { kind: "command", label: "Pixels", checked: true, run: vi.fn() },
    { kind: "command", label: "Inches", checked: false, run: vi.fn() },
    { kind: "command", label: "Copy", run: vi.fn() },
  ];
  render(ContextMenu, { x: 0, y: 0, items, onclose: vi.fn() });
  expect(screen.getByRole("menuitemradio", { name: /Pixels/ })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  expect(item("Pixels")).toHaveTextContent("✓");
  expect(screen.getByRole("menuitemradio", { name: /Inches/ })).toHaveAttribute(
    "aria-checked",
    "false",
  );
  expect(screen.getByRole("menuitem", { name: /Copy/ })).not.toHaveAttribute("aria-checked");
});
