// The tools of the toolbar (ADR 0013): what a left press on the image does. Listed in
// Photoshop's order with its single-key shortcuts, variants grouped in one slot as in Photoshop
// (Shift+key cycles them). Only tools that do what nothing else does are listed: no Hand or Zoom
// tool, since Space+drag, the middle button, the wheel and the zoom slider already do that with
// any tool (maintainer's choice, 2026-10-01).
import type { IconName } from "./Icon.svelte";
import type { MessageKey } from "./i18n/en";

export type ToolId =
  | "move"
  | "marquee"
  | "ellipse"
  | "lasso"
  | "polygonalLasso"
  | "objectSelection"
  | "quickSelection"
  | "wand"
  | "crop"
  | "eyedropper"
  | "brush"
  | "cloneStamp"
  | "healingBrush"
  | "eraser"
  | "restoreEraser"
  | "paintBucket"
  | "gradient"
  | "dodge"
  | "burn"
  | "blur"
  | "sharpen"
  | "smudge"
  | "pen"
  | "directSelection"
  | "shapeRectangle"
  | "shapeEllipse"
  | "shapePolygon"
  | "shapeLine";

export type Tool = {
  id: ToolId;
  icon: IconName;
  name: MessageKey;
};

/** One button of the toolbar: a tool, or a group of variants sharing a key. */
export type ToolSlot = {
  /** The key that picks the slot: a letter (as typed: AZERTY's M is M); none if empty. */
  key: string;
  tools: Tool[];
};

export const SLOTS: readonly ToolSlot[] = [
  { key: "V", tools: [{ id: "move", icon: "pointer", name: "tools.move" }] },
  {
    key: "M",
    tools: [
      { id: "marquee", icon: "marquee", name: "tools.marquee" },
      { id: "ellipse", icon: "ellipse", name: "tools.ellipse" },
    ],
  },
  {
    key: "L",
    tools: [
      { id: "lasso", icon: "lasso", name: "tools.lasso" },
      { id: "polygonalLasso", icon: "polygonalLasso", name: "tools.polygonalLasso" },
    ],
  },
  {
    key: "W",
    tools: [
      { id: "objectSelection", icon: "objectSelection", name: "tools.objectSelection" },
      { id: "quickSelection", icon: "quickSelection", name: "tools.quickSelection" },
      { id: "wand", icon: "wand", name: "tools.wand" },
    ],
  },
  { key: "C", tools: [{ id: "crop", icon: "crop", name: "tools.crop" }] },
  { key: "I", tools: [{ id: "eyedropper", icon: "eyedropper", name: "tools.eyedropper" }] },
  // Retouching by intention: Healing (Remove, an AI tool, joins it later).
  { key: "J", tools: [{ id: "healingBrush", icon: "healing", name: "tools.healingBrush" }] },
  { key: "B", tools: [{ id: "brush", icon: "brush", name: "tools.brush" }] },
  // Paints pixels taken elsewhere (Alt+click sets where).
  { key: "S", tools: [{ id: "cloneStamp", icon: "stamp", name: "tools.cloneStamp" }] },
  {
    key: "E",
    tools: [
      { id: "eraser", icon: "eraser", name: "tools.eraser" },
      // Brings back a layer's original through its paint (ADR 0029).
      { id: "restoreEraser", icon: "restoreEraser", name: "tools.restoreEraser" },
    ],
  },
  {
    key: "G",
    tools: [
      { id: "gradient", icon: "gradient", name: "tools.gradient" },
      // The Magic Wand's region, filled as Edit > Fill does.
      { id: "paintBucket", icon: "bucket", name: "tools.paintBucket" },
    ],
  },
  // No key, as in Photoshop.
  {
    key: "",
    tools: [
      { id: "blur", icon: "blur", name: "tools.blur" },
      { id: "sharpen", icon: "sharpen", name: "tools.sharpen" },
      { id: "smudge", icon: "smudge", name: "tools.smudge" },
    ],
  },
  {
    key: "O",
    tools: [
      { id: "dodge", icon: "dodge", name: "tools.dodge" },
      { id: "burn", icon: "burn", name: "tools.burn" },
    ],
  },
  // Paths (ADR 0041): the Pen draws a vector layer, Direct Selection moves its anchors.
  { key: "P", tools: [{ id: "pen", icon: "pen", name: "tools.pen" }] },
  {
    key: "A",
    tools: [{ id: "directSelection", icon: "directSelection", name: "tools.directSelection" }],
  },
  // Vector shapes (ADR 0041): each draws a vector layer.
  {
    key: "U",
    tools: [
      { id: "shapeRectangle", icon: "shapeRectangle", name: "tools.shapeRectangle" },
      { id: "shapeEllipse", icon: "shapeEllipse", name: "tools.shapeEllipse" },
      { id: "shapePolygon", icon: "shapePolygon", name: "tools.shapePolygon" },
      { id: "shapeLine", icon: "shapeLine", name: "tools.shapeLine" },
    ],
  },
];

export const TOOLS: readonly Tool[] = SLOTS.flatMap((slot) => slot.tools);

export function toolInfo(id: ToolId): Tool {
  return TOOLS.find((tool) => tool.id === id) ?? TOOLS[0];
}

export function slotOf(id: ToolId): ToolSlot {
  return SLOTS.find((slot) => slot.tools.some((tool) => tool.id === id)) ?? SLOTS[0];
}

/** The slot a letter picks (see `shortcutLetter`), if any. */
export function slotForLetter(letter: string | null): ToolSlot | null {
  if (!letter) return null;
  return SLOTS.find((slot) => slot.key.toLowerCase() === letter) ?? null;
}

/**
 * The tool a slot's key picks: the one it shows (`shown`, the last chosen), or with `next`
 * (Shift) the variant after it, going round.
 */
export function slotTool(slot: ToolSlot, shown: ToolId | undefined, next: boolean): ToolId {
  const current = shown ?? slot.tools[0].id;
  if (!next || slot.tools.length < 2) return current;
  const index = slot.tools.findIndex((tool) => tool.id === current);
  return slot.tools[(index + 1) % slot.tools.length].id;
}

/** The tools that paint (ADR 0027). */
export function isPaintTool(
  id: ToolId,
): id is
  | "brush"
  | "cloneStamp"
  | "healingBrush"
  | "dodge"
  | "burn"
  | "blur"
  | "sharpen"
  | "smudge"
  | "eraser"
  | "restoreEraser" {
  return id === "brush" || isCloneTool(id) || isToneTool(id) || isFocusTool(id) || isEraser(id);
}

/** Blur, Sharpen and Smudge: they soften, sharpen or smear what the layer shows. */
export function isFocusTool(id: ToolId): id is "blur" | "sharpen" | "smudge" {
  return id === "blur" || id === "sharpen" || id === "smudge";
}

/** Dodge and Burn: they lighten or darken what the layer shows where they paint. */
export function isToneTool(id: ToolId): id is "dodge" | "burn" {
  return id === "dodge" || id === "burn";
}

/** The tools that paint pixels taken elsewhere (Alt+click sets where): they share a source. */
export function isCloneTool(id: ToolId): id is "cloneStamp" | "healingBrush" {
  return id === "cloneStamp" || id === "healingBrush";
}

/** The erasers: they share their options, and paint no color. */
export function isEraser(id: ToolId): id is "eraser" | "restoreEraser" {
  return id === "eraser" || id === "restoreEraser";
}

/** The tools that draw a selection. */
export function isSelectionTool(id: ToolId): boolean {
  return (
    id === "marquee" ||
    id === "ellipse" ||
    id === "lasso" ||
    id === "polygonalLasso" ||
    id === "objectSelection" ||
    id === "quickSelection" ||
    id === "wand"
  );
}
