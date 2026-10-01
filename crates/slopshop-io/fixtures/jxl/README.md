# JPEG XL test files

Written by ImageMagick 7.1.1 with libjxl 0.11.1 from 64 × 48 patterns that the tests compute
again (`crates/slopshop-io/src/jxl.rs`):

- `rgb8.jxl`: RGB 8-bit, lossless (`-quality 100`): `(x·4, y·5, (x + y)·3) mod 256`.
- `gray8.jxl`: gray 8-bit, lossless: `(x·3 + y·2) mod 256`.
- `rgba16.jxl`: RGBA 16-bit, lossless: `(x·1000, y·1300, x·y·37) mod 65536`, alpha
  `1000 + (x + y)·500`.
- `lossy.jxl`: `rgb8`'s pattern, lossy (`-quality 80`, XYB).
- `float.jxl`: RGB 32-bit float, lossless: `(x / 16, y / 12, (x + y) / 40 − 0.5)`.
- `anim.jxl`: two frames, `rgb8`'s pattern then its negative.
