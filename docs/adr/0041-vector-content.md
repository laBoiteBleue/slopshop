# 0041 — Vector content: shapes, paths, text and vector masks

Status: accepted (2026-10-07, the maintainer's answers below; drafted for them the same day).

## Context

An editor without the Type tool, shapes and the Pen surprises users and contributors alike
(roadmap, 2026-10-05). Photoshop has three vector things: shape layers (live rectangles,
ellipses, polygons, lines, custom shapes, and paths drawn with the Pen), type layers, and
vector masks (a path masking any layer). They stay sharp at any scale and are edited again at
any time.

SlopShop draws pixels by tiles at the pyramid level the view needs, on the GPU, with a CPU
compositor as the reference the GPU is tested against (ADR 0005, 0022, 0035). Layers are
placed by their transform (ADR 0017, 0038). A vector source has no pixel grid: it must be drawn
at whatever scale and region is needed, the same on the CPU and the GPU.

A survey of the Rust libraries (2026-10-07) found:

- **Geometry**: `kurbo` (Linebender), already in the tree through the PDF importer.
- **Coverage**: `vello_common` / `vello_cpu` 0.3 (Linebender's sparse strips: analytic area
  coverage, `forbid(unsafe)`, also what Vello's GPU path uses on the CPU side), already in the
  tree at an older version through `hayro`; `tiny-skia` 0.12 (also in the tree, 4× supersampling,
  CPU only) as a fallback. Rejected: Vello's compute renderer (experimental, coverage computed
  on the GPU, so never equal to the CPU's), `lyon` (triangles and MSAA, same), `forma`
  (archived).
- **Text**: `parley` (layout: bidi, line breaking, spans, OpenType features, variations),
  `fontique` (system fonts through DirectWrite, CoreText, fontconfig; registered fonts),
  `harfrust` (shaping: a safe port of HarfBuzz, matching it), `skrifa` (outlines, variable
  fonts, `forbid(unsafe)`), all MIT or Apache-2.0, the last two already in the tree through the
  SVG importer. Alternative: `cosmic-text` (complete for editing, but older dependencies,
  bitmaps for screens, no system font API, no vertical text).

## Decision

1. **Two vector source kinds** (ADR 0040, point 1), immutable like every source:
   - **Shape**: paths (cubic Béziers, closed or open, a fill rule), a fill (a color or a
     gradient, as a gradient fill layer's) and a stroke (width, alignment, caps, joins, dashes,
     color). A live shape keeps its parameters (rectangle with corner radii, ellipse, polygon
     with its sides and star ratio, line with its arrowheads) and is drawn from them; the Pen's
     paths are paths.
   - **Text**: the string, its spans (font, size, color, tracking, leading, features), point or
     box (wrapping in a rectangle), alignment. Laid out into glyph outlines, then drawn as
     paths, so that text and shapes share one rasterizer.
   Editing one makes a new source and repoints the layers (ADR 0040, point 2): two layers
   duplicated from a title show the same text until one is made unique.
2. **Drawn at the scale they are seen.** A vector layer is not resampled: its paths go through
   the layer's transform (affine or projective, ADR 0038) into document space and are covered
   at the pyramid level and region needed, tile by tile. The coverage is computed on the CPU
   (`vello_common`), kept in a bounded cache keyed by (source, transform, level, tile) like the
   display cache (ADR 0022), and the compositor (CPU) and the shader (GPU) both multiply the
   same coverage by the fill. CPU and GPU agree exactly, by construction. Export draws at full
   resolution the same way.
3. **No stack** (ADR 0040, point 8). Layer styles (ADR 0032) apply, drawn from the coverage.
   Painting asks "Paint on a new layer above?" (a clipped pixel layer); filters come through
   filter layers (ADR 0034, point 8); Layer > Bake to Pixels > Rasterize gives an image source.
4. **Vector masks**: a layer may have a path mask besides its pixel mask (Photoshop's vector
   mask), covered like a shape (anti-aliased, optionally feathered) and multiplied with the
   pixel mask. Edited with the path tools.
5. **Tools**, in this order: the Shape tools (U: Rectangle, Ellipse, Polygon, Line; Custom Shape
   later), then the Pen (P) and Path/Direct Selection (A) with anchors and handles, then the
   Type tool (T). Their settings in the options bar and the Properties panel, as Photoshop.
6. **`.slop`**: sources of kind `shape` and `text` in `document.sources` (their description as
   JSON), and `slopshop.vector` nodes naming them; a vector mask as
   `params.vector_mask`. Fonts: see the questions.
7. **PSD**: shape layers and vector masks imported editable (`vmsk`/`vsms` paths, `vogk` live
   shapes, `vstk` strokes, `SoCo`/`GdFl` fills, all documented); type layers kept as the pixels
   the file holds until editable text is imported (Adobe's `EngineData` is undocumented).
8. **Order of work**: shapes in the engine (sources, coverage, cache, CPU and GPU, `.slop`),
   the Shape tools; then paths and the Pen, vector masks; then text; then PSD.

## Dependencies

Where they live (the maintainer's choice, 2026-10-07): `kurbo` and `vello_cpu` are
dependencies of `slopshop-core`, an exception to its rule of having none, so that the CPU
compositor, export and the CLI draw shapes as the display does. Curves are flattened finely
(a few hundredths of a pixel) before `vello_cpu` covers them: its own flattening loses about a
tenth of a pixel of coverage along curved edges.


| Crate | Licence | Why |
| ----- | ------- | --- |
| `kurbo` | MIT OR Apache-2.0 | Béziers, strokes' outlines, bounds (in the tree already) |
| `vello_common` 0.3 | MIT OR Apache-2.0 | analytic coverage; `hayro` moves to 0.8 so that one version is built |
| `parley`, `fontique` | MIT OR Apache-2.0 | text layout, system fonts (with the text step only) |
| `harfrust`, `skrifa` | MIT; MIT OR Apache-2.0 | shaping, outlines (in the tree already) |
| `icu_segmenter` | Unicode-3.0 | line breaking (through `parley`) |

## Alternatives

- **Vector content rasterized into an image source on each edit** (GIMP's text layers): simple, but blurred by every scale and transform.
- **GPU coverage** (Vello's renderer or `vello_gpu`): faster for huge paths, but the CPU
  reference and the GPU would differ, and `vello_gpu` does not do every mask and blend mode.
- **Tessellation and MSAA** (`lyon`): fast, never equal to the CPU.
- **`cosmic-text`** for text: complete for editing today, but behind on its dependencies,
  bitmap glyphs meant for screens, no vertical text.

## Consequences

- A new layer content (`Vector`) beside `Raster`, `Fill`, `Group` and `Adjustment`, in the
  compositor, the renderer and the display cache; a coverage cache.
- Hit testing and bounds come from the paths, not from pixels.
- Text adds the largest dependencies; it comes last, after shapes and paths prove the
  rasterizer and the cache.

## Answers (the maintainer, 2026-10-07)

1. Text: Parley.
2. Coverage computed on the CPU only and uploaded as tiles: yes.
3. 8-bit coverage (16 bits later if banding shows).
4. Text always smooth and unhinted.
5. Missing fonts: embedded in `.slop` when their licence's `fsType` allows it; otherwise the
   glyph outlines kept and a warning.
6. Inter bundled as the default font (OFL). Wanted besides: a list of fonts to choose from,
   and a study of downloading fonts on demand or embedding them (a proposal to come).
7. Vertical text, text on a path and warped text: later.
8. Color emoji: later (outlines first).
9. Path operations: later.
10. Text edited on the canvas from the first version.
11. PSD's editable text: later, its pixels kept meanwhile.

Order of work: shapes, then the Pen and paths (and vector masks), then text, then PSD.

## Amendment: the Pen (2026-10-13)

Status: accepted (2026-10-13, under the maintainer's delegation; to review).

The toolbar decision (2026-10-05) asked for one simplified, ergonomic Pen rather than
Photoshop's six tools. Its first step:

1. **One Pen (P)** builds one path: a click adds a corner anchor, a drag a smooth one with
   opposite handles (Alt during the drag moves the outgoing handle alone), Shift keeps the next
   anchor or the handle on 45° steps, a line follows the pointer to the next anchor. A click on
   the first anchor closes the path (a drag there shapes its handles); Enter finishes an open
   path, and so does changing tool; Backspace removes the last anchor; Esc drops the path.
2. **A finished path is a vector layer** (the shape tools' `AddShape`, one undo entry, named
   "Shape n" as Photoshop's), filled and stroked as the options bar says. There is no Path
   mode, work path or Paths panel yet: a path lives in a vector layer.
3. **An open path is filled as if its ends were joined** (as Photoshop fills one); its stroke
   follows the open outline and is centered whatever its alignment (an open outline has no
   inside). `Geometry::is_fillable` (everything but a line) now says what a fill covers;
   `is_closed` keeps saying whether a stroke can lie inside or outside.
4. **Make Selection (Ctrl+Enter)**: the area a path encloses (open subpaths joined, its fill
   rule kept), anti-aliased, replacing the selection, one undo entry "Make Selection". With a
   path being drawn, that path (it is then dropped: no layer); otherwise the active vector
   layer's geometry, placed by its transform and its groups'. Curves are flattened within a
   hundredth of a pixel and covered by the selection rasterizer (the lasso's, now taking
   several polygons and the even-odd rule).
5. Over the IPC, paths travel as `.slop` writes them (segments of 2 or 6 coordinates); the
   Properties panel shows a path layer's fill and stroke.

Later steps: editing anchors and handles after the fact (adding, deleting, converting and
moving them as Pen gestures, Photoshop's Ctrl and Alt), selecting a path (A), vector masks.

## Amendment: editing paths with the Pen (2026-10-13)

Status: accepted (2026-10-13, under the maintainer's delegation; to review).

1. **The Pen edits the active vector layer's path**, with Photoshop's Pen and Direct Selection
   gestures and no tool of its own: its anchors and handles show; Ctrl-drag moves an anchor
   (its handles with it), a handle (a smooth anchor stays smooth: the other handle turns,
   keeping its length) or a segment (the whole path); a click on a segment adds an anchor
   without changing the curve, on an anchor deletes it (two stay at least), on an open path's
   end continues the path from there; Alt+click makes an anchor a corner, Alt-drag pulls new
   smooth handles out of it, Alt-drag on a handle moves it alone. A press away from the path
   starts a new path (a new layer), as before.
2. **One gesture, one edit**: the release sends the whole new geometry as `SetShape`
   (`Edit::reshape`: linked copies follow, ADR 0040). The overlay previews the drag; the layer
   is redrawn on release.
3. **A live shape is turned into a regular path first**, after a confirmation as in
   Photoshop: `shape::path::as_path` gives a rectangle's corners (two anchors per rounded
   corner), an ellipse's four anchors (top, right, bottom, left, the usual cubic quarter
   arcs), a polygon's corners and a line's ends; the UI receives it as `LayerView.livePath`
   to show the anchors, and the conversion is an ordinary `SetShape` (one undo entry). The
   shape's Properties (corner radius, sides) no longer apply afterwards.
4. **Layers in perspective are refused** (a message): their handles would need the projective
   map, under which a Bézier is no longer a Bézier. Groups' and layers' affine transforms are
   composed to map the path to the document and back.

## Amendment: text, part 1 (2026-10-13)

Status: accepted (2026-10-13, under the maintainer's delegation; to review).

1. **Where layout lives.** A new crate, `slopshop-text` (depends on core only), lays text out
   with `parley` 0.12 and reads outlines with `skrifa` 0.44 (`harfrust`, `skrifa` and
   `read-fonts` at the versions already in the tree; no duplicate built). Core stays without
   fonts: a text source is a shape source that also holds its text, its shape being the glyph
   outlines (a non-zero path filled with the text's color). Core, the renderer, export, PSD
   (as pixels, like other vector layers) and the CLI draw it as a shape, so CPU and GPU agree
   by construction and styles, linked copies and Bake to Pixels work unchanged. Only editing
   lays text out again; the outlines are the layout cache (typing lays out the text once per
   keystroke, well under a millisecond for a title).
2. **`.slop` 0.33** keeps both, in the `shapes` table: `text` (the description) beside
   `shape` (the outlines), instead of a separate `text` kind (point 6). A reader without the
   fonts, or of an older schema, still draws the text exactly.
3. **The text model of part 1**: one style for the whole text (family, weight 1–1000, italic,
   size in pixels, color, tracking in thousandths of an em, leading in pixels or automatic at
   120 %), point text only, aligned left, center or right on its anchor (the first baseline at
   the alignment point). Spans (per-character styles) and box text come with part 2, as
   compatible additions.
4. **Sizes** are stored in pixels; the interface shows points through the document's
   resolution (ADR 0028), as Photoshop does. Changing the resolution without resampling keeps
   the text's pixels.
5. **Fonts, v1** (answer 6 asked for a proposal): the font picker lists the system's fonts
   (DirectWrite, CoreText, fontconfig loaded at run time through `fontique`) plus Inter, bundled
   (variable, upright and italic, 1.8 MB, OFL-1.1 with its license next to it) as the default
   family and the only font tests use. Styles are a family's weights (a variable font's named
   weights, Thin to Black) and italics. `.slop` keeps the family's name and the style, no font
   data. A missing family is drawn from the saved outlines; the options bar says it is
   missing, and editing the text sets it in the default family. Embedding fonts whose `fsType`
   allows it (answer 5) and downloading free fonts on demand (a curated list of OFL families
   from a pinned manifest, as the AI models) are left for later: they need a font subsetter
   and a download consent, and are proposed to the maintainer rather than decided here.
6. **The Type tool (T)**: a click places point text, or edits the text layer under it (picked
   by the box around its letters, as Photoshop does, also for the Move tool), the caret where
   it was clicked; a click in the text moves the caret. Keys go to a hidden text field so that
   dead keys and input methods work; the engine draws each keystroke live; Escape, Ctrl+Enter
   (Cmd+Enter), the keypad's Enter, a click outside the text or another tool commit; the
   session is one gesture, so one undo entry;
   a text left empty removes its layer. A type layer is named after its text, and renamed as
   it changes until the user renames it (Photoshop's behavior).
