# 0030 — The panels dock

Status: accepted (2026-10-04, maintainer's choice among four models; details left to the
implementation).

## Context

The right column held Layers and, below it, Properties while an adjustment or fill layer was
selected. Saved selections need a Selections panel, History and others will follow, and a
Window menu will list them. The maintainer weighed horizontal tabs, vertical accordions, a
Blender-like icon bar and a real docking system (detachable panels were dropped earlier).

## Decision

1. **Layers stays on top**, always shown: it is used all the time.
2. **Below it, a dock**: a row of tab icons and one panel unfolded under them. A click on a
   tab unfolds its panel; on the unfolded one's tab, the dock folds down to its icons and
   Layers takes the room. The unfolded tab shows its name, the others their icon (and their
   name on hover).
3. The dock's top edge resizes it (Layers keeps a minimum); a double-click puts the default
   height back. The panel unfolded and the height are remembered on this machine.
4. Selecting an adjustment or fill layer unfolds Properties, as Photoshop shows it.
5. Panels are listed once (`DOCK_PANELS`): a Window menu can show or unfold them later.

## Alternatives

- **Tabs for the whole column**: Layers would hide behind another tab.
- **Accordions**: several panels at once, but they compete for height and Layers moves when one
  opens.
- **An icon bar on the side (Blender)**: compact and extensible, one panel at a time beside
  Layers, less discoverable without names.
- **A docking system**: flexible, much more to build, close to the detachable panels dropped.

## Consequences

- `app/src/lib/panelDock.ts` (state, clamping, storage) and `PanelDock.svelte`; Properties and
  Selections are its first panels. Properties lost its own tab row.
- A Window menu, History, or more zones (tabs inside the dock's zone, a second dock) can come
  without changing Layers.
