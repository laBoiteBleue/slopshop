"""Writes the DICOM test files (see README.md) with pydicom and numpy.

Run from this folder: python make.py. `ct-j2k.dcm` also needs `ct.j2k`, the CT pattern encoded
by OpenJPEG: opj_compress -i ct.raw -o ct.j2k -F 64,48,1,12,s (ct.raw is written here first).
"""
import os

import numpy as np
from pydicom.dataset import Dataset, FileMetaDataset
from pydicom.encaps import encapsulate
from pydicom.uid import (ExplicitVRLittleEndian, JPEG2000Lossless, JPEGLSLossless,
                         RLELossless, generate_uid)

W, H = 64, 48
y, x = np.mgrid[0:H, 0:W]


def dataset(rows, cols, photometric, samples, bits_allocated, bits_stored, signed, syntax):
    meta = FileMetaDataset()
    meta.MediaStorageSOPClassUID = "1.2.840.10008.5.1.4.1.1.7"  # Secondary Capture
    meta.MediaStorageSOPInstanceUID = generate_uid(entropy_srcs=["slopshop", photometric])
    meta.TransferSyntaxUID = syntax
    ds = Dataset()
    ds.file_meta = meta
    ds.SOPClassUID = meta.MediaStorageSOPClassUID
    ds.SOPInstanceUID = meta.MediaStorageSOPInstanceUID
    ds.Modality = "OT"
    ds.Rows, ds.Columns = rows, cols
    ds.PhotometricInterpretation = photometric
    ds.SamplesPerPixel = samples
    ds.BitsAllocated, ds.BitsStored, ds.HighBit = bits_allocated, bits_stored, bits_stored - 1
    ds.PixelRepresentation = 1 if signed else 0
    if samples > 1:
        ds.PlanarConfiguration = 0
    return ds


def save(ds, name):
    ds.save_as(name, enforce_file_format=True)


# CT-like: signed 12-bit in 16, rescale intercept -1024, a soft-tissue window (40 / 400).
ct = np.clip(x * 60 + y * 30 - 2048, -2048, 2047).astype(np.int16)
ds = dataset(H, W, "MONOCHROME2", 1, 16, 12, True, ExplicitVRLittleEndian)
ds.RescaleSlope, ds.RescaleIntercept = 1, -1024
ds.WindowCenter, ds.WindowWidth = 40, 400
ds.PixelData = ct.tobytes()
save(ds, "ct.dcm")

# The same, RLE Lossless.
ds.compress(RLELossless, ct)
save(ds, "ct-rle.dcm")

# The same, JPEG 2000 Lossless, from OpenJPEG's codestream.
ct.astype(">i2").tofile("ct.raw")
if os.path.exists("ct.j2k"):
    ds = dataset(H, W, "MONOCHROME2", 1, 16, 12, True, JPEG2000Lossless)
    ds.RescaleSlope, ds.RescaleIntercept = 1, -1024
    ds.WindowCenter, ds.WindowWidth = 40, 400
    ds.PixelData = encapsulate([open("ct.j2k", "rb").read()])
    ds["PixelData"].VR = "OB"
    ds["PixelData"].is_undefined_length = True
    save(ds, "ct-j2k.dcm")

# MONOCHROME1 (the lowest value is white), 8-bit, no window.
mono1 = ((x * 3 + y * 2) % 256).astype(np.uint8)
ds = dataset(H, W, "MONOCHROME1", 1, 8, 8, False, ExplicitVRLittleEndian)
ds.PixelData = mono1.tobytes()
save(ds, "mono1.dcm")

# RGB 8-bit, interleaved, then planar.
rgb = np.stack([(x * 4) % 256, (y * 5) % 256, ((x + y) * 3) % 256], axis=-1).astype(np.uint8)
ds = dataset(H, W, "RGB", 3, 8, 8, False, ExplicitVRLittleEndian)
ds.PixelData = rgb.tobytes()
save(ds, "rgb.dcm")
ds.PlanarConfiguration = 1
ds.PixelData = rgb.transpose(2, 0, 1).tobytes()
save(ds, "rgb-planar.dcm")

# Two frames of unsigned 16-bit, a LINEAR_EXACT window.
frames = np.stack([(x * 1000 + y * 10).astype(np.uint16), np.full((H, W), 7, np.uint16)])
ds = dataset(H, W, "MONOCHROME2", 1, 16, 16, False, ExplicitVRLittleEndian)
ds.NumberOfFrames = 2
ds.WindowCenter, ds.WindowWidth = 32000, 64000
ds.VOILUTFunction = "LINEAR_EXACT"
ds.PixelData = frames.tobytes()
save(ds, "frames.dcm")

# JPEG-LS: not supported (the data is not even valid; it must be refused before decoding).
ds = dataset(H, W, "MONOCHROME2", 1, 8, 8, False, JPEGLSLossless)
ds.PixelData = encapsulate([b"\xff\xd8\xff\xd9"])
ds["PixelData"].VR = "OB"
ds["PixelData"].is_undefined_length = True
save(ds, "jpegls.dcm")
