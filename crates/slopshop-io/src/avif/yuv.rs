//! YUV pictures to RGB samples (H.273): the matrix (identity, BT.601, BT.709, BT.2020 and a few
//! older ones), full or limited range, chroma upsampled bilinearly with centered siting. Rows
//! are converted in parallel.

use super::ffi::{Chroma, Picture};

/// Where a picture goes in an interleaved output image of `channels` samples per pixel, each in
/// `0..=max` (255 or 65535).
pub(super) struct Target<'a> {
    pub out: &'a mut [u16],
    pub width: usize,
    pub height: usize,
    pub channels: usize,
    pub max: f32,
    /// Where the picture's top-left pixel goes (grid tiles).
    pub x0: usize,
    pub y0: usize,
}

/// `Kr` and `Kb` of a YCbCr matrix (H.273 code point); 2 (unspecified) is taken as BT.601, as
/// libavif does. `None` for unsupported matrices.
fn coefficients(matrix: u8) -> Option<(f32, f32)> {
    match matrix {
        1 => Some((0.2126, 0.0722)),
        2 | 5 | 6 => Some((0.299, 0.114)),
        4 => Some((0.30, 0.11)),
        7 => Some((0.212, 0.087)),
        9 | 10 => Some((0.2627, 0.0593)),
        _ => None,
    }
}

/// Bilinear sample of a plane of `w × h` at (`fx`, `fy`), positions clamped to the plane.
fn bilinear(plane: &[u16], w: usize, h: usize, fx: f32, fy: f32) -> f32 {
    let fx = fx.clamp(0.0, (w - 1) as f32);
    let fy = fy.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (fx as usize, fy as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let at = |x: usize, y: usize| f32::from(plane[y * w + x]);
    let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
    let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
    top + (bottom - top) * ty
}

/// Write the color of `picture` into channels `0..3` (or channel 0 for gray targets), with
/// the given matrix and range (from the container, or the picture's own header).
pub(super) fn color(
    picture: &Picture,
    matrix: u8,
    full_range: bool,
    target: &mut Target<'_>,
) -> Result<(), String> {
    let gray_target = target.channels < 3;
    let mono = picture.chroma == Chroma::Monochrome;
    let identity = matrix == 0 && !mono;
    let kr_kb = if mono || identity {
        None
    } else {
        Some(coefficients(matrix).ok_or(format!("AVIF matrix coefficients {matrix}"))?)
    };
    let depth_scale = (1u32 << (picture.depth - 8)) as f32;
    let max_in = ((1u32 << picture.depth) - 1) as f32;
    // Luma and chroma to [0, 1] and [−½, ½].
    let luma = |v: f32| {
        if full_range {
            v / max_in
        } else {
            (v - 16.0 * depth_scale) / (219.0 * depth_scale)
        }
    };
    let chroma = |v: f32| {
        if full_range {
            (v - 128.0 * depth_scale) / max_in
        } else {
            (v - 128.0 * depth_scale) / (224.0 * depth_scale)
        }
    };
    let (w, h) = (picture.width as usize, picture.height as usize);
    let (cw, ch) = picture.plane_size(1);
    let (sx, sy) = match picture.chroma {
        Chroma::I420 => (2.0, 2.0),
        Chroma::I422 => (2.0, 1.0),
        _ => (1.0, 1.0),
    };
    let (channels, max) = (target.channels, target.max);
    let quantize = |v: f32| (v.clamp(0.0, 1.0) * max).round() as u16;
    rows_in_parallel(target, w, h, |y, row| {
        for (x, px) in row.chunks_exact_mut(channels).enumerate() {
            let i = y * w + x;
            let yv = f32::from(picture.planes[0][i]);
            if mono {
                let v = quantize(luma(yv));
                px[0] = v;
                if !gray_target {
                    px[1] = v;
                    px[2] = v;
                }
                continue;
            }
            let (fx, fy) = ((x as f32 + 0.5) / sx - 0.5, (y as f32 + 0.5) / sy - 0.5);
            let u = bilinear(&picture.planes[1], cw, ch, fx, fy);
            let v = bilinear(&picture.planes[2], cw, ch, fx, fy);
            let (r, g, b) = match kr_kb {
                // GBR: Y is green, U blue, V red, all with the luma range.
                None => (luma(v), luma(yv), luma(u)),
                Some((kr, kb)) => {
                    let (l, cb, cr) = (luma(yv), chroma(u), chroma(v));
                    let r = l + 2.0 * (1.0 - kr) * cr;
                    let b = l + 2.0 * (1.0 - kb) * cb;
                    let g = (l - kr * r - kb * b) / (1.0 - kr - kb);
                    (r, g, b)
                }
            };
            if gray_target {
                px[0] = quantize(g);
            } else {
                px[0] = quantize(r);
                px[1] = quantize(g);
                px[2] = quantize(b);
            }
        }
    });
    Ok(())
}

/// Write the luma of `picture` (an alpha plane) into the last channel.
pub(super) fn alpha(picture: &Picture, full_range: bool, target: &mut Target<'_>) {
    let max_in = ((1u32 << picture.depth) - 1) as f32;
    let depth_scale = (1u32 << (picture.depth - 8)) as f32;
    let (channels, max) = (target.channels, target.max);
    let w = picture.width as usize;
    rows_in_parallel(target, w, picture.height as usize, |y, row| {
        for (x, px) in row.chunks_exact_mut(channels).enumerate() {
            let v = f32::from(picture.planes[0][y * w + x]);
            let a = if full_range {
                v / max_in
            } else {
                (v - 16.0 * depth_scale) / (219.0 * depth_scale)
            };
            px[channels - 1] = (a.clamp(0.0, 1.0) * max).round() as u16;
        }
    });
}

/// Call `f(y, pixels)` for each row `y` of a `w × h` picture placed in `target`, with the
/// target's samples of that row the picture covers (rows and columns outside are skipped).
fn rows_in_parallel(
    target: &mut Target<'_>,
    w: usize,
    h: usize,
    f: impl Fn(usize, &mut [u16]) + Sync,
) {
    let stride = target.width * target.channels;
    let rows = h.min(target.height.saturating_sub(target.y0));
    let cols = w.min(target.width.saturating_sub(target.x0));
    if rows == 0 || cols == 0 {
        return;
    }
    let start = target.y0 * stride;
    let area = &mut target.out[start..start + rows * stride];
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let rows_per_thread = rows.div_ceil(threads).max(16);
    let (x0, channels) = (target.x0, target.channels);
    std::thread::scope(|scope| {
        for (chunk, part) in area.chunks_mut(rows_per_thread * stride).enumerate() {
            let f = &f;
            scope.spawn(move || {
                for (i, row) in part.chunks_mut(stride).enumerate() {
                    let y = chunk * rows_per_thread + i;
                    f(y, &mut row[x0 * channels..(x0 + cols) * channels]);
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(chroma: Chroma, planes: Vec<Vec<u16>>, w: u32, h: u32, depth: u8) -> Picture {
        Picture {
            width: w,
            height: h,
            depth,
            chroma,
            planes,
            primaries: 1,
            transfer: 13,
            matrix: 1,
            full_range: true,
        }
    }

    fn convert(p: &Picture, matrix: u8, full: bool, channels: usize) -> Vec<u16> {
        let (w, h) = (p.width as usize, p.height as usize);
        let mut out = vec![0; w * h * channels];
        let mut target = Target {
            out: &mut out,
            width: w,
            height: h,
            channels,
            max: 255.0,
            x0: 0,
            y0: 0,
        };
        color(p, matrix, full, &mut target).unwrap();
        out
    }

    #[test]
    fn neutral_chroma_gives_gray_and_ranges_map_to_black_and_white() {
        // Full range: Y 0 and 255 with neutral chroma are black and white.
        let full = picture(
            Chroma::I444,
            vec![vec![0, 255], vec![128, 128], vec![128, 128]],
            2,
            1,
            8,
        );
        assert_eq!(convert(&full, 1, true, 3), [0, 0, 0, 255, 255, 255]);
        // Limited range: 16 and 235.
        let limited = picture(
            Chroma::I444,
            vec![vec![16, 235], vec![128, 128], vec![128, 128]],
            2,
            1,
            8,
        );
        assert_eq!(convert(&limited, 1, false, 3), [0, 0, 0, 255, 255, 255]);
        // 10-bit full range to 8-bit samples.
        let deep = picture(
            Chroma::I444,
            vec![vec![1023], vec![512], vec![512]],
            1,
            1,
            10,
        );
        assert_eq!(convert(&deep, 9, true, 3), [255, 255, 255]);
    }

    #[test]
    fn bt709_red_and_identity_matrices() {
        // BT.709 red (full range): Y 0.2126, Cb −0.1146, Cr 0.5.
        let y = (0.2126f32 * 255.0).round() as u16;
        let cb = (128.0 - 0.1146f32 * 255.0).round() as u16;
        let red = picture(Chroma::I444, vec![vec![y], vec![cb], vec![255]], 1, 1, 8);
        let out = convert(&red, 1, true, 3);
        assert!(out[0] >= 253 && out[1] <= 2 && out[2] <= 2, "{out:?}");
        // GBR: Y green, U blue, V red.
        let gbr = picture(Chroma::I444, vec![vec![10], vec![20], vec![30]], 1, 1, 8);
        assert_eq!(convert(&gbr, 0, true, 3), [30, 10, 20]);
    }

    #[test]
    fn subsampled_chroma_is_interpolated_and_gray_targets_keep_luma() {
        // 4:2:0, 4 × 2: two chroma samples across.
        let p = picture(
            Chroma::I420,
            vec![vec![128; 8], vec![128, 128], vec![0, 255]],
            4,
            2,
            8,
        );
        let out = convert(&p, 1, true, 3);
        let reds: Vec<u16> = out.chunks(3).take(4).map(|px| px[0]).collect();
        // From cyan-ish to red across the row, monotonically.
        assert!(reds.windows(2).all(|w| w[0] <= w[1]), "{reds:?}");
        let mono = picture(Chroma::Monochrome, vec![vec![0, 128, 255]], 3, 1, 8);
        assert_eq!(convert(&mono, 2, true, 1), [0, 128, 255]);
        assert_eq!(convert(&mono, 2, true, 3)[3..6], [128, 128, 128]);
    }
}
