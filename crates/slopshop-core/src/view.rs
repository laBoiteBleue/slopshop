//! Mapping between output pixels (a viewport, a thumbnail, an exported region) and document
//! pixels. Rendering only ever needs the document area covered by the output.

use crate::geom::Size;

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

    /// Document coordinate of a continuous output coordinate.
    pub fn output_to_document(&self, x: f64, y: f64) -> [f64; 2] {
        [
            self.origin[0] + x * self.scale,
            self.origin[1] + y * self.scale,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
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
}
