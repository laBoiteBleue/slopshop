//! Filters (ADR 0034): operations that read neighbouring pixels, applied as entries of a raster
//! layer's stack. Their parameters, and their math on premultiplied values (applying them to a
//! layer's pixels is the stack's business, `stack::filtered`).

use crate::raster::parallel_for_each;

/// Gaussian Blur's radius range in pixels (Photoshop's): its standard deviation. Unsharp Mask
/// and High Pass blur by a radius in the same range.
pub const MIN_BLUR_RADIUS: f32 = 0.1;
pub const MAX_BLUR_RADIUS: f32 = 1000.0;

/// Motion Blur's angle range in degrees, and distance range in pixels (Photoshop's).
pub const MAX_MOTION_ANGLE: f32 = 90.0;
pub const MIN_MOTION_DISTANCE: f32 = 1.0;
pub const MAX_MOTION_DISTANCE: f32 = 2000.0;

/// Up to this length in pixels, a Motion Blur samples its line on the layer itself; beyond, on
/// the layer reduced (see [`Plan::line`]), whatever the distance costs about the same.
pub const LINE_UP_TO: f64 = 256.0;

/// Add Noise's amount range, in percent (Photoshop's): at 100 %, uniform noise reaches half the
/// range of a channel either way.
pub const MIN_NOISE_AMOUNT: f32 = 0.1;
pub const MAX_NOISE_AMOUNT: f32 = 400.0;

/// The seeds Add Noise takes: whole numbers below 2^24, exact in an `f32` parameter.
pub const NOISE_SEEDS: u32 = 1 << 24;

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
    /// Photoshop's Motion Blur: each pixel the average of the line of `distance` pixels through
    /// it at `angle` degrees (counterclockwise from the horizontal).
    MotionBlur { angle: f32, distance: f32 },
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
    /// Photoshop's Add Noise: `amount` percent of noise added to each color, `gaussian` or
    /// uniform, the same on the three channels when `monochromatic`. The noise is a function of
    /// the document pixel and `seed` (a whole number below [`NOISE_SEEDS`]): computing it again
    /// gives the same grain, another seed another grain.
    AddNoise {
        amount: f32,
        gaussian: bool,
        monochromatic: bool,
        seed: u32,
    },
}

impl Filter {
    /// Every filter's identifier, in menu order.
    pub const IDS: [&'static str; 5] = [
        "gaussianBlur",
        "motionBlur",
        "unsharpMask",
        "addNoise",
        "highPass",
    ];

    /// The identifier the UI and files know it by.
    pub fn id(&self) -> &'static str {
        match self {
            Self::GaussianBlur { .. } => "gaussianBlur",
            Self::MotionBlur { .. } => "motionBlur",
            Self::UnsharpMask { .. } => "unsharpMask",
            Self::HighPass { .. } => "highPass",
            Self::AddNoise { .. } => "addNoise",
        }
    }

    /// Its parameters, in a fixed order per filter.
    pub fn params(&self) -> Vec<f32> {
        match *self {
            Self::GaussianBlur { radius } | Self::HighPass { radius } => vec![radius],
            Self::MotionBlur { angle, distance } => vec![angle, distance],
            Self::UnsharpMask {
                amount,
                radius,
                threshold,
            } => vec![amount, radius, threshold],
            Self::AddNoise {
                amount,
                gaussian,
                monochromatic,
                seed,
            } => vec![
                amount,
                f32::from(u8::from(gaussian)),
                f32::from(u8::from(monochromatic)),
                seed as f32,
            ],
        }
    }

    /// The filter `id` with `values` (in [`Self::params`] order); `None` for an unknown
    /// identifier or a wrong number of values. Not validated: see [`Self::is_valid`].
    pub fn from_params(id: &str, values: &[f32]) -> Option<Self> {
        match (id, values) {
            ("gaussianBlur", &[radius]) => Some(Self::GaussianBlur { radius }),
            ("motionBlur", &[angle, distance]) => Some(Self::MotionBlur { angle, distance }),
            ("unsharpMask", &[amount, radius, threshold]) => Some(Self::UnsharpMask {
                amount,
                radius,
                threshold,
            }),
            ("highPass", &[radius]) => Some(Self::HighPass { radius }),
            ("addNoise", &[amount, gaussian, monochromatic, seed]) => {
                // Flags are 0 or 1, the seed a whole number in range: anything else is not
                // this filter's.
                let flag = |v: f32| (v == 0.0 || v == 1.0).then_some(v == 1.0);
                let whole = seed.fract() == 0.0 && (0.0..NOISE_SEEDS as f32).contains(&seed);
                Some(Self::AddNoise {
                    amount,
                    gaussian: flag(gaussian)?,
                    monochromatic: flag(monochromatic)?,
                    seed: whole.then_some(seed as u32)?,
                })
            }
            _ => None,
        }
    }

    /// The filter `id` at the settings its dialog opens with the first time (Photoshop's).
    pub fn defaults(id: &str) -> Option<Self> {
        match id {
            "gaussianBlur" => Some(Self::GaussianBlur { radius: 1.0 }),
            "motionBlur" => Some(Self::MotionBlur {
                angle: 0.0,
                distance: 10.0,
            }),
            "unsharpMask" => Some(Self::UnsharpMask {
                amount: 100.0,
                radius: 1.0,
                threshold: 0.0,
            }),
            "highPass" => Some(Self::HighPass { radius: 10.0 }),
            "addNoise" => Some(Self::AddNoise {
                amount: 12.5,
                gaussian: false,
                monochromatic: false,
                seed: 0,
            }),
            _ => None,
        }
    }

    pub fn is_valid(&self) -> bool {
        let radius = |r: f32| r.is_finite() && (MIN_BLUR_RADIUS..=MAX_BLUR_RADIUS).contains(&r);
        match *self {
            Self::GaussianBlur { radius: r } | Self::HighPass { radius: r } => radius(r),
            Self::MotionBlur { angle, distance } => {
                angle.is_finite()
                    && (-MAX_MOTION_ANGLE..=MAX_MOTION_ANGLE).contains(&angle)
                    && distance.is_finite()
                    && (MIN_MOTION_DISTANCE..=MAX_MOTION_DISTANCE).contains(&distance)
            }
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
            Self::AddNoise { amount, seed, .. } => {
                amount.is_finite()
                    && (MIN_NOISE_AMOUNT..=MAX_NOISE_AMOUNT).contains(&amount)
                    && seed < NOISE_SEEDS
            }
        }
    }

    /// The radius of the Gaussian blur the filter is made from, if it is.
    pub fn blur_radius(&self) -> Option<f32> {
        match *self {
            Self::GaussianBlur { radius }
            | Self::UnsharpMask { radius, .. }
            | Self::HighPass { radius } => Some(radius),
            Self::MotionBlur { .. } | Self::AddNoise { .. } => None,
        }
    }

    /// The filter on the layer reduced `factor` times (a pyramid level, a look): its distances
    /// divided, no less than the smallest it takes.
    pub fn scaled(&self, factor: f32) -> Self {
        let radius = (self.blur_radius().unwrap_or(MIN_BLUR_RADIUS) / factor).max(MIN_BLUR_RADIUS);
        match *self {
            Self::GaussianBlur { .. } => Self::GaussianBlur { radius },
            Self::MotionBlur { angle, distance } => Self::MotionBlur {
                angle,
                distance: (distance / factor).max(MIN_MOTION_DISTANCE),
            },
            Self::UnsharpMask {
                amount, threshold, ..
            } => Self::UnsharpMask {
                amount,
                radius,
                threshold,
            },
            Self::HighPass { .. } => Self::HighPass { radius },
            // A reduced pixel averages `factor`² noisy ones: its noise is `factor` times weaker.
            Self::AddNoise {
                amount,
                gaussian,
                monochromatic,
                seed,
            } => Self::AddNoise {
                amount: (amount / factor).max(MIN_NOISE_AMOUNT),
                gaussian,
                monochromatic,
                seed,
            },
        }
    }

    /// How far a pixel's result reads, in pixels on each side, with some room (a look's
    /// margin): three and a half sigmas of a blur, half a line.
    pub fn reach(&self) -> f64 {
        match *self {
            Self::MotionBlur { distance, .. } => f64::from(distance) / 2.0 + 2.0,
            Self::AddNoise { .. } => 0.0,
            _ => 3.5 * f64::from(self.blur_radius().unwrap_or(0.0)) + 2.0,
        }
    }

    /// How the filter is computed over a layer: what it blurs by, at what reduction.
    pub(crate) fn plan(&self) -> Plan {
        match *self {
            Self::MotionBlur { angle, distance } => {
                Plan::line(f64::from(angle), f64::from(distance))
            }
            Self::AddNoise { .. } => Plan {
                factor: 1,
                kernel: Kernel::Identity,
            },
            _ => Plan::gaussian(f64::from(self.blur_radius().unwrap_or(MIN_BLUR_RADIUS))),
        }
    }

    /// Whether a pixel's result depends on its own value besides its blur's.
    pub(crate) fn reads_original(&self) -> bool {
        !matches!(self, Self::GaussianBlur { .. } | Self::MotionBlur { .. })
    }

    /// A pixel's result from its premultiplied value `original` and its blur's `blurred`, in
    /// the blend space (values of 1 are white); `at` is the document pixel it shows. Unsharp
    /// Mask, High Pass and Add Noise work on the colors (straight, the blur's by its own
    /// coverage), and keep the pixel's alpha.
    pub(crate) fn finish(&self, original: [f64; 4], blurred: [f64; 4], at: [i64; 2]) -> [f64; 4] {
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
            Self::GaussianBlur { .. } | Self::MotionBlur { .. } => blurred,
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
            Self::AddNoise {
                amount,
                gaussian,
                monochromatic,
                seed,
            } => {
                if alpha <= 0.0 {
                    return original;
                }
                let n = noise(at, seed, gaussian, monochromatic);
                let k = f64::from(amount) / 100.0;
                let c: [f64; 3] = std::array::from_fn(|i| o[i] + k * n[i]);
                [c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha]
            }
        }
    }
}

/// Add Noise's noise at document pixel `at` for `seed`, per channel: uniform within ±0.5, or
/// Gaussian of the same spread (a standard deviation of 0.5 / √3); the red channel's on all
/// three when `monochromatic`. From a hash of the pixel, the seed and the channel, so that the
/// GPU computes the same (ADR 0035): whole 32-bit operations, then 24 bits of it as a float.
pub fn noise(at: [i64; 2], seed: u32, gaussian: bool, monochromatic: bool) -> [f64; 3] {
    let uniform = |channel: u32, draw: u32| {
        let h = hash(
            at[0] as i32 as u32 ^ hash(at[1] as i32 as u32 ^ hash(seed ^ hash(channel * 2 + draw))),
        );
        // The middle of 2^24 cells of [0, 1): never 0, so that a logarithm of it is finite.
        (f64::from(h >> 8) + 0.5) / f64::from(1u32 << 24)
    };
    let one = |channel: u32| {
        if gaussian {
            // Box-Muller, scaled to the uniform noise's variance (1/12).
            let (u, v) = (uniform(channel, 0), uniform(channel, 1));
            (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos() / 12f64.sqrt()
        } else {
            uniform(channel, 0) - 0.5
        }
    };
    if monochromatic {
        [one(0); 3]
    } else {
        [one(0), one(1), one(2)]
    }
}

/// A 32-bit integer hash (PCG's output permutation of an LCG step): well mixed, and the same in
/// the shader.
fn hash(x: u32) -> u32 {
    let h = x.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let h = ((h >> ((h >> 28) + 4)) ^ h).wrapping_mul(277_803_737);
    (h >> 22) ^ h
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

/// How a filter's blur is computed over a layer, whatever its size (ADR 0034): directly, tile by
/// tile with a margin of [`Kernel::reach`]; or, when that would reach too far, on the layer
/// reduced by `factor` (a power of two, averages of blocks), read back interpolated.
#[derive(Debug, Clone)]
pub(crate) struct Plan {
    pub(crate) factor: usize,
    pub(crate) kernel: Kernel,
}

impl Plan {
    /// A Gaussian of standard deviation `sigma`: up to [`TILED_UP_TO`] directly; beyond, the
    /// layer reduced at least 4 times (so that it is at most a sixteenth of it), blurred by what
    /// is left of the radius.
    pub(crate) fn gaussian(sigma: f64) -> Self {
        if sigma <= TILED_UP_TO {
            return Self {
                factor: 1,
                kernel: Kernel::Gaussian(Blur::gaussian(sigma)),
            };
        }
        let factor = 1usize << (sigma / REDUCED_UP_TO).log2().ceil().max(2.0) as u32;
        // Averaging blocks of `factor` pixels then interpolating between them spreads by about
        // a quarter of a reduced pixel (variances of f²/12 and f²/6): taken off the radius.
        let f = factor as f64;
        let reduced = (sigma * sigma / (f * f) - 0.25).max(1.0).sqrt();
        Self {
            factor,
            kernel: Kernel::Gaussian(Blur::gaussian(reduced)),
        }
    }

    /// A line of `distance` pixels at `angle` degrees: up to [`LINE_UP_TO`] directly; beyond,
    /// the layer reduced so that the line is at most that long (a few reduced pixels of
    /// softness across it, hidden by the length along it).
    pub(crate) fn line(angle: f64, distance: f64) -> Self {
        let factor = if distance <= LINE_UP_TO {
            1
        } else {
            1usize << (distance / LINE_UP_TO).log2().ceil() as u32
        };
        Self {
            factor,
            kernel: Kernel::Line(Line::new(angle, distance / factor as f64)),
        }
    }
}

/// What a filter blurs a region or an image by; nothing for a filter of each pixel alone.
#[derive(Debug, Clone)]
pub(crate) enum Kernel {
    Gaussian(Blur),
    Line(Line),
    Identity,
}

impl Kernel {
    /// How far a pixel's result reads, in pixels on each side.
    pub(crate) fn reach(&self) -> usize {
        match self {
            Self::Gaussian(blur) => blur.reach(),
            Self::Line(line) => line.reach(),
            Self::Identity => 0,
        }
    }

    /// `region` (`width` × `height` pixels) blurred in place on the calling thread, its edges
    /// repeating outward.
    pub(crate) fn region(&self, region: &mut [[f32; 4]], width: usize, height: usize) {
        match self {
            Self::Gaussian(blur) => blur.region(region, width, height),
            Self::Line(line) => {
                let source = region.to_vec();
                line.rows(&source, width, height, 0, region);
            }
            Self::Identity => {}
        }
    }

    /// `image` (`width` × `height` pixels) blurred, its edges repeating outward, on every core.
    pub(crate) fn image(&self, image: Vec<[f32; 4]>, width: usize, height: usize) -> Vec<[f32; 4]> {
        match self {
            Self::Gaussian(blur) => blur.image(image, width, height),
            Self::Identity => image,
            Self::Line(line) => {
                let mut out = vec![[0.0f32; 4]; image.len()];
                if width == 0 {
                    return out;
                }
                let mut bands: Vec<(usize, &mut [[f32; 4]])> =
                    out.chunks_mut(width * BAND_ROWS).enumerate().collect();
                parallel_for_each(&mut bands, |(index, band)| {
                    line.rows(&image, width, height, *index * BAND_ROWS, band);
                });
                out
            }
        }
    }
}

/// The samples of a line of `length` pixels at `angle` degrees (counterclockwise from the
/// horizontal) through a pixel: their offsets from it, x right and y down, a pixel apart or less
/// from one end to the other (the GPU samples the same, ADR 0035).
pub fn line_offsets(angle: f64, length: f64) -> Vec<[f64; 2]> {
    let (sin, cos) = angle.to_radians().sin_cos();
    let samples = (length.ceil() as usize + 1).max(2);
    (0..samples)
        .map(|i| {
            let t = length * (i as f64 / (samples - 1) as f64 - 0.5);
            [t * cos, -t * sin]
        })
        .collect()
}

/// Motion Blur's line: equally weighted samples, a pixel apart or less, from one end of the
/// line through the pixel to the other, read bilinearly.
#[derive(Debug, Clone)]
pub(crate) struct Line {
    /// Each sample's offset from the pixel, x right and y down.
    offsets: Vec<[f64; 2]>,
}

impl Line {
    /// A line of `length` pixels at `angle` degrees, counterclockwise from the horizontal.
    pub(crate) fn new(angle: f64, length: f64) -> Self {
        Self {
            offsets: line_offsets(angle, length),
        }
    }

    pub(crate) fn reach(&self) -> usize {
        self.offsets
            .iter()
            .map(|o| o[0].abs().max(o[1].abs()).ceil() as usize + 1)
            .max()
            .unwrap_or(0)
    }

    /// Rows `first..` of `src` (`width` × `height`) blurred into `out` (whole rows), the edges
    /// repeating outward.
    fn rows(
        &self,
        src: &[[f32; 4]],
        width: usize,
        height: usize,
        first: usize,
        out: &mut [[f32; 4]],
    ) {
        if width == 0 || height == 0 {
            return;
        }
        let weight = 1.0 / self.offsets.len() as f64;
        let (last_x, last_y) = ((width - 1) as f64, (height - 1) as f64);
        let at = |x: usize, y: usize| src[y * width + x].map(f64::from);
        for (n, row) in out.chunks_mut(width).enumerate() {
            let y = (first + n) as f64;
            for (x, px) in row.iter_mut().enumerate() {
                let mut sum = [0.0f64; 4];
                for [dx, dy] in &self.offsets {
                    let sx = (x as f64 + dx).clamp(0.0, last_x);
                    let sy = (y + dy).clamp(0.0, last_y);
                    let (x0, y0) = (sx.floor() as usize, sy.floor() as usize);
                    let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
                    let (a, b) = (sx - x0 as f64, sy - y0 as f64);
                    let (p00, p10, p01, p11) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
                    for c in 0..4 {
                        sum[c] += (p00[c] * (1.0 - a) + p10[c] * a) * (1.0 - b)
                            + (p01[c] * (1.0 - a) + p11[c] * a) * b;
                    }
                }
                *px = sum.map(|v| (v * weight) as f32);
            }
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
        let motion = |angle, distance| Filter::MotionBlur { angle, distance };
        assert!(motion(-90.0, 2000.0).is_valid() && motion(90.0, 1.0).is_valid());
        for wrong in [motion(91.0, 10.0), motion(0.0, 0.5), motion(0.0, 2001.0)] {
            assert!(!wrong.is_valid(), "{wrong:?}");
        }
        assert_eq!(
            motion(30.0, 100.0).scaled(4.0),
            motion(30.0, 25.0),
            "the angle stays"
        );
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
        let out = sharpen(0.0).finish(original, blurred, [0, 0]);
        let expected = [0.7, 0.4, 0.1].map(|c| c * 0.5);
        for c in 0..3 {
            assert!((out[c] - expected[c]).abs() < 1e-12, "{out:?}");
        }
        assert_eq!(out[3], 0.5);
        // Within the threshold on every channel (0.2 is 51 levels, 0 is 0): left as it is.
        assert_eq!(sharpen(52.0).finish(original, blurred, [0, 0]), original);
        assert_ne!(sharpen(50.0).finish(original, blurred, [0, 0]), original);
        // Transparent stays transparent.
        assert_eq!(sharpen(0.0).finish([0.0; 4], blurred, [0, 0]), [0.0; 4]);
    }

    #[test]
    fn noise_is_the_pixels_and_the_seeds_and_spread_as_its_distribution() {
        // The same pixel and seed: the same noise; another of either: another.
        assert_eq!(
            noise([5, 9], 7, false, false),
            noise([5, 9], 7, false, false)
        );
        assert_ne!(
            noise([5, 9], 7, false, false),
            noise([5, 9], 8, false, false)
        );
        assert_ne!(
            noise([5, 9], 7, false, false),
            noise([6, 9], 7, false, false)
        );
        assert_ne!(
            noise([5, 9], 7, false, false),
            noise([-5, 9], 7, false, false)
        );
        let mono = noise([1, 2], 3, true, true);
        assert!(mono[0] == mono[1] && mono[1] == mono[2]);
        let color = noise([1, 2], 3, false, false);
        assert!(color[0] != color[1] && color[1] != color[2]);
        // Over many pixels: centered, uniform within ±0.5, both of variance 1/12.
        for gaussian in [false, true] {
            let values: Vec<f64> = (0..200_000)
                .map(|i| noise([i % 1000, i / 1000], 11, gaussian, false)[1])
                .collect();
            let n = values.len() as f64;
            let mean = values.iter().sum::<f64>() / n;
            let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
            assert!(mean.abs() < 0.005, "{gaussian}: mean {mean}");
            assert!(
                (variance - 1.0 / 12.0).abs() < 0.002,
                "{gaussian}: {variance}"
            );
            if !gaussian {
                assert!(values.iter().all(|v| v.abs() < 0.5));
            }
        }
    }

    #[test]
    fn add_noise_moves_colors_and_keeps_alpha() {
        let filter = |amount, monochromatic| Filter::AddNoise {
            amount,
            gaussian: false,
            monochromatic,
            seed: 4,
        };
        let original = [0.25, 0.25, 0.25, 0.5];
        let out = filter(100.0, false).finish(original, original, [3, 4]);
        let n = noise([3, 4], 4, false, false);
        for c in 0..3 {
            assert!((out[c] - (0.5 + n[c]) * 0.5).abs() < 1e-12, "{out:?}");
        }
        assert_eq!(out[3], 0.5);
        // Half the amount, half the noise; transparent stays transparent.
        let half = filter(50.0, false).finish(original, original, [3, 4]);
        assert!((half[0] - (0.5 + n[0] / 2.0) * 0.5).abs() < 1e-12);
        assert_eq!(
            filter(100.0, true).finish([0.0; 4], [0.0; 4], [3, 4]),
            [0.0; 4]
        );
        // Flags and seeds out of their values are not Add Noise's.
        assert_eq!(
            Filter::from_params("addNoise", &[10.0, 2.0, 0.0, 1.0]),
            None
        );
        assert_eq!(
            Filter::from_params("addNoise", &[10.0, 0.0, 0.0, 1.5]),
            None
        );
        assert_eq!(
            Filter::from_params("addNoise", &[10.0, 0.0, 0.0, NOISE_SEEDS as f32]),
            None
        );
        assert!(!filter(401.0, false).is_valid() && !filter(0.0, false).is_valid());
        assert_eq!(
            filter(100.0, true).scaled(4.0),
            filter(25.0, true),
            "a reduced pixel's noise is weaker"
        );
    }

    #[test]
    fn high_pass_is_the_difference_with_the_blur_around_middle_gray() {
        let high = Filter::HighPass { radius: 3.0 };
        let same = high.finish([0.4, 0.4, 0.4, 1.0], [0.4, 0.4, 0.4, 1.0], [0, 0]);
        for v in &same[..3] {
            assert!((v - 0.5).abs() < 1e-12, "{same:?}");
        }
        let out = high.finish([0.9, 0.1, 0.5, 1.0], [0.5, 0.5, 0.5, 1.0], [0, 0]);
        for (c, v) in [0.9, 0.1, 0.5].iter().enumerate() {
            assert!((out[c] - v).abs() < 1e-12, "{out:?}");
        }
        // Half opaque over a transparent blur (read as black): a straight 0.5 becomes 1.
        let out = high.finish([0.25, 0.25, 0.25, 0.5], [0.0; 4], [0, 0]);
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
        assert_eq!(Plan::gaussian(64.0).factor, 1);
        for (sigma, factor) in [(65.0, 4), (128.0, 4), (129.0, 8), (1000.0, 32)] {
            let plan = Plan::gaussian(sigma);
            assert_eq!(plan.factor, factor, "sigma {sigma}");
            assert!(
                plan.kernel.reach() <= 200,
                "sigma {sigma}: {}",
                plan.kernel.reach()
            );
        }
    }

    #[test]
    fn a_long_line_is_sampled_on_the_layer_reduced() {
        assert_eq!(Plan::line(30.0, 256.0).factor, 1);
        for (distance, factor) in [(257.0, 2), (512.0, 2), (2000.0, 8)] {
            let plan = Plan::line(0.0, distance);
            assert_eq!(plan.factor, factor, "distance {distance}");
            assert!(plan.kernel.reach() <= 130, "distance {distance}");
        }
    }

    #[test]
    fn a_motion_blur_spreads_a_point_along_its_line_only() {
        // A point in a 41 × 41 image, blurred over 11 pixels: horizontally, it becomes a
        // segment of the row; at 90 degrees, of the column; at 45, of the diagonal. Its mass
        // stays where the line is.
        let (w, h) = (41, 41);
        let mut image = vec![[0.0f32; 4]; w * h];
        image[20 * w + 20] = [1.0; 4];
        for (angle, along) in [(0.0, (1, 0)), (90.0, (0, -1)), (45.0, (1, -1))] {
            let kernel = Plan::line(angle, 10.0).kernel;
            let out = kernel.image(image.clone(), w, h);
            let mass: f32 = out.iter().map(|px| px[3]).sum();
            assert!((mass - 1.0).abs() < 1e-4, "{angle}: {mass}");
            // Along the line, a few pixels out: some of the point; across it, none.
            let (dx, dy) = along;
            let on = |k: i64| out[((20 + dy * k) as usize) * w + (20 + dx * k) as usize][3];
            assert!(on(2) > 0.02 && on(-2) > 0.02, "{angle}");
            // Three pixels across the line, (-dy, dx) from the point.
            let across = out[((20 + dx * 3) as usize) * w + (20 - dy * 3) as usize][3];
            assert!(across < 1e-6, "{angle}: {across}");
            // Nothing beyond its ends.
            assert!(on(8) < 1e-6 && on(-8) < 1e-6, "{angle}");
        }
        // The region path is the image path.
        let kernel = Plan::line(30.0, 7.0).kernel;
        let whole = kernel.image(image.clone(), w, h);
        let mut region = image;
        kernel.region(&mut region, w, h);
        assert_eq!(whole, region);
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
