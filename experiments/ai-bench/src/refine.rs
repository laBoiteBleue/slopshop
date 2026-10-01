//! Experiment: AI selection on a large image as the maintainer described it: a coarse mask from
//! a downscaled image, then refinement of the uncertain band only, at full resolution.
//!
//! 1. The image is downscaled to 1024² and SAM 2.1 runs on it with one click: 256² logits.
//! 2. The logits are upsampled to full resolution (bilinear, as SAM's own post-processing).
//! 3. The band where the coarse mask is uncertain (probability between 1% and 99%) is found;
//!    elsewhere the mask is 0 or 1.
//! 4. In the band only, tile by tile, a guided filter (He, Sun and Tang) aligns the mask with the
//!    edges of the full-resolution image.
//!
//! Each step is timed, and crops of the coarse and refined masks are written as PNG files.

use std::path::Path;
use std::time::Instant;

use image::{GrayImage, ImageBuffer, Luma, Rgb, RgbImage};
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;

use crate::{Ep, session};

const SIDE: u32 = 1024;
const TILE: usize = 256;

pub struct Options {
    pub image: std::path::PathBuf,
    /// Upscale the photo by this factor first, to stand for a larger one.
    pub scale: f32,
    /// The click, as fractions of the width and height.
    pub click: (f32, f32),
    /// A box around the subject, as fractions (x0, y0, x1, y1), with the click.
    pub boxed: Option<[f32; 4]>,
    /// The top-left corner of the full-resolution crop, as fractions.
    pub crop: (f32, f32),
    /// ViTMatte's undecided band: pixels on each side of the coarse contour, at its 1024² input.
    pub trimap_band: usize,
    /// Guided filter radius and regularization (on luminance in [0, 1]).
    pub radius: usize,
    pub eps: f32,
    pub out: std::path::PathBuf,
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

pub fn run(models: &Path, ep: Ep, o: &Options) -> Result<(), String> {
    let t = Instant::now();
    let mut photo = image::open(&o.image).map_err(|e| e.to_string())?.to_rgb8();
    if o.scale != 1.0 {
        let (w, h) = photo.dimensions();
        let (w, h) = ((w as f32 * o.scale) as u32, (h as f32 * o.scale) as u32);
        photo = image::imageops::resize(&photo, w, h, image::imageops::FilterType::CatmullRom);
    }
    let (w, h) = photo.dimensions();
    println!(
        "image {w}×{h} ({:.0} MP), loaded in {:.0} ms",
        (w * h) as f64 / 1e6,
        ms(t)
    );

    // 1. Coarse mask on a 1024² copy.
    let t = Instant::now();
    let small = image::imageops::resize(&photo, SIDE, SIDE, image::imageops::FilterType::Triangle);
    let (mean, std) = ([0.485f32, 0.456, 0.406], [0.229f32, 0.224, 0.225]);
    let plane = (SIDE * SIDE) as usize;
    let mut pixels = vec![0f32; 3 * plane];
    for (i, p) in small.pixels().enumerate() {
        for c in 0..3 {
            pixels[c * plane + i] = (f32::from(p[c]) / 255.0 - mean[c]) / std[c];
        }
    }
    println!("downscale to {SIDE}²: {:.0} ms", ms(t));
    let repo = models.join("onnx-community/sam2.1-hiera-base-plus-ONNX/onnx");
    let level = GraphOptimizationLevel::Level3;
    let mut encoder =
        session(&repo.join("vision_encoder_fp16.onnx"), ep, level).map_err(|e| e.to_string())?;
    let mut decoder = session(
        &repo.join("prompt_encoder_mask_decoder_fp16.onnx"),
        ep,
        level,
    )
    .map_err(|e| e.to_string())?;
    let input = Tensor::from_array(([1usize, 3, SIDE as usize, SIDE as usize], pixels))
        .map_err(|e| e.to_string())?;
    // Warm-up runs, so that the timings below are steady state.
    encoder
        .run(ort::inputs!["pixel_values" => input.view()])
        .map_err(|e| e.to_string())?;
    let t = Instant::now();
    let embeddings = encoder
        .run(ort::inputs!["pixel_values" => input.view()])
        .map_err(|e| e.to_string())?;
    let e0 = embeddings["image_embeddings.0"]
        .try_extract_array::<f32>()
        .map_err(|e| e.to_string())?
        .to_owned();
    let e1 = embeddings["image_embeddings.1"]
        .try_extract_array::<f32>()
        .map_err(|e| e.to_string())?
        .to_owned();
    let e2 = embeddings["image_embeddings.2"]
        .try_extract_array::<f32>()
        .map_err(|e| e.to_string())?
        .to_owned();
    drop(embeddings);
    println!("SAM 2.1 base+ image encoder: {:.1} ms", ms(t));
    let point = vec![o.click.0 * SIDE as f32, o.click.1 * SIDE as f32];
    let t = Instant::now();
    let decoded = decoder
        .run(ort::inputs![
            "input_points" => Tensor::from_array(([1usize, 1, 1, 2], point)).map_err(|e| e.to_string())?,
            "input_labels" => Tensor::from_array(([1usize, 1, 1], vec![1i64])).map_err(|e| e.to_string())?,
            "input_boxes" => match o.boxed {
                Some(b) => Tensor::from_array(([1usize, 1, 4], b.iter().map(|v| v * SIDE as f32).collect::<Vec<_>>())),
                None => Tensor::from_array(([1usize, 0, 4], Vec::<f32>::new())),
            }
            .map_err(|e| e.to_string())?,
            "image_embeddings.0" => Tensor::from_array(e0).map_err(|e| e.to_string())?,
            "image_embeddings.1" => Tensor::from_array(e1).map_err(|e| e.to_string())?,
            "image_embeddings.2" => Tensor::from_array(e2).map_err(|e| e.to_string())?,
        ])
        .map_err(|e| e.to_string())?;
    let scores = decoded["iou_scores"]
        .try_extract_array::<f32>()
        .map_err(|e| e.to_string())?;
    let best = (0..3)
        .max_by(|&a, &b| scores[[0, 0, a]].total_cmp(&scores[[0, 0, b]]))
        .unwrap_or(0);
    let masks = decoded["pred_masks"]
        .try_extract_array::<f32>()
        .map_err(|e| e.to_string())?;
    let logits: Vec<f32> = (0..256 * 256)
        .map(|i| masks[[0, 0, best, i / 256, i % 256]])
        .collect();
    println!(
        "SAM 2.1 decoder (one click): {:.1} ms, mask {best} of 3, predicted IoU {:.3}",
        ms(t),
        scores[[0, 0, best]]
    );
    drop(decoded);

    // 2. Upsampled to full resolution: the coarse probability.
    let t = Instant::now();
    let (wu, hu) = (w as usize, h as usize);
    let mut coarse = vec![0f32; wu * hu];
    let sx = 256.0 / w as f32;
    let sy = 256.0 / h as f32;
    std::thread::scope(|scope| {
        for (band, rows) in coarse.chunks_mut(wu * hu.div_ceil(16)).enumerate() {
            let logits = &logits;
            scope.spawn(move || {
                let first = band * hu.div_ceil(16);
                for (r, row) in rows.chunks_mut(wu).enumerate() {
                    let fy = ((first + r) as f32 + 0.5) * sy - 0.5;
                    let y0 = fy.floor().clamp(0.0, 255.0) as usize;
                    let y1 = (y0 + 1).min(255);
                    let ty = (fy - y0 as f32).clamp(0.0, 1.0);
                    for (x, out) in row.iter_mut().enumerate() {
                        let fx = (x as f32 + 0.5) * sx - 0.5;
                        let x0 = fx.floor().clamp(0.0, 255.0) as usize;
                        let x1 = (x0 + 1).min(255);
                        let tx = (fx - x0 as f32).clamp(0.0, 1.0);
                        let at = |y: usize, x: usize| logits[y * 256 + x];
                        let top = at(y0, x0) * (1.0 - tx) + at(y0, x1) * tx;
                        let bottom = at(y1, x0) * (1.0 - tx) + at(y1, x1) * tx;
                        let l = top * (1.0 - ty) + bottom * ty;
                        *out = 1.0 / (1.0 + (-l).exp());
                    }
                }
            });
        }
    });
    println!("upsample to full resolution: {:.0} ms", ms(t));

    // 3. The uncertain band, by tiles.
    let t = Instant::now();
    let (cols, rows) = (wu.div_ceil(TILE), hu.div_ceil(TILE));
    let mut band_tiles = Vec::new();
    let mut band_pixels = 0usize;
    for ty in 0..rows {
        for tx in 0..cols {
            let mut n = 0;
            for y in ty * TILE..((ty + 1) * TILE).min(hu) {
                for x in tx * TILE..((tx + 1) * TILE).min(wu) {
                    let p = coarse[y * wu + x];
                    if p > 0.01 && p < 0.99 {
                        n += 1;
                    }
                }
            }
            if n > 0 {
                band_tiles.push((tx, ty));
                band_pixels += n;
            }
        }
    }
    println!(
        "uncertain band: {:.1}% of the pixels, {} of {} tiles ({:.0} ms)",
        100.0 * band_pixels as f64 / (wu * hu) as f64,
        band_tiles.len(),
        cols * rows,
        ms(t)
    );

    // 4. Guided filter on the band tiles, guided by the luminance.
    let t = Instant::now();
    let luma: Vec<f32> = photo
        .pixels()
        .map(|p| {
            (0.2126 * f32::from(p[0]) + 0.7152 * f32::from(p[1]) + 0.0722 * f32::from(p[2])) / 255.0
        })
        .collect();
    let mut refined = coarse
        .iter()
        .map(|&p| if p >= 0.5 { 1.0 } else { 0.0 })
        .collect::<Vec<f32>>();
    let r = o.radius;
    let results: Vec<((usize, usize), Vec<f32>)> = std::thread::scope(|scope| {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let chunk = band_tiles.len().div_ceil(threads).max(1);
        let workers: Vec<_> = band_tiles
            .chunks(chunk)
            .map(|tiles| {
                let (luma, coarse) = (&luma, &coarse);
                scope.spawn(move || {
                    tiles
                        .iter()
                        .map(|&(tx, ty)| {
                            (
                                (tx, ty),
                                guided_tile(luma, coarse, wu, hu, tx, ty, r, o.eps),
                            )
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().unwrap_or_default())
            .collect()
    });
    for ((tx, ty), values) in results {
        for y in ty * TILE..((ty + 1) * TILE).min(hu) {
            for x in tx * TILE..((tx + 1) * TILE).min(wu) {
                let p = coarse[y * wu + x];
                if p > 0.01 && p < 0.99 {
                    refined[y * wu + x] = values[(y - ty * TILE) * TILE + x - tx * TILE];
                }
            }
        }
    }
    println!(
        "guided filter on the band (r = {r}, eps = {}): {:.0} ms",
        o.eps,
        ms(t)
    );

    // Crops for a look: coarse and refined masks over the photo, around the click and at the
    // left edge of the hair.
    std::fs::create_dir_all(&o.out).map_err(|e| e.to_string())?;
    let view = |mask: &[f32], name: &str| -> Result<(), String> {
        let small = 1600.0 / w as f32;
        let (vw, vh) = (1600u32, (h as f32 * small) as u32);
        let full: GrayImage = ImageBuffer::from_fn(vw, vh, |x, y| {
            let sx = ((x as f32 + 0.5) / small) as usize;
            let sy = ((y as f32 + 0.5) / small) as usize;
            Luma([(mask[sy.min(hu - 1) * wu + sx.min(wu - 1)] * 255.0) as u8])
        });
        full.save(o.out.join(format!("{name}-mask.png")))
            .map_err(|e| e.to_string())?;
        // A full-resolution crop of the hair, on red.
        let (cw, ch) = (800usize, 800usize);
        let (cx, cy) = (
            (wu as f32 * o.crop.0) as usize,
            (hu as f32 * o.crop.1) as usize,
        );
        let crop: RgbImage = ImageBuffer::from_fn(cw as u32, ch as u32, |x, y| {
            let (px, py) = ((cx + x as usize).min(wu - 1), (cy + y as usize).min(hu - 1));
            let a = mask[py * wu + px];
            let c = photo.get_pixel(px as u32, py as u32);
            let mix = |v: u8, bg: f32| (f32::from(v) * a + bg * (1.0 - a)) as u8;
            Rgb([mix(c[0], 200.0), mix(c[1], 30.0), mix(c[2], 30.0)])
        });
        crop.save(o.out.join(format!("{name}-hair-crop.png")))
            .map_err(|e| e.to_string())
    };
    view(&coarse, "coarse")?;
    view(&refined, "refined")?;

    // The learned alternative on one band tile: BiRefNet lite on the full-resolution 800² crop
    // of the hair edge (resized to its 1024² input), what refining band tiles with a model costs.
    let t = Instant::now();
    let (cx, cy) = (
        (wu as f32 * o.crop.0) as usize,
        (hu as f32 * o.crop.1) as usize,
    );
    let crop = image::imageops::crop_imm(&photo, cx as u32, cy as u32, 800, 800).to_image();
    let input = image::imageops::resize(&crop, SIDE, SIDE, image::imageops::FilterType::Triangle);
    let mut pixels = vec![0f32; 3 * plane];
    for (i, p) in input.pixels().enumerate() {
        for c in 0..3 {
            pixels[c * plane + i] = (f32::from(p[c]) / 255.0 - mean[c]) / std[c];
        }
    }
    let path = models.join("onnx-community/BiRefNet_lite-ONNX/onnx/model_fp16.onnx");
    let mut birefnet = session(&path, ep, level).map_err(|e| e.to_string())?;
    let tensor = Tensor::from_array(([1usize, 3, SIDE as usize, SIDE as usize], pixels))
        .map_err(|e| e.to_string())?;
    birefnet
        .run(ort::inputs!["input_image" => tensor.view()])
        .map_err(|e| e.to_string())?;
    let t_run = Instant::now();
    let out = birefnet
        .run(ort::inputs!["input_image" => tensor.view()])
        .map_err(|e| e.to_string())?;
    let run_ms = ms(t_run);
    let alpha = out["output_image"]
        .try_extract_array::<f32>()
        .map_err(|e| e.to_string())?;
    // BiRefNet decides by itself what the foreground is: it is trusted only where it agrees with
    // the coarse mask where that one is sure (or is its exact opposite, when the selection is not
    // the salient object), and only inside the uncertain band.
    let crop_alpha = |x: usize, y: usize| {
        let (sx, sy) = (x * SIDE as usize / 800, y * SIDE as usize / 800);
        1.0 / (1.0 + (-alpha[[0, 0, sy, sx]]).exp())
    };
    let (mut direct, mut inverse, mut sure) = (0.0f64, 0.0f64, 0usize);
    for y in 0..800usize.min(hu - cy) {
        for x in 0..800usize.min(wu - cx) {
            let p = coarse[(cy + y) * wu + cx + x];
            if p <= 0.01 || p >= 0.99 {
                let truth = if p >= 0.5 { 1.0 } else { 0.0 };
                let a = f64::from(crop_alpha(x, y));
                direct += (a - truth).abs();
                inverse += (1.0 - a - truth).abs();
                sure += 1;
            }
        }
    }
    let (direct, inverse) = (direct / sure.max(1) as f64, inverse / sure.max(1) as f64);
    let choice = if direct <= 0.1 && direct <= inverse {
        "BiRefNet as it is"
    } else if inverse <= 0.1 {
        "BiRefNet inverted"
    } else {
        "the guided filter (BiRefNet disagrees)"
    };
    println!(
        "BiRefNet vs the sure part of the coarse mask: error {direct:.3} direct, {inverse:.3} inverted → {choice}"
    );
    let mut learned = refined.clone();
    for y in 0..800usize.min(hu - cy) {
        for x in 0..800usize.min(wu - cx) {
            let k = (cy + y) * wu + cx + x;
            let p = coarse[k];
            if p > 0.01 && p < 0.99 {
                let a = crop_alpha(x, y);
                if choice.starts_with("BiRefNet as") {
                    learned[k] = a;
                } else if choice.starts_with("BiRefNet inv") {
                    learned[k] = 1.0 - a;
                }
            }
        }
    }
    drop(out);
    println!(
        "BiRefNet lite on one 800² crop: {run_ms:.0} ms warm ({:.0} ms with loading)",
        ms(t)
    );
    view(&learned, "birefnet")?;

    // ViTMatte-S, guided by a trimap from the coarse mask: it mattes whatever is selected.
    let t = Instant::now();
    let path = models.join("Xenova/vitmatte-small-composition-1k/onnx/model.onnx");
    let mut vitmatte = session(&path, ep, level).map_err(|e| e.to_string())?;
    let input_name = vitmatte.inputs()[0].name().to_owned();
    let output_name = vitmatte.outputs()[0].name().to_owned();
    let mut pixels = vec![0f32; 4 * plane];
    for (i, p) in input.pixels().enumerate() {
        for c in 0..3 {
            pixels[c * plane + i] = (f32::from(p[c]) / 255.0 - 0.5) / 0.5;
        }
    }
    let side = SIDE as usize;
    // The trimap: a band of fixed width on both sides of the coarse mask's half-way contour is
    // left for the model to decide; beyond it, sure inside or outside. (Thresholds on the
    // probability leave almost no sure region where the coarse mask is soft, and the model then
    // chooses the foreground itself.)
    let inside: Vec<bool> = (0..side * side)
        .map(|i| {
            let (x, y) = (i % side, i / side);
            let (px, py) = (
                (cx + x * 800 / side).min(wu - 1),
                (cy + y * 800 / side).min(hu - 1),
            );
            coarse[py * wu + px] >= 0.5
        })
        .collect();
    let band = o.trimap_band.max(1);
    let mut sat = vec![0u32; (side + 1) * (side + 1)];
    for y in 0..side {
        let mut row = 0;
        for x in 0..side {
            row += u32::from(inside[y * side + x]);
            sat[(y + 1) * (side + 1) + x + 1] = sat[y * (side + 1) + x + 1] + row;
        }
    }
    let mut unknown = vec![false; side * side];
    for y in 0..side {
        let (ya, yb) = (y.saturating_sub(band), (y + band + 1).min(side));
        for x in 0..side {
            let (xa, xb) = (x.saturating_sub(band), (x + band + 1).min(side));
            let count = sat[yb * (side + 1) + xb] + sat[ya * (side + 1) + xa]
                - sat[ya * (side + 1) + xb]
                - sat[yb * (side + 1) + xa];
            let area = ((xb - xa) * (yb - ya)) as u32;
            unknown[y * side + x] = count > 0 && count < area;
            pixels[3 * plane + y * side + x] = if unknown[y * side + x] {
                128.0 / 255.0
            } else if inside[y * side + x] {
                1.0
            } else {
                0.0
            };
        }
    }
    let tensor =
        Tensor::from_array(([1usize, 4, side, side], pixels)).map_err(|e| e.to_string())?;
    vitmatte
        .run(ort::inputs![input_name.as_str() => tensor.view()])
        .map_err(|e| e.to_string())?;
    let t_run = Instant::now();
    let out = vitmatte
        .run(ort::inputs![input_name.as_str() => tensor.view()])
        .map_err(|e| e.to_string())?;
    let run_ms = ms(t_run);
    let alphas = out[output_name.as_str()]
        .try_extract_array::<f32>()
        .map_err(|e| e.to_string())?;
    let mut matted = refined.clone();
    for y in 0..800usize.min(hu - cy) {
        for x in 0..800usize.min(wu - cx) {
            let k = (cy + y) * wu + cx + x;
            let (sx, sy) = (x * side / 800, y * side / 800);
            matted[k] = if unknown[sy * side + sx] {
                alphas[[0, 0, sy, sx]].clamp(0.0, 1.0)
            } else if inside[sy * side + sx] {
                1.0
            } else {
                0.0
            };
        }
    }
    drop(out);
    println!(
        "ViTMatte-S ({input_name} -> {output_name}) on one 800² crop: {run_ms:.0} ms warm ({:.0} ms with loading)",
        ms(t)
    );
    view(&matted, "vitmatte")?;
    println!("crops in {}", o.out.display());
    Ok(())
}

/// The guided filter over tile (`tx`, `ty`): the coarse probability `p` aligned with the edges
/// of the luminance `i`, over a window of radius `r` (computed with summed areas over the tile
/// and a margin of 2r, enough for the filter's two box passes).
#[allow(clippy::too_many_arguments)]
fn guided_tile(
    i: &[f32],
    p: &[f32],
    w: usize,
    h: usize,
    tx: usize,
    ty: usize,
    r: usize,
    eps: f32,
) -> Vec<f32> {
    let m = 2 * r;
    let x0 = (tx * TILE).saturating_sub(m);
    let y0 = (ty * TILE).saturating_sub(m);
    let x1 = ((tx + 1) * TILE + m).min(w);
    let y1 = ((ty + 1) * TILE + m).min(h);
    let (ww, wh) = (x1 - x0, y1 - y0);
    // Box mean of `f` over the window, from summed areas, clamped at the window's edges.
    let box_mean = |f: &dyn Fn(usize, usize) -> f64| -> Vec<f32> {
        let mut sat = vec![0f64; (ww + 1) * (wh + 1)];
        for y in 0..wh {
            let mut row = 0.0;
            for x in 0..ww {
                row += f(x, y);
                sat[(y + 1) * (ww + 1) + x + 1] = sat[y * (ww + 1) + x + 1] + row;
            }
        }
        let mut out = vec![0f32; ww * wh];
        for y in 0..wh {
            let (ya, yb) = (y.saturating_sub(r), (y + r + 1).min(wh));
            for x in 0..ww {
                let (xa, xb) = (x.saturating_sub(r), (x + r + 1).min(ww));
                let s = sat[yb * (ww + 1) + xb] - sat[ya * (ww + 1) + xb] - sat[yb * (ww + 1) + xa]
                    + sat[ya * (ww + 1) + xa];
                out[y * ww + x] = (s / ((xb - xa) * (yb - ya)) as f64) as f32;
            }
        }
        out
    };
    let at = |x: usize, y: usize| (y0 + y) * w + x0 + x;
    let mean_i = box_mean(&|x, y| f64::from(i[at(x, y)]));
    let mean_p = box_mean(&|x, y| f64::from(p[at(x, y)]));
    let corr_ip = box_mean(&|x, y| f64::from(i[at(x, y)] * p[at(x, y)]));
    let corr_ii = box_mean(&|x, y| f64::from(i[at(x, y)] * i[at(x, y)]));
    let a: Vec<f32> = (0..ww * wh)
        .map(|k| {
            let cov = corr_ip[k] - mean_i[k] * mean_p[k];
            let var = corr_ii[k] - mean_i[k] * mean_i[k];
            cov / (var + eps)
        })
        .collect();
    let b: Vec<f32> = (0..ww * wh).map(|k| mean_p[k] - a[k] * mean_i[k]).collect();
    let mean_a = box_mean(&|x, y| f64::from(a[y * ww + x]));
    let mean_b = box_mean(&|x, y| f64::from(b[y * ww + x]));
    let mut out = vec![0f32; TILE * TILE];
    for y in ty * TILE..((ty + 1) * TILE).min(h) {
        for x in tx * TILE..((tx + 1) * TILE).min(w) {
            let k = (y - y0) * ww + x - x0;
            let q = mean_a[k] * i[y * w + x] + mean_b[k];
            out[(y - ty * TILE) * TILE + x - tx * TILE] = q.clamp(0.0, 1.0);
        }
    }
    out
}
