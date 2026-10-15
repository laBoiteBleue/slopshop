//! Contours of layer styles (ADR 0032), as Photoshop's: a curve on 0–255 that reshapes an
//! effect's mask (Drop Shadow, Inner Shadow, Outer and Inner Glow, Satin) or Bevel and Emboss's
//! height (its Contour) and lighting (its Gloss Contour). Its input is the effect's mask (0 far
//! from the shape, 1 at it), its output what is drawn there.
//!
//! The curve goes through its points smoothly (a natural cubic spline, as the Curves
//! adjustment's), but at a corner point, where the curve breaks: each run between corners is a
//! spline of its own (two points: a straight line).

use crate::curve::{CURVE_LUT, CURVE_POINTS, Spline, lookup};

/// Points of a contour at most (as a curve's).
pub const CONTOUR_POINTS: usize = CURVE_POINTS;

/// A point of a contour: input `x` and output `y` on 0–255; at a `corner` the curve breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ContourPoint {
    pub x: u8,
    pub y: u8,
    pub corner: bool,
}

/// A point, smooth.
const fn smooth(x: u8, y: u8) -> ContourPoint {
    ContourPoint {
        x,
        y,
        corner: false,
    }
}

/// A point where the curve breaks.
const fn corner(x: u8, y: u8) -> ContourPoint {
    ContourPoint { x, y, corner: true }
}

/// A contour: 2 to [`CONTOUR_POINTS`] points, inputs strictly increasing. Before the first point
/// and after the last, the output stays flat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Contour {
    points: [ContourPoint; CONTOUR_POINTS],
    len: u8,
}

impl Default for Contour {
    fn default() -> Self {
        Self::LINEAR
    }
}

impl Contour {
    /// The straight line from 0 to 255: the effect as it is (Photoshop's Linear).
    pub const LINEAR: Contour = Contour::of(&[smooth(0, 0), smooth(255, 255)]);

    /// `points` as a contour, at compile time: they are valid.
    const fn of(points: &[ContourPoint]) -> Contour {
        let mut all = [smooth(0, 0); CONTOUR_POINTS];
        let mut i = 0;
        while i < points.len() {
            all[i] = points[i];
            i += 1;
        }
        Contour {
            points: all,
            len: points.len() as u8,
        }
    }

    /// The contour through `points`; `None` unless there are 2 to [`CONTOUR_POINTS`] of them with
    /// strictly increasing inputs.
    pub fn new(points: &[ContourPoint]) -> Option<Contour> {
        if !(2..=CONTOUR_POINTS).contains(&points.len())
            || points.windows(2).any(|w| w[0].x >= w[1].x)
        {
            return None;
        }
        Some(Contour::of(points))
    }

    pub fn points(&self) -> &[ContourPoint] {
        &self.points[..usize::from(self.len)]
    }

    /// Whether it changes nothing: its points on the diagonal, from 0 to 255.
    pub fn is_linear(&self) -> bool {
        let p = self.points();
        p.iter().all(|q| q.x == q.y) && p[0].x == 0 && p[p.len() - 1].x == 255
    }

    /// The preset it is, if it is one.
    pub fn preset(&self) -> Option<ContourPreset> {
        if self.is_linear() {
            return Some(ContourPreset::Linear);
        }
        ContourPreset::ALL
            .into_iter()
            .find(|p| p.contour().points() == self.points())
    }

    /// Its output at `x` (in `[0, 1]`), in `[0, 1]`.
    pub fn value(&self, x: f64) -> f64 {
        self.evaluator()(x)
    }

    /// Its lookup table: the contour at `i / (CURVE_LUT − 1)`.
    pub(crate) fn lut(&self) -> Vec<f32> {
        let value = self.evaluator();
        (0..CURVE_LUT)
            .map(|i| value(i as f64 / (CURVE_LUT - 1) as f64) as f32)
            .collect()
    }

    /// Its value at an input: a spline for each run between corners.
    fn evaluator(&self) -> impl Fn(f64) -> f64 {
        let p = self.points();
        let mut runs: Vec<(f64, Spline)> = Vec::new();
        let mut start = 0;
        for i in 1..p.len() {
            if p[i].corner || i == p.len() - 1 {
                let points = p[start..=i]
                    .iter()
                    .map(|q| (f64::from(q.x) / 255.0, f64::from(q.y) / 255.0))
                    .collect();
                runs.push((f64::from(p[i].x) / 255.0, Spline::new(points)));
                start = i;
            }
        }
        move |x: f64| {
            let x = if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
            // The first run ending at or after `x` (there is one: the last ends at the last
            // point, beyond which the spline stays flat).
            let run = runs
                .iter()
                .find(|(end, _)| x <= *end)
                .or(runs.last())
                .map(|(_, spline)| spline);
            run.map_or(x, |spline| spline.value(x))
        }
    }
}

/// Photoshop's contours (the Contour picker's defaults), drawn after its shapes: their exact
/// points are Photoshop's own; a document from Photoshop brings its contours' points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ContourPreset {
    Linear,
    Cone,
    ConeInverted,
    CoveDeep,
    CoveShallow,
    Gaussian,
    HalfRound,
    Ring,
    RingDouble,
    RollingSlopeDescending,
    RoundedSteps,
    Sawtooth,
}

impl ContourPreset {
    /// In the order of Photoshop's picker.
    pub const ALL: [ContourPreset; 12] = [
        ContourPreset::Linear,
        ContourPreset::Cone,
        ContourPreset::ConeInverted,
        ContourPreset::CoveDeep,
        ContourPreset::CoveShallow,
        ContourPreset::Gaussian,
        ContourPreset::HalfRound,
        ContourPreset::Ring,
        ContourPreset::RingDouble,
        ContourPreset::RollingSlopeDescending,
        ContourPreset::RoundedSteps,
        ContourPreset::Sawtooth,
    ];

    /// Its identifier (the UI's, the IPC's).
    pub fn id(self) -> &'static str {
        match self {
            ContourPreset::Linear => "linear",
            ContourPreset::Cone => "cone",
            ContourPreset::ConeInverted => "coneInverted",
            ContourPreset::CoveDeep => "coveDeep",
            ContourPreset::CoveShallow => "coveShallow",
            ContourPreset::Gaussian => "gaussian",
            ContourPreset::HalfRound => "halfRound",
            ContourPreset::Ring => "ring",
            ContourPreset::RingDouble => "ringDouble",
            ContourPreset::RollingSlopeDescending => "rollingSlopeDescending",
            ContourPreset::RoundedSteps => "roundedSteps",
            ContourPreset::Sawtooth => "sawtooth",
        }
    }

    pub fn from_id(id: &str) -> Option<ContourPreset> {
        ContourPreset::ALL.into_iter().find(|p| p.id() == id)
    }

    /// Its name in Photoshop (as its documents name it).
    pub fn photoshop_name(self) -> &'static str {
        match self {
            ContourPreset::Linear => "Linear",
            ContourPreset::Cone => "Cone",
            ContourPreset::ConeInverted => "Cone - Inverted",
            ContourPreset::CoveDeep => "Cove - Deep",
            ContourPreset::CoveShallow => "Cove - Shallow",
            ContourPreset::Gaussian => "Gaussian",
            ContourPreset::HalfRound => "Half Round",
            ContourPreset::Ring => "Ring",
            ContourPreset::RingDouble => "Ring - Double",
            ContourPreset::RollingSlopeDescending => "Rolling Slope - Descending",
            ContourPreset::RoundedSteps => "Rounded Steps",
            ContourPreset::Sawtooth => "Sawtooth 1",
        }
    }

    pub fn contour(self) -> Contour {
        const CONE: Contour = Contour::of(&[smooth(0, 0), corner(128, 255), smooth(255, 0)]);
        const CONE_INVERTED: Contour =
            Contour::of(&[smooth(0, 255), corner(128, 0), smooth(255, 255)]);
        // Concave: slow, then steep (a cove's profile).
        const COVE_DEEP: Contour = Contour::of(&[
            smooth(0, 0),
            smooth(128, 34),
            smooth(191, 86),
            smooth(230, 144),
            smooth(255, 255),
        ]);
        const COVE_SHALLOW: Contour =
            Contour::of(&[smooth(0, 0), smooth(128, 64), smooth(255, 255)]);
        // An S: soft at both ends.
        const GAUSSIAN: Contour = Contour::of(&[
            smooth(0, 0),
            smooth(64, 18),
            smooth(128, 128),
            smooth(191, 237),
            smooth(255, 255),
        ]);
        // Convex: steep, then flat (a rounded edge).
        const HALF_ROUND: Contour = Contour::of(&[
            smooth(0, 0),
            smooth(25, 111),
            smooth(64, 169),
            smooth(128, 221),
            smooth(255, 255),
        ]);
        const RING: Contour = Contour::of(&[smooth(0, 0), smooth(128, 255), smooth(255, 0)]);
        const RING_DOUBLE: Contour = Contour::of(&[
            smooth(0, 0),
            smooth(64, 255),
            smooth(128, 0),
            smooth(191, 255),
            smooth(255, 0),
        ]);
        const ROLLING_SLOPE: Contour = Contour::of(&[
            smooth(0, 255),
            smooth(64, 224),
            smooth(128, 128),
            smooth(176, 104),
            smooth(255, 0),
        ]);
        // Plateaus the spline rounds into steps.
        const ROUNDED_STEPS: Contour = Contour::of(&[
            smooth(0, 0),
            smooth(42, 0),
            smooth(85, 85),
            smooth(127, 85),
            smooth(170, 170),
            smooth(212, 170),
            smooth(255, 255),
        ]);
        const SAWTOOTH: Contour = Contour::of(&[
            smooth(0, 0),
            corner(127, 255),
            corner(128, 0),
            smooth(255, 255),
        ]);
        match self {
            ContourPreset::Linear => Contour::LINEAR,
            ContourPreset::Cone => CONE,
            ContourPreset::ConeInverted => CONE_INVERTED,
            ContourPreset::CoveDeep => COVE_DEEP,
            ContourPreset::CoveShallow => COVE_SHALLOW,
            ContourPreset::Gaussian => GAUSSIAN,
            ContourPreset::HalfRound => HALF_ROUND,
            ContourPreset::Ring => RING,
            ContourPreset::RingDouble => RING_DOUBLE,
            ContourPreset::RollingSlopeDescending => ROLLING_SLOPE,
            ContourPreset::RoundedSteps => ROUNDED_STEPS,
            ContourPreset::Sawtooth => SAWTOOTH,
        }
    }
}

/// Entries of a [`Shaping`]'s table: one for each value of a 16-bit mask.
const TABLE: usize = 1 << 16;

/// Below this input, an effect drawn outward whose contour does not start at 0 fades out, so
/// that it does not fill everything around the shape (Photoshop hides that edge).
const EDGE: f64 = 23.0 / 255.0;

/// How an effect's mask goes through its contour: first scaled to `range` percent of it
/// (Photoshop's Range: the contour spans the part of the mask up to it, what is beyond at its
/// end), then through the contour, its input in 1 % steps unless anti-aliased (Photoshop's
/// aliasing, as Krita reads it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Shaping {
    pub contour: Contour,
    pub anti_aliased: bool,
    /// Percent, 1 to 100.
    pub range: f64,
}

impl Shaping {
    /// Over the whole mask.
    pub(crate) fn new(contour: Contour, anti_aliased: bool) -> Self {
        Self {
            contour,
            anti_aliased,
            range: 100.0,
        }
    }

    /// Whether it changes nothing (a linear contour over the whole mask; its 1 % steps when not
    /// anti-aliased are not drawn: Linear costs nothing).
    pub(crate) fn is_identity(&self) -> bool {
        self.contour.is_linear() && self.range >= 100.0
    }

    /// Its table over a 16-bit mask's values; `hide_edge`: the mask is an effect drawn outward,
    /// faded out near 0 when the contour does not start at 0.
    pub(crate) fn table(&self, hide_edge: bool) -> Vec<u16> {
        let lut = self.contour.lut();
        let hide = hide_edge && lut[0] > 0.0;
        let scale = 100.0 / self.range.clamp(1.0, 100.0);
        let full = f64::from(u16::MAX);
        (0..TABLE)
            .map(|i| {
                let mut x = (i as f64 / full * scale).min(1.0);
                if !self.anti_aliased {
                    // 1 % steps; a small epsilon keeps exact steps where they are.
                    x = (x * 100.0 + 1e-9).floor() / 100.0;
                }
                let mut y = lookup(&lut, x);
                if hide {
                    y *= (x / EDGE).min(1.0);
                }
                (y.clamp(0.0, 1.0) * full).round() as u16
            })
            .collect()
    }
}

/// `v` (in `[0, 1]`) through a [`Shaping::table`].
pub(crate) fn through(table: &[u16], v: f64) -> f64 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    let full = f64::from(u16::MAX);
    f64::from(table[(v * full).round() as usize]) / full
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contours_need_two_to_sixteen_increasing_points() {
        assert_eq!(Contour::new(&[smooth(0, 0)]), None);
        assert_eq!(Contour::new(&[smooth(10, 0), smooth(10, 255)]), None);
        assert_eq!(Contour::new(&[smooth(0, 0); 17]), None);
        let c = Contour::new(&[smooth(0, 0), smooth(255, 255)]).unwrap();
        assert_eq!(c, Contour::LINEAR);
        assert!(c.is_linear());
        // On the diagonal with a middle point, corner or not: still linear.
        let c = Contour::new(&[smooth(0, 0), corner(100, 100), smooth(255, 255)]).unwrap();
        assert!(c.is_linear());
        assert_eq!(c.preset(), Some(ContourPreset::Linear));
    }

    #[test]
    fn linear_is_the_identity_and_corners_break_the_curve() {
        for x in [0.0, 0.2, 0.5, 0.93, 1.0] {
            assert!((Contour::LINEAR.value(x) - x).abs() < 1e-9);
        }
        // Cone: straight up to its corner, straight down.
        let cone = ContourPreset::Cone.contour();
        assert!((cone.value(64.0 / 255.0) - 0.5).abs() < 1e-3);
        assert!((cone.value(128.0 / 255.0) - 1.0).abs() < 1e-9);
        assert!((cone.value(192.0 / 255.0) - 0.498).abs() < 1e-2);
        // Ring, smooth through the same points, bulges: above the cone between them.
        let ring = ContourPreset::Ring.contour();
        assert!(ring.value(64.0 / 255.0) > 0.6);
        // Flat beyond its points, a number for anything.
        let c = Contour::new(&[smooth(50, 20), smooth(200, 220)]).unwrap();
        assert!((c.value(0.0) - 20.0 / 255.0).abs() < 1e-9);
        assert!((c.value(1.0) - 220.0 / 255.0).abs() < 1e-9);
        assert!((0.0..=1.0).contains(&c.value(f64::NAN)));
    }

    #[test]
    fn presets_are_distinct_valid_contours_known_by_id() {
        for p in ContourPreset::ALL {
            let c = p.contour();
            assert_eq!(Contour::new(c.points()), Some(c), "{p:?}");
            assert_eq!(c.preset(), Some(p));
            assert_eq!(ContourPreset::from_id(p.id()), Some(p));
            for i in 0..=20 {
                assert!((0.0..=1.0).contains(&c.value(f64::from(i) / 20.0)));
            }
        }
        assert_eq!(ContourPreset::from_id("nope"), None);
        let custom = Contour::new(&[smooth(0, 0), smooth(30, 200), smooth(255, 255)]).unwrap();
        assert_eq!(custom.preset(), None);
        // Gaussian is an S, Half Round above the diagonal, Cove under it.
        let at = |p: ContourPreset, x: f64| p.contour().value(x);
        assert!(at(ContourPreset::Gaussian, 0.2) < 0.2 && at(ContourPreset::Gaussian, 0.8) > 0.8);
        assert!(at(ContourPreset::HalfRound, 0.3) > 0.5);
        assert!(at(ContourPreset::CoveDeep, 0.6) < 0.3);
    }

    #[test]
    fn a_shaping_scales_to_its_range_steps_unless_anti_aliased_and_hides_the_edge() {
        let mid = |table: &[u16], v: f64| through(table, v);
        // Range 50: half the mask is the whole contour.
        let half = Shaping {
            range: 50.0,
            ..Shaping::new(Contour::LINEAR, true)
        };
        assert!(!half.is_identity());
        let table = half.table(false);
        assert!((mid(&table, 0.25) - 0.5).abs() < 1e-3);
        assert_eq!(mid(&table, 0.8), 1.0);
        // Not anti-aliased: 1 % steps.
        let stepped = Shaping::new(ContourPreset::Gaussian.contour(), false).table(false);
        assert_eq!(mid(&stepped, 0.5031), mid(&stepped, 0.5001));
        let smooth = Shaping::new(ContourPreset::Gaussian.contour(), true).table(false);
        assert_ne!(mid(&smooth, 0.5081), mid(&smooth, 0.5001));
        // Starting above 0: outward, it fades out toward 0; inward, not.
        let inverted = Shaping::new(ContourPreset::ConeInverted.contour(), true);
        assert_eq!(mid(&inverted.table(true), 0.0), 0.0);
        assert_eq!(mid(&inverted.table(false), 0.0), 1.0);
        assert!(Shaping::new(Contour::LINEAR, false).is_identity());
    }
}
