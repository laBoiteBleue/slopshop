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

1. **Flattened import** (can start now): the merged composite image stored in the file, at its
   native depth (8/16/32-bit), gray or RGB, with its ICC profile. Only reliable when the file
   was saved with "Maximize Compatibility" (the default): otherwise the composite is blank, and
   the user must be told. CMYK and Lab are refused with an explanation until the engine has
   them (ADR 0006).
2. **Layers** (can start now): pixel layers with name, opacity, visibility, position, blend
   mode (Photoshop's modes are in the engine, ADR 0012; the document blends in perceptual space,
   as Photoshop's 8/16-bit documents do) and layer mask (ADR 0014). What the engine cannot
   represent yet (Dissolve, groups, clipping, adjustment layers, layer styles, text, smart
   objects, layers offset from the origin) is reported as a warning, and the composite of step
   1 stays available.
3. **Groups and clipping**: when the engine has them (roadmap Phase 2).
4. **Export** (PSD, and PSB above the PSD limits), layered, with a merged composite for other
   readers.
5. **Adjustment layers and layer styles** as native nodes (they are parameters in the file, not
   pixels), text as rasterized pixels plus its parameters, smart objects as embedded
   documents.

## PDF (P1)

PDF is how many images arrive (designers, print, scans). Photoshop opens PDFs by rasterizing
their pages. Import only: PDF export is not planned.

- **Import**: each page rasterized at a resolution chosen when opening (size or DPI), in its
  color space when the page declares one (ICC-based); CMYK pages are refused or reported until
  the engine has CMYK. Several pages open like a multi-file open (tabs, or layers when dropped
  on the canvas). Pure-Rust hayro by default, PDFium as an optional backend for difficult files.
  Later: re-rendering the vector page at any zoom instead of fixed pixels. MuPDF, Poppler and
  Ghostscript are GPL/AGPL: excluded.
- **Photoshop PDF** saved with "Preserve Photoshop Editing Capabilities" embeds the PSD data:
  once the PSD reader exists, those files open with their layers.

## Photoshop's formats

The list follows Adobe's help page on the formats Photoshop supports.

| Format | Extensions | Import | Export | Priority | Approach | Notes |
|---|---|---|---|---|---|---|
| **Photoshop** | `.psd`, `.pdd` | 🔎 | — | **P0** | in-house reader (see above) | |
| **Large Document Format** | `.psb` | 🔎 | — | **P0** | same reader (64-bit lengths) | |
| JPEG | `.jpg`, `.jpeg`, `.jpe` | ✅ | ✅ | done | `image` (zune-jpeg) / `jpeg-encoder` | 12-bit and lossless JPEG not yet (libjpeg-turbo, optional) |
| PNG | `.png` | ✅ | ✅ | done | `png` | 8/16-bit, cICP, ICC; gray export |
| TIFF | `.tif`, `.tiff` | ✅ | ✅ | done | `tiff` | CMYK and Lab refused; BigTIFF export |
| OpenEXR | `.exr` | ✅ | ✅ | done | `exr` | deep data not supported; gray export pending (needs a luminance-only reader) |
| WebP | `.webp` | ✅ | ✅ | done | `image-webp`, libwebp (lossy export) | |
| GIF | `.gif` | ✅ (first frame) | — | P2 export | `gif` | animation import needs frame support in the model |
| BMP | `.bmp` | ✅ | — | P2 export | `image` | embedded V5 ICC not read yet; `.dib` not recognized yet |
| Targa | `.tga` | ✅ | — | P2 export | `image` | Photoshop also uses `.vda`, `.icb`, `.vst`: not recognized yet |
| Portable Bit Map | `.pbm`, `.pgm`, `.ppm`, `.pnm`, `.pfm`, `.pam` | ✅ | — | P2 export | `image`, zune-ppm (PFM) | |
| Radiance | `.hdr` | ✅ | — | P2 export | `image` | RGBE, kept linear, never tone-mapped; XYZE variant not supported |
| JPEG XL | `.jxl` | 🔎 | — | P1 | jxl-rs or jxl-oxide (pure Rust); export: own thin FFI to libjxl (BSD) | the Rust bindings of libjxl are GPL: not usable |
| AVIF | `.avif` | 🔎 | — | P1 | avif-decode (rav1d) or dav1d; export: ravif / rav1e | grid AVIF needs libheif (LGPL, isolated) |
| Camera RAW (Camera Raw formats, DNG) | `.dng`, `.cr2`, `.cr3`, `.nef`, `.arw`, `.raf`, `.orf`, `.rw2`, `.pef`, … | 🔎 | — | P1 | rawler (LGPL-2.1, isolated behind a feature) | a non-destructive development node, not a baked import; demosaic algorithms written clean-room (the best known ones are GPL) |
| DICOM | `.dcm` | 🔎 | — | P1 | dicom-rs | an interpretation node (modality and VOI LUT) instead of baking the window; 12-bit JPEG needs libjpeg-turbo |
| JPEG 2000 | `.jp2`, `.jpf`, `.jpx`, `.j2k`, `.j2c`, `.jpc` | 🔎 | — | P1 | hayro-jpeg2000 (pure Rust) → OpenJPEG (optional) | OpenJPEG declared itself unmaintained in 2026 |
| **Photoshop PDF, Generic PDF** | `.pdf`, `.pdp` | 🔎 | — | **P1** | hayro → PDFium (optional) | import only, see [PDF](#pdf-p1) |
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
