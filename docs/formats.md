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

**Status (2026-10-01).** The maintainer closes the format work after SVG import, an export for
every format read (except camera RAW and SVG) and a basic camera RAW import: every common,
professional, scientific and medical format is then supported. What is still 🔎 or — below is
open to contributors, with its approach already researched.

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
   with name, visibility, opacity, position, blend mode (ADR 0012; the
   document blends in perceptual space, as Photoshop's 8/16-bit documents do, and 32-bit ones in
   linear light) and layer mask (ADR 0014, the real pixel mask when there is also a vector
   mask); solid color fill layers as native fill layers; groups as groups (ADR 0015: pass-through
   or isolated, nested, with their opacity, blend mode and mask); clipping masks (ADR 0016);
   layer styles (ADR 0032, `lfx2`/`lmfx` read with a descriptor parser) on pixel and fill
   layers and groups: Drop Shadow, Inner Shadow, Outer and Inner Glow, Color Overlay and Stroke
   (the first of each kind when several are stacked), with the document's global light and the
   style's scale, and Fill as the style's Fill Opacity. A layer smaller than the canvas
   shares one tile for its empty area, so it costs what its pixels cost. What the engine cannot
   represent yet is approximated and reported layer by layer: the effects SlopShop does not draw
   (Bevel & Emboss, Satin, Gradient and Pattern Overlay; glow gradients, contours, noise and
   techniques approximated), advanced blending, mask density and feather, the adjustment layers not reproduced yet and
   gradient or pattern fills (left out), text, shapes, smart objects and vector masks (their pixels), pixels outside the
   canvas (cropped). A file saved without "Maximize Compatibility" opens from its layers.
3. **Groups and clipping** ✅ (ADR 0015, ADR 0016).
4. **Export** ✅ for PSD and PSB (`slopshop_io::export::export_psd`, 8/16-bit RGB, RLE, streamed
   through a temporary file so that memory does not grow with the document): pixel layers
   rendered one by one at their pixel size (transformed layers resampled, parts outside the
   canvas cut and reported), fill layers as pixels, groups, clipping, layer masks, blend modes
   and the adjustment layers the importer reads, as native ones; layer styles as Photoshop's
   (`lfx2` and Fill); a merged composite stored over
   white as Photoshop does, and the ICC profile. Checked by re-importing and with psd-tools.
5. **Adjustment layers** ✅ in part (ADR 0020): Brightness/Contrast (from its descriptor, as
   current Photoshop versions write it), Levels, Curves, Exposure, Vibrance, Hue/Saturation,
   Color Balance, Black & White, Photo Filter, Channel Mixer, Invert, Posterize, Threshold,
   Gradient Map and Selective Color become
   native adjustment layers with their mask (and are exported as such); Hue/Saturation color
   ranges, Gradient Map's smoothness, midpoints and transparency, legacy Brightness/Contrast, the Black & White tint, Photo
   Filter colors other than RGB or Lab (and version 3's XYZ) and blend modes other than normal
   are approximated and reported; Photo Filter colors are kept within sRGB; Colorize, drawn
   ("map") curves, curves of more than 16 points and Color Lookup are left out for now.
   Then text as rasterized pixels plus its parameters, smart
   objects as embedded documents.

## PDF (P1)

PDF is how many images arrive (designers, print, scans). Photoshop opens PDFs by rasterizing
their pages. Export writes one page holding the image.

- **Import** (done, hayro, pure Rust): the Import PDF dialog shows the pages as thumbnails to
  pick and the resolution (300 pixels/inch by default, remembered) or the size in pixels of a
  page. The picked pages are rasterized as 8-bit sRGB and open as one document (the maintainer's
  layout, never a tab per page): an isolated group named after the file, the pages in it (the
  first on top) over a white background layer; dropped on a document, that group lands on top
  of it. Without the dialog (command line, folders, zips), the first page opens the same way at
  300 dpi and the others are reported; the CLI exports the page over white and has `--page` and
  `--dpi`. Content the renderer
  cannot draw (unsupported fonts, undecodable images) is reported. Fonts that are not embedded
  use the 14 standard fonts the renderer carries.
- **Export** (done, in-house writer, Flate through flate2): one page holding the image, 8-bit
  gray or RGB in an ICC-based color space (any space an ICC profile describes, as JPEG and
  TIFF), alpha as a soft mask. One point per pixel (the page opens at the image's size at 72
  dpi); beyond 200 inches (Acrobat's largest page) the page is scaled down, every pixel kept.
  The color samples stream to the file (the PNG Up predictor, a length object after the
  stream); the soft mask is compressed in memory meanwhile.
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
| **Photoshop** | `.psd`, `.pdd` | ✅ layers | ✅ layers | **P0** | in-house reader (see above) | groups, clipping, layer styles and fifteen kinds of adjustment layers kept; other adjustments and effects reported; CMYK, Lab refused |
| **Large Document Format** | `.psb` | ✅ layers | ✅ layers | **P0** | same reader and writer (64-bit lengths) | up to 300,000 px per side |
| JPEG | `.jpg`, `.jpeg`, `.jpe` | ✅ | ✅ | done | `image` (zune-jpeg) / `jpeg-encoder` | 12-bit and lossless JPEG not yet (libjpeg-turbo, optional) |
| PNG | `.png` | ✅ | ✅ | done | `png` | 8/16-bit, cICP, ICC; gray export; APNG frames as layers (see GIF) |
| TIFF | `.tif`, `.tiff` | ✅ | ✅ | done | `tiff` | CMYK and Lab refused; BigTIFF export |
| OpenEXR | `.exr` | ✅ | ✅ | done | `exr` | deep data not supported; gray export pending (needs a luminance-only reader) |
| WebP | `.webp` | ✅ | ✅ | done | `image-webp`, libwebp (lossy export) | animated: frames as layers (see GIF) |
| GIF | `.gif` | ✅ frames | ✅ one frame | — | `gif`; export palette: `color_quant` (NeuQuant) | an animation opens as one isolated group, one layer per frame (the first on top and the only one shown), each frame as the animation shows it; timing not kept; flattened opens give the first frame. Export: sRGB, at most 256 colors (exact when the image has no more, else a learned palette, without dithering yet), transparency on or off; changed pixels reported |
| BMP | `.bmp` | ✅ | ✅ | done | `image` / in-house writer | export: 8-bit sRGB (V5 header), alpha; embedded V5 ICC not read yet; `.dib` not recognized yet |
| Targa | `.tga` | ✅ | ✅ | done | `image` / in-house writer | export: 8-bit sRGB, alpha, RLE or uncompressed; Photoshop also uses `.vda`, `.icb`, `.vst`: not recognized yet |
| Portable Bit Map | `.pbm`, `.pgm`, `.ppm`, `.pnm`, `.pfm`, `.pam` | ✅ | ✅ | done | `image`; PFM and every export in-house | export: PGM/PPM/PAM 8/16-bit sRGB, PFM 32-bit float linear |
| Radiance | `.hdr` | ✅ | ✅ | done | `image` / in-house writer | RGBE, kept linear, never tone-mapped; XYZE variant not supported. Export: linear sRGB, no alpha, flat scanlines; negative values clipped (reported) |
| JPEG XL | `.jxl` | ✅ | ✅ lossless | P1 lossy export | jxl-oxide (pure Rust); export: zune-jpegxl (pure Rust, lossless) with our own image header; lossy would need libjxl (C++, BSD) | import: first frame of animations (reported); native depth (8/16-bit, float); color from the code points or the ICC profile; CMYK refused. Export: 8/16-bit, alpha, gray, every space as JPEG XL's color encodings. The Rust bindings of libjxl are GPL: not usable |
| AVIF | `.avif` | ✅ | ✅ | done | rav1d and rav1e without assembly, our own container reader, avif-serialize ([ADR 0021](adr/0021-avif-import.md)) | import: 8-bit, or 10/12-bit as 16-bit; alpha; grids; `colr`, `irot`, `imir`, `clap`; the still image of animations (reported). Export: 8/10-bit, alpha, gray, spaces with H.273 code points (HDR included); lossy only (rav1e has no lossless mode) |
| Camera RAW (Camera Raw formats, DNG) | `.dng`, `.cr2`, `.cr3`, `.nef`, `.arw`, `.raf`, `.orf`, `.rw2`, `.pef`, … | ✅ basic | — | — | rawler (LGPL-2.1) in the separate `slopshop-raw` helper ([ADR 0023](adr/0023-camera-raw-helper.md)) | developed "as shot": rawler's demosaicing (PPG, bilinear X-Trans), the camera's white balance (highlights clipped to white) and color matrix to linear Rec.2020, no tone curve, scene-linear float; four-color sensors refused; no export (as Photoshop); an adjustable development node is future work |
| DICOM | `.dcm`, `.dicom` | ✅ | ✅ Secondary Capture | — | dicom-rs (pure Rust: native, deflate, RLE, JPEG incl. lossless 12/16-bit); JPEG 2000 frames through hayro-jpeg2000 | every frame (slice) at its precision (signed samples offset), in one isolated group with the display window (after the modality rescale) as one Levels on top, acting on the slices only, nothing cut; several files opened together (a series) make one such document, ordered by Instance Number; no window: 8-bit as is, deeper stretched from min to max; MONOCHROME1 inverted; SIGMOID and VOI LUT tables approximated (reported); JPEG-LS (CharLS is C++), big endian and palette color refused; files without the `DICM` prefix or without an extension not recognized yet; flattened opens (CLI) give the first frame. Export: a Secondary Capture image (modality OT, new UIDs, empty patient attributes), MONOCHROME2 or RGB, 8/16-bit, uncompressed Explicit VR Little Endian, display values (sRGB), no alpha; 16-bit gray carries the window of its whole range; the rows stream into the pixel data (at most 65,535 px per side, 4 GiB) |
| JPEG 2000 | `.jp2`, `.jpf`, `.jpx`, `.j2k`, `.j2c`, `.jpc` | ✅ | ✅ JP2 | done | hayro-jpeg2000 (pure Rust); export: openjp2 (pure Rust port of OpenJPEG 2.5, BSD-2-Clause, safe API) | JP2 and raw codestreams; native depth (8/16-bit, deeper as float); gray, RGB, alpha (straight or premultiplied); sRGB, sYCC, ROMM-RGB, ICC; CMYK and CIELab refused, e-sRGB read as sRGB (reported); single-threaded decoder. OpenJPEG declared itself unmaintained in 2026. Export: JP2, 8/16-bit, gray or RGB, straight alpha, sRGB only (enumerated; no ICC profile yet), lossless (5/3) or lossy (9/7, a ratio from the quality), tiles of 1024 px; the image held as 32-bit samples (at most 65,535 px per side, 2^28 samples), encoded in a temporary file then copied |
| **Photoshop PDF, Generic PDF** | `.pdf`, `.pdp` | ✅ pages | ✅ one page | P2 layers | hayro (pure Rust); in-house writer | pages rasterized as 8-bit sRGB; export: one page holding the image, see [PDF](#pdf-p1); `.pdp` not recognized yet |
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
| QOI | `.qoi` | ✅ | ✅ | done | `qoi` / in-house writer | export: 8-bit, alpha, sRGB or linear sRGB (the header's flag) |
| farbfeld | `.ff` | ✅ | ✅ | done | `image` / in-house writer | export: 16-bit sRGB RGBA |
| DDS | `.dds` | ✅ | ✅ uncompressed | — | `image` (DXT1/3/5); in-house reader and writer for uncompressed files | import: DXT1/3/5, uncompressed 24/32-bit RGB with 8-bit channels, the top-level surface (other cube faces and volume slices reported); BC4–7 and float via `dds` later. Export: uncompressed BGRA (BGR without alpha), 8-bit sRGB, no mip levels; block compression not written |
| ICO | `.ico` | ✅ | ✅ | done | `image` / in-house writer (`png`) | export: one 8-bit sRGB PNG image (Windows Vista and later), alpha, at most 256 px per side |
| FITS | `.fits`, `.fit`, `.fts` | ✅ | ✅ | done | in-house reader and writer | astronomy; the first image (primary or IMAGE extension) at its precision, flipped upright, NAXIS3 = 3 as RGB; an automatic stretch ("STF auto") as a Levels layer; floats scaled to [0, 1] (reported); tile-compressed images not supported yet. Export: the primary HDU, gray or RGB planes, 8/16-bit (BZERO 32768) or 32-bit float, display values (sRGB), no alpha, streamed |
| Krita, GIMP, OpenRaster | `.kra`, `.xcf`, `.ora` | 🔎 | — | P2 | in-house readers | layered: same staging as PSD; ORA also as a layered export |
| SVG | `.svg`, `.svgz` | ✅ | — | — | resvg (pure Rust) | rasterized as 8-bit sRGB through the Import SVG dialog (resolution or size; 96 px/inch, the intrinsic size, by default), transparent where nothing is drawn; text with the system's fonts; no export (maintainer's choice); re-rendering at any zoom later |
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
