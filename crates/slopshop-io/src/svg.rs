//! SVG import (resvg, pure Rust): the drawing rasterized at a chosen resolution, as 8-bit sRGB
//! with premultiplied alpha, transparent where nothing is drawn. Its intrinsic size is in CSS
//! pixels (96 per inch): [`SVG_DPI`] renders it at that size. Text uses the system's fonts;
//! images it links are read next to the file; `.svgz` is unzipped.

use std::path::Path;
use std::sync::{Arc, OnceLock};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{self, Tree};
use slopshop_core::Size;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, SampleType};

use crate::orient::Orientation;
use crate::vector::{PageSize, Thumbnail};
use crate::{Decoded, ImportError, Imported, check_budget, finish};

/// CSS pixels per inch: an SVG rendered at this resolution has its intrinsic size.
pub const SVG_DPI: f32 = 96.0;

/// Whether the file looks like an SVG: its extension, or markup (a tag first, after a byte
/// order mark and white space) with an `<svg` element near the start. Binary files can hold
/// `<svg` too, in their metadata (a PNG's XMP or Content Credentials).
pub(crate) fn is_svg(head: &[u8], path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if matches!(extension.as_deref(), Some("svg" | "svgz")) {
        return true;
    }
    let text = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    let markup = text.trim_ascii_start().starts_with(b"<");
    markup && head.windows(4).any(|w| w == b"<svg")
}

/// The system's fonts, loaded once (it takes a moment) for every SVG with text.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    Arc::clone(FONTS.get_or_init(|| {
        let mut fonts = usvg::fontdb::Database::new();
        fonts.load_system_fonts();
        Arc::new(fonts)
    }))
}

/// The intrinsic size, rendered.
pub(crate) fn decode(path: &Path) -> Result<Decoded, ImportError> {
    SvgFile::open(path)?.decode(SVG_DPI)
}

/// An SVG file parsed for import; rendered on demand, shareable between threads.
pub struct SvgFile {
    tree: Tree,
}

impl std::fmt::Debug for SvgFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SvgFile")
            .field("size", &self.tree.size())
            .finish_non_exhaustive()
    }
}

impl SvgFile {
    pub fn open(path: &Path) -> Result<Self, ImportError> {
        let data = std::fs::read(path)?;
        let options = usvg::Options {
            resources_dir: path.parent().map(Path::to_path_buf),
            fontdb: fonts(),
            ..usvg::Options::default()
        };
        let tree = Tree::from_data(&data, &options)
            .map_err(|e| ImportError::Decode(format!("SVG: {e}")))?;
        Ok(Self { tree })
    }

    /// The intrinsic size, in points (1/72 inch) like a PDF page.
    pub fn size(&self) -> PageSize {
        let size = self.tree.size();
        let points = 72.0 / SVG_DPI;
        PageSize {
            width: size.width() * points,
            height: size.height() * points,
        }
    }

    /// The drawing at `dpi` (96: its intrinsic size).
    pub fn render(&self, dpi: f32) -> Result<Imported, ImportError> {
        finish(self.decode(dpi)?)
    }

    /// The drawing (straight alpha), its longer side `max_side` pixels.
    pub fn thumbnail(&self, max_side: u32) -> Result<Thumbnail, ImportError> {
        let size = self.size();
        let dpi = max_side as f32 / size.width.max(size.height) * 72.0;
        let pixmap = self.rasterize(dpi)?;
        Ok(Thumbnail {
            width: pixmap.width(),
            height: pixmap.height(),
            pixels: pixmap.take_demultiplied(),
        })
    }

    fn decode(&self, dpi: f32) -> Result<Decoded, ImportError> {
        let pixmap = self.rasterize(dpi)?;
        Ok(Decoded {
            size: Size::new(pixmap.width(), pixmap.height()),
            layout: ChannelLayout::Rgba,
            sample: SampleType::U8,
            alpha: AlphaMode::Premultiplied,
            icc: None,
            space: Some(ColorSpace::SRGB),
            orientation: Orientation::Normal,
            pixels: pixmap.take(),
            warnings: Vec::new(),
        })
    }

    fn rasterize(&self, dpi: f32) -> Result<Pixmap, ImportError> {
        if !(dpi.is_finite() && dpi > 0.0) {
            return Err(ImportError::Decode(format!(
                "SVG: invalid resolution {dpi}"
            )));
        }
        let (width, height) = self.size().pixels(dpi);
        let (width, height) = (width.max(1), height.max(1));
        check_budget(width, height, ChannelLayout::Rgba, SampleType::U8, 4)?;
        let mut pixmap =
            Pixmap::new(width, height).ok_or(ImportError::TooLarge { width, height })?;
        let size = self.tree.size();
        let transform =
            Transform::from_scale(width as f32 / size.width(), height as f32 / size.height());
        resvg::render(&self.tree, transform, &mut pixmap.as_mut());
        Ok(pixmap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/svg")
            .join(name)
    }

    fn pixel(decoded: &Decoded, x: u32, y: u32) -> [u8; 4] {
        let i = (y * decoded.size.width + x) as usize * 4;
        decoded.pixels[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn svg_files_can_be_shared_between_threads() {
        fn shareable<T: Send + Sync>() {}
        shareable::<SvgFile>();
    }

    #[test]
    fn the_drawing_renders_at_its_size_or_any_resolution() {
        let file = SvgFile::open(&fixture("shapes.svg")).unwrap();
        assert_eq!(
            file.size(),
            PageSize {
                width: 60.0,
                height: 30.0
            }
        );
        let page = file.decode(SVG_DPI).unwrap();
        assert_eq!(page.size, Size::new(80, 40));
        assert_eq!(pixel(&page, 10, 20), [255, 0, 0, 255]);
        assert_eq!(pixel(&page, 70, 20), [0, 0, 0, 0]);
        let large = file.decode(SVG_DPI * 4.0).unwrap();
        assert_eq!(large.size, Size::new(320, 160));
        assert_eq!(pixel(&large, 40, 80), [255, 0, 0, 255]);
    }

    #[test]
    fn compressed_files_and_detection() {
        let file = SvgFile::open(&fixture("shapes.svgz")).unwrap();
        assert_eq!(file.decode(SVG_DPI).unwrap().size, Size::new(80, 40));
        let head = std::fs::read(fixture("shapes.svg")).unwrap();
        assert!(is_svg(&head, Path::new("noextension")));
        assert!(!is_svg(b"\x89PNG", Path::new("a.png")));
        // A byte order mark and white space before the markup.
        let bom = [b"\xEF\xBB\xBF\n  ".as_slice(), &head].concat();
        assert!(is_svg(&bom, Path::new("noextension")));
    }

    #[test]
    fn binary_files_mentioning_svg_are_not_svg() {
        // A PNG whose metadata (here an iTXt chunk) holds `<svg`, as some generated images'
        // Content Credentials do: it was handed to the SVG parser, which refused it.
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let text = b"XML:com.adobe.xmp\0\0\0\0\0<x><svg xmlns='http://www.w3.org/2000/svg'/></x>";
        let mut chunk = (text.len() as u32).to_be_bytes().to_vec();
        chunk.extend(b"iTXt");
        chunk.extend(text);
        let crc = crc32(&chunk[4..]);
        chunk.extend(crc.to_be_bytes());
        // After the signature and the header chunk (8 + 25 bytes).
        png.splice(33..33, chunk);
        assert!(!is_svg(&png, Path::new("Image ChatGPT.png")));

        let path =
            std::env::temp_dir().join(format!("slopshop-svg-{}-meta.png", std::process::id()));
        std::fs::write(&path, &png).unwrap();
        let imported = crate::open_image(&path);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(imported.unwrap().image.size(), Size::new(3, 2));
    }

    /// PNG's chunk checksum (CRC-32 of ISO 3309).
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    #[test]
    fn thumbnails_fit_the_longer_side() {
        let file = SvgFile::open(&fixture("shapes.svg")).unwrap();
        let thumbnail = file.thumbnail(64).unwrap();
        assert_eq!((thumbnail.width, thumbnail.height), (64, 32));
    }

    #[test]
    fn opening_gives_the_intrinsic_size() {
        let imported = crate::open_image(&fixture("shapes.svg")).unwrap();
        assert_eq!(imported.image.size(), Size::new(80, 40));
        assert!(imported.warnings.is_empty());
    }

    #[test]
    fn broken_files_are_errors() {
        let path =
            std::env::temp_dir().join(format!("slopshop-svg-{}-broken.svg", std::process::id()));
        std::fs::write(&path, b"<svg><rect").unwrap();
        let result = SvgFile::open(&path);
        std::fs::remove_file(&path).ok();
        assert!(result.is_err());
    }
}
