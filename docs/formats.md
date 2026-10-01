# Image formats: support and plan

What SlopShop reads and writes today, and the plan for every format Photoshop handles, in
priority order. It is meant for contributors: pick a format, read its row and the linked
research, and follow [Adding a format](#adding-a-format).

Background: the import strategy and license policy are in
[ADR 0006](adr/0006-universal-import-and-licensing.md), with a detailed survey of codecs,
licenses and pitfalls per format in [research/universal-import.md](research/universal-import.md)
(§2 is the reference for the "Approach" column below). Export is described in
[ADR 0008](adr/0008-export.md), [ADR 0010](adr/0010-jpeg-webp-export.md) and
[ADR 0011](adr/0011-gray-export.md).

**Legend.** ✅ supported · 🔎 recognized and refused with an explanation (the file is identified,
the user is told the format is not supported yet) · ⛔ blocked (license or patents, see notes) ·
— not planned.

**Priorities.**
- **P0**: the next format work. PSD/PSB, because Photoshop users need to bring their files.
- **P1**: widely used formats, or formats central to SlopShop's goals (very large, high bit
  depth, scientific and medical images).
- **P2**: useful, less common, or waiting for an engine feature.
- **P3**: legacy formats. Small and self-contained: good first contributions.

## PSD and PSB (P0)

PSD (and PSB, its large-document variant: over 30,000 px per side or 2 GB) is Photoshop's own
format and the key to migrating from Photoshop. The reader is written in-house from Adobe's
specification (existing crates narrow 16/32-bit samples or lack PSB; see the research notes),
with ag-psd and psd-tools as structure oracles. It comes in stages, each one useful on its own:

1. **Flattened import** ✅ (`slopshop_io::psd`): the merged composite image stored in the
   file, at its native depth (8/16/32-bit), in bitmap, gray, indexed, RGB or duotone (read as
   gray, with a warning), with its ICC profile and transparency (Photoshop's white matte
   removed); raw, RLE and zip compression; PSD and PSB. CMYK, Lab and multichannel are refused
   until the engine has them (ADR 0006). The composite is used when the layers cannot be read
   (with a warning), and for the CLI.
2. **Layers** ✅ (`slopshop_io::psd::layers`, gray, RGB and duotone documents): pixel layers
   with name, visibility, opacity (times the fill opacity), position, blend mode (ADR 0012; the
   document blends in perceptual space, as Photoshop's 8/16-bit documents do, and 32-bit ones in
   linear light) and layer mask (ADR 0014, the real pixel mask when there is also a vector
   mask); solid color fill layers as native fill layers; groups as groups (ADR 0015: pass-through
   or isolated, nested, with their opacity, blend mode and mask); clipping masks (ADR 0016). A
   layer smaller than the canvas
   shares one tile for its empty area, so it costs what its pixels cost. What the engine cannot
   represent yet is approximated and reported layer by layer: layer styles and
   advanced blending, mask density and feather, the adjustment layers not reproduced yet and
   gradient or pattern fills (left out), text, shapes, smart objects and vector masks (their pixels), pixels outside the
   canvas (cropped). A file saved without "Maximize Compatibility" opens from its layers.
3. **Groups and clipping** ✅ (ADR 0015, ADR 0016).
4. **Export** ✅ for PSD and PSB (`slopshop_io::export::export_psd`, 8/16-bit RGB, RLE, streamed
   through a temporary file so that memory does not grow with the document): pixel layers
   rendered one by one at their pixel size (transformed layers resampled, parts outside the
   canvas cut and reported), fill layers as pixels, groups, clipping, layer masks, blend modes
   and the adjustment layers the importer reads, as native ones; a merged composite stored over
   white as Photoshop does, and the ICC profile. Checked by re-importing and with psd-tools.
5. **Adjustment layers** ✅ in part (ADR 0020): Brightness/Contrast (from its descriptor, as
   current Photoshop versions write it), Levels, Curves, Exposure, Vibrance, Hue/Saturation,
   Color Balance, Black & White, Photo Filter, Channel Mixer, Invert, Posterize and Threshold
   become
   native adjustment layers with their mask (and are exported as such); Levels per channel,
   Hue/Saturation color ranges, legacy Brightness/Contrast, the Black & White tint, Photo
   Filter colors other than RGB or Lab (and version 3's XYZ) and blend modes other than normal
   are approximated and reported; Photo Filter colors are kept within sRGB; Colorize, drawn
   ("map") curves, curves of more than 16 points, Color Lookup, Selective Color and Gradient
   Map are left out for now.
   Then layer styles as native nodes, text as rasterized pixels plus its parameters, smart
   objects as embedded documents.

## PDF (P1)

PDF is how many images arrive (designers, print, scans). Photoshop opens PDFs by rasterizing
their pages. Import only: PDF export is not planned.

- **Import** (done, hayro, pure Rust): the Import PDF dialog shows the pages as thumbnails to
  pick and the resolution (300 pixels/inch by default, remembered) or the size in pixels of a
  page. Each picked page is rasterized as 8-bit sRGB, transparent where the page draws nothing
  (like Photoshop), and opens like a file of a multi-file open (tabs, or layers when dropped on
  the canvas). Without the dialog (command line, folders, zips, the CLI), the first page opens at
  300 dpi and the others are reported; the CLI has `--page` and `--dpi`. Content the renderer
  cannot draw (unsupported fonts, undecodable images) is reported. Fonts that are not embedded
  use the 14 standard fonts the renderer carries.
- **Not yet**: encrypted files; the page's own color space (the renderer converts CMYK,
  ICC-based and Lab content to sRGB: a CMYK engine would keep it); a 16-bit or gray mode;
  pages beyond 65,535 pixels a side; PDFium as an optional backend for difficult files;
  re-rendering the vector page at any zoom instead of fixed pixels. MuPDF, Poppler and
  Ghostscript are GPL/AGPL: excluded.
- **Photoshop PDF** saved with "Preserve Photoshop Editing Capabilities" embeds the PSD data:
  once the PSD reader exists, those files open with their layers.

## Photoshop's formats

The list follows Adobe's help page on the formats Photoshop supports.

| Format | Extensions | Import | Export | Priority | Approach | Notes |
|---|---|---|---|---|---|---|
| **Photoshop** | `.psd`, `.pdd` | ✅ layers | ✅ layers | **P0** | in-house reader (see above) | groups, clipping and thirteen kinds of adjustment layers kept; other adjustments and styles reported; CMYK, Lab refused |
| **Large Document Format** | `.psb` | ✅ layers | ✅ layers | **P0** | same reader and writer (64-bit lengths) | up to 300,000 px per side |
| JPEG | `.jpg`, `.jpeg`, `.jpe` | ✅ | ✅ | done | `image` (zune-jpeg) / `jpeg-encoder` | 12-bit and lossless JPEG not yet (libjpeg-turbo, optional) |
| PNG | `.png` | ✅ | ✅ | done | `png` | 8/16-bit, cICP, ICC; gray export |
| TIFF | `.tif`, `.tiff` | ✅ | ✅ | done | `tiff` | CMYK and Lab refused; BigTIFF export |
| OpenEXR | `.exr` | ✅ | ✅ | done | `exr` | deep data not supported; gray export pending (needs a luminance-only reader) |
| WebP | `.webp` | ✅ | ✅ | done | `image-webp`, libwebp (lossy export) | |
| GIF | `.gif` | ✅ (first frame) | — | P2 export | `gif` | animation import needs frame support in the model |
| BMP | `.bmp` | ✅ | ✅ | done | `image` / in-house writer | export: 8-bit sRGB (V5 header), alpha; embedded V5 ICC not read yet; `.dib` not recognized yet |
| Targa | `.tga` | ✅ | ✅ | done | `image` / in-house writer | export: 8-bit sRGB, alpha, RLE or uncompressed; Photoshop also uses `.vda`, `.icb`, `.vst`: not recognized yet |
| Portable Bit Map | `.pbm`, `.pgm`, `.ppm`, `.pnm`, `.pfm`, `.pam` | ✅ | ✅ | done | `image`; PFM and every export in-house | export: PGM/PPM/PAM 8/16-bit sRGB, PFM 32-bit float linear |
| Radiance | `.hdr` | ✅ | — | P2 export | `image` | RGBE, kept linear, never tone-mapped; XYZE variant not supported |
| JPEG XL | `.jxl` | ✅ | ✅ lossless | P1 lossy export | jxl-oxide (pure Rust); export: zune-jpegxl (pure Rust, lossless) with our own image header; lossy would need libjxl (C++, BSD) | import: first frame of animations (reported); native depth (8/16-bit, float); color from the code points or the ICC profile; CMYK refused. Export: 8/16-bit, alpha, gray, every space as JPEG XL's color encodings. The Rust bindings of libjxl are GPL: not usable |
| AVIF | `.avif` | ✅ | ✅ | done | rav1d and rav1e without assembly, our own container reader, avif-serialize ([ADR 0021](adr/0021-avif-import.md)) | import: 8-bit, or 10/12-bit as 16-bit; alpha; grids; `colr`, `irot`, `imir`, `clap`; the still image of animations (reported). Export: 8/10-bit, alpha, gray, spaces with H.273 code points (HDR included); lossy only (rav1e has no lossless mode) |
| Camera RAW (Camera Raw formats, DNG) | `.dng`, `.cr2`, `.cr3`, `.nef`, `.arw`, `.raf`, `.orf`, `.rw2`, `.pef`, … | 🔎 | — | P1 | rawler (LGPL-2.1, isolated behind a feature) | a non-destructive development node, not a baked import; demosaic algorithms written clean-room (the best known ones are GPL) |
| DICOM | `.dcm` | 🔎 | — | P1 | dicom-rs | an interpretation node (modality and VOI LUT) instead of baking the window; 12-bit JPEG needs libjpeg-turbo |
| JPEG 2000 | `.jp2`, `.jpf`, `.jpx`, `.j2k`, `.j2c`, `.jpc` | ✅ | — | P2 export | hayro-jpeg2000 (pure Rust) | JP2 and raw codestreams; native depth (8/16-bit, deeper as float); gray, RGB, alpha (straight or premultiplied); sRGB, sYCC, ROMM-RGB, ICC; CMYK and CIELab refused, e-sRGB read as sRGB (reported); single-threaded decoder. OpenJPEG declared itself unmaintained in 2026 |
| **Photoshop PDF, Generic PDF** | `.pdf`, `.pdp` | ✅ pages | — | P2 layers | hayro (pure Rust) | import only, pages rasterized as 8-bit sRGB, see [PDF](#pdf-p1); `.pdp` not recognized yet |
| HEIF / HEIC | `.heic`, `.heif` | ⛔ | — | P2 | OS decoders (Windows WIC, macOS ImageIO) or libheif (LGPL, isolated) | HEVC patent pools (ADR 0006); refused with an explanation |
| Cineon | `.cin` | — | — | P2 | in-house (simple header, 10-bit log) | needs a log transfer function in the color model; DPX (film scans) is the same family |
| Multi-Picture Format, JPEG Stereo | `.mpo`, `.jps` | — | — | P2 | JPEG decoder + MPF index | stereo pairs; the first image probably opens as a JPEG already (to verify); the model has one image per layer |
| Photoshop EPS, DCS 1.0 / 2.0 | `.eps`, `.dcs` | 🔎 | — | P3 | ⛔ for now: PostScript interpreters are AGPL (Ghostscript) or embed AGPL fonts (stet) | EPS files saved by Photoshop contain the raster data directly: a reader for that case only is possible |
| PCX | `.pcx` | — | — | P3 | in-house or image-extras | good first contribution |
| IFF (Amiga) | `.iff`, `.ilbm` | — | — | P3 | in-house | good first contribution |
| Pixar | `.pxr` | — | — | P3 | in-house | good first contribution |
| Scitex CT | `.sct` | — | — | P3 | in-house | CMYK or gray: gray first |
| Wireless Bitmap | `.wbmp` | — | — | P3 | in-house | 1-bit; good first contribution |
| PICT (Photoshop reads only) | `.pct`, `.pict` | — | — | P3 | in-house, raster opcodes only | vector opcodes out of scope |
| Photoshop Raw | `.raw` | — | — | P3 | in-house | headerless: size, channels and depth asked to the user |
| Photoshop Cloud Document | `.psdc` | — | — | — | | proprietary cloud format; use PSD |
| 3D formats | `.obj`, `.3ds`, `.dae`, … | — | — | — | | out of scope (Adobe retired Photoshop's 3D features) |

Photoshop also opens formats through optional plugins (for example DDS or ICO); SlopShop
already reads several of them (next section).

## Other formats

Formats that are not in Photoshop's list, read by SlopShop or planned.

| Format | Extensions | Import | Export | Priority | Approach | Notes |
|---|---|---|---|---|---|---|
| QOI | `.qoi` | ✅ | — | P3 export | `qoi` | |
| farbfeld | `.ff` | ✅ | — | P3 export | `image` | |
| DDS | `.dds` | ✅ | — | — | `image` | DXT1/3/5; BC4–7 and float via `dds` later |
| ICO | `.ico` | ✅ | — | P3 export | `image` | |
| FITS | `.fits`, `.fit` | 🔎 | — | P1 | fitsrs → CFITSIO (optional) | astronomy; stretch node, like DICOM's interpretation |
| Krita, GIMP, OpenRaster | `.kra`, `.xcf`, `.ora` | 🔎 | — | P2 | in-house readers | layered: same staging as PSD; ORA also as a layered export |
| SVG | `.svg`, `.svgz` | 🔎 | — | P2 | resvg | rasterized, re-rendered at any zoom later |
| Adobe Illustrator | `.ai` | 🔎 | — | P2 | as PDF | modern `.ai` files are PDF-compatible |
| OME-TIFF, whole-slide images, GeoTIFF | `.ome.tif`, `.svs`, … | — | — | P2 | `tiff` + own metadata parsing → OpenSlide / GDAL (optional) | pyramidal gigapixel images: a strength to aim for |
| JPEG XR | `.jxr`, `.wdp` | — | — | — | | rare; not investigated |

## Adding a format

1. **Check the license** of the codec against ADR 0006 (permissive by default; LGPL only
   isolated behind a feature; never GPL or AGPL). `cargo deny check` enforces it in CI.
2. **Decode natively**: the file's own sample type (8/16-bit, half or float) and channel layout
   (gray, gray + alpha, RGB, RGBA), never narrowed to 8-bit RGB. Look at how `slopshop-io`
   imports TIFF and PNG.
3. **Color is explicit**: read the ICC profile, cICP or equivalent tag, or declare the format's
   convention. Anything that cannot be represented faithfully yet is refused with an explicit
   error or imported with a warning (an id translated by the UI), never converted silently.
4. **Untrusted input**: bound every size before allocating, never panic on malformed data. A
   fuzz target is welcome.
5. **Tests**: small fixture files (or files written by the test), round trips when there is an
   export, and damaged files. Remove the format from the "recognized" list in
   `slopshop-io/src/lib.rs` when it becomes supported.
6. **Docs**: update this page, the README feature list, and `docs/cli.md` when the CLI changes.
