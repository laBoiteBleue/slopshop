// Platform conventions the UI must follow to feel native.

/** macOS (WKWebView): Cmd is the shortcut modifier, Ctrl/Option keep their Cocoa meanings. */
export const isMac = /Mac|iPhone|iPad/.test(navigator.userAgent);

export const isWindows = /Windows/.test(navigator.userAgent);

/** Label of the shortcut modifier, for hints and tooltips. */
export const modifierLabel = isMac ? "⌘" : "Ctrl";

/** Whether the platform's shortcut modifier is held (Cmd on macOS, Ctrl elsewhere). */
export function hasShortcutModifier(e: KeyboardEvent | MouseEvent): boolean {
  return isMac ? e.metaKey : e.ctrlKey;
}

/**
 * `onmousedown` of toolbar-like buttons (tools, options, view icons): a click acts without
 * taking the keyboard focus, as native toolbars do, so no focus ring appears when a key is
 * pressed next and Space or Enter keep going to the image.
 */
export function keepFocus(e: MouseEvent) {
  e.preventDefault();
}

/**
 * The letter of a letter shortcut, lower case, as Photoshop reads it: the letter the key types
 * on a Latin layout (AZERTY's A key is "a", wherever it sits), else the physical key, for
 * layouts without Latin letters (Cyrillic, Greek…) or when AltGr (Ctrl+Alt on Windows) types
 * another character. `null` for keys that are not letters.
 */
export function shortcutLetter(e: KeyboardEvent): string | null {
  if (/^[a-z]$/i.test(e.key)) return e.key.toLowerCase();
  const physical = /^Key([A-Z])$/.exec(e.code);
  return physical ? physical[1].toLowerCase() : null;
}
