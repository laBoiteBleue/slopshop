# Ergonomics

How SlopShop should feel, and the ideas waiting for the maintainer's decision. Technical plans
are in the [roadmap](roadmap.md); this page is about what a user sees and does. Proposed ideas
are unchecked: the maintainer validates (✅), rejects (strike it) or adds some before they are
built. Contributors: propose here first.

## Principles

1. **Familiar, not frozen** ([ADR 0013](adr/0013-familiar-layout.md)). Photoshop's layout, names,
   shortcuts and gestures where a Photoshop user would look for them; modernized where
   Photoshop shows its age (one Save As that continues as an export, no obsolete formats in the
   way).
2. **Direct manipulation.** Drag and drop works wherever it makes sense: files, folders, zips,
   pasted images, tabs, layers, between documents. Hovering a target (a tab) opens it.
3. **Expected gestures just work.** A click in the empty area deselects, right-click shows the
   commands of what is under the pointer, Ctrl/Shift+click select several, Escape cancels, a
   double-click renames.
4. **Nothing is lost silently.** Every action is undoable, one undo entry per gesture;
   non-destructive by default; whatever cannot be kept (a PSD effect, a color profile) is
   reported, next to what it concerns.
5. **Native feel.** No web page behavior (text selection, browser menus and shortcuts), instant
   feedback, heavy work off the UI thread, the window and the app remember their state.
6. **Few dialogs.** A dialog only when a choice is needed (export options); otherwise act, show a
   short notice, and make it undoable.

## Done

- Menu bar in Photoshop's order with its shortcuts; Save As lists every format and continues as
  an export; Export (Shift+Ctrl+E); Import as Layers (Shift+Ctrl+O).
- One instance: opening a file while the app runs focuses it; the window remembers its size and
  place.
- Drop files on the image or the layers panel: layers; elsewhere: new tabs. Folders and zips open
  like several files. Paste (Ctrl+V) an image or a file as a layer.
- A layered file imported into a document arrives as one group named after it.
- Tabs: reorder by dragging, rename by double-click, drop a tab on another image to copy its
  layers, Ctrl+Tab.
- Layers panel: thumbnails, visibility, live opacity, blend modes, rename (double-click, F2),
  drag to reorder; several layers with Ctrl/Shift+click, Select > All Layers (Alt+Ctrl+A); a
  click in the empty area deselects; Delete deletes; groups as folders that fold, Ctrl+G,
  Shift+Ctrl+G, drag into folders, Pass Through; mask thumbnail, Shift+click disables the mask;
  Duplicate (Ctrl+J); right-click menu; drag layers onto another tab to copy them there, with a
  thumbnail following the pointer; clipping masks (Alt+Ctrl+G, Alt+click on the line between two
  layers; clipped layers indented with an arrow, the base underlined).
- Move tool (the default action on the image, until the tools palette): drag moves the selected
  layers, arrows nudge by 1 pixel, Shift+arrows by 10; one undo entry per drag.

## Proposed

### Layers panel

- [ ] Alt+click on an eye shows only that layer (again: shows them all back).
- [ ] Drag over several eyes to show or hide them in one stroke.
- [ ] Ctrl+[ and Ctrl+] move the selected layers down and up; Shift+Ctrl+[ and ] to the bottom
      and the top.
- [ ] Alt+[ and Alt+] select the layer below and above (with Shift: add it to the selection).
- [ ] Drag a layer onto the "+" button duplicates it, onto the trash deletes it (Photoshop).
- [ ] Shift+Ctrl+N: new empty layer (once painting exists).
- [ ] Scrubby labels: dragging the "Opacity" label changes the value (Photoshop).
- [ ] Shift++ / Shift+- cycle through the blend modes of the selected layers.
- [ ] Layer locks: transparency, pixels, position, all (padlock icons).
- [ ] Color labels on layers, and a filter/search box above the list.
- [ ] A warning badge on each layer with import warnings (hover for the details), instead of
      only the document's notice.
- [ ] Alt+click on the mask thumbnail shows the mask alone on the canvas.
- [ ] Double-click the thumbnail opens the layer's properties (name, color label, blend options).

### Tabs and documents

- [ ] Middle-click closes a tab.
- [ ] File > Open Recent (with thumbnails).
- [ ] Reopen the tabs of the last session at startup (optional).
- [ ] Drag a tab out of the window to open it in a window of its own.
- [ ] A dot on the tab of an unsaved document, and a clear prompt listing them on quit.

### Canvas and view

- [ ] Right-click on the canvas lists the layers under the pointer to select one (Photoshop
      with the Move tool).
- [ ] Alt+wheel zooms without Ctrl; R rotates the view (non-destructive, like Photoshop).
- [ ] Pixel grid at high zoom; rulers and guides (Ctrl+R, drag from a ruler).
- [ ] Number keys set the opacity of the selected layers (1 = 10 %, 0 = 100 %), as in
      Photoshop.

### Import, export, feedback

- [ ] Quick Export as PNG (Alt+Shift+Ctrl+W) with the last settings, no dialog.
- [ ] Drag an image straight from a web browser (deferred: needs a native drop target).
- [ ] Notices grouped per document, dismissed with one click, never modal.
- [ ] Preferences: interface size, theme, language (today in Edit > Language), tile cache size.

### Tools (when they arrive)

- [ ] Tools palette on the left in Photoshop's order, with its single-key shortcuts (V, M, L, W,
      B, E…) and Shift+key to cycle a tool group; options bar under the menu bar.
- [ ] Brush size and hardness with [ and ], Alt+right-drag to resize on the canvas.
