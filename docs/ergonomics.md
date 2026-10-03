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
7. **Modern controls** (maintainer's choice, 2026-10-01). Buttons in the spirit of Material UI,
   normal case: the main action contained (accent, soft shadow), the others text buttons, icon
   buttons with a round hover, a ripple on press. Dense rows (layers, tabs, menus) keep the
   editor's compact style.
8. **Groups keep their adjustments** (maintainer's choice, 2026-10-01). New groups, and a document
   copied into another as a group, are isolated (blend mode Normal): an adjustment layer inside
   changes the group only. Photoshop passes through by default; Pass Through stays one choice
   away in the blend modes.

## Done

- Menu bar in Photoshop's order with its shortcuts, each defined once for the menus and the
  keyboard, on macOS with ⌘ and Apple's symbols; Edit > Keyboard Shortcuts (Alt+Shift+Ctrl+K)
  lists them all, with a filter (read only); Redo is Shift+Ctrl+Z or Ctrl+Y; Save As lists every format and continues as
  an export; Export (Shift+Ctrl+E); Import as Layers (Shift+Ctrl+O).
- One instance: opening a file while the app runs focuses it; the window remembers its size and
  place.
- Drop files on the image or the layers panel: layers; elsewhere: new tabs. Folders and zips open
  like several files. Paste (Ctrl+V) an image or a file as a layer;
  Copy and Cut (Ctrl+C, Ctrl+X) the selected layers, Paste them on top of any document or as a new
  one. Several images in one file (PDF pages, DICOM slices, animation frames) open
  as one document: one isolated group, the first image shown, the others hidden; the eye of a
  layer in a multi-selection shows or hides the whole selection.
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
- Tools: a toolbar on the left in Photoshop's order, one key each: Move (V, the tool at
  startup), the marquees (M), the lassos (L), the Magic Wand (W) and Crop (C); the active one is highlighted, its name and key in
  the tooltip. Variants share one slot, as in Photoshop: the slot shows the one used last, with
  a corner mark; a right-click or a long press lists them, Shift+key cycles them. An
  options bar under the menu bar shows the active tool's icon and its own settings only. No
  Hand or Zoom tool, and no view or apply buttons in the options bar (maintainer's choice,
  2026-10-01): Space+drag, the middle button, the wheel, Enter and Esc already do that with any
  tool. Next to the zoom slider in the status bar, two icon buttons: 100% and Fit on Screen.
- Move tool: drag moves the selected
  layers, arrows nudge by 1 pixel, Shift+arrows by 10; one undo entry per drag. The pointer stays
  the normal arrow over the image (no move cross). Auto-Select (options bar, on by default):
  the drag takes the layer whose pixels are under the pointer (inside a group, the layer itself;
  a layer already selected keeps the whole selection moving); Ctrl held inverts it. Snap:
  the edges and centers of what moves stick to those of the canvas and of the other visible
  layers within 6 screen pixels, with magenta smart guides; Ctrl held moves freely; View > Snap
  turns it off. A drag from inside the selection moves the selected pixels instead (Photoshop):
  those of the active layer, or of its mask when the mask is the target, with the selection,
  leaving a hole (transparent, or hidden in a mask); Alt copies them; arrows nudge them. As in
  Photoshop they float until something else happens: moving them again brings back what they
  covered. Pixels moved off the canvas are kept: the layer grows to hold them. The move is kept as the layer's paint (ADR 0027): Delete Paint restores the original.
- Free Transform (Ctrl+T, Edit > Free Transform, or a double-click on a layer in the image): a
  box around the selected layers with eight
  handles. Drag inside to move (Shift: along one axis); a corner scales keeping the proportions
  (Shift: freely), a side scales one way (Shift: proportionally), Alt scales about the center;
  drag outside to rotate about the center (Shift: steps of 15°). A readout next to the pointer
  shows the move, the size in % or the angle. Enter, a double-click inside or a click outside
  (without dragging) applies it as one undo entry; Esc or Ctrl+Z cancels it; Ctrl+T again,
  another edit or another tab applies it, without a question or buttons (undo is there for
  that).
  Moving the box and dragging a handle snap to the canvas and the other layers with magenta
  smart guides, like the Move tool; a handle also snaps where the box gets the same width or
  height as another layer (a magenta measure across the middle of both, with end ticks like the serifs of an I). Ctrl held: freely; View > Snap turns
  it off. Pixels are
  never resampled into the layer: the transform stays editable.
- Edit > Transform: Rotate 180°, 90° clockwise and counter clockwise, Flip Horizontal and
  Vertical, about the center of the selected layers, exact (pixels are copied, not resampled).
- Image > Image Size (Alt+Ctrl+I) and Canvas Size (Alt+Ctrl+C), Photoshop's dialogs: width and
  height in pixels or percent; Image Size keeps the proportions by default, Canvas Size has
  Relative and a 3×3 anchor. Image > Image Rotation: 180°, 90° both ways, flip the canvas. All
  undoable, and nothing is cut: pixels outside the canvas are kept, resizing resamples when
  shown.
- Adjustment layers (Layer > New Adjustment Layer, in Photoshop's order): Brightness/Contrast,
  Levels, Exposure, Vibrance, Hue/Saturation, Invert, Posterize, Threshold, placed
  above the active layer (in its folder) and selected. A Properties panel below Layers (the
  list never moves; the maintainer's choice) shows the selected adjustment's parameters: sliders applied live (one undo entry
  per drag), number fields, Reset. An adjustment icon in the layers list; the blend mode stays
  normal for now.
- Crop tool (C, Image > Crop): a frame on the whole image with eight handles, the outside
  shaded and the rule of thirds inside. Drag inside to move it, a handle to resize it (Shift on
  a corner keeps the proportions), outside to draw a new one; edges snap to the canvas, the
  layers and their sizes, with magenta guides (Ctrl: freely). A readout shows the size. Enter,
  a double-click inside or a click outside applies; Esc starts the frame over. As in Photoshop,
  the tool stays active: a new frame starts on the cropped canvas (and follows an undo); another
  tool drops the frame. Whole pixels only, and nothing
  is deleted: cropped pixels stay outside the canvas. Image > Crop to a selection comes with
  selections.

- Selections (ADR 0024): Rectangular and Elliptical Marquee (M): drag on the image, whole
  pixels, a size readout; Shift at the press adds, Alt subtracts, both intersect, otherwise the
  options bar's mode (new, add, subtract, intersect); during the drag Shift makes a square or a
  circle and Alt draws from the center (once the press keys are released); a click deselects. A
  small +, − or × next to the pointer shows how the next shape combines (maintainer's idea).
  Lasso (L): drag to draw freehand, the release closes the outline. Polygonal Lasso (Shift+L):
  a click per corner, the line to the pointer by steps of 45° with Shift; a click on the first
  corner (a small circle tells), a double-click or Enter closes it, Backspace removes the last
  corner, Esc drops it. Same keys, badge and click-to-deselect as the marquees.
  Magic Wand (W): a click selects the pixels of a similar color (Tolerance, 0–255 on the
  displayed 8-bit values, 32 by default), connected to the clicked one (Contiguous) or anywhere;
  it samples the active layer, or the image as displayed with Sample All Layers (unchecked by
  default, as in Photoshop); Anti-alias softens its edge by about a pixel.
  Select > Color Range: a panel beside the image (not modal): click colors on the image or on its
  preview (Shift adds one, Alt takes one away, or the three eyedroppers), Fuzziness 0–200,
  Invert; the preview shows the selection it would make, live; Enter applies, Esc cancels; a
  current selection limits it, as in Photoshop.
  Options bar: the four modes, Feather (px), Anti-alias for the ellipse and the lassos. Marching ants follow the
  selection at any zoom, where coverage crosses one half. Quick Mask (Q, Select > Edit in Quick
  Mask Mode) shows a soft edge: what the selection leaves out is tinted red, half opaque, fading
  where it is soft; the ants hide meanwhile (the maintainer preferred it to dotted limits around
  the ants). Painting in Quick Mask comes with the brushes. Select menu: All (Ctrl+A), Deselect (Ctrl+D), Reselect (Shift+Ctrl+D),
  Inverse (Shift+Ctrl+I), Modify (Border, Smooth, Expand, Contract, Feather with Shift+F6: a
  dialog with one number of pixels, remembered for the session; the canvas edge is not an
  outline, as in Photoshop by default). Selecting is undoable; crop and size changes deselect.
- Layer > Layer Mask: Reveal All, Hide All, Reveal Selection, Hide Selection on the selected
  layers without a mask (the selection follows the layer's transform; Reveal All and Reveal
  Selection also in the layers' right-click menu); a mask from the selection deselects, as in
  Photoshop. Image > Crop crops to the selection's bounds when there is one (otherwise it picks
  the Crop tool).

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

- [ ] Free Transform: an options bar with X, Y, W, H and angle fields and a link to keep the
      proportions (Photoshop's).
- [ ] Free Transform: Ctrl+drag a handle skews (Photoshop's distort needs perspective, which
      affine transforms cannot do), a movable pivot point, and the right-click menu of the box
      (flip, rotate 90°).
- [ ] Arrow keys move the Free Transform box, as in Photoshop.
- [ ] Smart guides also show equal spacing between three layers or more (Photoshop's distance
      marks), and the distance to the nearest layer while Alt-dragging.
- [ ] An option to make Auto-Select pick the top group rather than the layer (Photoshop's
      "Group / Layer" choice).
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
- [ ] Preferences: interface size, theme, tile cache size (the language is there already).
- [ ] Customizable keyboard shortcuts in Edit > Keyboard Shortcuts (the list exists).

### Tools (when they arrive)

- [ ] The active tool and its options remembered across sessions, as in Photoshop.
- [ ] Spring-loaded tools: holding a tool's key uses it until the key is released.
- [ ] Brush size and hardness with [ and ], Alt+right-drag to resize on the canvas.
