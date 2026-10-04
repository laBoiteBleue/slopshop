//! Filters (ADR 0034): operations that read neighbouring pixels, applied as entries of a raster
//! layer's stack. Their parameters, and their math on premultiplied values (applying them to a
//! layer's pixels is the stack's business, `stack::filtered`).

use crate::raster::parallel_for_each;

/// Gaussian Blur's radius range in pixels (Photoshop's): its standard deviation. Unsharp Mask
/// and High Pass blur by a radius in the same range.
pub const MIN_BLUR_RADIUS: f32 = 0.1;
pub const MAX_BLUR_RADIUS: f32 = 1000.0;

/// Unsharp Mask's amount range, in percent (Photoshop's).
pub const MIN_SHARPEN_AMOUNT: f32 = 1.0;
pub const MAX_SHARPEN_AMOUNT: f32 = 500.0;

/// Unsharp Mask's threshold range, in levels of 8 bits (Photoshop's).
pub const MAX_THRESHOLD: f32 = 255.0;

/// Up to this radius, the Gaussian is convolved exactly; beyond it, three box blurs approximate
/// it (as the selection's Feather does), whatever the radius costs the same.
const EXACT_UP_TO: f64 = 8.0;

/// Up to this radius, a layer is blurred tile by tile with a margin around each (a few
/// megabytes a thread, whatever the layer); beyond it, on the layer reduced (see
/// [`GaussianPlan`]).
const TILED_UP_TO: f64 = 64.0;

/// The radius a reduced layer is blurred with at most (the reduction a power of two).
const REDUCED_UP_TO: f64 = 32.0;

/// A filter and its parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Filter {
    /// Photoshop's Gaussian Blur: `radius` is the standard deviation, in the layer's pixels.
    GaussianBlur { radius: f32 },
    /// Photoshop's Unsharp Mask: each color pushed away from its Gaussian blur of `radius` by
    /// `amount` percent of their difference, where that difference reaches `threshold` levels
    /// (of 8 bits) on some channel.
    UnsharpMask {
        amount: f32,
        radius: f32,
        threshold: f32,
    },
    /// Photoshop's High Pass: each color's difference with its Gaussian blur of `radius`, around
    /// middle gray.
    HighPass { radius: f32 },
}

impl Filter {
    /// Every filter's identifier, in menu order.
    pub const IDS: [&'static str; 3] = ["gaussianBlur", "unsharpMask", "highPass"];

    /// The identifier the UI and files know it by.
    pub fn id(&self) -> &'static str {
        match self {
            Self::GaussianBlur { .. } => "gaussianBlur",
            Self::UnsharpMask { .. } => "unsharpMask",
            Self::HighPass { .. } => "highPass",
        }
    }

    /// Its parameters, in a fixed order per filter.
    pub fn params(&self) -> Vec<f32> {
        match *self {
            Self::GaussianBlur { radius } | Self::HighPass { radius } => vec![radius],
            Self::UnsharpMask {
                amount,
                radius,
                threshold,
            } => vec![amount, radius, threshold],
        }
    }

    /// The filter `id` with `values` (in [`Self::params`] order); `None` for an unknown
    /// identifier or a wrong number of values. Not validated: see [`Self::is_valid`].
    pub fn from_params(id: &str, values: &[f32]) -> Option<Self> {
        match (id, values) {
            ("gaussianBlur", &[radius]) => Some(Self::GaussianBlur { radius }),
            ("unsharpMask", &[amount, radius, threshold]) => Some(Self::UnsharpMask {
                amount,
                radius,
                threshold,
            }),
            ("highPass", &[radius]) => Some(Self::HighPass { radius }),
            _ => None,
        }
    }

    /// The filter `id` at the settings its dialog opens with the first time (Photoshop's).
    pub fn defaults(id: &str) -> Option<Self> {
        match id {
            "gaussianBlur" => Some(Self::GaussianBlur { radius: 1.0 }),
            "unsharpMask" => Some(Self::UnsharpMask {
                amount: 100.0,
                radius: 1.0,
                threshold: 0.0,
            }),
            "highPass" => Some(Self::HighPass { radius: 10.0 }),
            _ => None,
        }
    }

    pub fn is_valid(&self) -> bool {
        let radius = |r: f32| r.is_finite() && (MIN_BLUR_RADIUS..=MAX_BLUR_RADIUS).contains(&r);
        match *self {
            Self::GaussianBlur { radius: r } | Self::HighPass { radius: r } => radius(r),
            Self::UnsharpMask {
                amount,
                radius: r,
                threshold,
            } => {
                radius(r)
                    && amount.is_finite()
                    && (MIN_SHARPEN_AMOUNT..=MAX_SHARPEN_AMOUNT).contains(&amount)
                    && threshold.is_finite()
                    && (0.0..=MAX_THRESHOLD).contains(&threshold)
            }
        }
    }

    /// The radius of the Gaussian blur the filter is made from.
    pub fn blur_radius(&self) -> f32 {
        match *self {
            Self::GaussianBlur { radius }
            | Self::UnsharpMask { radius, .. }
            | Self::HighPass { radius } => radius,
        }
    }

    /// The filter on the layer reduced `factor` times (a pyramid level, a look): its distances
    /// divided, no less than the smallest it takes.
    pub fn scaled(&self, factor: f32) -> Self {
        let radius = (self.blur_radius() / factor).max(MIN_BLUR_RADIUS);
        match *self {
            Self::GaussianBlur { .. } => Self::GaussianBlur { radius },
            Self::UnsharpMask {
                amount, threshold, ..
            } => Self::UnsharpMask {
                amount,
                radius,
                threshold,
            },
            Self::HighPass { .. } => Self::HighPass { radius },
        }
    }

    /// How far a pixel's result reads, in pixels on each side, with some room (a look's
    /// margin): three and a half sigmas of its blur.
    pub fn reach(&self) -> f64 {
        3.5 * f64::from(self.blur_radius()) + 2.0
    }

    /// Whether a pixel's result depends on its own value besides its blur's.
    pub(crate) fn reads_original(&self) -> bool {
        !matches!(self, Self::GaussianBlur { .. })
    }

    /// A pixel's result from its premultiplied value `original` and its blur's `blurred`, in
    /// the blend space (values of 1 are white). Unsharp Mask and High Pass work on the colors
    /// (straight, the blur's by its own coverage), and keep the pixel's alpha.
    pub(crate) fn finish(&self, original: [f64; 4], blurred: [f64; 4]) -> [f64; 4] {
        let straight = |p: [f64; 4]| {
            if p[3] > 0.0 {
                [p[0] / p[3], p[1] / p[3], p[2] / p[3]]
            } else {
                [0.0; 3]
            }
        };
        let (o, b) = (straight(original), straight(blurred));
        let alpha = original[3];
        let color = |f: &dyn Fn(f64, f64) -> f64| {
            let c: [f64; 3] = std::array::from_fn(|i| f(o[i], b[i]));
            [c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha]
        };
        match *self {
            Self::GaussianBlur { .. } => blurred,
            Self::UnsharpMask {
                amount, threshold, ..
            } => {
                // Below the threshold on every channel, the pixel is left as it is.
                let level = f64::from(threshold) / 255.0;
                if alpha <= 0.0 || (0..3).all(|i| (o[i] - b[i]).abs() < level) {
                    return original;
                }
                let k = f64::from(amount) / 100.0;
                color(&|o, b| o + k * (o - b))
            }
            Self::HighPass { .. } => color(&|o, b| o - b + 0.5),
        }
    }
}

/// How a line of pixels is blurred: an exact kernel, or box blurs of these radii.
#[derive(Debug, Clone)]
pub(crate) enum Blur {
    Kernel(Vec<f64>),
    Boxes([usize; 3]),
}

impl Blur {
    /// The Gaussian of standard deviation `sigma` along one axis.
    pub(crate) fn gaussian(sigma: f64) -> Self {
        if sigma > EXACT_UP_TO {
            return Self::Boxes(crate::selection::box_radii(sigma));
        }
        let reach = (3.0 * sigma).ceil().max(1.0) as i64;
        let weights: Vec<f64> = (-reach..=reach)
            .map(|i| (-((i * i) as f64) / (2.0 * sigma * sigma)).exp())
            .collect();
        let total: f64 = weights.iter().sum();
        Self::Kernel(weights.into_iter().map(|w| w / total).collect())
    }

    /// How far a pixel's result reads, in pixels on each side.
    pub(crate) fn reach(&self) -> usize {
        match self {
            Self::Kernel(weights) => weights.len() / 2,
            Self::Boxes(radii) => radii.iter().sum(),
        }
    }

    /// `line` blurred in place, its ends repeating outward; `scratch` is reused between lines.
    pub(crate) fn line(&self, line: &mut [[f32; 4]], scratch: &mut Vec<[f32; 4]>) {
        let n = line.len();
        if n == 0 {
            return;
        }
        scratch.clear();
        scratch.extend_from_slice(line);
        let at = |src: &[[f32; 4]], i: i64| src[i.clamp(0, n as i64 - 1) as usize];
        match self {
            Self::Kernel(weights) => {
                let reach = (weights.len() / 2) as i64;
                for (i, out) in line.iter_mut().enumerate() {
                    let mut sum = [0.0f64; 4];
                    for (k, w) in weights.iter().enumerate() {
                        let px = at(scratch, i as i64 + k as i64 - reach);
                        for c in 0..4 {
                            sum[c] += w * f64::from(px[c]);
                        }
                    }
                    *out = sum.map(|v| v as f32);
                }
            }
            Self::Boxes(radii) => {
                for &r in radii {
                    if r == 0 {
                        continue;
                    }
                    scratch.clear();
                    scratch.extend_from_slice(line);
                    let r = r as i64;
                    let width = (2 * r + 1) as f64;
                    let mut sum = [0.0f64; 4];
                    for i in -r..=r {
                        let px = at(scratch, i);
                        for c in 0..4 {
                            sum[c] += f64::from(px[c]);
                        }
                    }
                    for (i, out) in line.iter_mut().enumerate() {
                        *out = sum.map(|v| (v / width) as f32);
                        let (add, sub) = (at(scratch, i as i64 + r + 1), at(scratch, i as i64 - r));
                        for c in 0..4 {
                            sum[c] += f64::from(add[c]) - f64::from(sub[c]);
                        }
                    }
                }
            }
        }
    }
}

/// How a Gaussian Blur is computed over a layer, whatever its size (ADR 0034): up to
/// [`TILED_UP_TO`], directly, tile by tile with a margin of [`Blur::reach`]; beyond, the layer
/// reduced by `factor` (a power of two, at least 4, so that the reduced layer is at most a
/// sixteenth of it) is blurred by what is left of the radius and read back interpolated.
#[derive(Debug, Clone)]
pub(crate) struct GaussianPlan {
    pub(crate) factor: usize,
    pub(crate) blur: Blur,
}

impl GaussianPlan {
    pub(crate) fn new(sigma: f64) -> Self {
        if sigma <= TILED_UP_TO {
            return Self {
                factor: 1,
                blur: Blur::gaussian(sigma),
            };
        }
        let factor = 1usize << (sigma / REDUCED_UP_TO).log2().ceil().max(2.0) as u32;
        // Averaging blocks of `factor` pixels then interpolating between them spreads by about
        // a quarter of a reduced pixel (variances of f²/12 and f²/6): taken off the radius.
        let f = factor as f64;
        let reduced = (sigma * sigma / (f * f) - 0.25).max(1.0).sqrt();
        Self {
            factor,
            blur: Blur::gaussian(reduced),
        }
    }
}

/// Rows each thread blurs at a time along the columns.
const BAND_ROWS: usize = 32;

impl Blur {
    /// `region` (`width` × `height` pixels, rows top to bottom) blurred in place along both
    /// axes on the calling thread, its edges repeating outward: one tile and its margin.
    pub(crate) fn region(&self, region: &mut [[f32; 4]], width: usize, height: usize) {
        if width == 0 || height == 0 {
            return;
        }
        let mut scratch = Vec::with_capacity(width.max(height));
        for row in region.chunks_mut(width) {
            self.line(row, &mut scratch);
        }
        let mut column = vec![[0.0f32; 4]; height];
        for x in 0..width {
            for (y, px) in column.iter_mut().enumerate() {
                *px = region[y * width + x];
            }
            self.line(&mut column, &mut scratch);
            for (y, px) in column.iter().enumerate() {
                region[y * width + x] = *px;
            }
        }
    }

    /// `image` (`width` × `height` pixels of premultiplied values, rows top to bottom) blurred
    /// along both axes, its edges repeating outward, on every core.
    pub(crate) fn image(
        &self,
        mut image: Vec<[f32; 4]>,
        width: usize,
        height: usize,
    ) -> Vec<[f32; 4]> {
        if width == 0 || height == 0 {
            return image;
        }
        let mut bands: Vec<&mut [[f32; 4]]> = image.chunks_mut(width * BAND_ROWS).collect();
        parallel_for_each(&mut bands, |band| {
            let mut scratch = Vec::with_capacity(width);
            for row in band.chunks_mut(width) {
                self.line(row, &mut scratch);
            }
        });
        match self {
            Self::Kernel(weights) => columns_kernel(&image, weights, width, height),
            Self::Boxes(radii) => {
                for &r in radii {
                    if r > 0 {
                        image = columns_box(&image, r, width, height);
                    }
                }
                image
            }
        }
    }
}

/// `src` convolved along its columns with `weights` (centered), into a new image.
fn columns_kernel(src: &[[f32; 4]], weights: &[f64], width: usize, height: usize) -> Vec<[f32; 4]> {
    let reach = (weights.len() / 2) as i64;
    let mut dst = vec![[0.0f32; 4]; width * height];
    let mut bands: Vec<(usize, &mut [[f32; 4]])> =
        dst.chunks_mut(width * BAND_ROWS).enumerate().collect();
    parallel_for_each(&mut bands, |(index, band)| {
        let mut sums = vec![[0.0f64; 4]; width];
        for (n, row) in band.chunks_mut(width).enumerate() {
            let y = (*index * BAND_ROWS + n) as i64;
            sums.fill([0.0; 4]);
            for (k, w) in weights.iter().enumerate() {
                let sy = (y + k as i64 - reach).clamp(0, height as i64 - 1) as usize;
                for (sum, px) in sums.iter_mut().zip(&src[sy * width..(sy + 1) * width]) {
                    for c in 0..4 {
                        sum[c] += w * f64::from(px[c]);
                    }
                }
            }
            for (out, sum) in row.iter_mut().zip(&sums) {
                *out = sum.map(|v| v as f32);
            }
        }
    });
    dst
}

/// One box blur of radius `r` along the columns of `src`, with running sums, into a new image.
fn columns_box(src: &[[f32; 4]], r: usize, width: usize, height: usize) -> Vec<[f32; 4]> {
    let r = r as i64;
    let size = (2 * r + 1) as f64;
    let row = |y: i64| {
        let y = y.clamp(0, height as i64 - 1) as usize;
        &src[y * width..(y + 1) * width]
    };
    let mut dst = vec![[0.0f32; 4]; width * height];
    let mut bands: Vec<(usize, &mut [[f32; 4]])> =
        dst.chunks_mut(width * BAND_ROWS).enumerate().collect();
    parallel_for_each(&mut bands, |(index, band)| {
        let first = (*index * BAND_ROWS) as i64;
        let mut sums = vec![[0.0f64; 4]; width];
        for y in first - r..=first + r {
            for (sum, px) in sums.iter_mut().zip(row(y)) {
                for c in 0..4 {
                    sum[c] += f64::from(px[c]);
                }
            }
        }
        for (n, out) in band.chunks_mut(width).enumerate() {
            let y = first + n as i64;
            for (px, sum) in out.iter_mut().zip(&sums) {
                *px = sum.map(|v| (v / size) as f32);
            }
            for ((sum, add), sub) in sums.iter_mut().zip(row(y + r + 1)).zip(row(y - r)) {
                for c in 0..4 {
                    sum[c] += f64::from(add[c]) - f64::from(sub[c]);
                }
            }
        }
    });
    dst
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parameters_round_trip_and_are_checked() {
        let blur = Filter::GaussianBlur { radius: 12.5 };
        assert_eq!(Filter::from_params(blur.id(), &blur.params()), Some(blur));
        assert_eq!(Filter::from_params("gaussianBlur", &[1.0, 2.0]), None);
        assert_eq!(Filter::from_params("twirl", &[1.0]), None);
        assert!(blur.is_valid());
        for radius in [0.0, 1001.0, f32::NAN] {
            assert!(!Filter::GaussianBlur { radius }.is_valid(), "{radius}");
            assert!(!Filter::HighPass { radius }.is_valid(), "{radius}");
        }
        let sharpen = |amount, radius, threshold| Filter::UnsharpMask {
            amount,
            radius,
            threshold,
        };
        assert!(sharpen(500.0, 1000.0, 255.0).is_valid());
        for wrong in [
            sharpen(0.5, 1.0, 0.0),
            sharpen(501.0, 1.0, 0.0),
            sharpen(100.0, 0.0, 0.0),
            sharpen(100.0, 1.0, -1.0),
            sharpen(100.0, 1.0, 256.0),
            sharpen(f32::NAN, 1.0, 0.0),
        ] {
            assert!(!wrong.is_valid(), "{wrong:?}");
        }
        for id in Filter::IDS {
            let filter = Filter::defaults(id).expect("every filter has defaults");
            assert!(filter.is_valid(), "{id}");
            assert_eq!(filter.id(), id);
            assert_eq!(Filter::from_params(id, &filter.params()), Some(filter));
        }
    }

    #[test]
    fn scaled_filters_divide_their_distances_only() {
        let sharpen = Filter::UnsharpMask {
            amount: 150.0,
            radius: 8.0,
            threshold: 4.0,
        };
        assert_eq!(
            sharpen.scaled(4.0),
            Filter::UnsharpMask {
                amount: 150.0,
                radius: 2.0,
                threshold: 4.0
            }
        );
        assert_eq!(
            Filter::HighPass { radius: 0.2 }.scaled(8.0),
            Filter::HighPass {
                radius: MIN_BLUR_RADIUS
            }
        );
        assert!(sharpen.scaled(4.0).reach() < sharpen.reach());
    }

    #[test]
    fn unsharp_mask_pushes_colors_from_their_blur_above_its_threshold() {
        let sharpen = |threshold| Filter::UnsharpMask {
            amount: 50.0,
            radius: 1.0,
            threshold,
        };
        // Half opaque: colors are straight, alpha kept.
        let original = [0.3, 0.2, 0.1, 0.5];
        let blurred = [0.2, 0.2, 0.2, 0.5];
        let out = sharpen(0.0).finish(original, blurred);
        let expected = [0.7, 0.4, 0.1].map(|c| c * 0.5);
        for c in 0..3 {
            assert!((out[c] - expected[c]).abs() < 1e-12, "{out:?}");
        }
        assert_eq!(out[3], 0.5);
        // Within the threshold on every channel (0.2 is 51 levels, 0 is 0): left as it is.
        assert_eq!(sharpen(52.0).finish(original, blurred), original);
        assert_ne!(sharpen(50.0).finish(original, blurred), original);
        // Transparent stays transparent.
        assert_eq!(sharpen(0.0).finish([0.0; 4], blurred), [0.0; 4]);
    }

    #[test]
    fn high_pass_is_the_difference_with_the_blur_around_middle_gray() {
        let high = Filter::HighPass { radius: 3.0 };
        let same = high.finish([0.4, 0.4, 0.4, 1.0], [0.4, 0.4, 0.4, 1.0]);
        for v in &same[..3] {
            assert!((v - 0.5).abs() < 1e-12, "{same:?}");
        }
        let out = high.finish([0.9, 0.1, 0.5, 1.0], [0.5, 0.5, 0.5, 1.0]);
        for (c, v) in [0.9, 0.1, 0.5].iter().enumerate() {
            assert!((out[c] - v).abs() < 1e-12, "{out:?}");
        }
        // Half opaque over a transparent blur (read as black): a straight 0.5 becomes 1.
        let out = high.finish([0.25, 0.25, 0.25, 0.5], [0.0; 4]);
        assert_eq!(out, [0.5, 0.5, 0.5, 0.5]);
    }

    /// A line of `n` pixels, all transparent but one opaque white in the middle.
    fn impulse(n: usize) -> Vec<[f32; 4]> {
        let mut line = vec![[0.0; 4]; n];
        line[n / 2] = [1.0; 4];
        line
    }

    /// The variance of a blurred impulse (its mass at 1).
    fn variance(line: &[[f32; 4]]) -> f64 {
        let center = (line.len() / 2) as f64;
        line.iter()
            .enumerate()
            .map(|(i, px)| f64::from(px[3]) * (i as f64 - center).powi(2))
            .sum()
    }

    #[test]
    fn a_blurred_impulse_keeps_its_mass_and_spreads_by_the_radius() {
        let mut scratch = Vec::new();
        // (Below a pixel, a sampled Gaussian is narrower than its sigma: tested from 1.)
        for sigma in [1.0, 2.0, 8.0, 9.0, 40.0] {
            let mut line = impulse(1001);
            Blur::gaussian(sigma).line(&mut line, &mut scratch);
            let mass: f64 = line.iter().map(|px| f64::from(px[3])).sum();
            assert!((mass - 1.0).abs() < 1e-4, "sigma {sigma}: mass {mass}");
            let spread = variance(&line).sqrt();
            // Exact up to 8; three boxes of whole widths beyond, a little off.
            let tolerance = if sigma > EXACT_UP_TO { 0.06 } else { 0.02 };
            assert!(
                (spread - sigma).abs() / sigma < tolerance,
                "sigma {sigma}: spread {spread}"
            );
            // Symmetric, highest at the center.
            let mid = line.len() / 2;
            assert!((line[mid - 3][3] - line[mid + 3][3]).abs() < 1e-6);
            assert!(line[mid][3] >= line[mid + 1][3]);
        }
    }

    #[test]
    fn an_image_is_blurred_along_both_axes_alike() {
        // A point in a 61 × 45 image: its blur is the product of the blurs of its row and
        // column, whatever the axis, both for the exact kernel and the boxes.
        for sigma in [3.0, 12.0] {
            let blur = Blur::gaussian(sigma);
            let (w, h) = (61, 45);
            let mut image = vec![[0.0f32; 4]; w * h];
            image[22 * w + 30] = [1.0; 4];
            let out = blur.image(image, w, h);
            let mut scratch = Vec::new();
            let mut row = vec![[0.0f32; 4]; w];
            row[30] = [1.0; 4];
            blur.line(&mut row, &mut scratch);
            let mut column = vec![[0.0f32; 4]; h];
            column[22] = [1.0; 4];
            blur.line(&mut column, &mut scratch);
            for y in 0..h {
                for x in 0..w {
                    let expected = row[x][3] * column[y][3];
                    assert!(
                        (out[y * w + x][3] - expected).abs() < 1e-6,
                        "sigma {sigma} at ({x}, {y})"
                    );
                }
            }
        }
    }

    #[test]
    fn a_region_is_blurred_as_an_image_is() {
        let (w, h) = (40, 30);
        let image: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let v = ((i * 37) % 101) as f32 / 100.0;
                [v, 1.0 - v, v * v, 1.0]
            })
            .collect();
        for sigma in [2.0, 20.0] {
            let blur = Blur::gaussian(sigma);
            let whole = blur.image(image.clone(), w, h);
            let mut region = image.clone();
            blur.region(&mut region, w, h);
            for (a, b) in whole.iter().zip(&region) {
                for c in 0..4 {
                    assert!((a[c] - b[c]).abs() < 1e-5, "sigma {sigma}");
                }
            }
        }
    }

    #[test]
    fn large_radii_are_blurred_reduced_at_least_four_times() {
        assert_eq!(GaussianPlan::new(64.0).factor, 1);
        for (sigma, factor) in [(65.0, 4), (128.0, 4), (129.0, 8), (1000.0, 32)] {
            let plan = GaussianPlan::new(sigma);
            assert_eq!(plan.factor, factor, "sigma {sigma}");
            assert!(
                plan.blur.reach() <= 200,
                "sigma {sigma}: {}",
                plan.blur.reach()
            );
        }
    }

    #[test]
    fn a_uniform_line_stays_uniform_up_to_its_repeated_ends() {
        let mut scratch = Vec::new();
        for sigma in [1.0, 30.0] {
            let mut line = vec![[0.25, 0.5, 0.75, 1.0]; 300];
            Blur::gaussian(sigma).line(&mut line, &mut scratch);
            for px in &line {
                for (c, v) in px.iter().enumerate() {
                    assert!((v - [0.25, 0.5, 0.75, 1.0][c]).abs() < 1e-5);
                }
            }
        }
    }
}
