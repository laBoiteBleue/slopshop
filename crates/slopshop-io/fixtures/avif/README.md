# AVIF test files

Written by ImageMagick 7.1.1 with libheif 1.19.7 from the 64 × 48 patterns of `../jxl`
(`crates/slopshop-io/src/avif/mod.rs` computes them again):

- `rgb8-lossy.avif`: `rgb8`'s pattern, 4:2:0, `-quality 80`.
- `rgb8-q100.avif`: the same, 4:4:4, `-quality 100` (still lossy).
- `rgba12.avif`: `rgba16`'s pattern at 12 bits, with an alpha plane, `-quality 90`.
- `rotated.avif`: `rgb8-lossy` with `-orient RightTop`: an `irot` of three quarter turns.

None has a `colr` property: the color comes from the AV1 sequence header (sRGB).
