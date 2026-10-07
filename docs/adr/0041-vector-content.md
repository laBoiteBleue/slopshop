# 0041 — Vector content: shapes, paths, text and vector masks

Status: proposed (2026-10-07, drafted for the maintainer; the questions at the end are theirs).
Builds on [ADR 0040](0040-sources.md), point 8: vector content is a source kind without a
stack, painting goes on a pixel layer clipped above it, Rasterize gives pixels.

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

## Decision (proposed)

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

## Dependencies (to approve)

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

## Questions for the maintainer

1. Text stack: Parley (active, no vertical text yet) or cosmic-text (more complete editing,
   older dependencies)?
2. Coverage on the CPU only, uploaded as tiles (exact CPU/GPU match), as proposed?
3. Coverage depth: 8 bits, or 16 for 16- and 32-bit documents?
4. Text anti-aliasing: always smooth and unhinted, or Photoshop's None/Sharp/Crisp/Strong/
   Smooth?
5. Fonts missing on another computer: embed the fonts used in `.slop` (when their licence's
   `fsType` allows), keep the glyph outlines, or substitute and warn?
6. A bundled default font (Inter or Noto Sans, OFL), and how much size it may add?
7. Vertical text, text on a path, warped text: first version or later?
8. Color emoji: drawn, or outlines only?
9. Path operations (unite, subtract, intersect shapes): which crate, and in the first version?
10. Text edited on the canvas from the first version (a caret, a selection, IME), or in the
    Properties panel first?
11. PSD's editable text: later, keeping its pixels meanwhile, as proposed?
