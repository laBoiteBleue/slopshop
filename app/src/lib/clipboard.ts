// The Edit menu's pastes and what the clipboard holds.

import type { CommandId } from "./commands";
import type { ClipboardContents } from "./engine";

/**
 * Whether paste command `id` does not apply to what the clipboard holds (`null`: not known
 * yet, nothing grayed). Paste needs something; Paste in Place, something with a place (pixels
 * copied from a document, or layers); Paste Into, pixels.
 */
export function pasteUnfit(clipboard: ClipboardContents | null, id: CommandId): boolean {
  if (clipboard === null) return false;
  switch (id) {
    case "paste":
      return clipboard === "nothing";
    case "pasteInPlace":
      return clipboard !== "placed" && clipboard !== "layers";
    case "pasteInto":
      return clipboard === "nothing" || clipboard === "files";
    default:
      return false;
  }
}
