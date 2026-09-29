//! Mapping between output pixels (a viewport, a thumbnail, an exported region) and document
//! pixels, and the interactive [`Viewport`] (fit, zoom, pan). Rendering only ever needs the
//! document area covered by the output.
//!
//! Output coordinates are device pixels: a zoom of 1.0 (100%) shows one document pixel per
//! screen pixel.

use crate::geom::Size;

/// Smallest zoom (0.1%): one output pixel covers 1000 document pixels.
pub const MIN_ZOOM: f64 = 0.001;
/// Largest zoom (6400%).
pub const MAX_ZOOM: f64 = 64.0;

/// Zoom presets used by zoom in/out steps, as factors (1.0 = 100%).
const ZOOM_STEPS: [f64; 26] = [
    0.001,
    0.0025,
    0.005,
    0.01,
    0.02,
    0.03,
    0.04,
    0.05,
    0.0625,
    1.0 / 12.0,
    0.125,
    1.0 / 6.0,
    0.25,
    1.0 / 3.0,
    0.5,
    2.0 / 3.0,
    1.0,
    2.0,
    3.0,
    4.0,
    5.0,
    6.0,
    8.0,
    16.0,
    32.0,
    64.0,
];

/// An affine, axis-aligned mapping: `document = origin + output * scale`, where `output` is
/// the continuous coordinate of an output pixel (its center is at `x + 0.5`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    /// Document coordinate of the output's top-left corner. May be negative (pasteboard).
    pub origin: [f64; 2],
    /// Document pixels per output pixel (> 1 when zoomed out).
    pub scale: f64,
}

impl ViewTransform {
    /// Fit the whole document inside `output`, centered, keeping `margin` output pixels free
    /// on each side when possible. Never returns a degenerate transform.
    pub fn fit(document: Size, output: Size, margin: u32) -> Self {
        let avail = |o: u32| f64::from(o.saturating_sub(margin.saturating_mul(2)).max(1));
        let doc_w = f64::from(document.width.max(1));
        let doc_h = f64::from(document.height.max(1));
        let scale = (doc_w / avail(output.width)).max(doc_h / avail(output.height));
        let out_w = f64::from(output.width.max(1));
        let out_h = f64::from(output.height.max(1));
        Self {
            origin: [(doc_w - out_w * scale) / 2.0, (doc_h - out_h * scale) / 2.0],
            scale,
        }
    }

    /// Output pixels per document pixel (1.0 = 100%).
    pub fn zoom(&self) -> f64 {
        1.0 / self.scale
    }

    /// Document coordinate of a continuous output coordinate.
    pub fn output_to_document(&self, x: f64, y: f64) -> [f64; 2] {
        [
            self.origin[0] + x * self.scale,
            self.origin[1] + y * self.scale,
        ]
    }

    /// Continuous output coordinate of a document coordinate.
    pub fn document_to_output(&self, x: f64, y: f64) -> [f64; 2] {
        [
            (x - self.origin[0]) / self.scale,
            (y - self.origin[1]) / self.scale,
        ]
    }
}

/// Direction of a preset zoom step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomStep {
    In,
    Out,
}

/// The next preset zoom strictly beyond `zoom` in `direction`, clamped to the zoom range.
pub fn step_zoom(zoom: f64, direction: ZoomStep) -> f64 {
    const EPSILON: f64 = 1e-6;
    match direction {
        ZoomStep::In => ZOOM_STEPS
            .iter()
            .copied()
            .find(|&z| z > zoom * (1.0 + EPSILON))
            .unwrap_or(MAX_ZOOM),
        ZoomStep::Out => ZOOM_STEPS
            .iter()
            .rev()
            .copied()
            .find(|&z| z < zoom * (1.0 - EPSILON))
            .unwrap_or(MIN_ZOOM),
    }
}

/// An interactive view onto a document. View state is not document state: it is never part of
/// the undo history.
///
/// A viewport is either in *fit* mode (the whole document, re-fitted on every resize) or free
/// (after any zoom or pan). Free views keep at least a strip of the document visible.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    output: Size,
    transform: ViewTransform,
    fit: bool,
}

impl Viewport {
    /// Output pixels kept free around the document in fit mode.
    pub const FIT_MARGIN: u32 = 24;
    /// Output pixels of the document that always stay visible when panning or zooming.
    pub const MIN_VISIBLE: f64 = 48.0;

    /// A viewport fitting the whole document.
    pub fn new(document: Size, output: Size) -> Self {
        Self {
            output,
            transform: Self::fitted(document, output),
            fit: true,
        }
    }

    /// The fit transform, with its zoom kept within [`MIN_ZOOM`, `MAX_ZOOM`] so that zoom
    /// steps from "fit" always go in the requested direction (a tiny icon is not shown beyond
    /// the maximum zoom, a gigantic image not below the minimum). Stays centered.
    fn fitted(document: Size, output: Size) -> ViewTransform {
        let fit = ViewTransform::fit(document, output, Self::FIT_MARGIN);
        let scale = fit.scale.clamp(1.0 / MAX_ZOOM, 1.0 / MIN_ZOOM);
        let doc_w = f64::from(document.width.max(1));
        let doc_h = f64::from(document.height.max(1));
        let out_w = f64::from(output.width.max(1));
        let out_h = f64::from(output.height.max(1));
        ViewTransform {
            origin: [(doc_w - out_w * scale) / 2.0, (doc_h - out_h * scale) / 2.0],
            scale,
        }
    }

    pub fn output(&self) -> Size {
        self.output
    }

    pub fn transform(&self) -> ViewTransform {
        self.transform
    }

    pub fn zoom(&self) -> f64 {
        self.transform.zoom()
    }

    pub fn is_fit(&self) -> bool {
        self.fit
    }

    /// Adapt to a new output (window resize) or document size. Fit views re-fit; free views
    /// keep the document point at the center of the output fixed.
    pub fn resize(&mut self, document: Size, output: Size) {
        if self.fit {
            self.transform = Self::fitted(document, output);
        } else {
            let center = self.center_in_document();
            let scale = self.transform.scale;
            self.transform.origin = [
                center[0] - f64::from(output.width) / 2.0 * scale,
                center[1] - f64::from(output.height) / 2.0 * scale,
            ];
        }
        self.output = output;
        self.keep_visible(document);
    }

    /// Show the whole document, and keep doing so on resize.
    pub fn fit(&mut self, document: Size) {
        self.fit = true;
        self.transform = Self::fitted(document, self.output);
    }

    /// Set the zoom (clamped to [`MIN_ZOOM`, `MAX_ZOOM`]) keeping the document point under the
    /// output position `anchor` fixed. Non-finite zooms are ignored.
    pub fn set_zoom_about(&mut self, document: Size, anchor: [f64; 2], zoom: f64) {
        if !zoom.is_finite() || zoom <= 0.0 || !anchor.iter().all(|v| v.is_finite()) {
            return;
        }
        let pinned = self.transform.output_to_document(anchor[0], anchor[1]);
        let scale = 1.0 / zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.transform = ViewTransform {
            origin: [pinned[0] - anchor[0] * scale, pinned[1] - anchor[1] * scale],
            scale,
        };
        self.fit = false;
        self.keep_visible(document);
    }

    /// Set the zoom around the center of the output (e.g. "100%").
    pub fn set_zoom(&mut self, document: Size, zoom: f64) {
        self.set_zoom_about(document, self.output_center(), zoom);
    }

    /// Multiply the zoom by `factor` around `anchor` (e.g. mouse wheel, pinch).
    pub fn zoom_by(&mut self, document: Size, anchor: [f64; 2], factor: f64) {
        self.set_zoom_about(document, anchor, self.zoom() * factor);
    }

    /// Step to the next preset zoom around `anchor` (or the output center).
    pub fn step(&mut self, document: Size, anchor: Option<[f64; 2]>, direction: ZoomStep) {
        let anchor = anchor.unwrap_or_else(|| self.output_center());
        self.set_zoom_about(document, anchor, step_zoom(self.zoom(), direction));
    }

    /// Move the content by an output-pixel delta (the content follows the pointer).
    pub fn pan(&mut self, document: Size, dx: f64, dy: f64) {
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }
        let scale = self.transform.scale;
        self.transform.origin[0] -= dx * scale;
        self.transform.origin[1] -= dy * scale;
        self.fit = false;
        self.keep_visible(document);
    }

    fn output_center(&self) -> [f64; 2] {
        [
            f64::from(self.output.width) / 2.0,
            f64::from(self.output.height) / 2.0,
        ]
    }

    fn center_in_document(&self) -> [f64; 2] {
        let [x, y] = self.output_center();
        self.transform.output_to_document(x, y)
    }

    /// Clamp the origin so that at least [`Self::MIN_VISIBLE`] output pixels of the document
    /// (or the whole document if it is smaller) stay inside the output on each axis.
    fn keep_visible(&mut self, document: Size) {
        let scale = self.transform.scale;
        let axes = [
            (document.width, self.output.width),
            (document.height, self.output.height),
        ];
        for (axis, (doc_len, out_len)) in axes.into_iter().enumerate() {
            let doc_len = f64::from(doc_len) / scale; // in output pixels
            let out_len = f64::from(out_len);
            let visible = Self::MIN_VISIBLE.min(doc_len).min(out_len);
            // Document span in output coordinates: [start, start + doc_len].
            let start = -self.transform.origin[axis] / scale;
            let clamped = start.clamp(visible - doc_len, out_len - visible);
            self.transform.origin[axis] = -clamped * scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    fn close2(a: [f64; 2], b: [f64; 2]) -> bool {
        close(a[0], b[0]) && close(a[1], b[1])
    }

    #[test]
    fn fit_wide_document_in_square_output() {
        let t = ViewTransform::fit(Size::new(2000, 1000), Size::new(100, 100), 0);
        assert!(close(t.scale, 20.0));
        // Horizontally flush, vertically centered.
        assert_eq!(t.output_to_document(0.0, 0.0), [0.0, -500.0]);
        let [x, y] = t.output_to_document(100.0, 50.0);
        assert!(close(x, 2000.0) && close(y, 500.0));
    }

    #[test]
    fn fit_respects_margin() {
        let t = ViewTransform::fit(Size::new(100, 100), Size::new(120, 120), 10);
        assert!(close(t.scale, 1.0));
        assert_eq!(t.output_to_document(10.0, 10.0), [0.0, 0.0]);
    }

    #[test]
    fn fit_degenerate_inputs_stay_finite() {
        for (doc, out) in [
            (Size::new(0, 0), Size::new(0, 0)),
            (Size::new(10, 10), Size::new(5, 5)),
            (Size::new(u32::MAX, 1), Size::new(1, 1)),
        ] {
            let t = ViewTransform::fit(doc, out, 50);
            assert!(t.scale.is_finite() && t.scale > 0.0);
            assert!(t.origin.iter().all(|v| v.is_finite()));
        }
    }

    #[test]
    fn document_output_round_trip() {
        let t = ViewTransform {
            origin: [-12.5, 40.0],
            scale: 3.0,
        };
        let [x, y] = t.document_to_output(100.0, 200.0);
        assert!(close2(t.output_to_document(x, y), [100.0, 200.0]));
    }

    #[test]
    fn zoom_steps() {
        assert_eq!(step_zoom(1.0, ZoomStep::In), 2.0);
        assert_eq!(step_zoom(1.0, ZoomStep::Out), 2.0 / 3.0);
        // From an arbitrary (fit) zoom, go to the neighboring presets.
        assert_eq!(step_zoom(0.3, ZoomStep::In), 1.0 / 3.0);
        assert_eq!(step_zoom(0.3, ZoomStep::Out), 0.25);
        assert_eq!(step_zoom(MAX_ZOOM, ZoomStep::In), MAX_ZOOM);
        assert_eq!(step_zoom(MIN_ZOOM, ZoomStep::Out), MIN_ZOOM);
    }

    const DOC: Size = Size::new(4000, 3000);
    const OUT: Size = Size::new(800, 600);

    #[test]
    fn new_viewport_fits() {
        let v = Viewport::new(DOC, OUT);
        assert!(v.is_fit());
        // (800 - 48) / 4000 = 0.188 and (600 - 48) / 3000 = 0.184: height limits.
        assert!(close(v.zoom(), 552.0 / 3000.0));
    }

    #[test]
    fn fit_zoom_stays_in_range_so_steps_go_the_right_way() {
        // An icon: the raw fit would be ~6900%, above the maximum.
        let icon = Size::new(8, 8);
        let mut v = Viewport::new(icon, OUT);
        assert!(close(v.zoom(), MAX_ZOOM));
        v.step(icon, None, ZoomStep::In);
        assert!(v.zoom() >= MAX_ZOOM, "zoom in never zooms out");
        // The clamped fit stays centered.
        v.fit(icon);
        let [x, y] = v.transform().document_to_output(4.0, 4.0);
        assert!(close(x, 400.0) && close(y, 300.0));

        // A gigantic image in a tiny output: the raw fit would be below the minimum.
        let huge = Size::new(6000, 4000);
        let mut v = Viewport::new(huge, Size::new(1, 1));
        assert!(close(v.zoom(), MIN_ZOOM));
        v.step(huge, None, ZoomStep::Out);
        assert!(v.zoom() <= MIN_ZOOM + 1e-12, "zoom out never zooms in");
    }

    #[test]
    fn zoom_keeps_anchor_fixed() {
        let mut v = Viewport::new(DOC, OUT);
        let anchor = [300.0, 200.0];
        let before = v.transform().output_to_document(anchor[0], anchor[1]);
        v.zoom_by(DOC, anchor, 2.0);
        assert!(!v.is_fit());
        assert!(close(v.zoom(), 2.0 * 552.0 / 3000.0));
        let after = v.transform().output_to_document(anchor[0], anchor[1]);
        assert!(close2(before, after));
    }

    #[test]
    fn zoom_is_clamped_and_ignores_garbage() {
        let mut v = Viewport::new(DOC, OUT);
        v.set_zoom(DOC, 1e9);
        assert!(close(v.zoom(), MAX_ZOOM));
        v.set_zoom(DOC, 1e-9);
        assert!(close(v.zoom(), MIN_ZOOM));
        let before = v;
        v.set_zoom(DOC, f64::NAN);
        v.set_zoom(DOC, -1.0);
        v.zoom_by(DOC, [f64::INFINITY, 0.0], 2.0);
        v.pan(DOC, f64::NAN, 0.0);
        assert_eq!(v, before);
    }

    #[test]
    fn set_zoom_centers_on_output_center() {
        let mut v = Viewport::new(DOC, OUT);
        let center = v.transform().output_to_document(400.0, 300.0);
        v.set_zoom(DOC, 1.0);
        assert!(close(v.zoom(), 1.0));
        assert!(close2(
            v.transform().output_to_document(400.0, 300.0),
            center
        ));
    }

    #[test]
    fn pan_moves_content_with_pointer() {
        let mut v = Viewport::new(DOC, OUT);
        v.set_zoom(DOC, 1.0);
        let before = v.transform().document_to_output(2000.0, 1500.0);
        v.pan(DOC, 30.0, -20.0);
        let after = v.transform().document_to_output(2000.0, 1500.0);
        assert!(close2(after, [before[0] + 30.0, before[1] - 20.0]));
    }

    #[test]
    fn document_never_leaves_the_output() {
        let mut v = Viewport::new(DOC, OUT);
        v.set_zoom(DOC, 1.0);
        v.pan(DOC, 1e7, 1e7);
        // The document's top-left corner can go at most MIN_VISIBLE px from the right/bottom.
        let corner = v.transform().document_to_output(0.0, 0.0);
        assert!(close2(corner, [800.0 - 48.0, 600.0 - 48.0]));
        v.pan(DOC, -1e8, -1e8);
        let far = v.transform().document_to_output(4000.0, 3000.0);
        assert!(close2(far, [48.0, 48.0]));
    }

    #[test]
    fn tiny_document_stays_entirely_visible() {
        let doc = Size::new(10, 10);
        let mut v = Viewport::new(doc, OUT);
        v.set_zoom(doc, 1.0);
        v.pan(doc, 1e6, 0.0);
        let [x, _] = v.transform().document_to_output(0.0, 0.0);
        assert!(
            close(x, 800.0 - 10.0),
            "10 px document fully visible at the edge"
        );
    }

    #[test]
    fn resize_refits_in_fit_mode_and_keeps_center_otherwise() {
        let mut v = Viewport::new(DOC, OUT);
        let bigger = Size::new(1600, 1200);
        v.resize(DOC, bigger);
        assert!(v.is_fit());
        assert!(close(v.zoom(), (1200.0 - 48.0) / 3000.0));

        v.set_zoom(DOC, 1.0);
        let center = v.transform().output_to_document(800.0, 600.0);
        v.resize(DOC, OUT);
        assert!(close2(
            v.transform().output_to_document(400.0, 300.0),
            center
        ));
        assert!(close(v.zoom(), 1.0));

        v.fit(DOC);
        assert!(v.is_fit());
        assert!(close(v.zoom(), 552.0 / 3000.0));
    }
}
