import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import MenuBar, { type Menu } from "../../src/lib/MenuBar.svelte";

function menus() {
  const run = {
    open: vi.fn(),
    save: vi.fn(),
    undo: vi.fn(),
    rotate: vi.fn(),
    flip: vi.fn(),
  };
  const bar: Menu[] = [
    {
      label: "File",
      items: [
        { kind: "command", label: "Open…", shortcut: "Ctrl+O", run: run.open },
        { kind: "separator" },
        { kind: "command", label: "Save", disabled: true, run: run.save },
      ],
    },
    {
      label: "Edit",
      items: [
        { kind: "command", label: "Undo", run: run.undo },
        {
          kind: "submenu",
          label: "Transform",
          items: [
            { kind: "command", label: "Rotate 180°", run: run.rotate },
            { kind: "command", label: "Flip Horizontal", run: run.flip },
          ],
        },
      ],
    },
  ];
  return { bar, run };
}

function open() {
  const { bar, run } = menus();
  const onopen = vi.fn();
  render(MenuBar, { menus: bar, onopen });
  return { run, onopen, user: userEvent.setup() };
}

const title = (name: string) => screen.getByRole("menuitem", { name });
const item = (name: string) => screen.getByText(name).closest("li") as HTMLElement;

test("a click opens a menu and runs a command, which closes it", async () => {
  const { run, onopen, user } = open();
  await user.click(title("File"));
  expect(title("File")).toHaveAttribute("aria-expanded", "true");
  expect(onopen).toHaveBeenCalledWith(0);
  expect(screen.getByText("Ctrl+O")).toBeInTheDocument();
  await user.click(item("Open…"));
  expect(run.open).toHaveBeenCalledOnce();
  expect(screen.queryByText("Open…")).not.toBeInTheDocument();
});

test("a disabled command does nothing", async () => {
  const { run, user } = open();
  await user.click(title("File"));
  expect(item("Save")).toHaveAttribute("aria-disabled", "true");
  await user.click(item("Save"));
  expect(run.save).not.toHaveBeenCalled();
  expect(screen.getByText("Save")).toBeInTheDocument();
});

test("once a menu is open, the pointer goes from menu to menu", async () => {
  const { onopen, user } = open();
  await user.click(title("File"));
  await user.hover(title("Edit"));
  expect(title("Edit")).toHaveAttribute("aria-expanded", "true");
  expect(title("File")).toHaveAttribute("aria-expanded", "false");
  expect(onopen).toHaveBeenLastCalledWith(1);
});

test("a submenu opens on hover and runs its commands", async () => {
  const { run, user } = open();
  await user.click(title("Edit"));
  await user.hover(item("Transform"));
  await user.click(item("Flip Horizontal"));
  expect(run.flip).toHaveBeenCalledOnce();
});

test("arrows skip separators and disabled entries, Enter runs, Escape closes", async () => {
  const { run, user } = open();
  await user.click(title("File"));
  await user.keyboard("{ArrowDown}");
  expect(item("Open…")).toHaveClass("current");
  // Down again: Save is disabled, the separator is skipped, back to Open….
  await user.keyboard("{ArrowDown}");
  expect(item("Open…")).toHaveClass("current");
  await user.keyboard("{ArrowRight}");
  expect(title("Edit")).toHaveAttribute("aria-expanded", "true");
  await user.keyboard("{ArrowDown}{ArrowDown}{ArrowRight}{Enter}");
  expect(run.rotate).toHaveBeenCalledOnce();
  await user.click(title("File"));
  await user.keyboard("{Escape}");
  expect(screen.queryByText("Open…")).not.toBeInTheDocument();
});

test("while a menu is open, its keys do not reach the app", async () => {
  const { user } = open();
  const appKeys = vi.fn();
  window.addEventListener("keydown", appKeys);
  await user.click(title("File"));
  await user.keyboard("{ArrowDown}{Escape}");
  expect(appKeys).not.toHaveBeenCalled();
  // Closed, keys go on.
  await user.keyboard("{ArrowDown}");
  expect(appKeys).toHaveBeenCalledOnce();
  window.removeEventListener("keydown", appKeys);
});

test("a press elsewhere closes the menu", async () => {
  const { user } = open();
  await user.click(title("File"));
  await user.click(document.body);
  expect(screen.queryByText("Open…")).not.toBeInTheDocument();
});
