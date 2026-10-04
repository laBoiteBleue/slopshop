import { screen } from "@testing-library/svelte";
import { expect, onTestFinished, test, vi } from "vitest";
import type { Guide } from "../../../src/lib/engine";
import { documentView, layer, menuLabels, open, respond, sent } from "./harness";

// The View menu (zoom, Hide Extras, Snap, Full Screen) and Window > Hide Panels.

/** The menu entry labelled `name` of the open menu. */
const entry = (name: string) => screen.getByText(name, { selector: ".label" }).closest("li")!;

async function openMenu(user: ReturnType<typeof open>, name: string) {
  await user.click(screen.getByRole("menuitem", { name }));
}

test("the View menu: zoom, then the extras and Snap, then full screen", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await openMenu(user, "View");
  expect(menuLabels()).toEqual([
    "Zoom In",
    "Zoom Out",
    "Fit on Screen",
    "100%",
    "—",
    "Rulers",
    "Clear Guides",
    "Hide Extras",
    "Snap",
    "—",
    "Full Screen",
  ]);
  expect(entry("Hide Extras")).toHaveTextContent("Ctrl+H");
  expect(entry("Rulers")).toHaveTextContent("Ctrl+R");
  expect(entry("Full Screen")).toHaveTextContent("F11");
});

test("Ctrl+H hides the selection's outline and shows it again, the selection kept", async () => {
  // A triangle outline, whatever the selection (see `parseOutline` in engine.ts).
  respond("selection_outline", () => new Uint32Array([1, 3, 10, 10, 50, 10, 50, 50]).buffer);
  // jsdom lays nothing out: the overlay is told its size.
  for (const name of ["clientWidth", "clientHeight"]) {
    Object.defineProperty(HTMLElement.prototype, name, { configurable: true, get: () => 200 });
  }
  onTestFinished(() => {
    for (const name of ["clientWidth", "clientHeight"]) {
      delete (HTMLElement.prototype as unknown as Record<string, unknown>)[name];
    }
  });
  const user = open({ ...documentView(1, "cat.jpg", [layer(1, "Cat")]), selectionKey: 7 });
  await screen.findByText("cat.jpg");
  const ants = () => document.querySelector("path.ants");
  await vi.waitFor(() => expect(ants()).toBeInTheDocument());
  await user.keyboard("{Control>}h{/Control}");
  expect(ants()).not.toBeInTheDocument();
  await openMenu(user, "View");
  expect(entry("Hide Extras")).toHaveAttribute("aria-checked", "true");
  await user.click(entry("Hide Extras"));
  await vi.waitFor(() => expect(ants()).toBeInTheDocument());
  // Nothing was asked of the engine: the selection is the same.
  expect(sent("deselect")).toEqual([]);
});

test("View > Snap is remembered for the next session", async () => {
  localStorage.clear();
  onTestFinished(() => localStorage.clear());
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await openMenu(user, "View");
  expect(entry("Snap")).toHaveAttribute("aria-checked", "true");
  await user.click(entry("Snap"));
  expect(JSON.parse(localStorage.getItem("slopshop.view")!)).toEqual({
    rulers: false,
    rulerUnit: "px",
    snap: false,
  });
  await openMenu(user, "View");
  expect(entry("Snap")).toHaveAttribute("aria-checked", "false");
});

test("F11 puts the window in full screen and takes it out", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("{F11}");
  await vi.waitFor(() =>
    expect(sent("plugin:window|set_fullscreen")).toEqual([
      expect.objectContaining({ value: true }),
    ]),
  );
  await openMenu(user, "View");
  expect(entry("Full Screen")).toHaveAttribute("aria-checked", "true");
  await user.click(entry("Full Screen"));
  await vi.waitFor(() =>
    expect(sent("plugin:window|set_fullscreen").at(-1)).toEqual(
      expect.objectContaining({ value: false }),
    ),
  );
});

test("Tab hides the toolbar, the options bar and the panels; a Window panel shows them again", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  const hidden = () => document.querySelectorAll(".chrome.hidden, .sidebar.hidden").length;
  expect(hidden()).toBe(0);
  await user.keyboard("{Tab}");
  // The options bar, the toolbar and the right column.
  expect(hidden()).toBe(3);
  // Still there, hidden: the layers keep their selection.
  expect(screen.getByText("Cat", { selector: "li .name" })).toBeInTheDocument();
  await openMenu(user, "Window");
  expect(menuLabels()).toEqual([
    "Properties",
    "Selections",
    "History",
    "Histogram",
    "Info",
    "—",
    "Options Bar",
    "Toolbar",
    "Hide Panels",
    "—",
    "Reset Layout",
  ]);
  expect(entry("Hide Panels")).toHaveTextContent("Tab");
  expect(entry("Hide Panels")).toHaveAttribute("aria-checked", "true");
  expect(entry("Properties")).toHaveAttribute("aria-checked", "false");
  await user.click(entry("Properties"));
  expect(hidden()).toBe(0);
  await user.keyboard("{Tab}");
  await user.keyboard("{Tab}");
  expect(hidden()).toBe(0);
});

test("Ctrl+R shows the rulers, remembered for the next session", async () => {
  localStorage.clear();
  onTestFinished(() => localStorage.clear());
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  expect(document.querySelector("svg.ruler")).not.toBeInTheDocument();
  await user.keyboard("{Control>}r{/Control}");
  await vi.waitFor(() => expect(document.querySelectorAll("svg.ruler")).toHaveLength(2));
  expect(JSON.parse(localStorage.getItem("slopshop.view")!)).toEqual({
    rulers: true,
    rulerUnit: "px",
    snap: true,
  });
  await openMenu(user, "View");
  expect(entry("Rulers")).toHaveAttribute("aria-checked", "true");
  await user.click(entry("Rulers"));
  expect(document.querySelector("svg.ruler")).not.toBeInTheDocument();
});

test("the document's guides are drawn; View > Clear Guides deletes them all, one undo entry", async () => {
  const user = open({
    ...documentView(1, "cat.jpg", [layer(1, "Cat")]),
    guides: [
      { vertical: true, position: 20 },
      { vertical: false, position: 30 },
    ],
  });
  await screen.findByText("cat.jpg");
  await vi.waitFor(() =>
    expect(document.querySelectorAll("svg.guides line.guide")).toHaveLength(2),
  );
  await openMenu(user, "View");
  await user.click(entry("Clear Guides"));
  await vi.waitFor(() =>
    expect(sent("perform")).toContainEqual(
      expect.objectContaining({ edit: { kind: "setGuides", guides: [] } }),
    ),
  );
});

test("Clear Guides waits for a guide", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await openMenu(user, "View");
  expect(entry("Clear Guides")).toHaveAttribute("aria-disabled", "true");
});

test("a guide placed while Crop is open holds the frame's edges too", async () => {
  respond("move_snap_targets", () => ({ moving: null, others: [] }));
  respond("perform", (args, doc) => {
    const edit = args.edit as { kind: string; guides?: Guide[] };
    if (edit.kind === "setGuides") doc.guides = edit.guides ?? [];
    return { ...doc, revision: doc.revision + 1 };
  });
  // jsdom lays nothing out: every element is the 400 × 300 image's area, at 100%.
  const rect = Object.getOwnPropertyDescriptor(Element.prototype, "getBoundingClientRect");
  Element.prototype.getBoundingClientRect = () => DOMRect.fromRect({ width: 400, height: 300 });
  onTestFinished(() => {
    if (rect) Object.defineProperty(Element.prototype, "getBoundingClientRect", rect);
  });
  localStorage.setItem("slopshop.view", JSON.stringify({ rulers: true, snap: true }));
  onTestFinished(() => localStorage.clear());
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("c");
  await vi.waitFor(() => expect(document.querySelectorAll(".handle")).toHaveLength(8));
  // Crop is open; a vertical guide is dragged out of the left ruler to x = 250.
  const left = document.querySelectorAll("svg.ruler")[1];
  await user.pointer([
    { keys: "[MouseLeft>]", target: left, coords: { clientX: 5, clientY: 150 } },
    { target: left, coords: { clientX: 250, clientY: 150 } },
    { keys: "[/MouseLeft]", target: left, coords: { clientX: 250, clientY: 150 } },
  ]);
  await vi.waitFor(() =>
    expect(sent("perform").map((a) => a.edit)).toContainEqual({
      kind: "setGuides",
      guides: [{ vertical: true, position: 250 }],
    }),
  );
  // The frame's right edge dragged near it snaps onto it.
  const right = document.querySelectorAll(".handle")[3];
  const svg = right.closest("svg")!;
  await user.pointer([
    { keys: "[MouseLeft>]", target: right, coords: { clientX: 400, clientY: 150 } },
    { target: svg, coords: { clientX: 246, clientY: 150 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 246, clientY: 150 } },
  ]);
  await user.keyboard("{Enter}");
  await vi.waitFor(() =>
    expect(sent("perform").map((a) => a.edit)).toContainEqual(
      expect.objectContaining({ kind: "crop", x: 0, width: 250 }),
    ),
  );
});

test("a right-click on a ruler chooses its unit, remembered", async () => {
  localStorage.setItem("slopshop.view", JSON.stringify({ rulers: true, snap: true }));
  onTestFinished(() => localStorage.clear());
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await vi.waitFor(() => expect(document.querySelectorAll("svg.ruler")).toHaveLength(2));
  const top = document.querySelector("svg.ruler")!;
  await user.pointer({ keys: "[MouseRight]", target: top, coords: { clientX: 40, clientY: 5 } });
  const labels = [...document.querySelectorAll(".context-menu .label")].map((l) => l.textContent);
  // Photoshop's order.
  expect(labels).toEqual(["Pixels", "Inches", "Centimeters", "Millimeters"]);
  expect(screen.getByRole("menuitemradio", { name: /Pixels/ })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  await user.click(screen.getByText("Inches"));
  expect(JSON.parse(localStorage.getItem("slopshop.view")!)).toMatchObject({ rulerUnit: "in" });
  expect(screen.queryByText("Inches")).not.toBeInTheDocument();
});
