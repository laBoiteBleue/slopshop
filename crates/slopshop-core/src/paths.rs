//! The document's paths (ADR 0041, amendment "the Paths panel"): Photoshop's Work Path and
//! saved paths, kept by the document rather than by a layer. Their geometry is a path
//! ([`Geometry::Path`]) in document pixels; canvas operations transform it with the image.
//! A vector layer's path and a vector mask stay the layer's: the Paths panel lists them beside
//! these while their layer is active.

use std::fmt;

use crate::shape::{FillRule, Geometry};

/// Stable identifier of a document path. Never reused, even after undo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PathId(u64);

impl PathId {
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Rebuild an id received from outside (the UI, a file); edits validate it.
    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }
}

impl fmt::Display for PathId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "path#{}", self.0)
    }
}

/// A path of the document: a saved path (named), or the Work Path (`work`), the temporary one
/// the Pen draws in Path mode, which the next one replaces unless it is saved. A document has
/// one Work Path at most.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentPath {
    pub id: PathId,
    /// What the panel shows; the Work Path's is the UI's ("Work Path"), kept empty.
    pub name: String,
    /// Its subpaths, in document pixels: always [`Geometry::Path`], possibly without any
    /// (Photoshop's New Path is empty until drawn in).
    pub geometry: Geometry,
    pub work: bool,
}

impl DocumentPath {
    /// A path with nothing in it yet (Photoshop's New Path), filled by the nonzero rule.
    pub fn empty(id: PathId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            geometry: Geometry::Path {
                subpaths: Vec::new(),
                rule: FillRule::NonZero,
            },
            work: false,
        }
    }

    /// A path, with valid coordinates.
    pub fn is_valid(&self) -> bool {
        matches!(self.geometry, Geometry::Path { .. }) && self.geometry.is_valid()
    }
}

/// Whether `paths` can be a document's: valid paths, ids unique, not 0 and below `next_id`,
/// one Work Path at most.
pub fn valid_paths(paths: &[DocumentPath], next_id: u64) -> bool {
    let mut seen = std::collections::HashSet::with_capacity(paths.len());
    paths.iter().all(|p| {
        let id = p.id.get();
        p.is_valid() && id != 0 && id < next_id && seen.insert(id)
    }) && paths.iter().filter(|p| p.work).count() <= 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::{Segment, Subpath};

    fn triangle(id: u64, work: bool) -> DocumentPath {
        DocumentPath {
            id: PathId::from_raw(id),
            name: format!("Path {id}"),
            geometry: Geometry::Path {
                subpaths: vec![Subpath {
                    start: [0.0, 0.0],
                    segments: vec![Segment::Line([10.0, 0.0]), Segment::Line([0.0, 10.0])],
                    closed: true,
                }],
                rule: FillRule::NonZero,
            },
            work,
        }
    }

    #[test]
    fn a_documents_paths_have_unique_allocated_ids_and_one_work_path_at_most() {
        assert!(valid_paths(&[triangle(1, false), triangle(2, true)], 3));
        assert!(valid_paths(
            &[DocumentPath::empty(PathId::from_raw(1), "Path 1")],
            2
        ));
        // Not allocated, repeated, 0, two Work Paths.
        assert!(!valid_paths(&[triangle(3, false)], 3));
        assert!(!valid_paths(&[triangle(1, false), triangle(1, false)], 3));
        assert!(!valid_paths(&[triangle(0, false)], 3));
        assert!(!valid_paths(&[triangle(1, true), triangle(2, true)], 3));
        // Only paths: a live shape is a layer's.
        let mut live = triangle(1, false);
        live.geometry = Geometry::Ellipse {
            center: [5.0, 5.0],
            radii: [5.0, 5.0],
        };
        assert!(!valid_paths(&[live], 2));
        let mut far = triangle(1, false);
        far.geometry = Geometry::Path {
            subpaths: vec![Subpath {
                start: [f64::NAN, 0.0],
                segments: Vec::new(),
                closed: false,
            }],
            rule: FillRule::EvenOdd,
        };
        assert!(!valid_paths(&[far], 2));
    }
}
