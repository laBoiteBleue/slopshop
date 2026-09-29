# 0006 — Universal import strategy and dependency licensing

Status: accepted (2026-09-29)

## Context

Import must eventually be universal (every image format: common raster, JPEG 2000, WebP, AVIF,
JPEG XL, EXR/HDR, TIFF, DICOM, FITS, camera RAW, PSD and other layered formats, SVG/PDF…), and
export later. Research: [universal-import.md](../research/universal-import.md). No single library
covers this on SlopShop's terms (native bit depth, explicit color, huge images, non-destructive
RAW/DICOM interpretation, permissive licensing, contained `unsafe`).

## Decision

1. **Hybrid strategy.** `slopshop-io` owns a decoder contract; each format is decoded by the best
   pure-Rust codec, called directly (not through a lossy interchange type). Layered formats get
   in-house readers. Long-tail formats come from **optional native backends** (C/C++), each in
   its own crate, preferably in a decode worker process.
2. **Decoder contract** (implemented progressively): native sample type and channel layout, an
   explicit color description (or "assumed" when the file has none), alpha mode, verbatim
   metadata blobs (ICC, EXIF, XMP), a multi-image tree (pages, frames, levels) and, later, a
   tile source with a declared access tier (random access, band streaming, full frame).
3. **No silent conversions.** Anything that cannot be represented faithfully yet (CMYK, Lab,
   unsupported ICC profiles, extra pages/frames, unsupported bit depths) is either refused with
   an explicit message or imported with a visible warning.
4. **Licensing policy.**
   - Default build: permissive licenses only (MIT, Apache-2.0, BSD, Zlib, ISC, Unicode, MPL-2.0
     as file-level copyleft is acceptable).
   - **LGPL is allowed only when isolated**: in a separate crate behind a feature, dynamically
     linked or in a worker process, and replaceable by the user.
   - **GPL and AGPL are forbidden** in every build (e.g. libjxl's Rust bindings, MuPDF,
     Poppler, Ghostscript, Grok, GPL demosaic packs).
   - Enforced by `cargo-deny` in CI (`deny.toml`).
5. **HEIC/HEIF is not supported for now** (HEVC patent pools): refused with an explanation.
6. **Engine first.** Formats beyond 8-bit RGB need the engine to store 16-bit and float pixels
   and to manage color ([ADR 0007](0007-color-management.md)); that comes before new formats.

## Phases

0. ADRs and license enforcement (this ADR, 0007, 0005 update).
1. Engine: U16/F16/F32 and gray/alpha storage, color-managed compositing and display; common
   formats (PNG 16-bit, JPEG, TIFF, WebP, GIF, BMP, TGA, ICO, PNM/PFM, QOI, farbfeld, EXR, HDR).
2. Modern codecs: JPEG XL, JPEG 2000, AVIF; fuzzing harness.
3. DICOM and FITS with interpretation nodes (modality/VOI LUT, stretches).
4. Camera RAW (rawler, LGPL-2.1, isolated) with a non-destructive development node.
5. Layered and vector formats (PSD/PSB, KRA, XCF, ORA, SVG, PDF).
6. Optional native backends (libjpeg-turbo 12-bit, OpenJPEG/OpenJPH, CharLS, LibRaw under CDDL,
   OpenSlide, GDAL, PDFium, OpenEXRCore) in a worker process.

## Alternatives

- **libvips as universal backend**: fast streaming, but bakes RAW/DICOM at open time, LGPL plus
  GPL components in its full build, and its broadest loaders are the "untrusted" ones.
- **OpenImageIO**: good license and tile API, but a heavy C++ build and a young Rust binding;
  kept as a candidate catch-all backend for phase 6.
- **ImageMagick**: too many security advisories; at most a sandboxed last resort.

## Consequences

- More integration work per format, in exchange for exact pixels, small default binaries and a
  clear license story.
- Every "unsupported" message should eventually name the optional component that would open the
  file.
