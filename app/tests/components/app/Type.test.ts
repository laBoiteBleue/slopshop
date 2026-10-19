import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { expect, test } from "vitest";
import type {
  DocumentView,
  EditRequest,
  LayerView,
  TextDto,
  TextLineView,
} from "../../../src/lib/engine";
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

/**
 * The engine's layout, as the mock draws texts: characters 7 px wide from x = 30, lines 16 px
 * apart from y = 30 (a caret 14 px tall).
 */
function layoutOf(content: string) {
  const carets: [number, number, number, number][] = [];
  const lines: TextLineView[] = [];
  let start = 0;
  content.split("\n").forEach((line, l, all) => {
    const top = 30 + 16 * l;
    const hardBreak = l < all.length - 1;
    // The last one stands before the line break, or at the text's end.
    for (let i = 0; i <= line.length; i++) carets.push([30 + 7 * i, top, 30 + 7 * i, top + 14]);
    const end = start + line.length + (hardBreak ? 1 : 0);
    const right = 30 + 7 * line.length;
    lines.push({
      start,
      end,
      hardBreak,
      corners: [
        [30, top],
        [right, top],
        [right, top + 14],
        [30, top + 14],
      ],
    });
    start = end;
  });
  return { carets, lines };
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
  const content = found?.text?.content ?? "";
  // Paragraph text: its box placed straight in the document; lines that pass it overflow.
  const frame = found?.text?.frame ?? null;
  const [x, y] = found?.text?.position ?? [0, 0];
  return {
    ...layoutOf(content),
    frame: frame
      ? [
          [x, y],
          [x + frame[0], y],
          [x + frame[0], y + frame[1]],
          [x, y + frame[1]],
        ]
      : null,
    overflow: frame !== null && content.split("\n").length * 16 > frame[1],
    missingFont: found?.text?.family === "Gone Font",
  };
});
respond("layer_at", () => hit);
/**
 * A text edit as the mock engine applies it: setText keeps the layer's spans; styleText sets
 * the whole text's own settings, or adds a span (the tests style apart ranges).
 */
function applyText(doc: DocumentView, edit: EditRequest) {
  if (edit.kind === "setText") {
    doc.layers = doc.layers.map((l) =>
      l.id === edit.id ? { ...l, text: { ...edit.text, spans: l.text?.spans ?? [] } } : l,
    );
  } else if (edit.kind === "styleText") {
    doc.layers = doc.layers.map((l) => {
      if (l.id !== edit.id || !l.text) return l;
      if (edit.range === null) return { ...l, text: { ...l.text, ...edit.style } as TextDto };
      const [start, end] = edit.range;
      const spans = [...(l.text.spans ?? []), { start, end, style: edit.style }];
      return { ...l, text: { ...l.text, spans } };
    });
  }
}

respond("perform", (args, doc) => {
  applyText(doc, args.edit as EditRequest);
  return { ...doc, revision: doc.revision + 1 };
});

// Live edits: a text layer added, or its text changed.
respond("perform_live", (args, doc) => {
  const edit = args.edit as EditRequest;
  if (edit.kind === "addText") {
    // Ids from 9 on.
    const id = Math.max(8, ...doc.layers.map((l) => l.id)) + 1;
    doc.layers = [...doc.layers, { ...textLayer(id, edit.text.content), text: edit.text }];
  } else if (edit.kind === "setText" || edit.kind === "styleText") {
    applyText(doc, edit);
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

test("a click on a text layer edits it where clicked; a click in it moves the caret", async () => {
  hit = 2;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), textLayer(2, "Title")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("t");
  // Carets every 7 px from x = 30: after "Ti".
  await click(user, [44, 38]);
  const field = await screen.findByRole("textbox", { name: "Horizontal Type Tool" });
  await waitFor(() => expect(field).toHaveFocus());
  await user.keyboard("!");
  await waitFor(() =>
    expect(liveEdits().at(-1)).toEqual({ kind: "setText", id: 2, text: text("Ti!tle") }),
  );
  // At the end now, by a click right of the text: still typing, nothing committed.
  await click(user, [80, 38]);
  expect(sent("end_gesture")).toHaveLength(0);
  await user.keyboard("?");
  await waitFor(() =>
    expect(liveEdits().at(-1)).toEqual({ kind: "setText", id: 2, text: text("Ti!tle?") }),
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
  const bar = within(screen.getByRole("toolbar", { name: "Tool options" }));
  const family = await bar.findByRole("combobox", { name: "Font family" });
  await waitFor(() => expect(family).toHaveValue(FAMILY));
  await user.selectOptions(family, "Arial");
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  // The whole text: a style without a range.
  expect(sent("perform")[0]).toEqual({
    documentId: 1,
    edit: { kind: "styleText", id: 2, range: null, style: { family: "Arial" } },
  });
  await user.selectOptions(bar.getByRole("combobox", { name: "Font style" }), "Bold");
  await waitFor(() => expect(sent("perform")).toHaveLength(2));
  expect(sent("perform")[1]).toMatchObject({ edit: { style: { weight: 700 } } });
  expect((sent("perform")[1].edit as { style: object }).style).toEqual({ weight: 700 });
  await user.click(bar.getByRole("button", { name: "Center text" }));
  await waitFor(() => expect(sent("perform")).toHaveLength(3));
  expect(sent("perform")[2]).toMatchObject({
    edit: { kind: "setText", text: { align: "center", family: "Arial", weight: 700 } },
  });
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

// Editing comfort: selections drawn from the engine's layout, keys by drawn lines, clipboard.

/** What the clipboard holds for `paste_text`. */
let clipboardText: string | null = null;
respond("paste_text", () => clipboardText);

/** Start editing text layer 2, which holds `content`, its caret at its end. */
async function editText(content: string) {
  hit = 2;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), textLayer(2, content)]));
  await screen.findByText("cat.jpg");
  await user.keyboard("t");
  // Right of the last line's end: the caret at the end.
  const lines = content.split("\n");
  await click(user, [30 + 7 * lines.at(-1)!.length + 3, 37 + 16 * (lines.length - 1)]);
  const field = await screen.findByRole("textbox", { name: "Horizontal Type Tool" });
  await waitFor(() => expect(field).toHaveFocus());
  return { user, field };
}

/** The last text sent live. */
const lastContent = () => {
  const edit = liveEdits().at(-1);
  return edit?.kind === "setText" ? edit.text.content : null;
};

/** The place of character `i` of line `l` in the mock's layout (its caret, mid-height). */
const place = (i: number, l = 0): [number, number] => [30 + 7 * i, 37 + 16 * l];

test("a drag selects characters, drawn as a highlight; typing replaces them", async () => {
  const { user } = await editText("Title");
  const svg = document.querySelector("svg.type-tool") as SVGSVGElement;
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: place(1)[0], clientY: 37 } },
    { target: svg, coords: { clientX: place(4)[0], clientY: 37 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: place(4)[0], clientY: 37 } },
  ]);
  // One highlight from "i" to "l", no caret while characters are selected.
  const highlight = document.querySelectorAll("polygon.selected");
  expect(highlight).toHaveLength(1);
  expect(highlight[0].getAttribute("points")).toBe("37,30 58,30 58,44 37,44");
  expect(document.querySelector("line.caret")).toBeNull();
  await user.keyboard("X");
  await waitFor(() => expect(lastContent()).toBe("TXe"));
  expect(document.querySelectorAll("polygon.selected")).toHaveLength(0);
});

test("a double click selects a word, a triple click a line; Shift+click extends", async () => {
  const { user } = await editText("Hello world\nagain");
  const svg = document.querySelector("svg.type-tool") as SVGSVGElement;
  const at = (p: [number, number]) => ({ target: svg, coords: { clientX: p[0], clientY: p[1] } });
  await user.pointer({ keys: "[MouseLeft][MouseLeft]", ...at(place(8)) });
  await user.keyboard("X");
  await waitFor(() => expect(lastContent()).toBe("Hello X\nagain"));
  // Away in time: a new count.
  await new Promise((resolve) => setTimeout(resolve, 600));
  await user.pointer({ keys: "[MouseLeft][MouseLeft][MouseLeft]", ...at(place(2)) });
  await user.keyboard("Y");
  await waitFor(() => expect(lastContent()).toBe("Y\nagain"));
  await new Promise((resolve) => setTimeout(resolve, 600));
  // From the second line's start, Shift+click after "ag".
  await user.pointer({ keys: "[MouseLeft]", ...at(place(0, 1)) });
  await user.keyboard("{Shift>}");
  await user.pointer({ keys: "[MouseLeft]", ...at(place(2, 1)) });
  await user.keyboard("{/Shift}Z");
  await waitFor(() => expect(lastContent()).toBe("Y\nZain"));
});

test("Up, Down, Home and End follow the drawn lines; with Shift they select", async () => {
  const { user } = await editText("abcdef\nab");
  // From the end of "ab" up: under it on the first line.
  await user.keyboard("{ArrowUp}1");
  await waitFor(() => expect(lastContent()).toBe("ab1cdef\nab"));
  await user.keyboard("{End}2");
  await waitFor(() => expect(lastContent()).toBe("ab1cdef2\nab"));
  await user.keyboard("{ArrowDown}{Home}3");
  await waitFor(() => expect(lastContent()).toBe("ab1cdef2\n3ab"));
  // Shift+Up from after "3": up to the first line's place above it.
  await user.keyboard("{Shift>}{ArrowUp}{/Shift}");
  await waitFor(() => expect(document.querySelectorAll("polygon.selected")).toHaveLength(2));
  await user.keyboard("4");
  await waitFor(() => expect(lastContent()).toBe("a4ab"));
});

test("Ctrl+A selects the whole text; Ctrl+C, Ctrl+X and Ctrl+V copy, cut and paste", async () => {
  const { user } = await editText("Hello");
  await user.keyboard("{Control>}a{/Control}");
  expect(document.querySelectorAll("polygon.selected")).toHaveLength(1);
  // Nothing else selected: the image's layers are not.
  expect(sent("select_all")).toHaveLength(0);
  await user.keyboard("{Control>}c{/Control}");
  await waitFor(() => expect(sent("copy_text")).toEqual([{ text: "Hello" }]));
  expect(sent("copy")).toHaveLength(0);
  await user.keyboard("{Control>}x{/Control}");
  await waitFor(() => expect(sent("copy_text")).toHaveLength(2));
  await waitFor(() => expect(lastContent()).toBe(""));
  // Another system's line breaks become the text's.
  clipboardText = "two\r\nlines";
  await user.keyboard("{Control>}v{/Control}");
  await waitFor(() => expect(lastContent()).toBe("two\nlines"));
  expect(sent("paste")).toHaveLength(0);
});

test("Edit > Copy, Cut, Paste and Select All act on the text being typed", async () => {
  const { user } = await editText("Hi there");
  const edit = async (entry: string) => {
    await user.click(screen.getByRole("menuitem", { name: "Edit" }));
    await user.click(screen.getByRole("menuitem", { name: new RegExp(`^${entry}`) }));
  };
  // Nothing selected: nothing to copy.
  await user.click(screen.getByRole("menuitem", { name: "Edit" }));
  expect(screen.getByRole("menuitem", { name: /^Copy Ctrl/ })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  await user.click(screen.getByRole("menuitem", { name: "Edit" }));
  await user.click(screen.getByRole("menuitem", { name: "Select" }));
  await user.click(screen.getByText("All", { selector: ".label" }));
  expect(sent("select_all")).toHaveLength(0);
  await edit("Copy Ctrl");
  await waitFor(() => expect(sent("copy_text")).toEqual([{ text: "Hi there" }]));
  clipboardText = "Yo";
  await edit("Paste Ctrl");
  await waitFor(() => expect(lastContent()).toBe("Yo"));
  // Still typing.
  expect(sent("end_gesture")).toHaveLength(0);
  expect(screen.getByRole("textbox", { name: "Horizontal Type Tool" })).toBeInTheDocument();
});

// Paragraph text: a box drawn by a drag, resized by its handles while it is edited.

test("a drag draws a box for paragraph text; its handles resize it while it is typed", async () => {
  hit = null;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("t");
  const svg = document.querySelector("svg.type-tool") as SVGSVGElement;
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 140, clientY: 100 } },
    { target: svg, coords: { clientX: 100, clientY: 80 } },
    { target: svg, coords: { clientX: 40, clientY: 50 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 40, clientY: 50 } },
  ]);
  await waitFor(() => expect(liveEdits()).toHaveLength(1));
  // From the drag's top left, 100 by 50; no layer was looked for under the pointer.
  expect(liveEdits()[0]).toEqual({
    kind: "addText",
    text: { ...text("", [40, 50]), frame: [100, 50] },
    parent: null,
    index: 1,
  });
  expect(sent("layer_at")).toHaveLength(0);
  await screen.findByRole("textbox", { name: "Horizontal Type Tool" });
  await waitFor(() => expect(document.querySelectorAll("rect.handle")).toHaveLength(8));
  expect(document.querySelector("polygon.frame")?.getAttribute("points")).toBe(
    "40,50 140,50 140,100 40,100",
  );
  // The right edge's handle, dragged 40 px right.
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 140, clientY: 75 } },
    { target: svg, coords: { clientX: 180, clientY: 75 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 180, clientY: 75 } },
  ]);
  await waitFor(() =>
    expect(liveEdits().at(-1)).toEqual({
      kind: "setText",
      id: 9,
      text: { ...text("", [40, 50]), frame: [140, 50] },
    }),
  );
  await user.keyboard("Hi");
  await waitFor(() => expect(lastContent()).toBe("Hi"));
  expect(liveEdits().at(-1)).toMatchObject({ text: { frame: [140, 50], position: [40, 50] } });
  // A click in the box, away from the letters, keeps typing; outside, it commits.
  await click(user, [170, 95]);
  expect(sent("end_gesture")).toHaveLength(0);
  await click(user, [300, 300]);
  await waitFor(() => expect(sent("end_gesture")).toHaveLength(1));
});

test("a short drag is a click: point text", async () => {
  hit = null;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("t");
  const svg = document.querySelector("svg.type-tool") as SVGSVGElement;
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 30, clientY: 40 } },
    { target: svg, coords: { clientX: 31, clientY: 41 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 31, clientY: 41 } },
  ]);
  await waitFor(() => expect(liveEdits()).toHaveLength(1));
  expect(liveEdits()[0]).toMatchObject({ kind: "addText", text: text("") });
  expect((liveEdits()[0] as { text: TextDto }).text.frame ?? null).toBeNull();
});

test("a box too small for its lines shows a plus in its bottom right handle", async () => {
  hit = 2;
  const boxed: LayerView = {
    ...textLayer(2, "a\nb\nc"),
    text: { ...text("a\nb\nc", [30, 30]), frame: [80, 20] },
  };
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), boxed]));
  await screen.findByText("cat.jpg");
  await user.keyboard("t");
  await click(user, place(1));
  await screen.findByRole("textbox", { name: "Horizontal Type Tool" });
  await waitFor(() => expect(document.querySelectorAll("rect.handle")).toHaveLength(8));
  expect(document.querySelector("path.overflow")).not.toBeNull();
});

// Character settings (spans): the characters selected while typed, else the whole text.

test("the Character settings style the characters selected while typed, else the whole text", async () => {
  const { user, field } = await editText("Hello");
  const character = within(await screen.findByRole("region", { name: "Character" }));
  // All selected: Bold on them, live in the typing's undo entry.
  await user.keyboard("{Control>}a{/Control}");
  await waitFor(() =>
    expect(character.getByRole("combobox", { name: "Font style" })).toHaveValue("400"),
  );
  await user.selectOptions(character.getByRole("combobox", { name: "Font style" }), "Bold");
  await waitFor(() =>
    expect(liveEdits().at(-1)).toEqual({
      kind: "styleText",
      id: 2,
      range: [0, 5],
      style: { weight: 700, italic: false },
    }),
  );
  // The options bar sets the selection too.
  const bar = within(screen.getByRole("toolbar", { name: "Tool options" }));
  await user.click(bar.getByRole("button", { name: "Center text" }));
  await waitFor(() =>
    expect(liveEdits().at(-1)).toMatchObject({ kind: "setText", text: { align: "center" } }),
  );
  // A caret, nothing selected: the style waits for the next characters typed there
  // (Photoshop's pending style), shown meanwhile.
  await click(user, place(5));
  const sent0 = liveEdits().length;
  const size = character.getByRole("spinbutton", { name: "Font size" });
  await user.clear(size);
  await user.type(size, "20");
  await user.tab();
  await waitFor(() => expect(size).toHaveValue(20));
  expect(liveEdits()).toHaveLength(sent0);
  // Back in the text, its caret where it was, a character typed is set so.
  field.focus();
  await user.keyboard("!");
  await waitFor(() =>
    expect(liveEdits().at(-1)).toMatchObject({
      kind: "setText",
      id: 2,
      text: { content: "Hello!" },
      typed: { range: [5, 6], style: { size: 20 } },
    }),
  );
  // The next one takes the style of the one before it (the engine's): nothing pending.
  await user.keyboard("?");
  await waitFor(() => expect(lastContent()).toBe("Hello!?"));
  expect(liveEdits().at(-1)).not.toHaveProperty("typed");
  expect(sent("perform")).toHaveLength(0);
  expect(sent("end_gesture")).toHaveLength(0);
});

test("a style chosen at a caret is forgotten when the caret moves", async () => {
  const { user } = await editText("Hello");
  const character = within(await screen.findByRole("region", { name: "Character" }));
  await user.click(character.getByRole("button", { name: "Underline" }));
  expect(character.getByRole("button", { name: "Underline" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await user.keyboard("{ArrowLeft}");
  await waitFor(() =>
    expect(character.getByRole("button", { name: "Underline" })).toHaveAttribute(
      "aria-pressed",
      "false",
    ),
  );
  await user.keyboard("!");
  await waitFor(() => expect(lastContent()).toBe("Hell!o"));
  expect(liveEdits().at(-1)).not.toHaveProperty("typed");
});

test("the style buttons set the characters selected; Standard Ligatures turns off", async () => {
  const { user } = await editText("Hello");
  const character = within(await screen.findByRole("region", { name: "Character" }));
  await user.keyboard("{Control>}a{/Control}");
  await user.click(character.getByRole("button", { name: "Small Caps" }));
  await waitFor(() =>
    expect(liveEdits().at(-1)).toEqual({
      kind: "styleText",
      id: 2,
      range: [0, 5],
      style: { case: "smallCaps" },
    }),
  );
  await waitFor(() =>
    expect(character.getByRole("button", { name: "Small Caps" })).toHaveAttribute(
      "aria-pressed",
      "true",
    ),
  );
  // All Caps replaces it.
  await user.click(character.getByRole("button", { name: "All Caps" }));
  await waitFor(() =>
    expect(liveEdits().at(-1)).toMatchObject({ style: { case: "allCaps" }, range: [0, 5] }),
  );
  await user.click(character.getByRole("button", { name: "Standard Ligatures" }));
  await waitFor(() =>
    expect(liveEdits().at(-1)).toMatchObject({ style: { ligatures: false }, range: [0, 5] }),
  );
  // Still typing: the buttons keep the focus in the text.
  expect(screen.getByRole("textbox", { name: "Horizontal Type Tool" })).toHaveFocus();
});

test("outside typing, the Character settings style the whole text layer, one undo entry each", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), textLayer(2, "Title")]));
  await waitFor(() => expect(row("Title")).toBeInTheDocument());
  await user.click(row("Title"));
  const character = within(await screen.findByRole("region", { name: "Character" }));
  await user.selectOptions(character.getByRole("combobox", { name: "Font family" }), "Arial");
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("perform")[0]).toEqual({
    documentId: 1,
    edit: { kind: "styleText", id: 2, range: null, style: { family: "Arial" } },
  });
  // The color, from the color picker.
  await user.click(character.getByRole("button", { name: "Text color" }));
  expect(await screen.findByRole("dialog")).toBeInTheDocument();
});

// The Type menu (Photoshop's): Convert to Shape, Paste Lorem Ipsum.

test("Type > Paste Lorem Ipsum types placeholder text at the caret, only while typing", async () => {
  const { user } = await editText("Hi ");
  await user.click(screen.getByRole("menuitem", { name: "Type" }));
  await user.click(screen.getByRole("menuitem", { name: "Paste Lorem Ipsum" }));
  await waitFor(() =>
    expect(lastContent()).toMatch(/^Hi Lorem ipsum dolor sit amet, .*laborum\.$/),
  );
  expect(sent("end_gesture")).toHaveLength(0);
  // Committed: nothing to paste into.
  await user.keyboard("{Escape}");
  await user.click(screen.getByRole("menuitem", { name: "Type" }));
  expect(screen.getByRole("menuitem", { name: "Paste Lorem Ipsum" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
});

test("Type > Convert to Shape converts the text layers selected, after committing the typing", async () => {
  hit = null;
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), textLayer(2, "Title")]));
  await waitFor(() => expect(row("Cat")).toBeInTheDocument());
  // A pixel layer: nothing to convert.
  await user.click(row("Cat"));
  await user.click(screen.getByRole("menuitem", { name: "Type" }));
  expect(screen.getByRole("menuitem", { name: "Convert to Shape" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  await user.keyboard("{Escape}");
  await user.click(row("Title"));
  await user.keyboard("[ControlLeft>]");
  await user.click(row("Cat"));
  await user.keyboard("[/ControlLeft]");
  await user.click(screen.getByRole("menuitem", { name: "Type" }));
  await user.click(screen.getByRole("menuitem", { name: "Convert to Shape" }));
  await waitFor(() =>
    expect(sent("perform").at(-1)).toEqual({
      documentId: 1,
      edit: { kind: "convertToShape", ids: [2] },
    }),
  );
});

test("Convert to Shape while a text is typed commits it first", async () => {
  const { user } = await editText("Hi");
  await user.click(screen.getByRole("menuitem", { name: "Type" }));
  await user.click(screen.getByRole("menuitem", { name: "Convert to Shape" }));
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("end_gesture")).toHaveLength(1);
  expect(sent("perform")[0].edit).toEqual({ kind: "convertToShape", ids: [2] });
});
