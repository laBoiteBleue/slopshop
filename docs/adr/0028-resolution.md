# 0028 — Document resolution (pixels per inch)

Status: accepted (2026-10-03; the maintainer asked for resolution "as in Photoshop" in File >
New and Image > Image Size).

## Context

Photoshop gives every document a resolution: how many pixels make an inch on paper. It sets the
print size, the units of New and Image Size (cm, mm, inches), and it travels with image files
(JFIF density, PNG `pHYs`, TIFF `XResolution`, PSD's resolution resource). SlopShop had none:
the print dialog had to ask for one every time, and files lost theirs on the way through.

## Decision

1. **A document has a resolution in pixels per inch** (`Document::resolution`, a positive `f64`
   in `[1, 100000]`). It is metadata: no pixel depends on it, and the renderer ignores it.
2. **Changing it is an edit** (`Edit::SetResolution`), undoable like any other. Image > Image
   Size changes the pixels and the resolution in one undo entry; with Resample off, only the
   resolution changes and the pixels stay, as in Photoshop.
3. **Defaults**: 72 ppi when a file does not say (Photoshop's), 300 ppi for File > New's
   presets meant for print, 72 for the screen ones.
4. **`.slop`**: an optional `document.resolution` (schema 0.11), absent in older files, which
   read as 72. A compatible addition: an older SlopShop keeps the field as unknown data.
5. **Pixels per inch internally, both units in the UI**: Photoshop shows pixels/inch or
   pixels/cm; sizes in cm, mm or inches are always computed from pixels and resolution.
6. **Image files**: readers and writers carry the resolution where the format has one: JPEG
   (JFIF density, else EXIF `XResolution` on reading; JFIF on writing, in whole pixels per
   inch), PNG (`pHYs`, whole pixels per meter), TIFF (`XResolution` and `ResolutionUnit`),
   PSD (ResolutionInfo, resource 1005). It is read from the header apart from the pixels, and
   only for a new document: a file opened as a layer does not change the document's.

## Alternatives

- **No document resolution, a print resolution in the print dialog only**: simpler, but New
  and Image Size could not work in cm, and files would keep losing their resolution.
- **Separate horizontal and vertical resolutions** (files allow it): Photoshop keeps one; non
  square pixels are rare and can be imported as one resolution (the horizontal one).

## Consequences

- File > Print defaults to the document's size at its resolution, as Photoshop does when
  "Scale to Fit Media" is off.
- Document Info shows the resolution and the print size.
