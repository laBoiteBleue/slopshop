import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import KeyboardShortcutsDialog from "../../src/lib/KeyboardShortcutsDialog.svelte";
import type { Menu } from "../../src/lib/MenuBar.svelte";

const MENUS: Menu[] = [
  {
    label: "Edit",
    items: [
      { kind: "command", label: "Undo", shortcuts: ["Ctrl+Z"], run: () => {} },
      { kind: "command", label: "Fill…", run: () => {} },
      {
        kind: "submenu",
        label: "Transform",
        items: [{ kind: "command", label: "Again", shortcuts: ["Shift+Ctrl+T"], run: () => {} }],
      },
    ],
  },
];

function open() {
  const onclose = vi.fn();
  render(KeyboardShortcutsDialog, {
    menus: MENUS,
    extra: [{ label: "Duplicate", keys: ["Ctrl+J"] }],
    onclose,
  });
  return { onclose, user: userEvent.setup() };
}

const section = (title: string) =>
  screen.queryByRole("heading", { name: title })?.closest("section") ?? null;

/** The keys listed for `label` in a section. */
function keys(title: string, label: string): string[] {
  const entry = [...(section(title)?.querySelectorAll("dt") ?? [])].find(
    (dt) => dt.textContent === label,
  );
  return [...(entry?.nextElementSibling?.querySelectorAll("kbd") ?? [])].map(
    (kbd) => kbd.textContent ?? "",
  );
}

test("the menus' shortcuts, submenus included, the tools' letters and the other keys", () => {
  open();
  expect(keys("Edit", "Undo")).toEqual(["Ctrl+Z"]);
  expect(keys("Edit", "Transform › Again")).toEqual(["Shift+Ctrl+T"]);
  // Without a shortcut, not listed.
  expect(keys("Edit", "Fill…")).toEqual([]);
  // A slot's variants: the letter, and Shift+letter for the next one.
  expect(keys("Tools", "Lasso Tool")).toEqual(["L", "Shift+L"]);
  expect(keys("Tools", "Move Tool")).toEqual(["V"]);
  expect(screen.getByText("Duplicate")).toBeInTheDocument();
});

test("the filter matches a command or a key, and says when nothing does", async () => {
  const { user } = open();
  const filter = screen.getByRole("searchbox", { name: "Search a command or a key" });
  await user.type(filter, "ctrl+z");
  expect(screen.getByText("Undo")).toBeInTheDocument();
  expect(screen.queryByText("Lasso Tool")).not.toBeInTheDocument();
  expect(section("Tools")).toBeNull();
  await user.clear(filter);
  await user.type(filter, "lasso");
  expect(screen.getByText("Polygonal Lasso Tool")).toBeInTheDocument();
  await user.type(filter, "zzz");
  expect(screen.getByText("No shortcut matches.")).toBeInTheDocument();
});

test("Escape and the button close it, and the app's keys wait meanwhile", async () => {
  const { onclose, user } = open();
  const appKeys = vi.fn();
  window.addEventListener("keydown", appKeys);
  await user.keyboard("v");
  expect(appKeys).not.toHaveBeenCalled();
  window.removeEventListener("keydown", appKeys);
  screen.getByRole("dialog").dispatchEvent(new Event("cancel", { cancelable: true }));
  expect(onclose).toHaveBeenCalledOnce();
  await user.click(screen.getByRole("button", { name: "Close" }));
  expect(onclose).toHaveBeenCalledTimes(2);
});
