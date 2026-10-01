//! `slopshop bench`: time the viewport renderer on a document the way the app uses it (the same
//! views redrawn, a pan, a zoom), to compare display performance before and after a change.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use slopshop_core::view::ViewTransform;
use slopshop_core::{Document, Size};
use slopshop_render::{FrameStats, Renderer};

use crate::parse_size;

struct Args {
    input: PathBuf,
    output: Size,
    at: Option<[f64; 2]>,
    frames: usize,
    /// Composite every frame directly, without the display cache.
    direct: bool,
}

pub fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;
    let started = Instant::now();
    let document = open(&args.input)?;
    let size = document.size();
    println!(
        "{}: {}x{}, opened in {:.0} ms",
        args.input.display(),
        size.width,
        size.height,
        started.elapsed().as_secs_f64() * 1000.0
    );
    let at = args
        .at
        .unwrap_or([f64::from(size.width) / 2.0, f64::from(size.height) / 2.0]);
    let output = args.output;
    let fit = ViewTransform::fit(size, output, 16);
    // A view of `zoom` output pixels per document pixel centered on `center`.
    let centered = |center: [f64; 2], zoom: f64| {
        let scale = 1.0 / zoom;
        ViewTransform {
            origin: [
                center[0] - f64::from(output.width) / 2.0 * scale,
                center[1] - f64::from(output.height) / 2.0 * scale,
            ],
            scale,
        }
    };
    let n = args.frames;
    // Panning at 100 %: a sixteenth of the view per frame, a brisk drag.
    let step = f64::from(output.width) / 16.0;
    // Zooming from fit to 800 % in equal ratios.
    let (from, to) = (fit.zoom(), 8.0f64);
    let scenarios: [(&str, Vec<ViewTransform>); 4] = [
        ("redraw, fit", vec![fit; n]),
        ("redraw, 100 %", vec![centered(at, 1.0); n]),
        (
            "pan, 100 %",
            (0..n)
                .map(|i| centered([at[0] + step * i as f64, at[1]], 1.0))
                .collect(),
        ),
        (
            "zoom, fit to 800 %",
            (0..n)
                .map(|i| {
                    let t = i as f64 / (n.max(2) - 1) as f64;
                    centered(at, from * (to / from).powf(t))
                })
                .collect(),
        ),
    ];

    let renderer = Renderer::new().map_err(|e| e.to_string())?;
    println!(
        "GPU: {}, output {}x{}, {n} frames per scenario, pixels not read back, {}",
        renderer.adapter_summary().name,
        output.width,
        output.height,
        if args.direct {
            "composited directly"
        } else {
            "through the display cache"
        }
    );
    println!(
        "{:<20} {:>9} {:>9} {:>9} {:>9} {:>9} {:>7} {:>7} {:>9}",
        "scenario", "first", "median", "p95", "prepare", "gpu", "layers", "tiles", "composed"
    );
    for (name, views) in scenarios {
        // A fresh renderer per scenario: its first frame starts from empty caches.
        let renderer = Renderer::new()
            .map_err(|e| e.to_string())?
            .with_display_cache(!args.direct);
        let mut frames: Vec<(Duration, FrameStats)> = Vec::with_capacity(views.len());
        for view in views {
            let started = Instant::now();
            let stats = renderer
                .profile_view(&document, view, output)
                .map_err(|e| e.to_string())?;
            frames.push((started.elapsed(), stats));
        }
        print_row(name, &frames);
    }
    Ok(())
}

fn print_row(name: &str, frames: &[(Duration, FrameStats)]) {
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let first = frames.first().map_or(0.0, |(d, _)| ms(*d));
    // Steady state: every frame but the first.
    let rest = frames.get(1..).unwrap_or_default();
    let totals: Vec<f64> = rest.iter().map(|(d, _)| ms(*d)).collect();
    let prepare: Vec<f64> = rest.iter().map(|(_, s)| ms(s.prepare)).collect();
    let gpu: Vec<f64> = rest.iter().filter_map(|(_, s)| s.gpu.map(ms)).collect();
    let layers = frames.iter().map(|(_, s)| s.layers).max().unwrap_or(0);
    let tiles: u64 = frames.iter().map(|(_, s)| s.tiles_uploaded).sum();
    let composited: u32 = frames.iter().map(|(_, s)| s.tiles_composited).sum();
    let gpu = if gpu.is_empty() {
        "-".to_owned()
    } else {
        format!("{:.2}", percentile(&gpu, 0.5))
    };
    println!(
        "{name:<20} {first:>9.2} {:>9.2} {:>9.2} {:>9.2} {gpu:>9} {layers:>7} {tiles:>7} {composited:>9}",
        percentile(&totals, 0.5),
        percentile(&totals, 0.95),
        percentile(&prepare, 0.5),
    );
}

/// The `p` quantile (0 to 1) of `values`, nearest rank; 0 when there are none.
fn percentile(values: &[f64], p: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((sorted.len() as f64 * p).ceil() as usize).saturating_sub(1);
    sorted.get(rank).copied().unwrap_or(0.0)
}

/// A .slop document, or an image file as a one-layer document.
fn open(path: &Path) -> Result<Document, String> {
    let cannot_open = |e: &dyn std::fmt::Display| format!("cannot open {}: {e}", path.display());
    if slopshop_io::slop::is_slop_file(path).unwrap_or(false) {
        let (document, _) = slopshop_io::slop::SlopFile::open(path).map_err(|e| cannot_open(&e))?;
        return Ok(document);
    }
    let imported = slopshop_io::open_image(path).map_err(|e| cannot_open(&e))?;
    crate::export::single_layer_document(imported.image, &crate::export::layer_name(path))
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut input = None;
    let mut output = Size::new(1920, 1080);
    let mut at = None;
    let mut frames = 30;
    let mut direct = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = || it.next().ok_or(format!("missing value for {arg}"));
        match arg.as_str() {
            "--size" => output = parse_size(value()?)?,
            "--at" => at = Some(parse_point(value()?)?),
            "--frames" => {
                let v = value()?;
                frames = v
                    .parse::<usize>()
                    .ok()
                    .filter(|&n| n >= 2)
                    .ok_or(format!("invalid frame count `{v}`, expected 2 or more"))?;
            }
            "--direct" => direct = true,
            other if other.starts_with("--") => return Err(format!("unknown option `{other}`")),
            path if input.is_none() => input = Some(PathBuf::from(path)),
            extra => return Err(format!("unexpected argument `{extra}`")),
        }
    }
    Ok(Args {
        input: input.ok_or("missing input file")?,
        output,
        at,
        frames,
        direct,
    })
}

/// `X,Y` in document pixels.
fn parse_point(s: &str) -> Result<[f64; 2], String> {
    let invalid = || format!("invalid point `{s}`, expected X,Y");
    let (x, y) = s.split_once(',').ok_or_else(invalid)?;
    let parse = |v: &str| {
        v.trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(invalid)
    };
    Ok([parse(x)?, parse(y)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parses_options() {
        let args = parse_args(&strings(&[
            "a.slop", "--size", "800x600", "--at", "10,20.5", "--frames", "5", "--direct",
        ]))
        .unwrap();
        assert!(args.direct);
        assert_eq!(args.input, PathBuf::from("a.slop"));
        assert_eq!(args.output, Size::new(800, 600));
        assert_eq!(args.at, Some([10.0, 20.5]));
        assert_eq!(args.frames, 5);
        assert!(parse_args(&strings(&[])).is_err());
        assert!(parse_args(&strings(&["a", "--frames", "1"])).is_err());
        assert!(parse_args(&strings(&["a", "--at", "1"])).is_err());
        assert!(parse_args(&strings(&["a", "b"])).is_err());
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let values = [4.0, 1.0, 3.0, 2.0];
        assert_eq!(percentile(&values, 0.5), 2.0);
        assert_eq!(percentile(&values, 0.95), 4.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
    }
}
