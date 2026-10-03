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
  like several files. Several images in one file (PDF pages, DICOM slices, animation frames) open
  as one document: one isolated group, the first image shown, the others hidden; the eye of a
  layer in a multi-selection shows or hides the whole selection.
- A layered file imported into a document arrives as one group named after it.
- Tabs: reorder by dragging, rename by double-click, drop a tab on another image to copy its
  layers, Ctrl+Tab.
- Layers panel: thumbnails, visibility, live opacity, blend modes, rename (double-click, F2),
  drag to reorder; several layers with Ctrl/Shift+click, Select > All Layers (Alt+Ctrl+A); a
  click in the empty area deselects; Delete deletes; groups as folders that fold, Ctrl+G,
  Shift+Ctrl+G, drag into folders, Pass Through; mask thumbnail, Shift+click disables the mask;
  Duplicate (Ctrl+J without a selection: Layer via Copy duplicates, as in Photoshop; with a
  selection, Layer via Copy puts the selected pixels in a new layer, Layer via Cut (Shift+Ctrl+J)
  too, leaving a hole as paint); right-click menu; drag layers onto another tab to copy them there, with a
  thumbnail following the pointer; clipping masks (Alt+Ctrl+G, Alt+click on the line between two
  layers; clipped layers indented with an arrow, the base underlined). What was applied to a
  layer's pixels (paint, and later Image > Adjustments; ADR 0029) is listed below it, as
  Photoshop lists smart filters: an arrow at the end of the row unfolds the list (folded at
  first), newest on top: Paint, or the adjustment's name (×2 when applied twice in a row).
  Entries are not edited: the trash shown on hover, or a right-click > Delete, deletes one;
  what was applied above it follows, and neighbours that become alike join.
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
  covered. Pixels moved off the canvas are kept: the layer grows to hold them. During the drag
  the pixels float in the view, as fluid as moving a layer whatever the image; the move is
  computed once, on release. The move is kept as the layer's paint (ADR 0027): Delete Paint
  restores the original.
- Free Transform (Ctrl+T, Edit > Free Transform, or a double-click on a layer in the image): a
  box around the selected layers with eight
  handles. Drag inside to move (Shift: along one axis); a corner scales keeping the proportions
  (Shift: freely), a side scales one way (Shift: proportionally), Alt scales about the reference point;
  drag outside to rotate about the reference point (Shift: steps of 15°); Ctrl and a side
  handle skew (Alt: about the reference point). The reference point (the small circle, at the
  center at first) can be dragged anywhere, snapping to the handles and the center; rotations
  and Alt scaling turn about it. A right-click on the box: Rotate 180°, 90° both ways, Flip
  Horizontal and Vertical about the reference point, Apply, Cancel. The options bar shows Photoshop's
  fields meanwhile: X and Y of the reference point, W and H in % (linked by a chain), the angle
  and the skew; a value typed transforms the box at once. A readout next to the pointer
  shows the move, the size in % or the angle. Enter, a double-click inside or a click outside
  (without dragging) applies it as one undo entry; Esc or Ctrl+Z cancels it; Ctrl+T again,
  another edit or another tab applies it, without a question or buttons (undo is there for
  that).
  Moving the box and dragging a handle snap to the canvas and the other layers with magenta
  smart guides, like the Move tool; a handle also snaps where the box gets the same width or
  height as another layer (a magenta measure across the middle of both, with end ticks like the serifs of an I). Ctrl held: freely; View > Snap turns
  it off. Pixels are
  never resampled into the layer: the transform stays editable.
- Clipboard, as in Photoshop. Copy (Ctrl+C): with a selection, the selected pixels of the
  active raster layer (or of its mask when it is the target) with their alpha and place; without
  one, the selected layers whole (groups, masks, clipping, adjustments, transforms, paint: nothing
  rasterized). Cut (Ctrl+X): the same, then the pixels are erased (as paint, ADR 0027) or the
  layers deleted, one undo entry. Copy Merged (Shift+Ctrl+C): the visible layers composited in
  the selection, or the whole canvas without one. Other applications get an 8-bit image of any
  copy; SlopShop pastes its own while the system clipboard still holds that image. Paste
  (Ctrl+V): where it was copied when that is in sight, else in the middle of the view; an image
  from elsewhere in the middle of the view; files copied in the file manager are placed like
  dropped ones; the pasted layers are selected. Paste in Place (Shift+Ctrl+V): where it was
  copied, always. Paste Into (Alt+Shift+Ctrl+V): in a new group whose mask is the selection (the
  content stays whole behind the mask and moves under it), deselected. Without a document, a
  paste opens a new one of the size of what was copied; File > New offers that size as its
  Clipboard preset.
- Delete with a selection erases the selected pixels of the active layer (Photoshop's Clear);
  Edit > Fill (Shift+F5 or Shift+Backspace), Photoshop's dialog: the foreground or background
  color, Color… (the picker), black, 50% gray or white, and an opacity, in the selection or the
  whole layer without one. Edit > Stroke, Photoshop's dialog: a band along the selection's
  outline, inside, centered or outside, its width, color (the swatch opens the picker) and
  opacity. All of them are paint (ADR 0027): the original stays intact, but the paint is not
  editable afterwards (ADR 0029); an editable stroke will be a layer of its own (Layer menu).
- Edit > Transform: Rotate 180°, 90° clockwise and counter clockwise, Flip Horizontal and
  Vertical, about the center of the selected layers, exact (pixels are copied, not resampled).
- Edit > Transform > Again (Shift+Ctrl+T) repeats the last Free Transform or Edit > Transform on
  the selected layers; Alt+Shift+Ctrl+T duplicates them and transforms the copies (one undo
  entry), the copies selected so that it can go on. Free Transform with a selection (Ctrl+T,
  Edit > Free Transform), as in Photoshop: the selected pixels of the active layer float in a
  new layer above it, leaving a hole (paint), and are transformed; Esc takes it all back. A
  double-click on a layer still transforms the whole layer.
- Image > Adjustments, in Photoshop's order and with its shortcuts (Levels Ctrl+L, Curves
  Ctrl+M, Hue/Saturation Ctrl+U, Color Balance Ctrl+B, Black & White Alt+Shift+Ctrl+B, Invert
  Ctrl+I): a dialog with the same settings as the adjustment layer's Properties, previewed on the
  canvas while it is open (Preview turns it off); OK applies it to every pixel layer shown (the
  whole visible image, not only the selected layers: maintainer's choice), within the
  selection, as one undo entry, the canvas going from the preview straight to the result;
  Invert applies at once. What is applied is
  kept in the layer's stack (ADR 0029), listed below the layer and deletable; the same
  adjustment applied twice in a row is one entry (×2), two Inverts cancel.
- Image > Auto Tone (Shift+Ctrl+L), Auto Contrast (Alt+Shift+Ctrl+L), Auto Color
  (Shift+Ctrl+B), Photoshop's three classic algorithms, 0.1 % clipped at each end: Auto
  Contrast stretches the three channels alike (colors keep their relations), Auto Tone each
  channel on its own, Auto Color maps each channel from the average of the darkest to the
  average of the lightest pixels and makes the nearly gray midtones gray. The visible image is
  analyzed once (within the selection when there is one; maintainer's choice, 2026-10-03) and
  the Levels found are applied to every pixel layer shown, as Image > Adjustments does: an entry
  of each layer's stack, deletable, never recomputed. Nothing to change: nothing done.
- The Restore Eraser, in the Eraser's group (E, Shift+E to switch, the Eraser's options): it
  brings back a layer's original through all of its paint where it rubs, the adjustments
  applied from Image > Adjustments staying; paint fully rubbed out leaves the layer's list.
  Photoshop has no such tool (its History Brush is the closest); on a mask it is refused.
- Image > Image Size (Alt+Ctrl+I) and Canvas Size (Alt+Ctrl+C), Photoshop's dialogs: width and
  height in pixels or percent; Image Size keeps the proportions by default, Canvas Size has
  Relative and a 3×3 anchor. Image > Image Rotation: 180°, 90° both ways, Arbitrary… (Photoshop's
  dialog: an angle, clockwise or counter clockwise, remembered for the session; the canvas grows
  to hold the turned image, centered, the corners transparent; the view fits the new canvas),
  flip the canvas. All undoable, and nothing is cut: pixels outside the canvas are kept,
  resizing and arbitrary turns resample when shown (the layers keep their pixels, and Free
  Transform can still change the turn).
- Image menu in Photoshop's order: Adjustments; Auto Tone, Auto Contrast, Auto Color; Image
  Size, Canvas Size, Image Rotation, Crop, Trim…, Reveal All; then the document's Blend Space.
  Image > Trim…, Photoshop's dialog: Based
  On transparent pixels, the top-left or the bottom-right pixel's color (an exact match, as
  composited), Trim Away top, bottom, left, right; remembered for the session. Image > Reveal
  All grows the canvas to every layer's pixels, hidden layers included, within their masks
  (fills count only within a mask), in whole pixels; refused beyond 300,000 pixels a side. Both
  are crops: nothing is deleted, one undo entry, nothing done (and no undo entry) when there is
  nothing to trim or reveal, or when the whole canvas is margin.
- Adjustment layers (Layer > New Adjustment Layer, in Photoshop's order): Brightness/Contrast,
  Levels, Curves, Exposure, Vibrance, Hue/Saturation, Color Balance, Black & White, Photo
  Filter, Channel Mixer, Invert, Posterize, Threshold, placed
  above the active layer (in its folder) and selected. A Properties panel below Layers (the
  list never moves; the maintainer's choice) shows the selected adjustment's parameters: sliders applied live (one undo entry
  per drag), number fields, Reset. An adjustment icon in the layers list; the blend mode stays
  normal for now. Levels has Photoshop's Channel menu: RGB, Red, Green, Blue, each channel's
  own settings applied before the RGB ones (as Curves). Gradient Map (Layer > New Adjustment Layer and
  Image > Adjustments, after Threshold) has a simple gradient editor: the gradient, its color
  stops below it; a click under the gradient adds a stop (of the color the gradient has
  there), a drag moves one between its neighbours, a drag down away from it removes it; Color,
  Location and Delete edit the selected stop; Reverse. Photoshop's preset gradients,
  smoothness, midpoints and transparency are not there. Selective Color (after Gradient Map) has
  Photoshop's Colors menu (Reds… Blacks), Cyan, Magenta, Yellow and Black in %, and the Method,
  Relative or Absolute.
- Crop tool (C, Image > Crop): a frame on the whole image with eight handles, the outside
  shaded and the rule of thirds inside. Drag inside to move it, a handle to resize it (Shift on
  a corner keeps the proportions), outside to draw a new one; edges snap to the canvas, the
  layers and their sizes, with magenta guides (Ctrl: freely). A readout shows the size. Enter,
  a double-click inside or a click outside applies; Esc starts the frame over. As in Photoshop,
  the tool stays active: a new frame starts on the cropped canvas (and follows an undo); another
  tool drops the frame. Whole pixels only, and nothing
  is deleted: cropped pixels stay outside the canvas. With a selection, Image > Crop crops to its
  bounds (see Selections).

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
  Invert; Localized (each color selected only near where it was sampled, fading to nothing at
  its Range, a quarter of the image at first); Sample All Layers (unchecked: the active layer);
  the preview shows the selection it would make, live; Enter applies, Esc cancels; a current
  selection limits it, as in Photoshop. Its settings are kept for the next time.
  Moving the outline alone, as in Photoshop: with the marquees, the Lasso or the Magic Wand in
  New Selection mode, a drag from inside the selection (without Shift or Alt; the pointer is the
  arrow there) moves the outline, its pixels staying where they are (Shift during the drag: by
  steps of 45°); a click there stays the tool's click. With any selection tool, the arrows nudge
  the outline by 1 pixel, 10 with Shift. One undo entry each.
  Options bar: the four modes, Feather (px), Anti-alias for the ellipse and the lassos. Marching ants follow the
  selection at any zoom, where coverage crosses one half. Quick Mask (Q, Select > Quick Mask
  Mode) shows a soft edge: what the selection leaves out is tinted red, half opaque, fading
  where it is soft; the ants hide meanwhile (the maintainer preferred it to dotted limits around
  the ants). In Quick Mask the Brush paints the selection (white selects, black unselects, a
  gray partly, the Eraser unselects) with a pair of colors of its own, black and white at first
  (D), so that the drawing colors come back on leaving it; the options bar says "Quick Mask"
  whatever the tool, with Add and Remove (that pair, X swapping it) and the overlay's opacity
  (an app preference, half by default); the tab's title ends with "(Quick Mask)". Select menu: All (Ctrl+A), Deselect (Ctrl+D), Reselect (Shift+Ctrl+D),
  Inverse (Shift+Ctrl+I), Modify (Border, Smooth, Expand, Contract, Feather with Shift+F6: a
  dialog with one number of pixels, remembered for the session, the change shown live on the
  image while it is set, Cancel taking it back, OK one undo entry; the canvas edge is not an
  outline, as in Photoshop by default). Selecting is undoable; crop and size changes deselect.
- Layer menu (2026-10-03): New (Layer, Group, Layer via Copy, Layer via Cut), then New Fill
  Layer and New Adjustment Layer at the top level as in Photoshop (menus nest one level deep),
  Duplicate, Delete, Rename, Show/Hide, Layer Mask, the clipping mask, Delete Paint, Group and
  Ungroup, Arrange. Every command acts on the selected layers where that makes sense: Ungroup
  ungroups every selected group, Disable/Enable and Delete of Layer Mask reach every selected
  mask, and the clipping command reads Release only when every selected layer is clipped
  (what it then does). Rename stays on the active layer. Arrange, Photoshop's shortcuts: Bring
  to Front (Shift+Ctrl+]), Bring Forward (Ctrl+]), Send Backward (Ctrl+[), Send to Back
  (Shift+Ctrl+[), by the physical keys (^ and $ on AZERTY); the selected layers move within
  their own groups, a run of them as a whole, and the commands are grayed when nothing would
  move.
- Layer > New Fill Layer > Solid Color (also in the layers panel's right-click menu): a fill
  layer of the foreground color above the active layer (in its group), selected, without a
  dialog. Its color stays editable: the Properties panel shows it
  as a swatch, and a click on the swatch or a double-click on the layer's thumbnail opens the
  color picker (one undo entry). Named "Color Fill 1" as in Photoshop.
- Layer > Layer Mask: Reveal All, Hide All, Reveal Selection, Hide Selection on the selected
  layers without a mask (the selection follows the layer's transform; Reveal All and Reveal
  Selection also in the layers' right-click menu); a mask from the selection deselects, as in
  Photoshop. Image > Crop crops to the selection's bounds when there is one (otherwise it picks
  the Crop tool).

## Proposed

### Image menu (maintainer's decisions, 2026-10-03)

Decided, nothing to build:

- No Image > Mode: a document has no mode or depth (each layer keeps its own format, the
  composite is float), CMYK and Lab are refused at import. Image > Blend Space is the
  document's one color setting.
- Entries of a layer's stack stay not editable, deletable (ADR 0029).
- An adjustment applied with a selection keeps it as a soft mask: a feathered selection
  applies it partly (ADR 0029).

Open:

- Converting layers to a deeper format (an 8-bit layer is rounded after each entry of its
  stack, so stacked adjustments can band).
- Showing or editing the selection an applied adjustment keeps.

### Layers panel

- [ ] Alt+click on an eye shows only that layer (again: shows them all back).
- [ ] Drag over several eyes to show or hide them in one stroke.
- [ ] Alt+[ and Alt+] select the layer below and above (with Shift: add it to the selection).
- [ ] Drag a layer onto the "+" button duplicates it, onto the trash deletes it (Photoshop).
- [ ] Scrubby labels: dragging the "Opacity" label changes the value (Photoshop).
- [ ] Shift++ / Shift+- cycle through the blend modes of the selected layers.
- [ ] Layer locks: transparency, pixels, position, all (padlock icons).
- [ ] Color labels on layers, and a filter/search box above the list.
- [ ] A warning badge on each layer with import warnings (hover for the details), instead of
      only the document's notice.
- [ ] Alt+click on the mask thumbnail shows the mask alone on the canvas.
- [ ] Double-click the thumbnail opens the layer's properties (name, color label, blend options).

### Layer menu (audit of 2026-10-03)

Photoshop's Layer menu is a reference for what users expect, not a list to copy. The
maintainer's answers to the audit:

- **Decided, to build**:
  - Gradient and Pattern fill layers come with a gradient engine and patterns, not before (no
    dead entries).
  - Layer > Align (left, horizontal centers, right, top, vertical centers, bottom) and
    Distribute (horizontal and vertical centers, horizontal and vertical spacing), one
    implementation shared with buttons in the Move tool's options bar. As Photoshop: aligned
    to the selected layers' common bounds, to the canvas for one layer, to the selection's
    bounds when there is one.
  - New Layer from Visible (Photoshop's stamp visible, Alt+Shift+Ctrl+E): the visible
    composite as a new layer at the top of the document, the layers kept.
  - Merge Visible keeps Photoshop's Shift+Ctrl+E; Export moves to Alt+Shift+Ctrl+W
    (Photoshop's Export As).
  - Layer styles (drop shadow, glows, stroke, overlays) as in Photoshop: a list of editable
    effects per layer, computed after its own stack and mask (they read its final alpha and
    draw around it), sharing the stack's primitives (blur, fills, blend modes). Not entries of
    the stack (those are applied before the mask, inside the layer, and never edited). An ADR
    comes first.
  - Image > Adjustments keeps applying to every visible pixel layer; a layer's stack stays
    listed below it in the layers panel.
- **Decided, not added**: Apply Layer Mask (masks stay non-destructive), Smart Objects (a
  shared, linked or replaceable source is a later document-level question, with the DAG),
  linked layers (selection and groups cover their uses), vector masks (no vector content
  yet: then one mask concept with several representations), Group from Layers (it is Group
  Layers), Rasterize forced by a tool (no text or vector layer yet; to settle with the first
  of them).
- **Open question**: a submenu kept for what is destructive, as the maintainer suggests:
  Rasterize, Merge Layers (Ctrl+E), Merge Visible, Flatten Image. Its name, what Rasterize
  does on each kind of layer (a stack, a fill layer, a group; whether it bakes the transform),
  whether Flatten keeps transparency, and whether merging an adjustment layer down makes an
  entry of the layer's stack rather than pixels.

### Select menu (audit of 2026-10-04)

Photoshop's reflexes (Ctrl+A, Ctrl+D, Shift+Ctrl+D, Shift+Ctrl+I, Q, Select Subject, Color
Range, Select and Mask, Transform Selection) on a cleaner model: a selection is a continuous
coverage (ADR 0024), saved selections are named objects of the document, not alpha channels.
The menu names intentions, never a technology (no "AI" category). The maintainer's answers:

- **Done**: All (the whole canvas), Deselect, Reselect, Inverse; Select Subject (the existing
  BiRefNet + ViTMatte path); Color Range…; Modify > Border, Smooth, Expand, Contract, Feather;
  Grow and Similar (the Magic Wand from every selected pixel, its tolerance around the range of
  their colors, as Photoshop; connected pixels or the whole image); Transform Selection (Free
  Transform's box, handles, fields and right-click menu on the selection's bounds; the outline
  follows live, Enter resamples the selection as a layer is resampled, one undo entry; Esc or
  undo leaves it as it was; the layers never change); Quick Mask Mode (Q, with its own Add / Remove colors and overlay opacity); All
  Layers, Deselect Layers (layers, not pixels). The menu is grouped as: basics; Select
  Subject, Color Range, Select and Mask; Modify; Grow, Similar, Transform Selection, Quick
  Mask; saved selections; layers.
- **Decided, to build**:
  - French labels follow Photoshop FR where a Photoshop user would look: Grow is
    « Généraliser », Similar « Similaire », Border « Cadre… » (« Contour… » is Edit > Stroke).
  - Saved selections are named objects of the document, kept in `.slop`: Select > Save
    Selection… asks a name (an existing name offers to replace it), Select > Load Selection >
    lists them and replaces the selection. Renaming, deleting and combining (Photoshop's
    Ctrl/Shift/Alt+click on a thumbnail) come with a Selections panel. The `.slop`
    compatibility may break until version 1 (maintainer, 2026-10-04).
  - Select and Mask…: a light panel beside the image (as Color Range), not a workspace: edge
    detection radius (computed on request), Smooth, Feather, Contrast, Shift Edge applied live;
    views: ants, overlay, on black, on white, mask; output to the selection, a layer mask, or a
    new layer with a mask. A refine-edge brush is a later, separate step.
  - Right panels: Layers stays; the other panels form a closable accordion that folds down to
    their tab icons, a click on an icon unfolding that panel (maintainer, 2026-10-04; details
    left to the implementation).
  - The selection an Image > Adjustments effect keeps (ADR 0029) is an implicit mask dedicated
    to that effect: not exposed, not edited.
- **Decided, not added**: Sky, Person, Hair and other subject kinds (evaluated separately if
  ever useful); Find Layers, Similar Layers, Isolate Layers (the Layers panel's business).

### Tabs and documents

- [ ] Middle-click closes a tab.
- [ ] File > Open Recent (with thumbnails).
- [ ] Reopen the tabs of the last session at startup (optional).
- [ ] Drag a tab out of the window to open it in a window of its own.
- [ ] A dot on the tab of an unsaved document, and a clear prompt listing them on quit.

### Canvas and view

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
