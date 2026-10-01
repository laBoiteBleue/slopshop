// The tools of the toolbar (ADR 0013): what a left press on the image does. Listed in
// Photoshop's order with its single-key shortcuts, variants grouped in one slot as in Photoshop
// (Shift+key cycles them). Only tools that do what nothing else does are listed: no Hand or Zoom
// tool, since Space+drag, the middle button, the wheel and the zoom slider already do that with
// any tool (maintainer's choice, 2026-10-01).
import type { IconName } from "./Icon.svelte";
import type { MessageKey } from "./i18n/en";

export type ToolId = "move" | "marquee" | "ellipse" | "crop";

export type Tool = {
  id: ToolId;
  icon: IconName;
  name: MessageKey;
};

/** One button of the toolbar: a tool, or a group of variants sharing a key. */
export type ToolSlot = {
  /** The key that picks the slot: a letter, matched by its physical key (`KeyM`). */
  key: string;
  tools: Tool[];
};

export const SLOTS: readonly ToolSlot[] = [
  { key: "V", tools: [{ id: "move", icon: "move", name: "tools.move" }] },
  {
    key: "M",
    tools: [
      { id: "marquee", icon: "marquee", name: "tools.marquee" },
      { id: "ellipse", icon: "ellipse", name: "tools.ellipse" },
    ],
  },
  { key: "C", tools: [{ id: "crop", icon: "crop", name: "tools.crop" }] },
];

export const TOOLS: readonly Tool[] = SLOTS.flatMap((slot) => slot.tools);

export function toolInfo(id: ToolId): Tool {
  return TOOLS.find((tool) => tool.id === id) ?? TOOLS[0];
}

export function slotOf(id: ToolId): ToolSlot {
  return SLOTS.find((slot) => slot.tools.some((tool) => tool.id === id)) ?? SLOTS[0];
}

/** The slot a key picks (`KeyboardEvent.code`, so that every layout agrees), if any. */
export function slotForKey(code: string): ToolSlot | null {
  return SLOTS.find((slot) => code === `Key${slot.key}`) ?? null;
}

/** The tools that draw a selection. */
export function isSelectionTool(id: ToolId): boolean {
  return id === "marquee" || id === "ellipse";
}
