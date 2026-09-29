# 0008 — Export

Status: accepted (2026-09-29)

## Context

Documents reach hundreds of megapixels in 8/16-bit, half and float sources, composited in
linear Rec.2020 unbounded f32 with premultiplied alpha (ADR 0005, 0007). Export must not assume
the image fits in RAM or VRAM, must not convert silently (bit depth, gamut, range, alpha) and
must tag its color. The viewport compositor outputs sRGB 8-bit over a checkerboard and may
coarsen the pyramid level to fit its tile budget: unusable as-is. The `image` crate's encoders
take whole buffers and its float→int conversion clamps silently (NaN → max); tiff 0.11.3's
streaming `write_strip` ignores the compression it declares (corrupt LZW/Deflate files); png
0.18.1 never writes cICP. The display read of float samples clamps every value to ±65504 (the
largest half float), finite values included. Research: encoder, pipeline and color-policy
reports (2026-09-29).

## Decision

1. **Export is a view.** It reads a document and never mutates it (no `Edit`). It is headless:
   `core` + `render` + `io`, callable from any front end (the CLI's `slopshop export` uses it).
   It is blocking and CPU-heavy: front ends run it off their UI thread.
2. **Band pipeline, bounded memory** (`io::export::export_image`). The image is processed in
   full-width bands of 256 rows (one tile row; the last band shorter), always at pyramid level
   0. A producer thread asks the pixel source for each band (premultiplied RGBA f32 in the
   working space) and hands it over a channel of capacity 1; the calling thread converts the
   previous band (rows split across threads) and feeds the format writer. Band buffers are
   recycled. At most 3 source bands are in memory: one being produced, one queued, one being
   converted, plus one converted band and the writers' own bounded queues (below). Peak memory
   ∝ width × 256, independent of the height.
3. **Pixel source.** `slopshop-io` depends on neither compositor: it receives the source as a
   closure. `render::export_source(Option<&Renderer>, &Document)` gives one:
   - the GPU renderer (`Renderer::render_region`): the same WGSL compositing code as the
     viewport (`export_main`), tile arrays owned by the call (the viewport's cache is neither
     used nor evicted), no display stage, chunked internally to the tile cache and to a memory
     budget (128 MiB of output per chunk, and as much for readback; buffers reused across the
     bands of one export);
   - the CPU reference compositor (`core::composite::composite_region`, rows in parallel with
     `std::thread::scope`, accumulation in f64): the test oracle, used when there is no
     renderer, and as a fallback: for one region when it shows more distinct raster images of one
     sample type than the GPU tile cache holds (`RenderError::TooManyLayers`), and for the rest
     of the export after a GPU out-of-memory or validation error (captured with wgpu error
     scopes as `RenderError::OutOfMemory` / `Gpu`, never a panic). Both give the same values
     within float rounding. Other failures (readback, device lost) are reported.
   - The source returns, with each region, the number of non-finite source values it replaced
     (`FnMut(Rect, &mut [f32]) -> Result<u64, String>`).
4. **Unbounded values, non-finite values mapped and counted by the source.** For export, finite
   values are never clamped (the ±65504 read is kept for display and averaging only, and its
   doc comment says it applies to finite values too). Both sources map NaN to 0 and ±inf to
   ±65504 (the largest half float), on stored samples and on decoded values, before any color
   matrix: ±`f32::MAX` would destroy the pixel's other channels through the matrices. Sums are
   kept finite between layers (a composite that overflows saturates to ±`f32::MAX`), so that an
   opaque layer still hides what is below it. Every replacement is counted (CPU:
   `CompositeReport`; GPU: an atomic counter in `export_main`, gray sources counted once) and
   returned to the export, which reports it as `nonFinite` (one infinity can count more than
   once, e.g. a stored sample and the composite it overflows). Very large *finite* values are
   kept on purpose and can still disturb a pixel's other channels through the matrices.
5. **One conversion stage, in core** (`core::convert::Converter`), per pixel: non-finite
   inputs (NaN → 0; ±inf kept for float, clipped for integer targets; counted) → matrix to the
   target primaries (f64) → un-premultiply for straight-alpha targets (alpha 0 gives color 0)
   → range policy (integers clip per channel and count high and low clips; floats keep every
   finite value, half floats count overflows) → transfer encoding and quantization at once
   through exact threshold tables → dither. A value outside the range by less than the
   working-space rounding noise (8 ε of the pixel's largest channel) takes the end code without
   being counted, so in-gamut content is never reported as clipped. Unedited sources re-export
   bit-exact on the CPU path for 8-bit samples in every named space and for 16-bit samples
   except pure power curves (Adobe RGB, ProPhoto) near black, where codes up to ~70 can be off
   by a few steps (tested); the GPU path rounds differently and is not bit-exact.
   Dither is 8-bit only: a 64 × 64 blue-noise table (in-house void-and-cluster, deterministic),
   ±0.49 step, indexed by absolute pixel position (independent of bands and tiles), the same
   offset for R, G and B; values exactly on a level never move. Alpha is linear, clamped to
   `[0, 1]`, never dithered.
6. **Explicit conversions, always reported.** Every lossy choice is an explicit setting of the
   `ExportSpec`; every lossy event is counted in an `ExportReport` of stable ids and numbers
   (except clipping within rounding noise, see 5),
   translated by front ends: `clippedHigh`, `clippedLow`, `nonFinite`, `halfOverflow` (counts
   in samples), `precisionReduced` (half-float samples) and `bigTiff`. Errors carry stable codes
   too (`io`, `source`, `cancelled`, `unsupportedSpace`, `tooLarge`, `invalidSpec`, `encode`)
   with a technical detail. Un-premultiplying rounding is inherent, not reported.
7. **Color is always tagged.** A space a format cannot tag is refused
   (`io::export::supports_space`, checked by `export_image` before the file is created); files are never
   written untagged.
   - PNG: the `sRGB` chunk for sRGB. Otherwise a `cICP` chunk when the space has H.273 code
     points our importer reads back (Rec.709/sRGB, Display P3 or Rec.2020 primaries with the
     Rec.709, gamma 2.2/2.8, linear, sRGB, PQ or HLG transfer; written by hand before the image
     data), plus an `iCCP` profile whenever ICC can describe the space (not PQ or HLG), for
     readers that ignore cICP; other spaces get `iCCP` only.
   - TIFF: the ICC profile tag (type `UNDEFINED`); PQ and HLG cannot be written.
   - EXR: the `chromaticities` attribute (primaries and white point); linear spaces only, since
     EXR samples are scene-linear. Our importer reads chromaticities back.
   - ICC profiles are synthesized in-house (`io::icc::write_matrix_trc`): v4.4 matrix/TRC
     display profiles, colorants adapted to D50 with Bradford (`chad`), exact `para` tone
     curves, a fixed creation date so that a space always gives the same bytes. They round-trip
     through our ICC reader, which snaps them back to the named spaces.
8. **Encoders are called directly**, never through `image`:
   - PNG: `png::StreamWriter` (1 MiB IDAT chunks), 8/16-bit RGB/RGBA, straight alpha, 16-bit
     big-endian. "Fast" is png's fdeflate mode, "Small" its balanced zlib level 6;
     single-threaded either way.
   - TIFF: our own strips, written through tiff's public low-level `DirectoryEncoder` (the
     streaming `write_strip` bug above; a regression test documents it, so the workaround can
     be dropped once upstream fixes it). Strips of at most 1 MiB and at most 32 rows (a power
     of two, so strips never straddle bands) are compressed in parallel (Deflate with flate2's
     zlib at level 2, LZW with weezl, as tiff does internally), with the horizontal predictor
     for integer samples, then written by a writer thread that owns the file (one compressed
     band queued). Every tag is written explicitly (`SampleFormat`, `ExtraSamples`, strip
     tables, resolution 1:1 without unit, ICC profile); the directory and every out-of-line
     value start at even (word) offsets. BigTIFF is chosen before anything is
     written, when a worst-case bound of the classic TIFF size (data + compression expansion +
     tags + profile) exceeds 4 GiB − 64 MiB, and is reported. Little-endian machines only.
   - EXR: `exr::block::write` driven block by block: one part, scan lines in increasing Y,
     lossless ZIP (16 lines per block), channels `A`, `B`, `G`, `R` (sorted, as EXR requires),
     f32 or f16, premultiplied. Rows are rearranged into planar blocks and sent over a bounded
     channel (one band of blocks) to an encoding thread, which compresses them on exr's thread
     pool (sequentially if the pool cannot be created) and writes them in order.
9. **Jobs.** A `core::CancelToken` is checked between bands; progress is reported after each
   band, in rows (`core::Progress`). The file is written to a temporary file of its own in the
   target directory (`.<name>.<pid>-<n>.slopshop-tmp`, created exclusively), synced, then
   renamed over the destination; on error or cancellation the TIFF and EXR writers stop their
   threads (closing the file, which Windows needs to delete it) and the temporary file is
   deleted, leaving the destination untouched. Concurrent exports to one destination never
   share a file: the last to finish replaces it, whole. In the app, closing the main window
   while exports run cancels them and waits up to 5 s for them to end before closing; a crash,
   a killed process, Ctrl+C in the CLI, or a job still running after 5 s can leave a temporary
   file behind.

### Defaults per format (`io::export::default_spec`)

| | PNG | TIFF | EXR |
|---|---|---|---|
| Samples | 8-bit if every visible raster is 8-bit, else 16-bit | the deepest source type: U8, U16, or F32 when any raster is float | F32 (F16 on request, reported) |
| Color space | 8-bit: sRGB; 16-bit: the source space if all visible rasters share one PNG can tag, else sRGB | the source space if unique and taggable, else Rec.2020 (integers) or linear Rec.2020 (float) | linear Rec.709 (sRGB primaries) |
| Tagging | sRGB chunk, or cICP (+ iCCP), or iCCP | ICC tag | chromaticities |
| Alpha | straight | integers: straight (`ExtraSamples` 2); float: premultiplied (1) | premultiplied |
| Compression | fast (fdeflate); "small" = zlib 6 | Deflate + horizontal predictor (integers), Deflate (float); LZW and none on request | ZIP, 16 lines (fixed) |
| Dither | on (8-bit only) | on (8-bit only) | never |

Alpha is kept unless the document is structurally opaque: its bottom visible layer (with an
opacity above 0) is an opaque fill, or a raster without alpha covering the canvas, at opacity
1. Without alpha the premultiplied color is written, i.e. the image over black.

### Out of scope

- JPEG and WebP (`image` 0.25: 8-bit only; JPEG ≤ 65535 px per side and drops alpha silently;
  WebP lossless only, whole buffer, ≤ 16384 px per side). Added by [ADR 0010](0010-jpeg-webp-export.md),
  which also replaces the "over black" of alpha-less targets with a matte.
- Gray targets (the converter accepts RGB and RGBA only), TIFF tiles, the TIFF floating-point
  predictor, 16-bit dither, non-linear EXR.
- Tone mapping, gamut compression and soft proofing: document operations, not export options.
- Re-embedding the source's ICC profile verbatim (rasters do not carry it).
- A report estimated before exporting: the report is exact and comes after the export.

## Alternatives

- **Whole-image export through `image`**: little code, but 3.7 GB of f32 at 233 MP, silent
  clamping, no alpha declaration in TIFF, no EXR chromaticities.
- **CPU-only compositor**: simpler, no VRAM contention, but every future node (transforms, blend
  modes, AI) would be written twice or export would diverge from the viewport. It is kept as
  the oracle and the fallback.
- **Conversion on the GPU**: 2–4× less readback for integer targets, but a second
  transfer/quantize implementation with GPU `pow` precision, and no exact counting.
- **TIFF through `write_strip`**: uncompressed only, until upstream fixes it. Always BigTIFF:
  simpler, but some older readers cannot open it.
- **EXR in linear Rec.2020 (no matrix) by default**: misread by the many tools that ignore
  chromaticities. Available on request.
- **ICC v2 with `curv` tables**: wider compatibility with very old readers, but tables are
  approximated on re-import (ADR 0007).
- **Untagged output, or everything converted to sRGB**: simpler, but ambiguous or lossy.

## Consequences

- New modules: `core::convert`, `core::composite`, `core::job` (`CancelToken`, `Progress`),
  `core::blue_noise`; `Renderer::render_region`, `render::export_source` and the `export_main`
  shader entry point; `io::export` (PNG, TIFF and EXR writers) and the ICC writer. The EXR
  importer reads chromaticities.
- Direct dependencies of `slopshop-io`: `exr` (block writer, chromaticities), `weezl` (TIFF
  LZW) and `flate2` (TIFF Deflate), all already in the tree through `image` and `tiff`, all
  permissive. `slopshop-core` stays dependency-free.
- PNG compression is single-threaded, so large PNG exports are CPU-bound; TIFF and EXR compress
  in parallel.
- EXR exports are much slower in dev builds: exr decompresses every block again to check it
  (a `debug_assert`).
- Every new pixel operation must exist in the export path (`export_main` and the CPU
  compositor) as well as in the viewport, and the GPU/CPU agreement tests must cover it.
