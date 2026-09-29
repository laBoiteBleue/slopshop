//! `slopshop export`: an image file, opened as a one-layer document, written to PNG, TIFF or
//! OpenEXR by the export pipeline (ADR 0008), exactly as a front end would drive it: the
//! format's default settings, explicit overrides, the GPU renderer as the pixel source (the
//! CPU compositor without a GPU), and the export report.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use slopshop_core::color::{ColorSpace, SampleType};
use slopshop_core::{CancelToken, Document, Edit, Layer, LayerContent, RasterImage, Rect, Size};
use slopshop_io::export::{
    ExportFormat, ExportFormatKind, ExportReport, ExportSpec, ExrSample, PngCompression, PngDepth,
    TiffCompression, TiffSample, default_spec, export_image, supports_space,
};
use slopshop_render::Renderer;

/// The color spaces `--space` accepts, by their id ([`ColorSpace::id`]).
const SPACES: [ColorSpace; 9] = [
    ColorSpace::SRGB,
    ColorSpace::LINEAR_SRGB,
    ColorSpace::DISPLAY_P3,
    ColorSpace::ADOBE_RGB,
    ColorSpace::PROPHOTO,
    ColorSpace::REC2020,
    ColorSpace::LINEAR_REC2020,
    ColorSpace::REC2100_PQ,
    ColorSpace::REC2100_HLG,
];

const FORMATS: [ExportFormatKind; 3] = [
    ExportFormatKind::Png,
    ExportFormatKind::Tiff,
    ExportFormatKind::Exr,
];

/// `--depth`: the sample type, whatever the format (each format accepts some of them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Depth {
    U8,
    U16,
    F16,
    F32,
}

impl Depth {
    const ALL: [Depth; 4] = [Depth::U8, Depth::U16, Depth::F16, Depth::F32];

    fn name(self) -> &'static str {
        match self {
            Depth::U8 => "u8",
            Depth::U16 => "u16",
            Depth::F16 => "f16",
            Depth::F32 => "f32",
        }
    }

    fn png(self) -> Option<PngDepth> {
        match self {
            Depth::U8 => Some(PngDepth::U8),
            Depth::U16 => Some(PngDepth::U16),
            Depth::F16 | Depth::F32 => None,
        }
    }

    fn tiff(self) -> Option<TiffSample> {
        match self {
            Depth::U8 => Some(TiffSample::U8),
            Depth::U16 => Some(TiffSample::U16),
            Depth::F32 => Some(TiffSample::F32),
            Depth::F16 => None,
        }
    }

    fn exr(self) -> Option<ExrSample> {
        match self {
            Depth::F16 => Some(ExrSample::F16),
            Depth::F32 => Some(ExrSample::F32),
            Depth::U8 | Depth::U16 => None,
        }
    }

    fn supported_by(self, kind: ExportFormatKind) -> bool {
        match kind {
            ExportFormatKind::Png => self.png().is_some(),
            ExportFormatKind::Tiff => self.tiff().is_some(),
            ExportFormatKind::Exr => self.exr().is_some(),
        }
    }
}

/// `--compression`: the compression settings of every format (EXR has none to choose).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    Fast,
    Small,
    None,
    Deflate,
    Lzw,
}

impl Compression {
    const ALL: [Compression; 5] = [
        Compression::Fast,
        Compression::Small,
        Compression::None,
        Compression::Deflate,
        Compression::Lzw,
    ];

    fn name(self) -> &'static str {
        match self {
            Compression::Fast => "fast",
            Compression::Small => "small",
            Compression::None => "none",
            Compression::Deflate => "deflate",
            Compression::Lzw => "lzw",
        }
    }

    fn png(self) -> Option<PngCompression> {
        match self {
            Compression::Fast => Some(PngCompression::Fast),
            Compression::Small => Some(PngCompression::Small),
            Compression::None | Compression::Deflate | Compression::Lzw => None,
        }
    }

    fn tiff(self) -> Option<TiffCompression> {
        match self {
            Compression::None => Some(TiffCompression::None),
            Compression::Deflate => Some(TiffCompression::Deflate),
            Compression::Lzw => Some(TiffCompression::Lzw),
            Compression::Fast | Compression::Small => None,
        }
    }

    fn supported_by(self, kind: ExportFormatKind) -> bool {
        match kind {
            ExportFormatKind::Png => self.png().is_some(),
            ExportFormatKind::Tiff => self.tiff().is_some(),
            ExportFormatKind::Exr => false,
        }
    }
}

/// The command line of `slopshop export`, validated: every option is valid for `format`.
#[derive(Debug, Clone, PartialEq)]
struct Args {
    input: PathBuf,
    output: PathBuf,
    format: ExportFormatKind,
    depth: Option<Depth>,
    space: Option<ColorSpace>,
    compression: Option<Compression>,
    no_alpha: bool,
    no_dither: bool,
    cpu: bool,
    bench: bool,
}

/// Where the pixels came from.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    Gpu {
        adapter: String,
    },
    /// `--cpu`.
    CpuRequested,
    /// No usable GPU: why.
    CpuFallback {
        reason: String,
    },
}

/// What an export did, for printing.
#[derive(Debug)]
struct Outcome {
    size: Size,
    spec: ExportSpec,
    source: Source,
    import_warnings: Vec<&'static str>,
    report: ExportReport,
    /// Bands produced and written (one progress call each).
    bands: u64,
    open: Duration,
    /// Creating the renderer; zero with `--cpu`.
    gpu_init: Duration,
    /// `export_image`, from the call to the renamed file.
    export: Duration,
    /// Time spent in the pixel source (rendering), which overlaps conversion and encoding.
    source_time: Duration,
}

/// `slopshop export …`: run and print the result.
pub fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;
    let outcome = export(&args)?;

    for id in &outcome.import_warnings {
        println!("import warning: {id}");
    }
    if let Source::CpuFallback { reason } = &outcome.source {
        println!("note: no usable GPU ({reason}), using the CPU compositor");
    }
    println!(
        "exported {}x{} {} to {}",
        outcome.size.width,
        outcome.size.height,
        args.input.display(),
        args.output.display()
    );
    println!("settings: {}", describe(&outcome.spec));
    println!(
        "source: {}",
        match &outcome.source {
            Source::Gpu { adapter } => format!("GPU ({adapter})"),
            Source::CpuRequested | Source::CpuFallback { .. } => "CPU".to_owned(),
        }
    );
    // Stable ids and numbers, as the engine reports them (the app translates them).
    for notice in &outcome.report.notices {
        match notice.count() {
            Some(count) => println!("report: {} ({count} samples)", notice.id()),
            None => println!("report: {}", notice.id()),
        }
    }
    if args.bench {
        let seconds = outcome.export.as_secs_f64();
        let megapixels = outcome.size.pixel_count() as f64 / 1e6;
        println!("bench: open      {:.3} s", outcome.open.as_secs_f64());
        if outcome.source != Source::CpuRequested {
            println!("bench: gpu init  {:.3} s", outcome.gpu_init.as_secs_f64());
        }
        println!(
            "bench: export    {seconds:.3} s ({:.1} MP/s, {} bands of up to {} rows)",
            megapixels / seconds.max(f64::MIN_POSITIVE),
            outcome.bands,
            slopshop_io::export::BAND_ROWS
        );
        println!(
            "bench: source    {:.3} s (overlaps conversion and encoding)",
            outcome.source_time.as_secs_f64()
        );
    }
    Ok(())
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut paths = Vec::new();
    let mut format = None;
    let mut depth = None;
    let mut space = None;
    let mut compression = None;
    let (mut no_alpha, mut no_dither, mut cpu, mut bench) = (false, false, false, false);

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("missing value for {arg}"));
        match arg.as_str() {
            "--format" => format = Some(parse_format(value()?)?),
            "--depth" => depth = Some(parse_depth(value()?)?),
            "--space" => space = Some(parse_space(value()?)?),
            "--compression" => compression = Some(parse_compression(value()?)?),
            "--no-alpha" => no_alpha = true,
            "--no-dither" => no_dither = true,
            "--cpu" => cpu = true,
            "--bench" => bench = true,
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option `{other}`"));
            }
            path => paths.push(PathBuf::from(path)),
        }
    }
    let [input, output] = <[PathBuf; 2]>::try_from(paths).map_err(|paths| {
        format!(
            "expected an input and an output path, got {} path(s)",
            paths.len()
        )
    })?;

    let format = match format {
        Some(format) => format,
        None => format_of(&output).ok_or(format!(
            "cannot tell the format from `{}`: use --format png|tiff|exr",
            output.display()
        ))?,
    };
    let name = format_name(format);
    if let Some(depth) = depth.filter(|d| !d.supported_by(format)) {
        let valid: Vec<_> = Depth::ALL
            .iter()
            .filter(|d| d.supported_by(format))
            .map(|d| d.name())
            .collect();
        return Err(format!(
            "--depth {} is not available for {name} (valid: {})",
            depth.name(),
            valid.join(", ")
        ));
    }
    if let Some(compression) = compression.filter(|c| !c.supported_by(format)) {
        let valid: Vec<_> = Compression::ALL
            .iter()
            .filter(|c| c.supported_by(format))
            .map(|c| c.name())
            .collect();
        return Err(if valid.is_empty() {
            format!("{name} has no --compression option (it always uses lossless ZIP)")
        } else {
            format!(
                "--compression {} is not available for {name} (valid: {})",
                compression.name(),
                valid.join(", ")
            )
        });
    }
    if let Some(space) = space.filter(|s| !supports_space(format, s)) {
        let valid: Vec<_> = SPACES
            .iter()
            .filter(|s| supports_space(format, s))
            .filter_map(ColorSpace::id)
            .collect();
        return Err(format!(
            "{name} cannot store and tag the color space `{}` (valid: {})",
            space_name(&space),
            valid.join(", ")
        ));
    }
    Ok(Args {
        input,
        output,
        format,
        depth,
        space,
        compression,
        no_alpha,
        no_dither,
        cpu,
        bench,
    })
}

fn parse_format(s: &str) -> Result<ExportFormatKind, String> {
    FORMATS
        .into_iter()
        .find(|kind| format_id(*kind) == s)
        .ok_or(format!("invalid format `{s}`, expected png, tiff or exr"))
}

fn parse_depth(s: &str) -> Result<Depth, String> {
    Depth::ALL
        .into_iter()
        .find(|d| d.name() == s)
        .ok_or(format!("invalid depth `{s}`, expected u8, u16, f16 or f32"))
}

fn parse_compression(s: &str) -> Result<Compression, String> {
    Compression::ALL
        .into_iter()
        .find(|c| c.name() == s)
        .ok_or(format!(
            "invalid compression `{s}`, expected fast, small, none, deflate or lzw"
        ))
}

fn parse_space(s: &str) -> Result<ColorSpace, String> {
    SPACES
        .into_iter()
        .find(|space| space.id() == Some(s))
        .ok_or_else(|| {
            let ids: Vec<_> = SPACES.iter().filter_map(ColorSpace::id).collect();
            format!(
                "unknown color space `{s}`, expected one of: {}",
                ids.join(", ")
            )
        })
}

/// The format a file name asks for, from its extension.
fn format_of(path: &Path) -> Option<ExportFormatKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some(ExportFormatKind::Png),
        "tif" | "tiff" => Some(ExportFormatKind::Tiff),
        "exr" => Some(ExportFormatKind::Exr),
        _ => None,
    }
}

/// As `--format` spells it.
fn format_id(kind: ExportFormatKind) -> &'static str {
    match kind {
        ExportFormatKind::Png => "png",
        ExportFormatKind::Tiff => "tiff",
        ExportFormatKind::Exr => "exr",
    }
}

fn format_name(kind: ExportFormatKind) -> &'static str {
    match kind {
        ExportFormatKind::Png => "PNG",
        ExportFormatKind::Tiff => "TIFF",
        ExportFormatKind::Exr => "OpenEXR",
    }
}

/// A space's id, or a description for spaces without one (e.g. from an ICC profile).
fn space_name(space: &ColorSpace) -> String {
    match space.id() {
        Some(id) => id.to_owned(),
        None => format!("{space:?}"),
    }
}

/// The defaults of the format for `document` (ADR 0008), with the options given overriding
/// them. Every option was validated for the format by [`parse_args`].
fn export_spec(args: &Args, document: &Document) -> ExportSpec {
    let mut spec = default_spec(args.format, document);
    spec.format = match spec.format {
        ExportFormat::Png { depth, compression } => ExportFormat::Png {
            depth: args.depth.and_then(Depth::png).unwrap_or(depth),
            compression: args
                .compression
                .and_then(Compression::png)
                .unwrap_or(compression),
        },
        ExportFormat::Tiff {
            sample,
            compression,
        } => ExportFormat::Tiff {
            sample: args.depth.and_then(Depth::tiff).unwrap_or(sample),
            compression: args
                .compression
                .and_then(Compression::tiff)
                .unwrap_or(compression),
        },
        ExportFormat::Exr { sample } => ExportFormat::Exr {
            sample: args.depth.and_then(Depth::exr).unwrap_or(sample),
        },
    };
    if let Some(space) = args.space {
        spec.space = space;
    }
    if args.no_alpha {
        spec.keep_alpha = false;
    }
    if args.no_dither {
        spec.dither = false;
    }
    spec
}

/// The settings, as `--` options would give them.
fn describe(spec: &ExportSpec) -> String {
    let (depth, compression) = match spec.format {
        ExportFormat::Png { depth, compression } => (
            match depth {
                PngDepth::U8 => Depth::U8,
                PngDepth::U16 => Depth::U16,
            },
            Some(match compression {
                PngCompression::Fast => Compression::Fast,
                PngCompression::Small => Compression::Small,
            }),
        ),
        ExportFormat::Tiff {
            sample,
            compression,
        } => (
            match sample {
                TiffSample::U8 => Depth::U8,
                TiffSample::U16 => Depth::U16,
                TiffSample::F32 => Depth::F32,
            },
            Some(match compression {
                TiffCompression::None => Compression::None,
                TiffCompression::Deflate => Compression::Deflate,
                TiffCompression::Lzw => Compression::Lzw,
            }),
        ),
        ExportFormat::Exr { sample } => (
            match sample {
                ExrSample::F16 => Depth::F16,
                ExrSample::F32 => Depth::F32,
            },
            None,
        ),
    };
    let mut text = format!(
        "--format {} --depth {} --space {}",
        format_id(spec.format.kind()),
        depth.name(),
        space_name(&spec.space)
    );
    if let Some(compression) = compression {
        text += &format!(" --compression {}", compression.name());
    }
    if !spec.keep_alpha {
        text += " --no-alpha";
    }
    // Dither only applies to 8-bit samples.
    if !spec.dither && spec.format.sample_type() == SampleType::U8 {
        text += " --no-dither";
    }
    text
}

/// Open, build the document, export.
fn export(args: &Args) -> Result<Outcome, String> {
    let started = Instant::now();
    let imported = slopshop_io::open_image(&args.input)
        .map_err(|e| format!("cannot open {}: {e}", args.input.display()))?;
    let open = started.elapsed();
    let import_warnings = imported.warnings.iter().map(|w| w.id()).collect();
    let document = single_layer_document(imported.image, &layer_name(&args.input))?;
    let spec = export_spec(args, &document);

    let started = Instant::now();
    let (renderer, source) = if args.cpu {
        (None, Source::CpuRequested)
    } else {
        match Renderer::new() {
            Ok(renderer) => {
                let adapter = renderer.adapter_summary().name;
                (Some(renderer), Source::Gpu { adapter })
            }
            Err(e) => (
                None,
                Source::CpuFallback {
                    reason: e.to_string(),
                },
            ),
        }
    };
    let gpu_init = started.elapsed();

    let mut source_time = Duration::ZERO;
    let mut render = slopshop_render::export_source(renderer.as_ref(), &document);
    let timed = |region: Rect, out: &mut [f32]| {
        let started = Instant::now();
        let result = render(region, out);
        source_time += started.elapsed();
        result
    };
    let mut bands = 0;
    let started = Instant::now();
    let report = export_image(
        &args.output,
        document.size(),
        &spec,
        timed,
        &CancelToken::new(),
        &mut |_| bands += 1,
    )
    .map_err(|e| {
        format!(
            "cannot export to {} ({}): {e}",
            args.output.display(),
            e.code()
        )
    })?;
    let export = started.elapsed();

    Ok(Outcome {
        size: document.size(),
        spec,
        source,
        import_warnings,
        report,
        bands,
        open,
        gpu_init,
        export,
        source_time,
    })
}

/// A document of the image's size holding the image as its only layer.
fn single_layer_document(image: RasterImage, name: &str) -> Result<Document, String> {
    let mut document = Document::new(image.size());
    let id = document.allocate_layer_id();
    let edit = Edit::InsertLayer {
        index: 0,
        layer: Layer {
            id,
            name: name.to_owned(),
            visible: true,
            opacity: 1.0,
            content: LayerContent::Raster {
                image: Arc::new(image),
            },
        },
    };
    // Initial content: applied directly, there is no history to record it in.
    edit.apply(&mut document).map_err(|e| e.to_string())?;
    Ok(document)
}

fn layer_name(path: &Path) -> String {
    path.file_stem()
        .map_or_else(|| "Image".to_owned(), |s| s.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::io::BufWriter;

    use slopshop_core::composite::composite_region;

    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    fn parse(args: &[&str]) -> Result<Args, String> {
        parse_args(&strings(args))
    }

    #[test]
    fn parses_paths_and_options_in_any_order() {
        let args = parse(&[
            "--depth",
            "u16",
            "in.jpg",
            "--no-alpha",
            "out.PNG",
            "--space",
            "display-p3",
            "--compression",
            "small",
            "--no-dither",
            "--cpu",
            "--bench",
        ])
        .unwrap();
        assert_eq!(
            args,
            Args {
                input: "in.jpg".into(),
                output: "out.PNG".into(),
                format: ExportFormatKind::Png,
                depth: Some(Depth::U16),
                space: Some(ColorSpace::DISPLAY_P3),
                compression: Some(Compression::Small),
                no_alpha: true,
                no_dither: true,
                cpu: true,
                bench: true,
            }
        );
        let args = parse(&["in.png", "out.tif"]).unwrap();
        assert_eq!(args.format, ExportFormatKind::Tiff);
        assert_eq!(
            (args.depth, args.space, args.compression),
            (None, None, None)
        );
        assert!(!args.no_alpha && !args.no_dither && !args.cpu && !args.bench);
        assert_eq!(
            parse(&["in.png", "out.tiff"]).unwrap().format,
            ExportFormatKind::Tiff
        );
        assert_eq!(
            parse(&["in.png", "out.exr"]).unwrap().format,
            ExportFormatKind::Exr
        );
    }

    #[test]
    fn format_option_overrides_the_extension() {
        let args = parse(&["in.png", "out.bin", "--format", "exr"]).unwrap();
        assert_eq!(args.format, ExportFormatKind::Exr);
        let args = parse(&["in.png", "out.png", "--format", "tiff"]).unwrap();
        assert_eq!(args.format, ExportFormatKind::Tiff);
    }

    #[test]
    fn rejects_malformed_command_lines() {
        let error = |args: &[&str]| parse(args).unwrap_err();
        assert!(error(&["in.png"]).contains("got 1 path"));
        assert!(error(&["a.png", "b.png", "c.png"]).contains("got 3 path"));
        assert!(error(&["in.png", "out.jpg"]).contains("--format"));
        assert!(error(&["in.png", "out"]).contains("--format"));
        assert!(error(&["in.png", "out.png", "--depth"]).contains("missing value"));
        assert!(error(&["in.png", "out.png", "--depth", "u12"]).contains("invalid depth"));
        assert!(error(&["in.png", "out.png", "--format", "jpeg"]).contains("invalid format"));
        assert!(error(&["in.png", "out.png", "--space", "cmyk"]).contains("unknown color space"));
        assert!(error(&["in.png", "out.png", "--compression", "zip"]).contains("invalid"));
        assert!(error(&["in.png", "out.png", "--alpha"]).contains("unknown option `--alpha`"));
    }

    #[test]
    fn rejects_options_the_format_does_not_have() {
        let error = |args: &[&str]| parse(args).unwrap_err();
        assert_eq!(
            error(&["in.png", "out.png", "--depth", "f32"]),
            "--depth f32 is not available for PNG (valid: u8, u16)"
        );
        assert_eq!(
            error(&["in.png", "out.tif", "--depth", "f16"]),
            "--depth f16 is not available for TIFF (valid: u8, u16, f32)"
        );
        assert_eq!(
            error(&["in.png", "out.exr", "--depth", "u16"]),
            "--depth u16 is not available for OpenEXR (valid: f16, f32)"
        );
        assert_eq!(
            error(&["in.png", "out.png", "--compression", "lzw"]),
            "--compression lzw is not available for PNG (valid: fast, small)"
        );
        assert_eq!(
            error(&["in.png", "out.tif", "--compression", "fast"]),
            "--compression fast is not available for TIFF (valid: none, deflate, lzw)"
        );
        assert!(
            error(&["in.png", "out.exr", "--compression", "none"]).contains("no --compression")
        );
    }

    #[test]
    fn rejects_color_spaces_the_format_cannot_tag() {
        let error = |args: &[&str]| parse(args).unwrap_err();
        // EXR data is scene-linear.
        assert_eq!(
            error(&["in.png", "out.exr", "--space", "srgb"]),
            "OpenEXR cannot store and tag the color space `srgb` (valid: linear-srgb, \
             linear-rec2020)"
        );
        // TIFF tags with ICC, which cannot describe PQ or HLG.
        assert!(error(&["in.png", "out.tif", "--space", "rec2100-pq"]).contains("rec2100-pq"));
        assert!(parse(&["in.png", "out.tif", "--space", "prophoto"]).is_ok());
        // PNG can tag every space of the list (sRGB chunk, cICP, iCCP).
        for space in SPACES {
            let id = space.id().unwrap();
            assert_eq!(
                parse(&["in.png", "out.png", "--space", id]).unwrap().space,
                Some(space)
            );
        }
    }

    #[test]
    fn options_override_the_format_defaults() {
        let size = Size::new(4, 4);
        let pixels = vec![128u8; 4 * 4 * 4];
        let image =
            RasterImage::from_pixels(size, slopshop_core::color::PixelFormat::RGBA8_SRGB, &pixels)
                .unwrap();
        let document = single_layer_document(image, "layer").unwrap();

        let args = parse(&["in.png", "out.png"]).unwrap();
        let spec = export_spec(&args, &document);
        assert_eq!(spec, default_spec(ExportFormatKind::Png, &document));
        assert_eq!(
            describe(&spec),
            "--format png --depth u8 --space srgb --compression fast"
        );

        let args = parse(&[
            "in.png",
            "out.tif",
            "--depth",
            "f32",
            "--space",
            "linear-rec2020",
            "--compression",
            "lzw",
            "--no-alpha",
            "--no-dither",
        ])
        .unwrap();
        let spec = export_spec(&args, &document);
        assert_eq!(
            spec.format,
            ExportFormat::Tiff {
                sample: TiffSample::F32,
                compression: TiffCompression::Lzw
            }
        );
        assert_eq!(spec.space, ColorSpace::LINEAR_REC2020);
        assert!(!spec.keep_alpha && !spec.dither);
        // Dither is not shown for float samples, which never have it.
        assert_eq!(
            describe(&spec),
            "--format tiff --depth f32 --space linear-rec2020 --compression lzw --no-alpha"
        );
        let args = parse(&["in.png", "out.tif", "--no-dither"]).unwrap();
        assert_eq!(
            describe(&export_spec(&args, &document)),
            "--format tiff --depth u8 --space srgb --compression deflate --no-dither"
        );

        let args = parse(&["in.png", "out.exr", "--depth", "f16"]).unwrap();
        let spec = export_spec(&args, &document);
        assert_eq!(
            spec.format,
            ExportFormat::Exr {
                sample: ExrSample::F16
            }
        );
        assert_eq!(
            describe(&spec),
            "--format exr --depth f16 --space linear-srgb"
        );
    }

    /// A fresh directory for one test, removed by the test when it succeeds.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("slopshop-cli-{}-{name}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// An 8-bit sRGB RGBA PNG taller than one band, with every alpha above 0 (color under
    /// alpha 0 is not kept, by design).
    fn write_test_png(path: &Path, size: Size) {
        let mut pixels = Vec::new();
        for y in 0..size.height {
            for x in 0..size.width {
                pixels.extend([
                    (x * 7 + y) as u8,
                    (y * 3) as u8,
                    (x ^ y) as u8,
                    (x + y * 5) as u8 | 1,
                ]);
            }
        }
        let file = BufWriter::new(File::create(path).unwrap());
        let mut encoder = png::Encoder::new(file, size.width, size.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
        writer.finish().unwrap();
    }

    /// The whole composited image, premultiplied RGBA f32 in the working space.
    fn composite(document: &Document) -> Vec<f32> {
        let size = document.size();
        let mut out = vec![0.0; size.pixel_count() as usize * 4];
        composite_region(document, size.bounds(), &mut out).unwrap();
        out
    }

    fn max_difference(a: &[f32], b: &[f32]) -> f32 {
        assert_eq!(a.len(), b.len());
        a.iter()
            .zip(b)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max)
    }

    /// Export a generated PNG to each format, re-import the result and compare the pixels.
    #[test]
    fn exports_png_to_png_tiff_and_exr_and_reads_them_back() {
        let dir = temp_dir("round-trip");
        let size = Size::new(70, 300);
        let input = dir.join("in.png");
        write_test_png(&input, size);
        let original = slopshop_io::open_image(&input).unwrap();
        let expected = composite(&single_layer_document(original.image, "in").unwrap());

        // (output, extra options, sample type read back, largest difference allowed)
        let cases: [(&str, &[&str], SampleType, f32); 6] = [
            // Unedited 8-bit sources export bit-exact, dither or not.
            ("out.png", &[], SampleType::U8, 0.0),
            ("out.tif", &[], SampleType::U8, 0.0),
            ("lzw.tif", &["--compression", "lzw"], SampleType::U8, 0.0),
            ("16.png", &["--depth", "u16"], SampleType::U16, 1e-4),
            // Float: exact but for the matrices to linear sRGB and back.
            ("out.exr", &[], SampleType::F32, 1e-5),
            // Half floats are read back as 32-bit floats by the importer.
            ("half.exr", &["--depth", "f16"], SampleType::F32, 1e-3),
        ];
        for (name, options, sample, tolerance) in cases {
            let output = dir.join(name);
            let mut args = vec![input.to_str().unwrap(), output.to_str().unwrap(), "--cpu"];
            args.extend(options);
            let outcome = export(&parse(&args).unwrap()).unwrap();
            assert_eq!(outcome.source, Source::CpuRequested);
            assert_eq!(outcome.size, size);
            // 300 rows: two bands.
            assert_eq!(outcome.bands, 2, "{name}");
            let half = options.contains(&"f16");
            assert_eq!(
                outcome
                    .report
                    .notices
                    .iter()
                    .map(|n| n.id())
                    .collect::<Vec<_>>(),
                if half {
                    vec!["precisionReduced"]
                } else {
                    vec![]
                },
                "{name}"
            );

            let reimported = slopshop_io::open_image(&output).unwrap();
            let format = reimported.image.format();
            assert_eq!(format.sample, sample, "{name}");
            assert_eq!(format.color_space, outcome.spec.space, "{name}");
            let actual = composite(&single_layer_document(reimported.image, name).unwrap());
            let difference = max_difference(&expected, &actual);
            assert!(difference <= tolerance, "{name}: {difference}");
        }
        fs::remove_dir_all(&dir).ok();
    }

    /// The GPU is the default source; the CPU compositor replaces it without an adapter
    /// (except in CI, where `SLOPSHOP_REQUIRE_GPU=1` makes the fallback an error).
    #[test]
    fn exports_with_the_gpu_when_there_is_one() {
        let dir = temp_dir("gpu");
        let size = Size::new(300, 20);
        let input = dir.join("in.png");
        write_test_png(&input, size);
        let output = dir.join("out.png");
        let args = [input.to_str().unwrap(), output.to_str().unwrap(), "--bench"];
        let outcome = export(&parse(&args).unwrap()).unwrap();
        match &outcome.source {
            Source::Gpu { .. } => {}
            Source::CpuFallback { reason }
                if std::env::var("SLOPSHOP_REQUIRE_GPU").as_deref() == Ok("1") =>
            {
                panic!("GPU required but unavailable: {reason}")
            }
            other => eprintln!("no GPU, exported with {other:?}"),
        }
        assert_eq!(outcome.bands, 1);
        assert_eq!(outcome.report, ExportReport::default());

        let original = slopshop_io::open_image(&input).unwrap();
        let reimported = slopshop_io::open_image(&output).unwrap();
        let expected = composite(&single_layer_document(original.image, "in").unwrap());
        let actual = composite(&single_layer_document(reimported.image, "out").unwrap());
        assert_eq!(max_difference(&expected, &actual), 0.0);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn failures_name_the_file_and_leave_nothing_behind() {
        let dir = temp_dir("failures");
        let missing = dir.join("missing.png");
        let output = dir.join("out.png");
        let args = [missing.to_str().unwrap(), output.to_str().unwrap(), "--cpu"];
        let error = export(&parse(&args).unwrap()).unwrap_err();
        assert!(error.contains("missing.png"), "{error}");
        assert!(!output.exists());

        // The output directory does not exist.
        let input = dir.join("in.png");
        write_test_png(&input, Size::new(8, 8));
        let output = dir.join("no-such-dir").join("out.tif");
        let args = [input.to_str().unwrap(), output.to_str().unwrap(), "--cpu"];
        let error = export(&parse(&args).unwrap()).unwrap_err();
        assert!(error.contains("(io)"), "{error}");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).ok();
    }
}
