# Camera RAW test file

`tiny.dng`, written by `make.py` (numpy): a 32 × 24 DNG, uncompressed 12-bit values in 16 bits,
an RGGB Bayer mosaic of two flat colors — `(0.5, 0.25, 0.125)` on the left half and
`(0.1, 0.4, 0.2)` on the right — with a ColorMatrix1 (D65) that maps XYZ to Rec.2020 and a
neutral as-shot white balance, so that developing it gives those colors back in linear Rec.2020.
Synthetic: no camera's data.
