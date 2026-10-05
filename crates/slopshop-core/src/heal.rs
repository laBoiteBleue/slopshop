//! The Healing Brush's blend (Poisson): what it paints keeps the texture of its source and takes
//! the tone of where it paints. Across the painted region the colors are the source's plus a
//! smooth correction, the membrane that matches the destination at the region's edge
//! (`Δd = 0` inside, `d = destination − source` around it).

/// Pixels of a region at most on a side solved directly; larger ones start from a coarser
/// solution (multigrid), so that the cost stays in proportion to the pixels.
const DIRECT_SIDE: usize = 24;
/// Relaxation sweeps on each level once started from the coarser one.
const SWEEPS: usize = 30;
/// Sweeps on the coarsest level.
const COARSE_SWEEPS: usize = 400;
/// Over-relaxation (Gauss-Seidel's successive over-relaxation).
const OMEGA: f32 = 1.85;

/// The membrane over a `width × height` grid: where `inside` (row-major), `values` become the
/// harmonic interpolation of the values around them (those not inside stay as given). A pixel
/// inside on the grid's edge sees no neighbour beyond it.
pub fn membrane(width: usize, height: usize, inside: &[bool], values: &mut [[f32; 4]]) {
    assert_eq!(inside.len(), width * height, "one flag per pixel");
    assert_eq!(values.len(), width * height, "one value per pixel");
    if width == 0 || height == 0 || !inside.iter().any(|&i| i) {
        return;
    }
    if width.max(height) > DIRECT_SIDE {
        // Start from the solution at half the size.
        let (cw, ch) = (width.div_ceil(2), height.div_ceil(2));
        let mut coarse_inside = vec![false; cw * ch];
        let mut coarse = vec![[0f32; 4]; cw * ch];
        for cy in 0..ch {
            for cx in 0..cw {
                let mut sum = [0f32; 4];
                let mut known = 0;
                let mut all_inside = true;
                for (x, y) in
                    [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(dx, dy)| (cx * 2 + dx, cy * 2 + dy))
                {
                    if x >= width || y >= height {
                        continue;
                    }
                    let i = y * width + x;
                    if !inside[i] {
                        all_inside = false;
                        for (s, v) in sum.iter_mut().zip(values[i]) {
                            *s += v;
                        }
                        known += 1;
                    }
                }
                // A coarse pixel is inside only if all its pixels are, so that the region never
                // grows over the values around it; else it holds their mean.
                let c = cy * cw + cx;
                coarse_inside[c] = all_inside;
                if known > 0 {
                    coarse[c] = sum.map(|s| s / known as f32);
                }
            }
        }
        membrane(cw, ch, &coarse_inside, &mut coarse);
        for y in 0..height {
            for x in 0..width {
                let i = y * width + x;
                if inside[i] {
                    values[i] = coarse[(y / 2) * cw + x / 2];
                }
            }
        }
        relax(width, height, inside, values, SWEEPS);
    } else {
        relax(width, height, inside, values, COARSE_SWEEPS);
    }
}

/// `sweeps` of over-relaxed Gauss-Seidel on the pixels inside.
fn relax(width: usize, height: usize, inside: &[bool], values: &mut [[f32; 4]], sweeps: usize) {
    for _ in 0..sweeps {
        for y in 0..height {
            for x in 0..width {
                let i = y * width + x;
                if !inside[i] {
                    continue;
                }
                let mut sum = [0f32; 4];
                let mut n = 0.0;
                let mut add = |j: usize| {
                    for (s, v) in sum.iter_mut().zip(values[j]) {
                        *s += v;
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
                if n == 0.0 {
                    continue;
                }
                let v = &mut values[i];
                for (c, s) in v.iter_mut().zip(sum) {
                    *c += OMEGA * (s / n - *c);
                }
            }
        }
    }
}

/// The healed colors of the pixels of a `width × height` area: `source` the colors taken
/// (texture), `destination` what is there (tone), both premultiplied, row-major; `inside` the
/// region painted. Inside: the source plus the membrane matching the destination around it;
/// outside: the destination.
pub fn healed(
    width: usize,
    height: usize,
    inside: &[bool],
    source: &[[f32; 4]],
    destination: &[[f32; 4]],
) -> Vec<[f32; 4]> {
    let mut difference: Vec<[f32; 4]> = destination
        .iter()
        .zip(source)
        .map(|(d, s)| std::array::from_fn(|c| d[c] - s[c]))
        .collect();
    membrane(width, height, inside, &mut difference);
    (0..width * height)
        .map(|i| {
            if inside[i] {
                std::array::from_fn(|c| source[i][c] + difference[i][c])
            } else {
                destination[i]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_membrane_is_flat_between_equal_edges_and_linear_between_two() {
        // A row: 0 on the left, 10 on the right, 9 pixels inside.
        let (w, h) = (11, 1);
        let inside: Vec<bool> = (0..w).map(|x| x > 0 && x < w - 1).collect();
        let mut values = vec![[0f32; 4]; w];
        values[w - 1] = [10.0; 4];
        membrane(w, h, &inside, &mut values);
        for (x, v) in values.iter().enumerate() {
            assert!((v[0] - x as f32).abs() < 0.01, "{x}: {v:?}");
        }
        // A hole in a uniform area takes its value, at any size.
        for side in [8, 60, 200] {
            let inside: Vec<bool> = (0..side * side)
                .map(|i| {
                    let (x, y) = (i % side, i / side);
                    x > 2 && y > 2 && x < side - 3 && y < side - 3
                })
                .collect();
            let mut values = vec![[0.5f32, 0.25, 0.75, 1.0]; side * side];
            for (v, &i) in values.iter_mut().zip(&inside) {
                if i {
                    *v = [9.0; 4];
                }
            }
            membrane(side, side, &inside, &mut values);
            for v in &values {
                assert!(
                    v.iter()
                        .zip([0.5, 0.25, 0.75, 1.0])
                        .all(|(a, b)| (a - b).abs() < 0.01),
                    "{side}: {v:?}"
                );
            }
        }
    }

    #[test]
    fn healing_keeps_the_texture_and_takes_the_tone() {
        // A dark area with a speck, healed from a bright textured source: the speck's pixels
        // take the darkness around them and keep the source's ups and downs.
        let (w, h) = (40, 40);
        let inside: Vec<bool> = (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                (10..30).contains(&x) && (10..30).contains(&y)
            })
            .collect();
        let texture = |i: usize| {
            if (i % w + i / w).is_multiple_of(2) {
                0.05
            } else {
                -0.05
            }
        };
        let source: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let v = 0.8 + texture(i);
                [v, v, v, 1.0]
            })
            .collect();
        let destination: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                if inside[i] {
                    [1.0, 0.0, 0.0, 1.0]
                } else {
                    [0.2, 0.2, 0.2, 1.0]
                }
            })
            .collect();
        let result = healed(w, h, &inside, &source, &destination);
        for i in 0..w * h {
            if inside[i] {
                // Around 0.2 (the tone), with the source's ±0.05 (the texture).
                let want = 0.2 + texture(i);
                assert!((result[i][0] - want).abs() < 0.03, "{i}: {:?}", result[i]);
                assert!((result[i][1] - want).abs() < 0.03);
            } else {
                assert_eq!(result[i], destination[i]);
            }
        }
    }
}
