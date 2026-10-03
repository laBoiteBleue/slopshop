// The keyboard shortcuts of the app's commands, in one table read by the menus (the shortcut
// shown next to a command), the keyboard (the command a key press runs) and Edit > Keyboard
// Shortcuts (the list). Photoshop's shortcuts where it has the command (ADR 0013).

/**
 * A shortcut as written in the table: modifiers then one key, joined by "+", lower case:
 * "mod+shift+z", "shift+f6", "q". `mod` is Ctrl, or ⌘ on macOS. Keys are a letter, a digit,
 * "+", "-", ",", "[", "]", or a `KeyboardEvent.key` name ("f2", "delete", "backspace").
 */
export type Shortcut = string;

/** The shortcuts of each command; the first one is the one shown in menus. */
export const SHORTCUTS = {
  // File
  newDocument: ["mod+n"],
  open: ["mod+o"],
  importLayers: ["shift+mod+o"],
  close: ["mod+w"],
  closeAll: ["alt+mod+w"],
  save: ["mod+s"],
  saveAs: ["shift+mod+s"],
  export: ["shift+mod+e"],
  print: ["mod+p"],
  documentInfo: ["alt+shift+mod+i"],
  quit: ["mod+q"],
  // Edit: both redo shortcuts are Photoshop's (Ctrl+Y as in most Windows applications).
  undo: ["mod+z"],
  redo: ["shift+mod+z", "mod+y"],
  cut: ["mod+x"],
  copy: ["mod+c"],
  copyMerged: ["shift+mod+c"],
  paste: ["mod+v"],
  pasteInPlace: ["shift+mod+v"],
  pasteInto: ["alt+shift+mod+v"],
  fill: ["shift+f5", "shift+backspace"],
  freeTransform: ["mod+t"],
  repeatTransform: ["shift+mod+t"],
  duplicateRepeat: ["alt+shift+mod+t"],
  keyboardShortcuts: ["alt+shift+mod+k"],
  // ⌘, is every macOS application's settings; Ctrl+, does no harm elsewhere.
  preferences: ["mod+k", "mod+,"],
  // Image
  adjustLevels: ["mod+l"],
  adjustCurves: ["mod+m"],
  adjustHueSaturation: ["mod+u"],
  adjustColorBalance: ["mod+b"],
  adjustBlackWhite: ["alt+shift+mod+b"],
  adjustInvert: ["mod+i"],
  imageSize: ["alt+mod+i"],
  canvasSize: ["alt+mod+c"],
  // Layer
  newLayer: ["shift+mod+n"],
  // As in Photoshop, Ctrl+J is Layer via Copy (which duplicates without a selection) and
  // Duplicate Layer has no shortcut.
  layerViaCopy: ["mod+j"],
  layerViaCut: ["shift+mod+j"],
  duplicateLayers: [],
  groupLayers: ["mod+g"],
  ungroupLayers: ["shift+mod+g"],
  clipping: ["alt+mod+g"],
  bringToFront: ["shift+mod+]"],
  bringForward: ["mod+]"],
  sendBackward: ["mod+["],
  sendToBack: ["shift+mod+["],
  renameLayer: ["f2"],
  deleteLayers: ["delete", "backspace"],
  // Select
  selectAll: ["mod+a"],
  deselect: ["mod+d"],
  reselect: ["shift+mod+d"],
  inverse: ["shift+mod+i"],
  feather: ["shift+f6"],
  quickMask: ["q"],
  selectAllLayers: ["alt+mod+a"],
  // View
  zoomIn: ["mod++"],
  zoomOut: ["mod+-"],
  fitOnScreen: ["mod+0"],
  actualSize: ["mod+1"],
} as const satisfies Record<string, readonly Shortcut[]>;

export type CommandId = keyof typeof SHORTCUTS;

/** What shortcuts read of a key press (a `KeyboardEvent`). */
export type KeyPress = {
  key: string;
  code: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
};

type Parsed = { mod: boolean; shift: boolean; alt: boolean; key: string };

export function parseShortcut(shortcut: Shortcut): Parsed {
  // "mod++": the key is "+" itself.
  const parts = shortcut.endsWith("++")
    ? [...shortcut.slice(0, -2).split("+"), "+"]
    : shortcut.split("+");
  const key = parts.pop() ?? "";
  return {
    mod: parts.includes("mod"),
    shift: parts.includes("shift"),
    alt: parts.includes("alt"),
    key,
  };
}

/**
 * The letter of a letter shortcut, lower case, as Photoshop reads it: the letter the key types
 * on a Latin layout (AZERTY's A key is "a", wherever it sits), else the physical key, for
 * layouts without Latin letters (Cyrillic, Greek…) or when AltGr (Ctrl+Alt on Windows) types
 * another character. `null` for keys that are not letters.
 */
export function shortcutLetter(e: Pick<KeyPress, "key" | "code">): string | null {
  if (/^[a-z]$/i.test(e.key)) return e.key.toLowerCase();
  const physical = /^Key([A-Z])$/.exec(e.code);
  return physical ? physical[1].toLowerCase() : null;
}

/** Keys typed with Shift on some layouts ("+" on QWERTY, digits on AZERTY): Shift is ignored. */
const SHIFT_FREE = /^[0-9+\-,]$/;

/** Whether `press` is `shortcut`, on macOS when `mac`. */
export function matches(shortcut: Shortcut, press: KeyPress, mac: boolean): boolean {
  const s = parseShortcut(shortcut);
  const mod = mac ? press.metaKey : press.ctrlKey;
  // On macOS, Ctrl is not a shortcut modifier: a press with it is someone else's.
  if (mod !== s.mod || press.altKey !== s.alt || (mac && press.ctrlKey)) return false;
  if (!SHIFT_FREE.test(s.key) && press.shiftKey !== s.shift) return false;
  if (/^[a-z]$/.test(s.key)) return shortcutLetter(press) === s.key;
  // Digits also match the physical key: on AZERTY the unshifted digit row types "à", "&"…
  if (/^[0-9]$/.test(s.key)) {
    return press.key === s.key || press.code === `Digit${s.key}` || press.code === `Numpad${s.key}`;
  }
  // [ and ] are the physical keys, as Photoshop reads them (^ and $ on AZERTY, which types
  // brackets with AltGr only).
  if (s.key === "[") return press.code === "BracketLeft";
  if (s.key === "]") return press.code === "BracketRight";
  if (s.key === "+") return press.key === "+" || press.key === "=" || press.code === "NumpadAdd";
  if (s.key === "-") {
    return press.key === "-" || press.key === "_" || press.code === "NumpadSubtract";
  }
  return press.key.toLowerCase() === s.key;
}

/** The command whose shortcut `press` is, if any. */
export function commandAt(press: KeyPress, mac: boolean): CommandId | null {
  for (const [id, shortcuts] of Object.entries(SHORTCUTS)) {
    if (shortcuts.some((s) => matches(s, press, mac))) return id as CommandId;
  }
  return null;
}

/** Names of keys that are words, in the interface's language ("Maj", "Suppr"…). */
export type KeyNames = { shift: string; alt: string; delete: string; backspace: string };

const MAC_SYMBOLS: Record<string, string> = {
  delete: "⌦",
  backspace: "⌫",
};

/**
 * A shortcut as shown in menus, in each platform's own way: Photoshop's order on Windows and
 * Linux ("Alt+Shift+Ctrl+K"), symbols in Apple's order on macOS ("⌥⇧⌘K").
 */
export function formatShortcut(shortcut: Shortcut, names: KeyNames, mac: boolean): string {
  const s = parseShortcut(shortcut);
  const named: Record<string, string> = { delete: names.delete, backspace: names.backspace };
  if (mac) {
    const key = MAC_SYMBOLS[s.key] ?? s.key.toUpperCase();
    return `${s.alt ? "⌥" : ""}${s.shift ? "⇧" : ""}${s.mod ? "⌘" : ""}${key}`;
  }
  const key = named[s.key] ?? s.key.toUpperCase();
  return [s.alt && names.alt, s.shift && names.shift, s.mod && "Ctrl", key]
    .filter(Boolean)
    .join("+");
}
