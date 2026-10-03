import { test } from "node:test";
import assert from "node:assert/strict";
import {
  SHORTCUTS,
  commandAt,
  formatShortcut,
  matches,
  parseShortcut,
  type KeyPress,
} from "../src/lib/commands.ts";

/** A key press: `key` as typed, `code` the physical key (by default the QWERTY one). */
function press(key: string, modifiers: Partial<KeyPress> = {}): KeyPress {
  const code = /^[a-z]$/i.test(key) ? `Key${key.toUpperCase()}` : "";
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
