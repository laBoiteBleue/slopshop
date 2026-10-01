# FITS test files

Written by `make.py`, 64 × 48 patterns that the tests compute again
(`crates/slopshop-io/src/fits.rs`). FITS stores the bottom row first: `y` below is the row on
screen, from the top.

- `gray16.fits`: BITPIX 16 with BZERO 32768 (unsigned 16-bit, as cameras write it):
  `(x·1000 + y·13) mod 65536`.
- `float.fits`: BITPIX −32, `−10 + x·10 + y·2`, the first stored pixel NaN (a blank).
- `rgb.fits`: BITPIX 8, NAXIS3 = 3 (red, green, blue planes): `(x·4, y·5, (x + y)·3) mod 256`.
- `extension.fits`: an empty primary HDU, then an IMAGE extension of 64-bit floats, `x / 63`.
- `cube.fits`: BITPIX 8, five planes of `x`: the first is imported.
