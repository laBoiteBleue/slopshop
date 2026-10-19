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

/// Crystallize's and Pointillize's cell size range in whole pixels (Photoshop's).
pub const MIN_CRYSTAL_CELL: f32 = 3.0;
pub const MAX_CRYSTAL_CELL: f32 = 300.0;

/// Pointillize's dots' radius range, in parts of a cell: drawn for each dot.
const DOT_RADIUS: [f64; 2] = [0.5, 0.69];

/// What Crystallize's points and Pointillize's dots are drawn with besides the seed (each
/// takes a few salts from these).
const CRYSTAL_SALT: u32 = 0xc710;
const DOT_SALT: u32 = 0x9d10;

/// Color Halftone's Max Radius range in whole pixels (Photoshop's): a dot's radius where its
/// channel is full, when it covers its whole cell (a square of `√2 radius` a side).
pub const MIN_HALFTONE_RADIUS: f32 = 4.0;
pub const MAX_HALFTONE_RADIUS: f32 = 127.0;

/// Color Halftone's Screen Angles range in degrees, either way (Photoshop's).
pub const MAX_SCREEN_ANGLE: f32 = 360.0;

/// Below this Max Radius in pixels of a reduced level (a look), Color Halftone's dots are
/// smaller than a pixel: the look shows the layer as it is (their coverage keeps its tone).
const HALFTONE_FROM: f64 = 1.5;

/// How far apart Fragment's four copies are from the pixel, diagonally, in pixels on each
/// axis.
pub const FRAGMENT_OFFSET: f64 = 4.0;

/// Facet's Kuwahara radius, in the layer's pixels.
const FACET_RADIUS: f64 = 2.0;

/// Mezzotint's Type (Photoshop's): the random pattern each channel is thresholded against,
/// dots (of a pixel, of clumps of about 2 pixels, of clumps and specks, of clumps of about 4),
/// horizontal lines or diagonal strokes (of 4, 12 or 32 pixels and 4, 8 or 16).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MezzotintType {
    FineDots,
    MediumDots,
    GrainyDots,
    CoarseDots,
    ShortLines,
    MediumLines,
    LongLines,
    ShortStrokes,
    MediumStrokes,
    LongStrokes,
}

impl MezzotintType {
    /// Every type, in Photoshop's order (its index is its parameter).
    pub const ALL: [Self; 10] = [
        Self::FineDots,
        Self::MediumDots,
        Self::GrainyDots,
        Self::CoarseDots,
        Self::ShortLines,
        Self::MediumLines,
        Self::LongLines,
        Self::ShortStrokes,
        Self::MediumStrokes,
        Self::LongStrokes,
    ];

    /// Its parameter: its index in [`Self::ALL`].
    pub fn index(self) -> u32 {
        Self::ALL.iter().position(|&t| t == self).unwrap_or(0) as u32
    }

    /// The type of parameter `v`, `None` when it is not one.
    fn of(v: f32) -> Option<Self> {
        (v.fract() == 0.0 && v >= 0.0)
            .then(|| Self::ALL.get(v as usize).copied())
            .flatten()
    }
}

/// Up to this many of the layer's pixels a side, Mezzotint's look of a reduced level counts
/// each of them under a pixel; beyond, this many evenly spaced.
const MEZZOTINT_SAMPLES: usize = 4;

/// Offset's range in pixels, either way (Photoshop's).
pub const MAX_OFFSET: f32 = 30000.0;

/// Twirl's angle range in degrees, either way (Photoshop's).
pub const MAX_TWIRL: f32 = 999.0;

/// Radial Blur's amount range (Photoshop's): Spin turns through as many degrees, Zoom reads along
/// the line to the center up to [`ZOOM_SPAN`] of its distance at 100.
pub const MIN_RADIAL_AMOUNT: f32 = 1.0;
pub const MAX_RADIAL_AMOUNT: f32 = 100.0;

/// The part of a pixel's distance to the center a Zoom of amount 100 reads along, half inward
/// and half outward.
pub const ZOOM_SPAN: f64 = 0.5;

/// Surface Blur's radius range in whole pixels, and threshold range in levels (Photoshop's).
pub const MIN_SURFACE_RADIUS: f32 = 1.0;
pub const MAX_SURFACE_RADIUS: f32 = 100.0;
pub const MIN_SURFACE_THRESHOLD: f32 = 2.0;
pub const MAX_SURFACE_THRESHOLD: f32 = 255.0;

/// A Surface Blur reads at most this many pixels on each side of a pixel along an axis: up to
/// that radius every pixel of the square, beyond every few (the square's pixels evenly spaced),
/// so that whatever the radius it costs about the same.
pub const SURFACE_TAPS: usize = 8;

/// Smart Blur's radius and threshold ranges (Photoshop's): the radius in pixels, the threshold
/// in levels (of 8 bits).
pub const MIN_SMART_RADIUS: f32 = 0.1;
pub const MAX_SMART_RADIUS: f32 = 100.0;
pub const MIN_SMART_THRESHOLD: f32 = 0.1;
pub const MAX_SMART_THRESHOLD: f32 = 100.0;

/// Smart Blur's Quality (Photoshop's Low, Medium, High): at most how many pixels it reads on each
/// side of a pixel along an axis (beyond, the disk's pixels evenly spaced).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartQuality {
    Low,
    Medium,
    High,
}

impl SmartQuality {
    fn taps(self) -> usize {
        match self {
            Self::Low => 4,
            Self::Medium => 8,
            Self::High => 12,
        }
    }
}

/// What Smart Blur shows (Photoshop's Mode): the blur, its edges white on black, or the blur with
/// its edges white over it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartMode {
    Normal,
    EdgeOnly,
    OverlayEdge,
}

/// A Radial Blur's sampling (Photoshop's Quality): Draft few samples (grainy), Good one per pixel
/// of the path, Best two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadialQuality {
    Draft,
    Good,
    Best,
}

/// Ripple's amount range in percent, either way (Photoshop's).
pub const MAX_RIPPLE_AMOUNT: f32 = 999.0;

/// How far Ripple moves a pixel at an amount of 100 %, in parts of its ripples' wavelength.
pub const RIPPLE_STRENGTH: f64 = 0.125;

/// Ripple's Size (Photoshop's Small, Medium, Large): its ripples' wavelength.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RippleSize {
    Small,
    Medium,
    Large,
}

impl RippleSize {
    /// The ripples' wavelength in the layer's pixels.
    pub fn wavelength(self) -> f64 {
        match self {
            Self::Small => 8.0,
            Self::Medium => 16.0,
            Self::Large => 32.0,
        }
    }
}

/// Wave's ranges (Photoshop's): generators, wavelengths and amplitudes (pixels) from 1 to 999,
/// scales from 1 to 100 %.
pub const MAX_WAVE_SETTING: f32 = 999.0;

/// Wave's Type (Photoshop's): the shape of its waves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveType {
    Sine,
    Triangle,
    Square,
}

impl WaveType {
    /// The wave at `t` periods: from -1 to 1, 0 at 0, rising.
    pub fn shape(self, t: f64) -> f64 {
        match self {
            Self::Sine => (std::f64::consts::TAU * t).sin(),
            Self::Triangle => 1.0 - 4.0 * ((t + 0.25).rem_euclid(1.0) - 0.5).abs(),
            Self::Square => {
                if t.rem_euclid(1.0) < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }
}

/// ZigZag's ranges (Photoshop's): the amount in percent either way, ridges from 1 to 20.
pub const MAX_ZIGZAG_AMOUNT: f32 = 100.0;
pub const MAX_ZIGZAG_RIDGES: f32 = 20.0;

/// How far ZigZag turns a pixel around the center at an amount of 100 % (Around Center), in
/// radians.
pub const ZIGZAG_TURN: f64 = 1.0;

/// ZigZag's Style (Photoshop's): ridges turning around the center, pushing out from it, or
/// moving pixels toward the upper left and the lower right (Pond Ripples).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZigZagStyle {
    AroundCenter,
    OutFromCenter,
    PondRipples,
}

/// Spherize's Mode (Photoshop's): around a sphere, or a cylinder standing (Horizontal Only) or
/// lying (Vertical Only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpherizeMode {
    Normal,
    HorizontalOnly,
    VerticalOnly,
}

impl RadialQuality {
    /// Samples per pixel of the path, and at most how many.
    fn sampling(self) -> (f64, usize) {
        match self {
            Self::Draft => (0.25, 24),
            Self::Good => (1.0, 96),
            Self::Best => (2.0, 256),
        }
    }
}

/// What Offset brings in where the layer moved away (Photoshop's Undefined Areas).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetEdge {
    Transparent,
    /// The edge's pixels repeated.
    Repeat,
    /// What went out on the other side.
    Wrap,
}

/// Displace's Horizontal and Vertical Scale range in percent, either way (Photoshop's).
pub const MAX_DISPLACE_SCALE: f32 = 999.0;

/// Shear's points at most (as many as a Curves curve's).
pub const SHEAR_POINTS: usize = crate::curve::CURVE_POINTS;

/// Shear's curve (Photoshop's): how far each row of the frame moves sideways, a smooth curve
/// (a natural cubic spline, as Curves') through 2 to [`SHEAR_POINTS`] points `[row, offset]`:
/// `row` in percent of the frame's height from its top (the first point at 0, the last at 100,
/// strictly increasing), `offset` in percent of half the frame's width (-100 to 100, positive to
/// the right: the edge of Photoshop's grid moves a row by half the frame).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShearCurve {
    points: [[f32; 2]; SHEAR_POINTS],
    len: u8,
    /// The spline's second derivative at each point (of offsets and rows as fractions): the
    /// points' function, kept so that each pixel only evaluates it.
    second: [f32; SHEAR_POINTS],
}

impl ShearCurve {
    /// The straight vertical line (Photoshop's Default): no row moves.
    pub const STRAIGHT: ShearCurve = {
        let mut points = [[0.0; 2]; SHEAR_POINTS];
        points[1] = [100.0, 0.0];
        ShearCurve {
            points,
            len: 2,
            second: [0.0; SHEAR_POINTS],
        }
    };

    /// The curve through `points`; `None` unless they are 2 to [`SHEAR_POINTS`], finite, rows
    /// from 0 to 100 strictly increasing, offsets within -100 to 100.
    pub fn new(points: &[[f32; 2]]) -> Option<ShearCurve> {
        let n = points.len();
        if !(2..=SHEAR_POINTS).contains(&n)
            || points[0][0] != 0.0
            || points[n - 1][0] != 100.0
            || points
                .iter()
                .any(|&[row, x]| !row.is_finite() || !(-100.0..=100.0).contains(&x))
            || points.windows(2).any(|w| w[0][0] >= w[1][0])
        {
            return None;
        }
        let spline = crate::curve::Spline::new(
            points
                .iter()
                .map(|&[row, x]| (f64::from(row) / 100.0, f64::from(x) / 100.0))
                .collect(),
        );
        let mut curve = ShearCurve {
            points: [[0.0; 2]; SHEAR_POINTS],
            len: n as u8,
            second: [0.0; SHEAR_POINTS],
        };
        curve.points[..n].copy_from_slice(points);
        for (s, &v) in curve.second.iter_mut().zip(spline.second()) {
            *s = v as f32;
        }
        Some(curve)
    }

    pub fn points(&self) -> &[[f32; 2]] {
        &self.points[..usize::from(self.len)]
    }

    /// The spline's second derivatives, a value per point (for the GPU, which evaluates the
    /// same spline).
    pub fn second(&self) -> &[f32] {
        &self.second[..usize::from(self.len)]
    }

    /// Whether no row moves.
    pub fn is_straight(&self) -> bool {
        self.points().iter().all(|&[_, x]| x == 0.0)
    }

    /// How far the row at `t` (a fraction of the frame's height from its top, clamped to
    /// `[0, 1]`) moves, in fractions of half the frame's width, within -1 to 1.
    pub fn offset(&self, t: f64) -> f64 {
        let p = self.points();
        let at = |i: usize| (f64::from(p[i][0]) / 100.0, f64::from(p[i][1]) / 100.0);
        let n = p.len();
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        let k = p
            .partition_point(|q| f64::from(q[0]) / 100.0 <= t)
            .clamp(1, n - 1);
        let ((x0, y0), (x1, y1)) = (at(k - 1), at(k));
        let h = x1 - x0;
        let (a, b) = ((x1 - t) / h, (t - x0) / h);
        let (s0, s1) = (f64::from(self.second[k - 1]), f64::from(self.second[k]));
        let y = a * y0 + b * y1 + ((a * a * a - a) * s0 + (b * b * b - b) * s1) * h * h / 6.0;
        y.clamp(-1.0, 1.0)
    }
}

/// How Diffuse moves pixels (Photoshop's Mode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffuseMode {
    /// Each pixel one of its neighbours, drawn at random.
    Normal,
    /// Only where that neighbour is darker.
    DarkenOnly,
    /// Only where that neighbour is lighter.
    LightenOnly,
    /// The neighbour closest in color: pixels shuffled along edges rather than across them.
    Anisotropic,
}

/// Wind's Method (Photoshop's): streaks, longer and stronger ones, or staggered ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindMethod {
    Wind,
    Blast,
    Stagger,
}

impl WindMethod {
    /// Its longest streak at full size (layer pixels), how strongly a streak covers what it
    /// crosses, and whether streaks are staggered (start further along, keep their strength).
    fn streaks(self) -> (f64, f64, bool) {
        match self {
            Self::Wind => (WIND_LENGTH, 0.6, false),
            Self::Blast => (2.0 * WIND_LENGTH, 0.9, false),
            Self::Stagger => (WIND_LENGTH, 0.7, true),
        }
    }
}

/// Wind's longest streak (Wind and Stagger; Blast's twice as long), in the layer's pixels.
pub const WIND_LENGTH: f64 = 30.0;

/// The difference of luma between two neighbours (0 to 1) from which Wind blows a streak.
const WIND_EDGE: f32 = 0.05;

/// What fills the gaps Tiles leaves (Photoshop's Fill Empty Area With).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TilesFill {
    Background,
    Foreground,
    /// The layer's pixel there, inverted.
    Inverse,
    /// The layer's pixel there.
    Unaltered,
}

/// Tiles' Number of Tiles and Maximum Offset (percent) range (Photoshop's).
pub const MAX_TILES_SETTING: f32 = 99.0;

/// Oil Paint's Stylization, Cleanliness, Scale, Bristle Detail and Shine range (Photoshop's);
/// Stylization and Scale from 0.1.
pub const MAX_OIL_SETTING: f32 = 10.0;

/// How strongly Oil Paint's light shades the paint's relief, at Shine 1 and Scale 1.
const OIL_RELIEF: f64 = 1.5;

/// How much less a quadrant's variance must be to be another quadrant's than the first's.
const OIL_TIE: f64 = 1e-7;

/// Maximum's and Minimum's radius range in whole pixels (Photoshop's, Squareness): each pixel
/// the largest (smallest) value of the square of `2 radius + 1` pixels around it.
pub const MIN_EXTREME_RADIUS: f32 = 1.0;
pub const MAX_EXTREME_RADIUS: f32 = 500.0;

/// Up to this radius, Maximum's and Minimum's Roundness is exact on the layer itself; beyond, on
/// the layer reduced (blocks' extremes) so that the radius is at most that.
pub const ROUND_UP_TO: f64 = 32.0;

/// Custom's ranges (Photoshop's): each of the 5 × 5 weights, Scale and Offset, whole numbers.
pub const MAX_CUSTOM_WEIGHT: f32 = 999.0;
pub const MAX_CUSTOM_SCALE: f32 = 9999.0;
pub const MAX_CUSTOM_OFFSET: f32 = 9999.0;

/// Custom's weights: 5 × 5, row by row from the top left.
pub const CUSTOM_WEIGHTS: usize = 25;

/// How HSB/HSL reads and writes a pixel's three channels (Photoshop's Input Mode and Row
/// Order): red, green, blue; or hue, saturation, brightness (HSV); or hue, saturation,
/// lightness. A hue from 0 to 1 is 0° to 360°.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorModel {
    Rgb,
    Hsb,
    Hsl,
}

impl ColorModel {
    fn index(self) -> f32 {
        match self {
            Self::Rgb => 0.0,
            Self::Hsb => 1.0,
            Self::Hsl => 2.0,
        }
    }

    fn of(v: f32) -> Option<Self> {
        match v {
            0.0 => Some(Self::Rgb),
            1.0 => Some(Self::Hsb),
            2.0 => Some(Self::Hsl),
            _ => None,
        }
    }

    /// Red, green and blue (0 to 1) as this model's three channels.
    fn encode(self, [r, g, b]: [f64; 3]) -> [f64; 3] {
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let chroma = max - min;
        let hue = if chroma <= 0.0 {
            0.0
        } else if max == r {
            ((g - b) / chroma).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / chroma + 2.0) / 6.0
        } else {
            ((r - g) / chroma + 4.0) / 6.0
        };
        match self {
            Self::Rgb => [r, g, b],
            Self::Hsb => [hue, if max > 0.0 { chroma / max } else { 0.0 }, max],
            Self::Hsl => {
                let l = (max + min) / 2.0;
                let d = 1.0 - (2.0 * l - 1.0).abs();
                [hue, if d > 0.0 { chroma / d } else { 0.0 }, l]
            }
        }
    }

    /// This model's three channels (0 to 1) as red, green and blue.
    fn decode(self, [h, s, v]: [f64; 3]) -> [f64; 3] {
        let (chroma, low) = match self {
            Self::Rgb => return [h, s, v],
            Self::Hsb => (v * s, v - v * s),
            Self::Hsl => {
                let c = (1.0 - (2.0 * v - 1.0).abs()) * s;
                (c, v - c / 2.0)
            }
        };
        let h6 = h.rem_euclid(1.0) * 6.0;
        let x = chroma * (1.0 - (h6.rem_euclid(2.0) - 1.0).abs());
        let [r, g, b] = match h6 as u32 {
            0 => [chroma, x, 0.0],
            1 => [x, chroma, 0.0],
            2 => [0.0, chroma, x],
            3 => [0.0, x, chroma],
            4 => [x, 0.0, chroma],
            _ => [chroma, 0.0, x],
        };
        [r + low, g + low, b + low]
    }
}

/// Reduce Noise's Strength range (Photoshop's), and how far apart (in values, 1 being white)
/// two neighbours may be and still be smoothed together at Strength 10, Preserve Details 0.
pub const MAX_NOISE_STRENGTH: f32 = 10.0;
const NOISE_SIGMA: f64 = 0.1;

/// The same for Reduce Color Noise at 100 %, on the colors without their lightness.
const COLOR_NOISE_SIGMA: f64 = 0.15;

/// The radii Reduce Noise smooths the lightness and the colors over, in the layer's pixels
/// (color noise is blotchier).
const NOISE_RADIUS: f64 = 2.0;
const COLOR_NOISE_RADIUS: f64 = 4.0;

/// Despeckle: the radius and the difference it smooths over (Reduce Noise's computation, on
/// the lightness and the colors alike).
const SPECKLE_RADIUS: f64 = 2.0;
const SPECKLE_SIGMA: f64 = 0.04;

/// Smart Sharpen's Amount and Radius ranges (Photoshop's).
pub const MIN_SMART_SHARPEN_RADIUS: f32 = 0.1;
pub const MAX_SMART_SHARPEN_RADIUS: f32 = 64.0;

/// The difference (in values) Smart Sharpen's Reduce Noise at 100 % leaves half sharpened:
/// smaller ones (grain) less, larger ones (edges) fully.
const SHARPEN_NOISE: f64 = 0.04;

/// What Smart Sharpen takes the blur it pushes colors from to be (Photoshop's Remove): a
/// Gaussian of the radius; a lens's (the same, without halos: no color pushed beyond those
/// around it); a motion along the angle, over twice the radius.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharpenRemove {
    GaussianBlur,
    LensBlur,
    MotionBlur,
}

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

/// Shadows/Highlights' radius range in pixels (Photoshop's): the size of the neighbourhood whose
/// tone decides how much a pixel is corrected, a Gaussian's standard deviation.
pub const MAX_SHADOWS_RADIUS: f32 = 2500.0;

/// Shadows/Highlights' Color and Midtone range (Photoshop's), in percent either way.
pub const MAX_SHADOWS_COLOR: f32 = 100.0;

/// How far Shadows/Highlights moves a neighbourhood at 100 %, in parts of its Tone: black lifted
/// to half the Tone (white lowered by as much).
pub const SHADOWS_LIFT: f64 = 0.5;

/// Unsharp Mask's amount range, in percent (Photoshop's).
pub const MIN_SHARPEN_AMOUNT: f32 = 1.0;
pub const MAX_SHARPEN_AMOUNT: f32 = 500.0;

/// Unsharp Mask's threshold range, in levels of 8 bits (Photoshop's).
pub const MAX_THRESHOLD: f32 = 255.0;

/// The colors a filter stores (Clouds', Fibers'): sRGB-encoded 8 bits a channel, packed
/// `0xRRGGBB` in one parameter, below this.
pub const PACKED_COLORS: u32 = 1 << 24;

/// The wavelength of Clouds' largest features, in the layer's pixels; each finer octave half
/// the one before, down to [`CLOUDS_FINEST`].
pub const CLOUDS_SCALE: f64 = 128.0;
const CLOUDS_FINEST: f64 = 2.0;

/// How much Clouds' and Fibers' noise, centered on 0.5, is stretched (their octaves averaged
/// would stay close to the middle).
const CLOUDS_CONTRAST: f64 = 1.8;
const FIBERS_CONTRAST: f64 = 2.2;

/// Fibers' Variance and Strength range (Photoshop's).
pub const MAX_FIBERS_SETTING: f32 = 64.0;

/// Fibers' finest streaks are half this many pixels long divided by Variance (Variance 16: 32
/// pixels).
pub const FIBERS_LENGTH: f64 = 1024.0;

/// Fibers' octaves: the finest 2 pixels across, each next one twice as wide and long.
const FIBERS_OCTAVES: u32 = 5;

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
    /// Photoshop's Maximum: each pixel the largest value of the square of `radius` pixels around
    /// it (Squareness), or of the disk (`round`, Roundness), channel by channel: light areas
    /// spread, dark ones shrink.
    Maximum { radius: f32, round: bool },
    /// Photoshop's Minimum: each pixel the smallest value of the square of `radius` pixels
    /// around it (Squareness), or of the disk (`round`), channel by channel: dark areas spread,
    /// light ones shrink.
    Minimum { radius: f32, round: bool },
    /// Photoshop's Custom: each channel the sum of the 5 × 5 pixels around it weighted by
    /// `weights` (row by row from the top left, whole numbers from -999 to 999), divided by
    /// `scale` (1 to 9999), plus `offset` levels of 8 bits (-9999 to 9999); each pixel keeps its
    /// opacity.
    Custom {
        weights: [f32; CUSTOM_WEIGHTS],
        scale: f32,
        offset: f32,
    },
    /// Photoshop's HSB/HSL: each pixel's three channels read as `input` says and written as
    /// `order` says (RGB to HSB: the hue in red, the saturation in green, the brightness in
    /// blue), its opacity kept.
    HsbHsl {
        input: ColorModel,
        order: ColorModel,
    },
    /// Photoshop's Despeckle: the layer smoothed except where it changes much (its edges).
    /// `reduction` as Diffuse's: the radius shrinks by it.
    Despeckle { reduction: f32 },
    /// Photoshop's Reduce Noise (Basic): the lightness smoothed where neighbours differ little,
    /// `strength` (0 to 10) how much, less with `preserve` (Preserve Details, percent); the
    /// colors (without the lightness) likewise by `color` (Reduce Color Noise, percent); the
    /// lightness then sharpened by `sharpen` percent (Sharpen Details). `reduction` as
    /// Diffuse's: the radii shrink by it.
    ReduceNoise {
        strength: f32,
        preserve: f32,
        color: f32,
        sharpen: f32,
        reduction: f32,
    },
    /// Photoshop's Smart Sharpen: each color pushed away from the blur `remove` says (of
    /// `radius` pixels, 0.1 to 64, along `angle` degrees for a motion) by `amount` percent (1 to
    /// 500) of their difference, small differences (grain) less with `noise` (Reduce Noise,
    /// percent).
    SmartSharpen {
        amount: f32,
        radius: f32,
        noise: f32,
        remove: SharpenRemove,
        angle: f32,
    },
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
    /// Photoshop's Color Halftone: each channel a screen of dots, a dot to a square cell of
    /// `√2 radius` pixels (a grid from the layer's origin turned by that channel's `angles`
    /// degrees, red, green, blue), the channel full in the dots, none outside. A dot's size
    /// grows with the channel's average around its center: it covers as much of its cell as
    /// the channel's value (the tone is kept), all of it at full, where its radius is `radius`
    /// (4 to 127). Each pixel keeps its opacity.
    ColorHalftone { radius: f32, angles: [f32; 3] },
    /// Photoshop's Crystallize: polygons, each the cell of a point drawn from `seed` in each
    /// square of `cell` pixels (3 to 300) from the layer's origin (the pixels closer to it than
    /// to any other), the color of the layer at that point: sampled as the distortions are.
    Crystallize { cell: f32, seed: u32 },
    /// Photoshop's Facet: like-colored pixels clumped into flat blocks (a Kuwahara filter of
    /// [`FACET_RADIUS`]). `reduction` as Diffuse's: the radius shrinks by it.
    Facet { reduction: f32 },
    /// Photoshop's Fragment: four copies of the layer, [`FRAGMENT_OFFSET`] pixels from it
    /// diagonally, averaged. `reduction` as Diffuse's: the offset shrinks by it.
    Fragment { reduction: f32 },
    /// Photoshop's Mezzotint: each channel full or none, against a random pattern of `kind`
    /// drawn from `seed` and the layer's pixel (the channel's value its odds of being full),
    /// each pixel keeping its opacity. `reduction` as Diffuse's: a pixel of a reduced level
    /// shows the share of the layer's pixels under it that are full.
    Mezzotint {
        kind: MezzotintType,
        seed: u32,
        reduction: f32,
    },
    /// Photoshop's Pointillize: dots, one drawn from `seed` in each square of `cell` pixels (3
    /// to 300) from the layer's origin, of a radius of half a cell to 0.69 of one, each the
    /// color of the layer at its center, later ones (row by row) over earlier ones;
    /// `background` (sRGB-encoded, the background color of when it was applied) between them.
    /// Sampled as the distortions are.
    Pointillize {
        cell: f32,
        background: [u8; 3],
        seed: u32,
    },
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
    /// Photoshop's Spherize: as if wrapped around a sphere filling the frame's inscribed
    /// ellipse, bulging (a positive `amount`, percent) or hollow (negative); `mode` a cylinder
    /// instead, across the width or the height only.
    Spherize { amount: f32, mode: SpherizeMode },
    /// Photoshop's Ripple: each pixel moved along a wave across the other axis, `amount` percent
    /// (either way) of [`RIPPLE_STRENGTH`] of the wavelength of `size`.
    Ripple { amount: f32, size: RippleSize },
    /// Photoshop's Wave: each pixel moved by the average of `generators` waves of `kind`, each of
    /// a wavelength and an amplitude (pixels) drawn from `seed` within `wavelength` and
    /// `amplitude` (min and max, either order), horizontally across the height and vertically
    /// across the width, by `scale` percent (horizontal, vertical); `wrap`: what goes beyond the
    /// frame comes back on the other side (else the edges repeat).
    Wave {
        generators: f32,
        wavelength: [f32; 2],
        amplitude: [f32; 2],
        scale: [f32; 2],
        kind: WaveType,
        wrap: bool,
        seed: u32,
    },
    /// Photoshop's ZigZag: `ridges` circular ridges within the frame's inscribed circle, fading
    /// out to it, moving pixels as `style` says by `amount` percent (either way).
    ZigZag {
        amount: f32,
        ridges: f32,
        style: ZigZagStyle,
    },
    /// Photoshop's Shear: each row of the frame moved sideways as `curve` says; `wrap`: what
    /// goes beyond the frame comes back on the other side (Wrap Around), else its edges repeat
    /// (Repeat Edge Pixels).
    Shear { curve: ShearCurve, wrap: bool },
    /// Photoshop's Displace: each pixel read from as far as its displacement map says (the
    /// step's `map`, see [`Self::displaced`]): its red channel across, its green one down (a
    /// gray map both), 128 of 255 nothing, by `scale` percent (horizontal, vertical, -999 to
    /// 999) of a pixel per level; the map stretched over the frame or `tile`d from its corner
    /// at its own size; `wrap`: what goes beyond the frame comes back on the other side (Wrap
    /// Around), else its edges repeat (Repeat Edge Pixels).
    Displace {
        scale: [f32; 2],
        tile: bool,
        wrap: bool,
    },
    /// Photoshop's Radial Blur: each pixel the average of the layer along the arc through it
    /// around `center` (Spin, `amount` degrees) or along the line to it (`zoom`, see
    /// [`ZOOM_SPAN`]), centered on the pixel; `center` in percent of the frame (the selection's
    /// box or the layer), `quality` how many samples.
    RadialBlur {
        amount: f32,
        zoom: bool,
        quality: RadialQuality,
        center: [f32; 2],
    },
    /// Photoshop's Surface Blur: each pixel the average of the square of `radius` pixels around
    /// it, channel by channel, each value weighted by how close it is to the pixel's (nothing
    /// from `2.5 × threshold` levels apart): flat areas smoothed, edges kept.
    SurfaceBlur { radius: f32, threshold: f32 },
    /// Photoshop's Smart Blur: each pixel the average of the pixels of the disk of `radius`
    /// around it that differ from it by less than `threshold` levels on every channel; `mode`
    /// shows that, or the edges (pixels a neighbor differs from by `threshold` or more) white on
    /// black, or white over the blur. `quality`: how many pixels of the disk it reads.
    SmartBlur {
        radius: f32,
        threshold: f32,
        quality: SmartQuality,
        mode: SmartMode,
    },
    /// Photoshop's Average: the layer (the selection when there is one) filled with its average
    /// color, each pixel keeping its own opacity.
    Average,
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
    /// Photoshop's Shadows/Highlights (Image > Adjustments): each pixel lightened by up to
    /// `shadows[0]` percent where its neighbourhood (a blur of `shadows[2]` pixels) is among the
    /// darkest `shadows[1]` percent of tones, darkened likewise by `highlights` where it is among
    /// the lightest; the pixel's detail kept (the correction moves it with its neighbourhood).
    /// `color` changes the saturation of what was corrected, `midtone` the contrast of the
    /// midtones (both -100 to 100). `scale`: as Clarity's, the radii shrink by it.
    ShadowsHighlights {
        shadows: [f32; 3],
        highlights: [f32; 3],
        color: f32,
        midtone: f32,
        scale: f32,
    },
    /// Photoshop's Clouds: the layer (the selection) replaced by soft clouds between
    /// `foreground` (where the noise is 0) and `background` (1), opaque: fractal noise of the
    /// layer's pixel and `seed` (see [`clouds`]). The colors are those of when it was applied,
    /// sRGB-encoded.
    Clouds {
        foreground: [u8; 3],
        background: [u8; 3],
        seed: u32,
    },
    /// Photoshop's Difference Clouds: Clouds' colors blended with the layer in Difference mode
    /// (each channel the difference), each pixel keeping its opacity.
    DifferenceClouds {
        foreground: [u8; 3],
        background: [u8; 3],
        seed: u32,
    },
    /// Photoshop's Fibers: the layer replaced by vertical fibers between `foreground` and
    /// `background`, opaque: `variance` (1 to 64) the shorter the streaks the higher, `strength`
    /// (1 to 64) a loose weave when low, short stringy fibers when high (see [`fibers`]).
    Fibers {
        variance: f32,
        strength: f32,
        foreground: [u8; 3],
        background: [u8; 3],
        seed: u32,
    },
    /// Photoshop's Diffuse: each pixel replaced by one of its eight neighbours drawn from `seed`
    /// and its place, as `mode` says. `reduction`: how many of the layer's pixels a pixel it is
    /// computed on stands for (1, more for a look; not a setting): beyond 1.5 nothing moves, a
    /// neighbour being farther than Diffuse reaches.
    Diffuse {
        mode: DiffuseMode,
        seed: u32,
        reduction: f32,
    },
    /// Photoshop's Wind: streaks blown along the rows from the layer's edges (neighbours of
    /// different lightness), from the right or `from_left`, their lengths drawn from `seed`
    /// (see [`WindMethod`]); `reduction` as Diffuse's, the streaks shortened by it.
    Wind {
        method: WindMethod,
        from_left: bool,
        seed: u32,
        reduction: f32,
    },
    /// Photoshop's Tiles: the frame (the selection's box or the layer) cut into square tiles,
    /// `tiles` of them (1 to 99) along its shorter side, each moved by up to `offset` percent of
    /// its side in a direction drawn from `seed`, later tiles over earlier ones (row by row);
    /// the gaps filled as `fill` says (the colors of when it was applied, sRGB-encoded).
    Tiles {
        tiles: f32,
        offset: f32,
        fill: TilesFill,
        foreground: [u8; 3],
        background: [u8; 3],
        seed: u32,
    },
    /// Photoshop's Trace Contour: thin lines where each channel crosses `level` (0 to 255) on
    /// white, drawn on the pixels above it (`upper`) or below it, each pixel keeping its opacity.
    TraceContour { level: f32, upper: bool },
    /// Photoshop's Oil Paint: the layer smoothed into strokes (a Kuwahara filter of a radius
    /// growing with `stylization` and `scale`), cleaned by a blur growing with `cleanliness`,
    /// then, with `lighting`, its relief (its lightness, `bristle` adding the brush's grain) lit
    /// from `angle` degrees, `shine` and `scale` how strongly. `reduction` as Diffuse's: the
    /// radii shrink by it.
    OilPaint {
        stylization: f32,
        cleanliness: f32,
        scale: f32,
        bristle: f32,
        lighting: bool,
        angle: f32,
        shine: f32,
        reduction: f32,
    },
}

impl Filter {
    /// Every filter's identifier, in menu order.
    /// Shadows/Highlights last: it is in Image > Adjustments, not in the Filter menu.
    pub const IDS: [&'static str; 49] = [
        "gaussianBlur",
        "motionBlur",
        "boxBlur",
        "radialBlur",
        "smartBlur",
        "surfaceBlur",
        "average",
        "smartSharpen",
        "unsharpMask",
        "addNoise",
        "despeckle",
        "dustAndScratches",
        "median",
        "reduceNoise",
        "clarityTexture",
        "custom",
        "highPass",
        "hsbHsl",
        "maximum",
        "minimum",
        "findEdges",
        "emboss",
        "solarize",
        "colorHalftone",
        "crystallize",
        "facet",
        "fragment",
        "mezzotint",
        "mosaic",
        "pointillize",
        "offset",
        "twirl",
        "pinch",
        "spherize",
        "polarCoordinates",
        "ripple",
        "wave",
        "zigZag",
        "shear",
        "displace",
        "clouds",
        "differenceClouds",
        "fibers",
        "diffuse",
        "oilPaint",
        "tiles",
        "traceContour",
        "wind",
        "shadowsHighlights",
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
            Self::Custom { .. } => "custom",
            Self::HsbHsl { .. } => "hsbHsl",
            Self::Despeckle { .. } => "despeckle",
            Self::ReduceNoise { .. } => "reduceNoise",
            Self::SmartSharpen { .. } => "smartSharpen",
            Self::Solarize => "solarize",
            Self::FindEdges => "findEdges",
            Self::Emboss { .. } => "emboss",
            Self::Mosaic { .. } => "mosaic",
            Self::ColorHalftone { .. } => "colorHalftone",
            Self::Crystallize { .. } => "crystallize",
            Self::Facet { .. } => "facet",
            Self::Fragment { .. } => "fragment",
            Self::Mezzotint { .. } => "mezzotint",
            Self::Pointillize { .. } => "pointillize",
            Self::Offset { .. } => "offset",
            Self::Twirl { .. } => "twirl",
            Self::Pinch { .. } => "pinch",
            Self::Spherize { .. } => "spherize",
            Self::PolarCoordinates { .. } => "polarCoordinates",
            Self::Ripple { .. } => "ripple",
            Self::Wave { .. } => "wave",
            Self::ZigZag { .. } => "zigZag",
            Self::Shear { .. } => "shear",
            Self::Displace { .. } => "displace",
            Self::RadialBlur { .. } => "radialBlur",
            Self::SurfaceBlur { .. } => "surfaceBlur",
            Self::SmartBlur { .. } => "smartBlur",
            Self::Average => "average",
            Self::ShadowsHighlights { .. } => "shadowsHighlights",
            Self::Clouds { .. } => "clouds",
            Self::DifferenceClouds { .. } => "differenceClouds",
            Self::Fibers { .. } => "fibers",
            Self::Diffuse { .. } => "diffuse",
            Self::Wind { .. } => "wind",
            Self::Tiles { .. } => "tiles",
            Self::TraceContour { .. } => "traceContour",
            Self::OilPaint { .. } => "oilPaint",
        }
    }

    /// Its parameters, in a fixed order per filter.
    pub fn params(&self) -> Vec<f32> {
        match *self {
            Self::GaussianBlur { radius }
            | Self::HighPass { radius }
            | Self::Median { radius }
            | Self::BoxBlur { radius } => vec![radius],
            Self::Maximum { radius, round } | Self::Minimum { radius, round } => {
                vec![radius, f32::from(u8::from(round))]
            }
            Self::Custom {
                weights,
                scale,
                offset,
            } => [weights.as_slice(), &[scale, offset]].concat(),
            Self::HsbHsl { input, order } => vec![input.index(), order.index()],
            Self::Despeckle { .. } => Vec::new(),
            Self::ReduceNoise {
                strength,
                preserve,
                color,
                sharpen,
                ..
            } => vec![strength, preserve, color, sharpen],
            Self::SmartSharpen {
                amount,
                radius,
                noise,
                remove,
                angle,
            } => vec![
                amount,
                radius,
                noise,
                match remove {
                    SharpenRemove::GaussianBlur => 0.0,
                    SharpenRemove::LensBlur => 1.0,
                    SharpenRemove::MotionBlur => 2.0,
                },
                angle,
            ],
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
            Self::ColorHalftone { radius, angles } => [[radius].as_slice(), &angles].concat(),
            Self::Crystallize { cell, seed } => vec![cell, seed as f32],
            Self::Facet { .. } | Self::Fragment { .. } => Vec::new(),
            Self::Mezzotint { kind, seed, .. } => vec![kind.index() as f32, seed as f32],
            Self::Pointillize {
                cell,
                background,
                seed,
            } => vec![cell, packed(background), seed as f32],
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
            Self::Pinch { amount } => vec![amount],
            Self::Spherize { amount, mode } => vec![
                amount,
                match mode {
                    SpherizeMode::Normal => 0.0,
                    SpherizeMode::HorizontalOnly => 1.0,
                    SpherizeMode::VerticalOnly => 2.0,
                },
            ],
            Self::Ripple { amount, size } => vec![
                amount,
                match size {
                    RippleSize::Small => 0.0,
                    RippleSize::Medium => 1.0,
                    RippleSize::Large => 2.0,
                },
            ],
            Self::Wave {
                generators,
                wavelength,
                amplitude,
                scale,
                kind,
                wrap,
                seed,
            } => vec![
                generators,
                wavelength[0],
                wavelength[1],
                amplitude[0],
                amplitude[1],
                scale[0],
                scale[1],
                match kind {
                    WaveType::Sine => 0.0,
                    WaveType::Triangle => 1.0,
                    WaveType::Square => 2.0,
                },
                if wrap { 0.0 } else { 1.0 },
                seed as f32,
            ],
            Self::ZigZag {
                amount,
                ridges,
                style,
            } => vec![
                amount,
                ridges,
                match style {
                    ZigZagStyle::AroundCenter => 0.0,
                    ZigZagStyle::OutFromCenter => 1.0,
                    ZigZagStyle::PondRipples => 2.0,
                },
            ],
            // Variable: Undefined Areas, then each point's row and offset.
            Self::Shear { curve, wrap } => std::iter::once(if wrap { 0.0 } else { 1.0 })
                .chain(curve.points().iter().flatten().copied())
                .collect(),
            Self::Displace { scale, tile, wrap } => vec![
                scale[0],
                scale[1],
                f32::from(u8::from(tile)),
                if wrap { 0.0 } else { 1.0 },
            ],
            Self::PolarCoordinates { to_polar } => vec![f32::from(u8::from(to_polar))],
            Self::RadialBlur {
                amount,
                zoom,
                quality,
                center,
            } => vec![
                amount,
                f32::from(u8::from(zoom)),
                match quality {
                    RadialQuality::Draft => 0.0,
                    RadialQuality::Good => 1.0,
                    RadialQuality::Best => 2.0,
                },
                center[0],
                center[1],
            ],
            Self::SurfaceBlur { radius, threshold } => vec![radius, threshold],
            Self::SmartBlur {
                radius,
                threshold,
                quality,
                mode,
            } => vec![
                radius,
                threshold,
                match quality {
                    SmartQuality::Low => 0.0,
                    SmartQuality::Medium => 1.0,
                    SmartQuality::High => 2.0,
                },
                match mode {
                    SmartMode::Normal => 0.0,
                    SmartMode::EdgeOnly => 1.0,
                    SmartMode::OverlayEdge => 2.0,
                },
            ],
            Self::Average => Vec::new(),
            Self::ShadowsHighlights {
                shadows,
                highlights,
                color,
                midtone,
                ..
            } => [shadows.as_slice(), &highlights, &[color, midtone]].concat(),
            Self::Clouds {
                foreground,
                background,
                seed,
            }
            | Self::DifferenceClouds {
                foreground,
                background,
                seed,
            } => vec![packed(foreground), packed(background), seed as f32],
            Self::Fibers {
                variance,
                strength,
                foreground,
                background,
                seed,
            } => vec![
                variance,
                strength,
                packed(foreground),
                packed(background),
                seed as f32,
            ],
            Self::Diffuse { mode, seed, .. } => vec![
                match mode {
                    DiffuseMode::Normal => 0.0,
                    DiffuseMode::DarkenOnly => 1.0,
                    DiffuseMode::LightenOnly => 2.0,
                    DiffuseMode::Anisotropic => 3.0,
                },
                seed as f32,
            ],
            Self::Wind {
                method,
                from_left,
                seed,
                ..
            } => vec![
                match method {
                    WindMethod::Wind => 0.0,
                    WindMethod::Blast => 1.0,
                    WindMethod::Stagger => 2.0,
                },
                f32::from(u8::from(from_left)),
                seed as f32,
            ],
            Self::Tiles {
                tiles,
                offset,
                fill,
                foreground,
                background,
                seed,
            } => vec![
                tiles,
                offset,
                match fill {
                    TilesFill::Background => 0.0,
                    TilesFill::Foreground => 1.0,
                    TilesFill::Inverse => 2.0,
                    TilesFill::Unaltered => 3.0,
                },
                packed(foreground),
                packed(background),
                seed as f32,
            ],
            Self::TraceContour { level, upper } => vec![level, f32::from(u8::from(upper))],
            Self::OilPaint {
                stylization,
                cleanliness,
                scale,
                bristle,
                lighting,
                angle,
                shine,
                ..
            } => vec![
                stylization,
                cleanliness,
                scale,
                bristle,
                f32::from(u8::from(lighting)),
                angle,
                shine,
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
            ("median", &[radius]) => Some(Self::Median { radius }),
            ("boxBlur", &[radius]) => Some(Self::BoxBlur { radius }),
            // Without Preserve (written before it existed): Squareness.
            ("maximum", &[radius]) => Some(Self::Maximum {
                radius,
                round: false,
            }),
            ("minimum", &[radius]) => Some(Self::Minimum {
                radius,
                round: false,
            }),
            ("maximum", &[radius, round]) => Some(Self::Maximum {
                radius,
                round: flag(round)?,
            }),
            ("minimum", &[radius, round]) => Some(Self::Minimum {
                radius,
                round: flag(round)?,
            }),
            ("custom", values) if values.len() == CUSTOM_WEIGHTS + 2 => Some(Self::Custom {
                weights: std::array::from_fn(|i| values[i]),
                scale: values[CUSTOM_WEIGHTS],
                offset: values[CUSTOM_WEIGHTS + 1],
            }),
            ("hsbHsl", &[input, order]) => Some(Self::HsbHsl {
                input: ColorModel::of(input)?,
                order: ColorModel::of(order)?,
            }),
            ("despeckle", &[]) => Some(Self::Despeckle { reduction: 1.0 }),
            ("reduceNoise", &[strength, preserve, color, sharpen]) => Some(Self::ReduceNoise {
                strength,
                preserve,
                color,
                sharpen,
                reduction: 1.0,
            }),
            ("smartSharpen", &[amount, radius, noise, remove, angle]) => Some(Self::SmartSharpen {
                amount,
                radius,
                noise,
                remove: match remove {
                    0.0 => SharpenRemove::GaussianBlur,
                    1.0 => SharpenRemove::LensBlur,
                    2.0 => SharpenRemove::MotionBlur,
                    _ => return None,
                },
                angle,
            }),
            ("solarize", &[]) => Some(Self::Solarize),
            ("findEdges", &[]) => Some(Self::FindEdges),
            ("emboss", &[angle, height, amount]) => Some(Self::Emboss {
                angle,
                height,
                amount,
            }),
            ("mosaic", &[cell]) => Some(Self::Mosaic { cell }),
            ("colorHalftone", &[radius, a1, a2, a3]) => Some(Self::ColorHalftone {
                radius,
                angles: [a1, a2, a3],
            }),
            ("crystallize", &[cell, seed]) => Some(Self::Crystallize {
                cell,
                seed: whole_seed(seed)?,
            }),
            ("facet", &[]) => Some(Self::Facet { reduction: 1.0 }),
            ("fragment", &[]) => Some(Self::Fragment { reduction: 1.0 }),
            ("mezzotint", &[kind, seed]) => Some(Self::Mezzotint {
                kind: MezzotintType::of(kind)?,
                seed: whole_seed(seed)?,
                reduction: 1.0,
            }),
            ("pointillize", &[cell, background, seed]) => Some(Self::Pointillize {
                cell,
                background: unpacked(background)?,
                seed: whole_seed(seed)?,
            }),
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
            // Without a mode (written before it existed): Normal.
            ("spherize", &[amount]) => Some(Self::Spherize {
                amount,
                mode: SpherizeMode::Normal,
            }),
            ("spherize", &[amount, mode]) => Some(Self::Spherize {
                amount,
                mode: match mode {
                    0.0 => SpherizeMode::Normal,
                    1.0 => SpherizeMode::HorizontalOnly,
                    2.0 => SpherizeMode::VerticalOnly,
                    _ => return None,
                },
            }),
            ("ripple", &[amount, size]) => Some(Self::Ripple {
                amount,
                size: match size {
                    0.0 => RippleSize::Small,
                    1.0 => RippleSize::Medium,
                    2.0 => RippleSize::Large,
                    _ => return None,
                },
            }),
            ("wave", &[generators, w0, w1, a0, a1, sx, sy, kind, edge, seed]) => {
                let whole = seed.fract() == 0.0 && (0.0..NOISE_SEEDS as f32).contains(&seed);
                Some(Self::Wave {
                    generators,
                    wavelength: [w0, w1],
                    amplitude: [a0, a1],
                    scale: [sx, sy],
                    kind: match kind {
                        0.0 => WaveType::Sine,
                        1.0 => WaveType::Triangle,
                        2.0 => WaveType::Square,
                        _ => return None,
                    },
                    wrap: match edge {
                        0.0 => true,
                        1.0 => false,
                        _ => return None,
                    },
                    seed: whole.then_some(seed as u32)?,
                })
            }
            ("zigZag", &[amount, ridges, style]) => Some(Self::ZigZag {
                amount,
                ridges,
                style: match style {
                    0.0 => ZigZagStyle::AroundCenter,
                    1.0 => ZigZagStyle::OutFromCenter,
                    2.0 => ZigZagStyle::PondRipples,
                    _ => return None,
                },
            }),
            ("shear", &[edge, ref points @ ..]) if points.len() % 2 == 0 => {
                let points: Vec<[f32; 2]> = points.chunks(2).map(|p| [p[0], p[1]]).collect();
                Some(Self::Shear {
                    curve: ShearCurve::new(&points)?,
                    wrap: match edge {
                        0.0 => true,
                        1.0 => false,
                        _ => return None,
                    },
                })
            }
            ("displace", &[h, v, tile, edge]) => Some(Self::Displace {
                scale: [h, v],
                tile: flag(tile)?,
                wrap: match edge {
                    0.0 => true,
                    1.0 => false,
                    _ => return None,
                },
            }),
            ("polarCoordinates", &[to_polar]) => match to_polar {
                0.0 => Some(Self::PolarCoordinates { to_polar: false }),
                1.0 => Some(Self::PolarCoordinates { to_polar: true }),
                _ => None,
            },
            ("radialBlur", &[amount, method, quality, x, y]) => Some(Self::RadialBlur {
                amount,
                zoom: match method {
                    0.0 => false,
                    1.0 => true,
                    _ => return None,
                },
                quality: match quality {
                    0.0 => RadialQuality::Draft,
                    1.0 => RadialQuality::Good,
                    2.0 => RadialQuality::Best,
                    _ => return None,
                },
                center: [x, y],
            }),
            ("surfaceBlur", &[radius, threshold]) => Some(Self::SurfaceBlur { radius, threshold }),
            ("smartBlur", &[radius, threshold, quality, mode]) => Some(Self::SmartBlur {
                radius,
                threshold,
                quality: match quality {
                    0.0 => SmartQuality::Low,
                    1.0 => SmartQuality::Medium,
                    2.0 => SmartQuality::High,
                    _ => return None,
                },
                mode: match mode {
                    0.0 => SmartMode::Normal,
                    1.0 => SmartMode::EdgeOnly,
                    2.0 => SmartMode::OverlayEdge,
                    _ => return None,
                },
            }),
            ("average", &[]) => Some(Self::Average),
            ("clouds", &[foreground, background, seed]) => Some(Self::Clouds {
                foreground: unpacked(foreground)?,
                background: unpacked(background)?,
                seed: whole_seed(seed)?,
            }),
            ("differenceClouds", &[foreground, background, seed]) => Some(Self::DifferenceClouds {
                foreground: unpacked(foreground)?,
                background: unpacked(background)?,
                seed: whole_seed(seed)?,
            }),
            ("fibers", &[variance, strength, foreground, background, seed]) => Some(Self::Fibers {
                variance,
                strength,
                foreground: unpacked(foreground)?,
                background: unpacked(background)?,
                seed: whole_seed(seed)?,
            }),
            ("diffuse", &[mode, seed]) => Some(Self::Diffuse {
                mode: match mode {
                    0.0 => DiffuseMode::Normal,
                    1.0 => DiffuseMode::DarkenOnly,
                    2.0 => DiffuseMode::LightenOnly,
                    3.0 => DiffuseMode::Anisotropic,
                    _ => return None,
                },
                seed: whole_seed(seed)?,
                reduction: 1.0,
            }),
            ("wind", &[method, direction, seed]) => Some(Self::Wind {
                method: match method {
                    0.0 => WindMethod::Wind,
                    1.0 => WindMethod::Blast,
                    2.0 => WindMethod::Stagger,
                    _ => return None,
                },
                from_left: flag(direction)?,
                seed: whole_seed(seed)?,
                reduction: 1.0,
            }),
            ("tiles", &[tiles, offset, fill, foreground, background, seed]) => Some(Self::Tiles {
                tiles,
                offset,
                fill: match fill {
                    0.0 => TilesFill::Background,
                    1.0 => TilesFill::Foreground,
                    2.0 => TilesFill::Inverse,
                    3.0 => TilesFill::Unaltered,
                    _ => return None,
                },
                foreground: unpacked(foreground)?,
                background: unpacked(background)?,
                seed: whole_seed(seed)?,
            }),
            ("traceContour", &[level, edge]) => Some(Self::TraceContour {
                level,
                upper: flag(edge)?,
            }),
            (
                "oilPaint",
                &[
                    stylization,
                    cleanliness,
                    scale,
                    bristle,
                    lighting,
                    angle,
                    shine,
                ],
            ) => Some(Self::OilPaint {
                stylization,
                cleanliness,
                scale,
                bristle,
                lighting: flag(lighting)?,
                angle,
                shine,
                reduction: 1.0,
            }),
            ("shadowsHighlights", &[sa, st, sr, ha, ht, hr, color, midtone]) => {
                Some(Self::ShadowsHighlights {
                    shadows: [sa, st, sr],
                    highlights: [ha, ht, hr],
                    color,
                    midtone,
                    scale: 1.0,
                })
            }
            ("dustAndScratches", &[radius, threshold]) => {
                Some(Self::DustAndScratches { radius, threshold })
            }
            ("clarityTexture", &[texture, clarity]) => Some(Self::ClarityTexture {
                texture,
                clarity,
                scale: 1.0,
            }),
            // Flags are 0 or 1, the seed a whole number in range: anything else is not this
            // filter's.
            ("addNoise", &[amount, gaussian, monochromatic, seed]) => Some(Self::AddNoise {
                amount,
                gaussian: flag(gaussian)?,
                monochromatic: flag(monochromatic)?,
                seed: whole_seed(seed)?,
            }),
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
            "maximum" => Some(Self::Maximum {
                radius: 1.0,
                round: false,
            }),
            "minimum" => Some(Self::Minimum {
                radius: 1.0,
                round: false,
            }),
            // Photoshop's: a sharpening kernel, the center 5 and its four neighbours -1.
            "custom" => Some(Self::Custom {
                weights: std::array::from_fn(|i| match i {
                    12 => 5.0,
                    7 | 11 | 13 | 17 => -1.0,
                    _ => 0.0,
                }),
                scale: 1.0,
                offset: 0.0,
            }),
            "hsbHsl" => Some(Self::HsbHsl {
                input: ColorModel::Rgb,
                order: ColorModel::Hsb,
            }),
            "despeckle" => Some(Self::Despeckle { reduction: 1.0 }),
            "reduceNoise" => Some(Self::ReduceNoise {
                strength: 6.0,
                preserve: 60.0,
                color: 45.0,
                sharpen: 25.0,
                reduction: 1.0,
            }),
            "smartSharpen" => Some(Self::SmartSharpen {
                amount: 200.0,
                radius: 1.0,
                noise: 10.0,
                remove: SharpenRemove::LensBlur,
                angle: 0.0,
            }),
            "solarize" => Some(Self::Solarize),
            "findEdges" => Some(Self::FindEdges),
            "emboss" => Some(Self::Emboss {
                angle: 135.0,
                height: 3.0,
                amount: 100.0,
            }),
            "mosaic" => Some(Self::Mosaic { cell: 10.0 }),
            "colorHalftone" => Some(Self::ColorHalftone {
                radius: 8.0,
                angles: [108.0, 162.0, 90.0],
            }),
            "crystallize" => Some(Self::Crystallize {
                cell: 10.0,
                seed: 0,
            }),
            "facet" => Some(Self::Facet { reduction: 1.0 }),
            "fragment" => Some(Self::Fragment { reduction: 1.0 }),
            "mezzotint" => Some(Self::Mezzotint {
                kind: MezzotintType::FineDots,
                seed: 0,
                reduction: 1.0,
            }),
            // Photoshop's default background color, white.
            "pointillize" => Some(Self::Pointillize {
                cell: 5.0,
                background: [255; 3],
                seed: 0,
            }),
            "offset" => Some(Self::Offset {
                horizontal: 0.0,
                vertical: 0.0,
                edge: OffsetEdge::Wrap,
            }),
            "twirl" => Some(Self::Twirl { angle: 50.0 }),
            "pinch" => Some(Self::Pinch { amount: 50.0 }),
            "spherize" => Some(Self::Spherize {
                amount: 100.0,
                mode: SpherizeMode::Normal,
            }),
            "ripple" => Some(Self::Ripple {
                amount: 100.0,
                size: RippleSize::Medium,
            }),
            "wave" => Some(Self::Wave {
                generators: 5.0,
                wavelength: [10.0, 120.0],
                amplitude: [5.0, 35.0],
                scale: [100.0, 100.0],
                kind: WaveType::Sine,
                wrap: false,
                seed: 0,
            }),
            "zigZag" => Some(Self::ZigZag {
                amount: 10.0,
                ridges: 5.0,
                style: ZigZagStyle::PondRipples,
            }),
            "shear" => Some(Self::Shear {
                curve: ShearCurve::STRAIGHT,
                wrap: true,
            }),
            // Photoshop's: 10 %, Stretch to Fit, Repeat Edge Pixels.
            "displace" => Some(Self::Displace {
                scale: [10.0, 10.0],
                tile: false,
                wrap: false,
            }),
            "polarCoordinates" => Some(Self::PolarCoordinates { to_polar: true }),
            "radialBlur" => Some(Self::RadialBlur {
                amount: 10.0,
                zoom: false,
                quality: RadialQuality::Good,
                center: [50.0, 50.0],
            }),
            "surfaceBlur" => Some(Self::SurfaceBlur {
                radius: 5.0,
                threshold: 15.0,
            }),
            "smartBlur" => Some(Self::SmartBlur {
                radius: 3.0,
                threshold: 25.0,
                quality: SmartQuality::Low,
                mode: SmartMode::Normal,
            }),
            "average" => Some(Self::Average),
            // Photoshop's default colors.
            "clouds" => Some(Self::Clouds {
                foreground: [0; 3],
                background: [255; 3],
                seed: 0,
            }),
            "differenceClouds" => Some(Self::DifferenceClouds {
                foreground: [0; 3],
                background: [255; 3],
                seed: 0,
            }),
            "fibers" => Some(Self::Fibers {
                variance: 16.0,
                strength: 4.0,
                foreground: [0; 3],
                background: [255; 3],
                seed: 0,
            }),
            "diffuse" => Some(Self::Diffuse {
                mode: DiffuseMode::Normal,
                seed: 0,
                reduction: 1.0,
            }),
            "wind" => Some(Self::Wind {
                method: WindMethod::Wind,
                from_left: false,
                seed: 0,
                reduction: 1.0,
            }),
            "tiles" => Some(Self::Tiles {
                tiles: 10.0,
                offset: 10.0,
                fill: TilesFill::Background,
                foreground: [0; 3],
                background: [255; 3],
                seed: 0,
            }),
            "traceContour" => Some(Self::TraceContour {
                level: 128.0,
                upper: true,
            }),
            "oilPaint" => Some(Self::OilPaint {
                stylization: 4.0,
                cleanliness: 5.0,
                scale: 1.0,
                bristle: 2.0,
                lighting: true,
                angle: -60.0,
                shine: 1.0,
                reduction: 1.0,
            }),
            "shadowsHighlights" => Some(Self::ShadowsHighlights {
                shadows: [35.0, 50.0, 30.0],
                highlights: [0.0, 50.0, 30.0],
                color: 20.0,
                midtone: 0.0,
                scale: 1.0,
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
            Self::ClarityTexture {
                texture,
                clarity,
                scale,
            } => {
                let amount = |v: f32| v.is_finite() && (-MAX_CLARITY..=MAX_CLARITY).contains(&v);
                amount(texture) && amount(clarity) && scale.is_finite() && scale >= 1.0
            }
            Self::ShadowsHighlights {
                shadows,
                highlights,
                color,
                midtone,
                scale,
            } => {
                let percent = |v: f32| v.is_finite() && (0.0..=100.0).contains(&v);
                let side = |[amount, tone, radius]: [f32; 3]| {
                    percent(amount)
                        && percent(tone)
                        && radius.fract() == 0.0
                        && (0.0..=MAX_SHADOWS_RADIUS).contains(&radius)
                };
                let either = |v: f32| v.fract() == 0.0 && v.abs() <= MAX_SHADOWS_COLOR;
                side(shadows)
                    && side(highlights)
                    && either(color)
                    && either(midtone)
                    && scale.is_finite()
                    && scale >= 1.0
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
            Self::Maximum { radius, .. } | Self::Minimum { radius, .. } => {
                radius.fract() == 0.0 && (MIN_EXTREME_RADIUS..=MAX_EXTREME_RADIUS).contains(&radius)
            }
            Self::Custom {
                weights,
                scale,
                offset,
            } => {
                let whole =
                    |v: f32, min: f32, max: f32| v.fract() == 0.0 && (min..=max).contains(&v);
                weights
                    .iter()
                    .all(|&w| whole(w, -MAX_CUSTOM_WEIGHT, MAX_CUSTOM_WEIGHT))
                    && whole(scale, 1.0, MAX_CUSTOM_SCALE)
                    && whole(offset, -MAX_CUSTOM_OFFSET, MAX_CUSTOM_OFFSET)
            }
            Self::HsbHsl { .. } => true,
            Self::Despeckle { reduction } => reduction.is_finite() && reduction >= 1.0,
            Self::ReduceNoise {
                strength,
                preserve,
                color,
                sharpen,
                reduction,
            } => {
                let percent = |v: f32| v.fract() == 0.0 && (0.0..=100.0).contains(&v);
                strength.fract() == 0.0
                    && (0.0..=MAX_NOISE_STRENGTH).contains(&strength)
                    && percent(preserve)
                    && percent(color)
                    && percent(sharpen)
                    && reduction.is_finite()
                    && reduction >= 1.0
            }
            Self::SmartSharpen {
                amount,
                radius,
                noise,
                angle,
                ..
            } => {
                amount.fract() == 0.0
                    && (MIN_SHARPEN_AMOUNT..=MAX_SHARPEN_AMOUNT).contains(&amount)
                    && radius.is_finite()
                    && (MIN_SMART_SHARPEN_RADIUS..=MAX_SMART_SHARPEN_RADIUS).contains(&radius)
                    && noise.fract() == 0.0
                    && (0.0..=100.0).contains(&noise)
                    && angle.fract() == 0.0
                    && (-180.0..=180.0).contains(&angle)
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
            Self::ColorHalftone { radius, angles } => {
                radius.fract() == 0.0
                    && (MIN_HALFTONE_RADIUS..=MAX_HALFTONE_RADIUS).contains(&radius)
                    && angles.iter().all(|a| {
                        a.fract() == 0.0 && (-MAX_SCREEN_ANGLE..=MAX_SCREEN_ANGLE).contains(a)
                    })
            }
            Self::Crystallize { cell, seed } | Self::Pointillize { cell, seed, .. } => {
                cell.fract() == 0.0
                    && (MIN_CRYSTAL_CELL..=MAX_CRYSTAL_CELL).contains(&cell)
                    && seed < NOISE_SEEDS
            }
            Self::Facet { reduction } | Self::Fragment { reduction } => {
                reduction.is_finite() && reduction >= 1.0
            }
            Self::Mezzotint {
                seed, reduction, ..
            } => seed < NOISE_SEEDS && reduction.is_finite() && reduction >= 1.0,
            Self::Offset {
                horizontal,
                vertical,
                ..
            } => [horizontal, vertical]
                .iter()
                .all(|v| v.fract() == 0.0 && (-MAX_OFFSET..=MAX_OFFSET).contains(v)),
            Self::Twirl { angle } => angle.is_finite() && (-MAX_TWIRL..=MAX_TWIRL).contains(&angle),
            Self::Pinch { amount } | Self::Spherize { amount, .. } => {
                amount.is_finite() && (-100.0..=100.0).contains(&amount)
            }
            Self::Ripple { amount, .. } => {
                amount.fract() == 0.0 && (-MAX_RIPPLE_AMOUNT..=MAX_RIPPLE_AMOUNT).contains(&amount)
            }
            Self::Wave {
                generators,
                wavelength,
                amplitude,
                scale,
                seed,
                ..
            } => {
                let setting = |v: &f32| v.fract() == 0.0 && (1.0..=MAX_WAVE_SETTING).contains(v);
                setting(&generators)
                    && wavelength.iter().chain(&amplitude).all(setting)
                    && scale
                        .iter()
                        .all(|v| v.fract() == 0.0 && (1.0..=100.0).contains(v))
                    && seed < NOISE_SEEDS
            }
            Self::ZigZag { amount, ridges, .. } => {
                amount.fract() == 0.0
                    && (-MAX_ZIGZAG_AMOUNT..=MAX_ZIGZAG_AMOUNT).contains(&amount)
                    && ridges.fract() == 0.0
                    && (1.0..=MAX_ZIGZAG_RIDGES).contains(&ridges)
            }
            // The curve is valid when made.
            Self::PolarCoordinates { .. } | Self::Average | Self::Shear { .. } => true,
            Self::Displace { scale, .. } => scale.iter().all(|v| {
                v.fract() == 0.0 && (-MAX_DISPLACE_SCALE..=MAX_DISPLACE_SCALE).contains(v)
            }),
            Self::Clouds { seed, .. } | Self::DifferenceClouds { seed, .. } => seed < NOISE_SEEDS,
            Self::Fibers {
                variance,
                strength,
                seed,
                ..
            } => {
                [variance, strength]
                    .iter()
                    .all(|v| v.fract() == 0.0 && (1.0..=MAX_FIBERS_SETTING).contains(v))
                    && seed < NOISE_SEEDS
            }
            Self::Diffuse {
                seed, reduction, ..
            }
            | Self::Wind {
                seed, reduction, ..
            } => seed < NOISE_SEEDS && reduction.is_finite() && reduction >= 1.0,
            Self::Tiles {
                tiles,
                offset,
                seed,
                ..
            } => {
                [tiles, offset]
                    .iter()
                    .all(|v| v.fract() == 0.0 && (1.0..=MAX_TILES_SETTING).contains(v))
                    && seed < NOISE_SEEDS
            }
            Self::TraceContour { level, .. } => {
                level.fract() == 0.0 && (0.0..=MAX_THRESHOLD).contains(&level)
            }
            Self::OilPaint {
                stylization,
                cleanliness,
                scale,
                bristle,
                angle,
                shine,
                reduction,
                ..
            } => {
                let within =
                    |v: f32, min: f32| v.is_finite() && (min..=MAX_OIL_SETTING).contains(&v);
                within(stylization, 0.1)
                    && within(cleanliness, 0.0)
                    && within(scale, 0.1)
                    && within(bristle, 0.0)
                    && within(shine, 0.0)
                    && angle.is_finite()
                    && (-180.0..=180.0).contains(&angle)
                    && reduction.is_finite()
                    && reduction >= 1.0
            }
            Self::RadialBlur { amount, center, .. } => {
                amount.fract() == 0.0
                    && (MIN_RADIAL_AMOUNT..=MAX_RADIAL_AMOUNT).contains(&amount)
                    && center
                        .iter()
                        .all(|v| v.is_finite() && (0.0..=100.0).contains(v))
            }
            Self::SurfaceBlur { radius, threshold } => {
                radius.fract() == 0.0
                    && (MIN_SURFACE_RADIUS..=MAX_SURFACE_RADIUS).contains(&radius)
                    && threshold.fract() == 0.0
                    && (MIN_SURFACE_THRESHOLD..=MAX_SURFACE_THRESHOLD).contains(&threshold)
            }
            Self::SmartBlur {
                radius, threshold, ..
            } => {
                radius.is_finite()
                    && (MIN_SMART_RADIUS..=MAX_SMART_RADIUS).contains(&radius)
                    && threshold.is_finite()
                    && (MIN_SMART_THRESHOLD..=MAX_SMART_THRESHOLD).contains(&threshold)
            }
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
            | Self::ShadowsHighlights { .. }
            | Self::Median { .. }
            | Self::BoxBlur { .. }
            | Self::Maximum { .. }
            | Self::Minimum { .. }
            | Self::Custom { .. }
            | Self::HsbHsl { .. }
            | Self::Despeckle { .. }
            | Self::ReduceNoise { .. }
            | Self::SmartSharpen { .. }
            | Self::Solarize
            | Self::FindEdges
            | Self::Emboss { .. }
            | Self::Mosaic { .. }
            | Self::ColorHalftone { .. }
            | Self::Crystallize { .. }
            | Self::Facet { .. }
            | Self::Fragment { .. }
            | Self::Mezzotint { .. }
            | Self::Pointillize { .. }
            | Self::Offset { .. }
            | Self::Twirl { .. }
            | Self::Pinch { .. }
            | Self::Spherize { .. }
            | Self::Ripple { .. }
            | Self::Wave { .. }
            | Self::ZigZag { .. }
            | Self::Shear { .. }
            | Self::Displace { .. }
            | Self::PolarCoordinates { .. }
            | Self::RadialBlur { .. }
            | Self::SurfaceBlur { .. }
            | Self::SmartBlur { .. }
            | Self::Average
            | Self::Clouds { .. }
            | Self::DifferenceClouds { .. }
            | Self::Fibers { .. }
            | Self::Diffuse { .. }
            | Self::Wind { .. }
            | Self::Tiles { .. }
            | Self::TraceContour { .. }
            | Self::OilPaint { .. } => None,
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

    /// Shadows/Highlights' blurs' radii, the shadows' then the highlights' (see
    /// [`Self::plans`]): no less than the smallest blur.
    pub fn shadows_radii(&self) -> Option<[f32; 2]> {
        match *self {
            Self::ShadowsHighlights {
                shadows,
                highlights,
                scale,
                ..
            } => Some([shadows[2], highlights[2]].map(|r| (r / scale).max(MIN_BLUR_RADIUS))),
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
            Self::Maximum { radius, round } => Self::Maximum {
                radius: (radius / factor).round().max(MIN_EXTREME_RADIUS),
                round,
            },
            Self::Minimum { radius, round } => Self::Minimum {
                radius: (radius / factor).round().max(MIN_EXTREME_RADIUS),
                round,
            },
            // Weights of the look's pixels: as on a reduced layer as Find Edges' lines.
            Self::Custom { .. } | Self::HsbHsl { .. } => *self,
            Self::Despeckle { reduction } => Self::Despeckle {
                reduction: reduction * factor,
            },
            Self::ReduceNoise {
                strength,
                preserve,
                color,
                sharpen,
                reduction,
            } => Self::ReduceNoise {
                strength,
                preserve,
                color,
                sharpen,
                reduction: reduction * factor,
            },
            Self::SmartSharpen {
                amount,
                radius,
                noise,
                remove,
                angle,
            } => Self::SmartSharpen {
                amount,
                radius: (radius / factor).max(MIN_SMART_SHARPEN_RADIUS),
                noise,
                remove,
                angle,
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
            // Dots in pixels of the reduced layer, from the same origin.
            Self::ColorHalftone { radius, angles } => Self::ColorHalftone {
                radius: radius / factor,
                angles,
            },
            Self::Facet { reduction } => Self::Facet {
                reduction: reduction * factor,
            },
            Self::Fragment { reduction } => Self::Fragment {
                reduction: reduction * factor,
            },
            Self::Mezzotint {
                kind,
                seed,
                reduction,
            } => Self::Mezzotint {
                kind,
                seed,
                reduction: reduction * factor,
            },
            Self::SurfaceBlur { radius, threshold } => Self::SurfaceBlur {
                radius: (radius / factor).round().max(MIN_SURFACE_RADIUS),
                threshold,
            },
            Self::SmartBlur {
                radius,
                threshold,
                quality,
                mode,
            } => Self::SmartBlur {
                radius: (radius / factor).max(MIN_SMART_RADIUS),
                threshold,
                quality,
                mode,
            },
            // Of the whole layer whatever the level.
            Self::Average => Self::Average,
            // Mapped in the layer's own pixels whatever the level (see [`Self::source`] and
            // [`Self::radial_samples`]).
            Self::RadialBlur { .. }
            | Self::Offset { .. }
            | Self::Twirl { .. }
            | Self::Pinch { .. }
            | Self::Spherize { .. }
            | Self::Ripple { .. }
            | Self::Wave { .. }
            | Self::ZigZag { .. }
            | Self::Shear { .. }
            | Self::Displace { .. }
            | Self::PolarCoordinates { .. }
            | Self::Clouds { .. }
            | Self::DifferenceClouds { .. }
            | Self::Fibers { .. }
            | Self::Tiles { .. }
            | Self::Crystallize { .. }
            | Self::Pointillize { .. } => *self,
            // Lines of a pixel: as thin on a reduced layer.
            Self::TraceContour { .. } => *self,
            Self::Diffuse {
                mode,
                seed,
                reduction,
            } => Self::Diffuse {
                mode,
                seed,
                reduction: reduction * factor,
            },
            Self::Wind {
                method,
                from_left,
                seed,
                reduction,
            } => Self::Wind {
                method,
                from_left,
                seed,
                reduction: reduction * factor,
            },
            Self::OilPaint {
                stylization,
                cleanliness,
                scale,
                bristle,
                lighting,
                angle,
                shine,
                reduction,
            } => Self::OilPaint {
                stylization,
                cleanliness,
                scale,
                bristle,
                lighting,
                angle,
                shine,
                reduction: reduction * factor,
            },
            Self::ClarityTexture {
                texture,
                clarity,
                scale,
            } => Self::ClarityTexture {
                texture,
                clarity,
                scale: scale * factor,
            },
            Self::ShadowsHighlights {
                shadows,
                highlights,
                color,
                midtone,
                scale,
            } => Self::ShadowsHighlights {
                shadows,
                highlights,
                color,
                midtone,
                scale: scale * factor,
            },
        }
    }

    /// How far a pixel's result reads, in pixels on each side, with some room (a look's
    /// margin): three and a half sigmas of a blur, half a line.
    pub fn reach(&self) -> f64 {
        match *self {
            Self::MotionBlur { distance, .. } => f64::from(distance) / 2.0 + 2.0,
            Self::AddNoise { .. }
            | Self::Clouds { .. }
            | Self::DifferenceClouds { .. }
            | Self::Fibers { .. } => 0.0,
            Self::Diffuse { .. } | Self::TraceContour { .. } => 2.0,
            Self::Wind { .. } | Self::OilPaint { .. } => self.plan().kernel.reach() as f64 + 2.0,
            Self::DustAndScratches { radius, .. }
            | Self::Median { radius }
            | Self::BoxBlur { radius }
            | Self::Maximum { radius, .. }
            | Self::Minimum { radius, .. } => f64::from(radius) + 2.0,
            Self::HsbHsl { .. } => 0.0,
            Self::Custom { .. }
            | Self::Despeckle { .. }
            | Self::ReduceNoise { .. }
            | Self::SmartSharpen { .. } => {
                self.plans()
                    .iter()
                    .map(|plan| plan.kernel.reach())
                    .max()
                    .unwrap_or(0) as f64
                    + 2.0
            }
            Self::Solarize => 0.0,
            Self::FindEdges => 2.0,
            Self::Emboss { height, .. } => f64::from(height) + 2.0,
            Self::Mosaic { cell } => f64::from(cell) + 2.0,
            Self::ColorHalftone { .. }
            | Self::Facet { .. }
            | Self::Fragment { .. }
            | Self::Mezzotint { .. } => self.plan().kernel.reach() as f64 + 2.0,
            // A pixel reads its cell's point (at most a cell's diagonal away), its dot's center
            // (at most a cell away): in the layer's pixels, as many as a reduced level needs at
            // most.
            Self::Crystallize { cell, .. } => f64::from(cell) * std::f64::consts::SQRT_2 + 3.0,
            Self::Pointillize { cell, .. } => f64::from(cell) + 3.0,
            Self::SurfaceBlur { radius, .. } | Self::SmartBlur { radius, .. } => {
                f64::from(radius).ceil() + 2.0
            }
            // A pixel may read from anywhere in the layer.
            Self::RadialBlur { .. }
            | Self::Average
            | Self::Offset { .. }
            | Self::Twirl { .. }
            | Self::Pinch { .. }
            | Self::Spherize { .. }
            | Self::Ripple { .. }
            | Self::Wave { .. }
            | Self::ZigZag { .. }
            | Self::Shear { .. }
            | Self::Displace { .. }
            | Self::PolarCoordinates { .. }
            | Self::Tiles { .. } => f64::INFINITY,
            Self::ClarityTexture { scale, .. } => {
                3.5 * f64::from((CLARITY_RADIUS / scale).max(MIN_BLUR_RADIUS)) + 2.0
            }
            Self::ShadowsHighlights { .. } => {
                let [s, h] = self.shadows_radii().unwrap_or_default();
                3.5 * f64::from(s.max(h)) + 2.0
            }
            _ => 3.5 * f64::from(self.blur_radius().unwrap_or(0.0)) + 2.0,
        }
    }

    /// How the filter is computed over a layer: what it blurs by, at what reduction; Clarity
    /// and Texture blur twice, Smart Sharpen's Lens Blur takes the extremes around each pixel
    /// too ([`Self::finish`] gets one blur per plan, in this order).
    pub(crate) fn plans(&self) -> Vec<Plan> {
        match *self {
            Self::SmartSharpen {
                radius,
                remove: SharpenRemove::LensBlur,
                ..
            } => {
                // The square's extremes: exact and as fast whatever the radius.
                let around = f64::from(radius).ceil().max(1.0) as usize;
                let extreme = |largest| Plan {
                    factor: 1,
                    kernel: Kernel::Extreme {
                        radius: around,
                        largest,
                        round: false,
                    },
                };
                vec![self.plan(), extreme(true), extreme(false)]
            }
            Self::ClarityTexture { .. } => self
                .clarity_radii()
                .unwrap_or_default()
                .map(|r| Plan::gaussian(f64::from(r)))
                .to_vec(),
            Self::ShadowsHighlights { .. } => self
                .shadows_radii()
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
            Self::AddNoise { .. }
            | Self::Clouds { .. }
            | Self::DifferenceClouds { .. }
            | Self::Fibers { .. }
            | Self::Tiles { .. } => Plan {
                factor: 1,
                kernel: Kernel::Identity,
            },
            Self::Diffuse {
                mode,
                seed,
                reduction,
            } => Plan {
                factor: 1,
                kernel: if reduction > 1.5 {
                    Kernel::Identity
                } else {
                    Kernel::Diffuse { mode, seed }
                },
            },
            Self::Wind {
                method,
                from_left,
                seed,
                reduction,
            } => {
                let (length, strength, stagger) = method.streaks();
                Plan {
                    factor: 1,
                    kernel: Kernel::Wind {
                        length: (length / f64::from(reduction)).round().max(1.0) as usize,
                        strength,
                        stagger,
                        from_left,
                        seed,
                    },
                }
            }
            Self::TraceContour { level, upper } => Plan {
                factor: 1,
                kernel: Kernel::Contour {
                    level: contour_level(level),
                    upper,
                },
            },
            Self::OilPaint {
                stylization,
                cleanliness,
                scale,
                bristle,
                lighting,
                angle,
                shine,
                reduction,
            } => {
                let r = f64::from(reduction);
                let (sin, cos) = f64::from(angle).to_radians().sin_cos();
                Plan {
                    factor: 1,
                    kernel: Kernel::Oil(Oil {
                        radius: ((1.0 + f64::from(stylization) * 0.6 * f64::from(scale).sqrt()) / r)
                            .round()
                            .max(1.0) as usize,
                        clean: f64::from(cleanliness) * 0.2 / r,
                        bristle: f64::from(bristle) / MAX_OIL_SETTING as f64,
                        grain: r,
                        // Toward the light (up is negative y).
                        light: lighting.then_some([cos, -sin]),
                        relief: f64::from(shine) * f64::from(scale) * OIL_RELIEF,
                    }),
                }
            }
            Self::DustAndScratches { radius, .. } | Self::Median { radius } => {
                Plan::median(f64::from(radius))
            }
            Self::BoxBlur { radius } => Plan::boxed(f64::from(radius)),
            Self::Maximum { radius, round } | Self::Minimum { radius, round } => {
                let largest = matches!(self, Self::Maximum { .. });
                Plan::extreme(f64::from(radius), largest, round)
            }
            Self::Custom { weights, scale, .. } => {
                let scale = f64::from(scale);
                Plan {
                    factor: 1,
                    kernel: Kernel::Convolve(Box::new(weights.map(|w| f64::from(w) / scale))),
                }
            }
            Self::HsbHsl { .. } => Plan {
                factor: 1,
                kernel: Kernel::Identity,
            },
            Self::Despeckle { reduction } => {
                let radius = (SPECKLE_RADIUS / f64::from(reduction)).round() as usize;
                let eps = SPECKLE_SIGMA * SPECKLE_SIGMA;
                Plan {
                    factor: 1,
                    kernel: Kernel::Denoise(Denoise {
                        radius,
                        eps,
                        color_radius: radius,
                        color_eps: eps,
                        sharpen: 0.0,
                    }),
                }
            }
            Self::ReduceNoise {
                strength,
                preserve,
                color,
                sharpen,
                reduction,
            } => {
                let r = f64::from(reduction);
                let sigma = NOISE_SIGMA * f64::from(strength) / f64::from(MAX_NOISE_STRENGTH)
                    * (1.0 - 0.9 * f64::from(preserve) / 100.0);
                let color_sigma = COLOR_NOISE_SIGMA * f64::from(color) / 100.0;
                Plan {
                    factor: 1,
                    kernel: Kernel::Denoise(Denoise {
                        radius: if sigma > 0.0 {
                            (NOISE_RADIUS / r).round() as usize
                        } else {
                            0
                        },
                        eps: sigma * sigma,
                        color_radius: if color_sigma > 0.0 {
                            (COLOR_NOISE_RADIUS / r).round() as usize
                        } else {
                            0
                        },
                        color_eps: color_sigma * color_sigma,
                        // Details of a pixel: none left to sharpen on a level reduced more.
                        sharpen: if r < 1.5 {
                            f64::from(sharpen) / 100.0
                        } else {
                            0.0
                        },
                    }),
                }
            }
            Self::SmartSharpen {
                radius,
                remove,
                angle,
                ..
            } => match remove {
                SharpenRemove::GaussianBlur | SharpenRemove::LensBlur => {
                    Plan::gaussian(f64::from(radius))
                }
                // Lines either way: the angle as Motion Blur's (-90 to 90).
                SharpenRemove::MotionBlur => Plan::line(
                    (f64::from(angle) + 90.0).rem_euclid(180.0) - 90.0,
                    (2.0 * f64::from(radius)).max(f64::from(MIN_MOTION_DISTANCE)),
                ),
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
            Self::ColorHalftone { radius, angles } => Plan {
                factor: 1,
                kernel: if f64::from(radius) < HALFTONE_FROM {
                    Kernel::Identity
                } else {
                    Kernel::Halftone(Halftone::new(f64::from(radius), angles))
                },
            },
            // A Kuwahara filter: Oil Paint's strokes, unlit.
            Self::Facet { reduction } => {
                let radius = (FACET_RADIUS / f64::from(reduction)).round() as usize;
                Plan {
                    factor: 1,
                    kernel: if radius == 0 {
                        Kernel::Identity
                    } else {
                        Kernel::Oil(Oil {
                            radius,
                            clean: 0.0,
                            bristle: 0.0,
                            grain: 1.0,
                            light: None,
                            relief: 0.0,
                        })
                    },
                }
            }
            Self::Fragment { reduction } => Plan {
                factor: 1,
                kernel: Kernel::Fragment(FRAGMENT_OFFSET / f64::from(reduction)),
            },
            Self::Mezzotint {
                kind,
                seed,
                reduction,
            } => Plan {
                factor: 1,
                kernel: Kernel::Mezzotint {
                    kind,
                    seed,
                    reduction: reduction.round().max(1.0) as usize,
                },
            },
            // Sampled where [`Self::source`] says.
            Self::Crystallize { .. } | Self::Pointillize { .. } => Plan {
                factor: 1,
                kernel: Kernel::Identity,
            },
            Self::SurfaceBlur { radius, threshold } => Plan {
                factor: 1,
                kernel: Kernel::Surface {
                    radius: radius as usize,
                    threshold: f64::from(threshold) / 255.0,
                },
            },
            Self::SmartBlur {
                radius,
                threshold,
                quality,
                mode,
            } => Plan {
                factor: 1,
                kernel: Kernel::Smart {
                    radius: f64::from(radius),
                    threshold: f64::from(threshold) / 255.0,
                    taps: quality.taps(),
                    mode,
                },
            },
            // Sampled where [`Self::source`] and [`Self::radial_samples`] say, or of the whole
            // layer: not filtered by a kernel.
            Self::RadialBlur { .. }
            | Self::Average
            | Self::Offset { .. }
            | Self::Twirl { .. }
            | Self::Pinch { .. }
            | Self::Spherize { .. }
            | Self::Ripple { .. }
            | Self::Wave { .. }
            | Self::ZigZag { .. }
            | Self::Shear { .. }
            | Self::Displace { .. }
            | Self::PolarCoordinates { .. } => Plan {
                factor: 1,
                kernel: Kernel::Identity,
            },
            Self::ClarityTexture { .. } | Self::ShadowsHighlights { .. } => Plan {
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
                | Self::Ripple { .. }
                | Self::Wave { .. }
                | Self::ZigZag { .. }
                | Self::Shear { .. }
                | Self::Displace { .. }
                | Self::PolarCoordinates { .. }
                | Self::Tiles { .. }
                | Self::Crystallize { .. }
                | Self::Pointillize { .. }
        )
    }

    /// What a distortion brings in where [`Self::source`] reads nothing: transparency, Tiles'
    /// fill, or Pointillize's background, from `original` (the pixel's own premultiplied value)
    /// and `colors` (the filter's colors in the blend space, see [`Self::render`]).
    pub fn gap(&self, original: [f64; 4], colors: Option<[[f64; 4]; 2]>) -> [f64; 4] {
        if let (Self::Pointillize { .. }, Some(colors)) = (self, colors) {
            return colors[1];
        }
        let Self::Tiles { fill, .. } = *self else {
            return [0.0; 4];
        };
        match (fill, colors) {
            (TilesFill::Background, Some(colors)) => colors[1],
            (TilesFill::Foreground, Some(colors)) => colors[0],
            (TilesFill::Inverse, _) => {
                let a = original[3];
                [a - original[0], a - original[1], a - original[2], a]
            }
            (TilesFill::Unaltered, _) | (_, None) => original,
        }
    }

    /// Whether the filter works within a frame (the selection's box or the layer): a distortion,
    /// or Radial Blur around its center.
    pub fn framed(&self) -> bool {
        self.samples() || matches!(self, Self::RadialBlur { .. })
    }

    /// Radial Blur's samples for the pixel at layer point `p` (layer pixels) in `frame`
    /// (`[left, top, right, bottom]`, as [`Self::source`]'s), each image pixel `pixel` layer
    /// pixels (more on a reduced level): `each` called with each point to read (layer pixels),
    /// evenly spread along the arc (or the line) centered on `p`, as many as the quality takes
    /// for the path's length in image pixels; the number of points. Only `p` for another filter,
    /// or a path shorter than half a pixel.
    pub fn radial_samples(
        &self,
        p: [f64; 2],
        frame: [f64; 4],
        pixel: f64,
        mut each: impl FnMut([f64; 2]),
    ) -> usize {
        let Self::RadialBlur {
            amount,
            zoom,
            quality,
            center,
        } = *self
        else {
            each(p);
            return 1;
        };
        let [left, top, right, bottom] = frame;
        let c = [
            left + (right - left) * f64::from(center[0]) / 100.0,
            top + (bottom - top) * f64::from(center[1]) / 100.0,
        ];
        let v = [p[0] - c[0], p[1] - c[1]];
        let d = (v[0] * v[0] + v[1] * v[1]).sqrt();
        let a = f64::from(amount);
        // The whole path: a fraction of the distance (Zoom), or an arc of `amount` degrees.
        let span = if zoom {
            a / 100.0 * ZOOM_SPAN
        } else {
            a.to_radians()
        };
        let length = d * span / pixel.max(f64::MIN_POSITIVE);
        if length < 0.5 {
            each(p);
            return 1;
        }
        let (density, most) = quality.sampling();
        let n = ((length * density).ceil() as usize + 1).clamp(2, most);
        for i in 0..n {
            let t = (i as f64 / (n - 1) as f64 - 0.5) * span;
            if zoom {
                let s = 1.0 + t;
                each([c[0] + v[0] * s, c[1] + v[1] * s]);
            } else {
                let (sin, cos) = t.sin_cos();
                each([
                    c[0] + v[0] * cos - v[1] * sin,
                    c[1] + v[0] * sin + v[1] * cos,
                ]);
            }
        }
        n
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
            Self::Spherize { amount, mode } => {
                let half = [w / 2.0, h / 2.0];
                if half[0] <= 0.0 || half[1] <= 0.0 {
                    return Some(p);
                }
                // A cylinder: the other axis left as it is.
                let (across, down) = match mode {
                    SpherizeMode::Normal => (true, true),
                    SpherizeMode::HorizontalOnly => (true, false),
                    SpherizeMode::VerticalOnly => (false, true),
                };
                let u = [
                    if across { v[0] / half[0] } else { 0.0 },
                    if down { v[1] / half[1] } else { 0.0 },
                ];
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
                Some([
                    if across {
                        c[0] + u[0] * s * half[0]
                    } else {
                        p[0]
                    },
                    if down {
                        c[1] + u[1] * s * half[1]
                    } else {
                        p[1]
                    },
                ])
            }
            Self::Ripple { amount, size } => {
                let lambda = size.wavelength();
                let a = f64::from(amount) / 100.0 * RIPPLE_STRENGTH * lambda;
                let k = std::f64::consts::TAU / lambda;
                Some([p[0] + a * (k * p[1]).sin(), p[1] + a * (k * p[0]).sin()])
            }
            Self::Wave {
                generators,
                wavelength,
                amplitude,
                scale,
                kind,
                wrap,
                seed,
            } => {
                let n = generators as u32;
                let range = |[a, b]: [f32; 2]| (f64::from(a.min(b)), f64::from(a.max(b)));
                let ((w0, w1), (a0, a1)) = (range(wavelength), range(amplitude));
                let mut shift = [0.0; 2];
                for i in 0..n {
                    // Three draws per direction: amplitude, wavelength, phase.
                    let draw = |k: u32| {
                        f64::from(hash(seed ^ hash(i * 8 + k)) >> 8) / f64::from(1u32 << 24)
                    };
                    for (axis, across) in [(0, p[1]), (1, p[0])] {
                        let k = axis as u32 * 3;
                        let amp = a0 + (a1 - a0) * draw(k);
                        let lambda = w0 + (w1 - w0) * draw(k + 1);
                        shift[axis] += amp * kind.shape(across / lambda + draw(k + 2));
                    }
                }
                let q = [0, 1]
                    .map(|i| p[i] + shift[i] / f64::from(n.max(1)) * f64::from(scale[i]) / 100.0);
                Some(if wrap && w > 0.0 && h > 0.0 {
                    [
                        left + (q[0] - left).rem_euclid(w),
                        top + (q[1] - top).rem_euclid(h),
                    ]
                } else {
                    q
                })
            }
            Self::ZigZag {
                amount,
                ridges,
                style,
            } => {
                if d >= radius || radius <= 0.0 {
                    return Some(p);
                }
                let r = d / radius;
                let ridges = f64::from(ridges);
                // Most near the center, nothing on the circle.
                let wave = f64::from(amount) / 100.0
                    * (std::f64::consts::TAU * ridges * r).sin()
                    * (1.0 - r);
                // Half a ridge's spacing at 100 %.
                let push = wave * radius / ridges / 2.0;
                match style {
                    ZigZagStyle::AroundCenter => {
                        let (sin, cos) = (wave * ZIGZAG_TURN).sin_cos();
                        Some([
                            c[0] + v[0] * cos - v[1] * sin,
                            c[1] + v[0] * sin + v[1] * cos,
                        ])
                    }
                    ZigZagStyle::OutFromCenter => {
                        if d == 0.0 {
                            return Some(p);
                        }
                        let k = (d + push) / d;
                        Some([c[0] + v[0] * k, c[1] + v[1] * k])
                    }
                    ZigZagStyle::PondRipples => {
                        let s = push * std::f64::consts::FRAC_1_SQRT_2;
                        Some([p[0] + s, p[1] + s])
                    }
                }
            }
            Self::Shear { curve, wrap } => {
                if w <= 0.0 || h <= 0.0 {
                    return Some(p);
                }
                // Read from as far back as the row moves.
                let x = p[0] - curve.offset((p[1] - top) / h) * w / 2.0;
                Some([
                    if wrap {
                        left + (x - left).rem_euclid(w)
                    } else {
                        // The frame's edge pixels (the selection's box's, not the layer's beyond).
                        x.clamp(left + 0.5, (right - 0.5).max(left + 0.5))
                    },
                    p[1],
                ])
            }
            Self::Tiles {
                tiles,
                offset,
                seed,
                ..
            } => {
                if w <= 0.0
                    || h <= 0.0
                    || p[0] < left
                    || p[1] < top
                    || p[0] >= right
                    || p[1] >= bottom
                {
                    return Some(p);
                }
                let side = w.min(h) / f64::from(tiles);
                let (columns, rows) = ((w / side).ceil() as i64, (h / side).ceil() as i64);
                let (col, row) = (
                    ((p[0] - left) / side).floor() as i64,
                    ((p[1] - top) / side).floor() as i64,
                );
                let most = f64::from(offset) / 100.0 * side;
                // A tile moves by less than its side: only those around can cover `p`, the
                // last one row by row on top.
                let mut found = None;
                for r in (row - 1).max(0)..=(row + 1).min(rows - 1) {
                    for c in (col - 1).max(0)..=(col + 1).min(columns - 1) {
                        let draw = |k: u32| {
                            let h = hash(c as i32 as u32 ^ hash(r as i32 as u32 ^ hash(seed ^ k)));
                            f64::from(h >> 8) / f64::from(1u32 << 24) * 2.0 - 1.0
                        };
                        let q = [p[0] - draw(0) * most, p[1] - draw(1) * most];
                        let (x0, y0) = (left + c as f64 * side, top + r as f64 * side);
                        if q[0] >= x0
                            && q[1] >= y0
                            && q[0] < (x0 + side).min(right)
                            && q[1] < (y0 + side).min(bottom)
                        {
                            found = Some(q);
                        }
                    }
                }
                found
            }
            // The color at the closest of the cells' points, from the layer's origin whatever
            // the frame.
            Self::Crystallize { cell, seed } => {
                let c = f64::from(cell);
                let square = nearest_square(p, c, seed, CRYSTAL_SALT, 1.0);
                Some(pixel_center(
                    cell_point(square, c, seed, CRYSTAL_SALT, 1.0),
                    layer,
                ))
            }
            // The center of the last dot over `p`: a dot is less than a cell across, so only
            // those of the squares around can be.
            Self::Pointillize { cell, seed, .. } => {
                let c = f64::from(cell);
                let (ci, cj) = ((p[0] / c).floor() as i64, (p[1] / c).floor() as i64);
                let mut found = None;
                for j in cj - 1..=cj + 1 {
                    for i in ci - 1..=ci + 1 {
                        let q = cell_point([i, j], c, seed, DOT_SALT, 1.0);
                        let [low, high] = DOT_RADIUS;
                        let r = c * (low + (high - low) * draw([i, j], seed, DOT_SALT + 2));
                        if (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) <= r * r {
                            found = Some(q);
                        }
                    }
                }
                found.map(|q| pixel_center(q, layer))
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

    /// Displace's map point for the pixel at layer point `p` (layer pixels) in `frame` (as
    /// [`Self::source`]'s), the map `map` pixels (width, height): in the map's pixels, their
    /// centers on whole numbers (as a bilinear read takes them). Stretched over the frame, or
    /// tiled from its corner at the map's size. `p` itself for another filter.
    pub fn map_point(&self, p: [f64; 2], frame: [f64; 4], map: [f64; 2]) -> [f64; 2] {
        let Self::Displace { tile, .. } = *self else {
            return p;
        };
        let [left, top, right, bottom] = frame;
        let (w, h) = (right - left, bottom - top);
        let at = |v: f64, start: f64, size: f64, side: f64| {
            if tile && side > 0.0 {
                (v - start).rem_euclid(side) - 0.5
            } else if size > 0.0 {
                (v - start) / size * side - 0.5
            } else {
                0.0
            }
        };
        [at(p[0], left, w, map[0]), at(p[1], top, h, map[1])]
    }

    /// Where Displace reads the pixel at layer point `p` from, in `frame` (as
    /// [`Self::source`]'s), its map there `value` (its red and green channels, 0 to 1, over
    /// 128 levels of gray where it is transparent): as many pixels as the value's levels (of 8
    /// bits) from 128, by the scale (Photoshop's: at 100 %, 0 reads 128 pixels back, 255 127
    /// on); wrapped within the frame or its edge pixels repeated. `p` itself for another
    /// filter.
    pub fn displaced(&self, p: [f64; 2], frame: [f64; 4], value: [f64; 2]) -> [f64; 2] {
        let Self::Displace { scale, wrap, .. } = *self else {
            return p;
        };
        let [left, top, right, bottom] = frame;
        let q: [f64; 2] = std::array::from_fn(|i| {
            p[i] + (value[i] * 255.0 - 128.0) * f64::from(scale[i]) / 100.0
        });
        let within = |v: f64, start: f64, end: f64| {
            if end - start <= 0.0 {
                v
            } else if wrap {
                start + (v - start).rem_euclid(end - start)
            } else {
                v.clamp(start + 0.5, (end - 0.5).max(start + 0.5))
            }
        };
        [within(q[0], left, right), within(q[1], top, bottom)]
    }

    /// Whether the filter reads a displacement map (the step's `map`): Displace.
    pub fn takes_map(&self) -> bool {
        matches!(self, Self::Displace { .. })
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
                | Self::Ripple { .. }
                | Self::Wave { .. }
                | Self::ZigZag { .. }
                | Self::Shear { .. }
                | Self::Displace { .. }
                | Self::PolarCoordinates { .. }
                | Self::RadialBlur { .. }
                | Self::SurfaceBlur { .. }
                | Self::SmartBlur { .. }
                | Self::Clouds { .. }
                | Self::Fibers { .. }
                | Self::Diffuse { .. }
                | Self::Wind { .. }
                | Self::Tiles { .. }
                | Self::TraceContour { .. }
                | Self::OilPaint { .. }
                | Self::ColorHalftone { .. }
                | Self::Crystallize { .. }
                | Self::Facet { .. }
                | Self::Fragment { .. }
                | Self::Mezzotint { .. }
                | Self::Pointillize { .. }
                | Self::Despeckle { .. }
                | Self::ReduceNoise { .. }
        )
    }

    /// Whether the filter draws (Clouds, Difference Clouds, Fibers): each pixel's
    /// [`Self::render`] its "blur".
    pub fn renders(&self) -> bool {
        matches!(
            self,
            Self::Clouds { .. } | Self::DifferenceClouds { .. } | Self::Fibers { .. }
        )
    }

    /// The colors a filter stores (a drawing filter's, Tiles', Pointillize's), foreground then
    /// background, sRGB-encoded.
    pub fn colors(&self) -> Option<[[u8; 3]; 2]> {
        match *self {
            Self::Clouds {
                foreground,
                background,
                ..
            }
            | Self::DifferenceClouds {
                foreground,
                background,
                ..
            }
            | Self::Fibers {
                foreground,
                background,
                ..
            }
            | Self::Tiles {
                foreground,
                background,
                ..
            } => Some([foreground, background]),
            // Its background only.
            Self::Pointillize { background, .. } => Some([background; 2]),
            _ => None,
        }
    }

    /// What a drawing filter draws at layer point `p` (layer pixels): its two colors `colors`
    /// (premultiplied blend values, opaque: [`Self::colors`] in the step's blend space) mixed by
    /// its noise there. `colors[0]` for another filter.
    pub fn render(&self, p: [f64; 2], colors: [[f64; 4]; 2]) -> [f64; 4] {
        let v = match *self {
            Self::Clouds { seed, .. } | Self::DifferenceClouds { seed, .. } => clouds(p, seed),
            Self::Fibers {
                variance,
                strength,
                seed,
                ..
            } => fibers(p, f64::from(variance), f64::from(strength), seed),
            _ => 0.0,
        };
        std::array::from_fn(|c| colors[0][c] + (colors[1][c] - colors[0][c]) * v)
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
            | Self::Ripple { .. }
            | Self::Wave { .. }
            | Self::ZigZag { .. }
            | Self::Shear { .. }
            | Self::Displace { .. }
            | Self::PolarCoordinates { .. }
            | Self::RadialBlur { .. }
            | Self::SurfaceBlur { .. }
            | Self::SmartBlur { .. }
            | Self::Clouds { .. }
            | Self::Fibers { .. }
            | Self::Diffuse { .. }
            | Self::Wind { .. }
            | Self::Tiles { .. }
            | Self::TraceContour { .. }
            | Self::OilPaint { .. }
            | Self::ColorHalftone { .. }
            | Self::Crystallize { .. }
            | Self::Facet { .. }
            | Self::Fragment { .. }
            | Self::Mezzotint { .. }
            | Self::Pointillize { .. }
            | Self::Despeckle { .. }
            | Self::ReduceNoise { .. } => blurred,
            // The weighted sum of the premultiplied values (divided by the scale) as a color of
            // the pixel's opacity, plus the offset.
            Self::Custom { offset, .. } => {
                if alpha <= 0.0 {
                    return original;
                }
                let k = f64::from(offset) / 255.0;
                let c: [f64; 3] = std::array::from_fn(|i| blurred[i] / alpha + k);
                [c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha]
            }
            Self::HsbHsl { input, order } => {
                if alpha <= 0.0 {
                    return original;
                }
                let c = order.encode(input.decode(o.map(|v| v.clamp(0.0, 1.0))));
                [c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha]
            }
            Self::SmartSharpen {
                amount,
                noise,
                remove,
                ..
            } => {
                if alpha <= 0.0 {
                    return original;
                }
                let k = f64::from(amount) / 100.0;
                // Differences well below the noise's level hardly sharpened, well above fully.
                let n = f64::from(noise) / 100.0 * SHARPEN_NOISE;
                let n2 = n * n;
                let c: [f64; 3] = std::array::from_fn(|i| {
                    let d = o[i] - b[i];
                    let d2 = d * d;
                    let pushed = if d2 + n2 > 0.0 {
                        d * d2 / (d2 + n2)
                    } else {
                        0.0
                    };
                    o[i] + k * pushed
                });
                // Lens Blur: no color beyond those around (no halo).
                let c = if remove == SharpenRemove::LensBlur && blurs.len() == 3 {
                    let (high, low) = (straight(blurs[1]), straight(blurs[2]));
                    std::array::from_fn(|i| c[i].clamp(low[i].min(high[i]), high[i]))
                } else {
                    c
                };
                [c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha]
            }
            // Each channel the difference of the pixel's color and the clouds' (`blurred`,
            // opaque), its own opacity.
            Self::DifferenceClouds { .. } => color(&|o, b| (o - b).abs()),
            // The average's color (premultiplied in `blurred`), the pixel's own opacity.
            Self::Average => {
                if blurred[3] <= 0.0 {
                    return original;
                }
                let c = straight(blurred);
                [c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha]
            }
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
            Self::ShadowsHighlights {
                shadows,
                highlights,
                color,
                midtone,
                ..
            } => {
                if alpha <= 0.0 {
                    return original;
                }
                let c = shadows_highlights(
                    o,
                    [straight(blurs[0]), straight(blurs[1])].map(luma),
                    [shadows[0], shadows[1], highlights[0], highlights[1]]
                        .map(|v| f64::from(v) / 100.0),
                    f64::from(color) / 100.0,
                    f64::from(midtone) / 100.0,
                );
                [c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha]
            }
        }
    }
}

/// `color` as a filter's parameter (see [`PACKED_COLORS`]).
fn packed([r, g, b]: [u8; 3]) -> f32 {
    ((u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)) as f32
}

/// A packed color parameter back (see [`PACKED_COLORS`]); `None` when it is not one.
fn unpacked(v: f32) -> Option<[u8; 3]> {
    (v.fract() == 0.0 && (0.0..PACKED_COLORS as f32).contains(&v)).then(|| {
        let n = v as u32;
        [(n >> 16) as u8, (n >> 8) as u8, n as u8]
    })
}

/// Trace Contour's `level` (of 8 bits) as the value a channel is above when its 8-bit level is:
/// halfway to the next level, so that a value at the level is never taken for one above it
/// whatever the rounding (the GPU's is the same).
pub fn contour_level(level: f32) -> f64 {
    (f64::from(level) + 0.5) / 255.0
}

/// A flag parameter back: 0 or 1.
fn flag(v: f32) -> Option<bool> {
    (v == 0.0 || v == 1.0).then_some(v == 1.0)
}

/// A seed parameter back: a whole number below [`NOISE_SEEDS`].
fn whole_seed(v: f32) -> Option<u32> {
    (v.fract() == 0.0 && (0.0..NOISE_SEEDS as f32).contains(&v)).then_some(v as u32)
}

/// Gradient (Perlin) noise at `p` (in lattice cells) for `seed` and `octave`, about -0.7 to
/// 0.7, zero on the lattice: each corner's gradient a direction drawn from a hash of the
/// corner, the seed and the octave, blended by a quintic.
pub fn gradient_noise(p: [f64; 2], seed: u32, octave: u32) -> f64 {
    let (x0, y0) = (p[0].floor(), p[1].floor());
    let (fx, fy) = (p[0] - x0, p[1] - y0);
    let (x0, y0) = (x0 as i64, y0 as i64);
    let salt = hash(seed ^ hash(octave.wrapping_add(0x9e37_79b9)));
    let corner = |dx: i64, dy: i64| {
        let h = hash((x0 + dx) as i32 as u32 ^ hash((y0 + dy) as i32 as u32 ^ salt));
        let angle = f64::from(h >> 8) / f64::from(1u32 << 24) * std::f64::consts::TAU;
        let (sin, cos) = angle.sin_cos();
        cos * (fx - dx as f64) + sin * (fy - dy as f64)
    };
    let fade = |t: f64| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v) = (fade(fx), fade(fy));
    let top = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * u;
    let bottom = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * u;
    top + (bottom - top) * v
}

/// Clouds' value at layer point `p` for `seed`, 0 to 1: octaves of gradient noise from
/// [`CLOUDS_SCALE`] down to a couple of pixels, each half as strong as the one before
/// (fractional Brownian motion), averaged and stretched around the middle.
pub fn clouds(p: [f64; 2], seed: u32) -> f64 {
    let (mut sum, mut total, mut amplitude, mut wavelength) = (0.0, 0.0, 1.0, CLOUDS_SCALE);
    let mut octave = 0;
    while wavelength >= CLOUDS_FINEST {
        sum += amplitude * gradient_noise([p[0] / wavelength, p[1] / wavelength], seed, octave);
        total += amplitude;
        amplitude *= 0.5;
        wavelength /= 2.0;
        octave += 1;
    }
    (0.5 + sum / total * CLOUDS_CONTRAST).clamp(0.0, 1.0)
}

/// Fibers' value at layer point `p` for `seed`, 0 to 1: octaves of gradient noise stretched
/// vertically, the finest 2 pixels across and [`FIBERS_LENGTH`] / `variance` / 2 long, each
/// next one twice as wide and long, `2^(-strength / 8)` times as strong as the one before (a
/// low strength weighs the broad weave about as much as the fine fibers, a high one the
/// finest only).
pub fn fibers(p: [f64; 2], variance: f64, strength: f64, seed: u32) -> f64 {
    let length = FIBERS_LENGTH / variance.max(1.0) / 2.0;
    let ratio = (-strength / 8.0).exp2();
    let (mut sum, mut total, mut amplitude) = (0.0, 0.0, 1.0);
    for octave in 0..FIBERS_OCTAVES {
        let grow = f64::from(1u32 << octave);
        let (across, along) = (2.0 * grow, length * grow);
        sum += amplitude * gradient_noise([p[0] / across, p[1] / along], seed, octave);
        total += amplitude;
        amplitude *= ratio;
    }
    (0.5 + sum / total * FIBERS_CONTRAST).clamp(0.0, 1.0)
}

/// Rec. 709 luma of a straight color.
fn luma(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Shadows/Highlights on straight color `o`, whose neighbourhoods' lumas are `around` (the
/// shadows' blur, the highlights'): `[shadow amount, shadow tone, highlight amount, highlight
/// tone]` in parts of 1, `color` and `midtone` in -1 to 1.
///
/// A neighbourhood among the darkest `tone` of tones weighs from 1 (black) down to 0 (at
/// `tone`), smoothly; the pixel's luma moves by `amount × weight × tone ×` [`SHADOWS_LIFT`], up
/// for the shadows, down for the highlights: the pixel and its neighbours by as much, so its
/// detail stays. Midtone then steepens (or flattens) the luma around middle gray, the midtones
/// most; Color scales the color's distance to its luma by how much the luma moved.
fn shadows_highlights(
    o: [f64; 3],
    around: [f64; 2],
    [sa, st, ha, ht]: [f64; 4],
    color: f64,
    midtone: f64,
) -> [f64; 3] {
    let smooth = |x: f64| {
        let x = x.clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    };
    let weight = |distance: f64, tone: f64| {
        if tone <= 0.0 {
            0.0
        } else {
            smooth(1.0 - distance / tone)
        }
    };
    let y = luma(o);
    let lift = sa * weight(around[0].max(0.0), st) * st * SHADOWS_LIFT;
    let lower = ha * weight((1.0 - around[1]).max(0.0), ht) * ht * SHADOWS_LIFT;
    let moved = (y + lift - lower).clamp(0.0, 1.0);
    // Monotonic for -1 to 1: the slope 1 + midtone (6 y (1 - y) - 1) is never negative.
    let target = moved + midtone * (moved - 0.5) * 2.0 * moved * (1.0 - moved);
    let gain = (1.0 + 4.0 * color * (target - y).abs()).max(0.0);
    std::array::from_fn(|i| target + (o[i] - y) * gain)
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

    /// Maximum's (`largest`) or Minimum's extremes of the square of `radius` whole pixels,
    /// exact whatever the radius; or of the disk (`round`): up to [`ROUND_UP_TO`] directly,
    /// beyond on the layer reduced (each reduced pixel its block's extreme) so that the radius
    /// is at most that.
    pub(crate) fn extreme(radius: f64, largest: bool, round: bool) -> Self {
        let factor = if !round || radius <= ROUND_UP_TO {
            1
        } else {
            1usize << (radius / ROUND_UP_TO).log2().ceil() as u32
        };
        Self {
            factor,
            kernel: Kernel::Extreme {
                radius: (radius / factor as f64).round().max(1.0) as usize,
                largest,
                round,
            },
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
    /// The largest (or smallest) value of the square of `radius` around each pixel, or of the
    /// disk (`round`), channel by channel.
    Extreme {
        radius: usize,
        largest: bool,
        round: bool,
    },
    /// Custom's: the 5 × 5 pixels around each one weighted (row by row from the top left),
    /// summed.
    Convolve(Box<[f64; CUSTOM_WEIGHTS]>),
    /// Reduce Noise's and Despeckle's smoothing (see [`denoise`]).
    Denoise(Denoise),
    /// The magnitude of the Sobel gradient of each channel, a full step 1.
    Sobel,
    /// The difference of the pixels `offset` after and before each one (read bilinearly).
    Relief {
        offset: [f64; 2],
    },
    /// The average of the square cell of this side each pixel lies in, cells from the layer's
    /// origin (see [`Kernel::region_at`]).
    Cells(f64),
    /// Surface Blur's average of the square of `radius` around each pixel, channel by channel,
    /// each value weighted by `1 - |value - pixel's| / (2.5 threshold)` (none below 0).
    Surface {
        radius: usize,
        threshold: f64,
    },
    /// Smart Blur's average of the pixels of the disk of `radius` around each pixel that differ
    /// from it by less than `threshold` on every channel (at most `taps` read a side), or its
    /// edges, as `mode` says (see [`Filter::SmartBlur`]).
    Smart {
        radius: f64,
        threshold: f64,
        taps: usize,
        mode: SmartMode,
    },
    /// Diffuse's: each pixel one of its eight neighbours drawn from `seed` and its place on the
    /// layer, as `mode` says.
    Diffuse {
        mode: DiffuseMode,
        seed: u32,
    },
    /// Wind's streaks along the rows, at most `length` pixels from their edge (see
    /// [`wind`]).
    Wind {
        length: usize,
        strength: f64,
        stagger: bool,
        from_left: bool,
        seed: u32,
    },
    /// Trace Contour's lines where a channel crosses `level` (0 to 1, see [`contour_level`]),
    /// on the pixels above it (`upper`) or below, white elsewhere.
    Contour {
        level: f64,
        upper: bool,
    },
    Oil(Oil),
    /// Color Halftone's dots (see [`halftone`]), cells from the layer's origin.
    Halftone(Halftone),
    /// Fragment's four copies this many pixels from each pixel diagonally (read bilinearly),
    /// averaged.
    Fragment(f64),
    /// Mezzotint's pattern of `kind` drawn from `seed` (see [`mezzotint`]), each pixel
    /// `reduction` of the layer's a side.
    Mezzotint {
        kind: MezzotintType,
        seed: u32,
        reduction: usize,
    },
    Identity,
}

/// Color Halftone's computation (see [`halftone`]): each channel's screen, square cells of
/// `side` pixels along `axes` (unit vectors), the dots read from the layer blurred by a box of
/// `blur` pixels (about half a cell).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Halftone {
    side: f64,
    axes: [[[f64; 2]; 2]; 3],
    blur: usize,
}

impl Halftone {
    /// Dots of Max Radius `radius` (pixels), the screens turned by `angles` degrees
    /// (counterclockwise, up being negative y).
    fn new(radius: f64, angles: [f32; 3]) -> Self {
        let side = radius * std::f64::consts::SQRT_2;
        Self {
            side,
            axes: angles.map(|a| {
                let (sin, cos) = f64::from(a).to_radians().sin_cos();
                [[cos, -sin], [sin, cos]]
            }),
            blur: (side / 2.0).round() as usize,
        }
    }

    /// The largest dot's radius, in pixels: one covering its cell.
    fn radius(&self) -> f64 {
        self.side * std::f64::consts::FRAC_1_SQRT_2
    }

    /// A pixel reads the dots whose center is within the largest radius (and half a pixel),
    /// each read between pixels from the layer blurred.
    fn reach(&self) -> usize {
        (self.radius() + 0.5).ceil() as usize + self.blur + 2
    }
}

/// Oil Paint's computation (see [`oil`]): a Kuwahara filter of `radius`, a Gaussian blur of
/// `clean` (none below 0.3), then the relief lit from `light` (a unit direction) by `relief`,
/// its lightness grained by `bristle` (0 to 1) gradient noise of 1.5 pixels at full size
/// (`grain`: how many of the layer's pixels a pixel is).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Oil {
    radius: usize,
    clean: f64,
    bristle: f64,
    grain: f64,
    light: Option<[f64; 2]>,
    relief: f64,
}

impl Oil {
    fn reach(&self) -> usize {
        self.radius + (3.0 * self.clean).ceil() as usize + 2
    }
}

/// Reduce Noise's computation (see [`denoise`]): the lightness smoothed over boxes of `radius`
/// where its variance is small against `eps`, the colors without it likewise over
/// `color_radius` and `color_eps` (no smoothing at a radius of 0), then the lightness sharpened
/// by `sharpen` (its difference with a 3 × 3 box's average).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Denoise {
    radius: usize,
    eps: f64,
    color_radius: usize,
    color_eps: f64,
    sharpen: f64,
}

impl Denoise {
    /// A guided filter reads two boxes away; sharpening one pixel more.
    fn reach(&self) -> usize {
        (2 * self.radius + usize::from(self.sharpen > 0.0)).max(2 * self.color_radius)
    }
}

impl Kernel {
    /// How far a pixel's result reads, in pixels on each side.
    pub(crate) fn reach(&self) -> usize {
        match self {
            Self::Gaussian(blur) => blur.reach(),
            Self::Line(line) => line.reach(),
            Self::Median(radius) | Self::Extreme { radius, .. } | Self::Surface { radius, .. } => {
                *radius
            }
            Self::Convolve(_) => 2,
            Self::Denoise(denoise) => denoise.reach(),
            Self::Smart { radius, .. } => radius.ceil() as usize + 1,
            Self::Diffuse { .. } | Self::Contour { .. } => 1,
            Self::Wind { length, .. } => *length,
            Self::Oil(oil) => oil.reach(),
            Self::Halftone(halftone) => halftone.reach(),
            Self::Fragment(offset) => offset.ceil() as usize + 1,
            Self::Mezzotint { .. } => 0,
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
            &Self::Diffuse { mode, seed } => diffuse(region, width, height, mode, seed, origin),
            &Self::Wind {
                length,
                strength,
                stagger,
                from_left,
                seed,
            } => wind(
                region,
                width,
                [length as i64, i64::from(from_left)],
                strength,
                stagger,
                seed,
                origin,
            ),
            Self::Oil(oil) => self::oil(region, width, height, oil, origin),
            Self::Halftone(screen) => halftone(region, width, height, screen, origin),
            &Self::Mezzotint {
                kind,
                seed,
                reduction,
            } => mezzotint(region, width, height, kind, seed, reduction, origin),
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
            &Self::Extreme {
                radius,
                largest,
                round: false,
            } => extreme(region, width, height, [radius; 2], largest),
            &Self::Extreme {
                radius,
                largest,
                round: true,
            } => round_extreme(region, width, height, radius, largest),
            Self::Convolve(weights) => convolve(region, width, height, weights),
            Self::Denoise(denoise) => self::denoise(region, width, height, denoise),
            Self::Sobel => sobel(region, width, height),
            &Self::Relief { offset } => relief(region, width, height, offset),
            &Self::Cells(cell) => cells(region, width, height, cell, [0, 0], [width, height]),
            &Self::Surface { radius, threshold } => {
                surface(region, width, height, radius, threshold);
            }
            &Self::Smart {
                radius,
                threshold,
                taps,
                mode,
            } => smart(region, width, height, radius, threshold, taps, mode),
            Self::Diffuse { .. }
            | Self::Wind { .. }
            | Self::Oil(_)
            | Self::Halftone(_)
            | Self::Mezzotint { .. } => {
                self.region_at(region, width, height, [0, 0], [width, height]);
            }
            &Self::Fragment(offset) => fragment(region, width, height, offset),
            &Self::Contour { level, upper } => contour(region, width, height, level, upper),
            Self::Identity => {}
        }
    }

    /// `image` (`width` × `height` pixels) blurred, its edges repeating outward, on every core.
    pub(crate) fn image(&self, image: Vec<[f32; 4]>, width: usize, height: usize) -> Vec<[f32; 4]> {
        match self {
            Self::Gaussian(blur) => blur.image(image, width, height),
            Self::Identity => image,
            // Never reduced (no plan of a factor above 1 takes them, but a large Roundness, then
            // small): one thread is enough.
            Self::Extreme { .. }
            | Self::Convolve(_)
            | Self::Denoise(_)
            | Self::Sobel
            | Self::Relief { .. }
            | Self::Cells(_)
            | Self::Surface { .. }
            | Self::Smart { .. }
            | Self::Diffuse { .. }
            | Self::Wind { .. }
            | Self::Contour { .. }
            | Self::Oil(_)
            | Self::Halftone(_)
            | Self::Fragment(_)
            | Self::Mezzotint { .. } => {
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
/// of the rectangle `across` pixels to each side and `down` up and down around it, channel by
/// channel, its edges repeating outward. A row pass then a column pass, each exact in three
/// comparisons a pixel whatever the radius (van
/// Herk and Gil–Werman: running extremes forward and backward within windows of the square's
/// side, each window then spanning at most two of them).
fn extreme(
    region: &mut [[f32; 4]],
    width: usize,
    height: usize,
    [across, down]: [usize; 2],
    largest: bool,
) {
    if width == 0 || height == 0 {
        return;
    }
    let mut buffers = ExtremeLine::default();
    if across > 0 {
        for row in region.chunks_mut(width) {
            buffers.line(row, across, largest);
        }
    }
    if down == 0 {
        return;
    }
    let mut column = vec![[0.0f32; 4]; height];
    for x in 0..width {
        for (y, px) in column.iter_mut().enumerate() {
            *px = region[y * width + x];
        }
        buffers.line(&mut column, down, largest);
        for (y, px) in column.iter().enumerate() {
            region[y * width + x] = *px;
        }
    }
}

/// The rectangles whose union is the disk of `radius` (the pixels within `radius` of the
/// center): their half widths and half heights, each the widest of its height.
fn disk_rectangles(radius: usize) -> Vec<[usize; 2]> {
    let r2 = radius * radius;
    let half = |dy: usize| (r2 - dy * dy).isqrt();
    (0..=radius)
        .filter(|&dy| dy == radius || half(dy + 1) < half(dy))
        .map(|dy| [half(dy), dy])
        .collect()
}

/// `region` (`width` × `height`) in place: each pixel the largest (`largest`) or smallest value
/// of the disk of `radius` around it, channel by channel, its edges repeating outward. Exact:
/// the disk the union of rectangles (see [`disk_rectangles`]), each one's extremes rows then
/// columns as [`extreme`]'s.
fn round_extreme(
    region: &mut [[f32; 4]],
    width: usize,
    height: usize,
    radius: usize,
    largest: bool,
) {
    if width == 0 || height == 0 || radius == 0 {
        return;
    }
    let source = region.to_vec();
    let mut rectangle = Vec::with_capacity(source.len());
    for (n, half) in disk_rectangles(radius).into_iter().enumerate() {
        let out: &mut [[f32; 4]] = if n == 0 {
            region.copy_from_slice(&source);
            &mut *region
        } else {
            rectangle.clear();
            rectangle.extend_from_slice(&source);
            &mut rectangle
        };
        extreme(out, width, height, half, largest);
        if n > 0 {
            for (px, v) in region.iter_mut().zip(&rectangle) {
                *px = std::array::from_fn(|c| {
                    if largest {
                        px[c].max(v[c])
                    } else {
                        px[c].min(v[c])
                    }
                });
            }
        }
    }
}

/// `region` (`width` × `height`) in place: each pixel the sum of the 5 × 5 pixels around it
/// weighted by `weights` (row by row from the top left), the edges repeating outward.
fn convolve(region: &mut [[f32; 4]], width: usize, height: usize, weights: &[f64; CUSTOM_WEIGHTS]) {
    if width == 0 || height == 0 {
        return;
    }
    let source = region.to_vec();
    // Only the weights that count.
    let taps: Vec<(i64, i64, f64)> = weights
        .iter()
        .enumerate()
        .filter(|(_, w)| **w != 0.0)
        .map(|(i, &w)| (i as i64 % 5 - 2, i as i64 / 5 - 2, w))
        .collect();
    let (w, h) = (width as i64, height as i64);
    for y in 0..h {
        for x in 0..w {
            let mut sum = [0.0f64; 4];
            for &(dx, dy, weight) in &taps {
                let sx = (x + dx).clamp(0, w - 1) as usize;
                let sy = (y + dy).clamp(0, h - 1) as usize;
                let px = source[sy * width + sx];
                for c in 0..4 {
                    sum[c] += weight * f64::from(px[c]);
                }
            }
            region[y as usize * width + x as usize] = sum.map(|v| v as f32);
        }
    }
}

/// The average of the box of `radius` around each value of `plane` (`width` × `height`), the
/// edges repeating outward: running sums along the rows, then down the columns.
fn box_plane(plane: &[f32], width: usize, height: usize, radius: usize) -> Vec<f32> {
    if radius == 0 || width == 0 || height == 0 {
        return plane.to_vec();
    }
    let r = radius as i64;
    let side = (2 * radius + 1) as f64;
    let mut rows = vec![0.0f32; plane.len()];
    for (src, out) in plane.chunks(width).zip(rows.chunks_mut(width)) {
        let at = |i: i64| f64::from(src[i.clamp(0, width as i64 - 1) as usize]);
        let mut sum: f64 = (-r..=r).map(at).sum();
        for (i, v) in out.iter_mut().enumerate() {
            *v = (sum / side) as f32;
            sum += at(i as i64 + r + 1) - at(i as i64 - r);
        }
    }
    let mut out = vec![0.0f32; plane.len()];
    let row = |y: i64| y.clamp(0, height as i64 - 1) as usize * width;
    let mut sums: Vec<f64> = vec![0.0; width];
    for dy in -r..=r {
        let start = row(dy);
        for (s, v) in sums.iter_mut().zip(&rows[start..start + width]) {
            *s += f64::from(*v);
        }
    }
    for y in 0..height {
        let line = &mut out[y * width..(y + 1) * width];
        for (o, s) in line.iter_mut().zip(&sums) {
            *o = (s / side) as f32;
        }
        let (add, sub) = (row(y as i64 + r + 1), row(y as i64 - r));
        for (x, s) in sums.iter_mut().enumerate() {
            *s += f64::from(rows[add + x]) - f64::from(rows[sub + x]);
        }
    }
    out
}

/// `plane` smoothed where it varies little (a guided filter guided by itself, He et al.): over
/// each box of `radius`, the values pulled toward their average by `eps / (variance + eps)`;
/// those pulls averaged in turn, so that a box across an edge (a variance far above `eps`)
/// leaves it as it is.
fn guided(plane: &[f32], width: usize, height: usize, radius: usize, eps: f64) -> Vec<f32> {
    if radius == 0 || eps <= 0.0 {
        return plane.to_vec();
    }
    let mean = box_plane(plane, width, height, radius);
    let squares: Vec<f32> = plane.iter().map(|v| v * v).collect();
    let mean_squares = box_plane(&squares, width, height, radius);
    let (mut a, mut b) = (vec![0.0f32; plane.len()], vec![0.0f32; plane.len()]);
    for i in 0..plane.len() {
        let m = f64::from(mean[i]);
        let variance = (f64::from(mean_squares[i]) - m * m).max(0.0);
        let k = variance / (variance + eps);
        a[i] = k as f32;
        b[i] = (m - k * m) as f32;
    }
    let (a, b) = (
        box_plane(&a, width, height, radius),
        box_plane(&b, width, height, radius),
    );
    plane
        .iter()
        .zip(a.iter().zip(&b))
        .map(|(v, (a, b))| a * v + b)
        .collect()
}

/// Rec. 601's weights of red, green and blue in the lightness Reduce Noise smooths.
const LUMA: [f32; 3] = [0.299, 0.587, 0.114];

/// `region` (`width` × `height`, premultiplied) in place, as Reduce Noise and Despeckle smooth
/// it: its lightness and its colors without it (blue and red minus the lightness, linear in
/// the premultiplied values) smoothed apart (see [`guided`]), the lightness then sharpened
/// against its 3 × 3 average; alpha kept.
fn denoise(region: &mut [[f32; 4]], width: usize, height: usize, denoise: &Denoise) {
    if width == 0 || height == 0 {
        return;
    }
    let luma: Vec<f32> = region
        .iter()
        .map(|p| LUMA[0] * p[0] + LUMA[1] * p[1] + LUMA[2] * p[2])
        .collect();
    let mut y = guided(&luma, width, height, denoise.radius, denoise.eps);
    if denoise.sharpen > 0.0 {
        let k = denoise.sharpen as f32;
        let soft = box_plane(&y, width, height, 1);
        for (v, s) in y.iter_mut().zip(&soft) {
            *v += k * (*v - s);
        }
    }
    let color = |c: usize| -> Vec<f32> {
        let plane: Vec<f32> = region.iter().zip(&luma).map(|(p, l)| p[c] - l).collect();
        guided(
            &plane,
            width,
            height,
            denoise.color_radius,
            denoise.color_eps,
        )
    };
    let (blue, red) = (color(2), color(0));
    for (i, px) in region.iter_mut().enumerate() {
        let (r, b) = (red[i] + y[i], blue[i] + y[i]);
        let g = (y[i] - LUMA[0] * r - LUMA[2] * b) / LUMA[1];
        *px = [r, g, b, px[3]];
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
    if width == 0 || height == 0 {
        return;
    }
    let source = region.to_vec();
    let at = |x: f64, y: f64| between(&source, width, height, x, y);
    let [dx, dy] = offset;
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f64, y as f64);
            let (after, before) = (at(fx + dx, fy + dy), at(fx - dx, fy - dy));
            region[y * width + x] = std::array::from_fn(|c| (after[c] - before[c]) as f32);
        }
    }
}

/// `source` (`width` × `height`, not empty) at (`x`, `y`) between its pixels' centers (whole
/// numbers), bilinearly, its edges repeating outward.
fn between(source: &[[f32; 4]], width: usize, height: usize, x: f64, y: f64) -> [f64; 4] {
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
}

/// `region` (`width` × `height`) in place: Fragment ([`Kernel::Fragment`]), each pixel the
/// average of the four pixels `offset` from it diagonally, read bilinearly, the edges repeating
/// outward.
fn fragment(region: &mut [[f32; 4]], width: usize, height: usize, offset: f64) {
    if width == 0 || height == 0 {
        return;
    }
    let source = region.to_vec();
    let d = offset;
    for y in 0..height {
        for x in 0..width {
            let (fx, fy) = (x as f64, y as f64);
            let mut sum = [0.0f64; 4];
            for [sx, sy] in [[-1.0, -1.0], [1.0, -1.0], [-1.0, 1.0], [1.0, 1.0]] {
                let v = between(&source, width, height, fx + sx * d, fy + sy * d);
                for c in 0..4 {
                    sum[c] += v[c];
                }
            }
            region[y * width + x] = sum.map(|v| (v / 4.0) as f32);
        }
    }
}

/// How many steps of a channel's value Color Halftone's table of dot radii has.
const DOT_STEPS: usize = 256;

/// Color Halftone's dot radius for each value of [`DOT_STEPS`], in parts of its cell's side:
/// the disk centered in the unit square that covers `value` of it (computed once).
fn dot_radii() -> &'static [f64; DOT_STEPS + 1] {
    static RADII: std::sync::OnceLock<[f64; DOT_STEPS + 1]> = std::sync::OnceLock::new();
    RADII.get_or_init(|| {
        // The part of the unit square the centered disk of radius `r` covers.
        let covered = |r: f64| {
            if r <= 0.5 {
                std::f64::consts::PI * r * r
            } else if r < std::f64::consts::FRAC_1_SQRT_2 {
                // The disk less its four caps beyond the square's sides.
                let cap = r * r * (0.5 / r).acos() - 0.5 * (r * r - 0.25).sqrt();
                std::f64::consts::PI * r * r - 4.0 * cap
            } else {
                1.0
            }
        };
        std::array::from_fn(|i| {
            let value = i as f64 / DOT_STEPS as f64;
            // None, or the whole square (its corners: the disk through them).
            if i == 0 {
                return 0.0;
            } else if i == DOT_STEPS {
                return std::f64::consts::FRAC_1_SQRT_2;
            }
            let (mut low, mut high) = (0.0, std::f64::consts::FRAC_1_SQRT_2);
            for _ in 0..48 {
                let mid = (low + high) / 2.0;
                if covered(mid) < value {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            high
        })
    })
}

/// The dot radius for `value` (0 to 1), in parts of its cell's side (see [`dot_radii`]).
fn dot_radius(value: f64) -> f64 {
    let radii = dot_radii();
    let v = value.clamp(0.0, 1.0) * DOT_STEPS as f64;
    let i = (v.floor() as usize).min(DOT_STEPS - 1);
    radii[i] + (radii[i + 1] - radii[i]) * (v - i as f64)
}

/// `region` (`width` × `height`, its first pixel pixel `origin` of the layer) in place: Color
/// Halftone ([`Kernel::Halftone`]), the edges repeating outward.
///
/// For each channel, the screen's cells around the pixel (its own and the eight around it, the
/// screen's cells being the Voronoi cells of their centers): each one's dot of the radius
/// covering as much of its cell as the channel's value at its center (the layer blurred by a
/// box of about half a cell), anti-aliased over a pixel; the channel full where a dot covers
/// the pixel, none elsewhere, the pixel keeping its opacity.
fn halftone(
    region: &mut [[f32; 4]],
    width: usize,
    height: usize,
    screen: &Halftone,
    origin: [i64; 2],
) {
    if width == 0 || height == 0 {
        return;
    }
    let mut blurred = region.to_vec();
    Blur::Boxes([screen.blur, 0, 0]).region(&mut blurred, width, height);
    let side = screen.side;
    let farthest = screen.radius() + 0.5;
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let alpha = region[i][3];
            if alpha <= 0.0 {
                continue;
            }
            // The pixel's center from the layer's origin.
            let p = [
                (origin[0] + x as i64) as f64 + 0.5,
                (origin[1] + y as i64) as f64 + 0.5,
            ];
            let mut out = [0.0f32; 3];
            for (ch, v) in out.iter_mut().enumerate() {
                let [e1, e2] = screen.axes[ch];
                let u = ((p[0] * e1[0] + p[1] * e1[1]) / side).floor();
                let w = ((p[0] * e2[0] + p[1] * e2[1]) / side).floor();
                let mut full = 0.0f64;
                for dj in -1..=1 {
                    for di in -1..=1 {
                        let (a, b) = (u + f64::from(di) + 0.5, w + f64::from(dj) + 0.5);
                        let c = [
                            side * (a * e1[0] + b * e2[0]),
                            side * (a * e1[1] + b * e2[1]),
                        ];
                        let d = ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt();
                        if d >= farthest {
                            continue;
                        }
                        let at = between(
                            &blurred,
                            width,
                            height,
                            c[0] - 0.5 - origin[0] as f64,
                            c[1] - 0.5 - origin[1] as f64,
                        );
                        let value = if at[3] > 0.0 { at[ch] / at[3] } else { 0.0 };
                        // A full dot covers its whole cell, the pixel's own (its corners too,
                        // where the dots only touch).
                        if value >= 1.0 && di == 0 && dj == 0 {
                            full = 1.0;
                            break;
                        }
                        let r = dot_radius(value) * side;
                        // Over a pixel, no more than the dot's area (a speck stays faint).
                        let k = (r + 0.5 - d)
                            .clamp(0.0, 1.0)
                            .min(std::f64::consts::PI * r * r);
                        full = full.max(k);
                    }
                }
                *v = full as f32;
            }
            region[i] = [out[0] * alpha, out[1] * alpha, out[2] * alpha, alpha];
        }
    }
}

/// `region` (`width` × `height`, its first pixel pixel `origin` of a level whose pixels are
/// `reduction` of the layer's a side) in place: Mezzotint ([`Kernel::Mezzotint`]). Each
/// channel of each of the layer's pixels full where its value is above the pattern's
/// threshold there, none elsewhere; a pixel of a reduced level the share of the layer's pixels
/// under it (up to [`MEZZOTINT_SAMPLES`] a side) that are full; each pixel keeping its opacity.
fn mezzotint(
    region: &mut [[f32; 4]],
    width: usize,
    height: usize,
    kind: MezzotintType,
    seed: u32,
    reduction: usize,
    origin: [i64; 2],
) {
    let n = reduction.clamp(1, MEZZOTINT_SAMPLES);
    let r = reduction.max(1) as i64;
    let samples = (n * n) as f32;
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let own = region[i];
            let alpha = own[3];
            if alpha <= 0.0 {
                continue;
            }
            let (lx, ly) = ((origin[0] + x as i64) * r, (origin[1] + y as i64) * r);
            let mut out = [0.0f32; 3];
            for (ch, v) in out.iter_mut().enumerate() {
                let value = f64::from(own[ch] / alpha);
                let mut full = 0;
                for sy in 0..n {
                    for sx in 0..n {
                        // Evenly spaced over the pixel's square of the layer.
                        let at = [
                            lx + (sx as i64 * r) / n as i64,
                            ly + (sy as i64 * r) / n as i64,
                        ];
                        full += usize::from(value > threshold(kind, at, seed, ch as u32));
                    }
                }
                *v = full as f32 / samples;
            }
            region[i] = [out[0] * alpha, out[1] * alpha, out[2] * alpha, alpha];
        }
    }
}

/// Mezzotint's threshold (0 to 1, uniform) for channel `channel` of layer pixel `at`: drawn for
/// each pixel (Fine Dots), for clumps of about 2 or 4 pixels (Medium, Coarse), for either
/// (Grainy, half and half), for runs of a row (Lines: 4, 12 or 32 pixels, each row's runs
/// starting at a place of their own) or of a diagonal down to the right (Strokes: 4, 8 or 16).
fn threshold(kind: MezzotintType, at: [i64; 2], seed: u32, channel: u32) -> f64 {
    use MezzotintType as T;
    let salt = 0x3e20 + channel * 16;
    let [x, y] = at;
    let clump = |size: f64| {
        let p = [x as f64 + 0.5, y as f64 + 0.5];
        // The point's draws take `salt + 6` and `salt + 7`.
        draw(nearest_square(p, size, seed, salt + 6, 0.5), seed, salt + 8)
    };
    let run = |length: i64| {
        let shift = (draw([0, y], seed, salt + 2) * length as f64) as i64;
        draw([(x + shift).div_euclid(length), y], seed, salt + 3)
    };
    let stroke = |length: i64| {
        let diagonal = x - y;
        let shift = (draw([diagonal, 0], seed, salt + 4) * length as f64) as i64;
        draw([diagonal, (y + shift).div_euclid(length)], seed, salt + 5)
    };
    match kind {
        T::FineDots => draw(at, seed, salt),
        T::MediumDots => clump(2.0),
        T::GrainyDots => {
            if draw(at, seed, salt + 1) < 0.5 {
                draw(at, seed, salt)
            } else {
                clump(2.0)
            }
        }
        T::CoarseDots => clump(4.0),
        T::ShortLines => run(4),
        T::MediumLines => run(12),
        T::LongLines => run(32),
        T::ShortStrokes => stroke(4),
        T::MediumStrokes => stroke(8),
        T::LongStrokes => stroke(16),
    }
}

/// The point drawn from `seed` and `salt` in square `square` of `cell` pixels from the layer's
/// origin (Crystallize's, Pointillize's, Mezzotint's clumps): anywhere in it (`jitter` 1), or
/// within its middle (`jitter` 0.5: half of it on each axis).
fn cell_point(square: [i64; 2], cell: f64, seed: u32, salt: u32, jitter: f64) -> [f64; 2] {
    let along = |axis: usize| {
        let u = 0.5 + (draw(square, seed, salt + axis as u32) - 0.5) * jitter;
        (square[axis] as f64 + u) * cell
    };
    [along(0), along(1)]
}

/// The square whose [`cell_point`] is closest to `p` (layer pixels), ties to the first row by
/// row. Exact: with points anywhere in their squares, the closest is no farther than the one of
/// `p`'s own square (a square's diagonal), so within two squares of it; with points in their
/// middle (`jitter` 0.5), within one.
fn nearest_square(p: [f64; 2], cell: f64, seed: u32, salt: u32, jitter: f64) -> [i64; 2] {
    let span = if jitter > 0.5 { 2 } else { 1 };
    let (ci, cj) = ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64);
    let mut best = (f64::MAX, [ci, cj]);
    for j in cj - span..=cj + span {
        for i in ci - span..=ci + span {
            let q = cell_point([i, j], cell, seed, salt, jitter);
            let d = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
            if d < best.0 {
                best = (d, [i, j]);
            }
        }
    }
    best.1
}

/// The center of the layer pixel `q` (layer pixels) lies in, within the layer of `layer`
/// pixels.
fn pixel_center(q: [f64; 2], layer: [f64; 2]) -> [f64; 2] {
    std::array::from_fn(|i| q[i].floor().clamp(0.0, (layer[i] - 1.0).max(0.0)) + 0.5)
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

/// `region` (`width` × `height`) in place: Surface Blur's weighted average of the square of
/// `radius` around each pixel ([`Kernel::Surface`]), the edges repeating outward. Beyond
/// [`SURFACE_TAPS`] pixels a side, the square's pixels evenly spaced (every `step`).
fn surface(region: &mut [[f32; 4]], width: usize, height: usize, radius: usize, threshold: f64) {
    if width == 0 || height == 0 || radius == 0 {
        return;
    }
    let source = region.to_vec();
    let step = radius.div_ceil(SURFACE_TAPS).max(1);
    let taps = (radius / step) as i64;
    let step = step as i64;
    // The weight falls by this much a unit of difference (2.5 thresholds: none).
    let fall = (1.0 / (2.5 * threshold)) as f32;
    let (w, h) = (width as i64, height as i64);
    let mut columns = Vec::with_capacity(2 * taps as usize + 1);
    for y in 0..h {
        let rows: Vec<usize> = (-taps..=taps)
            .map(|k| (y + k * step).clamp(0, h - 1) as usize * width)
            .collect();
        for x in 0..w {
            columns.clear();
            columns.extend((-taps..=taps).map(|k| (x + k * step).clamp(0, w - 1) as usize));
            let center = source[y as usize * width + x as usize];
            let mut sum = [0.0f32; 4];
            let mut total = [0.0f32; 4];
            for &row in &rows {
                for &column in &columns {
                    let px = source[row + column];
                    for c in 0..4 {
                        let weight = (1.0 - (px[c] - center[c]).abs() * fall).max(0.0);
                        sum[c] += weight * px[c];
                        total[c] += weight;
                    }
                }
            }
            // The pixel itself weighs 1: never nothing.
            region[y as usize * width + x as usize] = std::array::from_fn(|c| sum[c] / total[c]);
        }
    }
}

/// `region` (`width` × `height`) in place: Smart Blur ([`Kernel::Smart`]), the edges repeating
/// outward. The disk's pixels at whole offsets, every `step` beyond `taps` a side.
fn smart(
    region: &mut [[f32; 4]],
    width: usize,
    height: usize,
    radius: f64,
    threshold: f64,
    taps: usize,
    mode: SmartMode,
) {
    if width == 0 || height == 0 {
        return;
    }
    let source = region.to_vec();
    let reach = radius.floor() as usize;
    let step = reach.div_ceil(taps.max(1)).max(1);
    let n = (reach / step) as i64;
    let step = step as i64;
    let offsets: Vec<[i64; 2]> = (-n..=n)
        .flat_map(|j| (-n..=n).map(move |i| [i * step, j * step]))
        .filter(|[x, y]| ((x * x + y * y) as f64) <= radius * radius)
        .collect();
    let level = threshold as f32;
    let (w, h) = (width as i64, height as i64);
    let at = |x: i64, y: i64| source[(y.clamp(0, h - 1) * w + x.clamp(0, w - 1)) as usize];
    let close = |a: [f32; 4], b: [f32; 4]| (0..4).all(|c| (a[c] - b[c]).abs() < level);
    for y in 0..h {
        for x in 0..w {
            let center = at(x, y);
            let out = &mut region[(y * w + x) as usize];
            // An edge: a neighbor differs by the threshold or more.
            if mode != SmartMode::Normal {
                let edge = [[1, 0], [-1, 0], [0, 1], [0, -1]]
                    .iter()
                    .any(|[dx, dy]| !close(at(x + dx, y + dy), center));
                let a = center[3];
                if edge {
                    *out = [a, a, a, a];
                    continue;
                }
                if mode == SmartMode::EdgeOnly {
                    *out = [0.0, 0.0, 0.0, a];
                    continue;
                }
            }
            let mut sum = [0.0f32; 4];
            let mut count = 0.0f32;
            for [dx, dy] in &offsets {
                let px = at(x + dx, y + dy);
                if close(px, center) {
                    for c in 0..4 {
                        sum[c] += px[c];
                    }
                    count += 1.0;
                }
            }
            // The pixel itself is always close: never nothing.
            *out = sum.map(|v| v / count);
        }
    }
}

/// The eight neighbours of a pixel, clockwise from the left.
const NEIGHBOURS: [[i64; 2]; 8] = [
    [-1, 0],
    [-1, -1],
    [0, -1],
    [1, -1],
    [1, 0],
    [1, 1],
    [0, 1],
    [-1, 1],
];

/// Rec. 709 luma of a premultiplied value (its lightness over black).
fn luma_of(p: [f32; 4]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// A draw from 0 to 1 (24 bits) for layer pixel `at`, `seed` and `salt`.
fn draw(at: [i64; 2], seed: u32, salt: u32) -> f64 {
    let h = hash(at[0] as i32 as u32 ^ hash(at[1] as i32 as u32 ^ hash(seed ^ salt)));
    f64::from(h >> 8) / f64::from(1u32 << 24)
}

/// `region` (`width` × `height`, its first pixel pixel `origin` of the layer) in place:
/// Diffuse ([`Kernel::Diffuse`]), the edges repeating outward. Each pixel's neighbour drawn
/// from its place on the layer, so that tiles and their margins agree.
fn diffuse(
    region: &mut [[f32; 4]],
    width: usize,
    height: usize,
    mode: DiffuseMode,
    seed: u32,
    origin: [i64; 2],
) {
    if width == 0 || height == 0 {
        return;
    }
    let source = region.to_vec();
    let at = |x: usize, y: usize, [dx, dy]: [i64; 2]| {
        let x = (x as i64 + dx).clamp(0, width as i64 - 1) as usize;
        let y = (y as i64 + dy).clamp(0, height as i64 - 1) as usize;
        source[y * width + x]
    };
    for y in 0..height {
        for x in 0..width {
            let own = source[y * width + x];
            let pick = (draw([origin[0] + x as i64, origin[1] + y as i64], seed, 0xd1f) * 8.0)
                as usize
                % 8;
            let other = at(x, y, NEIGHBOURS[pick]);
            region[y * width + x] = match mode {
                DiffuseMode::Normal => other,
                DiffuseMode::DarkenOnly if luma_of(other) < luma_of(own) => other,
                DiffuseMode::LightenOnly if luma_of(other) > luma_of(own) => other,
                DiffuseMode::DarkenOnly | DiffuseMode::LightenOnly => own,
                // The closest neighbour in color, ties to the first from the one drawn.
                DiffuseMode::Anisotropic => {
                    let differs = |p: [f32; 4]| (0..4).map(|c| (p[c] - own[c]).abs()).sum::<f32>();
                    (0..8)
                        .map(|i| at(x, y, NEIGHBOURS[(pick + i) % 8]))
                        .fold((f32::MAX, own), |(best, chosen), p| {
                            let d = differs(p);
                            if d < best { (d, p) } else { (best, chosen) }
                        })
                        .1
                }
            };
        }
    }
}

/// `region` (`width` pixels a row, its first pixel pixel `origin` of the layer) in place:
/// Wind ([`Kernel::Wind`]), `[length, from_left]` its longest streak and its direction. A pixel
/// whose luma differs from its neighbour downwind by [`WIND_EDGE`] or more blows a streak of its
/// color downwind, a quarter of `length` to all of it long (drawn from its place and `seed`),
/// covering what it crosses by `strength`, less and less to its end (or, `stagger`, as much
/// all along, starting up to half its length further). Each pixel takes the strongest streak
/// over it.
fn wind(
    region: &mut [[f32; 4]],
    width: usize,
    [length, from_left]: [i64; 2],
    strength: f64,
    stagger: bool,
    seed: u32,
    origin: [i64; 2],
) {
    if width == 0 {
        return;
    }
    // Where the wind comes from, one pixel along the row.
    let up: i64 = if from_left == 1 { -1 } else { 1 };
    let n = width as i64;
    let mut streaks = vec![(0.0f64, 0.0f64); width];
    for (y, row) in region.chunks_mut(width).enumerate() {
        let source = row.to_vec();
        let ly = origin[1] + y as i64;
        for (x, streak) in streaks.iter_mut().enumerate() {
            let down = x as i64 - up;
            *streak = (0.0, 0.0);
            if (0..n).contains(&down)
                && (luma_of(source[x]) - luma_of(source[down as usize])).abs() >= WIND_EDGE
            {
                let at = [origin[0] + x as i64, ly];
                let len = length as f64 * (0.25 + 0.75 * draw(at, seed, 0x3a1));
                let start = if stagger {
                    len * 0.5 * draw(at, seed, 0x3a2)
                } else {
                    0.0
                };
                *streak = (len, start);
            }
        }
        for (x, out) in row.iter_mut().enumerate() {
            let own = source[x];
            let (mut best, mut color) = (0.0, own);
            for k in 1..=length {
                let s = x as i64 + up * k;
                if !(0..n).contains(&s) {
                    break;
                }
                let (len, start) = streaks[s as usize];
                let d = k as f64 - start;
                if len <= 0.0 || d < 0.0 || d > len {
                    continue;
                }
                let w = if stagger {
                    strength
                } else {
                    strength * (1.0 - d / (len + 1.0))
                };
                if w > best {
                    (best, color) = (w, source[s as usize]);
                }
            }
            let w = best as f32;
            *out = std::array::from_fn(|c| own[c] + (color[c] - own[c]) * w);
        }
    }
}

/// `region` (`width` × `height`) in place: Trace Contour ([`Kernel::Contour`]), the edges
/// repeating outward. A channel of a pixel on the side outlined (above `level` for `upper`,
/// else at or below it) with a neighbour (of four) on the other side is 0, every other one
/// white; each pixel keeps its opacity.
fn contour(region: &mut [[f32; 4]], width: usize, height: usize, level: f64, upper: bool) {
    if width == 0 || height == 0 {
        return;
    }
    let source = region.to_vec();
    let above = |p: [f32; 4], c: usize| p[3] > 0.0 && f64::from(p[c] / p[3]) > level;
    for y in 0..height {
        for x in 0..width {
            let own = source[y * width + x];
            if own[3] <= 0.0 {
                continue;
            }
            let near = [[-1i64, 0], [1, 0], [0, -1], [0, 1]].map(|[dx, dy]| {
                let x = (x as i64 + dx).clamp(0, width as i64 - 1) as usize;
                let y = (y as i64 + dy).clamp(0, height as i64 - 1) as usize;
                source[y * width + x]
            });
            let mut out = [own[3]; 4];
            for (c, v) in out.iter_mut().enumerate().take(3) {
                let side = above(own, c);
                if side == upper && near.iter().any(|&n| above(n, c) != side) {
                    *v = 0.0;
                }
            }
            region[y * width + x] = out;
        }
    }
}

/// `region` (`width` × `height`, its first pixel pixel `origin` of the layer) in place: Oil
/// Paint ([`Kernel::Oil`]), the edges repeating outward.
///
/// Each pixel the average of the quadrant of `radius` around it (of four, each including it)
/// whose luma varies least (a Kuwahara filter, from summed-area tables: strokes that keep
/// their edges); blurred by `clean`; then, with a light, its relief (the result's luma, plus
/// `bristle` gradient noise) shades it: lighter where the paint faces the light.
fn oil(region: &mut [[f32; 4]], width: usize, height: usize, oil: &Oil, origin: [i64; 2]) {
    if width == 0 || height == 0 {
        return;
    }
    // Summed-area tables (a row and a column of zeros first) of the four values, the luma and
    // its square.
    let (w1, h1) = (width + 1, height + 1);
    let mut sums = vec![[0.0f64; 6]; w1 * h1];
    for y in 0..height {
        let mut row = [0.0f64; 6];
        for x in 0..width {
            let p = region[y * width + x];
            let l = f64::from(luma_of(p));
            let v = [p[0], p[1], p[2], p[3]].map(f64::from);
            for (c, s) in row.iter_mut().enumerate() {
                *s += match c {
                    0..=3 => v[c],
                    4 => l,
                    _ => l * l,
                };
            }
            let above = sums[y * w1 + x + 1];
            sums[(y + 1) * w1 + x + 1] = std::array::from_fn(|c| above[c] + row[c]);
        }
    }
    let r = oil.radius as i64;
    let box_sum = |x0: i64, y0: i64, x1: i64, y1: i64| {
        // Inclusive bounds clamped to the region.
        let (x0, y0) = (x0.max(0) as usize, y0.max(0) as usize);
        let (x1, y1) = (
            (x1.min(width as i64 - 1) + 1) as usize,
            (y1.min(height as i64 - 1) + 1) as usize,
        );
        let n = ((x1 - x0) * (y1 - y0)) as f64;
        let s: [f64; 6] = std::array::from_fn(|c| {
            sums[y1 * w1 + x1][c] - sums[y0 * w1 + x1][c] - sums[y1 * w1 + x0][c]
                + sums[y0 * w1 + x0][c]
        });
        (s, n)
    };
    let mut painted = vec![[0.0f32; 4]; width * height];
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            let mut best = (f64::MAX, [0.0f64; 6], 1.0);
            for (x0, y0) in [(x - r, y - r), (x, y - r), (x - r, y), (x, y)] {
                let (s, n) = box_sum(x0, y0, x0 + r, y0 + r);
                let mean = s[4] / n;
                let variance = s[5] / n - mean * mean;
                // Ties (a smooth gradient) to the first quadrant whatever the sums' rounding,
                // which depends on where the region starts.
                if variance < best.0 - OIL_TIE {
                    best = (variance, s, n);
                }
            }
            let (_, s, n) = best;
            painted[y as usize * width + x as usize] = std::array::from_fn(|c| (s[c] / n) as f32);
        }
    }
    if oil.clean >= 0.3 {
        Blur::gaussian(oil.clean).region(&mut painted, width, height);
    }
    let Some(light) = oil.light.filter(|_| oil.relief > 0.0) else {
        region.copy_from_slice(&painted);
        return;
    };
    let relief: Vec<f64> = (0..width * height)
        .map(|i| {
            let (x, y) = ((i % width) as i64, (i / width) as i64);
            let at = [
                (origin[0] + x) as f64 * oil.grain / 1.5,
                (origin[1] + y) as f64 * oil.grain / 1.5,
            ];
            f64::from(luma_of(painted[i])) + oil.bristle * 0.1 * gradient_noise(at, 0, 0)
        })
        .collect();
    let height_at = |x: i64, y: i64| {
        relief
            [y.clamp(0, height as i64 - 1) as usize * width + x.clamp(0, width as i64 - 1) as usize]
    };
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            let i = y as usize * width + x as usize;
            let g = [
                (height_at(x + 1, y) - height_at(x - 1, y)) / 2.0,
                (height_at(x, y + 1) - height_at(x, y - 1)) / 2.0,
            ];
            // A slope rising toward the light faces away from it.
            let shade = -(g[0] * light[0] + g[1] * light[1]) * oil.relief;
            let p = painted[i];
            let lift = (shade * f64::from(p[3])) as f32;
            region[i] = [p[0] + lift, p[1] + lift, p[2] + lift, p[3]];
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
    fn shadows_highlights_moves_dark_and_light_neighbourhoods_keeping_detail() {
        let filter =
            |shadows: [f32; 3], highlights: [f32; 3], color, midtone| Filter::ShadowsHighlights {
                shadows,
                highlights,
                color,
                midtone,
                scale: 1.0,
            };
        let gray = |v: f64| [v, v, v, 1.0];
        let plain = |f: Filter, o: f64, around: f64| f.finish(gray(o), &[gray(around); 2], [0, 0]);
        let off = filter([0.0, 50.0, 30.0], [0.0, 50.0, 30.0], 0.0, 0.0);
        assert_eq!(plain(off, 0.3, 0.2), gray(0.3), "nothing at 0");
        // Shadows: black in a black neighbourhood lifted to half the Tone at 100 %.
        let shadows = filter([100.0, 50.0, 30.0], [0.0, 50.0, 30.0], 0.0, 0.0);
        assert!((plain(shadows, 0.0, 0.0)[0] - 0.25).abs() < 1e-12);
        // Less and less up to the Tone; beyond, nothing.
        let (dark, darker) = (plain(shadows, 0.3, 0.3)[0], plain(shadows, 0.3, 0.1)[0]);
        assert!(0.3 < dark && dark < darker, "{dark} {darker}");
        assert_eq!(plain(shadows, 0.6, 0.6), gray(0.6));
        // The pixel moves with its neighbourhood: its difference with it stays.
        let (a, b) = (plain(shadows, 0.05, 0.1)[0], plain(shadows, 0.15, 0.1)[0]);
        assert!((b - a - 0.1).abs() < 1e-12, "{a} {b}");
        // Highlights: white lowered as much, the shadows' blur not read for it.
        let highlights = filter([0.0, 50.0, 30.0], [100.0, 50.0, 30.0], 0.0, 0.0);
        let lowered = highlights.finish(gray(1.0), &[gray(0.0), gray(1.0)], [0, 0]);
        assert!((lowered[0] - 0.75).abs() < 1e-12, "{lowered:?}");
        // Midtone: more contrast around middle gray, middle gray and the ends left.
        let contrast = filter([0.0, 50.0, 30.0], [0.0, 50.0, 30.0], 0.0, 100.0);
        assert!(plain(contrast, 0.7, 0.7)[0] > 0.7 && plain(contrast, 0.3, 0.3)[0] < 0.3);
        for v in [0.0, 0.5, 1.0] {
            assert!((plain(contrast, v, v)[0] - v).abs() < 1e-12);
        }
        // Color: the corrected color more saturated (its luma the same), less below 0.
        let red = [0.2, 0.05, 0.05, 1.0];
        let saturation = |color| {
            let f = filter([100.0, 50.0, 30.0], [0.0, 50.0, 30.0], color, 0.0);
            let c = f.finish(red, &[gray(0.1); 2], [0, 0]);
            (c[0] - c[1], luma([c[0], c[1], c[2]]))
        };
        let ((low, y0), (mid, y1), (high, y2)) =
            (saturation(-100.0), saturation(0.0), saturation(100.0));
        assert!(low < mid && mid < high, "{low} {mid} {high}");
        assert!((y0 - y1).abs() < 1e-12 && (y1 - y2).abs() < 1e-12);
        // Transparent pixels stay; alpha is kept.
        assert_eq!(shadows.finish([0.0; 4], &[[0.0; 4]; 2], [0, 0]), [0.0; 4]);
        let half = shadows.finish([0.0, 0.0, 0.0, 0.5], &[[0.0, 0.0, 0.0, 0.5]; 2], [0, 0]);
        assert!(
            (half[0] - 0.125).abs() < 1e-12 && half[3] == 0.5,
            "{half:?}"
        );
        // Ranges (Photoshop's); the scale is no setting: looks shrink the radii.
        assert!(filter([100.0, 100.0, 2500.0], [0.0, 0.0, 0.0], -100.0, 100.0).is_valid());
        for wrong in [
            filter([101.0, 50.0, 30.0], [0.0, 50.0, 30.0], 0.0, 0.0),
            filter([0.0, 50.0, 2501.0], [0.0, 50.0, 30.0], 0.0, 0.0),
            filter([0.0, 50.0, 30.5], [0.0, 50.0, 30.0], 0.0, 0.0),
            filter([0.0, 50.0, 30.0], [0.0, -1.0, 30.0], 0.0, 0.0),
            filter([0.0, 50.0, 30.0], [0.0, 50.0, 30.0], 101.0, 0.0),
        ] {
            assert!(!wrong.is_valid(), "{wrong:?}");
        }
        let look = shadows.scaled(4.0);
        assert_eq!(
            Filter::from_params("shadowsHighlights", &look.params()),
            Some(shadows)
        );
        assert_eq!(look.shadows_radii(), Some([7.5, 7.5]));
        assert!(look.reach() < shadows.reach());
        assert_eq!(shadows.plans().len(), 2);
        // A radius of 0: the pixel's own tone, no less than the smallest blur.
        let zero = filter([50.0, 50.0, 0.0], [0.0, 50.0, 30.0], 0.0, 0.0);
        assert_eq!(zero.shadows_radii(), Some([MIN_BLUR_RADIUS, 30.0]));
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
    fn maximum_and_minimum_are_the_extremes_of_the_square_or_the_disk_around() {
        let (w, h) = (23, 17);
        let source = speckled(w * h, 3);
        let brute = |x: usize, y: usize, r: usize, largest: bool, round: bool| -> [f32; 4] {
            let mut best = [if largest { f32::MIN } else { f32::MAX }; 4];
            for dy in -(r as i64)..=r as i64 {
                for dx in -(r as i64)..=r as i64 {
                    if round && dx * dx + dy * dy > (r * r) as i64 {
                        continue;
                    }
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
            for (largest, round) in [(true, false), (false, false), (true, true), (false, true)] {
                let mut region = source.clone();
                Kernel::Extreme {
                    radius: r,
                    largest,
                    round,
                }
                .region(&mut region, w, h);
                for y in 0..h {
                    for x in 0..w {
                        assert_eq!(
                            region[y * w + x],
                            brute(x, y, r, largest, round),
                            "{r} {largest} {round} ({x}, {y})"
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
            round: false,
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
            round: false,
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
            (
                Filter::Maximum {
                    radius: 1.0,
                    round: false,
                },
                500.0,
            ),
            (
                Filter::Minimum {
                    radius: 1.0,
                    round: false,
                },
                500.0,
            ),
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
    fn ripple_wave_zigzag_and_spherize_modes_move_pixels_as_photoshop() {
        let frame = [0.0, 0.0, 200.0, 100.0];
        let layer = [200.0, 100.0];
        let center = [100.0, 50.0];
        let close =
            |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
        // Ripple: along a wave across the other axis, up to its strength, the other way below 0.
        let ripple = |amount| Filter::Ripple {
            amount,
            size: RippleSize::Medium,
        };
        let most = RIPPLE_STRENGTH * RippleSize::Medium.wavelength();
        // A quarter wavelength down (y = 4): the most to the right.
        let q = ripple(100.0).source([0.0, 4.0], frame, layer).unwrap();
        assert!(
            (q[0] - most).abs() < 1e-9 && (q[1] - 4.0).abs() < 1e-9,
            "{q:?}"
        );
        let q = ripple(-200.0).source([0.0, 4.0], frame, layer).unwrap();
        assert!((q[0] + 2.0 * most).abs() < 1e-9, "{q:?}");
        assert!(close(
            ripple(0.0).source([33.0, 7.0], frame, layer).unwrap(),
            [33.0, 7.0]
        ));
        // Wave: the same seed, the same waves; another seed, others; within the amplitudes;
        // wrapped within the frame, or read beyond it (the edge repeating).
        let wave = |seed, wrap| Filter::Wave {
            generators: 5.0,
            wavelength: [10.0, 120.0],
            amplitude: [5.0, 35.0],
            scale: [100.0, 100.0],
            kind: WaveType::Sine,
            wrap,
            seed,
        };
        let points = [[3.0, 4.0], [150.0, 20.0], [77.0, 99.0], [199.0, 0.0]];
        let moved = |f: Filter| points.map(|p| f.source(p, frame, layer).unwrap());
        assert_eq!(moved(wave(7, false)), moved(wave(7, false)));
        assert_ne!(moved(wave(7, false)), moved(wave(8, false)));
        for (p, q) in points.iter().zip(moved(wave(7, false))) {
            assert!((q[0] - p[0]).abs() <= 35.0 && (q[1] - p[1]).abs() <= 35.0);
        }
        for q in moved(wave(7, true)) {
            assert!(
                (0.0..200.0).contains(&q[0]) && (0.0..100.0).contains(&q[1]),
                "{q:?}"
            );
        }
        // Min and max either way round: the same range.
        let mut swapped = wave(7, false);
        if let Filter::Wave {
            wavelength,
            amplitude,
            ..
        } = &mut swapped
        {
            wavelength.reverse();
            amplitude.reverse();
        }
        assert!(swapped.is_valid());
        assert_eq!(moved(swapped), moved(wave(7, false)));
        // A scale of 1 % barely moves; the shapes keep within -1 and 1.
        let mut flat = wave(7, false);
        if let Filter::Wave { scale, .. } = &mut flat {
            *scale = [1.0, 1.0];
        }
        for (p, q) in points.iter().zip(moved(flat)) {
            assert!((q[0] - p[0]).abs() <= 0.35 && (q[1] - p[1]).abs() <= 0.35);
        }
        for kind in [WaveType::Sine, WaveType::Triangle, WaveType::Square] {
            assert!((kind.shape(0.25) - 1.0).abs() < 1e-9, "{kind:?}");
            assert!((kind.shape(0.75) + 1.0).abs() < 1e-9, "{kind:?}");
            assert!((0..100).all(|i| kind.shape(f64::from(i) / 37.0).abs() <= 1.0));
        }
        // ZigZag: nothing beyond the inscribed circle; around the center keeps the distance,
        // out from it keeps the direction, pond ripples move along the diagonal.
        let zigzag = |style| Filter::ZigZag {
            amount: 100.0,
            ridges: 5.0,
            style,
        };
        let distance =
            |a: [f64; 2]| ((a[0] - center[0]).powi(2) + (a[1] - center[1]).powi(2)).sqrt();
        // A twentieth of the radius out: a quarter of the first ridge, the most there.
        let p = [102.5, 50.0];
        for style in [
            ZigZagStyle::AroundCenter,
            ZigZagStyle::OutFromCenter,
            ZigZagStyle::PondRipples,
        ] {
            let f = zigzag(style);
            assert!(close(
                f.source([10.0, 5.0], frame, layer).unwrap(),
                [10.0, 5.0]
            ));
            assert!(!close(f.source(p, frame, layer).unwrap(), p), "{style:?}");
        }
        let q = zigzag(ZigZagStyle::AroundCenter)
            .source(p, frame, layer)
            .unwrap();
        assert!((distance(q) - 2.5).abs() < 1e-9 && q[1] != 50.0);
        let q = zigzag(ZigZagStyle::OutFromCenter)
            .source(p, frame, layer)
            .unwrap();
        assert!(q[1] == 50.0 && q[0] > 102.5);
        let q = zigzag(ZigZagStyle::PondRipples)
            .source(p, frame, layer)
            .unwrap();
        assert!((q[0] - 102.5 - (q[1] - 50.0)).abs() < 1e-9 && q[1] > 50.0);
        // Spherize Horizontal Only: across only, the same everywhere down the frame.
        let sphere = |mode| Filter::Spherize {
            amount: 100.0,
            mode,
        };
        let q = sphere(SpherizeMode::HorizontalOnly)
            .source([140.0, 20.0], frame, layer)
            .unwrap();
        let r = sphere(SpherizeMode::HorizontalOnly)
            .source([140.0, 90.0], frame, layer)
            .unwrap();
        assert!(q[1] == 20.0 && r[1] == 90.0 && q[0] < 140.0 && q[0] > 100.0 && q[0] == r[0]);
        let q = sphere(SpherizeMode::VerticalOnly)
            .source([140.0, 70.0], frame, layer)
            .unwrap();
        assert!(q[0] == 140.0 && q[1] < 70.0 && q[1] > 50.0);
        // Their settings, Photoshop's ranges; Spherize written without its mode is Normal.
        assert_eq!(
            Filter::from_params("spherize", &[40.0]),
            Some(Filter::Spherize {
                amount: 40.0,
                mode: SpherizeMode::Normal
            })
        );
        assert_eq!(Filter::from_params("spherize", &[40.0, 3.0]), None);
        assert!(ripple(999.0).is_valid() && !ripple(1000.0).is_valid() && !ripple(1.5).is_valid());
        let mut many = wave(7, false);
        if let Filter::Wave { generators, .. } = &mut many {
            *generators = 1000.0;
        }
        assert!(!many.is_valid());
        assert!(
            !Filter::ZigZag {
                amount: 10.0,
                ridges: 21.0,
                style: ZigZagStyle::PondRipples
            }
            .is_valid()
        );
        assert_eq!(
            Filter::from_params(
                "wave",
                &[5.0, 10.0, 120.0, 5.0, 35.0, 100.0, 100.0, 0.0, 1.0, 0.5]
            ),
            None
        );
        for id in ["ripple", "wave", "zigZag", "spherize"] {
            let f = Filter::defaults(id).unwrap();
            assert!(f.is_valid() && f.samples() && f.scaled(4.0) == f, "{id}");
            assert_eq!(Filter::from_params(id, &f.params()), Some(f));
        }
    }

    #[test]
    fn shear_curves_take_two_to_sixteen_points_from_top_to_bottom() {
        assert_eq!(
            ShearCurve::new(&[[0.0, 0.0], [100.0, 0.0]]),
            Some(ShearCurve::STRAIGHT)
        );
        assert!(ShearCurve::STRAIGHT.is_straight());
        for wrong in [
            vec![[0.0, 0.0]],
            vec![[0.0, 0.0], [90.0, 0.0]],
            vec![[10.0, 0.0], [100.0, 0.0]],
            vec![[0.0, 0.0], [50.0, 0.0], [50.0, 10.0], [100.0, 0.0]],
            vec![[0.0, 0.0], [60.0, 0.0], [40.0, 0.0], [100.0, 0.0]],
            vec![[0.0, 101.0], [100.0, 0.0]],
            vec![[0.0, 0.0], [f32::NAN, 0.0], [100.0, 0.0]],
            vec![[0.0, f32::INFINITY], [100.0, 0.0]],
        ] {
            assert!(ShearCurve::new(&wrong).is_none(), "{wrong:?}");
        }
        let many: Vec<[f32; 2]> = (0..=16).map(|i| [i as f32 * 6.25, 0.0]).collect();
        assert!(ShearCurve::new(&many).is_none());
        let sixteen: Vec<[f32; 2]> = (0..16).map(|i| [i as f32 * 100.0 / 15.0, 0.0]).collect();
        assert!(ShearCurve::new(&sixteen).is_some());
        // Two points: a straight slant, from one offset to the other.
        let slant = ShearCurve::new(&[[0.0, -40.0], [100.0, 60.0]]).unwrap();
        for (t, x) in [(0.0, -0.4), (0.5, 0.1), (1.0, 0.6), (0.25, -0.15)] {
            assert!((slant.offset(t) - x).abs() < 1e-6, "{t}");
        }
        // Through its points, smoothly between them, flat beyond the frame.
        let bend =
            ShearCurve::new(&[[0.0, 0.0], [30.0, 50.0], [70.0, -20.0], [100.0, 0.0]]).unwrap();
        assert!(!bend.is_straight());
        for (t, x) in [(0.0, 0.0), (0.3, 0.5), (0.7, -0.2), (1.0, 0.0)] {
            assert!((bend.offset(t) - x).abs() < 1e-6, "{t}");
        }
        assert!(bend.offset(0.15) > 0.0 && bend.offset(0.5) < 0.5 && bend.offset(0.5) > -0.2);
        assert_eq!(bend.offset(-1.0), bend.offset(0.0));
        assert_eq!(bend.offset(2.0), bend.offset(1.0));
        // A spline overshooting the grid stays within it.
        let steep =
            ShearCurve::new(&[[0.0, 100.0], [10.0, -100.0], [20.0, 100.0], [100.0, 0.0]]).unwrap();
        assert!((0..=100).all(|i| steep.offset(f64::from(i) / 100.0).abs() <= 1.0));
    }

    #[test]
    fn shear_moves_each_row_and_keeps_its_points_in_its_values() {
        let curve = ShearCurve::new(&[[0.0, 50.0], [50.0, 0.0], [100.0, -25.0]]).unwrap();
        let shear = |wrap| Filter::Shear { curve, wrap };
        // Undefined Areas then each point's row and offset, as many as it has.
        assert_eq!(
            shear(true).params(),
            vec![0.0, 0.0, 50.0, 50.0, 0.0, 100.0, -25.0]
        );
        assert_eq!(
            Filter::from_params("shear", &shear(false).params()),
            Some(shear(false))
        );
        assert_eq!(
            Filter::from_params("shear", &[0.0, 0.0, 0.0, 100.0, 0.0]),
            Filter::defaults("shear")
        );
        for wrong in [
            &[0.0, 0.0, 0.0, 100.0, 0.0, 50.0][..],
            &[0.0, 0.0, 0.0][..],
            &[2.0, 0.0, 0.0, 100.0, 0.0][..],
            &[][..],
        ] {
            assert_eq!(Filter::from_params("shear", wrong), None, "{wrong:?}");
        }
        assert!(shear(true).samples() && shear(true).framed());
        assert_eq!(
            shear(true).scaled(4.0),
            shear(true),
            "in the layer's pixels"
        );
        // A frame 200 wide from x = 100: the top row moves by a quarter of it, to the right.
        let frame = [100.0, 0.0, 300.0, 100.0];
        let layer = [400.0, 100.0];
        let read = |f: Filter, p| f.source(p, frame, layer).unwrap();
        assert_eq!(read(shear(true), [200.0, 0.0]), [150.0, 0.0]);
        assert_eq!(read(shear(true), [240.0, 50.0]), [240.0, 50.0]);
        // Wrapped within the frame, or its edge's pixels repeated.
        assert_eq!(read(shear(true), [120.0, 0.0]), [270.0, 0.0]);
        assert_eq!(read(shear(false), [120.0, 0.0]), [100.5, 0.0]);
        let bottom = read(shear(false), [290.0, 100.0]);
        assert_eq!(bottom, [299.5, 100.0]);
        assert_eq!(read(shear(true), [290.0, 100.0]), [115.0, 100.0]);
    }

    #[test]
    fn displace_reads_as_far_as_its_map_says_stretched_or_tiled() {
        let displace = |scale, tile, wrap| Filter::Displace { scale, tile, wrap };
        let d = Filter::defaults("displace").unwrap();
        assert_eq!(d, displace([10.0, 10.0], false, false));
        assert!(d.is_valid() && d.samples() && d.framed() && d.takes_map());
        assert_eq!(d.params(), vec![10.0, 10.0, 0.0, 1.0]);
        assert_eq!(d.scaled(4.0), d, "in the layer's pixels");
        let any = displace([-999.0, 999.0], true, true);
        assert!(any.is_valid());
        assert_eq!(Filter::from_params("displace", &any.params()), Some(any));
        for wrong in [
            displace([1000.0, 0.0], false, false),
            displace([0.0, -1000.0], false, false),
            displace([10.5, 0.0], false, false),
        ] {
            assert!(!wrong.is_valid(), "{wrong:?}");
        }
        assert_eq!(
            Filter::from_params("displace", &[10.0, 10.0, 2.0, 0.0]),
            None
        );
        assert_eq!(
            Filter::from_params("displace", &[10.0, 10.0, 0.0, 2.0]),
            None
        );
        assert!(!Filter::GaussianBlur { radius: 1.0 }.takes_map());
        // The map over a frame 200 × 100 from (100, 0): stretched, or tiled at its size.
        let frame = [100.0, 0.0, 300.0, 100.0];
        let map = [20.0, 10.0];
        let stretched = displace([100.0, 100.0], false, false);
        assert_eq!(stretched.map_point([100.0, 0.0], frame, map), [-0.5, -0.5]);
        assert_eq!(stretched.map_point([200.0, 50.0], frame, map), [9.5, 4.5]);
        let tiled = displace([100.0, 100.0], true, false);
        assert_eq!(tiled.map_point([125.5, 13.5], frame, map), [5.0, 3.0]);
        // 128 moves nothing; each level a pixel at 100 %, by the scale either way.
        let p = [200.0, 50.0];
        let level = |v: f64| v / 255.0;
        let close =
            |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
        assert!(close(stretched.displaced(p, frame, [level(128.0); 2]), p));
        assert!(close(
            stretched.displaced(p, frame, [level(148.0), level(118.0)]),
            [220.0, 40.0]
        ));
        assert!(close(
            displace([-50.0, 200.0], false, false).displaced(
                p,
                frame,
                [level(148.0), level(118.0)]
            ),
            [190.0, 30.0]
        ));
        // At the extremes, 128 back and 127 on; beyond the frame wrapped, or its edges kept.
        assert!(close(
            stretched.displaced(p, frame, [0.0, 1.0]),
            [100.5, 99.5]
        ));
        assert!(close(
            displace([100.0, 100.0], false, true).displaced(p, frame, [0.0, 1.0]),
            [272.0, 77.0]
        ));
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
        let bulge = Filter::Spherize {
            amount: 100.0,
            mode: SpherizeMode::Normal,
        };
        assert!(distance(bulge.source(p, frame, layer).unwrap()) < 20.0);
        let hollow = Filter::Spherize {
            amount: -100.0,
            mode: SpherizeMode::Normal,
        };
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

    #[test]
    fn radial_blur_reads_along_its_arc_or_its_ray_around_its_center() {
        let radial = |amount, zoom, quality| Filter::RadialBlur {
            amount,
            zoom,
            quality,
            center: [25.0, 50.0],
        };
        let spin = radial(30.0, false, RadialQuality::Good);
        // A frame of 200 × 100 from (100, 0): the center at (150, 50).
        let frame = [100.0, 0.0, 300.0, 100.0];
        let c = [150.0, 50.0];
        let distance = |q: [f64; 2]| (q[0] - c[0]).hypot(q[1] - c[1]);
        let collect = |filter: Filter, p: [f64; 2], pixel: f64| {
            let mut points = Vec::new();
            let n = filter.radial_samples(p, frame, pixel, |q| points.push(q));
            assert_eq!(n, points.len());
            points
        };
        // Spin: on the circle through the pixel, 30 degrees of it centered on the pixel, about
        // a sample a pixel of arc.
        let p = [190.0, 50.0];
        let points = collect(spin, p, 1.0);
        let arc = 40.0 * 30f64.to_radians();
        assert!((points.len() as f64 - arc).abs() <= 2.0, "{}", points.len());
        for q in &points {
            assert!((distance(*q) - 40.0).abs() < 1e-9, "{q:?}");
        }
        let (first, last) = (points[0], points[points.len() - 1]);
        assert!((first[0] - last[0]).abs() < 1e-9 && (first[1] + last[1] - 100.0).abs() < 1e-9);
        // Zoom: along the ray, a quarter of the distance either way at 100.
        let zoom = radial(100.0, true, RadialQuality::Good);
        let points = collect(zoom, p, 1.0);
        assert!(points.iter().all(|q| (q[1] - 50.0).abs() < 1e-9));
        assert!((points[0][0] - 180.0).abs() < 1e-9);
        assert!((points[points.len() - 1][0] - 200.0).abs() < 1e-9);
        // At the center nothing moves: the pixel alone.
        assert_eq!(collect(spin, c, 1.0), vec![c]);
        // Fewer samples on a reduced level, and for Draft; more for Best, never past its most.
        let many = collect(
            radial(100.0, false, RadialQuality::Best),
            [300.0, 50.0],
            1.0,
        );
        assert_eq!(many.len(), 256);
        assert!(collect(spin, p, 4.0).len() < collect(spin, p, 1.0).len());
        let draft = collect(radial(30.0, false, RadialQuality::Draft), p, 1.0);
        assert!(draft.len() < collect(spin, p, 1.0).len());
        // Settings.
        assert!(spin.framed() && !spin.samples() && !spin.reads_original());
        assert_eq!(spin.scaled(4.0), spin, "placed in the layer's own pixels");
        assert!(!radial(0.0, false, RadialQuality::Good).is_valid());
        assert!(!radial(10.5, false, RadialQuality::Good).is_valid());
        assert!(!radial(101.0, true, RadialQuality::Good).is_valid());
        assert!(
            !Filter::RadialBlur {
                amount: 30.0,
                zoom: false,
                quality: RadialQuality::Good,
                center: [-1.0, 50.0],
            }
            .is_valid()
        );
        assert_eq!(
            Filter::from_params("radialBlur", &[10.0, 2.0, 1.0, 50.0, 50.0]),
            None
        );
        assert_eq!(
            Filter::from_params("radialBlur", &[10.0, 1.0, 3.0, 50.0, 50.0]),
            None
        );
        assert_eq!(
            Filter::from_params("radialBlur", &[10.0, 1.0, 2.0, 20.0, 70.0]),
            Some(Filter::RadialBlur {
                amount: 10.0,
                zoom: true,
                quality: RadialQuality::Best,
                center: [20.0, 70.0]
            })
        );
    }

    #[test]
    fn surface_blur_weighs_each_value_by_its_closeness() {
        // A row: 0.5 then a value 0.1 apart then one 0.5 apart, threshold 40 levels (2.5 × that
        // is 0.39): the far one weighs nothing.
        let threshold = 40.0 / 255.0;
        let kernel = Kernel::Surface {
            radius: 1,
            threshold,
        };
        let gray = |v: f32| [v, v, v, 1.0];
        let mut row = vec![gray(0.6), gray(0.5), gray(0.0)];
        kernel.region(&mut row, 3, 1);
        let near = 1.0 - 0.1 / (2.5 * threshold);
        let expected = (0.5 + near * 0.6) / (1.0 + near);
        assert!(
            (f64::from(row[1][0]) - expected).abs() < 1e-5,
            "{:?}",
            row[1]
        );
        // Alpha alike: all opaque stays opaque.
        assert!(row.iter().all(|px| (px[3] - 1.0).abs() < 1e-6));
        // A large radius reads every few pixels, no farther than the radius.
        let large = Kernel::Surface {
            radius: 100,
            threshold,
        };
        assert_eq!(large.reach(), 100);
        let mut flat = vec![gray(0.3); 40 * 3];
        large.region(&mut flat, 40, 3);
        assert!(flat.iter().all(|px| (px[0] - 0.3).abs() < 1e-6));
        // Settings: whole radii and thresholds, Photoshop's ranges.
        let surface = |radius, threshold| Filter::SurfaceBlur { radius, threshold };
        assert!(surface(1.0, 2.0).is_valid() && surface(100.0, 255.0).is_valid());
        for wrong in [
            surface(0.0, 15.0),
            surface(101.0, 15.0),
            surface(2.5, 15.0),
            surface(5.0, 1.0),
            surface(5.0, 256.0),
            surface(5.0, 15.5),
        ] {
            assert!(!wrong.is_valid(), "{wrong:?}");
        }
        assert_eq!(surface(40.0, 15.0).scaled(4.0), surface(10.0, 15.0));
        assert_eq!(surface(40.0, 15.0).reach(), 42.0);
    }

    #[test]
    fn average_gives_its_color_and_keeps_each_pixels_opacity() {
        let average = Filter::Average;
        // The average premultiplied: a red of half opacity is pure red.
        let half = average.finish([0.0, 0.2, 0.0, 0.4], &[[0.5, 0.0, 0.0, 0.5]], [0, 0]);
        assert!((half[0] - 0.4).abs() < 1e-12 && half[1] == 0.0 && half[3] == 0.4);
        // Nothing to average (all transparent): the pixel as it was.
        let pixel = [0.1, 0.2, 0.3, 0.5];
        assert_eq!(average.finish(pixel, &[[0.0; 4]], [0, 0]), pixel);
        assert!(average.reads_original() && average.reach().is_infinite());
        assert_eq!(Filter::from_params("average", &[]), Some(average));
        assert_eq!(Filter::from_params("average", &[1.0]), None);
    }

    #[test]
    fn smart_blur_averages_what_is_close_and_shows_its_edges() {
        let gray = |v: f32| [v, v, v, 1.0];
        // A row: 0.5, 0.55 and 0.9 around a pixel of 0.5, a threshold of 25 levels (0.098).
        let kernel = |mode| Kernel::Smart {
            radius: 1.0,
            threshold: 25.0 / 255.0,
            taps: 4,
            mode,
        };
        let row = || vec![gray(0.55), gray(0.5), gray(0.9)];
        let mut normal = row();
        kernel(SmartMode::Normal).region(&mut normal, 3, 1);
        // The disk of 1 on a row of one: the pixel thrice (above, itself, below), its left one,
        // not the far right one.
        assert!(
            (normal[1][0] - (3.0 * 0.5 + 0.55) / 4.0).abs() < 1e-6,
            "{:?}",
            normal[1]
        );
        let mut edges = row();
        kernel(SmartMode::EdgeOnly).region(&mut edges, 3, 1);
        assert_eq!(edges[1], gray(1.0), "next to 0.9: an edge, white");
        let mut flat = vec![gray(0.3); 9];
        kernel(SmartMode::EdgeOnly).region(&mut flat, 3, 3);
        assert_eq!(flat[4], [0.0, 0.0, 0.0, 1.0], "no edge: black");
        let mut overlay = vec![gray(0.3); 9];
        kernel(SmartMode::OverlayEdge).region(&mut overlay, 3, 3);
        assert!((overlay[4][0] - 0.3).abs() < 1e-6, "no edge: the blur");
        // Settings: Photoshop's ranges, a radius in tenths.
        let smart = |radius, threshold| Filter::SmartBlur {
            radius,
            threshold,
            quality: SmartQuality::High,
            mode: SmartMode::OverlayEdge,
        };
        assert!(smart(0.1, 0.1).is_valid() && smart(100.0, 100.0).is_valid());
        for wrong in [
            smart(0.0, 25.0),
            smart(100.5, 25.0),
            smart(3.0, 0.0),
            smart(3.0, 101.0),
        ] {
            assert!(!wrong.is_valid(), "{wrong:?}");
        }
        assert_eq!(smart(12.0, 25.0).scaled(4.0), smart(3.0, 25.0));
        assert_eq!(
            Filter::from_params("smartBlur", &[3.0, 25.0, 3.0, 0.0]),
            None
        );
        assert_eq!(
            Filter::from_params("smartBlur", &[3.0, 25.0, 0.0, 3.0]),
            None
        );
        assert_eq!(
            Filter::from_params("smartBlur", &[3.0, 25.0, 2.0, 2.0]),
            Some(smart(3.0, 25.0))
        );
    }

    #[test]
    fn drawing_filters_keep_their_colors_packed_and_their_seed() {
        let clouds = Filter::Clouds {
            foreground: [0x12, 0x34, 0x56],
            background: [255, 128, 0],
            seed: 99,
        };
        assert_eq!(clouds.params(), vec![1_193_046.0, 16_744_448.0, 99.0]);
        assert_eq!(
            Filter::from_params("clouds", &clouds.params()),
            Some(clouds)
        );
        assert_eq!(clouds.colors(), Some([[0x12, 0x34, 0x56], [255, 128, 0]]));
        assert!(clouds.renders() && !Filter::Average.renders());
        // Not a color, not a seed: not this filter's.
        for wrong in [
            [-1.0, 0.0, 0.0],
            [16_777_216.0, 0.0, 0.0],
            [0.5, 0.0, 0.0],
            [0.0, 0.0, NOISE_SEEDS as f32],
        ] {
            assert_eq!(Filter::from_params("differenceClouds", &wrong), None);
        }
        let fibers = |variance, strength| Filter::Fibers {
            variance,
            strength,
            foreground: [0; 3],
            background: [255; 3],
            seed: 0,
        };
        assert!(fibers(1.0, 64.0).is_valid() && fibers(64.0, 1.0).is_valid());
        for wrong in [fibers(0.0, 4.0), fibers(16.0, 65.0), fibers(16.5, 4.0)] {
            assert!(!wrong.is_valid(), "{wrong:?}");
        }
        // Drawn in the layer's pixels whatever the level, reading nothing around.
        assert_eq!(fibers(16.0, 4.0).scaled(4.0), fibers(16.0, 4.0));
        assert_eq!(clouds.reach(), 0.0);
    }

    /// `f` at the `n` × `n` pixel centers of a grid from the origin.
    fn field(n: usize, f: impl Fn([f64; 2]) -> f64) -> Vec<f64> {
        (0..n * n)
            .map(|i| f([(i % n) as f64 + 0.5, (i / n) as f64 + 0.5]))
            .collect()
    }

    #[test]
    fn clouds_are_smooth_spread_and_the_seeds() {
        let n = 256;
        let a = field(n, |p| clouds(p, 1));
        assert_eq!(a, field(n, |p| clouds(p, 1)), "the same again");
        assert!(a.iter().all(|v| (0.0..=1.0).contains(v)));
        let mean = a.iter().sum::<f64>() / a.len() as f64;
        let deviation = (a.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / a.len() as f64).sqrt();
        assert!((0.4..0.6).contains(&mean), "{mean}");
        assert!(deviation > 0.15, "spread: {deviation}");
        // Soft: next pixels close, on average much closer than pixels far apart.
        let step = |d: usize| {
            (0..n * (n - d))
                .map(|i| (a[i] - a[i + d * n]).abs())
                .sum::<f64>()
                / (n * (n - d)) as f64
        };
        assert!(
            step(1) < 0.05 && step(64) > 4.0 * step(1),
            "{} {}",
            step(1),
            step(64)
        );
        let b = field(n, |p| clouds(p, 2));
        let apart = a.iter().zip(&b).map(|(a, b)| (a - b).abs()).sum::<f64>() / a.len() as f64;
        assert!(apart > 0.1, "another seed: {apart}");
    }

    #[test]
    fn fibers_streak_down_and_shorten_with_variance_and_strength() {
        let n = 256;
        // How much pixels `d` apart down (or across) differ, on average.
        let change = |f: &[f64], d: usize, down: bool| {
            let mut sum = 0.0;
            for y in 0..n - d {
                for x in 0..n - d {
                    let o = if down { d * n } else { d };
                    sum += (f[y * n + x] - f[y * n + x + o]).abs();
                }
            }
            sum / ((n - d) * (n - d)) as f64
        };
        let usual = field(n, |p| fibers(p, 16.0, 4.0, 5));
        assert!(usual.iter().all(|v| (0.0..=1.0).contains(v)));
        assert!(change(&usual, 1, false) > 4.0 * change(&usual, 1, true));
        // A high variance: shorter streaks, more change down a few pixels.
        let varied = field(n, |p| fibers(p, 64.0, 4.0, 5));
        let long = field(n, |p| fibers(p, 2.0, 4.0, 5));
        assert!(change(&varied, 8, true) > 2.0 * change(&long, 8, true));
        // A high strength: the finest fibers only, more change across a few pixels.
        let stringy = field(n, |p| fibers(p, 16.0, 64.0, 5));
        let loose = field(n, |p| fibers(p, 16.0, 1.0, 5));
        assert!(change(&stringy, 2, false) > change(&loose, 2, false));
    }

    #[test]
    fn difference_clouds_take_each_channel_apart_and_keep_alpha() {
        let filter = Filter::DifferenceClouds {
            foreground: [0; 3],
            background: [255; 3],
            seed: 0,
        };
        // A half-transparent (0.8, 0.2, 0.5) under clouds of (0.3, 0.3, 0.3).
        let original = [0.4, 0.1, 0.25, 0.5];
        let out = filter.finish(original, &[[0.3, 0.3, 0.3, 1.0]], [0, 0]);
        for (o, e) in out.iter().zip([0.25, 0.05, 0.1, 0.5]) {
            assert!((o - e).abs() < 1e-12, "{out:?}");
        }
        let colors = [[0.0, 0.0, 0.0, 1.0], [1.0, 0.5, 0.0, 1.0]];
        let p = [12.5, 40.5];
        let v = clouds(p, 0);
        assert_eq!(filter.render(p, colors), [v, v * 0.5, 0.0, 1.0]);
    }

    #[test]
    fn stylize_settings_are_photoshops_ranges() {
        let tiles = |tiles, offset| Filter::Tiles {
            tiles,
            offset,
            fill: TilesFill::Background,
            foreground: [0; 3],
            background: [255; 3],
            seed: 0,
        };
        assert!(tiles(1.0, 99.0).is_valid() && tiles(99.0, 1.0).is_valid());
        for wrong in [
            tiles(0.0, 10.0),
            tiles(100.0, 10.0),
            tiles(10.0, 0.0),
            tiles(10.5, 10.0),
        ] {
            assert!(!wrong.is_valid(), "{wrong:?}");
        }
        let contour = |level| Filter::TraceContour { level, upper: true };
        assert!(contour(0.0).is_valid() && contour(255.0).is_valid());
        assert!(!contour(256.0).is_valid() && !contour(12.5).is_valid());
        let Some(Filter::OilPaint { .. }) =
            Filter::from_params("oilPaint", &[0.1, 0.0, 0.1, 0.0, 0.0, -180.0, 0.0])
        else {
            panic!("Oil Paint at its least");
        };
        for wrong in [
            [0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0],
            [1.0, 10.5, 1.0, 0.0, 1.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 0.0, 1.0, 181.0, 1.0],
            [1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 11.0],
        ] {
            let filter = Filter::from_params("oilPaint", &wrong).expect("its parameters");
            assert!(!filter.is_valid(), "{wrong:?}");
        }
        assert_eq!(
            Filter::from_params("oilPaint", &[1.0, 1.0, 1.0, 0.0, 2.0, 0.0, 1.0]),
            None
        );
        assert_eq!(Filter::from_params("wind", &[3.0, 0.0, 0.0]), None);
        assert_eq!(Filter::from_params("diffuse", &[4.0, 0.0]), None);
        // A look of a reduced level: Wind's streaks and Oil Paint's strokes shrink; Diffuse
        // stops moving pixels beyond 1.5.
        let wind = Filter::defaults("wind").unwrap();
        assert!(wind.scaled(4.0).reach() < wind.reach());
        let oil = Filter::defaults("oilPaint").unwrap();
        assert!(oil.scaled(4.0).reach() < oil.reach());
        let diffuse = Filter::defaults("diffuse").unwrap();
        assert!(matches!(
            diffuse.scaled(2.0).plan().kernel,
            Kernel::Identity
        ));
        assert!(matches!(diffuse.plan().kernel, Kernel::Diffuse { .. }));
    }

    /// Pixel `(x, y)` of `region`, `width` wide.
    fn at(region: &[[f32; 4]], width: usize, x: usize, y: usize) -> [f32; 4] {
        region[y * width + x]
    }

    #[test]
    fn diffuse_takes_a_neighbour_drawn_from_its_place_as_its_mode_says() {
        let (w, h) = (24, 20);
        let source = speckled(w * h, 3);
        let diffused = |mode, origin: [i64; 2], region: &[[f32; 4]], (w, h): (usize, usize)| {
            let mut out = region.to_vec();
            diffuse(&mut out, w, h, mode, 9, origin);
            out
        };
        let normal = diffused(DiffuseMode::Normal, [0, 0], &source, (w, h));
        let darker = diffused(DiffuseMode::DarkenOnly, [0, 0], &source, (w, h));
        let lighter = diffused(DiffuseMode::LightenOnly, [0, 0], &source, (w, h));
        let mut moved = 0;
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let own = at(&source, w, x, y);
                let out = at(&normal, w, x, y);
                let neighbours: Vec<[f32; 4]> = NEIGHBOURS
                    .iter()
                    .map(|[dx, dy]| {
                        at(
                            &source,
                            w,
                            (x as i64 + dx) as usize,
                            (y as i64 + dy) as usize,
                        )
                    })
                    .collect();
                assert!(neighbours.contains(&out), "({x}, {y})");
                moved += usize::from(out != own);
                let d = at(&darker, w, x, y);
                assert!(d == own || (neighbours.contains(&d) && luma_of(d) < luma_of(own)));
                let l = at(&lighter, w, x, y);
                assert!(l == own || (neighbours.contains(&l) && luma_of(l) > luma_of(own)));
            }
        }
        assert!(moved > (w - 2) * (h - 2) * 9 / 10, "{moved}");
        // A part of the layer, from its place: the same neighbours.
        let part: Vec<[f32; 4]> = (5..15)
            .flat_map(|y| (4..20).map(move |x| (x, y)))
            .map(|(x, y)| at(&source, w, x, y))
            .collect();
        let shifted = diffused(DiffuseMode::Normal, [4, 5], &part, (16, 10));
        for y in 1..9 {
            for x in 1..15 {
                assert_eq!(
                    at(&shifted, 16, x, y),
                    at(&normal, w, x + 4, y + 5),
                    "({x}, {y})"
                );
            }
        }
        // Anisotropic: along an edge, never across it.
        let halves: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                if i % w < 12 {
                    [0.0, 0.0, 0.0, 1.0]
                } else {
                    [1.0; 4]
                }
            })
            .collect();
        assert_eq!(
            diffused(DiffuseMode::Anisotropic, [0, 0], &halves, (w, h)),
            halves
        );
    }

    #[test]
    fn wind_blows_streaks_downwind_from_edges() {
        // A white bar at x = 40..44 on black, 120 pixels wide, 16 rows.
        let (w, h) = (120, 16);
        let bar: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                if (40..44).contains(&(i % w)) {
                    [1.0; 4]
                } else {
                    [0.0, 0.0, 0.0, 1.0]
                }
            })
            .collect();
        let blown = |method: WindMethod, from_left| {
            let (length, strength, stagger) = method.streaks();
            let mut out = bar.clone();
            wind(
                &mut out,
                w,
                [length as i64, i64::from(from_left)],
                strength,
                stagger,
                4,
                [0, 0],
            );
            out
        };
        for method in [WindMethod::Wind, WindMethod::Blast, WindMethod::Stagger] {
            let (length, ..) = method.streaks();
            // From the right: streaks go left of the bar, nothing changes right of it.
            let out = blown(method, false);
            let lit = |x: usize| (0..h).filter(|&y| at(&out, w, x, y)[0] > 0.05).count();
            assert!((38..40).map(lit).sum::<usize>() > 0, "{method:?}");
            for x in 44..w {
                assert_eq!(lit(x), 0, "{method:?} at {x}");
            }
            for x in 0..40usize.saturating_sub(length as usize + 1) {
                assert_eq!(lit(x), 0, "{method:?} beyond its length, at {x}");
            }
            // From the left: mirrored.
            let out = blown(method, true);
            let lit = |x: usize| (0..h).filter(|&y| at(&out, w, x, y)[0] > 0.05).count();
            assert!((44..46).map(lit).sum::<usize>() > 0, "{method:?}");
            for x in 0..40 {
                assert_eq!(lit(x), 0, "{method:?} at {x}");
            }
        }
    }

    #[test]
    fn trace_contour_outlines_each_channel_on_the_side_chosen() {
        // Red 0.8 left of x = 5, 0.2 from there; green 1 everywhere; half-transparent.
        let (w, h) = (10, 3);
        let step: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let red = if i % w < 5 { 0.8 } else { 0.2 };
                [red * 0.5, 0.5, 0.0, 0.5]
            })
            .collect();
        for upper in [true, false] {
            let mut out = step.clone();
            contour(&mut out, w, h, 0.5, upper);
            let line = if upper { 4 } else { 5 };
            for x in 0..w {
                let px = at(&out, w, x, 1);
                let red = if x == line { 0.0 } else { 0.5 };
                assert_eq!(px, [red, 0.5, 0.5, 0.5], "upper {upper} at {x}");
            }
        }
    }

    #[test]
    fn tiles_move_squares_within_their_reach_and_leave_gaps() {
        let tiles = |offset| Filter::Tiles {
            tiles: 4.0,
            offset,
            fill: TilesFill::Background,
            foreground: [0; 3],
            background: [255; 3],
            seed: 11,
        };
        let frame = [0.0, 0.0, 200.0, 100.0];
        // Tiles of 25 pixels (4 along the height), 8 by 4.
        let mut gaps = 0;
        for y in 0..100 {
            for x in 0..200 {
                let p = [x as f64 + 0.5, y as f64 + 0.5];
                match tiles(40.0).source(p, frame, [200.0, 100.0]) {
                    Some(q) => {
                        // Read from within the frame, at most 40 % of a tile away.
                        assert!((p[0] - q[0]).abs() <= 10.0 && (p[1] - q[1]).abs() <= 10.0);
                        assert!(q[0] >= 0.0 && q[1] >= 0.0 && q[0] < 200.0 && q[1] < 100.0);
                    }
                    None => gaps += 1,
                }
            }
        }
        assert!(gaps > 500, "{gaps}");
        // Outside the frame: as it is.
        let p = [150.0, 150.0];
        assert_eq!(tiles(40.0).source(p, frame, [300.0, 300.0]), Some(p));
        // The gaps' fills.
        let original = [0.2, 0.3, 0.4, 0.5];
        let colors = Some([[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0]]);
        let fill = |fill| Filter::Tiles {
            tiles: 4.0,
            offset: 10.0,
            fill,
            foreground: [0; 3],
            background: [255; 3],
            seed: 11,
        };
        assert_eq!(fill(TilesFill::Background).gap(original, colors), [1.0; 4]);
        assert_eq!(
            fill(TilesFill::Foreground).gap(original, colors),
            [0.0, 0.0, 0.0, 1.0]
        );
        let inverse = fill(TilesFill::Inverse).gap(original, colors);
        for (v, e) in inverse.iter().zip([0.3, 0.2, 0.1, 0.5]) {
            assert!((v - e).abs() < 1e-12, "{inverse:?}");
        }
        assert_eq!(fill(TilesFill::Unaltered).gap(original, colors), original);
        assert_eq!(Filter::Average.gap(original, colors), [0.0; 4]);
    }

    #[test]
    fn oil_paint_smooths_into_strokes_keeping_edges_and_lights_its_relief() {
        // A step with fine noise: the strokes smooth the noise, the step stays sharp.
        let (w, h) = (40, 30);
        let noise = speckled(w * h, 5);
        let source: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let base = if i % w < 20 { 0.2 } else { 0.8 };
                let v = base + (noise[i][0] - 0.5) * 0.1;
                [v, v, v, 1.0]
            })
            .collect();
        let painted = |light: Option<[f64; 2]>, bristle| {
            let mut out = source.clone();
            let filter = Oil {
                radius: 3,
                clean: 0.0,
                bristle,
                grain: 1.0,
                light,
                relief: 1.5,
            };
            oil(&mut out, w, h, &filter, [0, 0]);
            out
        };
        let plain = painted(None, 0.0);
        let roughness = |img: &[[f32; 4]]| {
            (5..h - 5)
                .flat_map(|y| (3..15).map(move |x| (x, y)))
                .map(|(x, y)| (at(img, w, x + 1, y)[0] - at(img, w, x, y)[0]).abs())
                .sum::<f32>()
        };
        assert!(roughness(&plain) < roughness(&source) / 3.0);
        for y in 5..h - 5 {
            assert!(
                at(&plain, w, 19, y)[0] < 0.3 && at(&plain, w, 20, y)[0] > 0.7,
                "{y}"
            );
        }
        // Lit from the left: the step's rise faces it, lighter; lit from the right, darker.
        let lit = painted(Some([-1.0, 0.0]), 0.0);
        assert!(at(&lit, w, 20, 10)[0] > at(&plain, w, 20, 10)[0]);
        let shaded = painted(Some([1.0, 0.0]), 0.0);
        assert!(at(&shaded, w, 20, 10)[0] < at(&plain, w, 20, 10)[0]);
        let flat = Oil {
            radius: 2,
            clean: 0.0,
            bristle: 0.0,
            grain: 1.0,
            light: Some([1.0, 0.0]),
            relief: 1.5,
        };
        let mut gray = vec![[0.5, 0.5, 0.5, 1.0]; 100];
        oil(&mut gray, 10, 10, &flat, [0, 0]);
        assert!(gray.iter().all(|p| *p == [0.5, 0.5, 0.5, 1.0]));
        // The bristles grain even a flat area.
        let mut grained = vec![[0.5, 0.5, 0.5, 1.0]; 100];
        oil(
            &mut grained,
            10,
            10,
            &Oil {
                bristle: 1.0,
                ..flat
            },
            [0, 0],
        );
        assert!(grained.iter().any(|p| (p[0] - 0.5).abs() > 0.001));
    }

    #[test]
    fn pixelate_settings_are_photoshops_ranges() {
        // Photoshop's defaults.
        assert_eq!(
            Filter::defaults("colorHalftone").unwrap().params(),
            [8.0, 108.0, 162.0, 90.0]
        );
        assert_eq!(
            Filter::defaults("crystallize").unwrap().params(),
            [10.0, 0.0]
        );
        assert_eq!(
            Filter::defaults("pointillize").unwrap().params(),
            [5.0, 0xffffff as f32, 0.0]
        );
        assert_eq!(Filter::defaults("mezzotint").unwrap().params(), [0.0, 0.0]);
        assert!(Filter::defaults("facet").unwrap().params().is_empty());
        assert!(Filter::defaults("fragment").unwrap().params().is_empty());
        let halftone = |radius, angle| Filter::ColorHalftone {
            radius,
            angles: [angle, 162.0, 90.0],
        };
        assert!(halftone(4.0, -360.0).is_valid() && halftone(127.0, 360.0).is_valid());
        for wrong in [
            halftone(3.0, 0.0),
            halftone(128.0, 0.0),
            halftone(8.5, 0.0),
            halftone(8.0, 361.0),
            halftone(8.0, 0.5),
        ] {
            assert!(!wrong.is_valid(), "{wrong:?}");
        }
        let crystallize = |cell| Filter::Crystallize { cell, seed: 0 };
        let pointillize = |cell| Filter::Pointillize {
            cell,
            background: [255; 3],
            seed: 0,
        };
        assert!(crystallize(3.0).is_valid() && crystallize(300.0).is_valid());
        assert!(pointillize(3.0).is_valid() && pointillize(300.0).is_valid());
        for cell in [2.0, 301.0, 10.5] {
            assert!(!crystallize(cell).is_valid() && !pointillize(cell).is_valid());
        }
        // Mezzotint's ten types, in Photoshop's order.
        for (i, kind) in MezzotintType::ALL.iter().enumerate() {
            let filter = Filter::from_params("mezzotint", &[i as f32, 7.0]).unwrap();
            assert_eq!(
                filter,
                Filter::Mezzotint {
                    kind: *kind,
                    seed: 7,
                    reduction: 1.0
                }
            );
            assert_eq!(
                Filter::from_params("mezzotint", &filter.params()),
                Some(filter)
            );
        }
        assert_eq!(Filter::from_params("mezzotint", &[10.0, 0.0]), None);
        assert_eq!(Filter::from_params("mezzotint", &[1.5, 0.0]), None);
        assert_eq!(Filter::from_params("facet", &[1.0]), None);
        // Looks of reduced levels: smaller dots, Facet's radius down to nothing, Fragment's
        // offset; Crystallize's and Pointillize's cells stay in the layer's pixels.
        let dots = halftone(8.0, 108.0);
        assert_eq!(dots.scaled(2.0), halftone(4.0, 108.0));
        assert!(dots.scaled(2.0).reach() < dots.reach());
        assert!(matches!(dots.scaled(8.0).plan().kernel, Kernel::Identity));
        let facet = Filter::defaults("facet").unwrap();
        assert!(matches!(
            facet.plan().kernel,
            Kernel::Oil(Oil { radius: 2, .. })
        ));
        assert!(matches!(
            facet.scaled(2.0).plan().kernel,
            Kernel::Oil(Oil { radius: 1, .. })
        ));
        assert!(matches!(facet.scaled(8.0).plan().kernel, Kernel::Identity));
        let fragment = Filter::defaults("fragment").unwrap();
        assert!(matches!(
            fragment.scaled(4.0).plan().kernel,
            Kernel::Fragment(d) if d == 1.0
        ));
        for id in ["crystallize", "pointillize"] {
            let f = Filter::defaults(id).unwrap();
            assert!(
                f.samples() && f.scaled(4.0) == f && !f.reads_original(),
                "{id}"
            );
        }
    }

    #[test]
    fn crystallize_reads_each_pixel_at_its_closest_point() {
        let (w, h) = (90.0, 70.0);
        let layer = [w, h];
        let frame = [0.0, 0.0, w, h];
        let cell = 10.0;
        let filter = Filter::Crystallize {
            cell: 10.0,
            seed: 3,
        };
        // Every square's point, brute force.
        let points: Vec<[f64; 2]> = (-1..10)
            .flat_map(|j| (-1..11).map(move |i| [i, j]))
            .map(|square| cell_point(square, cell, 3, CRYSTAL_SALT, 1.0))
            .collect();
        let distance = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).hypot(a[1] - b[1]);
        let mut sources = std::collections::BTreeSet::new();
        for y in 0..70 {
            for x in 0..90 {
                let p = [x as f64 + 0.5, y as f64 + 0.5];
                let q = filter.source(p, frame, layer).expect("a point");
                let closest = points
                    .iter()
                    .copied()
                    .min_by(|a, b| distance(*a, p).total_cmp(&distance(*b, p)))
                    .expect("points");
                assert_eq!(q, pixel_center(closest, layer), "({x}, {y})");
                // A pixel's center within the layer, within a cell's diagonal of the pixel.
                assert!(q[0] > 0.0 && q[1] > 0.0 && q[0] < w && q[1] < h);
                assert!(distance(q, p) <= cell * 1.5 + 1.0);
                sources.insert([q[0] as i64, q[1] as i64]);
            }
        }
        // About a polygon a square, wherever the frame is.
        assert!((50..=90).contains(&sources.len()), "{}", sources.len());
        let p = [33.5, 21.5];
        assert_eq!(
            filter.source(p, [20.0, 10.0, 40.0, 30.0], layer),
            filter.source(p, frame, layer)
        );
        // Another seed, other polygons.
        let other = Filter::Crystallize {
            cell: 10.0,
            seed: 4,
        };
        let moved = (0..90)
            .filter(|&x| {
                let p = [x as f64 + 0.5, 30.5];
                other.source(p, frame, layer) != filter.source(p, frame, layer)
            })
            .count();
        assert!(moved > 45, "{moved}");
    }

    #[test]
    fn pointillize_draws_dots_of_the_layer_on_its_background() {
        let layer = [120.0, 80.0];
        let frame = [0.0, 0.0, 120.0, 80.0];
        let cell = 8.0;
        let filter = Filter::Pointillize {
            cell: 8.0,
            background: [10, 20, 30],
            seed: 5,
        };
        let mut background = 0;
        for y in 0..80 {
            for x in 0..120 {
                let p = [x as f64 + 0.5, y as f64 + 0.5];
                // The last dot over the pixel, row by row, brute force.
                let mut last = None;
                for j in -1..11 {
                    for i in -1..16 {
                        let q = cell_point([i, j], cell, 5, DOT_SALT, 1.0);
                        let [low, high] = DOT_RADIUS;
                        let r = cell * (low + (high - low) * draw([i, j], 5, DOT_SALT + 2));
                        if (q[0] - p[0]).hypot(q[1] - p[1]) <= r {
                            last = Some(pixel_center(q, layer));
                        }
                    }
                }
                let source = filter.source(p, frame, layer);
                assert_eq!(source, last, "({x}, {y})");
                background += usize::from(source.is_none());
            }
        }
        // Dots over most of the layer, the background between them.
        let share = background as f64 / (120.0 * 80.0);
        assert!((0.15..0.5).contains(&share), "{share}");
        let colors = Some([[0.1, 0.1, 0.1, 1.0], [0.2, 0.3, 0.4, 1.0]]);
        assert_eq!(filter.gap([0.5; 4], colors), [0.2, 0.3, 0.4, 1.0]);
        assert_eq!(filter.colors(), Some([[10, 20, 30]; 2]));
    }

    #[test]
    fn color_halftone_dots_cover_as_much_as_each_channel_and_keep_its_tone() {
        // The radius covering a value of the unit square, from none to the whole of it.
        assert_eq!(dot_radius(0.0), 0.0);
        assert!((dot_radius(1.0) - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-9);
        for v in [0.1, 0.5, 0.7] {
            let r = dot_radius(v);
            let n = 400;
            let mut count = 0;
            for j in 0..n {
                for i in 0..n {
                    let (x, y) = ((i as f64 + 0.5) / n as f64, (j as f64 + 0.5) / n as f64);
                    count += usize::from((x - 0.5).hypot(y - 0.5) <= r);
                }
            }
            let covered = count as f64 / (n * n) as f64;
            assert!((covered - v).abs() < 0.01, "{v}: {covered}");
        }
        // A flat color: each channel's dots cover about its share, half-transparent pixels kept
        // so.
        let (w, h) = (160, 120);
        let flat = [0.2 * 0.5, 0.5 * 0.5, 0.9 * 0.5, 0.5];
        let screen = Halftone::new(6.0, [108.0, 162.0, 90.0]);
        let mut out = vec![flat; w * h];
        halftone(&mut out, w, h, &screen, [0, 0]);
        let reach = screen.reach();
        for (c, value) in [0.2, 0.5, 0.9].into_iter().enumerate() {
            let (mut sum, mut n) = (0.0, 0.0);
            for y in reach..h - reach {
                for x in reach..w - reach {
                    let px = out[y * w + x];
                    assert_eq!(px[3], 0.5);
                    sum += f64::from(px[c] / px[3]);
                    n += 1.0;
                }
            }
            assert!((sum / n - value).abs() < 0.03, "channel {c}: {}", sum / n);
        }
        let mut black = vec![[0.0, 0.0, 0.0, 1.0]; w * h];
        halftone(&mut black, w, h, &screen, [0, 0]);
        assert!(black.iter().all(|p| *p == [0.0, 0.0, 0.0, 1.0]));
        let mut white = vec![[1.0; 4]; w * h];
        halftone(&mut white, w, h, &screen, [0, 0]);
        assert!(white.iter().all(|p| *p == [1.0; 4]));
        // A part of the layer, from its place: the same dots.
        let image: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let v = (i % w) as f32 / w as f32;
                [v, 1.0 - v, 0.5, 1.0]
            })
            .collect();
        let mut whole = image.clone();
        halftone(&mut whole, w, h, &screen, [0, 0]);
        let (ox, oy, rw, rh) = (37usize, 21usize, 90usize, 80usize);
        let mut part: Vec<[f32; 4]> = (0..rw * rh)
            .map(|i| image[(oy + i / rw) * w + ox + i % rw])
            .collect();
        halftone(&mut part, rw, rh, &screen, [ox as i64, oy as i64]);
        for y in reach..rh - reach {
            for x in reach..rw - reach {
                assert_eq!(part[y * rw + x], whole[(oy + y) * w + ox + x], "({x}, {y})");
            }
        }
    }

    #[test]
    fn fragment_averages_four_copies_diagonally_apart() {
        let (w, h) = (20, 20);
        let mut point = vec![[0.0f32; 4]; w * h];
        point[10 * w + 10] = [1.0; 4];
        fragment(&mut point, w, h, FRAGMENT_OFFSET);
        for y in 0..h {
            for x in 0..w {
                let copy = [6, 14].contains(&x) && [6, 14].contains(&y);
                let v = if copy { 0.25 } else { 0.0 };
                assert_eq!(at(&point, w, x, y), [v; 4], "({x}, {y})");
            }
        }
    }

    #[test]
    fn mezzotint_makes_each_channel_full_or_none_as_often_as_its_value() {
        let (w, h) = (128, 128);
        let flat = [0.3 * 0.5, 0.6 * 0.5, 0.0, 0.5];
        for kind in MezzotintType::ALL {
            let mut out = vec![flat; w * h];
            mezzotint(&mut out, w, h, kind, 2, 1, [0, 0]);
            for (c, value) in [0.3, 0.6, 0.0].into_iter().enumerate() {
                assert!(
                    out.iter()
                        .all(|p| (p[c] == 0.5 || p[c] == 0.0) && p[3] == 0.5)
                );
                let full = out.iter().filter(|p| p[c] == 0.5).count();
                let share = full as f64 / (w * h) as f64;
                assert!(
                    (share - value).abs() < 0.06,
                    "{kind:?} channel {c}: {share}"
                );
            }
            // A part of the layer, from its place: the same pattern.
            let mut part = vec![flat; 40 * 30];
            mezzotint(&mut part, 40, 30, kind, 2, 1, [50, 60]);
            for y in 0..30 {
                for x in 0..40 {
                    assert_eq!(at(&part, 40, x, y), at(&out, w, 50 + x, 60 + y), "{kind:?}");
                }
            }
        }
        // Lines run along the rows, strokes along the diagonals: a channel changes less often
        // that way than across; clumps less often than single dots.
        let changes = |kind, step: [usize; 2]| {
            let mut out = vec![[0.5, 0.5, 0.5, 1.0]; w * h];
            mezzotint(&mut out, w, h, kind, 2, 1, [0, 0]);
            (0..h - 1)
                .flat_map(|y| (0..w - 1).map(move |x| (x, y)))
                .filter(|&(x, y)| at(&out, w, x, y)[0] != at(&out, w, x + step[0], y + step[1])[0])
                .count()
        };
        let long = MezzotintType::LongLines;
        assert!(changes(long, [1, 0]) * 4 < changes(long, [0, 1]));
        let strokes = MezzotintType::LongStrokes;
        assert!(changes(strokes, [1, 1]) * 4 < changes(strokes, [1, 0]));
        assert!(
            changes(MezzotintType::CoarseDots, [1, 0]) * 2
                < changes(MezzotintType::FineDots, [1, 0])
        );
        // A reduced level: the share of the layer's pixels under each, about the value.
        let mut reduced = vec![flat; 32 * 32];
        mezzotint(&mut reduced, 32, 32, MezzotintType::FineDots, 2, 4, [0, 0]);
        let mean = reduced.iter().map(|p| f64::from(p[0] / p[3])).sum::<f64>() / 1024.0;
        assert!((mean - 0.3).abs() < 0.03, "{mean}");
        assert!(
            reduced
                .iter()
                .all(|p| ((p[0] / p[3]) * 16.0).fract().abs() < 1e-4)
        );
        assert!(reduced.iter().any(|p| p[0] > 0.0 && p[0] < p[3]));
    }

    #[test]
    fn other_noise_and_sharpen_settings_are_photoshops_ranges() {
        // Photoshop's defaults.
        let custom = Filter::defaults("custom").unwrap().params();
        assert_eq!(custom.len(), CUSTOM_WEIGHTS + 2);
        assert_eq!(custom[12], 5.0);
        assert_eq!([custom[7], custom[11], custom[13], custom[17]], [-1.0; 4]);
        assert_eq!(
            custom.iter().sum::<f32>(),
            2.0,
            "the rest 0, Scale 1, Offset 0"
        );
        assert_eq!(&custom[CUSTOM_WEIGHTS..], [1.0, 0.0]);
        assert_eq!(Filter::defaults("hsbHsl").unwrap().params(), [0.0, 1.0]);
        assert!(Filter::defaults("despeckle").unwrap().params().is_empty());
        assert_eq!(
            Filter::defaults("reduceNoise").unwrap().params(),
            [6.0, 60.0, 45.0, 25.0]
        );
        assert_eq!(
            Filter::defaults("smartSharpen").unwrap().params(),
            [200.0, 1.0, 10.0, 1.0, 0.0],
            "Lens Blur"
        );
        assert_eq!(Filter::defaults("maximum").unwrap().params(), [1.0, 0.0]);
        // Written before Preserve: Squareness.
        assert_eq!(
            Filter::from_params("minimum", &[3.0]),
            Some(Filter::Minimum {
                radius: 3.0,
                round: false
            })
        );
        assert_eq!(
            Filter::from_params("maximum", &[3.0, 1.0]),
            Some(Filter::Maximum {
                radius: 3.0,
                round: true
            })
        );
        assert_eq!(Filter::from_params("maximum", &[3.0, 2.0]), None);
        // Custom's ranges.
        let with = |i: usize, v: f32| {
            let mut values = custom.clone();
            values[i] = v;
            Filter::from_params("custom", &values).unwrap()
        };
        assert!(with(0, -999.0).is_valid() && with(24, 999.0).is_valid());
        assert!(with(25, 9999.0).is_valid() && with(26, -9999.0).is_valid());
        for (i, wrong) in [
            (0, 1000.0),
            (3, 0.5),
            (25, 0.0),
            (25, 10000.0),
            (26, 10000.0),
        ] {
            assert!(!with(i, wrong).is_valid(), "{i} {wrong}");
        }
        assert_eq!(Filter::from_params("custom", &custom[..26]), None);
        // HSB/HSL's three models each way.
        for input in 0..3 {
            for order in 0..3 {
                let values = [input as f32, order as f32];
                let filter = Filter::from_params("hsbHsl", &values).unwrap();
                assert_eq!(filter.params(), values);
            }
        }
        assert_eq!(Filter::from_params("hsbHsl", &[3.0, 0.0]), None);
        // Reduce Noise's and Smart Sharpen's ranges.
        let noise = |strength, preserve| Filter::ReduceNoise {
            strength,
            preserve,
            color: 100.0,
            sharpen: 0.0,
            reduction: 1.0,
        };
        assert!(noise(0.0, 0.0).is_valid() && noise(10.0, 100.0).is_valid());
        for wrong in [noise(11.0, 0.0), noise(5.5, 0.0), noise(5.0, 101.0)] {
            assert!(!wrong.is_valid(), "{wrong:?}");
        }
        let sharpen = |amount, radius, angle| Filter::SmartSharpen {
            amount,
            radius,
            noise: 0.0,
            remove: SharpenRemove::MotionBlur,
            angle,
        };
        assert!(sharpen(1.0, 0.1, -180.0).is_valid() && sharpen(500.0, 64.0, 180.0).is_valid());
        for wrong in [
            sharpen(0.0, 1.0, 0.0),
            sharpen(501.0, 1.0, 0.0),
            sharpen(100.0, 64.5, 0.0),
            sharpen(100.0, 0.05, 0.0),
            sharpen(100.0, 1.0, 181.0),
        ] {
            assert!(!wrong.is_valid(), "{wrong:?}");
        }
        assert_eq!(
            Filter::from_params("smartSharpen", &[100.0, 1.0, 0.0, 3.0, 0.0]),
            None
        );
        // Looks of reduced levels: the radii shrink, the weights and models stay.
        assert_eq!(
            sharpen(100.0, 8.0, 30.0).scaled(4.0),
            sharpen(100.0, 2.0, 30.0)
        );
        let default = |id| Filter::defaults(id).unwrap();
        assert_eq!(default("custom").scaled(4.0), default("custom"));
        assert_eq!(default("hsbHsl").scaled(4.0), default("hsbHsl"));
        assert_eq!(
            default("despeckle").scaled(4.0),
            Filter::Despeckle { reduction: 4.0 }
        );
    }

    #[test]
    fn custom_weighs_the_pixels_around_each_divides_by_its_scale_and_adds_its_offset() {
        let (w, h) = (9, 7);
        let source = speckled(w * h, 5);
        let at = |x: i64, y: i64| {
            source[y.clamp(0, h as i64 - 1) as usize * w + x.clamp(0, w as i64 - 1) as usize]
        };
        let run = |weights: [f32; CUSTOM_WEIGHTS], scale: f32| {
            let filter = Filter::Custom {
                weights,
                scale,
                offset: 0.0,
            };
            let mut region = source.clone();
            filter.plan().kernel.region(&mut region, w, h);
            region
        };
        // One weight right of the center: each pixel its right neighbour's value (the grid as
        // Photoshop shows it, over the pixels around).
        let right = run(std::array::from_fn(|i| f32::from(u8::from(i == 13))), 1.0);
        // Two below, halved by a scale of 2 and doubled by a weight of 2.
        let below = run(
            std::array::from_fn(|i| if i == 22 { 2.0 } else { 0.0 }),
            2.0,
        );
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let i = y as usize * w + x as usize;
                assert_eq!(right[i], at(x + 1, y), "({x}, {y})");
                for (got, want) in below[i].iter().zip(at(x, y + 2)) {
                    assert!((got - want).abs() < 1e-6, "({x}, {y})");
                }
            }
        }
        // The offset in levels of 8 bits, each pixel its opacity.
        let filter = Filter::Custom {
            weights: std::array::from_fn(|i| f32::from(u8::from(i == 12))),
            scale: 1.0,
            offset: 51.0,
        };
        let half = [0.1, 0.2, 0.3, 0.5];
        let out = filter.finish(half, &[half], [0, 0]);
        let expected = [0.4, 0.6, 0.8].map(|c| c * 0.5);
        for c in 0..3 {
            assert!((out[c] - expected[c]).abs() < 1e-6, "{out:?}");
        }
        assert_eq!(out[3], 0.5);
        // Photoshop's default sharpens, a flat area unchanged.
        let mut region = vec![[0.25f32; 4]; 25];
        Filter::defaults("custom")
            .unwrap()
            .plan()
            .kernel
            .region(&mut region, 5, 5);
        assert!(region.iter().all(|p| (p[0] - 0.25).abs() < 1e-6));
    }

    #[test]
    fn hsb_hsl_converts_the_channels_as_photoshop_does() {
        use ColorModel::{Hsb, Hsl, Rgb};
        let convert = |input, order, rgb: [f64; 3]| {
            let c = Filter::HsbHsl { input, order }.finish(
                [rgb[0], rgb[1], rgb[2], 1.0],
                &[[0.0; 4]],
                [0, 0],
            );
            [c[0], c[1], c[2]]
        };
        let close = |a: [f64; 3], b: [f64; 3]| (0..3).all(|i| (a[i] - b[i]).abs() < 1e-9);
        // The hue in red (0 to 1 for 0° to 360°), the saturation in green, the brightness or
        // the lightness in blue.
        assert!(close(convert(Rgb, Hsb, [1.0, 0.0, 0.0]), [0.0, 1.0, 1.0]));
        assert!(close(convert(Rgb, Hsl, [1.0, 0.0, 0.0]), [0.0, 1.0, 0.5]));
        assert!(close(
            convert(Rgb, Hsb, [0.0, 0.5, 0.0]),
            [1.0 / 3.0, 1.0, 0.5]
        ));
        assert!(close(
            convert(Rgb, Hsl, [0.5, 0.5, 1.0]),
            [2.0 / 3.0, 1.0, 0.75]
        ));
        assert!(close(convert(Rgb, Hsb, [0.4, 0.4, 0.4]), [0.0, 0.0, 0.4]));
        assert!(close(convert(Hsb, Rgb, [0.5, 1.0, 1.0]), [0.0, 1.0, 1.0]));
        // Each way back, whatever the color; a model to itself changes nothing.
        for px in speckled(200, 9) {
            let rgb = [px[0], px[1], px[2]].map(f64::from);
            for model in [Rgb, Hsb, Hsl] {
                assert!(close(convert(model, Rgb, convert(Rgb, model, rgb)), rgb));
                assert!(close(convert(model, model, rgb), rgb));
            }
            assert!(close(
                convert(Hsl, Rgb, convert(Hsb, Hsl, convert(Rgb, Hsb, rgb))),
                rgb
            ));
        }
        // Its opacity kept, the colors premultiplied by it.
        let half =
            Filter::defaults("hsbHsl")
                .unwrap()
                .finish([0.5, 0.0, 0.0, 0.5], &[[0.0; 4]], [0, 0]);
        assert_eq!(half, [0.0, 0.5, 0.5, 0.5]);
    }

    #[test]
    fn the_disk_is_the_union_of_its_rectangles() {
        for radius in 1..=40usize {
            let r = radius as i64;
            let rectangles = disk_rectangles(radius);
            for dy in -r..=r {
                for dx in -r..=r {
                    let inside = dx * dx + dy * dy <= r * r;
                    let covered = rectangles.iter().any(|&[hw, hh]| {
                        dx.unsigned_abs() as usize <= hw && dy.unsigned_abs() as usize <= hh
                    });
                    assert_eq!(covered, inside, "{radius} ({dx}, {dy})");
                }
            }
        }
        assert_eq!(disk_rectangles(1), [[1, 0], [0, 1]], "a plus");
        // A large Roundness on the layer reduced, Squareness never.
        let round = Plan::extreme(100.0, true, true);
        assert_eq!(round.factor, 4);
        assert_eq!(round.kernel.reach(), 25);
        assert_eq!(Plan::extreme(100.0, true, false).factor, 1);
        assert_eq!(Plan::extreme(32.0, false, true).factor, 1);
    }

    #[test]
    fn box_planes_and_guided_smoothing_keep_flat_areas_and_edges() {
        let (w, h) = (25, 11);
        let plane: Vec<f32> = speckled(w * h, 2).iter().map(|p| p[0]).collect();
        let at = |x: i64, y: i64| {
            plane[y.clamp(0, h as i64 - 1) as usize * w + x.clamp(0, w as i64 - 1) as usize]
        };
        for radius in [1usize, 3, 8] {
            let boxed = box_plane(&plane, w, h, radius);
            let r = radius as i64;
            for y in 0..h as i64 {
                for x in 0..w as i64 {
                    let sum: f32 = (-r..=r)
                        .flat_map(|dy| (-r..=r).map(move |dx| (dx, dy)))
                        .map(|(dx, dy)| at(x + dx, y + dy))
                        .sum();
                    let mean = sum / ((2 * r + 1) * (2 * r + 1)) as f32;
                    let got = boxed[y as usize * w + x as usize];
                    assert!((got - mean).abs() < 1e-5, "{radius} ({x}, {y})");
                }
            }
        }
        // A step of half the range kept, small grain smoothed.
        let step: Vec<f32> = (0..w * h)
            .map(|i| {
                let base = if i % w < 12 { 0.2 } else { 0.7 };
                base + (plane[i] - 0.5) * 0.04
            })
            .collect();
        let smooth = guided(&step, w, h, 2, 0.02 * 0.02);
        let spread = |v: &[f32], columns: std::ops::Range<usize>| {
            let values: Vec<f32> = (0..h)
                .flat_map(|y| columns.clone().map(move |x| v[y * w + x]))
                .collect();
            let max = values.iter().copied().fold(f32::MIN, f32::max);
            let min = values.iter().copied().fold(f32::MAX, f32::min);
            max - min
        };
        assert!(spread(&smooth, 0..6) < spread(&step, 0..6) / 2.0);
        let edge = smooth[5 * w + 13] - smooth[5 * w + 10];
        assert!(edge > 0.45, "{edge}");
        assert_eq!(guided(&step, w, h, 2, 0.0), step, "no smoothing at 0");
    }

    /// A gray of `level` with grain of up to `grain` on each channel, opaque.
    fn grainy(w: usize, h: usize, level: f32, grain: f32) -> Vec<[f32; 4]> {
        speckled(w * h, 11)
            .into_iter()
            .map(|p| {
                let mut px = [level; 4];
                for c in 0..3 {
                    px[c] += (p[c] - 0.5) * 2.0 * grain;
                }
                px[3] = 1.0;
                px
            })
            .collect()
    }

    fn deviation(region: &[[f32; 4]], c: usize) -> f64 {
        let n = region.len() as f64;
        let mean = region.iter().map(|p| f64::from(p[c])).sum::<f64>() / n;
        (region
            .iter()
            .map(|p| (f64::from(p[c]) - mean).powi(2))
            .sum::<f64>()
            / n)
            .sqrt()
    }

    #[test]
    fn reduce_noise_and_despeckle_smooth_grain_and_keep_edges_and_opacity() {
        let (w, h) = (40, 30);
        let noisy = grainy(w, h, 0.5, 0.05);
        for filter in [
            Filter::defaults("reduceNoise").unwrap(),
            Filter::defaults("despeckle").unwrap(),
        ] {
            let mut region = noisy.clone();
            filter.plan().kernel.region(&mut region, w, h);
            for c in 0..3 {
                let (before, after) = (deviation(&noisy, c), deviation(&region, c));
                assert!(after < before * 0.6, "{filter:?} {c}: {before} {after}");
            }
            assert!(region.iter().all(|p| p[3] == 1.0));
            // A step of black to white across the middle stays one.
            let mut step: Vec<[f32; 4]> = (0..w * h)
                .map(|i| {
                    let v = if i % w < w / 2 { 0.0 } else { 1.0 };
                    [v, v, v, 1.0]
                })
                .collect();
            filter.plan().kernel.region(&mut step, w, h);
            let row = 10 * w;
            assert!(
                step[row + w / 2 - 2][1] < 0.05 && step[row + w / 2 + 1][1] > 0.95,
                "{filter:?}"
            );
            assert!(!filter.reads_original());
        }
        // Reduce Color Noise: the colors' grain smoothed, the lightness left.
        let colors_only = Filter::ReduceNoise {
            strength: 0.0,
            preserve: 0.0,
            color: 100.0,
            sharpen: 0.0,
            reduction: 1.0,
        };
        let mut region = noisy.clone();
        colors_only.plan().kernel.region(&mut region, w, h);
        let split = |r: &[[f32; 4]]| {
            let planes: Vec<[f32; 4]> = r
                .iter()
                .map(|p| {
                    let y = LUMA[0] * p[0] + LUMA[1] * p[1] + LUMA[2] * p[2];
                    [p[0] - y, y, 0.0, 1.0]
                })
                .collect();
            (deviation(&planes, 0), deviation(&planes, 1))
        };
        let ((red_before, luma_before), (red_after, luma_after)) = (split(&noisy), split(&region));
        assert!(red_after < red_before * 0.5, "{red_before} {red_after}");
        assert!(
            (luma_after - luma_before).abs() < 1e-4,
            "lightness untouched"
        );
        // Nothing asked: nothing changes.
        let none = Filter::ReduceNoise {
            strength: 0.0,
            preserve: 60.0,
            color: 0.0,
            sharpen: 0.0,
            reduction: 1.0,
        };
        let mut same = noisy.clone();
        none.plan().kernel.region(&mut same, w, h);
        for (a, b) in same.iter().zip(&noisy) {
            assert!((0..4).all(|c| (a[c] - b[c]).abs() < 1e-5));
        }
        // Sharpen Details: a light dot on a ramp lighter still.
        let dot = |sharpen| {
            let mut ramp: Vec<[f32; 4]> = (0..w * h)
                .map(|i| {
                    let v = (i % w) as f32 / w as f32;
                    [v, v, v, 1.0]
                })
                .collect();
            ramp[5 * w + 20] = [0.9, 0.9, 0.9, 1.0];
            Filter::ReduceNoise {
                strength: 0.0,
                preserve: 0.0,
                color: 0.0,
                sharpen,
                reduction: 1.0,
            }
            .plan()
            .kernel
            .region(&mut ramp, w, h);
            ramp[5 * w + 20][0]
        };
        assert!(dot(100.0) > dot(0.0) + 0.1);
        // Looks of reduced levels: the radii shrink, down to nothing.
        let far = Filter::defaults("despeckle").unwrap().scaled(8.0);
        assert_eq!(far.plan().kernel.reach(), 0);
    }

    #[test]
    fn smart_sharpen_pushes_edges_lens_blur_without_halos_and_spares_grain() {
        // A soft edge from 0.2 to 0.8 across a few pixels.
        let (w, h) = (40, 8);
        let edge: Vec<[f32; 4]> = (0..w * h)
            .map(|i| {
                let x = (i % w) as f32;
                let v = 0.2 + 0.6 / (1.0 + (-(x - 20.0) / 1.5).exp());
                [v, v, v, 1.0]
            })
            .collect();
        let sharpened = |remove| {
            let filter = Filter::SmartSharpen {
                amount: 300.0,
                radius: 2.0,
                noise: 0.0,
                remove,
                angle: 0.0,
            };
            let blurs: Vec<Vec<[f32; 4]>> = filter
                .plans()
                .iter()
                .map(|plan| {
                    let mut region = edge.clone();
                    plan.kernel.region(&mut region, w, h);
                    region
                })
                .collect();
            (0..w)
                .map(|x| {
                    let i = 4 * w + x;
                    let b: Vec<[f64; 4]> = blurs.iter().map(|r| r[i].map(f64::from)).collect();
                    filter.finish(edge[i].map(f64::from), &b, [0, 0])[0]
                })
                .collect::<Vec<f64>>()
        };
        let gaussian = sharpened(SharpenRemove::GaussianBlur);
        let lens = sharpened(SharpenRemove::LensBlur);
        let motion = sharpened(SharpenRemove::MotionBlur);
        let original = |x: usize| f64::from(edge[4 * w + x][0]);
        // Steeper across the edge each way.
        for row in [&gaussian, &lens, &motion] {
            assert!(
                row[22] - row[18] > original(22) - original(18) + 0.1,
                "{row:?}"
            );
        }
        // Gaussian Blur overshoots (halos); Lens Blur stays within the values around.
        let high = gaussian.iter().copied().fold(f64::MIN, f64::max);
        let low = gaussian.iter().copied().fold(f64::MAX, f64::min);
        assert!(high > 0.82 && low < 0.18, "{high} {low}");
        assert!(
            lens.iter().all(|v| (0.2 - 1e-6..=0.8 + 1e-6).contains(v)),
            "{lens:?}"
        );
        // Reduce Noise: a small difference (grain) pushed less, a large one about as much.
        let filter = |noise| Filter::SmartSharpen {
            amount: 100.0,
            radius: 1.0,
            noise,
            remove: SharpenRemove::GaussianBlur,
            angle: 0.0,
        };
        let push = |noise, d: f64| {
            filter(noise).finish([0.5 + d, 0.5, 0.5, 1.0], &[[0.5, 0.5, 0.5, 1.0]], [0, 0])[0]
                - 0.5
                - d
        };
        assert!(push(100.0, 0.01) < push(0.0, 0.01) * 0.2);
        assert!(push(100.0, 0.3) > push(0.0, 0.3) * 0.95);
        // Motion Blur: a line along the angle.
        let motion = Filter::SmartSharpen {
            amount: 100.0,
            radius: 5.0,
            noise: 0.0,
            remove: SharpenRemove::MotionBlur,
            angle: 135.0,
        };
        assert!(matches!(motion.plan().kernel, Kernel::Line(_)));
        assert_eq!(motion.plans().len(), 1);
        assert_eq!(filter(0.0).plans().len(), 1);
        assert_eq!(Filter::defaults("smartSharpen").unwrap().plans().len(), 3);
    }
}
