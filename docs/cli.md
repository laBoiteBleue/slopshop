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
| [`slopshop export`](#slopshop-export) | Open an image file and export it to PNG, TIFF, OpenEXR or JPEG |
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

Opens `INPUT` (any format the importer reads: PNG, JPEG, TIFF, WebP, GIF, BMP, TGA, ICO,
PNM/PFM, QOI, farbfeld, OpenEXR, HDR, DDS) as a one-layer document and writes it to `OUTPUT`
through the export pipeline, exactly as the app does ([ADR 0008](adr/0008-export.md),
[ADR 0010](adr/0010-jpeg-webp-export.md)):

- the format's **default settings** for this document, each option overriding one of them;
- the **GPU** renders the pixels (the CPU compositor when there is no GPU, or with `--cpu`);
- the image is processed in bands of 256 rows, so memory does not grow with the height;
- the file is written to a temporary file next to `OUTPUT`, then renamed: an existing `OUTPUT`
  is only replaced by a complete file, and a failed export leaves it untouched.

### Options

| Option | Values | Meaning |
|---|---|---|
| `--format` | `png`, `tiff`, `exr`, `jpeg` | File format. Default: from the extension of `OUTPUT` (`.png`, `.tif`, `.tiff`, `.exr`, `.jpg`, `.jpeg`, any case) |
| `--depth` | `u8`, `u16`, `f16`, `f32` | Sample type: 8/16-bit integers, 16/32-bit floats (see the table below for each format). The color space stays the default one unless `--space` is given |
| `--space` | see [Color spaces](#color-spaces) | Color space of the file. Always tagged in the file |
| `--compression` | `fast`, `small` (PNG); `none`, `deflate`, `lzw` (TIFF) | Lossless compression. OpenEXR always uses lossless ZIP; JPEG uses `--quality` and `--subsampling` |
| `--quality` | `1` to `100` | JPEG only: quality, on the usual IJG scale |
| `--subsampling` | `444`, `422`, `420` | JPEG only: chroma subsampling. `444` keeps full color resolution, `420` gives the smallest files |
| `--no-alpha` | | Drop the alpha channel: transparency is flattened over the matte. JPEG never has alpha |
| `--matte` | `RRGGBB` or `#RRGGBB` | Color transparency is flattened over when there is no alpha, as sRGB hex. Default: `ffffff` (white) |
| `--no-dither` | | No dither for 8-bit samples (dither reduces banding; it is on by default) |
| `--cpu` | | Composite on the CPU instead of the GPU |
| `--bench` | | Also print timings (open, GPU init, export, throughput, time in the pixel source) and the number of bands |

Options can come in any order, before or after the paths. An option the format does not have
(e.g. `--depth u16` for JPEG, `--quality` for PNG) is an error, which lists the valid values.

### Formats and defaults

| | PNG | TIFF | OpenEXR | JPEG |
|---|---|---|---|---|
| `--depth` | `u8`, `u16` | `u8`, `u16`, `f32` | `f32`, `f16` | `u8` |
| Default depth | `u8` if every visible raster is 8-bit, else `u16` | the deepest source type (`f32` when any source is float) | `f32` | `u8` |
| Default space | 8-bit: sRGB; 16-bit: the source space if PNG can tag it, else sRGB | the source space if it can be tagged, else Rec.2020 (integers) or linear Rec.2020 (float) | linear sRGB | the source space if it is sRGB, Display P3 or Adobe RGB, else sRGB |
| Compression | `fast` | `deflate` | ZIP (fixed) | quality 90, `444` |
| Alpha | kept unless the document is opaque | kept unless the document is opaque | kept unless the document is opaque | never: flattened over the matte |
| Size limit | 2³¹−1 px per side | BigTIFF above 4 GiB | about 2³⁰ px per side | 65 500 px per side |

The **source space** is the color space of the input image (from its ICC profile, cICP, gAMA/cHRM
or format convention). A document is **opaque** when its bottom layer is an opaque fill or an
image without alpha covering the whole canvas.

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
- **TIFF** and **JPEG**: all but `rec2100-pq` and `rec2100-hlg` (ICC profile).
- **OpenEXR**: `linear-srgb` and `linear-rec2020` (EXR samples are linear; the primaries go in
  the `chromaticities` attribute).

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

# Measure throughput without the GPU
slopshop export big.tif out.tif --cpu --bench
```
