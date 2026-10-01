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
