import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import { expect, test } from "vitest";
import type { EditRequest, LayerView, TextDto } from "../../../src/lib/engine";
import { documentView, layer, open, respond, row, sent } from "./harness";

// The Type tool (T, ADR 0041): a click places a text layer and types into it, live, until the
// text is committed; a click on a text layer edits it; the options bar sets the font.

const FAMILY = "Inter Variable";

function text(content: string, position: [number, number] = [30, 40]): TextDto {
  return {
    content,
    family: FAMILY,
    weight: 400,
    italic: false,
    size: 12,
    color: [0, 0, 0, 1],
    tracking: 0,
    leading: null,
    align: "left",
    position,
  };
}

function textLayer(id: number, content: string, family = FAMILY): LayerView {
  return {
    ...layer(id, content),
    kind: "vector",
    shape: null,
    text: { ...text(content), family },
  };
}

/** The layer under the pointer, for the tool's clicks: none unless a test says so. */
let hit: number | null = null;

respond("font_families", () => ({ families: ["Arial", FAMILY], defaultFamily: FAMILY }));
respond("font_styles", () => [
  { weight: 400, italic: false },
  { weight: 700, italic: false },
  { weight: 400, italic: true },
]);
respond("text_layout", (args, doc) => {
  const found = doc.layers.find((l) => l.id === args.id);
  const length = found?.text?.content.length ?? 0;
  return {
    carets: Array.from({ length: length + 1 }, (_, i) => [30 + 7 * i, 30, 30 + 7 * i, 44]),
    missingFont: found?.text?.family === "Gone Font",
  };
});
respond("layer_at", () => hit);
// Live edits: a text layer added, or its text changed.
respond("perform_live", (args, doc) => {
  const edit = args.edit as EditRequest;
  if (edit.kind === "addText") {
    // Ids from 9 on.
    const id = Math.max(8, ...doc.layers.map((l) => l.id)) + 1;
    doc.layers = [...doc.layers, { ...textLayer(id, edit.text.content), text: edit.text }];
  } else if (edit.kind === "setText") {
    doc.layers = doc.layers.map((l) => (l.id === edit.id ? { ...l, text: edit.text } : l));
  } else if (edit.kind === "removeLayer") {
    doc.layers = doc.layers.filter((l) => l.id !== edit.id);
  }
  return { ...doc, revision: doc.revision + 1 };
});
respond("end_gesture", (_, doc) => ({ ...doc }));
respond("cancel_gesture", (_, doc) => {
  doc.layers = doc.layers.filter((l) => l.id < 9);
  return { ...doc };
});

function click(user: ReturnType<typeof open>, at: [number, number]) {
  const svg = document.querySelector("svg.type-tool") as SVGSVGElement;
  return user.pointer({
    keys: "[MouseLeft]",
    target: svg,
    coords: { clientX: at[0], clientY: at[1] },
  });
}

const liveEdits = () => sent("perform_live").map((a) => a.edit as EditRequest);

test("T picks the Type tool; a click places a text typed live, Escape commits it", async () => {
  hit = null;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("t");
  expect(screen.getByRole("button", { name: "Horizontal Type Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await click(user, [30, 40]);
  await waitFor(() => expect(liveEdits()).toHaveLength(1));
  // 12 pt at 72 ppi: 12 px, in the foreground color, above the active layer.
  expect(liveEdits()[0]).toEqual({
    kind: "addText",
    text: text(""),
    parent: null,
    index: 1,
  });
  const field = await screen.findByRole("textbox", { name: "Horizontal Type Tool" });
  await waitFor(() => expect(field).toHaveFocus());
  await user.keyboard("Hi{Enter}y");
  await waitFor(() =>
    expect(liveEdits().at(-1)).toEqual({ kind: "setText", id: 9, text: text("Hi\ny") }),
  );
  // The caret where the engine says, after the typed text.
  expect(document.querySelector("line.caret")).not.toBeNull();
  // The letters went to the text, not to the tools.
  expect(screen.getByRole("button", { name: "Horizontal Type Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await user.keyboard("{Escape}");
  await waitFor(() => expect(sent("end_gesture")).toHaveLength(1));
  expect(sent("cancel_gesture")).toHaveLength(0);
  expect(screen.queryByRole("textbox", { name: "Horizontal Type Tool" })).toBeNull();
});

test("a text committed empty leaves nothing: its gesture is taken back", async () => {
  hit = null;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("t");
  await click(user, [30, 40]);
  await screen.findByRole("textbox", { name: "Horizontal Type Tool" });
  // A click on the image commits, as in Photoshop.
  await click(user, [100, 100]);
  await waitFor(() => expect(sent("cancel_gesture")).toHaveLength(1));
  expect(sent("end_gesture")).toHaveLength(0);
  expect(liveEdits()).toHaveLength(1);
});

test("a click on a text layer edits it, the caret at its end; Ctrl+Enter commits", async () => {
  hit = 2;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), textLayer(2, "Title")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("t");
  await click(user, [35, 38]);
  const field = await screen.findByRole("textbox", { name: "Horizontal Type Tool" });
  await waitFor(() => expect(field).toHaveFocus());
  await user.keyboard("!");
  await waitFor(() =>
    expect(liveEdits().at(-1)).toEqual({ kind: "setText", id: 2, text: text("Title!") }),
  );
  expect(liveEdits().some((e) => e.kind === "addText")).toBe(false);
  await user.keyboard("{Control>}{Enter}{/Control}");
  await waitFor(() => expect(sent("end_gesture")).toHaveLength(1));
});

test("the keypad's Enter commits; choosing another tool commits too", async () => {
  hit = null;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("t");
  await click(user, [30, 40]);
  const field = await screen.findByRole("textbox", { name: "Horizontal Type Tool" });
  await user.keyboard("A");
  await fireEvent.keyDown(field, { key: "Enter", code: "NumpadEnter" });
  await waitFor(() => expect(sent("end_gesture")).toHaveLength(1));
  await click(user, [30, 40]);
  await screen.findByRole("textbox", { name: "Horizontal Type Tool" });
  await user.keyboard("B");
  await waitFor(() => expect(liveEdits().at(-1)).toMatchObject({ kind: "setText" }));
  await user.click(screen.getByRole("button", { name: "Move Tool" }));
  await waitFor(() => expect(sent("end_gesture")).toHaveLength(2));
});

test("the options bar sets the active text layer's font, style, size and alignment", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), textLayer(2, "Title")]));
  await waitFor(() => expect(row("Title")).toBeInTheDocument());
  await user.click(row("Title"));
  await user.keyboard("t");
  const family = await screen.findByRole("combobox", { name: "Font family" });
  await waitFor(() => expect(family).toHaveValue(FAMILY));
  await user.selectOptions(family, "Arial");
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("perform")[0]).toMatchObject({
    documentId: 1,
    edit: { kind: "setText", id: 2, text: { ...text("Title"), family: "Arial" } },
  });
  await user.selectOptions(screen.getByRole("combobox", { name: "Font style" }), "Bold");
  await waitFor(() => expect(sent("perform")).toHaveLength(2));
  expect(sent("perform")[1]).toMatchObject({ edit: { text: { weight: 700, italic: false } } });
  await user.click(screen.getByRole("button", { name: "Center text" }));
  await waitFor(() => expect(sent("perform")).toHaveLength(3));
  expect(sent("perform")[2]).toMatchObject({ edit: { text: { align: "center" } } });
});

test("a text layer whose font is not installed says so in the options bar", async () => {
  const user = open(
    documentView(1, "cat.jpg", [layer(1, "Cat"), textLayer(2, "Old", "Gone Font")]),
  );
  await waitFor(() => expect(row("Old")).toBeInTheDocument());
  await user.click(row("Old"));
  await user.keyboard("t");
  expect(
    await screen.findByText("Missing font: Gone Font. Edited, the text is set in Inter Variable."),
  ).toBeInTheDocument();
});
