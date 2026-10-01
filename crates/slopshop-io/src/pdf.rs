//! PDF import (hayro, pure Rust): pages rasterized at a chosen resolution, as 8-bit sRGB with
//! premultiplied alpha. The page itself is transparent, like Photoshop's import: only what it
//! draws is opaque. The renderer works in sRGB: other color spaces (CMYK, ICC-based, Lab) are
//! converted by it. Fonts that are not embedded are replaced by the standard ones the renderer
//! carries. Encrypted files are not supported yet.
//!
//! [`open_image`](crate::open_image) gives the first page at [`DEFAULT_DPI`]; [`PdfFile`] lists
//! the pages, renders thumbnails and any page at any resolution.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::vello_cpu::color::AlphaColor;
use hayro::vello_cpu::color::PremulRgba8;
use hayro::vello_cpu::color::Srgb;
use hayro::vello_cpu::color::palette::css::{TRANSPARENT, WHITE};
use hayro::{RenderCache, RenderSettings};
use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, SampleType};

use crate::orient::Orientation;
use crate::{Decoded, ImportError, ImportWarning, Imported, check_budget, finish};

/// The resolution pages are rendered at when nobody chose one (Photoshop's default).
pub const DEFAULT_DPI: f32 = 300.0;

/// PDF lengths are in points.
const POINTS_PER_INCH: f32 = 72.0;

/// Whether `head` starts like a PDF file.
pub(crate) fn is_pdf(head: &[u8]) -> bool {
    head.starts_with(b"%PDF")
}

/// The first page at [`DEFAULT_DPI`], reporting the others.
pub(crate) fn decode(path: &Path) -> Result<Decoded, ImportError> {
    let file = PdfFile::open(path)?;
    let mut decoded = file.decode(0, DEFAULT_DPI)?;
    if file.page_count() > 1 {
        decoded.warnings.insert(0, ImportWarning::FirstPageOnly);
    }
    Ok(decoded)
}

/// A page's size in points (1/72 inch), its rotation applied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: f32,
    pub height: f32,
}

impl PageSize {
    /// The size in pixels at `dpi`, as the page is rendered.
    pub fn pixels(self, dpi: f32) -> (u32, u32) {
        let scale = dpi / POINTS_PER_INCH;
        // Float-to-integer `as` saturates.
        (
            (self.width * scale).floor() as u32,
            (self.height * scale).floor() as u32,
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

/// A PDF file read for import. Pages are rendered on demand; it can be shared between threads
/// to render several pages at once.
pub struct PdfFile {
    pdf: Pdf,
}

impl std::fmt::Debug for PdfFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PdfFile")
            .field("pages", &self.page_count())
            .finish_non_exhaustive()
    }
}

impl PdfFile {
    /// Read and parse the file; nothing is rendered yet.
    pub fn open(path: &Path) -> Result<Self, ImportError> {
        let pdf = Pdf::new(std::fs::read(path)?).map_err(|e| match e {
            LoadPdfError::Decryption(_) => ImportError::NotYetSupported("encrypted PDF"),
            LoadPdfError::Invalid => ImportError::Decode("PDF: the file cannot be read".to_owned()),
        })?;
        if pdf.pages().is_empty() {
            return Err(ImportError::Decode("PDF: the file has no pages".to_owned()));
        }
        Ok(Self { pdf })
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    pub fn page_sizes(&self) -> Vec<PageSize> {
        self.pdf
            .pages()
            .iter()
            .map(|page| {
                let (width, height) = page.render_dimensions();
                PageSize { width, height }
            })
            .collect()
    }

    /// Page `index` (from 0) at `dpi`.
    pub fn render(&self, index: usize, dpi: f32) -> Result<Imported, ImportError> {
        finish(self.decode(index, dpi)?)
    }

    /// Page `index` on white, scaled so that its longer side is `max_side` pixels.
    pub fn thumbnail(&self, index: usize, max_side: u32) -> Result<Thumbnail, ImportError> {
        let size = self.page_size(index)?;
        let longer = size.width.max(size.height);
        let dpi = max_side as f32 / longer * POINTS_PER_INCH;
        let (pixmap, _) = self.rasterize(index, dpi, WHITE)?;
        Ok(Thumbnail {
            width: u32::from(pixmap.width()),
            height: u32::from(pixmap.height()),
            // Opaque on white: premultiplied and straight alpha are the same.
            pixels: bytes(pixmap.take()),
        })
    }

    fn page_size(&self, index: usize) -> Result<PageSize, ImportError> {
        let page =
            self.pdf.pages().get(index).ok_or_else(|| {
                ImportError::Decode(format!("PDF: there is no page {}", index + 1))
            })?;
        let (width, height) = page.render_dimensions();
        Ok(PageSize { width, height })
    }

    fn decode(&self, index: usize, dpi: f32) -> Result<Decoded, ImportError> {
        let (pixmap, skipped) = self.rasterize(index, dpi, TRANSPARENT)?;
        let size = Size::new(u32::from(pixmap.width()), u32::from(pixmap.height()));
        Ok(Decoded {
            size,
            layout: ChannelLayout::Rgba,
            sample: SampleType::U8,
            alpha: AlphaMode::Premultiplied,
            icc: None,
            space: Some(ColorSpace::SRGB),
            orientation: Orientation::Normal,
            pixels: bytes(pixmap.take()),
            warnings: if skipped {
                vec![ImportWarning::PdfContentSkipped]
            } else {
                Vec::new()
            },
        })
    }

    /// Render page `index` at `dpi` over `background`; also tells whether some content could
    /// not be rendered.
    fn rasterize(
        &self,
        index: usize,
        dpi: f32,
        background: AlphaColor<Srgb>,
    ) -> Result<(hayro::vello_cpu::Pixmap, bool), ImportError> {
        if !(dpi.is_finite() && dpi > 0.0) {
            return Err(ImportError::Decode(format!(
                "PDF: invalid resolution {dpi}"
            )));
        }
        let size = self.page_size(index)?;
        let (width, height) = size.pixels(dpi);
        // The renderer's pixmaps are at most 65,535 pixels a side.
        let max = u32::from(u16::MAX);
        if width > max || height > max {
            return Err(ImportError::TooLarge { width, height });
        }
        check_budget(
            width.max(1),
            height.max(1),
            ChannelLayout::Rgba,
            SampleType::U8,
            4,
        )?;
        let page = &self.pdf.pages()[index];
        let skipped = Arc::new(AtomicBool::new(false));
        let sink = Arc::clone(&skipped);
        let settings = InterpreterSettings {
            warning_sink: Arc::new(move |_| sink.store(true, Ordering::Relaxed)),
            ..InterpreterSettings::default()
        };
        let scale = dpi / POINTS_PER_INCH;
        // Checked above: both fit in u16.
        let (width, height) = (width.max(1) as u16, height.max(1) as u16);
        let pixmap = hayro::render(
            page,
            &RenderCache::new(),
            &settings,
            &RenderSettings {
                x_scale: scale,
                y_scale: scale,
                width: Some(width),
                height: Some(height),
                bg_color: background,
            },
        );
        Ok((pixmap, skipped.load(Ordering::Relaxed)))
    }
}

/// The pixels as bytes, without copying them (both are 1-byte aligned, 4 bytes a pixel).
fn bytes(pixels: Vec<PremulRgba8>) -> Vec<u8> {
    bytemuck::allocation::try_cast_vec(pixels)
        .unwrap_or_else(|(_, pixels)| bytemuck::cast_slice(&pixels).to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/pdf/pages.pdf")
    }

    fn pixel(decoded: &Decoded, x: u32, y: u32) -> [u8; 4] {
        let i = (y * decoded.size.width + x) as usize * 4;
        decoded.pixels[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn pdf_files_can_be_shared_between_threads() {
        fn shareable<T: Send + Sync>() {}
        shareable::<PdfFile>();
    }

    #[test]
    fn pages_list_their_sizes_rotation_applied() {
        let file = PdfFile::open(&fixture()).unwrap();
        let sizes = file.page_sizes();
        assert_eq!(
            sizes,
            [
                PageSize {
                    width: 72.0,
                    height: 36.0
                },
                PageSize {
                    width: 100.0,
                    height: 50.0
                },
                PageSize {
                    width: 200.0,
                    height: 50.0
                },
            ]
        );
        assert_eq!(sizes[0].pixels(300.0), (300, 150));
    }

    #[test]
    fn pages_render_at_the_chosen_resolution_transparent_where_empty() {
        let file = PdfFile::open(&fixture()).unwrap();
        let page = file.decode(0, 144.0).unwrap();
        assert_eq!(page.size, Size::new(144, 72));
        assert_eq!(pixel(&page, 10, 10), [255, 0, 0, 255]);
        assert_eq!(pixel(&page, 130, 60), [0, 0, 0, 0]);
        assert!(page.warnings.is_empty(), "{:?}", page.warnings);

        let rotated = file.decode(1, 72.0).unwrap();
        assert_eq!(rotated.size, Size::new(100, 50));
        assert_eq!(pixel(&rotated, 50, 25), [0, 0, 255, 255]);

        let imported = file.render(0, 72.0).unwrap();
        assert_eq!(imported.image.format().color_space, ColorSpace::SRGB);
        assert_eq!(imported.image.format().alpha, AlphaMode::Premultiplied);
    }

    #[test]
    fn text_in_standard_fonts_is_drawn() {
        let file = PdfFile::open(&fixture()).unwrap();
        let text = file.decode(2, 72.0).unwrap();
        let inked = text
            .pixels
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] == 255 && p[0] < 20)
            .count();
        assert!(inked > 200, "{inked}");
    }

    #[test]
    fn thumbnails_fit_the_longer_side_on_white() {
        let file = PdfFile::open(&fixture()).unwrap();
        let thumbnail = file.thumbnail(0, 64).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (64, 32));
        assert_eq!(&thumbnail.pixels[..4], [255, 0, 0, 255]);
        let last = thumbnail.pixels.len() - 4;
        assert_eq!(&thumbnail.pixels[last..], [255, 255, 255, 255]);
    }

    #[test]
    fn opening_a_pdf_gives_its_first_page_at_the_default_resolution() {
        let imported = crate::open_image(&fixture()).unwrap();
        assert_eq!(imported.image.size(), Size::new(300, 150));
        assert_eq!(imported.warnings, [ImportWarning::FirstPageOnly]);
    }

    #[test]
    fn bad_requests_and_damaged_files_are_errors() {
        let file = PdfFile::open(&fixture()).unwrap();
        assert!(file.decode(3, 72.0).is_err());
        assert!(file.decode(0, 0.0).is_err());
        assert!(file.decode(0, f32::NAN).is_err());
        assert!(matches!(
            file.decode(0, 100_000.0),
            Err(ImportError::TooLarge { .. })
        ));

        let path =
            std::env::temp_dir().join(format!("slopshop-pdf-{}-garbage.pdf", std::process::id()));
        std::fs::write(&path, b"%PDF-1.4\nnot really a pdf").unwrap();
        let result = PdfFile::open(&path);
        std::fs::remove_file(&path).ok();
        assert!(result.is_err());
    }
}
