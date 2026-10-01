# DICOM test files

Written by `make.py` (pydicom 3, numpy), 64 × 48 patterns that the tests compute again
(`crates/slopshop-io/src/dicom.rs`). Synthetic data only: no patient information.

- `ct.dcm`: MONOCHROME2, signed 12-bit samples in 16 bits, `clamp(x·60 + y·30 − 2048)`,
  rescale intercept −1024, window 40 / 400; explicit VR little endian.
- `ct-rle.dcm`: the same, RLE Lossless.
- `ct-j2k.dcm`: the same, JPEG 2000 Lossless, encoded by OpenJPEG 2.5.4
  (`opj_compress -F 64,48,1,12,s`).
- `mono1.dcm`: MONOCHROME1, 8-bit, `(x·3 + y·2) mod 256`, no window.
- `rgb.dcm`, `rgb-planar.dcm`: RGB 8-bit, `(x·4, y·5, (x + y)·3) mod 256`, interleaved then
  planar.
- `frames.dcm`: two frames of unsigned 16-bit (`x·1000 + y·10`, then 7), a LINEAR_EXACT window
  32000 / 64000.
- `jpegls.dcm`: declared JPEG-LS (its data is not valid): refused before decoding.
