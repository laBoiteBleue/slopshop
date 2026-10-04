# 0036 — Panels, the Window menu and the saved layout

Status: accepted (2026-10-04, after the Window menu audit; the maintainer left the ergonomics to
Claude: "intuitive for a Photoshop user, without its flaws"). Extends
[ADR 0030](0030-panel-dock.md).

## Context

ADR 0030 put Layers on top of the right column and a foldable dock of icon tabs below it. The
audit of the Window menu found what the next panels would trip on: the active layer lived in the
Layers panel's component (switching tabs lost it), each panel was wired by hand in App.svelte
(its id in one list, its tab in another, its props in a branch of the dock), the dock and the
column's width were saved as two unrelated records, and nothing could be put back as it was.
Photoshop's own flaws to avoid: panels closed or floating by mistake and lost, workspaces that
drift and need resetting, Properties taking over the panel being looked at, F-keys for panels,
commands hidden in panel flyout menus.

## Decision

1. **Layers stays on top and the dock below it** (ADR 0030). No panel closes or floats: each one
   stays a tab of the dock, at worst folded. No floating panels, detached windows or several
   views of a document for now (a Navigator would bring the engine's several views first).
2. **The Layers panel's state is the document's, not the component's**: selection, folded groups
   and stacks, mask targets, scroll, kept per open document (`layersUi.svelte.ts`).
3. **Panels are listed once** (`app/src/lib/panels/registry.ts`: stable id, icon, title) with
   their component (`panels/index.ts`). A panel reads the app through a context
   (`panels/context.ts`: the active document and layer, the edit functions); the dock's tabs, the
   Window menu and the layout follow the list. Adding a panel: one component, one line in each
   file, its title in the catalogs.
4. **One saved layout** (`slopshop.layout`, versioned): the dock's tabs in order, the unfolded
   one and its height, the column's width, whether the options bar and the toolbar are shown.
   Read part by part: an unknown panel is dropped, a missing one gets its default place, an odd
   value its default; the records of before (dock, width) are read once. Documents are not part
   of it. No named workspaces until panels make them differ (Histogram, Navigator, Color…).
5. **Window menu**: the dock's panels (checked while unfolded), then Options Bar, Toolbar, Hide
   Panels (Tab), then Reset Layout. No shortcuts for panels.
6. **Properties gives the dock back**: it unfolds when an adjustment or fill layer is selected
   (as in Photoshop) and, once none is, the dock shows again what Properties replaced, unless the
   user chose a panel meanwhile.
7. **Tabs are reordered by dragging them** along the dock's row.
8. **History is the dock's third panel**: the steps oldest first under the initial state, the
   current one marked, those undone dimmed until the next change; a click goes to the state
   after a step (undo or redo in one call). Each history entry has a label in the engine
   (`HistoryLabel`: an identifier the UI translates, and the adjustment or filter applied),
   named after its edit unless its command names it (the tool, the menu command). The panel
   asks for the history only while it shows. No snapshots and no non-linear history (later, if
   asked).

## Alternatives

- **A docking system** (groups to drag panels between, floating panels, workspaces): flexible,
  much more to build, and the source of Photoshop's lost panels. Several tab groups in the column
  remain possible on the same registry and layout when the dock holds too many panels.
- **Closable panels**: needless while every panel fits as an icon tab.
- **Workspace presets now** (Essentials, Photography, Painting): they would all be the same.

## Consequences

- `panelDock.ts` and `panelWidth.ts` keep their rules; the storage is `layout.ts`'s.
- The app tests' harness starts each test with empty storage (the app saves its layout as soon
  as it shows).
- Keyboard navigation of the dock's tabs is not provided: Tab hides the panels (Photoshop's), so
  focus never walks to them; their commands are in the menus.
