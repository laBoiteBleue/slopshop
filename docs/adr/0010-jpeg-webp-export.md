# 0010 — JPEG and WebP export

Status: accepted (2026-09-29). Extends [ADR 0008](0008-export.md).

## Context

ADR 0008 left JPEG and WebP out: `image`'s encoders are 8-bit, whole-buffer, and its JPEG
encoder drops alpha silently. Both formats are 8-bit only, have no alpha (JPEG) or a lossy
alpha plane (WebP), and cap the image size (JPEG 65535 px per side in the file, 65500 in the
libjpeg-family decoders; WebP 16383 px). Export must stay bounded in memory, never convert
silently, and always tag color.

Measured on the reference machine (i9-13980HX, Windows 11), 24 Kodak images for quality
(SSIMULACRA2) and a 100.7 MP mosaic for speed, 256-row bands:
- `jpeg-encoder` 0.7.1 (pure Rust): 110–118 MP/s at 4:4:4 q90, 165–170 MP/s at 4:2:0, about
  36 MiB for the whole pipeline; same size as libjpeg-style baseline at equal quality.
- mozjpeg (C): no gain in its streamable mode; optimized Huffman saves 1.1–1.9 % but buffers
  the whole image (9–12 B/px). mozjpeg-rs corrupts its streamed output; jpegli does not build
  on MSVC without clang-cl.
- `image-webp` 0.2.4 lossless (pure Rust): about 150 MP/s, files 8–9 % larger than libwebp
  `-z0`, 4–5× faster.
- No pure-Rust lossy WebP encoder is usable: the best is 16–32 % larger and panics above about
  30 MP. libwebp 1.6.0 through `libwebp-sys` builds with `cc` only (no nasm, clang or cmake).
- Lossy WebP overflows VP8's 512 KiB first partition on detailed content before 16383²;
  `partition_limit` and fewer segments push the limit back but do not remove it.

## Decision

Maintainer decisions (2026-09-29): libwebp through our own FFI module (1), a per-crate IJG
exception (2), a white matte everywhere (4), the source space as default when common (5).

1. **Encoders.**
   - JPEG: `jpeg-encoder` (feature `simd`), interleaved baseline only, standard Annex K
     tables and Huffman codes, chroma averaged (`ChromaSubsamplingMethod::Average`; the
     default nearest costs quality and bytes). Its pull API is fed from a writer thread that
     owns the file, band by band, as the EXR writer does: memory stays ∝ width × 256.
     Progressive and optimized Huffman are not offered (small gain, whole-image memory).
   - WebP lossless: `image-webp`.
   - WebP lossy: libwebp through `libwebp-sys` (pinned, default features kept), called from a
     small `ffi` module in `slopshop-io`, the only `#[allow(unsafe_code)]` of the crate, every
     block documented with `// SAFETY:`. We write the RIFF container ourselves (VP8X, ICCP,
     ALPH, VP8). A second accepted exception to "no C code in slopshop-io", after zstd
     (ADR 0009), because nothing else produces usable lossy WebP.
2. **Licenses.** `jpeg-encoder` is `(MIT OR Apache-2.0) AND IJG`: `deny.toml` allows IJG for
   this crate only. libwebp's BSD-3-Clause and patent grant, and the IJG acknowledgement
   ("this software is based in part on the work of the Independent JPEG Group"), go into the
   third-party notices, since cargo-deny cannot see the terms of bundled C code.
3. **WebP buffers one frame.** A WebP file cannot be written incrementally, so the WebP writers
   keep the whole frame (RGB(A) for lossless, YUV 4:2:0 planes for lossy): an explicit
   exception to ADR 0008's bounded memory, capped by the 16383 px limit and reserved with
   `try_reserve_exact` (failure → `tooLarge`). The encoding happens in `finish`, which becomes
   cancellable and reports an "encoding" phase. Lossy encoding scales `partition_limit` with
   the image size, retries once with one segment, then fails with a new error code
   `contentTooComplex` (the UI suggests lossless WebP or JPEG).
4. **Matte.** A target without alpha (JPEG always, other formats when alpha is dropped) is
   flattened over a matte color, white by default, in the linear working space before the
   conversion: `c + (1 − a) × matte`. It replaces the implicit "over black" of ADR 0008. The
   matte is a setting of the `ExportSpec`, shown in the dialog and the CLI, and the flattened
   pixels (alpha below 1 beyond rounding noise) are counted in a new notice `alphaFlattened`.
   JPEG with alpha requested is an `invalidSpec` error. Flattening in linear light differs
   slightly from editors that flatten encoded values, at soft edges.
5. **Color.** JPEG and WebP accept every space an ICC profile can describe (not PQ or HLG),
   always tagged: APP2 for JPEG, ICCP for WebP, sRGB included. Default: the source space when
   all visible rasters share it and it is sRGB, Display P3 or Adobe RGB, else sRGB. Dither is
   on by default, as for every 8-bit target. No EXIF or XMP (none is carried, and an
   orientation tag must never be written).
6. **Reporting.** Quality, chroma subsampling and WebP's 4:2:0 are explicit settings, not
   notices; clipping, non-finite values and flattening are counted, as in ADR 0008.
7. **Size limits**, checked before the file is created: JPEG 65500 px per side, WebP 16383.

### Defaults

| | JPEG | WebP |
|---|---|---|
| Compression | quality 90, 4:4:4 | lossy, quality 90 (method 4, alpha quality 100, exact) |
| Other choices | quality 1–100; 4:4:4, 4:2:2, 4:2:0 | quality 0–100, or lossless |
| Alpha | never (flattened over the matte) | kept unless the document is structurally opaque |
| Matte | white | white (when alpha is dropped) |
| Space | source if sRGB, Display P3 or Adobe RGB, else sRGB | same |
| Dither | on | on |

## Alternatives

- **mozjpeg or libjpeg-turbo (C)**: same bytes in streaming mode, IJG terms as well, and a
  broken ICC helper in the Rust wrapper; their gains need whole-image modes.
- **An in-house JPEG encoder** written from T.81: no IJG, and a path to better quantization
  tables and parallel encoding, but 1–2k lines to write and test. Kept for later.
- **webpx** (safe wrapper over libwebp): no `unsafe` of ours, but a young crate with most
  releases yanked and a history of soundness fixes.
- **Lossless-only WebP**: pure Rust, but no lossy WebP at all.
- **Black matte outside JPEG**: keeps today's output, but makes the formats inconsistent.

## Consequences

- New dependencies of `slopshop-io`: `jpeg-encoder`, `image-webp` (already in the tree through
  `image`), `libwebp-sys` (C, built with `cc`).
- `ExportSpec` gains a matte, `ConversionReport` counts flattened pixels, the writer contract's
  `finish` becomes cancellable with an encoding phase, and `ExportError` gains
  `contentTooComplex`.
- The crate gains a small, audited `unsafe` module (libwebp FFI).
- Lossy WebP buffers up to about 1 GB at 16383² (more with alpha, whose plane is encoded by a
  side thread); lossless up to about 1 GB (RGBA). Cancelling lossless WebP during encoding
  waits for the encoder (a few seconds at most).
- libwebp compiled by GCC or Clang uses SSE2 only on x86 (Linux, macOS Intel): slower than
  on MSVC, not measured. `jpeg-encoder` has no NEON path. Both are untested on macOS and
  Linux outside CI.
