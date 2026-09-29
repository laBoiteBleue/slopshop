# Research: universal image import

Status: **research brief**, no decision taken yet. Produced by a multi-agent research run with
adversarial fact-checking of its claims.

*SlopShop decision brief, state as of 2026-09-29. Research only; the repository was not modified. Every claim below comes from the verified research material. Findings the verification marked wrong have been corrected, and anything unverified or older than about 18 months is flagged.*

---

## 1. Summary

- **No single library gives universal import on SlopShop's terms.** Those terms are native bit depth, explicit colour, no whole-image-in-RAM assumption, permissive licensing and contained `unsafe`. Umbrella libraries (libvips, OpenImageIO, ImageMagick) either bake RAW and DICOM at open time, bring LGPL/GPL components and 20–50 C libraries, or both.
- **In practice, "universal" means four things.** (1) A SlopShop-owned decoder contract. (2) Pure-Rust codecs for about 90% of real files. (3) In-house readers for layered formats (PSD, KRA, XCF, ORA). (4) Optional native backends for the long tail, run out of process. Formats that cannot be decoded get a clear explanation: Affinity, deep EXR, 12-bit DICOM JPEG without the optional backend, and so on.
- **Recommended strategy: (D) hybrid.** The default build is pure Rust behind a `slopshop-io` decoder trait. Opt-in native backends (OpenJPEG, libjpeg-turbo 3.2, libheif, LibRaw/CDDL, CharLS, OpenSlide, GDAL, PDFium) sit in isolated crates and a decode worker process.
- **The engine is the real bottleneck, not the libraries.** Today `RasterImage::from_pixels` accepts only `RGBA8_SRGB` (`crates/slopshop-core/src/raster.rs:106-112`), ADR 0005 explicitly rejects 16-bit and float, and `ColorSpace` only has `LinearSrgb` and `Srgb`. Import cannot be universal until storage, the CMS and the "interpretation node" model exist.
- **Do not make `image::DynamicImage` the interchange type.** It silently maps TIFF CMYK to RGB, has no f16, Lab or multiband types, and defaults to a 512 MiB allocation limit. Call the individual codec crates directly.

---

## 2. Format coverage matrix

Legend. **Region**: whether it can decode part of an image without decoding the whole. "tile" means random access, "band" means sequential row streaming, "no" means full frame only. **Maturity**: H = high, M = medium, L = low or young. ⚠ = older than 18 months or possibly outdated.

| Format | Best option (default → optional) | Kind | License | Max bit depth | Region/tiled | Maturity | Notes |
|---|---|---|---|---|---|---|---|
| **JPEG** | zune-jpeg 0.5.15 (released 2026-03-26; 0.5.16-rc2 is a pre-release) → jpeg-decoder 0.3.2 for lossless only → libjpeg-turbo 3.2 (optional FFI) | pure Rust → C FFI | MIT/Apache/Zlib; libjpeg-turbo IJG+BSD-3+Zlib | 8 (zune); 2–16 lossless (jpeg-decoder); 12/16 (turbo) | no (zune); crop/scanline only through raw turbo APIs | H | zune rejects arithmetic, 12-bit, lossless and hierarchical JPEG ([src](https://github.com/etemesi254/zune-image)). jpeg-decoder is in maintenance mode (Jun 2025) and has a quirk: colour lossless above 8 bits emits 16-bit samples while `info()` still says RGB24. The safe `turbojpeg` 1.5.1 crate is 8-bit only. `turbojpeg-sys` has `tj3Decompress12/16` and `tj3SetCroppingRegion` but vendors the older libjpeg-turbo 3.1.0, so link a system 3.2+ instead. No maintained crate exposes libjpeg-turbo 3's scanline API, so that would mean writing our own bindings. |
| **JPEG 2000 / HTJ2K** | hayro-jpeg2000 0.4.0 → OpenJPEG 2.5.4 via jpeg2k 0.10.1 → OpenJPH 0.32.0 (HTJ2K only) | pure Rust → C / C++ | MIT/Apache; OpenJPEG BSD-2; OpenJPH BSD-2 | per-component samples (f32 plus bit depth) in hayro; 16+ in OpenJPEG | hayro: no (resolution hint only, whole codestream in memory); OpenJPEG: decode area plus reduce | hayro M, OpenJPEG M⚠ | hayro forbids `unsafe` but its default `simd` feature pulls unsafe via fearless_simd, so use `default-features=false`. It has no HTJ2K. **OpenJPEG declared itself "unmaintained" on 2026-07-07** ([README](https://github.com/uclouvain/openjpeg/blob/master/README.md)). `openjpeg-sys` 1.0.12 bundles 2.5.3, not 2.5.4. Grok is AGPL and Kakadu cannot be part of an open build, so both are excluded. Watch j2k 0.11.3 (3 months old, single author). |
| **PNG / APNG** | png 0.18.1 (used directly) | pure Rust | MIT/Apache | 16 | band (non-interlaced); Adam7 needs full frame or our own deinterlacing | H | Exposes iCCP, cICP, mDCV, cLLI, eXIf, gAMA and cHRM. `image`'s ApngDecoder returns an *error* on 16-bit APNG, so use png directly for animation. |
| **GIF** | gif 0.14.2 | pure Rust | MIT/Apache | 8 (palette) | no (per frame) | H | Frames carry their own rect, disposal and delay; `ColorOutput::Indexed` keeps palette indices. |
| **BMP** | `image` 0.25.10 built-in | pure Rust | MIT/Apache | 8/channel | `read_rect` exists but is **removed in `image` 0.26** | H | Embedded V5 ICC is not exposed. The standalone `bmp` crate (2019) is ⚠. |
| **TIFF / BigTIFF** | tiff 0.11.3 | pure Rust | MIT | u8–u64, i8–i64, f16/f32/f64 | tile/strip (`read_chunk`) | H | Supports Gray, RGB, Palette, CMYK(A), YCbCr, ICCLab (PI 9) and Multiband. Rejected: CIELab (PI 8), ITULab (PI 10), old-style JPEG, LZMA, LERC, JPEG XL and JPEG 2000 in TIFF, and 12-bit JPEG. **0.11.3 cannot decode SubIFD pixels**, so OME-TIFF and SubIFD pyramids need the unreleased 0.12. It also has a right-border tile corruption bug that is fixed on master. The `zstd` feature pulls C libzstd. |
| **WebP** | image-webp 0.2.4 (Aug 2025) | pure Rust, `forbid(unsafe)` | MIT/Apache | 8 (format limit) | no (≤16383²) | H | Decodes every feature. `read_frame` returns composited canvases, so raw ANMF rect, blend and dispose need our own parsing. Fixes after 0.2.4 are unreleased. Lossy encoding needs libwebp. |
| **AVIF** | avif-decode 3.0.0 (rav1d) → dav1d FFI → libheif | pure Rust* → C | BSD-3 / BSD-2 / MPL-2.0 (avif-parse) | 12 | no. **Grid AVIF is rejected** by avif-parse and mp4parse; libheif handles grids | M | *rav1d is transpiled with 565 `unsafe` and needs nasm on x86. avif-decode is marked "passively-maintained" and supports no HDR and no ICC. rav1d 1.1.0 dates from May 2025 (borderline). mp4parse 0.17 (2023) ⚠. In `image`, the default `avif` feature is the *encoder*; decoding needs non-default `avif-native` plus a system libdav1d. |
| **HEIC / HEIF** | libheif 1.23.5 (libheif-sys 5.3.1 / libheif-rs 3.0.0) + libde265 → OS decoders (WIC, ImageIO) | C FFI / system | **LGPL-3.0** (libheif, libde265) | 16 | tile decode only through raw libheif-sys (API ≥1.19); the safe libheif-rs has none | H upstream | **HEVC patent pools** (Access Advance, Via LA); patent and Fedora notes not re-checked this round. `embedded-libheif` builds libheif without its codecs. On Windows, HEIF Image Extensions is free but HEVC Video Extensions is paid. Linux has no OS fallback. The pure-Rust imazen `heic` is **AGPL**, so avoid it. |
| **JPEG XL** | jxl 0.7.4 (jxl-rs) or jxl-oxide 0.12.6 | pure Rust | BSD-3 / MIT-Apache | F32 | jxl-rs: no crop API; jxl-oxide: yes (`set_image_region`) | jxl-rs H (very active), jxl-oxide M (bus factor 1) | jxl-rs has 84 `unsafe` outside SIMD, which its README says needs review. jxl-oxide has a pluggable CMS. **libjxl Rust bindings (jpegxl-rs/sys) are GPL-3.0.** For export, write our own thin FFI to BSD libjxl. |
| **OpenEXR** | exr 1.74.2 → OpenEXRCore 3.5.1 C (optional) | pure Rust → C | BSD-3 | f16/f32/u32 | tile/block via low-level API (custom code) | H | DWAA/DWAB are **now supported** (1.74.1 and 1.74.2). Not supported: deep data, subsampling, HTJ2K (recognised then rejected) and Zstd (OpenEXR 3.5, no variant). OpenEXR fixed 15 CVEs in Aug 2026. openexr-rs has been dead since 2021 ⚠. |
| **Radiance HDR** | `image` hdr codec | pure Rust | MIT/Apache | f32 (RGBE) | no | H | `PRIMARIES` is only in `custom_attributes`, so parse it ourselves. Keep data linear and never tone-map at import. |
| **PSD / PSB** | **In-house** reader from Adobe's spec; ag-psd (TS 31.0.2) and psd-tools 1.21.0 as structure oracles | pure Rust (ours) | MIT (ours) | 32 (float, linear) | row-level (PackBits row table) | to build | **Correction: layer styles (lfx2) and adjustment layers are *not* stored as pixels.** A faithful render needs the composite, which is only real with "Maximize Compatibility", or native nodes. Rejected crates: psd 0.3.5 ⚠ (no PSB, 16-bit narrowed), rawpsd (8-bit), ag-psd-rs (narrows 16/32-bit to RGBA8, which is lossy), zune-psd (no layers, no CMYK). Adobe's spec is dated Nov 2019. |
| **KRA / XCF / ORA** | In-house readers (zip + own LZF + XML/PNG) | pure Rust (ours) | MIT (ours) | KRA U8/U16/F16/F32; XCF up to 32-bit int and 64-bit float; ORA 8/16 | tile-native (64×64 sparse tiles in KRA and XCF) | to build | KRA tiles are byte-planar BGRA with LZF. XCF spec is v26 (GIMP 3.2.6, 2026-09-10); a new zipped-XML GIMP format was announced with **no target version given**. The existing crates are ⚠ (krita 0.2.1 from 2020, MPL-2.0; xcf 0.4.0 from 2023) or panic on non-RGB (xcf, xcf-rs). The `lzf` crate is deprecated. image-extras ORA reads only `mergedimage.png`. |
| **SVG** | resvg/usvg 0.48.1 → vello_svg 0.11 (GPU) | pure Rust | MIT/Apache | 8 (RGBA8 premultiplied, sRGB) | yes: re-rasterise any tile or zoom through the transform | H | Text shaping now uses harfrust and skrifa. No animation. Fonts must be bundled or substitutions reported. |
| **PDF / EPS / AI** | hayro 0.7.1 → pdfium-render 0.9.4 + PDFium → stet 0.8.2 (EPS/PS) | pure Rust → C++ → pure Rust | MIT/Apache; PDFium BSD-3 | 8 (CMYK becomes RGB, so record it) | yes through an affine transform; hayro caps at 65,535 px per render target | hayro M, PDFium H, stet L | hayro has **no multithreading feature** (PR #1317 closed). pdfium-render serialises every call behind a mutex, and 0.9.4 fixed double-frees. **stet embeds AGPL URW fonts whose exception does not cover app binaries**, which blocks it without a font-free fork. MuPDF (AGPL), Poppler (GPL) and Ghostscript (AGPL) are rejected. Modern `.ai` files are PDF-compatible. |
| **Camera RAW** | rawler 0.8.0 (dnglab) → LibRaw 0.22.2 under CDDL-1.0, out of process | pure Rust → C++ | **LGPL-2.1** / LibRaw LGPL-2.1 OR CDDL-1.0 | u16 or f32 CFA | no (whole frame, about 800 MB at 400 MP) | rawler M-H (active, no SemVer) | rawler covers CR3, Sony lossless and ARW6, compressed RAF/X-Trans, and DNG 1.7 JXL. It lists **X3F but does not decode it, and its `raw_metadata()` panics (`todo!()`)** on X3F. LibRaw has a steady CVE stream (CVE-2026-21413, CVSS 9.8), X3F only with `USE_X3FTOOLS` (the riskiest code), and dropped RED, C500 and ARRI in 0.22. All high-end demosaics (AMaZE, RCD) are GPL, so write our own clean-room versions. |
| **DICOM** | dicom-rs 0.10.0 (object, pixeldata, transfer-syntax-registry) | pure Rust + optional codecs | MIT/Apache | 16 (signed/unsigned) | per-frame decode works; **whole file in RAM by default** | M-H | 12-bit DCT JPEG (process 4, common in CT) fails through jpeg-decoder, so libjpeg-turbo is needed. The `charls` feature statically builds **CharLS 2.4.2, which is in the vulnerable range** (fixed in 2.4.4); use system CharLS ≥2.4.4 via vcpkg. The `gdcm` feature uses gdcm-rs 0.6 (Feb 2024, GDCM pinned to 2022) ⚠. VOI helpers bake output, so take raw samples. |
| **FITS** | fitsrs 0.4.1 → fitsio 0.21.10 (CFITSIO) | pure Rust → C | MIT/Apache | i64 / f64 | row/pixel seek | M | Tile compression covers only GZIP, GZIP2 and RICE; **no HCOMPRESS or PLIO**. **An overflow fix is unreleased in 0.4.1**, so wait for 0.4.2 or use git for untrusted files. fitsio-src bundles CFITSIO 3.49 (2020) ⚠, so link system 4.x. |
| **GeoTIFF / whole-slide** | tiff + own GeoKeys and OME-XML parsing; zarrs 0.23.14 + ome_zarr_metadata (OME-Zarr) → OpenSlide 4.0.1 (openslide-bin) → GDAL 3.13 | pure Rust → C | MIT; OpenSlide LGPL-2.1; GDAL MIT | native (tiff); OpenSlide **8-bit premultiplied ARGB only** | tile / pyramid | M | SVS JPEG 2000 tiles need a J2K decoder on top of tiff. DICOM WSI frame indexing can be built in Rust or taken from libdicom (MIT, C, Meson). zarrs pulls C (blosc, zstd) by default. gdal-src disables JP2, Zarr and WebP, so the full set needs a system GDAL. gdal-sys 0.12 has no prebuilt bindings for GDAL 3.13 (needs libclang). Bio-Formats (GPL, JVM) is excluded. |
| **QOI / TGA / ICO / PNM / DDS** | qoi 0.4.1 ⚠ (frozen spec); `image` TGA/ICO/PNM; zune-ppm 0.5.1 (PFM); dds 0.2.0 (BC1–7, ASTC, float); ico 0.5.0 (CUR hotspots) | pure Rust | MIT/Apache | 16 (PNM); f32 (PFM, BC6H) | no | H / M | `image`'s DDS supports only DXT1/3/5 and is decode-only. `image`'s ICO tolerates CUR but hides the hotspot. image-extras (PCX, SGI, XBM…) is unfuzzed with all features on by default. KTX2 is parse-only; basis-universal is C++ and 2023 ⚠. |
| *Affinity .af / .afphoto* | afphoto 0.1.0 (embedded PNG preview only) | pure Rust | MIT | 8 (flattened preview) | no | L | Closed format. The preview route is tested only on Affinity Photo 2 `.afphoto` and **untested on `.af`**. Otherwise show "unsupported, export PSD or TIFF". |

---

## 3. Strategy options compared

| | **(A) Per-format crates behind an importer registry** | **(B) libvips as universal backend** | **(C) OpenImageIO** | **(D) Hybrid: A by default, optional native backends** |
|---|---|---|---|---|
| **Coverage** | Common raster, JXL, AVIF (not grid), JP2 Part 1, EXR (not deep), TIFF, FITS, DICOM native, RAW (rawler), SVG, PDF. Gaps: HEIC, 12-bit JPEG, HTJ2K, JPEG-LS, WSI vendor formats, deep EXR. | Broad in its "all" build: JP2, JXL, EXR, FITS, RAW via dcrawload, OpenSlide/libdicom, magick fallback. The prebuilt Windows zips **exclude libde265, so no HEIC**. | Broad: TIFF, EXR incl. deep, JP2/HTJ2K, RAW (LibRaw), DICOM (DCMTK, read-only, full frame), PSD composite, HEIF, JXL. | A's coverage, plus the gaps filled one at a time. |
| **Licensing for MIT distribution** | Mostly MIT/Apache/BSD. rawler is LGPL-2.1 (manageable, behind a feature). | libvips is LGPL-2.1+ and must be dynamically linked; glib, libheif and librsvg are LGPL-3. The "all" build includes **GPL fftw and poppler**, which must be removed. | OIIO is Apache-2.0 (good), but its LibRaw and libheif deps are LGPL. | Per-backend choice, enforced by a cargo-deny allowlist. |
| **Packaging (Win/macOS/Linux)** | Trivial: `cargo build`. Some deps need nasm (rav1d asm). | Official Windows prebuilt zips (9.4 MB web, 19.7 MB all). On macOS/Linux, sharp-libvips is web formats only, so build it ourselves. The vcpkg port **lacks** openjpeg, libraw, heif, jxl and exr features and needs an overlay port or meson. | Heaviest: CMake 3.23, C++17, required OpenColorIO, OpenEXR and more. oiio-sys supports vcpkg on Windows and pkg-config on Unix. The vcpkg port lacks DICOM, OpenJPH and UltraHDR features. | Only the backends a user enables. Each gets its own CI job with vcpkg binary caching. |
| **Security** | Mostly safe Rust; fuzz our own wrappers. | On OSS-Fuzz with 12 advisories (3 high in 2026). JP2, RAW, magick and older HEIF loaders are tagged **UNTRUSTED**, so `vips_block_untrusted_set` would switch off exactly the "universal" loaders; block per operation instead. | 22 advisories in 2026 (11 high); not on OSS-Fuzz. | C/C++ runs in a worker process with memory limits. |
| **Huge images** | Mixed: tiff, exr, jxl-oxide and fitsrs can do region reads; zune-jpeg, hayro and rawler cannot. Needs our own spill-to-disk. | Excellent streaming pipeline. Region loading covers only tiled TIFF, tiled EXR, FITS and OpenSlide. Its cache duplicates our tile store. | Strong: ImageCache, `read_tile`, MIP. The DICOM reader is full frame. | Tiers per codec (see §4.5). |
| **Binary size** | Small, a few MB. | Tens of MB with dependencies. | Likely tens of MB (not measured). | Small by default, grows per option. |
| **Maintenance risk** | Many small crates, some with bus factor 1 (jxl-oxide, zune, rawler). | The upstream project is very active, but the Rust binding is weak: the `libvips` crate generator pins 8.18.2 and its CI is disabled; rs-vips has been inactive since May 2026. | Upstream very active. The Rust binding `oiio` 0.2.1 is 6 weeks old with about 116 downloads. | Spread across the two. |
| **Non-destructive RAW / DICOM** | Yes: raw CFA and raw DICOM samples feed our own nodes. | No: dcrawload hard-codes camera white balance. | Possible: `raw:Demosaic=none` gives the CFA mosaic. | Yes. |

**Verdict.** Adopt D. For the long tail, prefer **targeted native libraries** (OpenJPEG, libjpeg-turbo 3.2, CharLS, libheif, LibRaw/CDDL, OpenSlide, PDFium, OpenEXRCore) over a full umbrella library. If an umbrella is wanted later as a catch-all plugin, OIIO fits better on licence and API (Apache-2.0, tiles, undemosaiced RAW). libvips is faster and leaner but brings LGPL obligations and the problem with its untrusted-loader flags. Keep ImageMagick out of the process entirely: 212 GitHub advisories in 2026, a whole-image pixel cache, and no vcpkg port. At most, allow it as a sandboxed last resort.

---

## 4. What the engine must change, whatever library is chosen

### 4.1 Pixel formats

- **Today:** `SampleType` already lists U8, U16, F16 and F32, and `ChannelLayout` lists Gray, GrayAlpha, Rgb and Rgba (`color.rs`). But `RasterImage` accepts only `RGBA8_SRGB`, and ADR 0005 rejects 16-bit and float sources.
- **Needed:**
  - U16, F16 and F32 tiles for real, plus gray and gray-alpha, with alpha marked straight or premultiplied.
  - Signed integers (i16/i32 for DICOM and FITS) and a `bits_stored` field (10- and 12-bit data in u16).
  - Arbitrary channel counts (EXR AOVs, multispectral, J2K components).
  - A policy for f64 and i64 (FITS −64/64): keep the native type, or convert with a visible warning.
  - CMYK and Lab: either as first-class layer models, or as an explicit, recorded, re-runnable "convert via ICC" node. Never the silent mapping `image`'s TIFF adapter does.
  - A **CFA** layout for RAW (a 2×2 Bayer or 6×6 X-Trans descriptor).
- **ADR 0005 follow-up:** its "store the source format" choice fits this well. The compositing path needs f16/f32.

### 4.2 Colour management

- **Keep source pixels untouched** together with their ICC blob, CICP (including PQ/HLG), gAMA/cHRM, or an explicit "unknown" flag. Conversion to the working space is a render-graph parameter, not an import step.
- **Matrix/TRC profiles on the GPU:** apply them analytically, unbounded in f32. This covers sRGB, P3, AdobeRGB, ProPhoto, Rec.2020 and RAW camera matrices.
- **LUT, CMYK and device-link profiles:** bake a float 3D LUT with **lcms2 6.2.0** (MIT, static build, trivial CI) in float mode, which stays unbounded (ProPhoto green becomes (−0.728, 1.232, −0.153)). **Never enable the GPL fast_float or threaded plugins.**
- **moxcms is not safe as the "no silent clipping" converter yet.**
  - Its default f32 path clips to [0,1].
  - Its extended-range path turns ProPhoto (curv gamma 1.8) exact-0 inputs into values around −1e8, reproduced on 0.9.1; the magnitude depends on the destination profile.
  - It also keeps an sRGB transfer through CICP if you edit TRCs without clearing `.cicp`.
  - The SlopShop lockfile resolves moxcms **0.8.1** through `image`, so adding 0.9.1 directly would put two copies in the build.
  - qcms (8-bit only, Jan 2024 ⚠) is ruled out.
- **Working space** (needs an ADR):
  - The document currently uses `LinearSrgb` (`document.rs:82`).
  - Candidates are linear Rec.2020, ACEScg, or unbounded linear sRGB primaries.
  - Whatever is chosen, f16/f32 values below 0 and above 1 must survive compositing. That is needed for RAW, PQ/HLG and wide-gamut sources.
  - The material does not settle the choice. The least disruptive option is unbounded linear sRGB; a wide-gamut space means fewer negative values.
  - HDR display also depends on the presentation path, which is 8-bit today (see ADR 0002).

### 4.3 Metadata

- Store ICC, EXIF, XMP and IPTC as **verbatim blobs** for round-trip export. Parse EXIF with one crate: nom-exif 3.8.0 (MIT; its "non-standard" licence field points to a plain MIT file). kamadak-exif 0.6.1 (Nov 2024) ⚠. xmp_toolkit is C++ FFI.
- **Orientation** is a non-destructive transform node. Set `adjust_orientation=false` in jxl-rs.
- Domain metadata is document data: DICOM dataset (including patient data, which raises a privacy policy question), FITS header and WCS, GeoTIFF CRS, EXR attributes, OME-XML, WSI vendor properties, physical pixel spacing, HDR metadata (mDCV/cLLI) and gain maps.

### 4.4 Multi-image files

Use a model of **container → images → frames, pages, levels and auxiliary planes**. It covers:

- multi-page TIFF;
- TIFF, JXL and HEIF pyramids;
- animation in GIF, APNG, WebP, JXL and AVIF, with delay, disposal and blend;
- HEIF alpha, depth and thumbnail items;
- EXR parts;
- DICOM frames and series;
- FITS HDUs and cubes;
- WSI associated images;
- DDS mips, arrays and cubemaps;
- PDF pages.

This matches the planned tab behaviour. Dropping on the tab bar opens a new document; dropping on the canvas imports layers. A "choose page, series or frame" step is needed for multi-image files.

### 4.5 Region and tiled decoding into the tiled raster

Add a `TileSource` trait that reads by (level, tile) and has its own cache and I/O threads. The trait advertises one of three tiers:

1. **Random access**: tiff `read_chunk`, jxl-oxide `set_image_region`, EXR blocks, OpenJPEG decode area or reduce, libheif tiles (raw sys), fitsrs row seek, zarrs chunks.
2. **Band streaming**: non-interlaced PNG `next_row`, BMP, PNM, TGA and farbfeld, and libjpeg scanlines (own bindings).
3. **Full frame**: WebP, GIF, QOI, AVIF, zune-jpeg (up to 65535² ≈ 12.8 GB RGB8), hayro-jpeg2000, rawler.

Tier 3 must be bounded by a memory budget checked from the header, and must spill to a disk-backed tile store while decoding. Pyramids should come **from the file** where they exist: J2K resolution levels, TIFF SubIFDs (needs tiff 0.12), EXR MIPs, DICOM WSI levels, OME-Zarr multiscales. Otherwise build them lazily. Always set explicit limits; `image` defaults to 512 MiB, and tiff, png and image-webp each have their own.

### 4.6 Non-destructive "interpretation" nodes

- **RAW development**, as GPU nodes with stored parameters, evaluated per visible tile. Stages in order:
  1. Linearise, subtract per-channel and pattern black, normalise by white, apply GainMap, fix bad pixels.
  2. White balance on the CFA (as shot, preset, or temperature/tint solved to a CCT).
  3. Highlight reconstruction mode.
  4. Demosaic as a parameter: superpixel or bilinear for mip levels and previews; a clean-room high-quality Bayer method and Markesteijn-style for X-Trans. Tile origins align to the CFA period, with a halo.
  5. Camera → XYZ → working space, using the DNG ColorMatrix/ForwardMatrix interpolated by CCT.
  6. Exposure, including BaselineExposure.
  7. Lens corrections.

  Tone curves, denoise and sharpening are *generic* nodes. Show the embedded JPEG preview instantly, labelled as a preview.
- **DICOM**: take raw samples, then apply the modality LUT (rescale slope and intercept), VOI LUT or window (LINEAR, LINEAR_EXACT, SIGMOID, LUT sequence), Presentation LUT and MONOCHROME1 inversion at display time. Never bake them.
- **FITS**: BSCALE/BZERO and BLANK, then a stretch (linear, log, asinh, zscale). **EXR/HDR**: exposure plus a view transform.
- **Foreign nodes**: PSD adjustments, effects, text and smart objects, XCF GEGL effects and Krita filter layers keep their parameter blob. They display the source app's cached pixels, or the composite, and appear in an import report until native nodes exist.

### 4.7 Security and isolation

Every parser reads hostile input.

- Run `cargo-fuzz` targets per in-house reader.
- Use `catch_unwind` plus dimension, allocation and time limits. rawler still has `todo!()` paths, and image-extras is unfuzzed.
- Put each C/C++ codec in its own `slopshop-io-ffi-*` crate and preferably in a **decode worker process** that returns tiles over shared memory. This also keeps LGPL libraries dynamically linked and replaceable.
- Add `cargo-deny` with a licence allowlist that blocks GPL and AGPL: jpegxl-rs/sys, imazen heic/zen*/jpegli-rs, Grok, MuPDF, Poppler, x265, librtprocess, and the LibRaw GPL demosaic packs.

---

## 5. Recommended phased plan

**Phase 0: ADRs (no new formats yet)**
- The decoder contract (header, sample type, colour descriptor, alpha, metadata blobs, multi-image tree, tier-tagged `TileSource`).
- The working space and CMS (lcms2 as the reference, GPU analytic path).
- A licence policy with `cargo-deny`.
- An isolation model (worker process).
- Relaxing ADR 0005 to allow U16, F16 and F32 tiles.

**Phase 1: engine plus common raster**
- Storage and compositing for U16, F16, F32, gray and alpha.
- Colour-managed display.
- Verbatim metadata and an orientation node.
- Container and multi-image model, with frames as layers or a frame list (product decision).
- Codecs called **directly**, not through `image` with all features on: png (16-bit, streaming), zune-jpeg plus jpeg-decoder for lossless, tiff 0.11 (CMYK and Lab kept or explicitly converted), image-webp, gif, `image` built-ins for BMP, TGA, ICO, PNM and HDR, qoi, zune-ppm, exr, dds.

**Phase 2: modern codecs, still pure Rust**
- JPEG XL: jxl-oxide for region decode, or jxl-rs; benchmark both.
- JPEG 2000: hayro-jpeg2000 with `default-features=false`.
- AVIF: avif-decode, which accepts no grids and no HDR.
- The fuzzing harness.
- Upgrade to tiff 0.12 when it ships, for SubIFD pyramids.

**Phase 3: DICOM and FITS**
- dicom-rs native codecs plus windowing and VOI nodes; custom header-only open and frame indexing for large multi-frame files.
- fitsrs 0.4.2 or later, plus stretch nodes.
- WSI via tiff plus our own SVS and DICOM WSI readers.

**Phase 4: RAW**
- rawler pinned to `=0.8.0` in a `slopshop-raw` crate behind a feature, with an ADR justifying LGPL-2.1.
- Develop nodes as above; embedded preview shown first.
- Golden tests against rawpy on *trusted* samples only (its bundled LibRaw is likely 0.22.1, inside the CVE range) and against DNG SDK `dng_validate`.

**Phase 5: layered and vector documents**
- In-house PSD/PSB reader, then KRA, ORA and XCF.
- Needs a layer tree, offsets and bounds, blend modes, masks, clipping and foreign nodes.
- SVG with resvg and PDF with hayro as live vector-source layers, plus a page picker.

**Phase 6: optional native backends (worker process, off by default)**
- libjpeg-turbo 3.2, system-linked with our own bindings, for 12-bit, arithmetic and huge JPEGs.
- OpenJPEG for region reads and HTJ2K, isolated because it is unmaintained.
- OpenJPH for HTJ2K speed.
- CharLS ≥2.4.4 as a system library.
- libheif plus a dav1d or libde265 policy.
- LibRaw under CDDL, X3F off.
- OpenSlide 4.0.1+ via openslide-bin.
- GDAL as a "geospatial pack".
- PDFium.
- OpenEXRCore for deep, HTJ2K and Zstd EXR.
- Every "unsupported" error names the optional component that would open the file.

**Decisions the maintainer must take**
1. Whether shipping LGPL is acceptable: rawler (LGPL-2.1, statically linked, source public), libheif and libde265 (LGPL-3, dynamic), OpenSlide (LGPL-2.1, dynamic). Or is a permissive-only default build a hard rule?
2. HEVC patents: ship libde265, use OS decoders only (no Linux), offer a user-installed plugin, or no HEIC.
3. The working space and f16 vs f32 compositing.
4. Binary size and CI budget for native backends: vcpkg builds are long when uncached.
5. Whether to accept the worker-process isolation boundary, and how it interacts with the ADR 0002 transport.
6. First-class CMYK and Lab, or an explicit conversion node.
7. How animation and multi-page files map to documents.

---

## 6. Open questions

1. **JXL default:** jxl-rs (official, very active, no crop API) or jxl-oxide (region decode and pluggable CMS, bus factor 1)? Does jxl-rs plan a region API?
2. **JPEG 2000 default:** hayro (safe, no region decode, no HTJ2K) or OpenJPEG (region and HTJ2K, but self-declared unmaintained C)? Benchmark both on a 100+ MP JP2 and on a single-tile file without PLT markers. Is j2k (frames-sg) worth a prototype?
3. **12-bit JPEG** (DICOM CT, TIFF): write our own bindings to libjpeg-turbo 3.2, or contribute 12-bit support to zune-jpeg or jpeg-decoder?
4. **Grid AVIF:** accept libheif or libavif for it (libavif Rust crates date from 2024 ⚠), or assemble grids ourselves on top of rav1d or dav1d?
5. **Whole-slide imaging:** accept OpenSlide's 8-bit ARGB as a *flagged lossy* import, or write our own SVS and DICOM WSI readers and use OpenSlide only for proprietary formats (MRXS, NDPI, BIF)?
6. **EXR deep, HTJ2K and Zstd:** bind OpenEXRCore optionally, or reject explicitly for now?
7. **n-D data** (FITS cubes, multi-frame DICOM, OME-Zarr z/t/c): frames as layers, a frame slider per layer, or one tab per frame?
8. **DICOM privacy:** show or hide patient data by default, and what de-identification policy applies on export?
9. **PSD:** which native adjustment nodes come first so imported adjustments stay live? When a layered file is dropped on the canvas, import it as a group or as an embedded, smart-object-like layer?
10. **Vector formats:** live per-tile layers (hayro needs `render_into` with an affine transform, capped at 65,535 px per target) or rasterise at a DPI chosen at import? hayro by default with PDFium as a fallback? Is a font-free fork of stet realistic?
11. **RAW:** does rawler support DNG opcode lists, DCP profiles and triple-illuminant matrices? Its docs cover about 10% of the crate, so the source needs checking. How big is the camera coverage gap to LibRaw (about 857 vs about 1,284 models)? Should Nikon HE/HE* wait for LibRaw's announced fall-2026 decoder or dnglab PR #835? Which clean-room demosaic, after a patent check (for example Malvar-He-Cutler)?
12. **moxcms:** report the extended-range zero-input bug and the clipping default upstream and re-test, or keep lcms2 as the only CPU CMS?
13. **Stale dependencies:**
    - Acceptable because the format is frozen: qoi 0.4.1.
    - Need alternatives: kamadak-exif, mozjpeg crates, mp4parse 0.17, gdcm-rs, the bundled CFITSIO 3.49, the bundled CharLS 2.4.2, the bundled libjpeg-turbo 3.1.0.
    - Borderline, watch: rav1d 1.1.0, openjpeg-sys, jpeg-decoder.
14. **Fuzzing gate:** should an OSS-Fuzz-style fuzzing and CVE policy be required before any decoder enters the default build?
15. **Unverified claims to re-check before relying on them:**
    - jxl-oxide being slower than jxl-rs;
    - rav1d being 5–10% slower than dav1d;
    - the HEVC patent and Fedora notes (not re-checked this round);
    - CVE-2026-75466 in libjpeg-turbo's 3.2.1 ChangeLog;
    - Affinity `.af` embedded previews;
    - the local CMS timings, from a single run on one Windows machine.