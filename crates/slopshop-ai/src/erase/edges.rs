//! The Erase tool's colors matched to the photo at the selection's edge. The model reproduces
//! the photo with a drift of its own: about 10 levels (of 255), varying across the image
//! (measured on the integration's test vectors, the reference's output included). Pasted as it
//! is, the fill shows its outline. The difference between the photo and the model's output is
//! measured on a ring just outside the selection, then spread smoothly over the selection (a
//! membrane: the smoothest field with those values on the ring), and added to the output. On
//! the ring the output becomes the photo; inside, the generated texture keeps its detail and
//! takes the tone of what surrounds it.

/// Width of the ring outside the selection where the difference is measured, in pixels.
const RING: usize = 6;
/// Relaxation sweeps per level of the pyramid (coarse to fine).
const SWEEPS: usize = 24;

/// Matches the colors of `output` (the model's, 8-bit RGB) to `input` (the photo it was given)
/// around `mask` (above 127: generated), all `width × height`. Changes `output` everywhere: on
/// the ring it becomes `input`, inside the mask it is shifted smoothly; beyond the ring it is not
/// meant to be used.
pub fn match_edges(output: &mut [u8], input: &[u8], mask: &[u8], width: usize, height: usize) {
    let pixels = width * height;
    if pixels == 0 || !mask.iter().any(|&m| m > 127) {
        return;
    }
    let near = grow(mask, width, height, RING);
    // Level 0: the difference where it is known (the ring), unknown elsewhere.
    let mut known = vec![false; pixels];
    let mut field = vec![[0f32; 3]; pixels];
    for i in 0..pixels {
        if mask[i] <= 127 && near[i] {
            known[i] = true;
            for c in 0..3 {
                field[i][c] = f32::from(input[3 * i + c]) - f32::from(output[3 * i + c]);
            }
        }
    }
    let field = membrane(Level {
        width,
        height,
        known,
        field,
    });
    for (i, d) in field.iter().enumerate() {
        for c in 0..3 {
            let v = f32::from(output[3 * i + c]) + d[c];
            output[3 * i + c] = v.round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// A grid of values, some known (fixed), the others to fill.
struct Level {
    width: usize,
    height: usize,
    known: Vec<bool>,
    field: Vec<[f32; 3]>,
}

/// The smoothest field through the known values (Laplace's equation, approximately): the
/// unknown cells are filled from a coarser level, the known ones averaged into it, then relaxed
/// towards the mean of their neighbours (a cascade from coarse to fine).
fn membrane(level: Level) -> Vec<[f32; 3]> {
    let Level {
        width,
        height,
        known,
        mut field,
    } = level;
    if !known.iter().any(|&k| k) {
        return field;
    }
    if known.iter().all(|&k| k) {
        return field;
    }
    // The coarser level: each cell the mean of the known values under it.
    if width > 2 || height > 2 {
        let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
        let mut sum = vec![[0f32; 3]; cw * ch];
        let mut count = vec![0u32; cw * ch];
        for y in 0..height {
            for x in 0..width {
                let i = y * width + x;
                if known[i] {
                    let j = (y / 2) * cw + x / 2;
                    for c in 0..3 {
                        sum[j][c] += field[i][c];
                    }
                    count[j] += 1;
                }
            }
        }
        let coarse_known: Vec<bool> = count.iter().map(|&n| n > 0).collect();
        let coarse_field = sum
            .iter()
            .zip(&count)
            .map(|(s, &n)| {
                let n = n.max(1) as f32;
                [s[0] / n, s[1] / n, s[2] / n]
            })
            .collect();
        let coarse = membrane(Level {
            width: cw,
            height: ch,
            known: coarse_known,
            field: coarse_field,
        });
        // The unknown cells start from the coarser level (bilinear).
        for y in 0..height {
            for x in 0..width {
                let i = y * width + x;
                if !known[i] {
                    field[i] = bilinear(
                        &coarse,
                        cw,
                        ch,
                        (x as f32 + 0.5) / 2.0,
                        (y as f32 + 0.5) / 2.0,
                    );
                }
            }
        }
    } else {
        // A few cells: the unknown ones take the mean of the known.
        let n = known.iter().filter(|&&k| k).count() as f32;
        let mut mean = [0f32; 3];
        for (f, _) in field.iter().zip(&known).filter(|(_, k)| **k) {
            for c in 0..3 {
                mean[c] += f[c] / n;
            }
        }
        for (f, k) in field.iter_mut().zip(&known) {
            if !k {
                *f = mean;
            }
        }
    }
    // Relaxation (Gauss–Seidel): each unknown cell the mean of its neighbours.
    for _ in 0..SWEEPS {
        for y in 0..height {
            for x in 0..width {
                let i = y * width + x;
                if known[i] {
                    continue;
                }
                let mut sum = [0f32; 3];
                let mut n = 0f32;
                let mut add = |j: usize| {
                    for c in 0..3 {
                        sum[c] += field[j][c];
                    }
                    n += 1.0;
                };
                if x > 0 {
                    add(i - 1);
                }
                if x + 1 < width {
                    add(i + 1);
                }
                if y > 0 {
                    add(i - width);
                }
                if y + 1 < height {
                    add(i + width);
                }
                field[i] = [sum[0] / n, sum[1] / n, sum[2] / n];
            }
        }
    }
    field
}

/// The field at cell coordinates (`x`, `y`), centres at `i + 0.5`, clamped at the edges.
fn bilinear(field: &[[f32; 3]], width: usize, height: usize, x: f32, y: f32) -> [f32; 3] {
    let fx = (x - 0.5).clamp(0.0, (width - 1) as f32);
    let fy = (y - 0.5).clamp(0.0, (height - 1) as f32);
    let (x0, y0) = (fx.floor() as usize, fy.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let at = |x: usize, y: usize| field[y * width + x];
    std::array::from_fn(|c| {
        let top = at(x0, y0)[c] * (1.0 - tx) + at(x1, y0)[c] * tx;
        let bottom = at(x0, y1)[c] * (1.0 - tx) + at(x1, y1)[c] * tx;
        top * (1.0 - ty) + bottom * ty
    })
}

/// The mask grown by `radius` pixels (a square), as booleans.
fn grow(mask: &[u8], width: usize, height: usize, radius: usize) -> Vec<bool> {
    // Separable: rows, then columns.
    let mut rows = vec![false; width * height];
    for y in 0..height {
        let line = &mask[y * width..][..width];
        let mut last: Option<usize> = None;
        // Distance to the nearest set pixel on the left, then on the right.
        for x in 0..width {
            if line[x] > 127 {
                last = Some(x);
            }
            rows[y * width + x] = last.is_some_and(|l| x - l <= radius);
        }
        last = None;
        for x in (0..width).rev() {
            if line[x] > 127 {
                last = Some(x);
            }
            if last.is_some_and(|l| l - x <= radius) {
                rows[y * width + x] = true;
            }
        }
    }
    let mut out = vec![false; width * height];
    for x in 0..width {
        let mut last: Option<usize> = None;
        for y in 0..height {
            if rows[y * width + x] {
                last = Some(y);
            }
            out[y * width + x] = last.is_some_and(|l| y - l <= radius);
        }
        last = None;
        for y in (0..height).rev() {
            if rows[y * width + x] {
                last = Some(y);
            }
            if last.is_some_and(|l| l - y <= radius) {
                out[y * width + x] = true;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w × h` image from a function of the pixel, and a centred square mask of side `side`.
    fn scene(
        w: usize,
        h: usize,
        side: usize,
        f: impl Fn(usize, usize) -> [u8; 3],
    ) -> (Vec<u8>, Vec<u8>) {
        let mut rgb = Vec::with_capacity(w * h * 3);
        let mut mask = Vec::with_capacity(w * h);
        let (x0, y0) = ((w - side) / 2, (h - side) / 2);
        for y in 0..h {
            for x in 0..w {
                rgb.extend_from_slice(&f(x, y));
                let inside = (x0..x0 + side).contains(&x) && (y0..y0 + side).contains(&y);
                mask.push(if inside { 255 } else { 0 });
            }
        }
        (rgb, mask)
    }

    #[test]
    fn a_uniform_drift_is_removed() {
        let (input, mask) = scene(96, 80, 40, |x, y| [(x * 2) as u8, (y * 3) as u8, 120]);
        // The model's output: the photo 10 levels darker, with texture of its own inside.
        let mut output: Vec<u8> = input
            .iter()
            .enumerate()
            .map(|(i, &v)| v.saturating_sub(10).wrapping_add((i % 7) as u8))
            .collect();
        let texture: Vec<i16> = (0..input.len()).map(|i| (i % 7) as i16).collect();
        match_edges(&mut output, &input, &mask, 96, 80);
        for i in 0..96 * 80 {
            if mask[i] > 127 {
                for c in 0..3 {
                    // The drift is gone; the texture stays (its mean, 3, is part of the drift).
                    let want = i16::from(input[3 * i + c]) + texture[3 * i + c] - 3;
                    let got = i16::from(output[3 * i + c]);
                    assert!(
                        (got - want).abs() <= 3,
                        "pixel {i} channel {c}: {got} vs {want}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_ring_becomes_the_photo_and_a_varying_drift_is_followed() {
        // A drift growing left to right (−20 to +20): inside, the correction follows it.
        let (input, mask) = scene(120, 60, 30, |_, _| [128, 128, 128]);
        let drift = |x: usize| -20.0 + 40.0 * x as f32 / 119.0;
        let mut output = Vec::with_capacity(input.len());
        for y in 0..60 {
            for x in 0..120 {
                let v = (128.0 + drift(x)).round() as u8;
                let _ = y;
                output.extend_from_slice(&[v, v, v]);
            }
        }
        match_edges(&mut output, &input, &mask, 120, 60);
        // The ring (just outside the square), and the square, back to the photo's 128.
        for (x, y) in [(43, 30), (76, 30), (60, 13), (60, 30), (50, 20), (70, 40)] {
            let got = i16::from(output[3 * (y * 120 + x)]);
            assert!((got - 128).abs() <= 2, "({x}, {y}): {got}");
        }
    }

    #[test]
    fn without_a_mask_nothing_changes() {
        let (input, _) = scene(16, 16, 4, |x, _| [x as u8, 0, 0]);
        let mut output = vec![50u8; input.len()];
        match_edges(&mut output, &input, &[0; 256], 16, 16);
        assert!(output.iter().all(|&v| v == 50));
    }

    #[test]
    fn growing_takes_every_pixel_within_the_radius() {
        let mut mask = vec![0u8; 10 * 10];
        mask[5 * 10 + 5] = 255;
        let grown = grow(&mask, 10, 10, 2);
        assert!(grown[3 * 10 + 3] && grown[7 * 10 + 7] && grown[5 * 10 + 5]);
        assert!(!grown[2 * 10 + 5] && !grown[5 * 10 + 8]);
    }
}
