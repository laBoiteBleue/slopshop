// The tools of the toolbar (ADR 0013): what a left press on the image does. Listed in
// Photoshop's order with its single-key shortcuts; only tools that exist are listed.
import type { IconName } from "./Icon.svelte";
import type { MessageKey } from "./i18n/en";

export type ToolId = "move" | "crop" | "hand" | "zoom";

export type Tool = {
  id: ToolId;
  icon: IconName;
  name: MessageKey;
  /** The key that picks the tool: a letter, matched by its physical key (`KeyV`). */
  key: string;
};

export const TOOLS: readonly Tool[] = [
  { id: "move", icon: "move", name: "tools.move", key: "V" },
  { id: "crop", icon: "crop", name: "tools.crop", key: "C" },
  { id: "hand", icon: "hand", name: "tools.hand", key: "H" },
  { id: "zoom", icon: "zoom", name: "tools.zoom", key: "Z" },
];

/** The tool a key picks (`KeyboardEvent.code`, so that every layout agrees), if any. */
export function toolForKey(code: string): ToolId | null {
  return TOOLS.find((tool) => code === `Key${tool.key}`)?.id ?? null;
}
