// What a key press does in the app window, before the focused control sees it: Escape's
// cancellations, Ctrl+Tab, the tools' letters, the brushes' [ and ], D and X, then the
// commands' shortcuts (`commands.ts`). The order matters: the first rule that applies wins.
// The app performs the action.

import { commandAt, shortcutLetter, type CommandId, type KeyPress } from "./commands";
import { isEraser, isPaintTool, slotForLetter, type ToolId, type ToolSlot } from "./tools";

/** A key press as the rules read it: `KeyboardEvent`'s fields, and where it goes. */
export type KeyEvent = KeyPress & {
  repeat: boolean;
  /** The focused element types text (its own keys: undo, copy, letters…). */
  inTextField: boolean;
  /** The focused element is a list (its own letters and Delete). */
  inSelect: boolean;
};

/** What the app is doing, as far as keys are concerned. */
export type KeyContext = {
  mac: boolean;
  tool: ToolId;
  aiTask: boolean;
  layerTransfer: boolean;
  tabDrag: boolean;
  /** A dialog is open: any (`modal` false) or a modal one. */
  dialogOpen: (modal: boolean) => boolean;
  command: (id: CommandId) => { whileTyping?: boolean; repeats?: boolean; disabled?: boolean };
};

export type KeyAction =
  /** Not the app's: the key goes on. */
  | { kind: "none" }
  /** The app's, but nothing to do (held, disabled): only the default action is prevented. */
  | { kind: "swallow" }
  | { kind: "cancelAiTask" }
  /** Escape ends a drag of layers to another tab, or of a tab (the key goes on). */
  | { kind: "endLayerTransfer" }
  | { kind: "cancelTabDrag" }
  | { kind: "cycleTabs"; step: 1 | -1 }
  /** A tool's letter: its slot, the next variant with Shift. */
  | { kind: "toolSlot"; slot: ToolSlot; next: boolean }
  /** The Brush's or an eraser's size (`[`, `]`), or with Shift its hardness. */
  | { kind: "paintSize"; larger: boolean; eraser: boolean }
  | { kind: "paintHardness"; larger: boolean; eraser: boolean }
  /** D: black and white; X: swapped. */
  | { kind: "defaultColors" }
  | { kind: "swapColors" }
  | { kind: "quickSelectionSize"; larger: boolean }
  | { kind: "command"; id: CommandId };

/** Whether `target` (an event's) is a field typing text: a text area, a text or number input. */
export function isTextField(target: EventTarget | null): boolean {
  const element = target as { tagName?: unknown; type?: unknown } | null;
  return (
    element?.tagName === "TEXTAREA" ||
    (element?.tagName === "INPUT" && ["text", "number", "search"].includes(String(element.type)))
  );
}

export function keyAction(e: KeyEvent, ctx: KeyContext): KeyAction {
  if (e.key === "Escape" && ctx.aiTask) return { kind: "cancelAiTask" };
  if (e.key === "Escape" && ctx.layerTransfer) return { kind: "endLayerTransfer" };
  if (e.key === "Escape" && ctx.tabDrag) return { kind: "cancelTabDrag" };
  // Ctrl+Tab everywhere, like browsers and most editors (Cmd+Tab belongs to macOS).
  if (e.ctrlKey && e.key === "Tab") return { kind: "cycleTabs", step: e.shiftKey ? -1 : 1 };
  const modifier = ctx.mac ? e.metaKey : e.ctrlKey;
  const bare = !modifier && !e.altKey && !e.inTextField;
  // The tools: a letter alone (V, M, C), Shift+letter for the next variant, as in Photoshop.
  const slot = bare ? slotForLetter(shortcutLetter(e)) : null;
  if (slot) return e.repeat ? { kind: "swallow" } : { kind: "toolSlot", slot, next: e.shiftKey };
  // [ and ], by the physical keys as in Photoshop (^ and $ on AZERTY).
  const bracket = e.code === "BracketLeft" || e.code === "BracketRight";
  const larger = e.code === "BracketRight";
  if (bare && bracket && isPaintTool(ctx.tool)) {
    // The size; with Shift, the hardness by quarters (Photoshop's steps).
    const eraser = isEraser(ctx.tool);
    return { kind: e.shiftKey ? "paintHardness" : "paintSize", larger, eraser };
  }
  const letter = shortcutLetter(e);
  if (bare && !e.shiftKey && (letter === "d" || letter === "x")) {
    return { kind: letter === "d" ? "defaultColors" : "swapColors" };
  }
  if (bare && bracket && ctx.tool === "quickSelection") {
    return { kind: "quickSelectionSize", larger };
  }
  // The commands' shortcuts.
  const id = commandAt(e, ctx.mac);
  if (!id) return { kind: "none" };
  const command = ctx.command(id);
  // Text fields keep their own keys (undo, copy, paste…), a list its letters and Delete, and
  // a modal dialog all of them.
  if (!command.whileTyping && e.inTextField) return { kind: "none" };
  const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
  if (plain && e.inSelect) return { kind: "none" };
  if (ctx.dialogOpen(!plain)) return { kind: "none" };
  if ((e.repeat && !command.repeats) || command.disabled) return { kind: "swallow" };
  return { kind: "command", id };
}
