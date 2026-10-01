# JPEG 2000 test files

Written by OpenJPEG 2.5.4 (`opj_compress`) from 64 × 48 patterns that the tests compute again
(`crates/slopshop-io/src/jpeg2000.rs`):

- `rgb8.jp2`: RGB 8-bit, lossless (JP2): `(x·4, y·5, (x + y)·3) mod 256`.
- `tiled.j2k`: the same pattern as a raw codestream, in 16 × 16 tiles, 3 resolution levels,
  RPCL progression (`-t 16,16 -n 3 -p RPCL`).
- `gray8.j2k`: gray 8-bit, lossless, raw codestream: `(x·3 + y·2) mod 256`.
- `rgba16.jp2`: RGBA 16-bit, lossless: `(x·1000, y·1300, x·y·37) mod 65536`, alpha
  `1000 + (x + y)·500`.
- `rgb12.jp2`: RGB 12-bit, lossless (from raw samples, `-F 64,48,3,12,u`):
  `((x·64 + y), y·85, x·y·3) mod 4096`.
- `lossy.jp2`: `rgb8`'s pattern, lossy (`-I -r 20`: the 9/7 wavelet, ratio 20).
