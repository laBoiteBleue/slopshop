//! Layer > Align and Distribute, and the Move tool's buttons (one implementation): the selected
//! layers moved by whole document pixels so that the edges or centers of their pixels line up,
//! or spread evenly. As in Photoshop: several layers align on the box around them all, a single
//! layer on the canvas, and with a selection everything aligns on the selection's box.
//!
//! A layer counts by the box of its visible pixels (masks aside, as snapping, see
//! [`pick::visible_layer_bounds`]); a group counts as one, by the box of what is inside it.
//! Layers without such pixels (hidden, empty, fill and adjustment layers) stay where they are.

use crate::document::{Document, LayerId};
use crate::edit::{Edit, EditError};
use crate::pick::{self, Bounds};
use crate::transform::Affine;

/// What Layer > Align lines up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    HorizontalCenters,
    Right,
    Top,
    VerticalCenters,
    Bottom,
}

/// What Layer > Distribute spreads evenly between the first and the last layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Distribute {
    HorizontalCenters,
    VerticalCenters,
    /// Equal gaps between the layers, left to right.
    HorizontalSpacing,
    /// Equal gaps between the layers, top to bottom.
    VerticalSpacing,
}

/// Distribute needs this many layers: with two, the first and the last stay.
pub const DISTRIBUTE_MIN: usize = 3;

/// The layers that may move, with the box of their pixels: `ids` without those inside another
/// of them, in stacking order.
fn items(doc: &Document, ids: &[LayerId]) -> Result<Vec<(LayerId, Bounds)>, EditError> {
    if let Some(&unknown) = ids.iter().find(|&&id| doc.layer(id).is_none()) {
        return Err(EditError::UnknownLayer(unknown));
    }
    Ok(doc
        .outermost(ids)
        .into_iter()
        .filter_map(|id| pick::bounds_of(doc, &[id]).map(|b| (id, b)))
        .collect())
}

/// What the layers align on: the selection's box, else the canvas for a single layer, else the
/// box around them all.
fn reference(doc: &Document, items: &[(LayerId, Bounds)]) -> Option<Bounds> {
    if let Some(rect) = doc
        .selection()
        .and_then(|s| crate::selection::bounds(s.image()))
    {
        return Some(Bounds {
            left: i64::from(rect.x),
            top: i64::from(rect.y),
            right: rect.right() as i64,
            bottom: rect.bottom() as i64,
        });
    }
    if let [_] = items {
        let size = doc.size();
        return Some(Bounds {
            left: 0,
            top: 0,
            right: i64::from(size.width),
            bottom: i64::from(size.height),
        });
    }
    items.iter().map(|&(_, b)| b).reduce(|a, b| Bounds {
        left: a.left.min(b.left),
        top: a.top.min(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    })
}

/// The edit moving each layer by its `(dx, dy)` (document pixels), the still ones left out.
fn moves(
    doc: &Document,
    shifts: impl IntoIterator<Item = (LayerId, i64, i64)>,
) -> Result<Edit, EditError> {
    let mut edits = Vec::new();
    for (id, dx, dy) in shifts {
        if dx != 0 || dy != 0 {
            let by = Affine::translation(dx as f64, dy as f64);
            edits.push(Edit::transform_layers(doc, &[id], by)?);
        }
    }
    Ok(Edit::Batch(edits))
}

/// Half of `twice`, rounded half up (centers are kept doubled to stay whole).
fn half(twice: i64) -> i64 {
    (twice + 1).div_euclid(2)
}

/// The edit that aligns `ids` (Layer > Align). An empty batch when nothing moves.
pub fn align_layers(doc: &Document, ids: &[LayerId], align: Align) -> Result<Edit, EditError> {
    let items = items(doc, ids)?;
    let Some(to) = reference(doc, &items) else {
        return Ok(Edit::Batch(Vec::new()));
    };
    moves(
        doc,
        items.iter().map(|&(id, b)| {
            let (dx, dy) = match align {
                Align::Left => (to.left - b.left, 0),
                Align::Right => (to.right - b.right, 0),
                Align::HorizontalCenters => (half(to.left + to.right - b.left - b.right), 0),
                Align::Top => (0, to.top - b.top),
                Align::Bottom => (0, to.bottom - b.bottom),
                Align::VerticalCenters => (0, half(to.top + to.bottom - b.top - b.bottom)),
            };
            (id, dx, dy)
        }),
    )
}

/// The edit that distributes `ids` (Layer > Distribute): the first and the last layer (by
/// position) stay, the others move so that their centers, or the gaps between them, are evenly
/// spaced. An empty batch with fewer than [`DISTRIBUTE_MIN`] layers.
pub fn distribute_layers(
    doc: &Document,
    ids: &[LayerId],
    distribute: Distribute,
) -> Result<Edit, EditError> {
    let mut items = items(doc, ids)?;
    if items.len() < DISTRIBUTE_MIN {
        return Ok(Edit::Batch(Vec::new()));
    }
    let horizontal = matches!(
        distribute,
        Distribute::HorizontalCenters | Distribute::HorizontalSpacing
    );
    // Along the axis: (start, end) of each layer.
    let span = |b: &Bounds| {
        if horizontal {
            (b.left, b.right)
        } else {
            (b.top, b.bottom)
        }
    };
    let spacing = matches!(
        distribute,
        Distribute::HorizontalSpacing | Distribute::VerticalSpacing
    );
    // Ordered by center (spacing: by start), ties by stacking order.
    items.sort_by_key(|(_, b)| {
        let (start, end) = span(b);
        if spacing { start } else { start + end }
    });
    let n = items.len();
    let (first_start, first_end) = span(&items[0].1);
    let (last_start, last_end) = span(&items[n - 1].1);
    let steps = (n - 1) as f64;
    let targets: Vec<i64> = if spacing {
        // Each layer's start, the free room shared into equal gaps.
        let lengths: i64 = items.iter().map(|(_, b)| span(b).1 - span(b).0).sum();
        let gap = ((last_end - first_start) - lengths) as f64 / steps;
        let mut at = first_start as f64;
        items
            .iter()
            .map(|(_, b)| {
                let start = at.round() as i64;
                at += (span(b).1 - span(b).0) as f64 + gap;
                start
            })
            .collect()
    } else {
        // Each layer's start, its center evenly spaced (centers doubled: whole numbers).
        let (from, to) = (
            (first_start + first_end) as f64,
            (last_start + last_end) as f64,
        );
        items
            .iter()
            .enumerate()
            .map(|(k, (_, b))| {
                let center = from + (to - from) * k as f64 / steps;
                let (start, end) = span(b);
                ((center - (end - start) as f64) / 2.0).round() as i64
            })
            .collect()
    };
    moves(
        doc,
        items.iter().zip(targets).map(|(&(id, b), target)| {
            let shift = target - span(&b).0;
            if horizontal {
                (id, shift, 0)
            } else {
                (id, 0, shift)
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::blend::BlendMode;
    use crate::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};
    use crate::document::{Layer, LayerContent};
    use crate::geom::{Rect, Size};
    use crate::raster::RasterImage;
    use crate::selection::Selection;

    /// A 100 × 100 document with an opaque box at each rect, bottom to top.
    fn document(rects: &[Rect]) -> (Document, Vec<LayerId>) {
        let mut doc = Document::new(Size::new(100, 100));
        let format = PixelFormat {
            layout: ChannelLayout::Rgba,
            sample: SampleType::U8,
            color_space: ColorSpace::SRGB,
            alpha: AlphaMode::Straight,
        };
        let ids = rects
            .iter()
            .map(|&rect| {
                let pixels = [200u8, 10, 10, 255].repeat(rect.size().pixel_count() as usize);
                let image =
                    RasterImage::from_placed(doc.size(), format, rect, &pixels, &[0; 4]).unwrap();
                let id = doc.allocate_layer_id();
                let index = doc.layers().len();
                Edit::InsertLayer {
                    parent: None,
                    index,
                    layer: Layer {
                        style: None,
                        id,
                        name: "box".into(),
                        visible: true,
                        opacity: 1.0,
                        blend_mode: BlendMode::Normal,
                        mask: None,
                        clipped: false,
                        transform: crate::transform::Projective::IDENTITY,
                        content: LayerContent::raster(Arc::new(image)),
                    },
                }
                .apply(&mut doc)
                .unwrap();
                id
            })
            .collect();
        (doc, ids)
    }

    /// Where each of `ids` is after `edit` (left, top), then undone.
    fn after(doc: &mut Document, ids: &[LayerId], edit: Edit) -> Vec<(i64, i64)> {
        let undo = edit.apply(doc).unwrap();
        let places = ids
            .iter()
            .map(|&id| {
                let b = pick::bounds_of(doc, &[id]).unwrap();
                (b.left, b.top)
            })
            .collect();
        undo.apply(doc).unwrap();
        places
    }

    #[test]
    fn several_layers_align_on_the_box_around_them() {
        let (mut doc, ids) = document(&[
            Rect::new(10, 10, 10, 10),
            Rect::new(40, 30, 20, 20),
            Rect::new(70, 5, 10, 30),
        ]);
        let align = |doc: &mut Document, a| {
            let edit = align_layers(doc, &ids, a).unwrap();
            after(doc, &ids, edit)
        };
        assert_eq!(align(&mut doc, Align::Left), [(10, 10), (10, 30), (10, 5)]);
        assert_eq!(align(&mut doc, Align::Right), [(70, 10), (60, 30), (70, 5)]);
        // The box spans 10..80: centers at 45, rounded half up for odd sizes.
        assert_eq!(
            align(&mut doc, Align::HorizontalCenters),
            [(40, 10), (35, 30), (40, 5)]
        );
        assert_eq!(align(&mut doc, Align::Top), [(10, 5), (40, 5), (70, 5)]);
        assert_eq!(
            align(&mut doc, Align::Bottom),
            [(10, 40), (40, 30), (70, 20)]
        );
        // Already aligned: nothing moves.
        let edit = align_layers(&doc, &ids[..1], Align::Left).unwrap();
        edit.apply(&mut doc).unwrap();
        assert_eq!(
            align_layers(&doc, &ids[..1], Align::Left),
            Ok(Edit::Batch(Vec::new()))
        );
    }

    #[test]
    fn one_layer_aligns_on_the_canvas_and_a_selection_wins() {
        let (mut doc, ids) = document(&[Rect::new(10, 20, 10, 10), Rect::new(50, 50, 20, 20)]);
        let edit = align_layers(&doc, &ids[..1], Align::Right).unwrap();
        assert_eq!(after(&mut doc, &ids[..1], edit), [(90, 20)]);
        let edit = align_layers(&doc, &ids[..1], Align::VerticalCenters).unwrap();
        assert_eq!(after(&mut doc, &ids[..1], edit), [(10, 45)]);

        // A selection from (30, 40) to (60, 80).
        let gray = PixelFormat {
            layout: ChannelLayout::Gray,
            sample: SampleType::U16,
            color_space: ColorSpace::LINEAR_SRGB,
            alpha: AlphaMode::Straight,
        };
        let selected = Rect::new(30, 40, 30, 40);
        let coverage = [255u8; 2].repeat(selected.size().pixel_count() as usize);
        let image =
            RasterImage::from_placed(doc.size(), gray, selected, &coverage, &[0; 2]).unwrap();
        let selection = Selection::new(Arc::new(image)).unwrap();
        Edit::SetSelection {
            selection: Some(selection),
        }
        .apply(&mut doc)
        .unwrap();
        let edit = align_layers(&doc, &ids, Align::Left).unwrap();
        assert_eq!(after(&mut doc, &ids, edit), [(30, 20), (30, 50)]);
        let edit = align_layers(&doc, &ids, Align::Bottom).unwrap();
        assert_eq!(after(&mut doc, &ids, edit), [(10, 70), (50, 60)]);
    }

    #[test]
    fn distributing_spreads_centers_or_gaps_between_the_outer_layers() {
        // Out of order in the stack: distribution goes by position.
        let (mut doc, ids) = document(&[
            Rect::new(80, 0, 10, 10),
            Rect::new(0, 0, 10, 10),
            Rect::new(20, 0, 30, 10),
            Rect::new(60, 0, 10, 10),
        ]);
        let distribute = |doc: &mut Document, d| {
            let edit = distribute_layers(doc, &ids, d).unwrap();
            after(doc, &ids, edit)
        };
        // Centers from 5 to 85, every 80 / 3: 31.7 and 58.3.
        assert_eq!(
            distribute(&mut doc, Distribute::HorizontalCenters),
            [(80, 0), (0, 0), (17, 0), (53, 0)]
        );
        // Widths 10 + 30 + 10 + 10 = 60 in 90 pixels: gaps of 10.
        assert_eq!(
            distribute(&mut doc, Distribute::HorizontalSpacing),
            [(80, 0), (0, 0), (20, 0), (60, 0)]
        );
        // Nothing to spread vertically: all on one row.
        assert_eq!(
            distribute_layers(&doc, &ids, Distribute::VerticalSpacing),
            Ok(Edit::Batch(Vec::new()))
        );
        // Two layers: nothing moves.
        assert_eq!(
            distribute_layers(&doc, &ids[..2], Distribute::HorizontalCenters),
            Ok(Edit::Batch(Vec::new()))
        );
    }

    #[test]
    fn layers_without_pixels_and_unknown_ids() {
        let (mut doc, ids) = document(&[Rect::new(10, 10, 10, 10), Rect::new(50, 50, 10, 10)]);
        Edit::SetLayerVisible {
            id: ids[1],
            visible: false,
        }
        .apply(&mut doc)
        .unwrap();
        // The hidden layer stays: the other one alone aligns on the canvas.
        let edit = align_layers(&doc, &ids, Align::Left).unwrap();
        assert_eq!(after(&mut doc, &ids[..1], edit), [(0, 10)]);
        let ghost = LayerId::from_raw(999);
        assert_eq!(
            align_layers(&doc, &[ghost], Align::Left),
            Err(EditError::UnknownLayer(ghost))
        );
    }
}
