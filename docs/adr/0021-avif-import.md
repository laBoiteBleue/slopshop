# 0021 — AVIF import with rav1d, without assembly

Status: accepted (2026-10-01, maintainer's choice).

## Context

AVIF images are AV1 frames in a HEIF container (ISOBMFF boxes). The only AV1 decoder usable
from Rust without C is rav1d (BSD-2-Clause), the Rust port of dav1d. Its default features
build dav1d's hand-written assembly, which needs the nasm assembler on every machine that
builds SlopShop (developers and CI). The ready-made AVIF crate, avif-decode, enables that
assembly on x86 and reads the container with avif-parse, which is MPL-2.0 (not in the licenses
allowed by `deny.toml`). rav1d's API is dav1d's C one: calling it needs `unsafe` code.

## Decision

- rav1d with `default-features = false` (8 and 16-bit pipelines, no assembly): pure Rust, no
  build tool to install.
- The container is read by our own code (`slopshop_io::avif::container`): the primary item
  (an AV1 image or a grid of tiles), its alpha auxiliary image, `colr` (H.273 code points or
  ICC), `ispe`, `irot`, `imir`, `clap`, `prem`; every offset checked.
- rav1d is called from one module, `slopshop_io::avif::ffi`, the crate's second `unsafe` module
  after libwebp's: it decodes one item and copies the planes into memory we own, with a
  `SAFETY:` comment on each block. YUV to RGB is safe Rust (`avif::yuv`).

## Alternatives

- **avif-decode with nasm**: the least code and the fastest decoding, but nasm to install on
  every build machine and a license exception for avif-parse.
- **libavif or dav1d (C)**: native libraries to build and ship.
- **Postponing AVIF**: no other pure-Rust AV1 decoder exists.

## Consequences

- Decoding is slower than with assembly (a 4000 × 2000 photo opens in about 0.2 s in a release
  build on the maintainer's machine). The `asm` feature can become an option later, for
  builds that have nasm.
- The container reader is ours to maintain; grids are implemented, but only the single-image
  files written by libheif were available as test files.
- Image sequences (animated AVIF) give their still image, reported.
- Export follows the same rule: rav1e (BSD-2-Clause) without its assembly encodes the AV1
  frames, in parallel tiles, and avif-serialize (BSD-3-Clause) writes the container with the
  color description we choose (ravif, built on both, only writes sRGB). rav1e has no lossless
  mode: AVIF export is lossy only.
