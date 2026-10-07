//! Sources (ADR 0040): what pixel layers show, kept once and referenced. A source is
//! immutable: changing what layers show makes a new source and points them at it. Layers hold
//! their source by reference (`Arc`), so a source lives while a layer, the history or the
//! clipboard uses it, and duplicating a layer shares it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::raster::RasterImage;

/// Process-unique identity of a [`Source`]. Sources are immutable, so the id is valid for the
/// source's whole lifetime; a file numbers its sources on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SourceId(u64);

impl SourceId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

/// Content that layers show: for now an image, as imported, pasted or baked.
#[derive(Debug)]
pub struct Source {
    id: SourceId,
    /// What the user knows it by (a file's name); empty when nothing names it.
    name: String,
    image: Arc<RasterImage>,
}

/// Sources are immutable: the same source is the same allocation, told by its id.
impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Source {
    /// A new source showing `image`.
    pub fn new(image: Arc<RasterImage>, name: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            id: SourceId::next(),
            name: name.into(),
            image,
        })
    }

    pub fn id(&self) -> SourceId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn image(&self) -> &Arc<RasterImage> {
        &self.image
    }

    /// Another source with the same content (Make Unique): no pixel is copied.
    pub fn unique(&self) -> Arc<Self> {
        Self::new(Arc::clone(&self.image), self.name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::PixelFormat;
    use crate::geom::{Rect, Size};

    fn image() -> Arc<RasterImage> {
        Arc::new(
            RasterImage::from_placed(
                Size::new(10, 10),
                PixelFormat::RGBA8_SRGB,
                Rect::new(0, 0, 0, 0),
                &[],
                &[0, 0, 0, 0],
            )
            .expect("a transparent image is valid"),
        )
    }

    #[test]
    fn sources_are_told_apart_by_identity_not_content() {
        let image = image();
        let a = Source::new(Arc::clone(&image), "a.png");
        let b = Source::new(Arc::clone(&image), "a.png");
        assert_ne!(a.id(), b.id());
        assert_ne!(a, b);
        assert_eq!(a, Arc::clone(&a));
        assert_eq!(a.name(), "a.png");
    }

    #[test]
    fn a_unique_copy_shares_the_pixels_under_a_new_identity() {
        let a = Source::new(image(), "a.png");
        let b = a.unique();
        assert_ne!(a.id(), b.id());
        assert!(Arc::ptr_eq(a.image(), b.image()));
        assert_eq!(b.name(), "a.png");
    }
}
