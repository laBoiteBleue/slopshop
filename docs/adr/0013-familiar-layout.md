# 0013 — A layout familiar to Photoshop users

Status: accepted (2026-09-30).

## Context

SlopShop wants to win over Photoshop users, many of them frustrated by its price, subscription,
cloud or limits, who have years of habits: where commands and tools are, their shortcuts. Every
difference costs them effort in the first hours, when most people give up on a new editor.
Editors that copied Photoshop's layout closely (Photopea) were adopted widely; editors that
departed from it (GIMP, to a lesser extent Affinity) are often criticized for their learning
curve. SlopShop will soon have many commands (image size, rotation, adjustments, filters…): a
row of icons cannot hold them.

## Decision

1. **Photoshop's layout and shortcuts by default**, with SlopShop's own look (colors, type,
   icons: never Adobe's icons or branding):
   - a **menu bar** holding every command, in Photoshop's order and names: File, Edit, Image,
     Layer, Select, Filter, View, Window, Help;
   - a vertical **toolbar** on the left for tools only (what is done with the pointer on the
     image), variants grouped in one slot;
   - an **options bar** at the top for the settings of the active tool;
   - **panels** on the right (layers, adjustments, properties, history).
2. **Shortcuts are Photoshop's** where a command exists in both (Ctrl+N, Ctrl+O, Ctrl+S,
   Ctrl+Shift+S, Ctrl+W, Ctrl+Q, Ctrl+Z / Ctrl+Shift+Z, Ctrl+0, Ctrl+1, Ctrl++ / Ctrl+-, F2…).
3. **Menus list only what exists** (README honesty rule): a menu appears when it has a command,
   in its Photoshop position; no disabled placeholders for features that do not exist.
4. **The menu bar is drawn in the window** on Windows and Linux, as Photoshop does, translated
   like the rest of the UI. On macOS it should become the system menu bar (native menu); until
   that is done and tested on a Mac, the in-window bar is used there too.
5. What SlopShop does differently (the node graph, AI nodes, huge images, non-destructive
   transforms) is added to this frame, not in place of it.
6. **Familiar, not frozen** (maintainer decision): where Photoshop shows its age, commands are
   merged or simplified, and features obsolete in 2026 are not reproduced (Save for Web
   (Legacy), PICT or EPS export…). First case: **Save As works as in older Photoshop
   versions** (Ctrl+Shift+S): its dialog lists the SlopShop document format first, then every
   image format, and choosing an image format continues as an export. **Export**
   (Ctrl+Shift+E) is the same dialog with the image formats only, the last one used first. A
   `.slop` file becomes the document's file; an image format writes a flattened copy after its
   options, and the document keeps its own file. So Ctrl+S never overwrites a JPEG with a
   flattened, recompressed image (the trap of those older versions, which is why Adobe later
   split "Save a Copy" out). Saving before closing offers `.slop` only.

## Alternatives

- **A modern, icon-first UI of our own**: attractive, but every Photoshop user relearns it, and
  it does not scale to hundreds of commands without menus anyway.
- **An exact copy, icons included**: fastest to learn, but Adobe's icons and branding are theirs,
  and SlopShop needs its own identity.

## Consequences

- The icon row of the title bar (new, open, save, export, undo, redo) and the language selector
  move into the File and Edit menus.
- Commands are defined once and reachable from the menu and the keyboard.
- Tools, the options bar and more panels come with the features that need them.
