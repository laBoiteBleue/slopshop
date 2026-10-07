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

/// Dust & Scratches' and Median's radius range in pixels (Photoshop's).
pub const MIN_MEDIAN_RADIUS: f32 = 1.0;
pub const MAX_MEDIAN_RADIUS: f32 = 500.0;

/// Box Blur's radius range in whole pixels (Photoshop's): each pixel the average of the square
/// of `2 radius + 1` pixels around it.
pub const MIN_BOX_RADIUS: f32 = 1.0;
pub const MAX_BOX_RADIUS: f32 = 2000.0;

/// Up to this radius, a Box Blur is computed on the layer itself; beyond, on the layer reduced
/// so that the radius is at most that (a smooth result, read back interpolated).
pub const BOX_UP_TO: f64 = 64.0;

/// Emboss's height range in whole pixels, and amount range in percent (Photoshop's).
pub const MIN_EMBOSS_HEIGHT: f32 = 1.0;
pub const MAX_EMBOSS_HEIGHT: f32 = 10.0;
pub const MIN_EMBOSS_AMOUNT: f32 = 1.0;
pub const MAX_EMBOSS_AMOUNT: f32 = 500.0;

/// Mosaic's cell size range in whole pixels (Photoshop's).
pub const MIN_MOSAIC_CELL: f32 = 2.0;
pub const MAX_MOSAIC_CELL: f32 = 200.0;

/// Offset's range in pixels, either way (Photoshop's).
pub const MAX_OFFSET: f32 = 30000.0;

/// Twirl's angle range in degrees, either way (Photoshop's).
pub const MAX_TWIRL: f32 = 999.0;

/// What Offset brings in where the layer moved away (Photoshop's Undefined Areas).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetEdge {
    Transparent,
    /// The edge's pixels repeated.
    Repeat,
    /// What went out on the other side.
    Wrap,
}

/// Maximum's and Minimum's radius range in whole pixels (Photoshop's, Squareness): each pixel
/// the largest (smallest) value of the square of `2 radius + 1` pixels around it.
pub const MIN_EXTREME_RADIUS: f32 = 1.0;
pub const MAX_EXTREME_RADIUS: f32 = 500.0;

/// Up to this radius, a median is taken on the layer itself; beyond, on the layer reduced (see
/// [`Plan::median`]).
pub const MEDIAN_UP_TO: f64 = 8.0;

/// Clarity and Texture's range (Lightroom's), and the radii of the blurs they push the colors
/// from, in the layer's pixels: Texture's fine details, Clarity's broad local contrast.
pub const MAX_CLARITY: f32 = 100.0;
const TEXTURE_RADIUS: f32 = 3.0;
const CLARITY_RADIUS: f32 = 25.0;

/// How strongly Clarity pushes the midtones at 100 (Texture pushes by its whole difference).
pub const CLARITY_STRENGTH: f64 = 0.6;

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
    /// Photoshop's Dust & Scratches: each pixel that differs from the median of the square of
    /// `radius` pixels around it by more than `threshold` levels (of 8 bits) on some channel
    /// becomes that median.
    DustAndScratches { radius: f32, threshold: f32 },
    /// Photoshop's Median: each pixel the median of the square of `radius` pixels around it,
    /// channel by channel (Dust & Scratches without a threshold).
    Median { radius: f32 },
    /// Photoshop's Box Blur: each pixel the average of the square of `radius` pixels around it.
    BoxBlur { radius: f32 },
    /// Photoshop's Maximum (Squareness): each pixel the largest value of the square of `radius`
    /// pixels around it, channel by channel: light areas spread, dark ones shrink.
    Maximum { radius: f32 },
    /// Photoshop's Minimum (Squareness): each pixel the smallest value of the square of
    /// `radius` pixels around it, channel by channel: dark areas spread, light ones shrink.
    Minimum { radius: f32 },
    /// Photoshop's Solarize: each color above the middle inverted, a negative and a positive
    /// blended (nothing lighter than middle gray).
    Solarize,
    /// Photoshop's Find Edges: each channel's edges dark lines on white, the stronger the change
    /// across them the darker (a Sobel gradient).
    FindEdges,
    /// Photoshop's Emboss: middle gray where the layer is flat, lighter or darker where it
    /// changes along the light coming from `angle` degrees, by the difference of the pixels
    /// `height` pixels before and after, `amount` percent of it.
    Emboss {
        angle: f32,
        height: f32,
        amount: f32,
    },
    /// Photoshop's Mosaic: square cells of `cell` pixels, from the layer's origin, each the
    /// average of its pixels.
    Mosaic { cell: f32 },
    /// Photoshop's Offset: the layer's pixels moved by `horizontal`, `vertical` pixels, `edge`
    /// what comes in.
    Offset {
        horizontal: f32,
        vertical: f32,
        edge: OffsetEdge,
    },
    /// Photoshop's Twirl: turned around the frame's center by `angle` degrees there, less and
    /// less out to the frame's inscribed circle (clockwise for a positive angle).
    Twirl { angle: f32 },
    /// Photoshop's Pinch: squeezed toward the frame's center (a positive `amount`, percent) or
    /// pushed out from it (negative), within the frame's inscribed circle.
    Pinch { amount: f32 },
    /// Photoshop's Spherize (Normal): as if wrapped around a sphere filling the frame's
    /// inscribed ellipse, bulging (a positive `amount`, percent) or hollow (negative).
    Spherize { amount: f32 },
    /// Photoshop's Polar Coordinates: rectangular to polar (`to_polar`: the frame's top edge at
    /// its center, its width around it) or back.
    PolarCoordinates { to_polar: bool },
    /// Lightroom's Texture and Clarity (-100 to 100): each color pushed away from (or, below 0,
    /// toward) its blur of a few pixels by `texture`, and from its blur of tens of pixels by
    /// `clarity`, in the midtones. `scale` is how many of the layer's pixels a pixel it is
    /// computed on stands for (1, more for a look): not a setting, the blurs' radii shrink by it.
    ClarityTexture {
        texture: f32,
        clarity: f32,
        scale: f32,
    },
}

impl Filter {
    /// Every filter's identifier, in menu order.
    pub const IDS: [&'static str; 20] = [
        "gaussianBlur",
        "motionBlur",
        "boxBlur",
        "unsharpMask",
        "addNoise",
        "dustAndScratches",
        "median",
        "clarityTexture",
        "highPass",
        "maximum",
        "minimum",
        "findEdges",
        "emboss",
        "solarize",
        "mosaic",
        "offset",
        "twirl",
        "pinch",
        "spherize",
        "polarCoordinates",
    ];

    /// The identifier the UI and files know it by.
    pub fn id(&self) -> &'static str {
        match self {
            Self::GaussianBlur { .. } => "gaussianBlur",
            Self::MotionBlur { .. } => "motionBlur",
            Self::UnsharpMask { .. } => "unsharpMask",
            Self::HighPass { .. } => "highPass",
            Self::AddNoise { .. } => "addNoise",
            Self::DustAndScratches { .. } => "dustAndScratches",
            Self::ClarityTexture { .. } => "clarityTexture",
            Self::Median { .. } => "median",
            Self::BoxBlur { .. } => "boxBlur",
            Self::Maximum { .. } => "maximum",
            Self::Minimum { .. } => "minimum",
            Self::Solarize => "solarize",
            Self::FindEdges => "findEdges",
            Self::Emboss { .. } => "emboss",
            Self::Mosaic { .. } => "mosaic",
            Self::Offset { .. } => "offset",
            Self::Twirl { .. } => "twirl",
            Self::Pinch { .. } => "pinch",
            Self::Spherize { .. } => "spherize",
            Self::PolarCoordinates { .. } => "polarCoordinates",
        }
    }

    /// Its parameters, in a fixed order per filter.
    pub fn params(&self) -> Vec<f32> {
        match *self {
            Self::GaussianBlur { radius }
            | Self::HighPass { radius }
            | Self::Median { radius }
            | Self::BoxBlur { radius }
            | Self::Maximum { radius }
            | Self::Minimum { radius } => vec![radius],
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
            Self::DustAndScratches { radius, threshold } => vec![radius, threshold],
            Self::ClarityTexture {
                texture, clarity, ..
            } => vec![texture, clarity],
            Self::Solarize | Self::FindEdges => Vec::new(),
            Self::Emboss {
                angle,
                height,
                amount,
            } => vec![angle, height, amount],
            Self::Mosaic { cell } => vec![cell],
            Self::Offset {
                horizontal,
                vertical,
                edge,
            } => vec![
                horizontal,
                vertical,
                match edge {
                    OffsetEdge::Transparent => 0.0,
                    OffsetEdge::Repeat => 1.0,
                    OffsetEdge::Wrap => 2.0,
                },
            ],
            Self::Twirl { angle } => vec![angle],
            Self::Pinch { amount } | Self::Spherize { amount } => vec![amount],
            Self::PolarCoordinates { to_polar } => vec![f32::from(u8::from(to_polar))],
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
            ("median", &[radius]) => Some(Self::Median { radius }),
            ("boxBlur", &[radius]) => Some(Self::BoxBlur { radius }),
            ("maximum", &[radius]) => Some(Self::Maximum { radius }),
            ("minimum", &[radius]) => Some(Self::Minimum { radius }),
            ("solarize", &[]) => Some(Self::Solarize),
            ("findEdges", &[]) => Some(Self::FindEdges),
            ("emboss", &[angle, height, amount]) => Some(Self::Emboss {
                angle,
                height,
                amount,
            }),
            ("mosaic", &[cell]) => Some(Self::Mosaic { cell }),
            ("offset", &[horizontal, vertical, edge]) => Some(Self::Offset {
                horizontal,
                vertical,
                edge: match edge {
                    0.0 => OffsetEdge::Transparent,
                    1.0 => OffsetEdge::Repeat,
                    2.0 => OffsetEdge::Wrap,
                    _ => return None,
                },
            }),
            ("twirl", &[angle]) => Some(Self::Twirl { angle }),
            ("pinch", &[amount]) => Some(Self::Pinch { amount }),
            ("spherize", &[amount]) => Some(Self::Spherize { amount }),
            ("polarCoordinates", &[to_polar]) => match to_polar {
                0.0 => Some(Self::PolarCoordinates { to_polar: false }),
                1.0 => Some(Self::PolarCoordinates { to_polar: true }),
                _ => None,
            },
            ("dustAndScratches", &[radius, threshold]) => {
                Some(Self::DustAndScratches { radius, threshold })
            }
            ("clarityTexture", &[texture, clarity]) => Some(Self::ClarityTexture {
                texture,
                clarity,
                scale: 1.0,
            }),
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
            "dustAndScratches" => Some(Self::DustAndScratches {
                radius: 1.0,
                threshold: 0.0,
            }),
            "clarityTexture" => Some(Self::ClarityTexture {
                texture: 0.0,
                clarity: 0.0,
                scale: 1.0,
            }),
            "median" => Some(Self::Median { radius: 1.0 }),
            "boxBlur" => Some(Self::BoxBlur { radius: 10.0 }),
            "maximum" => Some(Self::Maximum { radius: 1.0 }),
            "minimum" => Some(Self::Minimum { radius: 1.0 }),
            "solarize" => Some(Self::Solarize),
            "findEdges" => Some(Self::FindEdges),
            "emboss" => Some(Self::Emboss {
                angle: 135.0,
                height: 3.0,
                amount: 100.0,
            }),
            "mosaic" => Some(Self::Mosaic { cell: 10.0 }),
            "offset" => Some(Self::Offset {
                horizontal: 0.0,
                vertical: 0.0,
                edge: OffsetEdge::Wrap,
            }),
            "twirl" => Some(Self::Twirl { angle: 50.0 }),
            "pinch" => Some(Self::Pinch { amount: 50.0 }),
            "spherize" => Some(Self::Spherize { amount: 100.0 }),
            "polarCoordinates" => Some(Self::PolarCoordinates { to_polar: true }),
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
            Self::ClarityTexture {
                texture,
                clarity,
                scale,
            } => {
                let amount = |v: f32| v.is_finite() && (-MAX_CLARITY..=MAX_CLARITY).contains(&v);
                amount(texture) && amount(clarity) && scale.is_finite() && scale >= 1.0
            }
            // Whole pixels: a square of them.
            Self::DustAndScratches { radius, threshold } => {
                radius.fract() == 0.0
                    && (MIN_MEDIAN_RADIUS..=MAX_MEDIAN_RADIUS).contains(&radius)
                    && threshold.is_finite()
                    && (0.0..=MAX_THRESHOLD).contains(&threshold)
            }
            Self::Median { radius } => {
                radius.fract() == 0.0 && (MIN_MEDIAN_RADIUS..=MAX_MEDIAN_RADIUS).contains(&radius)
            }
            Self::BoxBlur { radius } => {
                radius.fract() == 0.0 && (MIN_BOX_RADIUS..=MAX_BOX_RADIUS).contains(&radius)
            }
            Self::Maximum { radius } | Self::Minimum { radius } => {
                radius.fract() == 0.0 && (MIN_EXTREME_RADIUS..=MAX_EXTREME_RADIUS).contains(&radius)
            }
            Self::Solarize | Self::FindEdges => true,
            Self::Emboss {
                angle,
                height,
                amount,
            } => {
                angle.is_finite()
                    && (-180.0..=180.0).contains(&angle)
                    && height.fract() == 0.0
                    && (MIN_EMBOSS_HEIGHT..=MAX_EMBOSS_HEIGHT).contains(&height)
                    && (MIN_EMBOSS_AMOUNT..=MAX_EMBOSS_AMOUNT).contains(&amount)
            }
            Self::Mosaic { cell } => {
                cell.fract() == 0.0 && (MIN_MOSAIC_CELL..=MAX_MOSAIC_CELL).contains(&cell)
            }
            Self::Offset {
                horizontal,
                vertical,
                ..
            } => [horizontal, vertical]
                .iter()
                .all(|v| v.fract() == 0.0 && (-MAX_OFFSET..=MAX_OFFSET).contains(v)),
            Self::Twirl { angle } => angle.is_finite() && (-MAX_TWIRL..=MAX_TWIRL).contains(&angle),
            Self::Pinch { amount } | Self::Spherize { amount } => {
                amount.is_finite() && (-100.0..=100.0).contains(&amount)
            }
            Self::PolarCoordinates { .. } => true,
        }
    }

    /// The radius of the Gaussian blur the filter is made from, if it is.
    pub fn blur_radius(&self) -> Option<f32> {
        match *self {
            Self::GaussianBlur { radius }
            | Self::UnsharpMask { radius, .. }
            | Self::HighPass { radius } => Some(radius),
            Self::MotionBlur { .. }
            | Self::AddNoise { .. }
            | Self::DustAndScratches { .. }
            | Self::ClarityTexture { .. }
            | Self::Median { .. }
            | Self::BoxBlur { .. }
            | Self::Maximum { .. }
            | Self::Minimum { .. }
            | Self::Solarize
            | Self::FindEdges
            | Self::Emboss { .. }
            | Self::Mosaic { .. }
            | Self::Offset { .. }
            | Self::Twirl { .. }
            | Self::Pinch { .. }
            | Self::Spherize { .. }
            | Self::PolarCoordinates { .. } => None,
        }
    }

    /// Clarity and Texture's blurs' radii, Texture's then Clarity's (see [`Self::plans`]).
    pub fn clarity_radii(&self) -> Option<[f32; 2]> {
        match *self {
            Self::ClarityTexture { scale, .. } => {
                Some([TEXTURE_RADIUS, CLARITY_RADIUS].map(|r| (r / scale).max(MIN_BLUR_RADIUS)))
            }
            _ => None,
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
            Self::DustAndScratches { radius, threshold } => Self::DustAndScratches {
                radius: (radius / factor).round().max(MIN_MEDIAN_RADIUS),
                threshold,
            },
            Self::Median { radius } => Self::Median {
                radius: (radius / factor).round().max(MIN_MEDIAN_RADIUS),
            },
            Self::BoxBlur { radius } => Self::BoxBlur {
                radius: (radius / factor).round().max(MIN_BOX_RADIUS),
            },
            Self::Maximum { radius } => Self::Maximum {
                radius: (radius / factor).round().max(MIN_EXTREME_RADIUS),
            },
            Self::Minimum { radius } => Self::Minimum {
                radius: (radius / factor).round().max(MIN_EXTREME_RADIUS),
            },
            Self::Solarize => Self::Solarize,
            // Lines of a pixel: as thin on a reduced layer.
            Self::FindEdges => Self::FindEdges,
            Self::Emboss {
                angle,
                height,
                amount,
            } => Self::Emboss {
                angle,
                height: (height / factor).round().max(MIN_EMBOSS_HEIGHT),
                amount,
            },
            // Cells of at least a pixel on a reduced layer (a look), from the same origin.
            Self::Mosaic { cell } => Self::Mosaic {
                cell: (cell / factor).max(1.0),
            },
            // Mapped in the layer's own pixels whatever the level (see [`Self::source`]).
            Self::Offset { .. }
            | Self::Twirl { .. }
            | Self::Pinch { .. }
            | Self::Spherize { .. }
            | Self::PolarCoordinates { .. } => *self,
            Self::ClarityTexture {
                texture,
                clarity,
                scale,
            } => Self::ClarityTexture {
                texture,
                clarity,
                scale: scale * factor,
            },
        }
    }

    /// How far a pixel's result reads, in pixels on each side, with some room (a look's
    /// margin): three and a half sigmas of a blur, half a line.
    pub fn reach(&self) -> f64 {
        match *self {
            Self::MotionBlur { distance, .. } => f64::from(distance) / 2.0 + 2.0,
            Self::AddNoise { .. } => 0.0,
            Self::DustAndScratches { radius, .. }
            | Self::Median { radius }
            | Self::BoxBlur { radius }
            | Self::Maximum { radius }
            | Self::Minimum { radius } => f64::from(radius) + 2.0,
            Self::Solarize => 0.0,
            Self::FindEdges => 2.0,
            Self::Emboss { height, .. } => f64::from(height) + 2.0,
            Self::Mosaic { cell } => f64::from(cell) + 2.0,
            // A pixel may read from anywhere in the layer.
            Self::Offset { .. }
            | Self::Twirl { .. }
            | Self::Pinch { .. }
            | Self::Spherize { .. }
            | Self::PolarCoordinates { .. } => f64::INFINITY,
            Self::ClarityTexture { scale, .. } => {
                3.5 * f64::from((CLARITY_RADIUS / scale).max(MIN_BLUR_RADIUS)) + 2.0
            }
            _ => 3.5 * f64::from(self.blur_radius().unwrap_or(0.0)) + 2.0,
        }
    }

    /// How the filter is computed over a layer: what it blurs by, at what reduction; Clarity
    /// and Texture blur twice ([`Self::finish`] gets one blur per plan, in this order).
    pub(crate) fn plans(&self) -> Vec<Plan> {
        match *self {
            Self::ClarityTexture { .. } => self
                .clarity_radii()
                .unwrap_or_default()
                .map(|r| Plan::gaussian(f64::from(r)))
                .to_vec(),
            _ => vec![self.plan()],
        }
    }

    fn plan(&self) -> Plan {
        match *self {
            Self::MotionBlur { angle, distance } => {
                Plan::line(f64::from(angle), f64::from(distance))
            }
            Self::AddNoise { .. } => Plan {
                factor: 1,
                kernel: Kernel::Identity,
            },
            Self::DustAndScratches { radius, .. } | Self::Median { radius } => {
                Plan::median(f64::from(radius))
            }
            Self::BoxBlur { radius } => Plan::boxed(f64::from(radius)),
            Self::Maximum { radius } => Plan {
                factor: 1,
                kernel: Kernel::Extreme {
                    radius: radius as usize,
                    largest: true,
                },
            },
            Self::Minimum { radius } => Plan {
                factor: 1,
                kernel: Kernel::Extreme {
                    radius: radius as usize,
                    largest: false,
                },
            },
            Self::Solarize => Plan {
                factor: 1,
                kernel: Kernel::Identity,
            },
            Self::FindEdges => Plan {
                factor: 1,
                kernel: Kernel::Sobel,
            },
            Self::Emboss { angle, height, .. } => {
                let (sin, cos) = f64::from(angle).to_radians().sin_cos();
                let h = f64::from(height);
                Plan {
                    factor: 1,
                    // Toward the light (up is negative y).
                    kernel: Kernel::Relief {
                        offset: [cos * h, -sin * h],
                    },
                }
            }
            Self::Mosaic { cell } => Plan {
                factor: 1,
                kernel: Kernel::Cells(f64::from(cell)),
            },
            // Sampled where [`Self::source`] says, not filtered by a kernel.
            Self::Offset { .. }
            | Self::Twirl { .. }
            | Self::Pinch { .. }
            | Self::Spherize { .. }
            | Self::PolarCoordinates { .. } => Plan {
                factor: 1,
                kernel: Kernel::Identity,
            },
            Self::ClarityTexture { .. } => Plan {
                factor: 1,
                kernel: Kernel::Identity,
            },
            _ => Plan::gaussian(f64::from(self.blur_radius().unwrap_or(MIN_BLUR_RADIUS))),
        }
    }

    /// Whether the filter moves pixels (a distortion): each pixel the layer sampled where
    /// [`Self::source`] says, rather than filtered by a kernel.
    pub fn samples(&self) -> bool {
        matches!(
            self,
            Self::Offset { .. }
                | Self::Twirl { .. }
                | Self::Pinch { .. }
                | Self::Spherize { .. }
                | Self::PolarCoordinates { .. }
        )
    }

    /// Where the pixel at layer point `p` (layer pixels) is read from, for a distortion of
    /// `frame` (`[left, top, right, bottom]`: the selection's box within the layer, or the
    /// layer) on a layer of `layer` pixels (width, height); `None`: nothing (transparent).
    pub fn source(&self, p: [f64; 2], frame: [f64; 4], layer: [f64; 2]) -> Option<[f64; 2]> {
        let [left, top, right, bottom] = frame;
        let (w, h) = (right - left, bottom - top);
        let c = [(left + right) / 2.0, (top + bottom) / 2.0];
        let v = [p[0] - c[0], p[1] - c[1]];
        let d = (v[0] * v[0] + v[1] * v[1]).sqrt();
        let radius = w.min(h) / 2.0;
        match *self {
            Self::Offset {
                horizontal,
                vertical,
                edge,
            } => {
                let q = [p[0] - f64::from(horizontal), p[1] - f64::from(vertical)];
                match edge {
                    // The edge repeats where the layer is read beyond it.
                    OffsetEdge::Repeat => Some(q),
                    OffsetEdge::Wrap => {
                        Some([q[0].rem_euclid(layer[0]), q[1].rem_euclid(layer[1])])
                    }
                    OffsetEdge::Transparent => {
                        (q[0] >= 0.0 && q[1] >= 0.0 && q[0] < layer[0] && q[1] < layer[1])
                            .then_some(q)
                    }
                }
            }
            Self::Twirl { angle } => {
                if d >= radius || radius <= 0.0 {
                    return Some(p);
                }
                // The most at the center, nothing on the circle; read from as far back.
                let turn = f64::from(angle).to_radians() * (1.0 - d / radius);
                let (sin, cos) = (-turn).sin_cos();
                Some([
                    c[0] + v[0] * cos - v[1] * sin,
                    c[1] + v[0] * sin + v[1] * cos,
                ])
            }
            Self::Pinch { amount } => {
                if d >= radius || radius <= 0.0 || d == 0.0 {
                    return Some(p);
                }
                let k = (std::f64::consts::FRAC_PI_2 * d / radius)
                    .sin()
                    .powf(-f64::from(amount) / 100.0);
                Some([c[0] + v[0] * k, c[1] + v[1] * k])
            }
            Self::Spherize { amount } => {
                let half = [w / 2.0, h / 2.0];
                if half[0] <= 0.0 || half[1] <= 0.0 {
                    return Some(p);
                }
                let u = [v[0] / half[0], v[1] / half[1]];
                let r = (u[0] * u[0] + u[1] * u[1]).sqrt();
                if r >= 1.0 || r == 0.0 {
                    return Some(p);
                }
                let a = f64::from(amount) / 100.0;
                // Bulging reads closer to the center (what is there grows), hollow farther.
                let target = if a >= 0.0 {
                    1.0 - (1.0 - r * r).sqrt()
                } else {
                    (1.0 - (1.0 - r).powi(2)).sqrt()
                };
                let s = (r + a.abs() * (target - r)) / r;
                Some([c[0] + u[0] * s * half[0], c[1] + u[1] * s * half[1]])
            }
            Self::PolarCoordinates { to_polar } => {
                let tau = std::f64::consts::TAU;
                if w <= 0.0 || h <= 0.0 || radius <= 0.0 {
                    return Some(p);
                }
                if to_polar {
                    // Around the center, clockwise from the top: the source's width; out to the
                    // inscribed circle: its height from the top edge.
                    let phi = v[0].atan2(-v[1]).rem_euclid(tau);
                    Some([left + phi / tau * w, top + d / radius * h])
                } else {
                    let phi = (p[0] - left) / w * tau;
                    let rho = (p[1] - top) / h * radius;
                    Some([c[0] + rho * phi.sin(), c[1] - rho * phi.cos()])
                }
            }
            _ => Some(p),
        }
    }

    /// Whether a pixel's result depends on its own value besides its blur's.
    pub(crate) fn reads_original(&self) -> bool {
        !matches!(
            self,
            Self::GaussianBlur { .. }
                | Self::MotionBlur { .. }
                | Self::Median { .. }
                | Self::BoxBlur { .. }
                | Self::Maximum { .. }
                | Self::Minimum { .. }
                | Self::Mosaic { .. }
                | Self::Offset { .. }
                | Self::Twirl { .. }
                | Self::Pinch { .. }
                | Self::Spherize { .. }
                | Self::PolarCoordinates { .. }
        )
    }

    /// A pixel's result from its premultiplied value `original` and its blurs' `blurs` (one per
    /// plan, [`Self::plans`]), in the blend space (values of 1 are white); `at` is the document
    /// pixel it shows. Unsharp Mask, High Pass, Add Noise, Clarity and Texture work on the colors
    /// (straight, a blur's by its own coverage), and keep the pixel's alpha.
    pub(crate) fn finish(&self, original: [f64; 4], blurs: &[[f64; 4]], at: [i64; 2]) -> [f64; 4] {
        let blurred = blurs[0];
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
            Self::GaussianBlur { .. }
            | Self::MotionBlur { .. }
            | Self::Median { .. }
            | Self::BoxBlur { .. }
            | Self::Maximum { .. }
            | Self::Minimum { .. }
            | Self::Mosaic { .. }
            | Self::Offset { .. }
            | Self::Twirl { .. }
            | Self::Pinch { .. }
            | Self::Spherize { .. }
            | Self::PolarCoordinates { .. } => blurred,
            Self::Solarize => color(&|o, _| if o > 0.5 { 1.0 - o } else { o }),
            // The kernel's gradient magnitude, per channel: dark lines on white, alpha kept.
            Self::FindEdges => {
                if alpha <= 0.0 {
                    return original;
                }
                let c: [f64; 3] =
                    std::array::from_fn(|i| 1.0 - (blurred[i] / alpha).clamp(0.0, 1.0));
                [c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha]
            }
            // Middle gray, moved by the kernel's difference across the pixel.
            Self::Emboss { amount, .. } => {
                if alpha <= 0.0 {
                    return original;
                }
                let k = f64::from(amount) / 100.0;
                let c: [f64; 3] = std::array::from_fn(|i| 0.5 + k * blurred[i] / alpha);
                [c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha]
            }
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
            // On the premultiplied values, alpha included: a speck of another opacity goes too.
            Self::DustAndScratches { threshold, .. } => {
                let level = f64::from(threshold) / 255.0;
                let differs = (0..4).any(|c| (original[c] - blurred[c]).abs() > level);
                if differs { blurred } else { original }
            }
            Self::ClarityTexture {
                texture, clarity, ..
            } => {
                if alpha <= 0.0 {
                    return original;
                }
                let broad = straight(blurs[1]);
                // Midtones most, the darkest and lightest not at all (Rec. 709 luma).
                let luma = 0.2126 * o[0] + 0.7152 * o[1] + 0.0722 * o[2];
                let midtones = (1.0 - (2.0 * luma - 1.0).powi(2)).clamp(0.0, 1.0);
                let kt = f64::from(texture) / 100.0;
                let kc = f64::from(clarity) / 100.0 * CLARITY_STRENGTH * midtones;
                let c: [f64; 3] =
                    std::array::from_fn(|i| o[i] + kt * (o[i] - b[i]) + kc * (o[i] - broad[i]));
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

    /// The median of a square of `radius` pixels a side from its center: up to
    /// [`MEDIAN_UP_TO`] directly; beyond, on the layer reduced so that the radius is at most
    /// that (an approximation of a large median: specks that large are rarely dust).
    pub(crate) fn median(radius: f64) -> Self {
        let factor = if radius <= MEDIAN_UP_TO {
            1
        } else {
            1usize << (radius / MEDIAN_UP_TO).log2().ceil() as u32
        };
        Self {
            factor,
            kernel: Kernel::Median((radius / factor as f64).round().max(1.0) as usize),
        }
    }

    /// A box of `radius` whole pixels: up to [`BOX_UP_TO`] directly; beyond, on the layer
    /// reduced so that the radius is at most that.
    pub(crate) fn boxed(radius: f64) -> Self {
        let factor = if radius <= BOX_UP_TO {
            1
        } else {
            1usize << (radius / BOX_UP_TO).log2().ceil() as u32
        };
        let reduced = (radius / factor as f64).round().max(1.0) as usize;
        Self {
            factor,
            kernel: Kernel::Gaussian(Blur::Boxes([reduced, 0, 0])),
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
    /// A separable blur: a Gaussian, or boxes (Box Blur's one).
    Gaussian(Blur),
    Line(Line),
    /// The median of the square of this radius around each pixel, channel by channel.
    Median(usize),
    /// The largest (or smallest) value of the square of `radius` around each pixel, channel by
    /// channel.
    Extreme {
        radius: usize,
        largest: bool,
    },
    /// The magnitude of the Sobel gradient of each channel, a full step 1.
    Sobel,
    /// The difference of the pixels `offset` after and before each one (read bilinearly).
    Relief {
        offset: [f64; 2],
    },
    /// The average of the square cell of this side each pixel lies in, cells from the layer's
    /// origin (see [`Kernel::region_at`]).
    Cells(f64),
    Identity,
}

impl Kernel {
    /// How far a pixel's result reads, in pixels on each side.
    pub(crate) fn reach(&self) -> usize {
        match self {
            Self::Gaussian(blur) => blur.reach(),
            Self::Line(line) => line.reach(),
            Self::Median(radius) | Self::Extreme { radius, .. } => *radius,
            Self::Sobel => 1,
            Self::Relief { offset } => offset[0].abs().max(offset[1].abs()).ceil() as usize + 1,
            Self::Cells(cell) => cell.ceil() as usize,
            Self::Identity => 0,
        }
    }

    /// [`Self::region`] for a region whose first pixel is pixel `origin` of a layer of
    /// `layer` pixels (width, height): what a kernel placed on the layer needs (Mosaic's
    /// cells start at its origin and stop at its edges).
    pub(crate) fn region_at(
        &self,
        region: &mut [[f32; 4]],
        width: usize,
        height: usize,
        origin: [i64; 2],
        layer: [usize; 2],
    ) {
        match self {
            &Self::Cells(cell) => cells(region, width, height, cell, origin, layer),
            _ => self.region(region, width, height),
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
            Self::Median(radius) => {
                let source = region.to_vec();
                median_rows(&source, width, height, *radius, 0, region);
            }
            &Self::Extreme { radius, largest } => extreme(region, width, height, radius, largest),
            Self::Sobel => sobel(region, width, height),
            &Self::Relief { offset } => relief(region, width, height, offset),
            &Self::Cells(cell) => cells(region, width, height, cell, [0, 0], [width, height]),
            Self::Identity => {}
        }
    }

    /// `image` (`width` × `height` pixels) blurred, its edges repeating outward, on every core.
    pub(crate) fn image(&self, image: Vec<[f32; 4]>, width: usize, height: usize) -> Vec<[f32; 4]> {
        match self {
            Self::Gaussian(blur) => blur.image(image, width, height),
            Self::Identity => image,
            // Never reduced (no plan of a factor above 1 takes them): one thread is enough.
            Self::Extreme { .. } | Self::Sobel | Self::Relief { .. } | Self::Cells(_) => {
                let mut image = image;
                self.region(&mut image, width, height);
                image
            }
            Self::Median(radius) => {
                let mut out = vec![[0.0f32; 4]; image.len()];
                if width == 0 {
                    return out;
                }
                let mut bands: Vec<(usize, &mut [[f32; 4]])> =
                    out.chunks_mut(width * BAND_ROWS).enumerate().collect();
                parallel_for_each(&mut bands, |(index, band)| {
                    median_rows(&image, width, height, *radius, *index * BAND_ROWS, band);
                });
                out
            }
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

/// `region` (`width` × `height`) in place: each pixel the largest (`largest`) or smallest value
/// of the square of `radius` around it, channel by channel, its edges repeating outward. A row
/// pass then a column pass, each exact in three comparisons a pixel whatever the radius (van
/// Herk and Gil–Werman: running extremes forward and backward within windows of the square's
/// side, each window then spanning at most two of them).
fn extreme(region: &mut [[f32; 4]], width: usize, height: usize, radius: usize, largest: bool) {
    if width == 0 || height == 0 || radius == 0 {
        return;
    }
    let mut buffers = ExtremeLine::default();
    for row in region.chunks_mut(width) {
        buffers.line(row, radius, largest);
    }
    let mut column = vec![[0.0f32; 4]; height];
    for x in 0..width {
        for (y, px) in column.iter_mut().enumerate() {
            *px = region[y * width + x];
        }
        buffers.line(&mut column, radius, largest);
        for (y, px) in column.iter().enumerate() {
            region[y * width + x] = *px;
        }
    }
}

/// `region` (`width` × `height`) in place: each channel's Sobel gradient magnitude, a step of 1
/// across the pixel giving 1, the edges repeating outward.
fn sobel(region: &mut [[f32; 4]], width: usize, height: usize) {
    let source = region.to_vec();
    let at = |x: i64, y: i64| {
        let x = x.clamp(0, width as i64 - 1) as usize;
        let y = y.clamp(0, height as i64 - 1) as usize;
        source[y * width + x]
    };
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            region[y as usize * width + x as usize] = std::array::from_fn(|c| {
                let p = |dx: i64, dy: i64| f64::from(at(x + dx, y + dy)[c]);
                let gx =
                    (p(1, -1) + 2.0 * p(1, 0) + p(1, 1)) - (p(-1, -1) + 2.0 * p(-1, 0) + p(-1, 1));
                let gy =
                    (p(-1, 1) + 2.0 * p(0, 1) + p(1, 1)) - (p(-1, -1) + 2.0 * p(0, -1) + p(1, -1));
                ((gx * gx + gy * gy).sqrt() / 4.0) as f32
            });
        }
    }
}

/// `region` (`width` × `height`) in place: the pixel at `offset` after each one minus the one
/// as far before it, read bilinearly, the edges repeating outward.
fn relief(region: &mut [[f32; 4]], width: usize, height: usize, offset: [f64; 2]) {
    let source = region.to_vec();
    let at = |x: f64, y: f64| -> [f64; 4] {
        let x = x.clamp(0.0, (width - 1) as f64);
        let y = y.clamp(0.0, (height - 1) as f64);
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
        let (a, b) = (x - x0 as f64, y - y0 as f64);
        let px = |x: usize, y: usize| source[y * width + x].map(f64::from);
        let (p00, p10, p01, p11) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
        std::array::from_fn(|c| {
            (p00[c] * (1.0 - a) + p10[c] * a) * (1.0 - b) + (p01[c] * (1.0 - a) + p11[c] * a) * b
        })
    };
    let [dx, dy] = offset;
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f64, y as f64);
            let (after, before) = (at(fx + dx, fy + dy), at(fx - dx, fy - dy));
            region[y * width + x] = std::array::from_fn(|c| (after[c] - before[c]) as f32);
        }
    }
}

/// `region` (`width` × `height`, its first pixel pixel `origin` of a layer of `layer` pixels) in
/// place: each pixel the average of the cell of `cell` pixels it lies in, cells from the
/// layer's origin, only the layer's pixels counting (a cell cut by its edge averages what is
/// in it). The cells of the region's pixels must lie within it (a margin of a cell).
fn cells(
    region: &mut [[f32; 4]],
    width: usize,
    height: usize,
    cell: f64,
    origin: [i64; 2],
    layer: [usize; 2],
) {
    if width == 0 || height == 0 {
        return;
    }
    let source = region.to_vec();
    // The cell of layer coordinate `v` along an axis: its first and last layer pixels, within
    // the layer.
    let span = |v: i64, size: usize| {
        let index = (v as f64 / cell).floor();
        let start = (index * cell).ceil() as i64;
        let end = (((index + 1.0) * cell).ceil() as i64).min(size as i64);
        (start.max(0), end.max(start + 1))
    };
    for y in 0..height {
        let ly = origin[1] + y as i64;
        let (y0, y1) = span(ly.clamp(0, layer[1] as i64 - 1), layer[1]);
        for x in 0..width {
            let lx = origin[0] + x as i64;
            let (x0, x1) = span(lx.clamp(0, layer[0] as i64 - 1), layer[0]);
            let mut sum = [0.0f64; 4];
            let mut count = 0.0;
            for sy in y0..y1 {
                let ry = (sy - origin[1]).clamp(0, height as i64 - 1) as usize;
                for sx in x0..x1 {
                    let rx = (sx - origin[0]).clamp(0, width as i64 - 1) as usize;
                    let px = source[ry * width + rx];
                    for c in 0..4 {
                        sum[c] += f64::from(px[c]);
                    }
                    count += 1.0;
                }
            }
            region[y * width + x] = sum.map(|v| (v / count) as f32);
        }
    }
}

/// The buffers of [`extreme`]'s passes, reused from line to line.
#[derive(Default)]
struct ExtremeLine {
    forward: Vec<[f32; 4]>,
    backward: Vec<[f32; 4]>,
}

impl ExtremeLine {
    /// `line` replaced by the extreme of the `2 radius + 1` values around each, its ends
    /// repeating outward.
    fn line(&mut self, line: &mut [[f32; 4]], radius: usize, largest: bool) {
        let n = line.len();
        let side = 2 * radius + 1;
        let padded = n + 2 * radius;
        let pick = |a: [f32; 4], b: [f32; 4]| -> [f32; 4] {
            std::array::from_fn(|c| {
                if largest {
                    a[c].max(b[c])
                } else {
                    a[c].min(b[c])
                }
            })
        };
        let at = |i: usize| line[i.saturating_sub(radius).min(n - 1)];
        // Within each window of `side` values: the extreme from its start to each value…
        self.forward.clear();
        for i in 0..padded {
            let v = at(i);
            let v = match i % side {
                0 => v,
                _ => pick(self.forward[i - 1], v),
            };
            self.forward.push(v);
        }
        // …and from each value to its end.
        self.backward.clear();
        self.backward.resize(padded, [0.0; 4]);
        for i in (0..padded).rev() {
            let v = at(i);
            self.backward[i] = if i % side == side - 1 || i == padded - 1 {
                v
            } else {
                pick(self.backward[i + 1], v)
            };
        }
        // The square around pixel `j` is padded values `j..j + side`.
        for (j, out) in line.iter_mut().enumerate() {
            *out = pick(self.backward[j], self.forward[j + side - 1]);
        }
    }
}

/// Rows `first..` of `src` (`width` × `height`) into `out` (whole rows): each pixel the median
/// of the square of `radius` around it, channel by channel, the edges repeating outward. Exact:
/// the middle of the window's values, found by selection.
fn median_rows(
    src: &[[f32; 4]],
    width: usize,
    height: usize,
    radius: usize,
    first: usize,
    out: &mut [[f32; 4]],
) {
    if width == 0 || height == 0 {
        return;
    }
    let r = radius as i64;
    let side = 2 * radius + 1;
    // The window's values, channel by channel, gathered once a pixel.
    let mut windows = vec![[0.0f32; 4]; side * side];
    let mut channel = vec![0.0f32; side * side];
    for (n, row) in out.chunks_mut(width).enumerate() {
        let y = (first + n) as i64;
        let rows_inside = y >= r && y + r < height as i64;
        for (x, px) in row.iter_mut().enumerate() {
            if rows_inside && x as i64 >= r && x as i64 + r < width as i64 {
                // Inside: the window's rows as they are.
                for (k, line) in windows.chunks_exact_mut(side).enumerate() {
                    let start = (y - r + k as i64) as usize * width + x - radius;
                    line.copy_from_slice(&src[start..start + side]);
                }
            } else {
                let mut k = 0;
                for dy in -r..=r {
                    let sy = (y + dy).clamp(0, height as i64 - 1) as usize;
                    for dx in -r..=r {
                        let sx = (x as i64 + dx).clamp(0, width as i64 - 1) as usize;
                        windows[k] = src[sy * width + sx];
                        k += 1;
                    }
                }
            }
            for (c, out) in px.iter_mut().enumerate() {
                for (value, sample) in channel.iter_mut().zip(&windows) {
                    *value = sample[c];
                }
                let middle = channel.len() / 2;
                let (_, median, _) = channel.select_nth_unstable_by(middle, f32::total_cmp);
                *out = *median;
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
        // Each sample's whole-pixel offset and bilinear weights, the same at every pixel: where
        // every sample and its neighbors fall inside, no clamping or rounding per pixel.
        let taps: Vec<Tap> = self.offsets.iter().map(|&o| Tap::new(o)).collect();
        let span = |pick: fn(&Tap) -> i64| {
            let low = taps.iter().map(pick).min().unwrap_or(0);
            let high = taps.iter().map(pick).max().unwrap_or(0);
            (low, high + 1)
        };
        let ((lx, hx), (ly, hy)) = (span(|t| t.dx), span(|t| t.dy));
        let inside = |x: i64, y: i64| {
            x + lx >= 0 && x + hx < width as i64 && y + ly >= 0 && y + hy < height as i64
        };
        for (n, row) in out.chunks_mut(width).enumerate() {
            let y = (first + n) as f64;
            for (x, px) in row.iter_mut().enumerate() {
                let mut sum = [0.0f64; 4];
                if inside(x as i64, y as i64) {
                    for tap in &taps {
                        let x0 = (x as i64 + tap.dx) as usize;
                        let i = (y as i64 + tap.dy) as usize * width + x0;
                        let (p00, p10) = (&src[i], &src[i + 1]);
                        let (p01, p11) = (&src[i + width], &src[i + width + 1]);
                        for c in 0..4 {
                            sum[c] += f64::from(p00[c]) * tap.w[0]
                                + f64::from(p10[c]) * tap.w[1]
                                + f64::from(p01[c]) * tap.w[2]
                                + f64::from(p11[c]) * tap.w[3];
                        }
                    }
                } else {
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
                }
                *px = sum.map(|v| (v * weight) as f32);
            }
        }
    }
}

/// One sample of a line away from the edges: the pixel above and left of it, from the pixel
/// blurred, and the weights of that pixel, the next one right, the one below and below right.
struct Tap {
    dx: i64,
    dy: i64,
    w: [f64; 4],
}

impl Tap {
    fn new([dx, dy]: [f64; 2]) -> Self {
        let (fx, fy) = (dx.floor(), dy.floor());
        let (a, b) = (dx - fx, dy - fy);
        Self {
            dx: fx as i64,
            dy: fy as i64,
            w: [(1.0 - a) * (1.0 - b), a * (1.0 - b), (1.0 - a) * b, a * b],
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
        assert_eq!(Filter::from_params("oilPaint", &[1.0]), None);
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
        let out = sharpen(0.0).finish(original, &[blurred], [0, 0]);
        let expected = [0.7, 0.4, 0.1].map(|c| c * 0.5);
        for c in 0..3 {
            assert!((out[c] - expected[c]).abs() < 1e-12, "{out:?}");
        }
        assert_eq!(out[3], 0.5);
        // Within the threshold on every channel (0.2 is 51 levels, 0 is 0): left as it is.
        assert_eq!(sharpen(52.0).finish(original, &[blurred], [0, 0]), original);
        assert_ne!(sharpen(50.0).finish(original, &[blurred], [0, 0]), original);
        // Transparent stays transparent.
        assert_eq!(sharpen(0.0).finish([0.0; 4], &[blurred], [0, 0]), [0.0; 4]);
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
        let out = filter(100.0, false).finish(original, &[original], [3, 4]);
        let n = noise([3, 4], 4, false, false);
        for c in 0..3 {
            assert!((out[c] - (0.5 + n[c]) * 0.5).abs() < 1e-12, "{out:?}");
        }
        assert_eq!(out[3], 0.5);
        // Half the amount, half the noise; transparent stays transparent.
        let half = filter(50.0, false).finish(original, &[original], [3, 4]);
        assert!((half[0] - (0.5 + n[0] / 2.0) * 0.5).abs() < 1e-12);
        assert_eq!(
            filter(100.0, true).finish([0.0; 4], &[[0.0; 4]], [3, 4]),
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
    fn a_median_takes_specks_out_and_keeps_edges() {
        // A speck in a flat field, and a straight edge: the median of 3 × 3 removes the one,
        // keeps the other.
        let (w, h) = (20, 12);
        let mut image: Vec<[f32; 4]> = (0..w * h)
            .map(|i| if i % w < 10 { [0.2; 4] } else { [0.8; 4] })
            .collect();
        image[5 * w + 4] = [1.0; 4];
        let kernel = Plan::median(1.0).kernel;
        let out = kernel.image(image.clone(), w, h);
        assert_eq!(out[5 * w + 4], [0.2; 4]);
        for y in 0..h {
            assert_eq!(out[y * w + 9], [0.2; 4], "row {y}");
            assert_eq!(out[y * w + 10], [0.8; 4], "row {y}");
        }
        let mut region = image;
        kernel.region(&mut region, w, h);
        assert_eq!(region, out);
        // Large radii on the layer reduced.
        assert_eq!(Plan::median(8.0).factor, 1);
        assert_eq!(Plan::median(9.0).factor, 2);
        assert_eq!(Plan::median(500.0).factor, 64);
        assert!(Plan::median(500.0).kernel.reach() <= 8);
    }

    #[test]
    fn dust_and_scratches_replaces_what_differs_beyond_its_threshold() {
        let filter = |threshold| Filter::DustAndScratches {
            radius: 2.0,
            threshold,
        };
        let (speck, median) = ([0.6, 0.5, 0.5, 1.0], [0.5, 0.5, 0.5, 1.0]);
        // 0.1 is 25.5 levels.
        assert_eq!(filter(0.0).finish(speck, &[median], [0, 0]), median);
        assert_eq!(filter(25.0).finish(speck, &[median], [0, 0]), median);
        assert_eq!(filter(26.0).finish(speck, &[median], [0, 0]), speck);
        assert!(filter(255.0).is_valid());
        for wrong in [
            filter(256.0),
            Filter::DustAndScratches {
                radius: 1.5,
                threshold: 0.0,
            },
        ] {
            assert!(!wrong.is_valid(), "{wrong:?}");
        }
        assert_eq!(
            Filter::DustAndScratches {
                radius: 10.0,
                threshold: 4.0
            }
            .scaled(4.0),
            Filter::DustAndScratches {
                radius: 3.0,
                threshold: 4.0
            }
        );
    }

    #[test]
    fn clarity_and_texture_push_colors_from_their_two_blurs() {
        let filter = |texture, clarity| Filter::ClarityTexture {
            texture,
            clarity,
            scale: 1.0,
        };
        let (o, fine, broad) = ([0.6, 0.5, 0.4, 1.0], [0.5; 4], [0.4, 0.4, 0.4, 1.0]);
        let fine = [fine[0], fine[1], fine[2], 1.0];
        // Nothing at 0.
        assert_eq!(filter(0.0, 0.0).finish(o, &[fine, broad], [0, 0]), o);
        // Texture at -100: the fine blur itself; at 100, as far again from it.
        let smooth = filter(-100.0, 0.0).finish(o, &[fine, broad], [0, 0]);
        let sharp = filter(100.0, 0.0).finish(o, &[fine, broad], [0, 0]);
        for c in 0..3 {
            assert!((smooth[c] - fine[c]).abs() < 1e-12);
            assert!((sharp[c] - (2.0 * o[c] - fine[c])).abs() < 1e-12);
        }
        // Clarity: in the midtones, not at black or white.
        let mid = filter(0.0, 100.0).finish(o, &[fine, broad], [0, 0]);
        assert!(mid[0] > o[0] && mid[2] == o[2], "{mid:?}");
        let white = [1.0; 4];
        assert_eq!(
            filter(0.0, 100.0).finish(white, &[fine, broad], [0, 0]),
            white
        );
        // Ranges; the scale is no setting: looks shrink the blurs, the settings round-trip.
        assert!(filter(-100.0, 100.0).is_valid() && !filter(101.0, 0.0).is_valid());
        let look = filter(30.0, 40.0).scaled(4.0);
        assert_eq!(
            Filter::from_params("clarityTexture", &look.params()),
            Some(filter(30.0, 40.0))
        );
        assert!(look.reach() < filter(30.0, 40.0).reach());
        assert_eq!(filter(1.0, 1.0).plans().len(), 2);
    }

    #[test]
    fn high_pass_is_the_difference_with_the_blur_around_middle_gray() {
        let high = Filter::HighPass { radius: 3.0 };
        let same = high.finish([0.4, 0.4, 0.4, 1.0], &[[0.4, 0.4, 0.4, 1.0]], [0, 0]);
        for v in &same[..3] {
            assert!((v - 0.5).abs() < 1e-12, "{same:?}");
        }
        let out = high.finish([0.9, 0.1, 0.5, 1.0], &[[0.5, 0.5, 0.5, 1.0]], [0, 0]);
        for (c, v) in [0.9, 0.1, 0.5].iter().enumerate() {
            assert!((out[c] - v).abs() < 1e-12, "{out:?}");
        }
        // Half opaque over a transparent blur (read as black): a straight 0.5 becomes 1.
        let out = high.finish([0.25, 0.25, 0.25, 0.5], &[[0.0; 4]], [0, 0]);
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

    #[test]
    fn the_line_and_the_median_inside_are_those_clamped_at_every_sample() {
        // Away from the edges, samples are read without clamping: the same values as reading
        // every sample clamped (the line but for the order of its sums).
        let (w, h) = (61usize, 47usize);
        let src: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let v = ((i * 2654435761) % 997) as f32 / 997.0;
                [v, 1.0 - v, (v * 7.0).fract(), 0.5 + v / 2.0]
            })
            .collect();
        let clamped = |sx: f64, sy: f64| {
            let (sx, sy) = (sx.clamp(0.0, (w - 1) as f64), sy.clamp(0.0, (h - 1) as f64));
            let (x0, y0) = (sx.floor() as usize, sy.floor() as usize);
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            let (a, b) = (sx - x0 as f64, sy - y0 as f64);
            let at = |x: usize, y: usize| src[y * w + x].map(f64::from);
            let (p00, p10, p01, p11) = (at(x0, y0), at(x1, y0), at(x0, y1), at(x1, y1));
            std::array::from_fn::<f64, 4, _>(|c| {
                (p00[c] * (1.0 - a) + p10[c] * a) * (1.0 - b)
                    + (p01[c] * (1.0 - a) + p11[c] * a) * b
            })
        };
        for (angle, length) in [(30.0, 9.0), (0.0, 12.0), (-90.0, 5.5), (137.0, 20.0)] {
            let line = Line::new(angle, length);
            let mut out = vec![[0.0f32; 4]; w * h];
            line.rows(&src, w, h, 0, &mut out);
            for y in 0..h {
                for x in 0..w {
                    let mut sum = [0.0f64; 4];
                    for [dx, dy] in &line.offsets {
                        let v = clamped(x as f64 + dx, y as f64 + dy);
                        for c in 0..4 {
                            sum[c] += v[c];
                        }
                    }
                    let expected = sum.map(|v| (v / line.offsets.len() as f64) as f32);
                    for c in 0..4 {
                        let d = (out[y * w + x][c] - expected[c]).abs();
                        assert!(d <= 1e-6, "{angle} {length} ({x}, {y}): {d}");
                    }
                }
            }
        }
        for radius in [1usize, 3] {
            let mut out = vec![[0.0f32; 4]; w * h];
            median_rows(&src, w, h, radius, 0, &mut out);
            let r = radius as i64;
            for y in 0..h as i64 {
                for x in 0..w as i64 {
                    for c in 0..4 {
                        let mut window: Vec<f32> = (-r..=r)
                            .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
                            .map(|(dx, dy)| {
                                let sx = (x + dx).clamp(0, w as i64 - 1) as usize;
                                let sy = (y + dy).clamp(0, h as i64 - 1) as usize;
                                src[sy * w + sx][c]
                            })
                            .collect();
                        window.sort_by(f32::total_cmp);
                        let expected = window[window.len() / 2];
                        assert_eq!(out[(y as usize) * w + x as usize][c], expected);
                    }
                }
            }
        }
    }

    /// Timing of Motion Blur's line and Dust & Scratches' median on a 1500 x 1000 region, one
    /// thread (run with `--ignored --nocapture`).
    #[test]
    #[ignore = "benchmark"]
    fn bench_line_and_median() {
        let (w, h) = (1500usize, 1000usize);
        let src: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let v = ((i * 2654435761) % 1000) as f32 / 1000.0;
                [v, 1.0 - v, v * 0.5, 1.0]
            })
            .collect();
        let mut out = vec![[0.0f32; 4]; w * h];
        for (angle, length) in [(30.0, 50.0), (0.0, 50.0)] {
            let line = Line::new(angle, length);
            let start = std::time::Instant::now();
            line.rows(&src, w, h, 0, &mut out);
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            println!(
                "motion blur {length} px at {angle}: {ms:.1} ms ({:?})",
                out[w * h / 2]
            );
        }
        for radius in [2, 4] {
            let start = std::time::Instant::now();
            median_rows(&src, w, h, radius, 0, &mut out);
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            println!("median radius {radius}: {ms:.1} ms ({:?})", out[w * h / 2]);
        }
    }

    /// Deterministic values in [0, 1) for every channel of `n` pixels.
    fn speckled(n: usize, seed: u32) -> Vec<[f32; 4]> {
        (0..n as u32)
            .map(|i| {
                std::array::from_fn(|c| (hash(i * 4 + c as u32 + seed) >> 8) as f32 / 16_777_216.0)
            })
            .collect()
    }

    #[test]
    fn maximum_and_minimum_are_the_extremes_of_the_square_around() {
        let (w, h) = (23, 17);
        let source = speckled(w * h, 3);
        let brute = |x: usize, y: usize, r: usize, largest: bool| -> [f32; 4] {
            let mut best = [if largest { f32::MIN } else { f32::MAX }; 4];
            for dy in -(r as i64)..=r as i64 {
                for dx in -(r as i64)..=r as i64 {
                    let sx = (x as i64 + dx).clamp(0, w as i64 - 1) as usize;
                    let sy = (y as i64 + dy).clamp(0, h as i64 - 1) as usize;
                    let px = source[sy * w + sx];
                    for c in 0..4 {
                        best[c] = if largest {
                            best[c].max(px[c])
                        } else {
                            best[c].min(px[c])
                        };
                    }
                }
            }
            best
        };
        // Radii within the region, as wide as it, and wider.
        for r in [1, 2, 5, 11, 30] {
            for largest in [true, false] {
                let mut region = source.clone();
                Kernel::Extreme { radius: r, largest }.region(&mut region, w, h);
                for y in 0..h {
                    for x in 0..w {
                        assert_eq!(
                            region[y * w + x],
                            brute(x, y, r, largest),
                            "{r} {largest} ({x}, {y})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_bright_point_grows_into_a_square_under_maximum_and_goes_under_minimum() {
        let (w, h) = (15, 15);
        let mut point = vec![[0.0f32; 4]; w * h];
        point[7 * w + 7] = [1.0; 4];
        let mut grown = point.clone();
        Kernel::Extreme {
            radius: 2,
            largest: true,
        }
        .region(&mut grown, w, h);
        for y in 0..h {
            for x in 0..w {
                let inside = (5..=9).contains(&x) && (5..=9).contains(&y);
                assert_eq!(
                    grown[y * w + x][0],
                    if inside { 1.0 } else { 0.0 },
                    "({x}, {y})"
                );
            }
        }
        let mut gone = point;
        Kernel::Extreme {
            radius: 1,
            largest: false,
        }
        .region(&mut gone, w, h);
        assert!(gone.iter().all(|px| *px == [0.0; 4]));
    }

    #[test]
    fn a_box_blur_spreads_a_point_evenly_over_its_square() {
        let filter = Filter::BoxBlur { radius: 3.0 };
        let plans = filter.plans();
        assert_eq!(plans[0].factor, 1);
        let (w, h) = (21, 21);
        let mut region = vec![[0.0f32; 4]; w * h];
        region[10 * w + 10] = [49.0; 4];
        plans[0].kernel.region(&mut region, w, h);
        for y in 0..h {
            for x in 0..w {
                let inside = (7..=13).contains(&x) && (7..=13).contains(&y);
                let v = region[y * w + x][0];
                assert!(
                    (v - if inside { 1.0 } else { 0.0 }).abs() < 1e-5,
                    "({x}, {y}) {v}"
                );
            }
        }
        // A large radius: on the layer reduced, the radius at most 64 there.
        let large = Filter::BoxBlur { radius: 2000.0 }.plans().remove(0);
        assert_eq!(large.factor, 32);
        assert_eq!(large.kernel.reach(), 63);
    }

    #[test]
    fn the_filters_of_a_square_take_whole_radii_in_their_ranges() {
        for (filter, max) in [
            (Filter::Median { radius: 1.0 }, 500.0),
            (Filter::BoxBlur { radius: 1.0 }, 2000.0),
            (Filter::Maximum { radius: 1.0 }, 500.0),
            (Filter::Minimum { radius: 1.0 }, 500.0),
        ] {
            let id = filter.id();
            let with = |radius: f32| Filter::from_params(id, &[radius]).unwrap();
            assert!(with(1.0).is_valid() && with(max).is_valid(), "{id}");
            for wrong in [0.0, 1.5, max + 1.0, f32::NAN] {
                assert!(!with(wrong).is_valid(), "{id} {wrong}");
            }
            assert!(!filter.reads_original(), "{id}");
            // Reduced: the radius too, whole, at least 1.
            assert_eq!(with(40.0).scaled(4.0), with(10.0), "{id}");
            assert_eq!(with(1.0).scaled(4.0), with(1.0), "{id}");
        }
    }

    #[test]
    fn solarize_inverts_what_is_above_the_middle() {
        let s = Filter::Solarize;
        let gray = |v: f64| [v, v, v, 1.0];
        let out = |v: f64| s.finish(gray(v), &[gray(v)], [0, 0])[0];
        assert!((out(0.2) - 0.2).abs() < 1e-12);
        assert!((out(0.8) - 0.2).abs() < 1e-12);
        assert!((out(0.5) - 0.5).abs() < 1e-12);
        // Half transparent: on the color, alpha kept.
        let half = s.finish([0.4, 0.4, 0.4, 0.5], &[[0.0; 4]], [0, 0]);
        assert!((half[0] - 0.1).abs() < 1e-12 && half[3] == 0.5, "{half:?}");
        assert!(Filter::from_params("solarize", &[]).is_some_and(|f| f.is_valid()));
        assert_eq!(Filter::from_params("solarize", &[1.0]), None);
    }

    #[test]
    fn find_edges_draws_a_steps_edge_dark_on_white() {
        // Black then white at x = 5, 10 × 4.
        let (w, h) = (10, 4);
        let mut region: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                if i % w < 5 {
                    [0.0, 0.0, 0.0, 1.0]
                } else {
                    [1.0; 4]
                }
            })
            .collect();
        Kernel::Sobel.region(&mut region, w, h);
        let edges = Filter::FindEdges;
        let shown = |x: usize, region: &[[f32; 4]]| {
            let original = if x < 5 {
                [0.0, 0.0, 0.0, 1.0]
            } else {
                [1.0; 4]
            };
            edges.finish(original, &[region[w + x].map(f64::from)], [0, 0])[0]
        };
        // Flat: white; on either side of the step: dark.
        assert!((shown(1, &region) - 1.0).abs() < 1e-6);
        assert!((shown(8, &region) - 1.0).abs() < 1e-6);
        assert!(shown(4, &region) < 0.01 && shown(5, &region) < 0.01);
    }

    #[test]
    fn emboss_is_middle_gray_where_flat_and_moves_along_its_light() {
        let (w, h) = (12, 3);
        // A ramp rising to the right.
        let ramp: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let v = (i % w) as f32 / 20.0;
                [v, v, v, 1.0]
            })
            .collect();
        let emboss = |angle: f32| {
            let filter = Filter::Emboss {
                angle,
                height: 2.0,
                amount: 100.0,
            };
            let mut region = ramp.clone();
            filter.plans()[0].kernel.region(&mut region, w, h);
            filter.finish(
                ramp[w + 6].map(f64::from),
                &[region[w + 6].map(f64::from)],
                [0, 0],
            )[0]
        };
        // Lit from the right: lighter (the pixel after is lighter); from the left: darker; across
        // the ramp: middle gray.
        assert!(emboss(0.0) > 0.6, "{}", emboss(0.0));
        assert!(emboss(180.0) < 0.4, "{}", emboss(180.0));
        assert!((emboss(90.0) - 0.5).abs() < 1e-6, "{}", emboss(90.0));
        assert!(
            Filter::Emboss {
                angle: 135.0,
                height: 3.0,
                amount: 100.0
            }
            .is_valid()
        );
        for wrong in [
            (181.0, 3.0, 100.0),
            (0.0, 2.5, 100.0),
            (0.0, 11.0, 100.0),
            (0.0, 3.0, 501.0),
        ] {
            let (angle, height, amount) = wrong;
            assert!(
                !Filter::Emboss {
                    angle,
                    height,
                    amount
                }
                .is_valid(),
                "{wrong:?}"
            );
        }
    }

    #[test]
    fn mosaic_cells_start_at_the_layers_origin_whatever_the_region() {
        let (w, h) = (23, 9);
        let image = speckled(w * h, 5);
        let cell = 4.0;
        let mut whole = image.clone();
        Kernel::Cells(cell).region(&mut whole, w, h);
        // Every pixel of a cell is its average; the last cells, cut by the edge, average what
        // is in them.
        let average = |x0: usize, x1: usize, y0: usize, y1: usize, c: usize| {
            let mut sum = 0.0f64;
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += f64::from(image[y * w + x][c]);
                }
            }
            sum / ((x1 - x0) * (y1 - y0)) as f64
        };
        assert!((f64::from(whole[0][0]) - average(0, 4, 0, 4, 0)).abs() < 1e-5);
        assert!((f64::from(whole[8 * w + 22][1]) - average(20, 23, 8, 9, 1)).abs() < 1e-5);
        assert_eq!(whole[w + 5], whole[3 * w + 7]);
        // A region placed within the layer (with a margin of a cell) gives the same pixels.
        let (ox, oy, rw, rh) = (6usize, 2usize, 13usize, 7usize);
        let mut region: Vec<[f32; 4]> = (0..rw * rh)
            .map(|i| image[(oy + i / rw) * w + ox + i % rw])
            .collect();
        Kernel::Cells(cell).region_at(&mut region, rw, rh, [ox as i64, oy as i64], [w, h]);
        for y in 4..rh - 1 {
            for x in 4..rw - 4 {
                assert_eq!(
                    region[y * rw + x],
                    whole[(oy + y) * w + ox + x],
                    "({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn distortions_read_from_where_they_move_pixels() {
        let frame = [0.0, 0.0, 200.0, 100.0];
        let layer = [200.0, 100.0];
        let center = [100.0, 50.0];
        let close =
            |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
        let distance =
            |a: [f64; 2]| ((a[0] - center[0]).powi(2) + (a[1] - center[1]).powi(2)).sqrt();
        // Twirl: the center stays, points turn along their circle, nothing beyond it moves.
        let twirl = Filter::Twirl { angle: 180.0 };
        assert!(close(twirl.source(center, frame, layer).unwrap(), center));
        let p = [120.0, 50.0];
        let q = twirl.source(p, frame, layer).unwrap();
        assert!((distance(q) - 20.0).abs() < 1e-9 && !close(q, p));
        assert!(close(
            twirl.source([10.0, 5.0], frame, layer).unwrap(),
            [10.0, 5.0]
        ));
        // Pinch in: read from farther out, so what is there shrinks toward the center.
        let pinch = Filter::Pinch { amount: 50.0 };
        assert!(distance(pinch.source(p, frame, layer).unwrap()) > 20.0);
        let push = Filter::Pinch { amount: -50.0 };
        assert!(distance(push.source(p, frame, layer).unwrap()) < 20.0);
        // Spherize: a bulge reads closer to the center; a hollow farther.
        let bulge = Filter::Spherize { amount: 100.0 };
        assert!(distance(bulge.source(p, frame, layer).unwrap()) < 20.0);
        let hollow = Filter::Spherize { amount: -100.0 };
        assert!(distance(hollow.source(p, frame, layer).unwrap()) > 20.0);
        // Polar coordinates there and back again: where it was.
        let (to, from) = (
            Filter::PolarCoordinates { to_polar: true },
            Filter::PolarCoordinates { to_polar: false },
        );
        for p in [[130.0, 40.0], [80.0, 70.0], [101.0, 20.0]] {
            let there = from.source(p, frame, layer).unwrap();
            let back = to.source(there, frame, layer).unwrap();
            assert!(close(back, p), "{p:?} {there:?} {back:?}");
        }
        // Offset: wrapped around, the edge repeated, or nothing.
        let offset = |edge| Filter::Offset {
            horizontal: 30.0,
            vertical: -10.0,
            edge,
        };
        assert!(close(
            offset(OffsetEdge::Wrap)
                .source([10.0, 95.0], frame, layer)
                .unwrap(),
            [180.0, 5.0]
        ));
        assert_eq!(
            offset(OffsetEdge::Transparent).source([10.0, 50.0], frame, layer),
            None
        );
        assert!(close(
            offset(OffsetEdge::Repeat)
                .source([10.0, 50.0], frame, layer)
                .unwrap(),
            [-20.0, 60.0]
        ));
        // Settings.
        assert!(Filter::Twirl { angle: -999.0 }.is_valid());
        assert!(!Filter::Twirl { angle: 1000.0 }.is_valid());
        assert!(!Filter::Pinch { amount: 101.0 }.is_valid());
        assert!(
            !Filter::Offset {
                horizontal: 30000.5,
                vertical: 0.0,
                edge: OffsetEdge::Wrap
            }
            .is_valid()
        );
        assert_eq!(Filter::from_params("offset", &[1.0, 2.0, 3.0]), None);
        assert_eq!(Filter::from_params("polarCoordinates", &[0.5]), None);
        assert!(twirl.samples() && !Filter::Mosaic { cell: 4.0 }.samples());
        assert_eq!(twirl.scaled(4.0), twirl, "mapped in the layer's own pixels");
    }
}
