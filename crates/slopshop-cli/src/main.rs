//! `slopshop`: headless command-line access to the engine.
//!
//! Exists from day one to keep the engine honest: everything it does works without the UI.

mod export;

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use slopshop_core::color::PixelFormat;
use slopshop_core::view::ViewTransform;
use slopshop_core::{Document, Edit, Layer, LayerContent, LinearRgba, Session, Size};
use slopshop_render::{Frame, Renderer};

const USAGE: &str = "\
slopshop — headless SlopShop engine

USAGE:
    slopshop gpu
        Show the GPU adapter the engine would use.

    slopshop render [--size WxH] [--doc WxH] [--out PATH]
        Render a built-in demo document (procedural fill layers), fitted into an
        output of --size, to a PNG file.
        Defaults: --size 1024x768 --doc 12000x8000 --out slopshop.png

    slopshop export <INPUT> <OUTPUT> [--format png|tiff|exr|jpeg|webp]
                    [--depth u8|u16|f16|f32] [--space ID]
                    [--compression fast|small|none|deflate|lzw|lossy|lossless]
                    [--quality 0-100] [--subsampling 444|422|420]
                    [--no-alpha] [--matte RRGGBB] [--no-dither] [--cpu] [--bench]
        Open an image file as a one-layer document and export it to PNG, TIFF,
        OpenEXR, JPEG or WebP with the format's default settings (ADR 0008, 0010);
        each option overrides one of them. Prints the settings used and the export
        report.
        --format       Default: from the OUTPUT extension (.png, .tif, .tiff, .exr,
                       .jpg, .jpeg, .webp).
        --depth        PNG: u8, u16. TIFF: u8, u16, f32. OpenEXR: f16, f32. JPEG and
                       WebP: u8.
                       The color space stays the default one unless --space is given.
        --space        srgb, linear-srgb, display-p3, adobe-rgb, prophoto, rec2020,
                       linear-rec2020, rec2100-pq, rec2100-hlg. TIFF, JPEG and WebP:
                       all but PQ and HLG. OpenEXR: linear-srgb, linear-rec2020.
        --compression  PNG: fast, small. TIFF: none, deflate, lzw. OpenEXR: always
                       lossless ZIP, no option. JPEG: see --quality, --subsampling.
                       WebP: lossy (default), lossless.
        --quality      JPEG 1 to 100, lossy WebP 0 to 100. Default: 90.
        --subsampling  JPEG chroma subsampling: 444 (full color), 422, 420 (smallest).
                       Default: 444.
        --no-alpha     Drop the alpha channel: the image is flattened over the matte.
                       JPEG has no alpha: it is always flattened.
        --matte        The color transparency is flattened over when alpha is dropped,
                       as sRGB RRGGBB. Default: ffffff (white).
        --no-dither    No dither for 8-bit samples.
        --cpu          Composite on the CPU instead of the GPU (also used when no
                       GPU is available).
        --bench        Also print timings (open, GPU init, export, throughput, time
                       in the pixel source) and the number of bands.

    slopshop --help | --version
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("gpu") => gpu(),
        Some("render") => render(&args[1..]),
        Some("export") => export::run(&args[1..]),
        Some("--version" | "-V") => {
            println!("slopshop {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("--help" | "-h") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn gpu() -> Result<(), String> {
    let renderer = Renderer::new().map_err(|e| e.to_string())?;
    let a = renderer.adapter_summary();
    println!("adapter: {}", a.name);
    println!("backend: {}", a.backend);
    println!("type:    {}", a.device_type);
    println!("driver:  {}", a.driver);
    Ok(())
}

fn render(args: &[String]) -> Result<(), String> {
    let mut output = Size::new(1024, 768);
    let mut doc_size = Size::new(12_000, 8_000);
    let mut out = PathBuf::from("slopshop.png");

    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("missing value for {flag}"));
        match flag.as_str() {
            "--size" => output = parse_size(value()?)?,
            "--doc" => doc_size = parse_size(value()?)?,
            "--out" => out = PathBuf::from(value()?),
            other => return Err(format!("unknown option `{other}`")),
        }
    }

    let session = demo_document(doc_size)?;
    let renderer = Renderer::new().map_err(|e| e.to_string())?;
    let view = ViewTransform::fit(session.document().size(), output, 16);
    let frame = renderer
        .render_view(session.document(), view, output)
        .map_err(|e| e.to_string())?;
    write_png(&frame, &out)?;
    println!(
        "rendered {}x{} document to {} ({}x{})",
        doc_size.width,
        doc_size.height,
        out.display(),
        output.width,
        output.height
    );
    Ok(())
}

fn parse_size(s: &str) -> Result<Size, String> {
    let (w, h) = s
        .split_once('x')
        .ok_or(format!("invalid size `{s}`, expected WxH"))?;
    let parse = |v: &str| {
        v.parse::<u32>()
            .ok()
            .filter(|&n| n > 0)
            .ok_or(format!("invalid size `{s}`, expected positive integers"))
    };
    Ok(Size::new(parse(w)?, parse(h)?))
}

/// A document built through the regular edit path, like the app does.
fn demo_document(size: Size) -> Result<Session, String> {
    let mut session = Session::new(Document::new(size));
    let layers = [
        (
            "Background",
            LinearRgba::from_srgb_encoded_to_working(0.10, 0.12, 0.16, 1.0),
            1.0,
        ),
        (
            "Slop",
            LinearRgba::from_srgb_encoded_to_working(0.91, 0.30, 0.64, 1.0),
            0.6,
        ),
    ];
    for (name, color, opacity) in layers {
        let id = session.allocate_layer_id();
        let index = session.document().layers().len();
        session
            .perform(Edit::InsertLayer {
                index,
                layer: Layer {
                    id,
                    name: name.into(),
                    visible: true,
                    opacity,
                    content: LayerContent::Fill { color },
                },
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(session)
}

fn write_png(frame: &Frame, path: &Path) -> Result<(), String> {
    // PNG output is written exactly as rendered: refuse anything else rather than convert.
    if frame.format != PixelFormat::RGBA8_SRGB {
        return Err(format!("cannot write {:?} as 8-bit sRGB PNG", frame.format));
    }
    let file = File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut encoder = png::Encoder::new(BufWriter::new(file), frame.size.width, frame.size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    // Tag the color space explicitly instead of letting readers guess.
    encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer
        .write_image_data(&frame.data)
        .map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The user documentation of the CLI (docs/cli.md) must cover every option of the usage.
    #[test]
    fn every_option_is_documented() {
        let docs = include_str!("../../../docs/cli.md");
        let options: Vec<&str> = USAGE
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .filter(|word| word.starts_with("--") && word.len() > 2)
            .collect();
        assert!(options.contains(&"--matte"), "{options:?}");
        for option in options {
            assert!(
                docs.contains(option),
                "{option} is missing from docs/cli.md"
            );
        }
        for command in ["slopshop gpu", "slopshop render", "slopshop export"] {
            assert!(docs.contains(command), "{command}");
        }
    }

    #[test]
    fn parses_sizes() {
        assert_eq!(parse_size("640x480"), Ok(Size::new(640, 480)));
        assert!(parse_size("640").is_err());
        assert!(parse_size("0x10").is_err());
        assert!(parse_size("-1x10").is_err());
    }

    #[test]
    fn demo_document_is_undoable() {
        let mut s = demo_document(Size::new(100, 100)).unwrap();
        assert_eq!(s.document().layers().len(), 2);
        while s.undo().unwrap() {}
        assert!(s.document().layers().is_empty());
    }
}
