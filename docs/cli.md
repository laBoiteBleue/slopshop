# Command-line tool: `slopshop`

`slopshop` runs the engine without the desktop app: GPU check, headless rendering and image
export. It is the proof that the engine works without the UI, and the base for batch work.

```sh
cargo run -p slopshop-cli -- --help          # from the repository
cargo run --release -p slopshop-cli -- …     # release build: much faster on large images
```

The binary is `slopshop` (`target/release/slopshop` after `cargo build --release -p
slopshop-cli`). Every command prints its result on standard output and its errors on standard
error, prefixed with `error:`. The exit code is 0 on success, 1 on any error.

| Command | What it does |
|---|---|
| [`slopshop gpu`](#slopshop-gpu) | Show the GPU adapter the engine would use |
| [`slopshop render`](#slopshop-render) | Render a built-in demo document to a PNG file |
| [`slopshop export`](#slopshop-export) | Open an image or a document and export it to PNG, TIFF, OpenEXR, JPEG, WebP, BMP, Targa or a layered PSD or PSB |
| [`slopshop save`](#slopshop-save) | Save images as one `.slop` document, one layer each |
| [`slopshop inspect`](#slopshop-inspect) | Show a `.slop` document: file state and layers |
| `slopshop --help`, `slopshop -h` | Print the usage |
| `slopshop --version`, `slopshop -V` | Print the version |

## `slopshop gpu`

```sh
slopshop gpu
```

Prints the adapter's name, backend (DX12 on Windows), device type and driver. Fails when no
usable GPU is found (the error says why). The backend can be forced with the `WGPU_BACKEND`
environment variable (e.g. `WGPU_BACKEND=vulkan`).

## `slopshop render`

```sh
slopshop render [--size WxH] [--doc WxH] [--out PATH]
```

Builds a demo document of procedural fill layers, fits it into an output image and writes it as
an 8-bit sRGB PNG. It exercises the viewport renderer (the one the app displays with), not the
export pipeline.

| Option | Meaning | Default |
|---|---|---|
| `--size WxH` | Size of the output image, in pixels | `1024x768` |
| `--doc WxH` | Size of the demo document | `12000x8000` |
| `--out PATH` | Output file | `slopshop.png` |

## `slopshop export`

```sh
slopshop export <INPUT> <OUTPUT> [options]
```

Opens `INPUT` (a `.slop` document, recognized by its content, or any image format the importer
reads: PNG, JPEG, TIFF, WebP, GIF, BMP, TGA, ICO, PNM/PFM, QOI, farbfeld, OpenEXR, HDR, DDS, as a
one-layer document) and writes the composited image to `OUTPUT`
through the export pipeline, exactly as the app does ([ADR 0008](adr/0008-export.md),
[ADR 0010](adr/0010-jpeg-webp-export.md)). A PSD or PSB keeps the layers instead (see
[Layered PSD and PSB](#layered-psd-and-psb)):

- the format's **default settings** for this document, each option overriding one of them;
- the **GPU** renders the pixels (the CPU compositor when there is no GPU, or with `--cpu`);
- the image is processed in bands of 256 rows, so memory does not grow with the height
  (except for WebP, which must hold the whole frame: at most 16383 × 16383 pixels);
- the file is written to a temporary file next to `OUTPUT`, then renamed: an existing `OUTPUT`
  is only replaced by a complete file, and a failed export leaves it untouched.

### Options

| Option | Values | Meaning |
|---|---|---|
| `--format` | `png`, `tiff`, `exr`, `jpeg`, `webp`, `psd`, `psb`, `bmp`, `tga` | File format. Default: from the extension of `OUTPUT` (`.png`, `.tif`, `.tiff`, `.exr`, `.jpg`, `.jpeg`, `.webp`, `.psd`, `.psb`, `.bmp`, `.tga`, any case) |
| `--depth` | `u8`, `u16`, `f16`, `f32` | Sample type: 8/16-bit integers, 16/32-bit floats (see the table below for each format). The color space stays the default one unless `--space` is given |
| `--space` | see [Color spaces](#color-spaces) | Color space of the file. Always tagged in the file |
| `--compression` | `fast`, `small` (PNG); `none`, `deflate`, `lzw` (TIFF); `lossy`, `lossless` (WebP); `rle`, `none` (TGA) | PNG, TIFF and TGA are always lossless. OpenEXR always uses lossless ZIP; JPEG uses `--quality` and `--subsampling`; BMP is uncompressed |
| `--quality` | `1` to `100` (JPEG), `0` to `100` (lossy WebP) | Quality of lossy compression |
| `--subsampling` | `444`, `422`, `420` | JPEG only: chroma subsampling. `444` keeps full color resolution, `420` gives the smallest files |
| `--no-alpha` | | Drop the alpha channel: transparency is flattened over the matte. JPEG never has alpha; a layered PSD or PSB always keeps it |
| `--matte` | `RRGGBB` or `#RRGGBB` | Color transparency is flattened over when there is no alpha, as sRGB hex. Default: `ffffff` (white) |
| `--no-dither` | | No dither for 8-bit samples (dither reduces banding; it is on by default) |
| `--gray` | | Gray samples (PNG, TIFF and JPEG): each pixel becomes the luminance of its color in the file's color space. Pixels that had color are counted in the report. Default for gray documents |
| `--color` | | Color samples, even for a gray document |
| `--scale` | a factor, e.g. `4` or `0.25` | Resample the whole image by this factor first, like Image Size: sides are rounded, and layers are resampled with the quality filter of transformed layers ([ADR 0018](adr/0018-resampling.md)) |
| `--cpu` | | Composite on the CPU instead of the GPU |
| `--bench` | | Also print timings (open, GPU init, export, throughput, time in the pixel source) and the number of bands |

Options can come in any order, before or after the paths. An option the format does not have
(e.g. `--depth u16` for JPEG, `--quality` for PNG) is an error, which lists the valid values.

### Formats and defaults

| | PNG | TIFF | OpenEXR | JPEG | WebP | PSD, PSB |
|---|---|---|---|---|---|---|
| `--depth` | `u8`, `u16` | `u8`, `u16`, `f32` | `f32`, `f16` | `u8` | `u8` | `u8`, `u16` |
| Default depth | `u8` if every visible raster is 8-bit, else `u16` | the deepest source type (`f32` when any source is float) | `f32` | `u8` | `u8` | as PNG |
| Default space | 8-bit: sRGB; 16-bit: the source space if PNG can tag it, else sRGB | the source space if it can be tagged, else Rec.2020 (integers) or linear Rec.2020 (float) | linear sRGB | the source space if it is sRGB, Display P3 or Adobe RGB, else sRGB | as JPEG | 8-bit: sRGB; 16-bit: the source space if it can be tagged, else sRGB |
| Compression | `fast` | `deflate` | ZIP (fixed) | quality 90, `444` | `lossy`, quality 90 (alpha kept losslessly) | RLE (fixed) |
| Alpha | kept unless the document is opaque | kept unless the document is opaque | kept unless the document is opaque | never: flattened over the matte | kept unless the document is opaque | always kept |
| Gray | yes, default for gray documents | yes, default for gray documents | no | yes, default for gray documents | no | no |
| Size limit | 2³¹−1 px per side | BigTIFF above 4 GiB | about 2³⁰ px per side | 65 500 px per side | 16 383 px per side | PSD: 30 000 px per side; PSB: 300 000 |

Lossy WebP stores its image modes in a first partition limited to 512 KiB: very detailed
images near the size limit may not fit, even at low quality. The export then fails with
`contentTooComplex`; lossless WebP and JPEG have no such limit.

### BMP and Targa

8-bit sRGB only (neither format can declare another space; BMP's header declares sRGB), with
alpha unless the document is opaque or `--no-alpha` is given. BMP is uncompressed (at most
4 GiB); Targa is run-length encoded by default (`--compression none` for uncompressed), at
most 65 535 pixels per side. Rows are streamed: memory does not grow with the image.

### Layered PSD and PSB

A `.psd` output keeps the document's structure, for Photoshop and the other editors that read
PSD layers; a `.psb` (Photoshop's large document format) does the same beyond PSD's 30,000
pixels per side:

- pixel layers with their position, opacity, blend mode, visibility and clipping; groups (pass
  through or isolated); layer masks (disabled ones too); fill layers, as pixels;
- adjustment layers as Photoshop adjustment layers (Hue/Saturation, Levels,
  Brightness/Contrast, Curves, Exposure, Vibrance, Color Balance, Black & White, Photo
  Filter, Channel Mixer, Invert, Posterize, Threshold);
- a merged composite for readers that do not read layers, and the ICC profile of `--space`.

Every layer is rendered on its own at its pixel size, so transformed (rotated, scaled) layers
are written resampled, and a layer's parts outside the canvas are cut (reported as
`pixelsOutsideCanvas`). Layer styles, text and smart objects do not exist in SlopShop yet.
The compressed layers wait in a temporary file next to `OUTPUT` until the file is assembled:
memory does not grow with the document.

The **source space** is the color space of the input image (from its ICC profile, cICP, gAMA/cHRM
or format convention). A document is **opaque** when its bottom layer is an opaque fill or an
image without alpha covering the whole canvas. A document is **gray** when every visible layer is
a gray image or a neutral fill, with one image at least: its gray samples are then kept.

### Color spaces

`--space` takes one of these identifiers:

| Id | Space |
|---|---|
| `srgb` | sRGB |
| `linear-srgb` | sRGB primaries, linear |
| `display-p3` | Display P3 |
| `adobe-rgb` | Adobe RGB (1998) |
| `prophoto` | ProPhoto RGB |
| `rec2020` | Rec.2020 |
| `linear-rec2020` | Rec.2020 primaries, linear |
| `rec2100-pq` | Rec.2100 PQ (HDR) |
| `rec2100-hlg` | Rec.2100 HLG (HDR) |

Which spaces each format can store *and tag* so that it reads back as the same space:

- **PNG**: all of them (sRGB chunk, cICP chunk, or ICC profile).
- **BMP** and **Targa**: `srgb` only.
- **TIFF**, **JPEG**, **WebP**, **PSD** and **PSB**: all but `rec2100-pq` and `rec2100-hlg` (ICC profile).
- **OpenEXR**: `linear-srgb` and `linear-rec2020` (EXR samples are linear; the primaries go in
  the `chromaticities` attribute).

Gray files keep only the tone curve of the space (and its primaries define the luminance): PNG
tags them with the sRGB chunk or an ICC gray profile, TIFF and JPEG with an ICC gray profile, so
`rec2100-pq` and `rec2100-hlg` are not available for gray.

### Output

```
exported 21600x10800 in.jpg to out.png
settings: --format png --depth u8 --space srgb --compression fast
source: GPU (<adapter name>)
report: clippedHigh (1234 samples)
```

- `settings:` the settings actually used, written as the options that would give them: copy
  them to repeat an export exactly.
- `source:` what rendered the pixels. A line `note: no usable GPU (…)` comes first when the GPU
  was not available.
- `import warning:` lines, first, when the input could not be read without approximation.
- `report:` lines, one per lossy event (nothing is ever converted silently):

| Report id | Meaning |
|---|---|
| `clippedHigh` | Values above the format's range were clipped (count in samples) |
| `clippedLow` | Negative or out-of-gamut values were clipped (samples) |
| `nonFinite` | Infinite or NaN values were replaced (samples) |
| `halfOverflow` | Values beyond the 16-bit float range were written as ±65504 (samples) |
| `precisionReduced` | The samples are 16-bit floats, less precise than the image |
| `bigTiff` | The file was written as BigTIFF (over 4 GiB): some older software cannot read it |
| `alphaFlattened` | Partly transparent pixels were flattened over the matte (count in pixels) |
| `pixelsOutsideCanvas` | Parts of layers were outside the canvas: the layered PSD or PSB keeps only what is inside |

### Examples

```sh
# Same format family, defaults: 8-bit sRGB PNG from an 8-bit JPEG
slopshop export photo.jpg photo.png

# 16-bit TIFF in Display P3, LZW for older software
slopshop export scan.tif scan-p3.tif --depth u16 --space display-p3 --compression lzw

# Linear half-float OpenEXR
slopshop export render.png render.exr --depth f16

# Web JPEG: smaller file, transparency over a dark gray background
slopshop export logo.png logo.jpg --quality 80 --subsampling 420 --matte 202020

# PNG without transparency, over white
slopshop export sticker.png sticker-flat.png --no-alpha

# WebP for the web, and a lossless one
slopshop export photo.png photo.webp --quality 80
slopshop export icon.png icon.webp --compression lossless

# Photoshop, with the layers, in 16-bit
slopshop export poster.slop poster.psd --depth u16

# Measure throughput without the GPU
slopshop export big.tif out.tif --cpu --bench
```

## `slopshop save`

```sh
slopshop save <IMAGE>... --out <FILE.slop> [--bench]
```

Opens the images and saves them as one document in SlopShop's own format
([ADR 0009](adr/0009-document-file-format.md)): one layer per image, the first at the bottom,
the canvas as large as the largest image. Pixels are kept exactly as imported (8/16-bit, half
or float, with their color space), compressed losslessly.

| Option | Meaning |
|---|---|
| `--out FILE` | The document to write (replaced whole if it exists) |
| `--bench` | Also print how long opening the images and saving took |

## `slopshop inspect`

```sh
slopshop inspect <FILE.slop> [--bench]
```

Opens a document and prints its generation (number of saves), its size and how much of it is
unused (older data, reclaimed when a save compacts the file), the canvas size and blend space,
and every layer (id, name, image format or fill color, blend mode, opacity, visibility). `--bench` also prints how long
opening took.

```sh
slopshop save background.tif overlay.png --out montage.slop
slopshop inspect montage.slop
slopshop export montage.slop montage.png
```
