//! Quick Selection by color (ADR 0026): a brush stroke says "this is inside" (or with Alt
//! "outside"), and the selection grows to the region of similar colors bounded by the image's
//! edges, like Photoshop's tool. In the manner of Liu, Sun and Shum's Paint Selection (2009): a
//! color model of the stroke and one of the rest (Gaussian mixtures), and a minimum cut between
//! them where neighbors of different colors are cheap to separate. Works on the colors as
//! displayed (8-bit sRGB), at a resolution the caller chooses (the view's, at most a few
//! megapixels); the result is combined with the selection at full resolution.

use crate::maxflow::{Grid, NEIGHBORS};

/// Mixture components per color model (GrabCut's choice).
const COMPONENTS: usize = 5;
/// Samples at most per color model.
const MAX_SAMPLES: usize = 4000;
/// Weight of the edges between neighbors against the color terms (GrabCut's γ).
const SMOOTHNESS: f32 = 50.0;
/// Cost that no cut pays: the stroke's pixels stay where the user painted them.
const HARD: f32 = 1.0e9;
/// Colors are looked up in a table of 32 levels per channel.
const BINS: usize = 32;
/// Longest side of the coarse level cut first.
const COARSE_SIDE: usize = 256;

/// What a stroke does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuickMode {
    /// A new selection: the region of the stroke.
    New,
    /// The region of the stroke joins the selection.
    Add,
    /// The region of the stroke leaves the selection.
    Subtract,
}

/// An image to select in: `width × height` colors as displayed, RGB, row-major.
#[derive(Debug, Clone, Copy)]
pub struct QuickImage<'a> {
    pub width: usize,
    pub height: usize,
    pub rgb: &'a [u8],
}

/// The pixels a stroke changes, on `image`'s grid: `true` where the selection gains (New, Add)
/// or loses (Subtract) coverage. `selected` is the current selection on the same grid (whether
/// each pixel is inside), `stroke` the pixels the brush covered. Empty when nothing changes.
pub fn quick_select(
    image: &QuickImage<'_>,
    selected: &[bool],
    stroke: &[bool],
    mode: QuickMode,
) -> Vec<bool> {
    let (w, h) = (image.width, image.height);
    let n = w * h;
    if n == 0 || image.rgb.len() != n * 3 || selected.len() != n || stroke.len() != n {
        return vec![false; n];
    }
    let color = |p: usize| {
        [
            f32::from(image.rgb[p * 3]),
            f32::from(image.rgb[p * 3 + 1]),
            f32::from(image.rgb[p * 3 + 2]),
        ]
    };
    // Pinned inside (source) and outside (sink); the model of each side.
    let subtract = mode == QuickMode::Subtract;
    let pinned_in = |p: usize| match mode {
        QuickMode::New => stroke[p],
        QuickMode::Add => stroke[p] || selected[p],
        QuickMode::Subtract => false,
    };
    let pinned_out = |p: usize| subtract && (stroke[p] || !selected[p]);
    let inside_samples: Vec<usize> = match mode {
        QuickMode::New | QuickMode::Add => (0..n).filter(|&p| stroke[p]).collect(),
        QuickMode::Subtract => (0..n).filter(|&p| selected[p] && !stroke[p]).collect(),
    };
    let outside_samples: Vec<usize> = match mode {
        QuickMode::New => (0..n).filter(|&p| !stroke[p]).collect(),
        QuickMode::Add => (0..n).filter(|&p| !stroke[p] && !selected[p]).collect(),
        QuickMode::Subtract => (0..n).filter(|&p| stroke[p]).collect(),
    };
    if inside_samples.is_empty() || outside_samples.is_empty() {
        // Nothing to tell apart: the stroke alone.
        return match mode {
            QuickMode::Subtract => (0..n).map(|p| stroke[p] && selected[p]).collect(),
            _ => stroke.to_vec(),
        };
    }
    let inside = Mixture::fit(&sampled(&inside_samples, &color));
    let outside = Mixture::fit(&sampled(&outside_samples, &color));
    let costs = CostTable::new(&inside, &outside);
    let colors: Vec<[f32; 3]> = (0..n).map(color).collect();
    let pins: Vec<Pin> = (0..n)
        .map(|p| {
            if pinned_in(p) {
                Pin::In
            } else if pinned_out(p) {
                Pin::Out
            } else {
                Pin::Free
            }
        })
        .collect();

    // Coarse to fine: the cut on a small version of the image, then at full resolution only in
    // a band around its outline (Lombaert et al., "A Multilevel Banded Graph Cuts Method",
    // 2005). Wide areas of similar colors cost countless augmentations at full resolution.
    let factor = w.max(h).div_ceil(COARSE_SIDE);
    let cut = if factor < 2 {
        cut(&colors, w, h, &costs, &pins)
    } else {
        let (cw, ch) = (w.div_ceil(factor), h.div_ceil(factor));
        let mut coarse_colors = vec![[0.0f32; 3]; cw * ch];
        // Per cell: pixels, painted ones, selected ones.
        let mut counts = vec![[0u32; 3]; cw * ch];
        for p in 0..n {
            let cell = (p / w / factor) * cw + p % w / factor;
            for (sum, v) in coarse_colors[cell].iter_mut().zip(colors[p]) {
                *sum += v;
            }
            counts[cell][0] += 1;
            counts[cell][1] += u32::from(stroke[p]);
            counts[cell][2] += u32::from(selected[p]);
        }
        // A stroke pins its cells; a selection, the cells it mostly covers (or mostly misses).
        let coarse_pins: Vec<Pin> = counts
            .iter()
            .map(|&[count, painted, chosen]| match mode {
                QuickMode::New if painted > 0 => Pin::In,
                QuickMode::Add if painted > 0 || 2 * chosen > count => Pin::In,
                QuickMode::Subtract if painted > 0 || 2 * chosen < count => Pin::Out,
                _ => Pin::Free,
            })
            .collect();
        for (c, &[count, ..]) in coarse_colors.iter_mut().zip(&counts) {
            *c = c.map(|v| v / count.max(1) as f32);
        }
        let coarse = cut(&coarse_colors, cw, ch, &costs, &coarse_pins);
        // Cells near a change of side, and their neighbors: the band refined.
        let mut band = vec![false; cw * ch];
        for cy in 0..ch {
            for cx in 0..cw {
                let side = coarse[cy * cw + cx];
                let edge = (cy.saturating_sub(1)..(cy + 2).min(ch)).any(|y| {
                    (cx.saturating_sub(1)..(cx + 2).min(cw)).any(|x| coarse[y * cw + x] != side)
                });
                if edge {
                    for y in cy.saturating_sub(1)..(cy + 2).min(ch) {
                        for x in cx.saturating_sub(1)..(cx + 2).min(cw) {
                            band[y * cw + x] = true;
                        }
                    }
                }
            }
        }
        let fine_pins: Vec<Pin> = (0..n)
            .map(|p| {
                let cell = (p / w / factor) * cw + p % w / factor;
                match pins[p] {
                    Pin::Free if !band[cell] => {
                        if coarse[cell] {
                            Pin::In
                        } else {
                            Pin::Out
                        }
                    }
                    pin => pin,
                }
            })
            .collect();
        cut(&colors, w, h, &costs, &fine_pins)
    };

    // The region of the stroke: what the cut put on the stroke's side, connected to it.
    let seeds: Vec<usize> = (0..n).filter(|&p| stroke[p]).collect();
    match mode {
        QuickMode::New | QuickMode::Add => {
            let grown = connected(w, h, &seeds, |p| cut[p]);
            (0..n).map(|p| grown[p] && !selected[p]).collect()
        }
        QuickMode::Subtract => {
            let removed = connected(w, h, &seeds, |p| !cut[p] && selected[p]);
            (0..n).map(|p| removed[p] && selected[p]).collect()
        }
    }
}

/// Scores for [`crate::selection::select_scores`] (positive inside) from the pixels a stroke
/// `changed`: one pixel more where the selection already is (Add) or is not (Subtract), so that
/// the region meets the selection at full resolution without a seam.
pub fn change_scores(
    width: usize,
    height: usize,
    changed: &[bool],
    selected: &[bool],
    mode: QuickMode,
) -> Vec<f32> {
    let n = width * height;
    let mut scores: Vec<f32> = changed
        .iter()
        .map(|&c| if c { 1.0 } else { -1.0 })
        .collect();
    if mode == QuickMode::New || changed.len() != n || selected.len() != n {
        return scores;
    }
    for p in 0..n {
        let joins = if mode == QuickMode::Add {
            selected[p]
        } else {
            !selected[p]
        };
        if changed[p] || !joins {
            continue;
        }
        let (x, y) = (p % width, p / width);
        let near = (y.saturating_sub(1)..(y + 2).min(height))
            .any(|qy| (x.saturating_sub(1)..(x + 2).min(width)).any(|qx| changed[qy * width + qx]));
        if near {
            scores[p] = 1.0;
        }
    }
    scores
}

/// How a pixel is held by the cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pin {
    Free,
    In,
    Out,
}

/// The minimum cut of a `w × h` image of `colors`: inside where it pays to be by `costs` and
/// the edges between neighbors of different colors, `pins` held where they say.
fn cut(colors: &[[f32; 3]], w: usize, h: usize, costs: &CostTable, pins: &[Pin]) -> Vec<bool> {
    // β: the inverse of twice the mean squared color difference between neighbors.
    let mut sum = 0.0f64;
    let mut count = 0u64;
    for y in 0..h {
        for x in 0..w {
            let c = colors[y * w + x];
            if x + 1 < w {
                sum += f64::from(distance2(c, colors[y * w + x + 1]));
                count += 1;
            }
            if y + 1 < h {
                sum += f64::from(distance2(c, colors[(y + 1) * w + x]));
                count += 1;
            }
        }
    }
    let beta = if sum > 0.0 {
        (count as f64 / (2.0 * sum)) as f32
    } else {
        0.0
    };
    let mut grid = Grid::new(w, h);
    for (p, &c) in colors.iter().enumerate() {
        // Cutting p from the source puts it outside: it pays the outside cost.
        match pins[p] {
            Pin::In => grid.set_terminals(p, HARD, 0.0),
            Pin::Out => grid.set_terminals(p, 0.0, HARD),
            Pin::Free => {
                let (cost_in, cost_out) = costs.at(c);
                grid.set_terminals(p, cost_out, cost_in);
            }
        }
        for k in 0..NEIGHBORS {
            if let Some(q) = grid.neighbor(p, k) {
                let (dx, dy) = Grid::direction(k);
                let length = if dx != 0 && dy != 0 {
                    std::f32::consts::SQRT_2
                } else {
                    1.0
                };
                let weight = SMOOTHNESS / length * (-beta * distance2(c, colors[q])).exp();
                grid.set_capacity(p, k, weight);
            }
        }
    }
    grid.solve()
}

/// The pixels reachable from `seeds` (8-neighbors) through pixels where `open` holds.
fn connected(w: usize, h: usize, seeds: &[usize], open: impl Fn(usize) -> bool) -> Vec<bool> {
    let mut reached = vec![false; w * h];
    let mut stack: Vec<usize> = seeds.iter().copied().filter(|&p| open(p)).collect();
    for &p in &stack {
        reached[p] = true;
    }
    while let Some(p) = stack.pop() {
        let (x, y) = (p % w, p / w);
        for dy in -1isize..=1 {
            for dx in -1isize..=1 {
                let (Some(qx), Some(qy)) = (x.checked_add_signed(dx), y.checked_add_signed(dy))
                else {
                    continue;
                };
                if qx >= w || qy >= h {
                    continue;
                }
                let q = qy * w + qx;
                if !reached[q] && open(q) {
                    reached[q] = true;
                    stack.push(q);
                }
            }
        }
    }
    reached
}

fn distance2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]) * (a[i] - b[i])).sum()
}

/// At most [`MAX_SAMPLES`] colors of `pixels`, evenly spread (deterministic).
fn sampled(pixels: &[usize], color: &impl Fn(usize) -> [f32; 3]) -> Vec<[f32; 3]> {
    let step = (pixels.len() as f64 / MAX_SAMPLES as f64).max(1.0);
    (0..pixels.len().min(MAX_SAMPLES))
        .map(|i| color(pixels[(i as f64 * step) as usize]))
        .collect()
}

/// A Gaussian mixture over colors.
struct Mixture {
    components: Vec<Component>,
}

struct Component {
    /// Log of the weight over the normalization.
    log_scale: f32,
    mean: [f32; 3],
    /// Inverse covariance.
    inverse: [[f32; 3]; 3],
}

impl Component {
    /// Squared Mahalanobis distance of `c` to the component.
    fn distance2(&self, c: [f32; 3]) -> f32 {
        let d = [0, 1, 2].map(|i| c[i] - self.mean[i]);
        let mut q = 0.0;
        for i in 0..3 {
            for j in 0..3 {
                q += d[i] * self.inverse[i][j] * d[j];
            }
        }
        q
    }
}

impl Mixture {
    /// Fitted to `colors` by k-means (seeded by farthest points), one Gaussian per cluster.
    fn fit(colors: &[[f32; 3]]) -> Self {
        let k = COMPONENTS.min(colors.len()).max(1);
        let mut centers = vec![colors[0]];
        while centers.len() < k {
            let farthest = colors
                .iter()
                .copied()
                .max_by(|a, b| {
                    let da = centers
                        .iter()
                        .map(|c| distance2(*a, *c))
                        .fold(f32::MAX, f32::min);
                    let db = centers
                        .iter()
                        .map(|c| distance2(*b, *c))
                        .fold(f32::MAX, f32::min);
                    da.total_cmp(&db)
                })
                .unwrap_or(colors[0]);
            centers.push(farthest);
        }
        let mut labels = vec![0usize; colors.len()];
        for _ in 0..8 {
            for (label, c) in labels.iter_mut().zip(colors) {
                *label = (0..k)
                    .min_by(|&a, &b| {
                        distance2(*c, centers[a]).total_cmp(&distance2(*c, centers[b]))
                    })
                    .unwrap_or(0);
            }
            let mut sums = vec![[0.0f64; 4]; k];
            for (label, c) in labels.iter().zip(colors) {
                for i in 0..3 {
                    sums[*label][i] += f64::from(c[i]);
                }
                sums[*label][3] += 1.0;
            }
            for (center, sum) in centers.iter_mut().zip(&sums) {
                if sum[3] > 0.0 {
                    *center = [0, 1, 2].map(|i| (sum[i] / sum[3]) as f32);
                }
            }
        }
        let total = colors.len() as f64;
        let components = (0..k)
            .filter_map(|cluster| {
                let members: Vec<&[f32; 3]> = colors
                    .iter()
                    .zip(&labels)
                    .filter(|(_, l)| **l == cluster)
                    .map(|(c, _)| c)
                    .collect();
                if members.is_empty() {
                    return None;
                }
                let m = members.len() as f64;
                let mean =
                    [0, 1, 2].map(|i| members.iter().map(|c| f64::from(c[i])).sum::<f64>() / m);
                let mut cov = [[0.0f64; 3]; 3];
                for c in &members {
                    for i in 0..3 {
                        for j in 0..3 {
                            cov[i][j] += (f64::from(c[i]) - mean[i]) * (f64::from(c[j]) - mean[j]);
                        }
                    }
                }
                for (i, row) in cov.iter_mut().enumerate() {
                    for v in row.iter_mut() {
                        *v /= m;
                    }
                    // Noise of 8-bit values, and colors seen only once or twice.
                    row[i] += 4.0;
                }
                let (inverse, det) = invert3(cov)?;
                let weight = m / total;
                let log_scale =
                    weight.ln() - 0.5 * det.ln() - 1.5 * (2.0 * std::f64::consts::PI).ln();
                Some(Component {
                    log_scale: log_scale as f32,
                    mean: mean.map(|v| v as f32),
                    inverse: inverse.map(|row| row.map(|v| v as f32)),
                })
            })
            .collect();
        Self { components }
    }

    /// −log of the density at `c`.
    fn cost(&self, c: [f32; 3]) -> f32 {
        let logs: Vec<f32> = self
            .components
            .iter()
            .map(|g| g.log_scale - 0.5 * g.distance2(c))
            .collect();
        let max = logs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        if !max.is_finite() {
            return 100.0;
        }
        let sum: f32 = logs.iter().map(|l| (l - max).exp()).sum();
        // Bounded: a color far from both models must not dominate its neighbors.
        (-(max + sum.ln())).min(100.0)
    }
}

/// The inverse and determinant of a symmetric 3×3 matrix, if it is invertible.
fn invert3(m: [[f64; 3]; 3]) -> Option<([[f64; 3]; 3], f64)> {
    let c = |i: usize, j: usize| {
        let (r0, r1) = ((i + 1) % 3, (i + 2) % 3);
        let (c0, c1) = ((j + 1) % 3, (j + 2) % 3);
        m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
    };
    let det = m[0][0] * c(0, 0) + m[0][1] * c(0, 1) + m[0][2] * c(0, 2);
    if det.is_nan() || det <= 1e-12 {
        return None;
    }
    let mut inverse = [[0.0; 3]; 3];
    for (i, row) in inverse.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = c(j, i) / det;
        }
    }
    Some((inverse, det))
}

/// Both models' costs for colors quantized to [`BINS`] levels per channel, computed once.
struct CostTable {
    costs: Vec<(f32, f32)>,
}

impl CostTable {
    fn new(inside: &Mixture, outside: &Mixture) -> Self {
        let step = 256.0 / BINS as f32;
        let center = |i: usize| (i as f32 + 0.5) * step;
        let costs = (0..BINS * BINS * BINS)
            .map(|i| {
                let c = [
                    center(i / (BINS * BINS)),
                    center(i / BINS % BINS),
                    center(i % BINS),
                ];
                (inside.cost(c), outside.cost(c))
            })
            .collect();
        Self { costs }
    }

    /// (cost inside, cost outside) of color `c`.
    fn at(&self, c: [f32; 3]) -> (f32, f32) {
        let bin = |v: f32| ((v as usize) * BINS / 256).min(BINS - 1);
        self.costs[(bin(c[0]) * BINS + bin(c[1])) * BINS + bin(c[2])]
    }
}

/// The pixels within `radius` of the polyline `points` (pixel coordinates of a `width × height`
/// grid, pixel centers at half units): what a round brush along it covers.
pub fn brush(width: usize, height: usize, points: &[[f64; 2]], radius: f64) -> Vec<bool> {
    let mut covered = vec![false; width * height];
    let radius = radius.max(0.5);
    let segments: Vec<([f64; 2], [f64; 2])> = match points {
        [] => Vec::new(),
        [only] => vec![(*only, *only)],
        _ => points.windows(2).map(|s| (s[0], s[1])).collect(),
    };
    for (a, b) in segments {
        let left = (a[0].min(b[0]) - radius).floor().max(0.0) as usize;
        let top = (a[1].min(b[1]) - radius).floor().max(0.0) as usize;
        let right = ((a[0].max(b[0]) + radius).ceil().max(0.0) as usize).min(width);
        let bottom = ((a[1].max(b[1]) + radius).ceil().max(0.0) as usize).min(height);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let length2 = dx * dx + dy * dy;
        for y in top..bottom {
            for x in left..right {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let t = if length2 > 0.0 {
                    (((px - a[0]) * dx + (py - a[1]) * dy) / length2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (ex, ey) = (a[0] + t * dx - px, a[1] + t * dy - py);
                if ex * ex + ey * ey <= radius * radius {
                    covered[y * width + x] = true;
                }
            }
        }
    }
    covered
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 40 × 30 image: a red disk on a noisy blue background with a green square.
    fn scene() -> (Vec<u8>, usize, usize) {
        let (w, h) = (40, 30);
        let mut rgb = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let p = (y * w + x) * 3;
                let noise = ((x * 7 + y * 13) % 9) as u8;
                let (dx, dy) = (x as f64 - 12.0, y as f64 - 15.0);
                let color = if dx * dx + dy * dy < 64.0 {
                    [200 + noise, 30, 40]
                } else if (28..36).contains(&x) && (10..20).contains(&y) {
                    [30, 190 + noise, 50]
                } else {
                    [40, 60, 180 + noise]
                };
                rgb[p..p + 3].copy_from_slice(&color);
            }
        }
        (rgb, w, h)
    }

    fn is_disk(x: usize, y: usize) -> bool {
        let (dx, dy) = (x as f64 - 12.0, y as f64 - 15.0);
        dx * dx + dy * dy < 64.0
    }

    #[test]
    fn a_stroke_inside_a_region_selects_it_up_to_its_edges() {
        let (rgb, w, h) = scene();
        let image = QuickImage {
            width: w,
            height: h,
            rgb: &rgb,
        };
        let stroke = brush(w, h, &[[10.5, 14.5], [13.5, 16.5]], 1.5);
        let none = vec![false; w * h];
        let selected = quick_select(&image, &none, &stroke, QuickMode::New);
        for y in 0..h {
            for x in 0..w {
                assert_eq!(selected[y * w + x], is_disk(x, y), "at {x}, {y}");
            }
        }

        // Adding the square: the disk stays, only the square is new.
        let stroke = brush(w, h, &[[31.5, 14.5]], 1.5);
        let added = quick_select(&image, &selected, &stroke, QuickMode::Add);
        for y in 0..h {
            for x in 0..w {
                let square = (28..36).contains(&x) && (10..20).contains(&y);
                assert_eq!(added[y * w + x], square, "at {x}, {y}");
            }
        }

        // Subtracting the square back.
        let both: Vec<bool> = (0..w * h).map(|p| selected[p] || added[p]).collect();
        let removed = quick_select(&image, &both, &stroke, QuickMode::Subtract);
        for y in 0..h {
            for x in 0..w {
                assert_eq!(removed[y * w + x], added[y * w + x], "at {x}, {y}");
            }
        }
    }

    #[test]
    fn the_brush_covers_a_round_tipped_band() {
        let covered = brush(10, 10, &[[2.5, 2.5], [7.5, 2.5]], 1.0);
        assert!(covered[2 * 10 + 2] && covered[2 * 10 + 7] && covered[3 * 10 + 5]);
        assert!(!covered[4 * 10 + 5] && !covered[2 * 10 + 9]);
        assert_eq!(brush(4, 4, &[], 3.0), vec![false; 16]);
    }
}
