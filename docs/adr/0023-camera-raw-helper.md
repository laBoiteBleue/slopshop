# 0023 — Camera RAW through a separate helper process

Status: accepted (2026-10-01, maintainer's choice: "rawler dans un processus séparé", then
"raw basique et on s'arrête là").

## Context

Camera RAW files (CR2, CR3, NEF, ARW, RAF, ORF, RW2, DNG and dozens more) need a decoder per
manufacturer and per camera. Every maintained Rust decoder (rawler, rawloader) is LGPL-2.1, and
so are the C/C++ libraries (LibRaw is LGPL or CDDL). ADR 0006 allows LGPL only isolated:
behind a feature, dynamically linked or in a worker process, and replaceable by the user. The
best known demosaicing algorithms (AMaZE, RCD, as shipped by RawTherapee and darktable) are
GPL and cannot be used at all.

A complete development module (exposure, white balance, highlights, noise, lens corrections,
profiles: Photoshop's Camera Raw) is a product of its own; the maintainer asked for a basic
import first.

## Decision

- **A separate executable, `slopshop-raw`** (crate `crates/slopshop-raw`), the only crate that
  depends on rawler. The importer (`slopshop_io::raw`) runs it once per RAW file with the
  file's path and reads the developed image from its standard output; it never links rawler.
  `deny.toml` allows LGPL-2.1 for rawler only. A crash in a camera decoder ends the helper,
  not the editor: the import fails with the helper's message.
- **Development "as shot"**, in the helper: rawler scales the sensor values (black and white
  levels) and demosaics them (its PPG for Bayer sensors, bilinear for X-Trans: LGPL code, which
  is fine inside the helper); then our steps: the camera's as-shot white balance, normalized so
  that its smallest multiplier is 1, highlights clipped where a channel saturates (saturated
  areas turn white, as dcraw does, rather than magenta), and the camera's color matrix (D65,
  else the closest illuminant as is) to **linear Rec.2020**, the working space, without clipping
  the gamut (rawler's own calibration clips to sRGB). No tone curve: the image is scene-linear,
  1.0 being the sensor's white; the user adjusts it with adjustment layers.
- **Protocol**: `SLOPRAW1`, width, height, EXIF orientation, channel count (3, or 1 for a
  monochrome sensor), flags (no color matrix), then little-endian `f32` samples. The importer
  checks the memory budget from the header before reading the samples.
- **Finding the helper**: `SLOPSHOP_RAW_WORKER`, else next to the running executable (or one
  folder up, for test binaries). `tauri dev` and `tauri build` build it first
  (`beforeDevCommand`, `beforeBuildCommand`); the CLI finds it in the same target folder.
- RAW files are recognized by extension (many are TIFF containers), before the content.

## Alternatives

- **rawler linked into slopshop-io behind a feature**: simpler, but the editor's binaries would
  then be LGPL-bound, and a decoder panic would need catching in-process.
- **LibRaw (C++) as a dynamic library**: broader camera support and better demosaicing options,
  but a native dependency to build and ship on three platforms.
- **DNG only, in-house**: MIT code, but no proprietary formats, which is what photographers have.

## Consequences

- One more executable to build and ship. Installers (Phase 5) must bundle it (Tauri
  `externalBin`); until then a packaged app without it reports "the slopshop-raw helper was not
  found".
- The pixels cross a pipe once (a 45 MP photo is about 540 MB of floats): acceptable for an
  import, and the helper streams them in chunks.
- The development is basic: no tone curve, no highlight reconstruction, no noise reduction or
  lens corrections, four-color sensors (CYGM, RGBE) refused, and rawler's demosaicing quality.
  A non-destructive, adjustable development node is future work: it would keep the RAW data
  and re-run the helper (or a successor) with its parameters.
- There is no RAW export, as in Photoshop.
