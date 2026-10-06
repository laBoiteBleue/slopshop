**English** · [Français](README.fr.md)

# SlopShop

> A modern, open-source image editor for creating professional-grade slop — non-destructive,
> GPU-accelerated, and AI-native.

The name is a joke. The engineering is not.

![SlopShop: a photo graded with adjustment layers in a group, a filter kept as an editable entry of the layer, and the Curves editor](docs/images/screenshot-main.jpg)

> [!NOTE]
> **SlopShop is at an early stage (pre-alpha).** It already does real editing: layers,
> selections, painting and retouching, adjustments, filters, transforms, and layered PSD files.
> It has no text, shapes or generative AI yet, and some rough edges: keep copies of the files
> that matter to you, and please [report what breaks](https://github.com/laBoiteBleue/slopshop/issues/new/choose).

## Download

Installers for each version are on the
[releases page](https://github.com/laBoiteBleue/slopshop/releases).

| System                                        | Installer                   | Tested by hand |
| --------------------------------------------- | --------------------------- | -------------- |
| Windows 10 and 11 (64-bit)                    | `.exe` (or `.msi`)          | Yes            |
| macOS, Apple silicon and Intel                | `.dmg`                      | **No**         |
| Linux (x86-64): Debian/Ubuntu, Fedora, others | `.deb`, `.rpm`, `.AppImage` | **No**         |

The maintainer tests on Windows only. The macOS and Linux builds come from continuous
integration, where the engine's tests pass on both systems, but nobody has used the application
there yet: reports are very welcome, even to say that it works.

The installers are not code-signed yet, so Windows and macOS warn before the first launch. The
release notes say how to get past the warning on each system. There are no automatic updates
yet.

### System requirements

|                         | Minimum                                                                                                                                                  |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Operating system**    | Windows 10 or 11 (64-bit); macOS 10.13 or later; Linux x86-64 with WebKitGTK 4.1 (Ubuntu 22.04, Debian 12, Fedora 38 or later)                           |
| **Graphics**            | A GPU with DirectX 12 (Windows), Metal (macOS) or Vulkan (Linux) drivers; SlopShop needs only the base level of those APIs, integrated graphics included |
| **Memory**              | 8 GB of RAM; 16 GB or more for images of hundreds of megapixels                                                                                          |
| **Disk**                | About 100 MB, plus up to about 1 GB for the optional AI models                                                                                           |
| **AI tools (optional)** | Windows x64 (DirectML, any DirectX 12 GPU), macOS on Apple silicon (Core ML), Linux (on the processor). Not available on Intel Macs.                     |

## What it can do

- **Layers**: pixel layers, groups, clipping masks, layer masks, Photoshop's blend modes;
  adjustment layers (Brightness/Contrast, Levels, Curves, Exposure, Vibrance, Hue/Saturation,
  Color Balance, Black & White, Photo Filter, Channel Mixer, Invert, Posterize, Threshold,
  Gradient Map, Selective Color); solid color and gradient fill layers; layer styles (shadows,
  glows, stroke, color overlay); merging and flattening, explicit and undoable.
- **Selections**: marquees, lassos, Magic Wand, Quick Selection, Color Range; Object Selection
  and Select Subject with local AI; Select and Mask with edge refinement; Quick Mask; modify,
  grow, transform, save and load selections.
- **Painting and retouching**: Brush, Pencil, Eraser and Restore Eraser with pen pressure,
  Paint Bucket, Gradient, Clone Stamp, Healing Brush and Patch, Dodge and Burn, Blur, Sharpen
  and Smudge; Edit > Fill and Stroke.
- **Filters**: Gaussian and Motion Blur, Unsharp Mask, High Pass, Add Noise, Dust & Scratches,
  Clarity and Texture, Liquify.
- **Transforms**: Move with snapping and smart guides, Align and Distribute, Free Transform
  with Distort and Perspective, Image Size, Canvas Size, rotation, Crop with Straighten, Trim,
  rulers and guides.
- **Files**: opens more than twenty formats in their native precision (PNG, JPEG, TIFF, WebP,
  AVIF, JPEG XL, JPEG 2000, OpenEXR, Radiance HDR, DICOM, FITS, SVG, PDF, camera RAW and more),
  Photoshop PSD and PSB with their layers; exports to nearly all of them, PSD and PSB with
  layers;
  saves documents in SlopShop's own `.slop` format (lossless, incremental, crash-safe); prints.
  Details: [docs/formats.md](docs/formats.md).
- **Interface**: Photoshop's menus, tools and shortcuts, document tabs, History, Histogram and
  Info panels, English and French.
- **Headless**: a `slopshop` command renders and converts without the interface
  ([docs/cli.md](docs/cli.md)).

Not yet: text, shapes, paths and vector masks; generative AI (Remove, Generative Fill);
plugins and scripting; saving the undo history in documents; the native viewport on macOS.
The [feature map](docs/feature-map.md) lists every feature, done or not, and the
[roadmap](docs/roadmap.md) what comes next.

## Why SlopShop

- **Non-destructive all the way down.** Not only adjustment layers: brush strokes, filters and
  adjustments applied to a layer are entries in that layer's own stack, each one editable
  again, hideable and removable, over pixels that are never rewritten. Transforms are always
  resampled from the original, and crops never delete anything.
- **Fast, on the GPU.** Compositing, most filters and the marching ants run on the GPU
  (DirectX 12, Metal, Vulkan through [wgpu](https://wgpu.rs)), checked against a CPU reference. Images are
  tiled, with mip levels, so documents of hundreds of megapixels stay smooth.
- **Precise color.** 8 and 16-bit integer, 16 and 32-bit float, HDR; layers composited in
  linear light; embedded color profiles applied; no silent or lossy conversion.
- **Local, optional AI.** The AI tools run on your machine, after you agree to download each
  model and its license. No account, no cloud, no upload. They are never required to use the
  editor.
- **Familiar.** Photoshop's menus, tools and shortcuts, and layered PSD files in and out.
- **Free software, built in Rust.** A memory-safe engine that runs without the interface, an
  open document format, and the freedom to study, change and share it all.

### Where it is going

Generative models work at 1 to 2 megapixels; professional images reach 50 or 100. SlopShop's
long-term bet is to apply generative tools to such images **without lowering their
resolution**, and without touching the pixels outside the edited area. The approach is still
open and will be decided by experiment: see
[docs/research/hd-generative-ai.md](docs/research/hd-generative-ai.md).

## Screenshots

| Select Subject with local AI                                                                                                 | Free Transform in perspective                                                                          |
| ---------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| ![The subject of a photo selected by the local AI model, outlined by marching ants](docs/images/screenshot-ai-selection.jpg) | ![A layer distorted with Free Transform's perspective handles](docs/images/screenshot-perspective.jpg) |

## Building from source

Prerequisites:

- [Rust](https://rustup.rs) (stable; the exact toolchain is pinned by `rust-toolchain.toml`)
- [Node.js](https://nodejs.org) 24+ with npm
- Tauri's system dependencies for your OS: see the
  [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) (WebView2 on Windows,
  Xcode Command Line Tools on macOS, WebKitGTK and friends on Linux)
- On Windows, Visual Studio's C++ ATL component as well (Visual Studio Installer > Modify >
  Individual components > "C++ ATL for latest build tools"): the shader compiler, DXC, is
  built in and links against it ([ADR 0019](docs/adr/0019-dxc-shader-compiler.md))

Run the desktop app, or build its installers:

```sh
cd app
npm install
npm run tauri dev     # development build
npm run bundle        # installers for this system, in target/release/bundle/
```

Headless CLI:

```sh
cargo run -p slopshop-cli -- gpu
cargo run -p slopshop-cli -- export photo.jpg photo.png
```

Checks (also run by CI):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd app && npm run format:check && npm run check && npm test && npm run build
```

Under the hood: a Rust engine (a workspace of crates that runs headless), GPU rendering with
[wgpu](https://wgpu.rs), a [Tauri 2](https://tauri.app) desktop shell and a
[Svelte 5](https://svelte.dev) + TypeScript interface. The image logic lives in the engine,
never in the interface. Details: [docs/architecture.md](docs/architecture.md) and the decision
records in [docs/adr/](docs/adr/).

## Contributing

Contributions are welcome, from bug reports to code: start with
[CONTRIBUTING.md](CONTRIBUTING.md). Testing on macOS and Linux is the most valuable help right
now. Questions and ideas go to
[Discussions](https://github.com/laBoiteBleue/slopshop/discussions); security problems to
[SECURITY.md](SECURITY.md).

## Languages

The code and documentation are in English. The application is available in **English and
French** (Edit > Preferences); adding a language means adding one translation catalog (see
[ADR 0004](docs/adr/0004-ui-internationalization.md)).

## License

Copyright (C) 2026 The SlopShop contributors.

SlopShop is free software, distributed under the GNU General Public License, version 3 only
(`GPL-3.0-only`). You may use, study, modify and redistribute it under the terms of that
license; see [LICENSE](LICENSE) for the exact conditions.

The specification of the `.slop` format ([docs/file-format.md](docs/file-format.md)) is under
[CC BY 4.0](LICENSES/CC-BY-4.0.txt) and its golden files under [CC0 1.0](LICENSES/CC0-1.0.txt),
so that other software can implement the format.

Third-party dependencies keep their own licenses. The AI models and runtimes that SlopShop can
download on request are not part of SlopShop: each comes under its own license, shown before
the download.

The name "SlopShop" and the project's logo are not licensed under the GPL: any rights in the
name and the branding are separate from the license of the code.

The photos in the screenshots are in the public domain (CC0), from Wikimedia Commons:
[San Juan Valley](https://commons.wikimedia.org/wiki/File:San_Juan_Valley.jpg) by Wilfredor,
[Lotus flower](<https://commons.wikimedia.org/wiki/File:Lotus_flower_(978659).jpg>) by Hong
Zhang, and
[Scuol-Motta Naluns](<https://commons.wikimedia.org/wiki/File:Scuol-Motta_Naluns,_15-09-2023._(actm.)_09.jpg>)
by Agnes Monkelbaan.

SlopShop is an independent project, not affiliated with, endorsed by or sponsored by Adobe.
Adobe and Photoshop are either registered trademarks or trademarks of Adobe in the United
States and/or other countries.
