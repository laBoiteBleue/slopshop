import { fireEvent, render, screen, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import PathsPanel from "../../src/lib/PathsPanel.svelte";
import type { PathRef } from "../../src/lib/engine";
import type { PathEntry } from "../../src/lib/paths";

const entry = (ref: PathRef, kind: PathEntry["kind"], name: string): PathEntry => ({
  ref,
  kind,
  name,
  outline: "M0 0 L10 0 L0 10 Z",
  evenOdd: false,
});

const ENTRIES: PathEntry[] = [
  entry({ kind: "path", id: 1 }, "path", "Outline"),
  entry({ kind: "path", id: 2 }, "work", ""),
  entry({ kind: "shape", layer: 7 }, "shape", "Shape 1"),
  entry({ kind: "vectorMask", layer: 7 }, "vectorMask", "Shape 1"),
];

function open(
  props: Partial<{
    entries: PathEntry[];
    chosen: PathRef | null;
    selected: boolean;
    canMask: boolean;
  }> = {},
) {
  const all = {
    entries: ENTRIES,
    chosen: null,
    document: { width: 100, height: 80 },
    selected: true,
    canMask: false,
    nextName: "Path 2",
    onchoose: vi.fn(),
    onload: vi.fn(),
    onfill: vi.fn(),
    onstroke: vi.fn(),
    onworkpath: vi.fn(),
    onmask: vi.fn(),
    onnew: vi.fn(),
    ondelete: vi.fn(),
    onrename: vi.fn(),
    onsave: vi.fn(),
    onduplicate: vi.fn(),
    ...props,
  };
  const view = render(PathsPanel, all);
  return { ...all, view, user: userEvent.setup() };
}

const row = (name: string) => screen.getByRole("option", { name: new RegExp(name) });
const button = (name: string) => screen.getByRole("button", { name });

test("rows: saved paths, the Work Path and the layer's paths in italics, each with a thumbnail", () => {
  open();
  const names = screen.getAllByRole("option").map((o) => o.textContent?.trim());
  expect(names).toEqual(["Outline", "Work Path", "Shape 1 Shape Path", "Shape 1 Vector Mask"]);
  expect(row("Outline")).not.toHaveClass("temporary");
  expect(row("Work Path")).toHaveClass("temporary");
  expect(row("Shape 1 Vector Mask")).toHaveClass("temporary");
  expect(row("Outline").querySelector("path.shown")).toHaveAttribute("d", "M0 0 L10 0 L0 10 Z");
});

test("nothing to list: a hint", () => {
  open({ entries: [] });
  expect(screen.getByText(/No paths/)).toBeInTheDocument();
});

test("a click chooses a path; a press on the empty part chooses none", async () => {
  const { onchoose, user } = open({ chosen: { kind: "path", id: 1 } });
  expect(row("Outline")).toHaveAttribute("aria-selected", "true");
  await user.click(row("Shape Path"));
  expect(onchoose).toHaveBeenLastCalledWith({ kind: "shape", layer: 7 });
  await fireEvent.pointerDown(screen.getByRole("listbox"), { button: 0 });
  expect(onchoose).toHaveBeenLastCalledWith(null);
});

test("Ctrl+click on a thumbnail loads the path as the selection, combined by Shift and Alt", async () => {
  const { onload, onchoose, user } = open();
  const thumb = (name: string) => within(row(name)).getByTestId("path-thumbnail") as HTMLElement;
  await user.keyboard("{Control>}");
  await user.click(thumb("Outline"));
  await user.keyboard("{Shift>}");
  await user.click(thumb("Work Path"));
  await user.keyboard("{Alt>}");
  await user.click(thumb("Vector Mask"));
  await user.keyboard("{/Shift}");
  await user.click(thumb("Outline"));
  await user.keyboard("{/Alt}{/Control}");
  expect(onload.mock.calls).toEqual([
    [{ kind: "path", id: 1 }, "replace"],
    [{ kind: "path", id: 2 }, "add"],
    [{ kind: "vectorMask", layer: 7 }, "intersect"],
    [{ kind: "path", id: 1 }, "subtract"],
  ]);
  // Loading does not choose.
  expect(onchoose).not.toHaveBeenCalled();
});

test("a double-click renames a saved path, saves the Work Path under the name proposed", async () => {
  const { onrename, onsave, user } = open();
  await user.dblClick(row("Outline"));
  const field = screen.getByRole("textbox", { name: "Rename Path" });
  await user.clear(field);
  await user.type(field, "Edge{Enter}");
  expect(onrename).toHaveBeenCalledWith(1, "Edge");
  await user.dblClick(row("Work Path"));
  const save = screen.getByRole("textbox", { name: "Save Path" });
  expect(save).toHaveValue("Path 2");
  await user.type(save, "{Enter}");
  expect(onsave).toHaveBeenCalledWith(2, "Path 2");
  // A layer's path is not renamed here.
  await user.dblClick(row("Shape Path"));
  expect(screen.queryByRole("textbox")).toBeNull();
});

test("the buttons fill, stroke, load, make a work path, mask, add and delete", async () => {
  const chosen = { kind: "path", id: 1 } as const;
  const props = open({ chosen, canMask: true });
  const { user } = props;
  await user.click(button("Fill path with foreground color (Alt: options)"));
  await user.keyboard("{Alt>}");
  await user.click(button("Fill path with foreground color (Alt: options)"));
  await user.click(button("Stroke path with the tool chosen last (Alt: options)"));
  await user.click(button("Make work path from selection (Alt: tolerance)"));
  await user.keyboard("{/Alt}");
  await user.click(button("Stroke path with the tool chosen last (Alt: options)"));
  await user.click(button("Load path as a selection"));
  await user.click(button("Make work path from selection (Alt: tolerance)"));
  await user.click(button("Add a Mask"));
  await user.click(button("New Path"));
  await user.click(button("Delete Path"));
  expect(props.onfill.mock.calls).toEqual([
    [chosen, false],
    [chosen, true],
  ]);
  expect(props.onstroke.mock.calls).toEqual([
    [chosen, true],
    [chosen, false],
  ]);
  expect(props.onworkpath.mock.calls).toEqual([[true], [false]]);
  expect(props.onload).toHaveBeenCalledWith(chosen, "replace");
  expect(props.onmask).toHaveBeenCalledWith(chosen);
  expect(props.onnew).toHaveBeenCalledOnce();
  expect(props.ondelete).toHaveBeenCalledWith(chosen);
});

test("without a path chosen or a selection, the buttons that need them are off", () => {
  open({ selected: false });
  for (const name of [
    "Fill path with foreground color (Alt: options)",
    "Stroke path with the tool chosen last (Alt: options)",
    "Load path as a selection",
    "Make work path from selection (Alt: tolerance)",
    "Add a Mask",
    "Delete Path",
  ]) {
    expect(button(name)).toBeDisabled();
  }
  expect(button("New Path")).toBeEnabled();
});

test("a layer's path is not deleted from the panel; Delete removes a saved one", async () => {
  const { ondelete, user, view } = open({ chosen: { kind: "shape", layer: 7 } });
  expect(button("Delete Path")).toBeDisabled();
  await view.rerender({ chosen: { kind: "path", id: 1 } });
  screen.getByRole("listbox").focus();
  await user.keyboard("{Delete}");
  expect(ondelete).toHaveBeenCalledWith({ kind: "path", id: 1 });
});

test("the right-click menu has every command", async () => {
  const props = open();
  const { user } = props;
  await fireEvent.contextMenu(row("Work Path"), { clientX: 5, clientY: 5 });
  expect(props.onchoose).toHaveBeenCalledWith({ kind: "path", id: 2 });
  const labels = [...document.querySelectorAll(".menu .label, [role=menu] .label")].map(
    (l) => l.textContent,
  );
  expect(labels).toEqual([
    "Save Path",
    "Duplicate Path",
    "Delete Path",
    "Make Work Path…",
    "Make Selection",
    "Fill Path…",
    "Stroke Path…",
    "Add a Mask",
  ]);
  await user.click(screen.getByText("Duplicate Path"));
  expect(props.onduplicate).toHaveBeenCalledWith({ kind: "path", id: 2 }, "Work Path copy");
  await fireEvent.contextMenu(row("Vector Mask"), { clientX: 5, clientY: 5 });
  await user.click(screen.getByText("Fill Path…"));
  expect(props.onfill).toHaveBeenCalledWith({ kind: "vectorMask", layer: 7 }, true);
  await fireEvent.contextMenu(row("Outline"), { clientX: 5, clientY: 5 });
  await user.click(screen.getByText("Stroke Path…"));
  expect(props.onstroke).toHaveBeenCalledWith({ kind: "path", id: 1 }, true);
  await fireEvent.contextMenu(row("Outline"), { clientX: 5, clientY: 5 });
  await user.click(screen.getByText("Make Selection"));
  expect(props.onload).toHaveBeenCalledWith({ kind: "path", id: 1 }, "replace");
});
