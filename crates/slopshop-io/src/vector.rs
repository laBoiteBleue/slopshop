//! Vector files rasterized on import (PDF, SVG), behind one interface for the import dialog:
//! pages with their sizes in points, thumbnails, and any page at any resolution.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use slopshop_core::color::LinearRgba;
use slopshop_core::document::LayerContent;

use crate::pdf::{self, PdfFile};
use crate::svg::{self, SvgFile};
use crate::{ImportError, Imported, Opened, adjusted};

/// Lengths in points: 72 per inch.
pub const POINTS_PER_INCH: f32 = 72.0;

/// A page's size in points (1/72 inch), its rotation applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

impl PageSize {
    /// The size in pixels at `dpi`, as the page is rendered: rounded, so that a resolution
    /// computed from a size in pixels gives that size back.
    pub fn pixels(self, dpi: f32) -> (u32, u32) {
        let scale = dpi / POINTS_PER_INCH;
        // Float-to-integer `as` saturates.
        (
            (self.width * scale).round() as u32,
            (self.height * scale).round() as u32,
        )
    }
}

/// A small RGBA8 image (straight alpha, sRGB), for previews.
#[derive(Debug)]
pub struct Thumbnail {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// A PDF (pages) or an SVG (one page).
#[derive(Debug)]
pub enum VectorFile {
    Pdf(PdfFile),
    /// Boxed: a parsed SVG tree is much larger than a PDF handle.
    Svg(Box<SvgFile>),
}

impl VectorFile {
    /// The file at `path`, recognized by its content (an SVG also by its extension); `None`
    /// for other files.
    pub fn open(path: &Path) -> Result<Option<Self>, ImportError> {
        let mut head = Vec::with_capacity(4096);
        File::open(path)?.take(4096).read_to_end(&mut head)?;
        if pdf::is_pdf(&head) {
            Ok(Some(Self::Pdf(PdfFile::open(path)?)))
        } else if svg::is_svg(&head, path) {
            Ok(Some(Self::Svg(Box::new(SvgFile::open(path)?))))
        } else {
            Ok(None)
        }
    }

    /// "pdf" or "svg", for the dialog.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Pdf(_) => "pdf",
            Self::Svg(_) => "svg",
        }
    }

    /// The resolution used when nobody chose one.
    pub fn default_dpi(&self) -> f32 {
        match self {
            Self::Pdf(_) => pdf::DEFAULT_DPI,
            Self::Svg(_) => svg::SVG_DPI,
        }
    }

    pub fn page_count(&self) -> usize {
        match self {
            Self::Pdf(file) => file.page_count(),
            Self::Svg(_) => 1,
        }
    }

    pub fn page_sizes(&self) -> Vec<PageSize> {
        match self {
            Self::Pdf(file) => file.page_sizes(),
            Self::Svg(file) => vec![file.size()],
        }
    }

    /// Page `index` (from 0), its longer side `max_side` pixels.
    pub fn thumbnail(&self, index: usize, max_side: u32) -> Result<Thumbnail, ImportError> {
        match self {
            Self::Pdf(file) => file.thumbnail(index, max_side),
            Self::Svg(file) if index == 0 => file.thumbnail(max_side),
            Self::Svg(_) => Err(no_page(index)),
        }
    }

    /// Page `index` (from 0) at `dpi`.
    pub fn render(&self, index: usize, dpi: f32) -> Result<Imported, ImportError> {
        match self {
            Self::Pdf(file) => file.render(index, dpi),
            Self::Svg(file) if index == 0 => file.render(dpi),
            Self::Svg(_) => Err(no_page(index)),
        }
    }
}

impl VectorFile {
    /// `pages` (from 0) at `dpi` as one document named after `stem`, the maintainer's layout:
    /// for a PDF, one isolated group of the pages (the first on top, named "stem 3/12") over a
    /// white fill named `background`; an SVG is its drawing alone. Pages render in parallel.
    pub fn open_pages(
        &self,
        stem: &str,
        pages: &[usize],
        dpi: f32,
        background: &str,
    ) -> Result<Opened, ImportError> {
        if let Self::Svg(file) = self {
            return file.render(dpi).map(Opened::Image);
        }
        let count = self.page_count();
        let rendered: Vec<Result<Imported, ImportError>> = std::thread::scope(|scope| {
            let workers: Vec<_> = pages
                .iter()
                .map(|&page| scope.spawn(move || self.render(page, dpi)))
                .collect();
            workers
                .into_iter()
                .map(|worker| {
                    worker.join().unwrap_or_else(|_| {
                        Err(ImportError::Decode("PDF renderer panicked".into()))
                    })
                })
                .collect()
        });
        let mut images = Vec::with_capacity(pages.len());
        for (&page, imported) in pages.iter().zip(rendered) {
            let name = if count == 1 {
                stem.to_owned()
            } else {
                format!("{stem} {}/{count}", page + 1)
            };
            images.push((name, imported?));
        }
        let white = LayerContent::Fill {
            color: LinearRgba::new(1.0, 1.0, 1.0, 1.0),
        };
        adjusted::grouped(
            stem.to_owned(),
            images,
            None,
            Some((background.to_owned(), white)),
        )
    }
}

fn no_page(index: usize) -> ImportError {
    ImportError::Decode(format!("there is no page {}", index + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_pages_open_as_one_group_over_a_white_background() {
        let pdf = VectorFile::open(&fixture("pdf/pages.pdf"))
            .unwrap()
            .unwrap();
        let Opened::Layers(layers) = pdf
            .open_pages("pages", &[0, 2], 72.0, "Background")
            .unwrap()
        else {
            panic!("expected layers");
        };
        let document = layers.document;
        assert_eq!(document.layers().len(), 1);
        let group = &document.layers()[0];
        let LayerContent::Group {
            children,
            pass_through: false,
        } = &group.content
        else {
            panic!("expected an isolated group");
        };
        let names: Vec<&str> = children.iter().rev().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["pages 1/3", "pages 3/3", "Background"]);
        assert!(matches!(children[0].content, LayerContent::Fill { .. }));
        // The canvas fits the largest page: 72 × 36 and 200 × 50 points at 72 dpi.
        assert_eq!(document.size(), slopshop_core::Size::new(200, 50));
        assert_eq!(layers.layer_warnings.len(), 4);
    }

    fn fixture(path: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join(path)
    }

    #[test]
    fn pdf_and_svg_open_behind_one_interface() {
        let pdf = VectorFile::open(&fixture("pdf/pages.pdf"))
            .unwrap()
            .unwrap();
        assert_eq!((pdf.kind(), pdf.page_count()), ("pdf", 3));
        let svg = VectorFile::open(&fixture("svg/shapes.svg"))
            .unwrap()
            .unwrap();
        assert_eq!((svg.kind(), svg.page_count()), ("svg", 1));
        assert_eq!(svg.default_dpi(), 96.0);
        assert!(svg.render(1, 96.0).is_err());
        assert!(matches!(
            svg.open_pages("shapes", &[0], 96.0, "Background"),
            Ok(Opened::Image(_))
        ));
        assert!(
            VectorFile::open(&fixture("jxl/rgb8.jxl"))
                .unwrap()
                .is_none()
        );
    }
}
