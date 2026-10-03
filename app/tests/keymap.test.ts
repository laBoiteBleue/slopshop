import { test } from "vitest";
import assert from "node:assert/strict";
import type { CommandId } from "../src/lib/commands";
import { isTextField, keyAction, type KeyContext, type KeyEvent } from "../src/lib/keymap";

/** A key press: `key` as typed, `code` the physical key (by default the QWERTY one). */
function press(key: string, changes: Partial<KeyEvent> = {}): KeyEvent {
  const code = /^[a-z]$/i.test(key)
    ? `Key${key.toUpperCase()}`
    : key === "["
      ? "BracketLeft"
      : key === "]"
        ? "BracketRight"
        : key;
  return {
    key,
    code,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    repeat: false,
    inTextField: false,
    inSelect: false,
    ...changes,
  };
}

function context(changes: Partial<KeyContext> = {}): KeyContext {
  return {
    mac: false,
    tool: "move",
    aiTask: false,
    layerTransfer: false,
    tabDrag: false,
    dialogOpen: () => false,
    command: () => ({}),
    ...changes,
  };
}

const kind = (e: KeyEvent, ctx = context()) => keyAction(e, ctx).kind;

test("Escape cancels the AI task first, then a drag of layers, then a drag of a tab", () => {
  const all = context({ aiTask: true, layerTransfer: true, tabDrag: true });
  assert.equal(kind(press("Escape"), all), "cancelAiTask");
  assert.equal(
    kind(press("Escape"), context({ layerTransfer: true, tabDrag: true })),
    "endLayerTransfer",
  );
  assert.equal(kind(press("Escape"), context({ tabDrag: true })), "cancelTabDrag");
});

test("Ctrl+Tab goes to the next tab, with Shift the previous one, even while typing", () => {
  assert.deepEqual(keyAction(press("Tab", { ctrlKey: true }), context()), {
    kind: "cycleTabs",
    step: 1,
  });
  assert.deepEqual(
    keyAction(press("Tab", { ctrlKey: true, shiftKey: true, inTextField: true }), context()),
    { kind: "cycleTabs", step: -1 },
  );
});

test("a tool's letter picks its slot, Shift the next variant; held, nothing more", () => {
  const action = keyAction(press("M", { shiftKey: true }), context());
  assert.equal(action.kind, "toolSlot");
  assert.ok(action.kind === "toolSlot" && action.next && action.slot.key === "M");
  assert.equal(kind(press("m", { repeat: true })), "swallow");
  // Not while typing, nor with a modifier.
  assert.equal(kind(press("m", { inTextField: true })), "none");
  assert.equal(kind(press("v", { altKey: true })), "none");
});

test("on AZERTY, the letter typed picks the tool", () => {
  // AZERTY's M is QWERTY's ; key.
  const action = keyAction(press("m", { code: "Semicolon" }), context());
  assert.ok(action.kind === "toolSlot" && action.slot.key === "M");
});

test("[ and ] size the brush and the erasers, Shift their hardness", () => {
  assert.deepEqual(keyAction(press("]"), context({ tool: "brush" })), {
    kind: "paintSize",
    larger: true,
    eraser: false,
  });
  assert.deepEqual(keyAction(press("[", { shiftKey: true }), context({ tool: "restoreEraser" })), {
    kind: "paintHardness",
    larger: false,
    eraser: true,
  });
  // By the physical keys: AZERTY's ^ is QWERTY's [.
  assert.equal(kind(press("^", { code: "BracketLeft" }), context({ tool: "eraser" })), "paintSize");
  assert.deepEqual(keyAction(press("]"), context({ tool: "quickSelection" })), {
    kind: "quickSelectionSize",
    larger: true,
  });
  assert.equal(kind(press("]", { inTextField: true }), context({ tool: "brush" })), "none");
});

test("D sets the default colors and X swaps them, without Shift", () => {
  assert.equal(kind(press("d")), "defaultColors");
  assert.equal(kind(press("x")), "swapColors");
  assert.equal(kind(press("x", { inTextField: true })), "none");
});

test("a shortcut runs its command unless a field, a list or a dialog keeps the key", () => {
  const undo = press("z", { ctrlKey: true });
  assert.deepEqual(keyAction(undo, context()), { kind: "command", id: "undo" });
  // A text field keeps its own undo, unless the command works while typing.
  assert.equal(kind({ ...undo, inTextField: true }), "none");
  const whileTyping = context({ command: () => ({ whileTyping: true }) });
  assert.equal(kind({ ...undo, inTextField: true }, whileTyping), "command");
  // A modal dialog keeps every shortcut; a non-modal one only keys without modifiers.
  const modal = context({ dialogOpen: (modalOnly) => modalOnly });
  assert.equal(kind(undo, modal), "none");
  const nonModal = context({ dialogOpen: (modalOnly) => !modalOnly });
  assert.equal(kind(undo, nonModal), "command");
  // A list keeps keys without modifiers (Delete), not shortcuts with one.
  assert.equal(kind(press("Delete", { inSelect: true })), "none");
  assert.equal(kind({ ...undo, inSelect: true }), "command");
});

test("a held or disabled shortcut is swallowed", () => {
  const undo = press("z", { ctrlKey: true });
  assert.equal(kind({ ...undo, repeat: true }), "swallow");
  const repeats = context({ command: () => ({ repeats: true }) });
  assert.equal(kind({ ...undo, repeat: true }, repeats), "command");
  const disabled = context({ command: () => ({ disabled: true }) });
  assert.equal(kind(undo, disabled), "swallow");
  assert.equal(kind(press("F13")), "none");
});

test("on macOS, Cmd is the shortcut modifier", () => {
  const mac = context({ mac: true });
  const action = keyAction(press("z", { metaKey: true }), mac);
  assert.deepEqual(action, { kind: "command", id: "undo" satisfies CommandId });
  // Ctrl keeps its Cocoa meaning: Ctrl+Z is not Undo.
  assert.equal(kind(press("z", { ctrlKey: true }), mac), "none");
});

test("text fields are text areas and text or number inputs", () => {
  assert.ok(isTextField({ tagName: "TEXTAREA" } as unknown as EventTarget));
  assert.ok(isTextField({ tagName: "INPUT", type: "number" } as unknown as EventTarget));
  assert.ok(!isTextField({ tagName: "INPUT", type: "checkbox" } as unknown as EventTarget));
  assert.ok(!isTextField({ tagName: "SELECT" } as unknown as EventTarget));
  assert.ok(!isTextField(null));
});
