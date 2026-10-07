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
  an export; Export (Alt+Shift+Ctrl+W, Photoshop's Export As: Shift+Ctrl+E is Merge Visible);
  Import as Layers (Shift+Ctrl+O).
- One instance: opening a file while the app runs focuses it; the window remembers its size and
  place.
- Drop files on the image or the layers panel: layers; elsewhere: new tabs. Folders and zips open
  like several files. Several images in one file (PDF pages, DICOM slices, animation frames) open
  as one document: one isolated group, the first image shown, the others hidden; the eye of a
  layer in a multi-selection shows or hides the whole selection.
- A layered file imported into a document arrives as one group named after it.
- Tabs: reorder by dragging, rename by double-click, drop a tab on another image to copy its
  layers, Ctrl+Tab; switching tabs back finds the Layers panel as it was left (selected
  layers, folded groups and stacks, mask targets, scroll).
- Layers panel: thumbnails, visibility, live opacity, blend modes, rename (double-click, F2),
  drag to reorder; several layers with Ctrl/Shift+click, Select > All Layers (Alt+Ctrl+A); a
  click in the empty area deselects; Delete deletes; groups as folders that fold, Ctrl+G,
  Shift+Ctrl+G, drag into folders, Pass Through; mask thumbnail, Shift+click disables the mask;
  Duplicate (Ctrl+J without a selection: Layer via Copy duplicates, as in Photoshop; with a
  selection, Layer via Copy puts the selected pixels in a new layer, Layer via Cut (Shift+Ctrl+J)
  too, leaving a hole as paint); right-click menu; drag layers onto another tab to copy them there, with a
  thumbnail following the pointer; clipping masks (Alt+Ctrl+G, Alt+click on the line between two
  layers; clipped layers indented with an arrow, the base underlined). What was applied to a
  layer's pixels (paint, Image > Adjustments; ADR 0029) is listed below it, as Photoshop
  lists smart filters: an arrow at the end of the row unfolds the list (folded at first),
  newest on top: Paint, or the adjustment's name. Each entry
  has an eye that hides it without deleting it ([ADR 0034](adr/0034-editable-operations.md)).
  An adjustment is edited again with the icon shown on hover next to the trash (or
  right-click > Edit Settings…; a double-click does nothing): its dialog, previewed on the canvas (Preview
  off hides the entry meanwhile), OK one undo entry, Cancel takes it back. What was painted above it
  follows. The trash, or right-click > Delete, deletes an entry; what was applied above it
  follows, and neighbours that become alike join.
- Tools: a toolbar on the left in Photoshop's order, one key each: Move (V, the tool at
  startup), the marquees (M), the lassos (L), the Magic Wand (W), Crop (C), the Eyedropper (I), the Healing Brush (J), the Clone Stamp (S), the Gradient and the Paint Bucket (G), Blur, Sharpen and Smudge (no key, as in Photoshop), Dodge and Burn (O); the active one is highlighted, its name and key in
  the tooltip. Variants share one slot, as in Photoshop: the slot shows the one used last, with
  a corner mark; a right-click or a long press lists them, Shift+key cycles them. An
  options bar under the menu bar shows the active tool's icon and its own settings only. No
  Hand or Zoom tool, and no view or apply buttons in the options bar (maintainer's choice,
  2026-10-01): Space+drag, the middle button, the wheel, Enter and Esc already do that with any
  tool. Next to the zoom slider in the status bar, two icon buttons: 100% and Fit on Screen.
- Move tool: drag moves the selected
  layers, arrows nudge by 1 pixel, Shift+arrows by 10; one undo entry per drag. The pointer stays
  the normal arrow over the image (no move cross). Auto-Select (options bar, on by default):
  the press takes the layer whose pixels are under the pointer (inside a group, the layer
  itself; not a fill layer without a mask, which covers the whole canvas) in the Layers panel's
  selection, by its rules: alone, or with Shift added to the selection or taken out of it (the
  layer clicked last active); a press on a layer of a multiple selection keeps them all moving,
  and a click without a drag selects it alone; Ctrl held inverts Auto-Select. Shift pressed
  during the drag holds one axis. Among several selected layers, the panel marks the active
  one with an accent along its row. A right-click lists the layers showing under the pointer
  at the top of the image's menu (fill and adjustment layers too, which a click does not take),
  the active one checked; choosing one selects it. A drag from where no layer shows draws a
  rectangle in the accent color (never marching ants: it selects layers, not pixels) selecting
  the layers whose visible pixels' box it touches, Shift adding them; a click there deselects
  the layers (not with Shift), as a click in the empty part of the panel. Snap:
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
  kept in the layer's stack (ADR 0029), listed below the layer, editable again, hidden by its
  eye and deletable; the same adjustment applied twice in a row is one entry when the settings
  combine exactly (Exposure's stops, a hue shift), two entries otherwise; two Inverts cancel.
- Filter menu ([ADR 0034](adr/0034-editable-operations.md)), between Select and View as in
  Photoshop: Repeat (Ctrl+F: the filter applied last, as it was, on the active layer, a new
  entry; "Repeat Gaussian Blur" once there is one) and Last Filter Settings… (Alt+Ctrl+F: its
  dialog at those settings), then Blur > Gaussian Blur…: Photoshop's dialog, a radius in pixels
  (0.1 to 1000, the field and a logarithmic slider), Blur > Motion Blur… (Angle -90 to 90°,
  Distance 1 to 2000 pixels); Noise > Add Noise… (Amount 0.1 to 400 %, Distribution Uniform or
  Gaussian, Monochromatic; each application draws another grain, an entry edited again keeps
  its own), Noise > Dust & Scratches… (Radius 1 to 500 whole pixels, Threshold 0 to 255
  levels), Noise > Median… (Radius 1 to 500 whole pixels), Blur > Box Blur… (Radius 1 to 2000
  whole pixels), Other > Maximum… and Minimum… (Radius 1 to 500 whole pixels); Pixelate >
  Mosaic… (Cell Size 2 to 200 pixels); Stylize > Emboss… (Angle, Height 1 to 10 pixels, Amount
  1 to 500 %), Stylize > Find Edges and Solarize (no settings: applied at once, without an
  ellipsis, as in Photoshop; their entries have nothing to edit again); Sharpen > Clarity and Texture… (Lightroom's two strengths, -100 to 100, nothing at
  first: Texture the fine details, Clarity the broad local contrast of the midtones; not in
  Photoshop's Filter menu, placed with the sharpening filters), Sharpen > Unsharp Mask… (Amount 1 to 500 %,
  Radius, Threshold 0 to 255 levels: linear sliders but the radius) and Other > High Pass…
  (Radius), Photoshop's ranges and defaults. A filter's settings show live on the canvas (Preview off
  shows the layer without it), OK one undo entry, Cancel takes it back. A filter applies to the
  active layer only, within the selection: grayed on a hidden layer, a layer that is not
  pixels, a targeted mask and in Quick Mask. It is an entry of the layer's stack, listed below
  it with a funnel, its eye, edited again with its icon (the same dialog); two in a row are one
  blur (3 then 4 px: 5 px). Painting above it paints over its result; the Restore Eraser reaches the paint
  above the topmost filter only.
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
  above the active layer (in its folder) and selected. A Properties panel in the dock below Layers (the
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
  layers and their sizes, with magenta guides (Ctrl: freely). A readout shows the size. The
  options bar chooses a ratio (Free, Original Ratio, 1 : 1, 4 : 5, 5 : 7, 2 : 3, 16 : 9, or typed
  in W and H, which a button swaps): the frame fits the largest of it, centered, and every
  handle keeps it; or a size in pixels: the frame takes it and only moves, a press outside it
  putting it there (no resampling: the size is the frame's). Remembered for the session.
  Straighten (its button there): a drag draws a line along what should be level, or upright
  when it is nearer to vertical; on release the image turns by that angle (Image Rotation's
  arbitrary turn, one undo entry: nothing is resampled into the layers) and the frame starts
  on the largest part of the canvas's proportions inside the turned image, to adjust and apply. Enter,
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
  selection limits it, as in Photoshop. Its settings are kept for the next time. Applying it
  takes seconds on a large image: its progress shows above the status bar, Esc or ✕ cancels.
  Moving the outline alone, as in Photoshop: with the marquees, the Lasso or the Magic Wand in
  New Selection mode, a drag from inside the selection (without Shift or Alt; the pointer is the
  arrow there) moves the outline, its pixels staying where they are (Shift during the drag: by
  steps of 45°); a click there stays the tool's click. With any selection tool, the arrows nudge
  the outline by 1 pixel, 10 with Shift. One undo entry each.
  Options bar: the four modes, Feather (px), Anti-alias for the ellipse and the lassos. Marching ants follow the
  selection at any zoom, where coverage crosses one half; where the engine presents to the window
  (Windows) the GPU draws them in the image, one device pixel wide just inside the selection,
  black and white in dashes of four (Photoshop-like), so they never lag behind a pan or a zoom;
  with frames over IPC (macOS, Linux) they are the page's SVG line, as before. Quick Mask (Q, Select > Quick Mask
  Mode) shows a soft edge: what the selection leaves out is tinted red, half opaque, fading
  where it is soft; the ants hide meanwhile (the maintainer preferred it to dotted limits around
  the ants). Quick Mask works as Photoshop's (maintainer, 2026-10-04): entering it turns the
  selection into a mask (everything when nothing is selected) and deselects; the painting
  tools then paint that mask (Brush, Eraser, Edit > Fill and Stroke, Delete: white selects,
  black leaves out, a gray partly; as in Photoshop, on any mask the Eraser, Delete and Cut
  paint the background color, white at first), and a selection made meanwhile with the selection tools
  limits them, as on any image; leaving it turns the mask back into the selection. Entering
  and leaving are undo entries. While a mask is painted (Quick Mask, or a layer's mask), the
  swatches are a pair of grays of their own (black and white at first, D, X swapping them) and
  the color picker offers grays only (its eyedropper takes a color's gray); the drawing colors
  come back afterwards. The toolbar's last button (a circle in a frame, Photoshop's) turns red
  while Quick Mask is on, a click entering or leaving it as Q does. The options bar says "Quick
  Mask" whatever the tool, with the overlay's
  opacity (an app preference, half by default); the tab's title ends with "(Quick Mask)". Select menu: All (Ctrl+A), Deselect (Ctrl+D), Reselect (Shift+Ctrl+D: the selection a change last removed or replaced, not only a
  deselected one, the maintainer's choice of 2026-10-04; twice, the two swap; undo and a
  gesture's steps do not count),
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
- Layer > Align (Left Edges, Horizontal Centers, Right Edges, Top Edges, Vertical Centers,
  Bottom Edges) and Distribute (Horizontal and Vertical Centers, Horizontal and Vertical
  Spacing: Photoshop's four edge distributions left out, rarely useful), also as buttons in the
  Move tool's options bar, as in Photoshop: one implementation in the engine. Layers count by
  the box of their visible pixels, a group as one; several align on the box around them all,
  a single one on the canvas, and with a selection on the selection's box. Distribute keeps the
  first and the last layer and needs three; one undo entry each.
- Layer > New Layer from Visible (Alt+Shift+Ctrl+E, Photoshop's stamp visible, which has no
  menu entry there): the visible layers composited over the whole canvas, the selection
  ignored, as a new pixel layer at the top of the document (the maintainer's choice, rather
  than above the active layer), selected; the layers stay. One undo entry. It shows at once,
  its pixels following, as a merge.
- Layer > Bake to Pixels ("Figer en pixels", [ADR 0031](adr/0031-bake-to-pixels.md)), what
  loses editability on purpose, in one place, one undo entry each. Rasterize: the selected
  layers keep their place, transform, opacity, mode, mask and clipping, their content becomes
  pixels (a stack its result, a fill its color over the canvas, a group its layers
  composited). Merge Layers (Ctrl+E; with one layer, Merge Down onto the visible layer below
  it, named after it): the layers composited as they show into one layer in the place of the
  topmost, hidden ones dropped, nothing cut outside the canvas; an adjustment layer merged
  down bakes into the pixels below. Merge Visible (Shift+Ctrl+E): every visible layer, hidden
  ones staying. Flatten Image: every layer into one named Background, hidden ones dropped,
  transparency kept. Grayed when there is nothing to bake. A merge shows at once (its layers
  drawn as one while the pixels are composited, listed as the new layer; the maintainer's
  idea), and the pixels replace them in the same undo entry.
- Layer styles ([ADR 0032](adr/0032-layer-styles.md)), as in Photoshop: Layer > Layer Style
  (Blending Options…, Stroke…, Color Overlay…, Drop Shadow…, Clear Layer Style) or a
  double-click on a layer's row (outside its name and thumbnail; not an adjustment layer's) opens the Layer
  Style dialog: the effects on the left with their checkboxes, the selected one's settings
  (each number with a slider, the color swatch opening the picker), the canvas following every
  change; OK keeps them as one undo entry, Cancel takes them back. A styled layer shows "fx";
  its effects unfold below it with the arrow (folded at first), each with an eye and a trash
  (shown on hover) that deletes it, a double-click opening its settings. Fill, under Opacity in
  the layers panel, fades the content but not its effects; it shows only when it means
  something there (the layer has an effect, or a Fill already set; Blending Options still sets
  it on any layer). Drop Shadow, Outer Glow, Inner Shadow, Inner Glow (from the
  edge), Color Overlay and Stroke, in Photoshop's order; inner effects say Choke where outer
  ones say Spread. A group's effects are drawn around what it holds.
- Layer > Layer Mask: Reveal All, Hide All, Reveal Selection, Hide Selection on the selected
  layers without a mask (the selection follows the layer's transform; Reveal All and Reveal
  Selection also in the layers' right-click menu); a mask from the selection deselects, as in
  Photoshop. Image > Crop crops to the selection's bounds when there is one (otherwise it picks
  the Crop tool).
- Every dialog moves by its title bar, its top edge kept in the window and a part of it always
  left to grab again; it opens again where it was left, for the session (Layer Style comes back
  in place after the color picker).
- The Eyedropper (I): a click on the image takes the color shown as the foreground color, Alt+click
  as the background (a gray while a mask is painted); its options, Photoshop's: Sample Size
  (Point Sample, 3 by 3 to 101 by 101 Average, transparent pixels not counting) and Sample (All
  Layers, as shown, or Current Layer, the active layer alone; the loupe shows what is sampled).
  Alt held with the Brush is the eyedropper until it is released (not during a stroke).
- The Clone Stamp (S): Alt+click sets where it takes its pixels (a cross marks it); a stroke
  then paints the pixels that far from it, with its own brush (size, hardness, opacity, flow,
  pressure; [ and ]), as they are when the stroke starts: from the active layer alone (Sample:
  Current Layer, Photoshop's default) or the image as shown (All Layers). Aligned (on by
  default) keeps the first stroke's distance for the next ones; off, each stroke starts from
  the source again. On a mask or in Quick Mask it paints the source's grays. One undo entry per
  stroke; what it took stays as it was ([ADR 0034](adr/0034-editable-operations.md), point 6).
  Without a source, a stroke says how to set one. Not yet: Current & Below, a source in another
  document, the source shown under the brush.
- The Healing Brush (J): as the Clone Stamp (the same Alt+click source, Aligned, Sample, its own
  brush, hard by default), but on release the stroke is blended into where it lands: it keeps
  the source's texture and takes the tone around the stroke (Photoshop's Healing Brush, a
  Poisson blend). The painting shows the source as it is meanwhile. One undo entry. Its Patch
  mode (options bar: Mode, Photoshop's Patch tool): draw around what to heal (a freehand
  outline, the Lasso's), then drag the selection onto where to take the texture: the selection
  is healed from there on release, the outline coming back; one undo entry. Not yet: a pattern
  as source, Patch's Destination mode.
- The Gradient (G, the Paint Bucket in its group): a drag draws the gradient's line (Shift: by
  steps of 45°, Esc drops it); on release the gradient is laid along it, Linear or Radial,
  Photoshop's first presets (Foreground to Background, Foreground to Transparent, Black, White;
  Reverse) at the options bar's Opacity, within the selection, as Edit > Fill paints: paint of
  the active layer (which grows to the canvas), grays on its mask or in Quick Mask; one undo
  entry. Not yet: editing a gradient's stops for the tool, the other shapes (Angle, Reflected,
  Diamond), dithering.
- Blur and Sharpen (their group has no key, as in Photoshop; the tooltip says so): strokes
  soften what the active layer shows (a Gaussian blur of 0.5 to 5 pixels by the options bar's
  Strength) or sharpen it (an unsharp mask), with their own soft brush. They take the layer as
  it shows when the stroke starts and keep the result; refused on a mask. One undo entry per
  stroke. Not yet: Sample All Layers, going over a place again within one stroke blurring more.
- Smudge (the third of the Blur group): strokes push the active layer's pixels along the stroke
  (Liquify's Forward Warp, shown live), by the group's Strength and brush; on release they are
  kept as the layer's paint, one undo entry; refused on a mask. Not yet: Finger Painting (the
  foreground color at the stroke's start), Sample All Layers, the selection limiting it.
- Dodge and Burn (O, Photoshop's Dodge and Burn tools): strokes lighten or darken what the
  active layer shows, most in the options bar's Range (Shadows, Midtones, Highlights), by its
  Exposure (50 % at first), with their own soft brush; Alt held at the press does the other
  for the stroke. They take the layer as it shows when the stroke starts and keep the result
  (ADR 0034, point 6); refused on a mask or in Quick Mask. One undo entry per stroke. Not yet:
  Protect Tones, Sponge (not added: Vibrance through a mask).
- The Paint Bucket (G): a click fills the pixels of a color similar to the clicked one (the
  Magic Wand's Tolerance, Anti-alias, Contiguous and Sample All Layers, its own settings) with
  the foreground color at the options bar's Opacity, within the selection when there is one, as
  Edit > Fill paints (paint of the active layer, a gray on its mask or in Quick Mask): one undo
  entry, the selection unchanged; its progress shows on a large image, Esc cancels it.
- The eyedropper, wherever the image is sampled (the Eyedropper, the color picker open, Select >
  Color Range on the image and on its preview): the pointer is a small cross open in its middle, with a + or a
  − for Color Range's adding and subtracting eyedroppers (Shift and Alt show theirs while held).
  Over the image a loupe, above right of the pointer, magnifies the 13 × 13 pixels around it,
  the sampled one framed in the middle, inside Photoshop's sampling ring: the new color over the
  current one (the color picker's; the new one all round for Color Range), in a neutral gray
  ring, the new color's value under it. It follows the pointer frame by frame, its pixels cut
  from a tile kept around the pointer.
- Help menu, an open-source project's only: Keyboard Shortcuts (the list of Edit > Keyboard
  Shortcuts), Report a Bug (GitHub's new issue form) and Contribute to SlopShop
  (`CONTRIBUTING.md`) in the browser, About SlopShop (version as built, license, a link to the
  project). Nothing of Photoshop's online services. Not yet: Documentation (no user
  documentation is published) and Check for Updates (no releases nor update mechanism).
- Sources panel ([ADR 0040](adr/0040-sources.md)), in the dock after Selections, like a video
  editor's media bin: each source the pixel layers show, its thumbnail (its own pixels,
  whatever the layers applied), its name (an opened file's, else its first layer's), its size
  and how many layers show it; the active layer's source marked by the accent. A click selects
  the layers showing it; the right-click menu has Select Layers and New Layer from Source (above
  the active layer). Layer > Make Source Unique (and the layers' right-click menu) gives the
  selected layers that share their source one of their own, grayed when none does. Not yet: a
  source dragged onto the canvas, renaming a source.

## Proposed

### Image menu (maintainer's decisions, 2026-10-03)

Decided, nothing to build:

- No Image > Mode: a document has no mode or depth (each layer keeps its own format, the
  composite is float), CMYK and Lab are refused at import. Image > Blend Space is the
  document's one color setting.
- ~~Entries of a layer's stack stay not editable, deletable (ADR 0029).~~ Revised on
  2026-10-04: entries are editable ([ADR 0034](adr/0034-editable-operations.md), Filter menu
  below).
- An adjustment applied with a selection keeps it as a soft mask: a feathered selection
  applies it partly (ADR 0029).

Open:

- Converting layers to a deeper format (an 8-bit layer is rounded after each entry of its
  stack, so stacked adjustments can band).
- ~~Showing or editing the selection an applied adjustment keeps.~~ Decided on 2026-10-04:
  loaded as the selection, replaced by the current one (Filter menu below).

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
    dead entries). ✅ Gradient fill layers (Layer > New Fill Layer > Gradient): the drawing
    colors from the bottom to the top of the document, as in Photoshop; the Properties panel
    edits the gradient (Gradient Map's editor), its style (linear, radial), angle and scale
    about its center, and reverses it. Not yet: opacity stops, Pattern fill layers.
  - Layer styles (drop shadow, glows, stroke, overlays) as in Photoshop
    ([ADR 0032](adr/0032-layer-styles.md)): an "fx" mark and the effects listed below the
    layer, each with an eye; Photoshop's Layer Style dialog (double-click on the layer, the fx
    button, Layer > Layer Style) with a live preview; Fill Opacity next to Opacity. Drop
    Shadow, Stroke and Color Overlay first, then the glows and Inner Shadow.
  - Image > Adjustments keeps applying to every visible pixel layer; a layer's stack stays
    listed below it in the layers panel.
- **Decided, not added**: Apply Layer Mask (masks stay non-destructive), Smart Objects (a
  shared, linked or replaceable source is a later document-level question, with the DAG),
  linked layers (selection and groups cover their uses), vector masks (no vector content
  yet: then one mask concept with several representations), Group from Layers (it is Group
  Layers), Rasterize forced by a tool (no text or vector layer yet; to settle with the first
  of them).
- **Decided** (2026-10-04, [ADR 0031](adr/0031-bake-to-pixels.md)): what is destructive
  goes in one submenu, Layer > Bake to Pixels; Rasterize keeps the transform; Flatten keeps
  transparency; merging an adjustment layer down bakes it into pixels.

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
  undo leaves it as it was; the layers never change); Quick Mask Mode (Q, the mask painted by every painting tool in grays, a selection limiting them); Save Selection… (a
  name, "Selection 1" at first; a name already used says it will be replaced) and Load
  Selection (the saved selections by name; a click makes one the selection): named objects of
  the document, kept in `.slop` and following crops, canvas and image size changes and
  rotations, each change one undo entry (the `.slop` compatibility may break until version 1,
  maintainer, 2026-10-04). The Selections panel lists them under a pinned Last Selection row
  (the maintainer's idea: Select > Reselect; greyed when there is none): a click loads one, Shift+click
  adds it, Alt+click subtracts it, Shift+Alt+click intersects (the selection tools' keys);
  double-click renames; the right-click menu also replaces one with the current selection or
  deletes it; + saves the current selection, the trash or Delete removes the row clicked last;
  a press outside the rows deselects the row, and on the empty part of the list deselects in
  the image too (as a press under the layers deselects them). The rows the selection is made
  of are marked (an accent along the row, and +, − or ∩ for how they were combined) until the
  selection changes otherwise; a line under the list recalls the keys (the maintainer's
  choice, 2026-10-04, rather than selecting several rows as in the Layers panel, where Shift
  means something else).
  The right column (ADR 0030, maintainer's choice of 2026-10-04): Layers on top, below it a
  dock of tab icons (Properties, Selections, History, Histogram, Info) whose unfolded panel's tab folds it down to the
  icons; its top edge resizes it; its tabs are reordered by dragging them; remembered;
  selecting an adjustment or fill layer unfolds Properties, and leaving it gives the dock back
  to the panel Properties replaced (unless another was chosen meanwhile). No panel closes or
  floats (ADR 0036). The Window menu lists the dock's panels, the unfolded one checked:
  choosing one unfolds it (the unfolded one folds the dock), as its tab does; then Options
  Bar and Toolbar (shown or not), Hide Panels (Tab) and Reset Layout (panels, sizes, order and
  bars as at first). The layout is one record on this machine; documents are not part of it.
  History lists the document's steps under its initial state, named as Photoshop names them
  (the tool, the command, the filter or adjustment applied); a click goes back or forward to
  a step, the steps undone stay dimmed until the next change. Histogram shows the visible
  image (within the selection) as Colors, Luminosity, Red, Green or Blue, with its mean,
  standard deviation, median and pixel count; Info the color and pixel under the pointer,
  the selection's size and place and the document's size.
  Select and Mask… (maintainer's choice of a light panel, 2026-10-04): a panel
  beside the image, not a workspace: View (Marching Ants, Overlay at Quick Mask's opacity, On
  Black, On White, Mask, drawn by the GPU over the image), Edge Detection (a radius and Detect:
  ViTMatte mattes the edge, on request since it takes seconds), Global Refinements shown live
  on the image (Smooth, Shift Edge in pixels, Feather, Contrast, applied in that order), Output
  To (Selection, Layer Mask on the active layer, New Layer with Layer Mask: a copy of the
  layer with the mask, the layer hidden); OK is one undo entry, Cancel puts the selection back;
  its settings are kept for the next time. Its Refine Edge Brush (Photoshop's): while it is on,
  strokes on the image mark hair or fur where edge detection decides too (Paint / Erase, Alt
  does the other; a size slider); the stroke's path shows while painting, and on release edge
  detection runs again on the selection the panel opened with, then the settings apply. All
  Layers, Deselect Layers (layers, not pixels). The menu is grouped as: basics; Select
  Subject, Color Range, Select and Mask; Modify; Grow, Similar, Transform Selection, Quick
  Mask; saved selections; layers.
- **Decided, to build**:
  - French labels follow Photoshop FR where a Photoshop user would look: Grow is
    « Généraliser », Similar « Similaire », Border « Cadre… » (« Contour… » is Edit > Stroke).
  - The selection an Image > Adjustments effect keeps (ADR 0029) is an implicit mask dedicated
    to that effect: not exposed, not painted (from 2026-10-04 it can be loaded as the selection
    and replaced by the current one, [ADR 0034](adr/0034-editable-operations.md)).
- **Decided, not added**: Sky, Person, Hair and other subject kinds (evaluated separately if
  ever useful); Find Layers, Similar Layers, Isolate Layers (the Layers panel's business).

### Filter menu and editable operations (audit of 2026-10-04)

Photoshop's filters without its Smart Objects: what is applied to a layer stays in its stack
with its parameters and is edited again ([ADR 0034](adr/0034-editable-operations.md)). The
maintainer's answers:

- **Decided, to build**:
  - Entries of a layer's stack are editable: an icon next to the trash on the entry's row
    reopens its dialog (previewed, OK one undo entry), and an eye hides it without deleting it.
    The same operation applied over an entry of its kind is one entry when their settings
    combine exactly (two blurs, Exposure's stops, hue shifts; revised 2026-10-04: no "1 of 2"
    choice), two entries otherwise; two Inverts that meet cancel and disappear.
  - Image > Adjustments keeps applying to every visible pixel layer, an entry on top of each
    stack, edited layer by layer (no link between layers). Filter > … applies to the active
    layer only; grayed when the active layer is not a pixel layer, when its mask is the target,
    and in Quick Mask (its features kept to the minimum).
  - The selection an operation is applied with stays its mask: Ctrl+click on the entry loads
    it as the selection, the entry's dialog replaces it by the current one. A new adjustment,
    fill or filter layer made with a selection gets a layer mask from it (Photoshop).
  - Editing an old entry changes the render of what was done above it: paint follows by
    construction. Better than Photoshop forcing a rasterize. The tools that took pixels from
    below (moved pixels; Clone Stamp, Healing, Remove, Smudge, Mixer when they come) keep what
    they took (revised 2026-10-05, rather than replaying their gesture): an edit below may give
    an unexpected render where they went, accepted.
  - Filter menu, no dead entries: Repeat Last Filter (Ctrl+F: a new entry, same settings, on
    the active layer; Alt+Ctrl+F reopens its dialog), Blur > Gaussian Blur first, then Blur,
    Sharpen, Noise, Distort, Pixelate, Stylize as each works.
  - Filter layers, after Gaussian Blur works in the stack: Layer > New Filter Layer at the top
    level beside New Adjustment Layer. Two concepts for the user (Adjustment Layer, Filter
    Layer), one in the engine. A filter layer changes what is below it as an adjustment layer
    does; its opacity is the effect's strength; the canvas edge is repeated.
  - Liquify (Fluidité, Shift+Ctrl+X, [ADR 0037](adr/0037-liquify.md)): a stack entry, edited
    again in its own workspace (the Layers panel's edit icon on the entry). A large modal window
    as Photoshop's: the layer previewed live, tools on the left (W Forward Warp, R Reconstruct,
    E Smooth, C Twirl Clockwise, S Pucker, B Bloat, O Push Left, F Freeze Mask, D Thaw Mask; Alt
    turns the twirl, bloats a pucker, thaws a freeze), the brush on the right (Size, Density,
    Pressure, Rate; [ and ] change the size), Restore All, OK and Cancel. No Hand and no Zoom
    tool: the wheel zooms about the pointer, the middle button (or Space) pans, as in the rest
    of the app. The frozen area is tinted red (Show Freeze Mask turns it off); Ctrl+Z and
    Shift+Ctrl+Z undo and redo strokes inside the workspace. It acts on the whole layer, the
    selection ignored (Photoshop's too); a field with nothing left displaced removes the entry.
    To validate: the feel of the tools' strengths (Pressure and Rate scale constants), the
    cursor outline, Alt on Push Left, Thaw All / Freeze All.
- **Decided, not added**: Convert for Smart Filters, Filter Gallery (the stack does it), Camera
  Raw Filter (its tools as native operations), Neural Filters and any category named after a
  technology, render generators as filters (fill layers instead), linking the entries one
  Image > Adjustments made on several layers, a single "Effect Layer" concept ("Effects" names
  layer styles), the transform as an entry of the stack (it stays a property of the layer).

### View menu (audit of 2026-10-04)

The maintainer asked for a View menu kept to what a modern editor needs, rather than
Photoshop's whole menu. View presents the document (zoom, aids, snapping, full screen); the
working interface (panels) belongs to Window.

- **Done**: Zoom In (Ctrl++), Zoom Out (Ctrl+-), Fit on Screen (Ctrl+0), 100% (Ctrl+1: one
  image pixel per screen pixel), then Hide Extras (Ctrl+H, Photoshop's Extras: the selection
  outline and the smart guides hidden for a moment, the selection and the snap kept; handles,
  the crop frame and pointers always shown; not remembered), Snap (remembered), then Full
  Screen (F11, the system's full screen: F11 rather than Photoshop's F, every application's
  key). Window > Hide Panels (Tab, as in Photoshop): the toolbar, the options bar and the
  panels hidden, in any mode (full screen and Tab is the canvas alone); choosing a panel in
  Window shows them again. Rulers (Ctrl+R, remembered) along the top and the left, labelled
  every 1, 2 or 5 × 10ⁿ units; a right-click on a ruler chooses its unit (Pixels, Inches,
  Centimeters, Millimeters, as Photoshop lists them; lengths at the document's resolution;
  remembered). Guides, cyan as in Photoshop: dragged out of
  a ruler (the top one gives a horizontal guide), moved with the Move tool, deleted by
  dragging them out of the image (onto a ruler), on whole pixels, snapping to the canvas and
  the layers' edges and centers (Ctrl: freely), Escape dropping the one dragged; stored in the
  document and moved with the image by Crop, Canvas Size, Image Size and Image Rotation; each
  drag one undo entry; View > Clear Guides. Snap is one switch and snaps to everything (the
  canvas, the layers, the guides); the magenta smart guides show what snapped, there is no
  separate Smart Guides switch; Ctrl held moves freely. From 800% a pixel grid shows by
  itself over the canvas, without a menu entry. Hide Extras also hides the guides and the
  pixel grid.
  Free Transform's upright box keeps its edges on whole pixels while it is moved or scaled
  (moves were already in whole pixels): no blur from a fraction of a pixel, and no "Pixels"
  snap target to choose; the options bar's fields stay exact.
- **Decided, not added**: the Show and Snap To submenus, a Smart Guides switch, New Guide…
  and Lock Guides (later if missed), a regular grid and New Guide Layout (guides cover it),
  a wheel scrolling preference (the wheel is for navigation: it zooms, the middle button pans;
  maintainer, 2026-10-05),
  Fill Screen (the wheel does it), screen modes cycled with F, rotating the view (the engine's
  view, every overlay and the snap would need it), soft proofing and the gamut warning (the
  engine is not ready: see [ADR 0007](adr/0007-color-management.md)).
- Ctrl+H is the application's Hide on macOS (Cmd+H): to check on a Mac.

### Toolbar (audit of 2026-10-05)

Photoshop's toolbar is a mental compatibility spec, not a template: its tools, keys, groups and
gestures where users look for them, without the tools that exist for historical reasons. One
function has one implementation, reachable from the familiar places. A tool still appears only
with the feature that makes it work ([ADR 0013](adr/0013-familiar-layout.md)). The maintainer's
answers:

- **Target order**, slots appearing as their tools are built: V Move; M Marquee (Rectangle,
  Ellipse); L Lasso (Freehand, Polygonal); W Selection (Object, Quick, Magic Wand); C Crop;
  I Eyedropper; J Retouch (Remove, Healing, Patch a mode of Healing); B Brush; S Clone Stamp;
  E Eraser (Eraser, Restore Eraser); G Gradient (Gradient, Paint Bucket); Blur (Blur, Sharpen,
  Smudge); O Dodge and Burn; P Pen; T Type; U Shape (Rectangle, Ellipse, Polygon, Line); then
  the colors (X, D) and Quick Mask.
- **Decided, to build**:
  - ✅ Layers chosen in the image with the Move tool, in the Layers panel's own selection (one
    state, not undoable, as in the panel): click, Shift+click, the active layer marked in the
    panel, Shift during the drag for one axis (see Done). What a click takes: the topmost layer
    whose pixels show there (from 5 % coverage, within its mask, a clipped layer where its
    base shows, its stack's result; inside a group, the layer itself); neither a fill layer
    without a mask, nor a layer style's pixels (a shadow), nor an adjustment layer: they are
    chosen in the panel or with the right-click.
  - ✅ Right-click with the Move tool: the layers under the pointer at the top of the image's
    menu, a click selecting one.
  - ✅ A drag from where no layer shows, with the Move tool, draws a rectangle (the accent
    color, never marching ants) selecting the layers whose visible pixels it touches; Shift
    adds them.
  - Layers and pixels stay two selections: layers by the Move tool, the panel and Select > All
    Layers; pixels by M, L, W and the Select menu. ✅ One explicit bridge, as in Photoshop:
    Ctrl+click on a layer's thumbnail loads its transparency as the selection (its pixels
    alone: opacity, blending, mask and effects left out; on the mask's thumbnail, the mask),
    Shift adding, Alt subtracting, both intersecting; one undo entry, the layers selected as
    they were. Select > Load Selection offers the same for the active layer (Layer
    Transparency, Layer Mask), above the saved selections, for those who look in the menu.
  - ✅ Quick Mask: a round red button at the bottom of the toolbar, as in Photoshop, lit while
    it is on; a click toggles it as Q does.
  - ✅ Eyedropper (I): a click takes the foreground color (Alt: the background), Sample (Current
    Layer or All Layers) and its size in the options bar, with the eyedropper's pointer and
    loupe; Alt held with the Brush takes the color, the Brush coming back on release.
  - ✅ Crop options: a ratio or a size, and Straighten (a line drawn along the horizon).
  - ✅ Edit > Transform gains Distort and Perspective, in Free Transform's box: a projective
    transform per layer ([ADR 0038](adr/0038-projective-transforms.md)). Ctrl and a corner
    places it freely, Alt+Shift+Ctrl and a corner moves its pair the other way (as in
    Photoshop); the menu entries, and the box's right-click menu, make that a corner's plain
    drag. Once a corner is free the box is a quad: sides move with their two corners, inside
    moves it, outside turns it; the options bar's numbers are grayed. Pixel layers only; a
    corner that would fold the box stays put. A layer in perspective is painted, filled,
    cloned and masked through its map (it does not grow to the canvas); not yet: moving or
    copying its selected pixels, Smudge. The Crop tool gets no perspective mode.
  - ✅ Gradient (G) paints into the layer (paint, [ADR 0027](adr/0027-painting.md)); ✅ gradient
    fill layers come with it, one gradient engine (the Gradient Map's editor). ✅ The Paint Bucket in
    its slot: the Magic Wand's region, filled.
  - Retouching by intention, never by technology: the Clone Stamp (S) first (✅), then Healing
    with its Patch mode (✅), then
    Remove (J), which works without a downloaded model. They keep the pixels they took
    ([ADR 0034](adr/0034-editable-operations.md), point 6 amended).
  - Pen (P): one tool designed to be simple, not Photoshop's family of anchor tools; with the
    vector ADR, as Type and Shapes.
  - The options bar holds how the next gesture acts; Properties what the selected object is,
    edited afterwards. A tool has variants only when the gesture differs: the Pencil is an
    option of the Brush (✅, and of the Eraser: whole pixels, no anti-aliasing, a diameter of 1
    painting one pixel), vertical text a property of text, a triangle a polygon of 3 sides.
- **Decided, not added**: Hand, Zoom and Rotate View tools (again), a Screen Mode button (F11
  and Tab do it), Single Row and Single Column Marquees, the Magnetic Lasso (Quick Selection
  follows edges), Content-Aware Move, Red Eye, Color Replacement (Hue/Saturation through a
  mask), Pattern Stamp, the History and Art History Brushes (the Restore Eraser, the stack's
  eyes), the Background and Magic Erasers (Magic Wand or Select Subject, then a mask), Sponge
  (Vibrance through a mask), the Type Mask tools, Note, Count, Slice, Artboard and Frame.
- **Later**: Mixer Brush, ✅ Blur and Sharpen, ✅ Smudge, ✅ Dodge and Burn, the Color Sampler, Custom
  Shape; ✅ Ctrl+Space+click zooming in (Alt: out) for a pen without a wheel (a drag sideways
  zooms about where it began, Photoshop's scrubby zoom; Cmd+Space on macOS).

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
- [ ] Alt+right-drag resizes the brush on the canvas ([ and ] already do, Shift for the
      hardness).
