//! Pillow's resizing of 8-bit images, reproduced exactly (`Image.resize` with `LANCZOS` and
//! `NEAREST`, Pillow's `libImaging/Resample.c` and `Geometry.c`): the Erase tool's reference
//! resizes the photo, the mask and the result this way, and its test vectors depend on it.

/// Lanczos (a = 3) as Pillow's `LANCZOS`: two separable passes in 22-bit fixed point, each
/// rounded to 8 bits, horizontal first. `pixels` holds `channels` interleaved samples per pixel.
pub fn resize_lanczos(
    pixels: &[u8],
    channels: usize,
    (width, height): (usize, usize),
    (new_width, new_height): (usize, usize),
) -> Vec<u8> {
    let horizontal = new_width != width;
    let vertical = new_height != height;
    let (ksize_v, mut bounds_v, kk_v) = coefficients(height, new_height);
    let (ksize_h, bounds_h, kk_h) = coefficients(width, new_width);

    // The horizontal pass covers only the rows the vertical one reads.
    let first = bounds_v[0].0;
    let last = bounds_v[new_height - 1].0 + bounds_v[new_height - 1].1;
    let mut current: Vec<u8>;
    let mut rows = height;
    let source: &[u8] = if horizontal {
        for b in &mut bounds_v {
            b.0 -= first;
        }
        rows = last - first;
        current = vec![0; new_width * rows * channels];
        for y in 0..rows {
            let line = &pixels[(y + first) * width * channels..][..width * channels];
            let out = &mut current[y * new_width * channels..][..new_width * channels];
            for (xx, &(xmin, count)) in bounds_h.iter().enumerate() {
                let k = &kk_h[xx * ksize_h..][..count];
                for c in 0..channels {
                    let mut ss = HALF;
                    for (x, &w) in k.iter().enumerate() {
                        ss += i32::from(line[(x + xmin) * channels + c]) * w;
                    }
                    out[xx * channels + c] = clip8(ss);
                }
            }
        }
        &current
    } else {
        pixels
    };
    let row_width = if horizontal { new_width } else { width };
    if !vertical {
        return if horizontal {
            source.to_vec()
        } else {
            pixels.to_vec()
        };
    }
    debug_assert!(rows >= bounds_v.iter().map(|b| b.0 + b.1).max().unwrap_or(0));
    let stride = row_width * channels;
    let mut out = vec![0; stride * new_height];
    for (yy, &(ymin, count)) in bounds_v.iter().enumerate() {
        let k = &kk_v[yy * ksize_v..][..count];
        let line = &mut out[yy * stride..][..stride];
        for (i, sample) in line.iter_mut().enumerate() {
            let mut ss = HALF;
            for (y, &w) in k.iter().enumerate() {
                ss += i32::from(source[(y + ymin) * stride + i]) * w;
            }
            *sample = clip8(ss);
        }
    }
    out
}

/// Nearest neighbour as Pillow's `NEAREST` (its affine scaling: source positions accumulated
/// in double precision from the first pixel's centre, then truncated).
pub fn resize_nearest(
    pixels: &[u8],
    channels: usize,
    (width, height): (usize, usize),
    (new_width, new_height): (usize, usize),
) -> Vec<u8> {
    let positions = |from: usize, to: usize| {
        let step = from as f32 as f64 / to as f64;
        let mut at = step * 0.5;
        (0..to)
            .map(|_| {
                let i = (at as usize).min(from - 1);
                at += step;
                i
            })
            .collect::<Vec<_>>()
    };
    let (xs, ys) = (positions(width, new_width), positions(height, new_height));
    let mut out = Vec::with_capacity(new_width * new_height * channels);
    for &y in &ys {
        for &x in &xs {
            out.extend_from_slice(&pixels[(y * width + x) * channels..][..channels]);
        }
    }
    out
}

const PRECISION_BITS: u32 = 32 - 8 - 2;
const HALF: i32 = 1 << (PRECISION_BITS - 1);

fn clip8(ss: i32) -> u8 {
    (ss >> PRECISION_BITS).clamp(0, 255) as u8
}

fn lanczos(x: f64) -> f64 {
    fn sinc(x: f64) -> f64 {
        if x == 0.0 {
            1.0
        } else {
            let x = x * std::f64::consts::PI;
            x.sin() / x
        }
    }
    if (-3.0..3.0).contains(&x) {
        sinc(x) * sinc(x / 3.0)
    } else {
        0.0
    }
}

/// Pillow's `precompute_coeffs` then `normalize_coeffs_8bpc`: per output sample, the first
/// input sample and how many it weighs, and the weights in fixed point (`ksize` per sample).
fn coefficients(in_size: usize, out_size: usize) -> (usize, Vec<(usize, usize)>, Vec<i32>) {
    let scale = in_size as f32 as f64 / out_size as f64;
    let filterscale = scale.max(1.0);
    let support = 3.0 * filterscale;
    let ksize = support.ceil() as usize * 2 + 1;
    let inv = 1.0 / filterscale;
    let mut bounds = Vec::with_capacity(out_size);
    let mut kk = vec![0; out_size * ksize];
    let mut weights = vec![0.0; ksize];
    for xx in 0..out_size {
        let center = (xx as f64 + 0.5) * scale;
        // C's `(int)` truncates toward zero.
        let xmin = ((center - support + 0.5) as i64).max(0) as usize;
        let xmax = ((center + support + 0.5) as i64).min(in_size as i64) as usize - xmin;
        let mut sum = 0.0;
        for (x, w) in weights.iter_mut().enumerate().take(xmax) {
            *w = lanczos((x as f64 + xmin as f64 - center + 0.5) * inv);
            sum += *w;
        }
        for (x, &w) in weights.iter().enumerate().take(xmax) {
            let w = if sum != 0.0 { w / sum } else { w };
            let fixed = w * f64::from(1u32 << PRECISION_BITS);
            kk[xx * ksize + x] = if w < 0.0 {
                (fixed - 0.5) as i32
            } else {
                (fixed + 0.5) as i32
            };
        }
        bounds.push((xmin, xmax));
    }
    (ksize, bounds, kk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_size_is_a_copy() {
        let px: Vec<u8> = (0..48).collect();
        assert_eq!(resize_lanczos(&px, 3, (4, 4), (4, 4)), px);
        assert_eq!(resize_nearest(&px, 3, (4, 4), (4, 4)), px);
    }

    #[test]
    fn a_flat_image_stays_flat() {
        let px = vec![77u8; 3 * 37 * 23];
        assert!(
            resize_lanczos(&px, 3, (37, 23), (16, 11))
                .iter()
                .all(|&v| v == 77)
        );
        assert!(
            resize_lanczos(&px, 3, (37, 23), (90, 50))
                .iter()
                .all(|&v| v == 77)
        );
    }

    /// Lanczos in one pass, in double precision, from the definition (the filter widened by the
    /// scale when reducing, weights normalized, edges clipped).
    fn direct(src: &[u8], (w, h): (usize, usize), (nw, nh): (usize, usize)) -> Vec<f64> {
        let weights = |from: usize, to: usize, o: usize| {
            let scale = from as f64 / to as f64;
            let fs = scale.max(1.0);
            let center = (o as f64 + 0.5) * scale;
            let w: Vec<f64> = (0..from)
                .map(|i| lanczos((i as f64 + 0.5 - center) / fs))
                .collect();
            let sum: f64 = w.iter().sum();
            w.into_iter().map(|v| v / sum).collect::<Vec<_>>()
        };
        let mut out = Vec::new();
        for oy in 0..nh {
            let wy = weights(h, nh, oy);
            for ox in 0..nw {
                let wx = weights(w, nw, ox);
                let mut v = 0.0;
                for y in 0..h {
                    for x in 0..w {
                        v += wy[y] * wx[x] * f64::from(src[y * w + x]);
                    }
                }
                out.push(v);
            }
        }
        out
    }

    #[test]
    fn lanczos_follows_its_definition_within_the_8_bit_roundings() {
        let src: Vec<u8> = (0..23)
            .flat_map(|y| (0..37).map(move |x| ((x * 37 + y * 11) % 200 + 20) as u8))
            .collect();
        for to in [(16, 11), (37, 9), (90, 50), (5, 23)] {
            let got = resize_lanczos(&src, 1, (37, 23), to);
            let want = direct(&src, (37, 23), to);
            for (g, w) in got.iter().zip(&want) {
                // Two passes each rounded to 8 bits (and clipped): within 1.5 of the exact value.
                assert!(
                    (f64::from(*g) - w.clamp(0.0, 255.0)).abs() <= 1.5,
                    "{to:?}: {g} vs {w}"
                );
            }
        }
    }

    #[test]
    fn channels_are_resized_independently() {
        let rgb: Vec<u8> = (0..3 * 10 * 6).map(|i| (i * 7 % 256) as u8).collect();
        let together = resize_lanczos(&rgb, 3, (10, 6), (4, 9));
        for c in 0..3 {
            let plane: Vec<u8> = rgb.iter().skip(c).step_by(3).copied().collect();
            let alone = resize_lanczos(&plane, 1, (10, 6), (4, 9));
            let picked: Vec<u8> = together.iter().skip(c).step_by(3).copied().collect();
            assert_eq!(picked, alone);
        }
    }

    #[test]
    fn nearest_takes_the_pixel_under_each_centre() {
        // 5 → 2: centres at 1.25 and 3.75 → pixels 1 and 3.
        let px = [0, 10, 20, 30, 40];
        assert_eq!(resize_nearest(&px, 1, (5, 1), (2, 1)), vec![10, 30]);
        // 2 → 5: centres at 0.2, 0.6, 1.0, 1.4, 1.8.
        assert_eq!(
            resize_nearest(&[1, 2], 1, (2, 1), (5, 1)),
            vec![1, 1, 2, 2, 2]
        );
    }
}
