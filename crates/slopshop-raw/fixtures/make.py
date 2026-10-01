"""Writes tiny.dng (see README.md). Run from this folder: python make.py (needs numpy)."""
import struct

import numpy as np

W, H = 32, 24
WHITE = 4095
LEFT, RIGHT = (0.5, 0.25, 0.125), (0.1, 0.4, 0.2)

# Rec.2020 (D65) to XYZ; the DNG's ColorMatrix1 is its inverse, XYZ to "camera" = Rec.2020.
REC2020_TO_XYZ = np.array([[0.636958, 0.144617, 0.168881],
                           [0.262700, 0.677998, 0.059302],
                           [0.000000, 0.028073, 1.060985]])
XYZ_TO_CAMERA = np.linalg.inv(REC2020_TO_XYZ)

# RGGB Bayer mosaic: each site keeps its color's channel.
raw = np.zeros((H, W), dtype="<u2")
for y in range(H):
    for x in range(W):
        color = LEFT if x < W // 2 else RIGHT
        channel = (0 if x % 2 == 0 else 1) if y % 2 == 0 else (1 if x % 2 == 0 else 2)
        raw[y, x] = round(color[channel] * WHITE)
data = raw.tobytes()

BYTE, ASCII, SHORT, LONG, RATIONAL, SRATIONAL = 1, 2, 3, 4, 5, 10
SIZES = {BYTE: 1, ASCII: 1, SHORT: 2, LONG: 4, RATIONAL: 8, SRATIONAL: 8}


def pack(kind, values):
    if kind == ASCII:
        return values.encode() + b"\0"
    if kind == BYTE:
        return bytes(values)
    if kind == SHORT:
        return struct.pack(f"<{len(values)}H", *values)
    if kind == LONG:
        return struct.pack(f"<{len(values)}I", *values)
    if kind == RATIONAL:
        return b"".join(struct.pack("<II", n, d) for n, d in values)
    return b"".join(struct.pack("<ii", n, d) for n, d in values)


def count(kind, values):
    return len(values) + 1 if kind == ASCII else len(values)


matrix = [(int(round(v * 10000)), 10000) for v in XYZ_TO_CAMERA.flatten()]
tags = {
    254: (LONG, [0]),                        # NewSubfileType: main image
    256: (LONG, [W]),
    257: (LONG, [H]),
    258: (SHORT, [16]),                      # BitsPerSample
    259: (SHORT, [1]),                       # no compression
    262: (SHORT, [32803]),                   # CFA
    271: (ASCII, "SlopShop"),                # Make
    272: (ASCII, "Test"),                    # Model
    273: (LONG, [0]),                        # StripOffsets (patched below)
    274: (SHORT, [1]),                       # Orientation
    277: (SHORT, [1]),                       # SamplesPerPixel
    278: (LONG, [H]),                        # RowsPerStrip
    279: (LONG, [len(data)]),                # StripByteCounts
    284: (SHORT, [1]),                       # PlanarConfiguration
    33421: (SHORT, [2, 2]),                  # CFARepeatPatternDim
    33422: (BYTE, [0, 1, 1, 2]),             # CFAPattern: RGGB
    50706: (BYTE, [1, 4, 0, 0]),             # DNGVersion
    50707: (BYTE, [1, 1, 0, 0]),             # DNGBackwardVersion
    50708: (ASCII, "SlopShop Test"),         # UniqueCameraModel
    50714: (LONG, [0]),                      # BlackLevel
    50717: (LONG, [WHITE]),                  # WhiteLevel
    50721: (SRATIONAL, matrix),              # ColorMatrix1
    50728: (RATIONAL, [(1, 1)] * 3),         # AsShotNeutral
    50778: (SHORT, [21]),                    # CalibrationIlluminant1: D65
}

ifd_offset = 8
entries = len(tags)
extra_offset = ifd_offset + 2 + 12 * entries + 4
extra = bytearray()
entries_bytes = bytearray()
values_at = {}
for tag in sorted(tags):
    kind, values = tags[tag]
    payload = pack(kind, values)
    n = count(kind, values)
    if len(payload) <= 4:
        field = payload.ljust(4, b"\0")
    else:
        if len(extra) % 2:
            extra += b"\0"
        values_at[tag] = extra_offset + len(extra)
        field = struct.pack("<I", values_at[tag])
        extra += payload
    entries_bytes += struct.pack("<HHI", tag, kind, n) + field
data_offset = extra_offset + len(extra)
# Patch StripOffsets (stored inline).
out = bytearray(b"II*\0" + struct.pack("<I", ifd_offset))
out += struct.pack("<H", entries) + entries_bytes + struct.pack("<I", 0) + extra
i = out.index(struct.pack("<HHI", 273, LONG, 1))
out[i + 8:i + 12] = struct.pack("<I", data_offset)
out += data
open("tiny.dng", "wb").write(out)
