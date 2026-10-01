//! `slopshop export`: an image file, opened as a one-layer document, written to PNG, TIFF,
//! OpenEXR, JPEG or WebP by the export pipeline (ADR 0008, 0010), exactly as a front end would drive
//! it: the format's default settings, explicit overrides, the GPU renderer as the pixel source
//! (the CPU compositor without a GPU), and the export report. User documentation:
//! `docs/cli.md`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use slopshop_core::color::{ColorSpace, LinearRgba, SampleType, WORKING_SPACE};
use slopshop_core::{
    BlendMode, CancelToken, Document, Edit, Layer, LayerContent, RasterImage, Rect, Size,
};
use slopshop_io::export::{
    AvifDepth, ExportFormat, ExportFormatKind, ExportNotice, ExportReport, ExportSpec, ExrSample,
    JpegSubsampling, PngCompression, PngDepth, PsdDepth, TgaCompression, TiffCompression,
    TiffSample, WebpCompression, default_spec, export_image, export_psd, has_gray, supports_alpha,
    supports_gray, supports_space,
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

const FORMATS: [ExportFormatKind; 13] = [
    ExportFormatKind::Png,
    ExportFormatKind::Tiff,
    ExportFormatKind::Exr,
    ExportFormatKind::Jpeg,
    ExportFormatKind::Webp,
    ExportFormatKind::Psd,
    ExportFormatKind::Psb,
    ExportFormatKind::Bmp,
    ExportFormatKind::Tga,
    ExportFormatKind::Pnm,
    ExportFormatKind::Pfm,
    ExportFormatKind::Avif,
    ExportFormatKind::Jxl,
];

/// `--subsampling`, as it spells each value.
const SUBSAMPLINGS: [(JpegSubsampling, &str); 3] = [
    (JpegSubsampling::S444, "444"),
    (JpegSubsampling::S422, "422"),
    (JpegSubsampling::S420, "420"),
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

    /// AVIF: `u16` writes 10-bit samples.
    fn avif(self) -> Option<AvifDepth> {
        match self {
            Depth::U8 => Some(AvifDepth::U8),
            Depth::U16 => Some(AvifDepth::U10),
            Depth::F16 | Depth::F32 => None,
        }
    }

    fn psd(self) -> Option<PsdDepth> {
        match self {
            Depth::U8 => Some(PsdDepth::U8),
            Depth::U16 => Some(PsdDepth::U16),
            Depth::F16 | Depth::F32 => None,
        }
    }

    fn supported_by(self, kind: ExportFormatKind) -> bool {
        match kind {
            ExportFormatKind::Png | ExportFormatKind::Pnm | ExportFormatKind::Jxl => {
                self.png().is_some()
            }
            ExportFormatKind::Tiff => self.tiff().is_some(),
            ExportFormatKind::Exr => self.exr().is_some(),
            ExportFormatKind::Pfm => self == Depth::F32,
            ExportFormatKind::Avif => self.avif().is_some(),
            ExportFormatKind::Jpeg
            | ExportFormatKind::Webp
            | ExportFormatKind::Bmp
            | ExportFormatKind::Tga => self == Depth::U8,
            ExportFormatKind::Psd | ExportFormatKind::Psb => self.psd().is_some(),
        }
    }
}

/// `--compression`: the compression settings of every format (EXR and JPEG have none to
/// choose).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    Fast,
    Small,
    None,
    Deflate,
    Lzw,
    Lossy,
    Lossless,
    Rle,
}

impl Compression {
    const ALL: [Compression; 8] = [
        Compression::Fast,
        Compression::Small,
        Compression::None,
        Compression::Deflate,
        Compression::Lzw,
        Compression::Lossy,
        Compression::Lossless,
        Compression::Rle,
    ];

    fn name(self) -> &'static str {
        match self {
            Compression::Fast => "fast",
            Compression::Small => "small",
            Compression::None => "none",
            Compression::Deflate => "deflate",
            Compression::Lzw => "lzw",
            Compression::Lossy => "lossy",
            Compression::Lossless => "lossless",
            Compression::Rle => "rle",
        }
    }

    fn png(self) -> Option<PngCompression> {
        match self {
            Compression::Fast => Some(PngCompression::Fast),
            Compression::Small => Some(PngCompression::Small),
            _ => None,
        }
    }

    fn tiff(self) -> Option<TiffCompression> {
        match self {
            Compression::None => Some(TiffCompression::None),
            Compression::Deflate => Some(TiffCompression::Deflate),
            Compression::Lzw => Some(TiffCompression::Lzw),
            _ => None,
        }
    }

    fn is_webp(self) -> bool {
        matches!(self, Compression::Lossy | Compression::Lossless)
    }

    fn tga(self) -> Option<TgaCompression> {
        match self {
            Compression::None => Some(TgaCompression::None),
            Compression::Rle => Some(TgaCompression::Rle),
            _ => None,
        }
    }

    fn supported_by(self, kind: ExportFormatKind) -> bool {
        match kind {
            ExportFormatKind::Png => self.png().is_some(),
            ExportFormatKind::Tiff => self.tiff().is_some(),
            ExportFormatKind::Webp => self.is_webp(),
            ExportFormatKind::Tga => self.tga().is_some(),
            ExportFormatKind::Exr
            | ExportFormatKind::Jpeg
            | ExportFormatKind::Psd
            | ExportFormatKind::Psb
            | ExportFormatKind::Bmp
            | ExportFormatKind::Pnm
            | ExportFormatKind::Pfm
            | ExportFormatKind::Avif
            | ExportFormatKind::Jxl => false,
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
    /// `--quality`: JPEG 1 to 100, lossy WebP and AVIF 0 to 100.
    quality: Option<u8>,
    /// `--subsampling` (JPEG).
    subsampling: Option<JpegSubsampling>,
    no_alpha: bool,
    /// `--matte`: sRGB-encoded 8-bit components.
    matte: Option<[u8; 3]>,
    no_dither: bool,
    /// `--gray` (`Some(true)`) or `--color` (`Some(false)`); the default depends on the document.
    gray: Option<bool>,
    cpu: bool,
    bench: bool,
    /// `--scale`: resample the whole image by this factor first (Image Size, ADR 0018).
    scale: Option<f64>,
    /// `--page`: the page of a PDF input, from 1.
    page: Option<usize>,
    /// `--dpi`: the resolution a PDF input is rendered at.
    dpi: Option<f32>,
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
            Some(count) => {
                let unit = match notice {
                    ExportNotice::AlphaFlattened(_) => "pixels",
                    _ => "samples",
                };
                println!("report: {} ({count} {unit})", notice.id());
            }
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
    let mut quality = None;
    let mut subsampling = None;
    let mut matte = None;
    let (mut no_alpha, mut no_dither, mut cpu, mut bench) = (false, false, false, false);
    let mut gray = None;
    let mut scale = None;
    let (mut page, mut dpi) = (None, None);

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("missing value for {arg}"));
        match arg.as_str() {
            "--format" => format = Some(parse_format(value()?)?),
            "--depth" => depth = Some(parse_depth(value()?)?),
            "--space" => space = Some(parse_space(value()?)?),
            "--compression" => compression = Some(parse_compression(value()?)?),
            "--quality" => quality = Some(parse_quality(value()?)?),
            "--subsampling" => subsampling = Some(parse_subsampling(value()?)?),
            "--matte" => matte = Some(parse_matte(value()?)?),
            "--no-alpha" => no_alpha = true,
            "--no-dither" => no_dither = true,
            "--gray" | "--color" => {
                let wanted = arg == "--gray";
                if gray.is_some_and(|g| g != wanted) {
                    return Err("--gray and --color exclude each other".to_owned());
                }
                gray = Some(wanted);
            }
            "--cpu" => cpu = true,
            "--bench" => bench = true,
            "--scale" => scale = Some(parse_scale(value()?)?),
            "--page" => page = Some(parse_page(value()?)?),
            "--dpi" => dpi = Some(parse_dpi(value()?)?),
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
            "cannot tell the format from `{}`: use --format png|tiff|exr|jpeg|webp",
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
            match format {
                ExportFormatKind::Jpeg => {
                    format!("{name} has no --compression option (use --quality and --subsampling)")
                }
                ExportFormatKind::Psd | ExportFormatKind::Psb => {
                    format!("{name} has no --compression option (it always uses RLE)")
                }
                ExportFormatKind::Bmp | ExportFormatKind::Pnm | ExportFormatKind::Pfm => {
                    format!("{name} has no --compression option (it is uncompressed)")
                }
                ExportFormatKind::Jxl => {
                    format!("{name} has no --compression option (it is always lossless)")
                }
                _ => format!("{name} has no --compression option (it always uses lossless ZIP)"),
            }
        } else {
            format!(
                "--compression {} is not available for {name} (valid: {})",
                compression.name(),
                valid.join(", ")
            )
        });
    }
    if let Some(quality) = quality {
        let lossless = compression == Some(Compression::Lossless);
        match format {
            ExportFormatKind::Jpeg if quality == 0 => {
                return Err("--quality 0 is not available for JPEG (1 to 100)".to_owned());
            }
            ExportFormatKind::Jpeg => {}
            ExportFormatKind::Webp if lossless => {
                return Err("--quality is not available for lossless WebP".to_owned());
            }
            ExportFormatKind::Webp | ExportFormatKind::Avif => {}
            _ => {
                return Err(format!(
                    "--quality is not available for {name} (JPEG, lossy WebP and AVIF only)"
                ));
            }
        }
    }
    if format != ExportFormatKind::Jpeg && subsampling.is_some() {
        return Err(format!(
            "--subsampling is not available for {name} (JPEG only)"
        ));
    }
    if format.is_layered() && no_alpha {
        return Err(format!(
            "--no-alpha is not available for {name} (layers keep their transparency)"
        ));
    }
    if gray == Some(true) && !has_gray(format) {
        return Err(format!(
            "--gray is not available for {name} (PNG, TIFF and JPEG only)"
        ));
    }
    let taggable = |s: &ColorSpace| {
        if gray == Some(true) {
            supports_gray(format, s)
        } else {
            supports_space(format, s)
        }
    };
    if let Some(space) = space.filter(|s| !taggable(s)) {
        let valid: Vec<_> = SPACES
            .iter()
            .filter(|s| taggable(s))
            .filter_map(ColorSpace::id)
            .collect();
        let what = if gray == Some(true) {
            "gray samples in the color space"
        } else {
            "the color space"
        };
        return Err(format!(
            "{name} cannot store and tag {what} `{}` (valid: {})",
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
        quality,
        subsampling,
        no_alpha,
        matte,
        no_dither,
        gray,
        cpu,
        bench,
        scale,
        page,
        dpi,
    })
}

fn parse_page(s: &str) -> Result<usize, String> {
    match s.parse::<usize>() {
        Ok(v) if v >= 1 => Ok(v),
        _ => Err(format!("invalid --page `{s}` (a page number, from 1)")),
    }
}

fn parse_dpi(s: &str) -> Result<f32, String> {
    match s.parse::<f32>() {
        Ok(v) if v.is_finite() && (1.0..=10_000.0).contains(&v) => Ok(v),
        _ => Err(format!("invalid --dpi `{s}` (1 to 10000)")),
    }
}

fn parse_scale(s: &str) -> Result<f64, String> {
    match s.trim_end_matches('x').parse::<f64>() {
        Ok(v) if v.is_finite() && v > 0.0 && v <= 1000.0 => Ok(v),
        _ => Err(format!(
            "invalid --scale `{s}` (a factor above 0, e.g. 4 or 0.25)"
        )),
    }
}

/// `document` resampled by `factor` (Image Size): each side rounded, at least one pixel.
fn scaled(document: Document, factor: f64) -> Result<Document, String> {
    let size = document.size();
    let side = |v: u32| ((f64::from(v) * factor).round() as u32).max(1);
    let mut session = slopshop_core::Session::new(document);
    let edit = slopshop_core::Edit::resize_image(
        session.document(),
        Size::new(side(size.width), side(size.height)),
    )
    .map_err(|e| format!("cannot scale: {e}"))?;
    session
        .perform(edit)
        .map_err(|e| format!("cannot scale: {e}"))?;
    Ok(session.document().clone())
}

fn parse_format(s: &str) -> Result<ExportFormatKind, String> {
    FORMATS
        .into_iter()
        .find(|kind| format_id(*kind) == s)
        .ok_or(format!(
            "invalid format `{s}`, expected png, tiff, exr, jpeg or webp"
        ))
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
            "invalid compression `{s}`, expected fast, small, none, deflate, lzw, lossy or lossless"
        ))
}

/// 0 to 100; JPEG's lower bound (1) is checked with the format.
fn parse_quality(s: &str) -> Result<u8, String> {
    s.parse::<u8>()
        .ok()
        .filter(|q| *q <= 100)
        .ok_or(format!("invalid quality `{s}`, expected 0 to 100"))
}

fn parse_subsampling(s: &str) -> Result<JpegSubsampling, String> {
    SUBSAMPLINGS
        .into_iter()
        .find(|(_, name)| *name == s)
        .map(|(subsampling, _)| subsampling)
        .ok_or(format!(
            "invalid subsampling `{s}`, expected 444, 422 or 420"
        ))
}

fn subsampling_name(subsampling: JpegSubsampling) -> &'static str {
    SUBSAMPLINGS
        .into_iter()
        .find(|(s, _)| *s == subsampling)
        .map_or("?", |(_, name)| name)
}

/// `RRGGBB` or `#RRGGBB`, sRGB-encoded.
fn parse_matte(s: &str) -> Result<[u8; 3], String> {
    let hex = s.strip_prefix('#').unwrap_or(s);
    let invalid = || format!("invalid matte `{s}`, expected an sRGB color as RRGGBB");
    if hex.len() != 6 || !hex.is_ascii() {
        return Err(invalid());
    }
    let mut rgb = [0u8; 3];
    for (i, c) in rgb.iter_mut().enumerate() {
        *c = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| invalid())?;
    }
    Ok(rgb)
}

/// The working-space matte of an sRGB `--matte`.
fn matte_color([r, g, b]: [u8; 3]) -> LinearRgba {
    let unit = |c: u8| f32::from(c) / 255.0;
    LinearRgba::from_srgb_encoded_to_working(unit(r), unit(g), unit(b), 1.0)
}

/// A working-space matte as `--matte` spells it (rounded to 8-bit sRGB).
fn matte_hex(matte: LinearRgba) -> String {
    let srgb = matte
        .transform(&WORKING_SPACE.matrix_to(&ColorSpace::LINEAR_SRGB))
        .to_srgb_encoded();
    let code = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!(
        "{:02x}{:02x}{:02x}",
        code(srgb[0]),
        code(srgb[1]),
        code(srgb[2])
    )
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
        "jpg" | "jpeg" => Some(ExportFormatKind::Jpeg),
        "webp" => Some(ExportFormatKind::Webp),
        "psd" => Some(ExportFormatKind::Psd),
        "psb" => Some(ExportFormatKind::Psb),
        "bmp" => Some(ExportFormatKind::Bmp),
        "tga" => Some(ExportFormatKind::Tga),
        "pnm" | "ppm" | "pgm" | "pam" => Some(ExportFormatKind::Pnm),
        "pfm" => Some(ExportFormatKind::Pfm),
        "avif" => Some(ExportFormatKind::Avif),
        "jxl" => Some(ExportFormatKind::Jxl),
        _ => None,
    }
}

/// As `--format` spells it.
fn format_id(kind: ExportFormatKind) -> &'static str {
    match kind {
        ExportFormatKind::Png => "png",
        ExportFormatKind::Tiff => "tiff",
        ExportFormatKind::Exr => "exr",
        ExportFormatKind::Jpeg => "jpeg",
        ExportFormatKind::Webp => "webp",
        ExportFormatKind::Psd => "psd",
        ExportFormatKind::Psb => "psb",
        ExportFormatKind::Bmp => "bmp",
        ExportFormatKind::Tga => "tga",
        ExportFormatKind::Pnm => "pnm",
        ExportFormatKind::Pfm => "pfm",
        ExportFormatKind::Avif => "avif",
        ExportFormatKind::Jxl => "jxl",
    }
}

fn format_name(kind: ExportFormatKind) -> &'static str {
    match kind {
        ExportFormatKind::Png => "PNG",
        ExportFormatKind::Tiff => "TIFF",
        ExportFormatKind::Exr => "OpenEXR",
        ExportFormatKind::Jpeg => "JPEG",
        ExportFormatKind::Webp => "WebP",
        ExportFormatKind::Psd => "Photoshop (layered PSD)",
        ExportFormatKind::Psb => "Photoshop large document (layered PSB)",
        ExportFormatKind::Bmp => "BMP",
        ExportFormatKind::Tga => "Targa",
        ExportFormatKind::Pnm => "Netpbm (PGM, PPM, PAM)",
        ExportFormatKind::Pfm => "Portable Float Map",
        ExportFormatKind::Avif => "AVIF",
        ExportFormatKind::Jxl => "JPEG XL (lossless)",
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
        ExportFormat::Jpeg {
            quality,
            subsampling,
        } => ExportFormat::Jpeg {
            quality: args.quality.unwrap_or(quality),
            subsampling: args.subsampling.unwrap_or(subsampling),
        },
        ExportFormat::Webp { compression } => {
            let default_quality = match compression {
                WebpCompression::Lossy { quality } => quality,
                WebpCompression::Lossless => 90,
            };
            ExportFormat::Webp {
                compression: match (args.compression, compression) {
                    (Some(Compression::Lossless), _) => WebpCompression::Lossless,
                    (Some(Compression::Lossy), _) | (_, WebpCompression::Lossy { .. }) => {
                        WebpCompression::Lossy {
                            quality: args.quality.unwrap_or(default_quality),
                        }
                    }
                    (_, WebpCompression::Lossless) => WebpCompression::Lossless,
                },
            }
        }
        ExportFormat::Psd { depth } => ExportFormat::Psd {
            depth: args.depth.and_then(Depth::psd).unwrap_or(depth),
        },
        ExportFormat::Psb { depth } => ExportFormat::Psb {
            depth: args.depth.and_then(Depth::psd).unwrap_or(depth),
        },
        ExportFormat::Bmp => ExportFormat::Bmp,
        ExportFormat::Pnm { depth } => ExportFormat::Pnm {
            depth: args.depth.and_then(Depth::png).unwrap_or(depth),
        },
        ExportFormat::Pfm => ExportFormat::Pfm,
        ExportFormat::Jxl { depth } => ExportFormat::Jxl {
            depth: args.depth.and_then(Depth::png).unwrap_or(depth),
        },
        ExportFormat::Avif { depth, quality } => ExportFormat::Avif {
            depth: args.depth.and_then(Depth::avif).unwrap_or(depth),
            quality: args.quality.unwrap_or(quality),
        },
        ExportFormat::Tga { compression } => ExportFormat::Tga {
            compression: args
                .compression
                .and_then(Compression::tga)
                .unwrap_or(compression),
        },
    };
    if let Some(space) = args.space {
        spec.space = space;
    }
    if args.no_alpha {
        spec.keep_alpha = false;
    }
    if let Some(matte) = args.matte {
        spec.matte = matte_color(matte);
    }
    if args.no_dither {
        spec.dither = false;
    }
    if let Some(gray) = args.gray {
        spec.gray = gray;
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
        ExportFormat::Jpeg { .. } | ExportFormat::Bmp => (Depth::U8, None),
        ExportFormat::Pnm { depth } | ExportFormat::Jxl { depth } => (
            match depth {
                PngDepth::U8 => Depth::U8,
                PngDepth::U16 => Depth::U16,
            },
            None,
        ),
        ExportFormat::Pfm => (Depth::F32, None),
        ExportFormat::Avif { depth, .. } => (
            match depth {
                AvifDepth::U8 => Depth::U8,
                AvifDepth::U10 => Depth::U16,
            },
            None,
        ),
        ExportFormat::Tga { compression } => (
            Depth::U8,
            Some(match compression {
                TgaCompression::None => Compression::None,
                TgaCompression::Rle => Compression::Rle,
            }),
        ),
        ExportFormat::Webp { compression } => (
            Depth::U8,
            Some(match compression {
                WebpCompression::Lossy { .. } => Compression::Lossy,
                WebpCompression::Lossless => Compression::Lossless,
            }),
        ),
        ExportFormat::Psd { depth } | ExportFormat::Psb { depth } => (
            match depth {
                PsdDepth::U8 => Depth::U8,
                PsdDepth::U16 => Depth::U16,
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
    if let ExportFormat::Jpeg {
        quality,
        subsampling,
    } = spec.format
    {
        text += &format!(
            " --quality {quality} --subsampling {}",
            subsampling_name(subsampling)
        );
    }
    if let ExportFormat::Webp {
        compression: WebpCompression::Lossy { quality },
    }
    | ExportFormat::Avif { quality, .. } = spec.format
    {
        text += &format!(" --quality {quality}");
    }
    if !spec.keep_alpha {
        // Formats without alpha always flatten: no option needed for that.
        if supports_alpha(spec.format.kind()) {
            text += " --no-alpha";
        }
        text += &format!(" --matte {}", matte_hex(spec.matte));
    }
    // Dither only applies to 8-bit samples.
    if !spec.dither && spec.format.sample_type() == SampleType::U8 {
        text += " --no-dither";
    }
    if spec.gray {
        text += " --gray";
    }
    text
}

/// Open, build the document, export.
fn export(args: &Args) -> Result<Outcome, String> {
    let started = Instant::now();
    let cannot_open =
        |e: &dyn std::fmt::Display| format!("cannot open {}: {e}", args.input.display());
    // A .slop document (recognized by its content), or an image as a one-layer document.
    let is_document = slopshop_io::slop::is_slop_file(&args.input).unwrap_or(false);
    let (document, import_warnings) = if is_document {
        let (document, _) =
            slopshop_io::slop::SlopFile::open(&args.input).map_err(|e| cannot_open(&e))?;
        (document, Vec::new())
    } else if args.page.is_some() || args.dpi.is_some() {
        let file = slopshop_io::vector::VectorFile::open(&args.input)
            .map_err(|e| cannot_open(&e))?
            .ok_or("--page and --dpi apply to PDF and SVG files only")?;
        let page = args.page.unwrap_or(1);
        if page > file.page_count() {
            return Err(format!(
                "--page {page}: {} has {} page(s)",
                args.input.display(),
                file.page_count()
            ));
        }
        let imported = file
            .render(page - 1, args.dpi.unwrap_or(file.default_dpi()))
            .map_err(|e| cannot_open(&e))?;
        let warnings = imported.warnings.iter().map(|w| w.id()).collect();
        (
            single_layer_document(imported.image, &layer_name(&args.input))?,
            warnings,
        )
    } else {
        let imported = slopshop_io::open_image(&args.input).map_err(|e| cannot_open(&e))?;
        let warnings = imported.warnings.iter().map(|w| w.id()).collect();
        (
            single_layer_document(imported.image, &layer_name(&args.input))?,
            warnings,
        )
    };
    let document = match args.scale {
        Some(factor) => scaled(document, factor)?,
        None => document,
    };
    let open = started.elapsed();
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
    let mut render = slopshop_render::export_renderer(renderer.as_ref());
    let mut timed = |d: &Document, region: Rect, out: &mut [f32]| {
        let started = Instant::now();
        let result = render(d, region, out);
        source_time += started.elapsed();
        result
    };
    let mut bands = 0;
    let started = Instant::now();
    let report = match spec.psd_options() {
        // A layered file: each layer rendered on its own.
        Some(options) => export_psd(
            &args.output,
            &document,
            &options,
            &mut timed,
            &CancelToken::new(),
            &mut |_| bands += 1,
        ),
        None => export_image(
            &args.output,
            document.size(),
            &spec,
            |region, out| timed(&document, region, out),
            &CancelToken::new(),
            &mut |_| bands += 1,
        ),
    }
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
        parent: None,
        index: 0,
        layer: Layer {
            transform: slopshop_core::Affine::IDENTITY,
            clipped: false,
            id,
            name: name.to_owned(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
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
    fn gray_is_checked_against_the_format_and_the_space() {
        let error = |args: &[&str]| parse(args).unwrap_err();
        assert!(error(&["in.png", "out.exr", "--gray"]).contains("PNG, TIFF and JPEG only"));
        assert!(error(&["in.png", "out.webp", "--gray"]).contains("PNG, TIFF and JPEG only"));
        assert!(error(&["in.png", "out.png", "--gray", "--color"]).contains("exclude"));
        let pq = error(&["in.png", "out.png", "--gray", "--space", "rec2100-pq"]);
        assert!(pq.contains("gray samples"), "{pq}");
        assert!(!pq.contains("rec2100-pq,"), "{pq}");
        // Color PQ is fine in PNG.
        assert!(parse(&["in.png", "out.png", "--space", "rec2100-pq"]).is_ok());
        let args = parse(&["in.png", "out.jpg", "--gray"]).unwrap();
        let document = Document::new(Size::new(4, 4));
        let spec = export_spec(&args, &document);
        assert!(spec.gray);
        assert!(describe(&spec).ends_with(" --gray"));
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
            "--gray",
            "--cpu",
            "--bench",
            "--scale",
            "0.5x",
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
                quality: None,
                subsampling: None,
                no_alpha: true,
                matte: None,
                no_dither: true,
                gray: Some(true),
                cpu: true,
                bench: true,
                scale: Some(0.5),
                page: None,
                dpi: None,
            }
        );
        let pdf = parse(&["doc.pdf", "out.png", "--page", "3", "--dpi", "150"]).unwrap();
        assert_eq!((pdf.page, pdf.dpi), (Some(3), Some(150.0)));
        assert!(parse(&["doc.pdf", "out.png", "--page", "0"]).is_err());
        assert!(parse(&["doc.pdf", "out.png", "--dpi", "0"]).is_err());
        assert!(parse(&["in.png", "out.png", "--scale", "0"]).is_err());
        let args = parse(&["in.png", "out.tif"]).unwrap();
        assert_eq!(args.format, ExportFormatKind::Tiff);
        assert_eq!(
            (args.depth, args.space, args.compression),
            (None, None, None)
        );
        assert!(!args.no_alpha && !args.no_dither && !args.cpu && !args.bench);
        assert_eq!(args.gray, None);
        assert_eq!(
            parse(&["in.png", "out.png", "--color"]).unwrap().gray,
            Some(false)
        );
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
        assert!(error(&["in.png", "out.gif"]).contains("--format"));
        assert!(error(&["in.png", "out"]).contains("--format"));
        assert!(error(&["in.png", "out.png", "--depth"]).contains("missing value"));
        assert!(error(&["in.png", "out.png", "--depth", "u12"]).contains("invalid depth"));
        assert!(error(&["in.png", "out.png", "--format", "gif"]).contains("invalid format"));
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
    fn matte_is_an_srgb_hex_color() {
        assert_eq!(parse_matte("ff8000"), Ok([255, 128, 0]));
        assert_eq!(parse_matte("#0A0b0C"), Ok([10, 11, 12]));
        for bad in ["fff", "ff80000", "gg0000", "#", "ff 800", "ééé"] {
            assert!(parse_matte(bad).is_err(), "{bad}");
        }
        // The working-space color comes back to the same 8-bit code.
        for rgb in [[255, 255, 255], [0, 0, 0], [255, 128, 0], [10, 11, 12]] {
            let hex = format!("{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
            assert_eq!(matte_hex(matte_color(rgb)), hex);
        }

        let size = Size::new(4, 4);
        let pixels = vec![128u8; 4 * 4 * 4];
        let image =
            RasterImage::from_pixels(size, slopshop_core::color::PixelFormat::RGBA8_SRGB, &pixels)
                .unwrap();
        let document = single_layer_document(image, "layer").unwrap();
        let args = parse(&["in.png", "out.png", "--no-alpha", "--matte", "#FF8000"]).unwrap();
        assert_eq!(args.matte, Some([255, 128, 0]));
        let spec = export_spec(&args, &document);
        assert_eq!(spec.matte, matte_color([255, 128, 0]));
        assert!(describe(&spec).ends_with("--no-alpha --matte ff8000"));
        assert!(parse(&["in.png", "out.png", "--matte", "red"]).is_err());
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
            "--format tiff --depth f32 --space linear-rec2020 --compression lzw --no-alpha --matte ffffff"
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

    #[test]
    fn exports_jpeg_flattened_and_tagged() {
        let dir = temp_dir("jpeg");
        let size = Size::new(70, 300);
        let input = dir.join("in.png");
        write_test_png(&input, size);
        for (name, options) in [
            ("out.jpg", &[][..]),
            (
                "small.jpeg",
                &["--quality", "60", "--subsampling", "420"][..],
            ),
        ] {
            let output = dir.join(name);
            let mut args = vec![input.to_str().unwrap(), output.to_str().unwrap(), "--cpu"];
            args.extend(options);
            let outcome = export(&parse(&args).unwrap()).unwrap();
            assert_eq!(outcome.spec.format.kind(), ExportFormatKind::Jpeg, "{name}");
            assert!(!outcome.spec.keep_alpha);
            // The translucent input is flattened over the (white) matte, and reported.
            let ids: Vec<_> = outcome.report.notices.iter().map(|n| n.id()).collect();
            assert_eq!(ids, ["alphaFlattened"], "{name}");
            let reimported = slopshop_io::open_image(&output).unwrap();
            assert_eq!(reimported.image.size(), size);
            assert_eq!(reimported.image.format().color_space, ColorSpace::SRGB);
            assert!(!reimported.image.format().layout.has_alpha());
        }
        let default = fs::metadata(dir.join("out.jpg")).unwrap().len();
        let small = fs::metadata(dir.join("small.jpeg")).unwrap().len();
        assert!(small < default, "{small} vs {default}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn exports_webp_lossy_and_lossless() {
        let dir = temp_dir("webp");
        let size = Size::new(70, 300);
        let input = dir.join("in.png");
        write_test_png(&input, size);
        for (name, options, lossless) in [
            ("lossy.webp", &[][..], false),
            ("small.webp", &["--quality", "40"][..], false),
            ("lossless.webp", &["--compression", "lossless"][..], true),
        ] {
            let output = dir.join(name);
            let mut args = vec![input.to_str().unwrap(), output.to_str().unwrap(), "--cpu"];
            args.extend(options);
            let outcome = export(&parse(&args).unwrap()).unwrap();
            assert_eq!(outcome.spec.format.kind(), ExportFormatKind::Webp, "{name}");
            // The input is translucent: alpha is kept, nothing is reported.
            assert!(outcome.spec.keep_alpha);
            assert!(outcome.report.notices.is_empty(), "{name}");
            let reimported = slopshop_io::open_image(&output).unwrap();
            assert_eq!(reimported.image.size(), size);
            assert!(reimported.image.format().layout.has_alpha());
            if lossless {
                // Unedited 8-bit pixels come back exactly.
                let original = slopshop_io::open_image(&input).unwrap();
                let expected = composite(&single_layer_document(original.image, "in").unwrap());
                let actual = composite(&single_layer_document(reimported.image, name).unwrap());
                assert_eq!(max_difference(&expected, &actual), 0.0, "{name}");
            }
        }
        let default = fs::metadata(dir.join("lossy.webp")).unwrap().len();
        let small = fs::metadata(dir.join("small.webp")).unwrap().len();
        assert!(small < default, "{small} vs {default}");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn webp_options_are_validated() {
        let args = parse(&["in.png", "out.WEBP", "--quality", "0"]).unwrap();
        assert_eq!(
            (args.format, args.quality),
            (ExportFormatKind::Webp, Some(0))
        );
        for bad in [
            &[
                "in.png",
                "out.webp",
                "--compression",
                "lossless",
                "--quality",
                "80",
            ][..],
            &["in.png", "out.webp", "--compression", "fast"],
            &["in.png", "out.webp", "--subsampling", "420"],
            &["in.png", "out.webp", "--depth", "u16"],
            &["in.png", "out.webp", "--space", "rec2100-hlg"],
            &["in.png", "out.jpg", "--quality", "0"],
            &["in.png", "out.jpg", "--compression", "lossy"],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }

        let size = Size::new(4, 4);
        let image = RasterImage::from_pixels(
            size,
            slopshop_core::color::PixelFormat::RGBA8_SRGB,
            &[128u8; 4 * 4 * 4],
        )
        .unwrap();
        let document = single_layer_document(image, "layer").unwrap();
        let describe_args =
            |args: &[&str]| describe(&export_spec(&parse(args).unwrap(), &document));
        assert_eq!(
            describe_args(&["in.png", "out.webp"]),
            "--format webp --depth u8 --space srgb --compression lossy --quality 90"
        );
        assert_eq!(
            describe_args(&[
                "in.png",
                "out.webp",
                "--compression",
                "lossless",
                "--no-alpha"
            ]),
            "--format webp --depth u8 --space srgb --compression lossless --no-alpha --matte ffffff"
        );
    }

    #[test]
    fn jpeg_options_are_validated() {
        let args = parse(&[
            "in.png",
            "out.JPG",
            "--quality",
            "75",
            "--subsampling",
            "422",
        ]);
        let args = args.unwrap();
        assert_eq!(args.format, ExportFormatKind::Jpeg);
        assert_eq!(
            (args.quality, args.subsampling),
            (Some(75), Some(JpegSubsampling::S422))
        );
        for bad in [
            &["in.png", "out.jpg", "--quality", "0"][..],
            &["in.png", "out.jpg", "--quality", "101"],
            &["in.png", "out.jpg", "--subsampling", "411"],
            &["in.png", "out.jpg", "--depth", "u16"],
            &["in.png", "out.jpg", "--compression", "fast"],
            &["in.png", "out.jpg", "--space", "rec2100-pq"],
            &["in.png", "out.png", "--quality", "90"],
            &["in.png", "out.tif", "--subsampling", "420"],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }

        let size = Size::new(4, 4);
        let image = RasterImage::from_pixels(
            size,
            slopshop_core::color::PixelFormat::RGBA8_SRGB,
            &[128u8; 4 * 4 * 4],
        )
        .unwrap();
        let document = single_layer_document(image, "layer").unwrap();
        let spec = export_spec(&parse(&["in.png", "out.jpg"]).unwrap(), &document);
        assert_eq!(
            describe(&spec),
            "--format jpeg --depth u8 --space srgb --quality 90 --subsampling 444 --matte ffffff"
        );
    }

    #[test]
    fn scale_resamples_the_whole_image() {
        let dir = temp_dir("scale");
        let input = dir.join("in.png");
        write_test_png(&input, Size::new(30, 20));
        for (factor, expected) in [("4", Size::new(120, 80)), ("0.25", Size::new(8, 5))] {
            let output = dir.join(format!("out-{factor}.png"));
            let args = [
                input.to_str().unwrap(),
                output.to_str().unwrap(),
                "--scale",
                factor,
                "--cpu",
            ];
            let outcome = export(&parse(&args).unwrap()).unwrap();
            assert_eq!(outcome.size, expected);
            let reimported = slopshop_io::open_image(&output).unwrap();
            assert_eq!(reimported.image.size(), expected);
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
