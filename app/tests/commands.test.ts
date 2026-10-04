import { test } from "vitest";
import assert from "node:assert/strict";
import {
  SHORTCUTS,
  commandAt,
  formatShortcut,
  matches,
  parseShortcut,
  type KeyPress,
} from "../src/lib/commands";

/** A key press: `key` as typed, `code` the physical key (by default the QWERTY one). */
function press(key: string, modifiers: Partial<KeyPress> = {}): KeyPress {
  const brackets: Record<string, string> = { "[": "BracketLeft", "]": "BracketRight" };
  const code = /^[a-z]$/i.test(key) ? `Key${key.toUpperCase()}` : (brackets[key] ?? "");
  return {
    key,
    code,
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    ...modifiers,
  };
}

const ctrl = { ctrlKey: true };
const cmd = { metaKey: true };

test("Ctrl+Z undoes, Ctrl+Shift+Z and Ctrl+Y redo", () => {
  assert.equal(commandAt(press("z", ctrl), false), "undo");
  assert.equal(commandAt(press("Z", { ...ctrl, shiftKey: true }), false), "redo");
  assert.equal(commandAt(press("y", ctrl), false), "redo");
});

test("Copy Merged, Paste in Place and Paste Into are Photoshop's", () => {
  assert.equal(commandAt(press("C", { ...ctrl, shiftKey: true }), false), "copyMerged");
  assert.equal(commandAt(press("c", { ...ctrl, altKey: true }), false), "canvasSize");
  assert.equal(commandAt(press("V", { ...ctrl, shiftKey: true }), false), "pasteInPlace");
  assert.equal(
    commandAt(press("V", { ...ctrl, shiftKey: true, altKey: true }), false),
    "pasteInto",
  );
});

test("Transform Again and Duplicate and Transform Again are Photoshop's", () => {
  assert.equal(commandAt(press("t", ctrl), false), "freeTransform");
  assert.equal(commandAt(press("T", { ...ctrl, shiftKey: true }), false), "repeatTransform");
  assert.equal(
    commandAt(press("T", { ...ctrl, shiftKey: true, altKey: true }), false),
    "duplicateRepeat",
  );
});

test("macOS: Cmd is the modifier, Ctrl is not", () => {
  assert.equal(commandAt(press("z", cmd), true), "undo");
  assert.equal(commandAt(press("z", { ...cmd, shiftKey: true }), true), "redo");
  assert.equal(commandAt(press("z", ctrl), true), null);
  assert.equal(commandAt(press("z", { ...cmd, ctrlKey: true }), true), null);
  assert.equal(commandAt(press(",", cmd), true), "preferences");
});

test("letters are read as typed (AZERTY), else by the physical key", () => {
  // AZERTY: the key typing "z" is QWERTY's W.
  assert.equal(commandAt({ ...press("z", ctrl), code: "KeyW" }, false), "undo");
  // A Cyrillic layout types "я" on the Z key.
  assert.equal(commandAt({ ...press("я", ctrl), code: "KeyZ" }, false), "undo");
});

test("digits and zoom keys ignore Shift, and match the physical key", () => {
  // AZERTY: Ctrl+& (the 1 key without Shift).
  assert.equal(commandAt({ ...press("&", ctrl), code: "Digit1" }, false), "actualSize");
  assert.equal(commandAt({ ...press("0", ctrl), code: "Numpad0" }, false), "fitOnScreen");
  // QWERTY: "+" is Shift+=.
  assert.equal(commandAt(press("+", { ...ctrl, shiftKey: true }), false), "zoomIn");
  assert.equal(commandAt(press("=", ctrl), false), "zoomIn");
  assert.equal(commandAt(press("-", ctrl), false), "zoomOut");
});

test("modifiers must match exactly", () => {
  assert.equal(commandAt(press("i", { ...ctrl, altKey: true }), false), "imageSize");
  assert.equal(commandAt(press("i", { ...ctrl, shiftKey: true }), false), "inverse");
  assert.equal(
    commandAt(press("i", { ...ctrl, altKey: true, shiftKey: true }), false),
    "documentInfo",
  );
  assert.equal(commandAt(press("q"), false), "quickMask");
  assert.equal(commandAt(press("q", { shiftKey: true }), false), null);
  assert.equal(commandAt(press("F6", { shiftKey: true }), false), "feather");
  assert.equal(commandAt(press("Delete"), false), "deleteLayers");
});

test("no two commands share a shortcut", () => {
  const seen = new Map<string, string>();
  for (const [id, shortcuts] of Object.entries(SHORTCUTS)) {
    for (const shortcut of shortcuts) {
      const s = parseShortcut(shortcut);
      const canonical = `${s.mod}-${s.shift}-${s.alt}-${s.key}`;
      assert.equal(seen.get(canonical), undefined, `${shortcut} of ${id}`);
      seen.set(canonical, id);
    }
  }
});

test("every shortcut matches a press of its own keys", () => {
  for (const [id, shortcuts] of Object.entries(SHORTCUTS)) {
    for (const shortcut of shortcuts) {
      const s = parseShortcut(shortcut);
      const key = s.key.length === 1 ? s.key : s.key[0].toUpperCase() + s.key.slice(1);
      const p = press(key, { ctrlKey: s.mod, shiftKey: s.shift, altKey: s.alt });
      assert.ok(matches(shortcut, p, false), shortcut);
      assert.equal(commandAt(p, false), id, shortcut);
    }
  }
});

test("shortcuts are shown in each platform's way", () => {
  const names = { shift: "Maj", alt: "Alt", delete: "Suppr", backspace: "Retour arrière" };
  assert.equal(formatShortcut("shift+mod+z", names, false), "Maj+Ctrl+Z");
  assert.equal(formatShortcut("alt+shift+mod+k", names, false), "Alt+Maj+Ctrl+K");
  assert.equal(formatShortcut("delete", names, false), "Suppr");
  assert.equal(formatShortcut("shift+f6", names, false), "Maj+F6");
  assert.equal(formatShortcut("mod++", names, false), "Ctrl++");
  assert.equal(formatShortcut("alt+shift+mod+k", names, true), "⌥⇧⌘K");
  assert.equal(formatShortcut("mod+,", names, true), "⌘,");
});

test("Ctrl+E merges, Shift+Ctrl+E merges the visible layers, Export is Alt+Shift+Ctrl+W", () => {
  assert.equal(commandAt(press("e", ctrl), false), "mergeLayers");
  assert.equal(commandAt(press("E", { ...ctrl, shiftKey: true }), false), "mergeVisible");
  assert.equal(commandAt(press("W", { ...ctrl, shiftKey: true, altKey: true }), false), "export");
});

test("New Layer from Visible is Photoshop's stamp visible, Alt+Shift+Ctrl+E", () => {
  assert.equal(
    commandAt(press("E", { ...ctrl, shiftKey: true, altKey: true }), false),
    "newLayerFromVisible",
  );
});

test("Layer > Arrange is Ctrl+[ and ], by the physical keys (^ and $ on AZERTY)", () => {
  const bracket = (key: string, code: string, modifiers: Partial<KeyPress>) => ({
    ...press(key, modifiers),
    code,
  });
  assert.equal(commandAt(bracket("]", "BracketRight", ctrl), false), "bringForward");
  assert.equal(commandAt(bracket("$", "BracketRight", ctrl), false), "bringForward");
  assert.equal(commandAt(bracket("[", "BracketLeft", ctrl), false), "sendBackward");
  assert.equal(
    commandAt(bracket("}", "BracketRight", { ...ctrl, shiftKey: true }), false),
    "bringToFront",
  );
  assert.equal(
    commandAt(bracket("£", "BracketRight", { ...ctrl, shiftKey: true }), false),
    "bringToFront",
  );
  assert.equal(
    commandAt(bracket("{", "BracketLeft", { ...cmd, shiftKey: true }), true),
    "sendToBack",
  );
  // Alone, the brackets are the brushes' (keymap.ts), not a command.
  assert.equal(commandAt(bracket("]", "BracketRight", {}), false), null);
});

test("View: Ctrl+H hides the extras and F11 is full screen; Window: Tab hides the panels", () => {
  assert.equal(commandAt(press("r", ctrl), false), "rulers");
  assert.equal(commandAt(press("h", ctrl), false), "hideExtras");
  assert.equal(commandAt(press("h", cmd), true), "hideExtras");
  assert.equal(commandAt(press("F11"), false), "fullScreen");
  assert.equal(commandAt(press("Tab"), false), "hidePanels");
  // Shift+Tab moves the focus back, as everywhere.
  assert.equal(commandAt(press("Tab", { shiftKey: true }), false), null);
  const names = { shift: "Maj", alt: "Alt", delete: "Suppr", backspace: "Retour arrière" };
  assert.equal(formatShortcut("tab", names, false), "Tab");
  assert.equal(formatShortcut("tab", names, true), "⇥");
  assert.equal(formatShortcut("f11", names, false), "F11");
});
