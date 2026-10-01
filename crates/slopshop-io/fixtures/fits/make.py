"""Writes the FITS test files (see README.md). Run from this folder: python make.py"""
import math
import struct

W, H = 64, 48


def card(key, value=None, text=None):
    if value is None:
        line = key.ljust(80) if text is None else (key.ljust(8) + text).ljust(80)
    else:
        if isinstance(value, bool):
            value = "T" if value else "F"
        elif isinstance(value, str):
            value = "'" + value.ljust(8) + "'"
        line = key.ljust(8) + "= " + str(value).rjust(20)
    return line.ljust(80).encode("ascii")


def header(cards):
    data = b"".join(cards) + card("END")
    return data + b" " * (-len(data) % 2880)


def data(values, fmt):
    raw = b"".join(struct.pack(">" + fmt, v) for v in values)
    return raw + b"\0" * (-len(raw) % 2880)


def image(bitpix, axes, primary=True, extra=()):
    first = card("SIMPLE", True) if primary else card("XTENSION", "IMAGE")
    cards = [first, card("BITPIX", bitpix), card("NAXIS", len(axes))]
    cards += [card(f"NAXIS{i + 1}", n) for i, n in enumerate(axes)]
    if not primary:
        cards += [card("PCOUNT", 0), card("GCOUNT", 1)]
    return header(cards + list(extra))


def rows():
    # FITS stores the bottom row first: row r of the file is y = H - 1 - r on screen.
    for r in range(H):
        for x in range(W):
            yield x, H - 1 - r


# Unsigned 16-bit through BZERO 32768, as cameras write it.
gray16 = [(x * 1000 + y * 13) % 65536 - 32768 for x, y in rows()]
open("gray16.fits", "wb").write(
    image(16, (W, H), extra=[card("BZERO", 32768), card("BSCALE", 1)]) + data(gray16, "h"))

# 32-bit float with a blank (NaN) pixel, values from -10 to 1000.
floats = [-10.0 + x * 10 + y * 2 for x, y in rows()]
floats[0] = float("nan")
open("float.fits", "wb").write(image(-32, (W, H)) + data(floats, "f"))

# RGB as three 8-bit planes (NAXIS3 = 3).
planes = [[(x * 4) % 256 for x, y in rows()], [(y * 5) % 256 for x, y in rows()],
          [((x + y) * 3) % 256 for x, y in rows()]]
open("rgb.fits", "wb").write(image(8, (W, H, 3)) + data(sum(planes, []), "B"))

# An empty primary HDU, then the image in an IMAGE extension (64-bit float).
doubles = [x / 63 for x, y in rows()]
open("extension.fits", "wb").write(
    image(8, (), extra=[card("EXTEND", True)]) + image(-64, (W, H), False)
    + data(doubles, "d"))

# A cube of five planes: the first is imported.
cube = [x for x, y in rows()] * 5
open("cube.fits", "wb").write(image(8, (W, H, 5)) + data(cube, "B"))
