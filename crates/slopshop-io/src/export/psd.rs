//! Layered Photoshop export (`docs/formats.md`, PSD stage 4): a PSD file whose layers are the
//! document's, with a merged composite for readers that do not read layers.
//!
//! Every pixel layer is rendered on its own by the caller's compositor (`render`: the GPU or
//! the CPU, as for flat exports), placed and resampled by its transform (ADR 0017, 0018) and
//! converted to the file's samples; masks are rendered the same way, as coverage. Layers keep
//! their name, visibility, opacity, blend mode (ADR 0012), clipping (ADR 0016) and mask
//! (ADR 0014); groups (ADR 0015) are Photoshop groups, pass-through or isolated; adjustment
//! layers (ADR 0020) are Photoshop adjustment layers with the same settings; fill layers are
//! pixel layers covering the canvas. Pixels outside the canvas are cropped (reported). 8 and
//! 16 bits per sample, RGB with transparency, the color space tagged with an ICC profile, RLE
//! (PackBits) compression. Documents above PSD's 30,000 pixels per side (PSB) are refused.
//!
//! The file is built in memory (compressed), then written through a temporary file like the
//! other exports.

use std::io::Write;
use std::path::Path;

use slopshop_core::adjust::Adjustment;
use slopshop_core::color::{AlphaMode, ChannelLayout, ColorSpace, PixelFormat, SampleType};
use slopshop_core::convert::{ConversionReport, ConvertOptions, Converter};
use slopshop_core::document::{Document, Layer, LayerContent, LayerId, LayerMask};
use slopshop_core::{BlendMode, CancelToken, LinearRgba, Progress, Rect, Size};

use super::{BAND_ROWS, ExportError, ExportNotice, ExportReport, WHITE_MATTE};
use crate::atomic::TempFile;
use crate::icc;

/// Largest side of a PSD file (PSB goes beyond).
pub const MAX_SIDE: u32 = 30_000;

/// Samples of a layered PSD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PsdDepth {
    U8,
    U16,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PsdOptions {
    pub depth: PsdDepth,
    /// The file's color space, tagged with an ICC profile (matrix/TRC spaces only).
    pub space: ColorSpace,
    /// Blue-noise dither for 8-bit samples.
    pub dither: bool,
}

/// Premultiplied working-space pixels of `region` of a document (as `export_image`'s source),
/// returning the non-finite samples it replaced.
pub type Render<'a> = dyn FnMut(&Document, Rect, &mut [f32]) -> Result<u64, String> + 'a;

/// Write `document` as a layered PSD to `path`. `render` composites a region of a document (the
/// document itself for the composite, one-layer documents for the layers and masks).
/// `progress` counts rendered rows. Blocking and CPU/GPU-heavy: call it off the UI thread.
pub fn export_psd(
    path: &Path,
    document: &Document,
    options: &PsdOptions,
    render: &mut Render<'_>,
    cancel: &CancelToken,
    progress: &mut dyn FnMut(Progress),
) -> Result<ExportReport, ExportError> {
    let size = document.size();
    if size.is_empty() {
        return Err(ExportError::InvalidSpec("empty image".into()));
    }
    if size.width > MAX_SIDE || size.height > MAX_SIDE {
        return Err(ExportError::TooLarge {
            width: size.width,
            height: size.height,
        });
    }
    let profile = icc::write_matrix_trc(&options.space)
        .map_err(|_| ExportError::UnsupportedSpace(options.space))?;
    if path.file_name().is_none() {
        return Err(ExportError::InvalidSpec(format!(
            "{} is not a file path",
            path.display()
        )));
    }
    let sample = match options.depth {
        PsdDepth::U8 => SampleType::U8,
        PsdDepth::U16 => SampleType::U16,
    };
    let converter = |layout: ChannelLayout| {
        Converter::new(
            PixelFormat {
                layout,
                sample,
                color_space: options.space,
                alpha: AlphaMode::Straight,
            },
            ConvertOptions {
                dither: options.dither,
                big_endian: true,
                matte: WHITE_MATTE,
                blend_space: document.blend_space(),
            },
        )
        .map_err(|e| ExportError::InvalidSpec(e.to_string()))
    };
    let mut writer = Writer {
        document,
        rgba: converter(ChannelLayout::Rgba)?,
        sample_bytes: if sample == SampleType::U8 { 1 } else { 2 },
        render,
        cancel,
        progress,
        rows_done: 0,
        rows_total: 0,
        conversion: ConversionReport::default(),
        non_finite: 0,
        cropped: false,
    };
    writer.rows_total = writer.count_rows(document.layers()) + u64::from(size.height);

    let mut records = Vec::new();
    writer.layers(document.layers(), &mut records)?;
    // Photoshop stores the merged composite over white, and readers remove the white.
    let composite = writer.render_planes(document, size.bounds(), true)?;

    let mut file = Vec::new();
    header(&mut file, size, options.depth);
    // Color mode data: none for RGB.
    file.extend(0u32.to_be_bytes());
    image_resources(&mut file, &profile);
    layer_and_mask_info(&mut file, &records, options.depth);
    merged_image(&mut file, &composite);

    let (temp, handle) = TempFile::create(path)?;
    let mut out = std::io::BufWriter::new(handle);
    out.write_all(&file)?;
    let handle = out
        .into_inner()
        .map_err(|e| ExportError::Io(e.into_error()))?;
    handle.sync_all()?;
    drop(handle);
    temp.persist(path)?;

    let mut report = ExportReport::from_conversion(&writer.conversion);
    if writer.non_finite > 0 {
        report.add_non_finite(writer.non_finite);
    }
    if writer.cropped {
        report.notices.push(ExportNotice::PixelsOutsideCanvas);
    }
    Ok(report)
}

/// Compressed planes of an area: one per channel, RLE rows.
struct Planes {
    channels: Vec<Plane>,
}

struct Plane {
    /// Compressed length of each row.
    counts: Vec<u16>,
    data: Vec<u8>,
}

impl Plane {
    fn new() -> Self {
        Self {
            counts: Vec::new(),
            data: Vec::new(),
        }
    }

    /// Length in the file: compression, row lengths, rows.
    fn len(&self) -> usize {
        2 + 2 * self.counts.len() + self.data.len()
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend(1u16.to_be_bytes());
        for count in &self.counts {
            out.extend(count.to_be_bytes());
        }
        out.extend(&self.data);
    }
}

/// One layer record and its channels, in Photoshop's order (bottom to top).
struct Record {
    name: String,
    bounds: Rect,
    /// `(id, plane)`: −1 transparency, 0–2 red, green, blue, −2 the mask; `None` for a channel
    /// without pixels.
    channels: Vec<(i16, Option<Plane>)>,
    blend_key: [u8; 4],
    opacity: u8,
    clipped: bool,
    hidden: bool,
    /// The mask's area and whether it is disabled.
    mask: Option<(Rect, bool)>,
    /// Tagged blocks after the name (`luni` is added when writing).
    blocks: Vec<([u8; 4], Vec<u8>)>,
}

struct Writer<'a, 'r> {
    document: &'a Document,
    rgba: Converter,
    sample_bytes: usize,
    render: &'a mut Render<'r>,
    cancel: &'a CancelToken,
    progress: &'a mut dyn FnMut(Progress),
    rows_done: u64,
    rows_total: u64,
    conversion: ConversionReport,
    non_finite: u64,
    cropped: bool,
}

impl Writer<'_, '_> {
    /// Rows to render for `layers` (for progress): each pixel layer's area, each mask's.
    fn count_rows(&self, layers: &[Layer]) -> u64 {
        let height = u64::from(self.document.size().height);
        layers
            .iter()
            .map(|layer| {
                let mask = if layer.mask.is_some() { height } else { 0 };
                mask + match &layer.content {
                    LayerContent::Group { children, .. } => self.count_rows(children),
                    LayerContent::Raster { .. } | LayerContent::Fill { .. } => height,
                    LayerContent::Adjustment { .. } => 0,
                }
            })
            .sum()
    }

    /// Records of `layers` (bottom to top), groups as a divider, their layers, then the group.
    fn layers(&mut self, layers: &[Layer], out: &mut Vec<Record>) -> Result<(), ExportError> {
        for layer in layers {
            if let LayerContent::Group {
                children,
                pass_through,
            } = &layer.content
            {
                let mut divider =
                    self.record(layer, "</Layer group>".into(), Rect::new(0, 0, 0, 0));
                divider.blocks.push((*b"lsct", 3u32.to_be_bytes().to_vec()));
                divider.mask = None;
                divider.channels = empty_channels();
                out.push(divider);
                self.layers(children, out)?;
                let mut group = self.record(layer, layer.name.clone(), Rect::new(0, 0, 0, 0));
                group.channels = empty_channels();
                let mut section = 1u32.to_be_bytes().to_vec();
                section.extend(b"8BIM");
                if *pass_through {
                    group.blend_key = *b"pass";
                }
                section.extend(group.blend_key);
                group.blocks.push((*b"lsct", section));
                self.add_mask(layer, &mut group)?;
                out.push(group);
                continue;
            }
            let bounds = match &layer.content {
                LayerContent::Raster { .. } => self.content_bounds(layer),
                LayerContent::Fill { .. } => self.document.size().bounds(),
                _ => Rect::new(0, 0, 0, 0),
            };
            let mut record = self.record(layer, layer.name.clone(), bounds);
            match &layer.content {
                LayerContent::Raster { .. } | LayerContent::Fill { .. } => {
                    let planes = if bounds.is_empty() {
                        None
                    } else {
                        let alone = self.isolated(layer);
                        Some(self.render_planes(&alone, bounds, false)?)
                    };
                    record.channels = match planes {
                        Some(planes) => {
                            let mut channels: Vec<Option<Plane>> =
                                planes.channels.into_iter().map(Some).collect();
                            // Planes come R, G, B, A; records list transparency first.
                            let alpha = channels.pop().flatten();
                            let mut ordered = vec![(-1, alpha)];
                            ordered.extend((0..3).zip(channels.drain(..)));
                            ordered
                        }
                        None => empty_channels(),
                    };
                }
                LayerContent::Adjustment { adjustment } => {
                    record.channels = empty_channels();
                    record.blocks.extend(adjustment_blocks(adjustment));
                }
                LayerContent::Group { .. } => {}
            }
            self.add_mask(layer, &mut record)?;
            out.push(record);
        }
        Ok(())
    }

    /// A record of `layer`'s common properties.
    fn record(&self, layer: &Layer, name: String, bounds: Rect) -> Record {
        Record {
            name,
            bounds,
            channels: Vec::new(),
            blend_key: blend_key(layer.blend_mode),
            opacity: (layer.opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
            clipped: layer.clipped,
            hidden: !layer.visible,
            mask: None,
            blocks: Vec::new(),
        }
    }

    /// Where `layer`'s pixels are on the canvas (cropped to it, reported).
    fn content_bounds(&mut self, layer: &Layer) -> Rect {
        let Some(b) = slopshop_core::pick::bounds_of(self.document, &[layer.id]) else {
            return Rect::new(0, 0, 0, 0);
        };
        let size = self.document.size();
        let (w, h) = (i64::from(size.width), i64::from(size.height));
        if b.left < 0 || b.top < 0 || b.right > w || b.bottom > h {
            self.cropped = true;
        }
        let (x0, y0) = (b.left.clamp(0, w), b.top.clamp(0, h));
        let (x1, y1) = (b.right.clamp(0, w), b.bottom.clamp(0, h));
        // Within the canvas: the values fit in u32.
        Rect::new(x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32)
    }

    /// A document holding only `layer`, placed as in the document, at full opacity, in normal
    /// mode, unmasked (its mask is written apart): its own pixels.
    fn isolated(&self, layer: &Layer) -> Document {
        let mut copy = layer.clone();
        copy.id = LayerId::from_raw(1);
        copy.visible = true;
        copy.opacity = 1.0;
        copy.blend_mode = BlendMode::Normal;
        copy.clipped = false;
        copy.transform = layer
            .transform
            .then(self.document.parent_transform(layer.id));
        // A mask made from the transparency still replaces the layer's alpha, not shown.
        copy.mask = layer
            .mask
            .as_ref()
            .filter(|m| m.replaces_alpha)
            .map(|m| LayerMask {
                enabled: false,
                ..m.clone()
            });
        self.single(copy)
    }

    fn single(&self, layer: Layer) -> Document {
        let document = self.document;
        Document::restore(
            document.size(),
            document.working_space(),
            document.blend_space(),
            vec![layer],
            2,
        )
        .unwrap_or_else(|_| Document::new(document.size()))
    }

    /// `layer`'s mask as channel −2 over the canvas: the coverage of a white fill it masks.
    fn add_mask(&mut self, layer: &Layer, record: &mut Record) -> Result<(), ExportError> {
        let Some(mask) = &layer.mask else {
            return Ok(());
        };
        if mask.replaces_alpha {
            // The transparency moved to the mask: Photoshop gets it back as the layer's alpha
            // (rendered above), no mask.
            return Ok(());
        }
        let transform = layer
            .transform
            .then(self.document.parent_transform(layer.id));
        let coverage = Layer {
            id: LayerId::from_raw(1),
            name: String::new(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: Some(LayerMask {
                enabled: true,
                ..mask.clone()
            }),
            clipped: false,
            transform,
            content: LayerContent::Fill {
                color: LinearRgba::new(1.0, 1.0, 1.0, 1.0),
            },
        };
        let area = self.document.size().bounds();
        let alone = self.single(coverage);
        let planes = self.render_planes(&alone, area, false)?;
        let alpha = planes.channels.into_iter().nth(3);
        record.channels.push((-2, alpha));
        record.mask = Some((area, !mask.enabled));
        Ok(())
    }

    /// `area` of `document`, rendered in bands, converted to straight RGBA samples (colors
    /// composited over white if `matte`) and split into four RLE-compressed planes (R, G, B, A).
    fn render_planes(
        &mut self,
        document: &Document,
        area: Rect,
        matte: bool,
    ) -> Result<Planes, ExportError> {
        let mut channels: Vec<Plane> = (0..4).map(|_| Plane::new()).collect();
        let width = area.width as usize;
        let mut pixels = Vec::new();
        let mut row = vec![0u8; width * 4 * self.sample_bytes];
        let mut plane_row = vec![0u8; width * self.sample_bytes];
        let mut y = area.y;
        while y < area.y + area.height {
            if self.cancel.is_cancelled() {
                return Err(ExportError::Cancelled);
            }
            let rows = BAND_ROWS.min(area.y + area.height - y);
            let band = Rect::new(area.x, y, area.width, rows);
            pixels.resize(width * rows as usize * 4, 0.0);
            self.non_finite +=
                (self.render)(document, band, &mut pixels).map_err(ExportError::Source)?;
            for r in 0..rows as usize {
                let src = &pixels[r * width * 4..(r + 1) * width * 4];
                self.rgba
                    .convert_row(src, area.x, y + r as u32, &mut row, &mut self.conversion)
                    .map_err(|e| ExportError::Encode(e.to_string()))?;
                let bytes = self.sample_bytes;
                if matte {
                    matte_white(&mut row, bytes);
                }
                for (c, plane) in channels.iter_mut().enumerate() {
                    for (x, out) in plane_row.chunks_exact_mut(bytes).enumerate() {
                        let at = (x * 4 + c) * bytes;
                        out.copy_from_slice(&row[at..at + bytes]);
                    }
                    let start = plane.data.len();
                    pack_bits(&plane_row, &mut plane.data);
                    // A row compresses to at most 60,470 bytes (30,000 16-bit samples).
                    plane.counts.push((plane.data.len() - start) as u16);
                }
            }
            y += rows;
            self.rows_done += u64::from(rows);
            (self.progress)(Progress {
                done: self.rows_done,
                total: self.rows_total.max(self.rows_done),
            });
        }
        Ok(Planes { channels })
    }
}

/// Composite straight big-endian RGBA samples over white, alpha kept: `c·a + (1 − a)`.
fn matte_white(row: &mut [u8], bytes: usize) {
    let max = if bytes == 1 { 255u64 } else { 65_535 };
    let read = |s: &[u8]| s.iter().fold(0u64, |v, &b| v << 8 | u64::from(b));
    for pixel in row.chunks_exact_mut(4 * bytes) {
        let alpha = read(&pixel[3 * bytes..]);
        for sample in pixel[..3 * bytes].chunks_exact_mut(bytes) {
            let matted = (read(sample) * alpha + max * (max - alpha) + max / 2) / max;
            sample.copy_from_slice(&matted.to_be_bytes()[8 - bytes..]);
        }
    }
}

/// The four channels of a layer without pixels.
fn empty_channels() -> Vec<(i16, Option<Plane>)> {
    [-1, 0, 1, 2].into_iter().map(|id| (id, None)).collect()
}

/// PackBits: runs of 2 to 128 equal bytes as (1 − n, byte), the rest as literals (n − 1, bytes).
fn pack_bits(src: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < src.len() {
        let mut run = 1;
        while i + run < src.len() && run < 128 && src[i + run] == src[i] {
            run += 1;
        }
        if run >= 2 {
            out.push((1 - run as i32) as i8 as u8);
            out.push(src[i]);
            i += run;
            continue;
        }
        let start = i;
        i += 1;
        while i < src.len() && i - start < 128 && !(i + 1 < src.len() && src[i] == src[i + 1]) {
            i += 1;
        }
        out.push((i - start - 1) as u8);
        out.extend(&src[start..i]);
    }
}

fn blend_key(mode: BlendMode) -> [u8; 4] {
    *match mode {
        BlendMode::Normal => b"norm",
        BlendMode::Dissolve => b"diss",
        BlendMode::Darken => b"dark",
        BlendMode::Multiply => b"mul ",
        BlendMode::ColorBurn => b"idiv",
        BlendMode::LinearBurn => b"lbrn",
        BlendMode::DarkerColor => b"dkCl",
        BlendMode::Lighten => b"lite",
        BlendMode::Screen => b"scrn",
        BlendMode::ColorDodge => b"div ",
        BlendMode::LinearDodge => b"lddg",
        BlendMode::LighterColor => b"lgCl",
        BlendMode::Overlay => b"over",
        BlendMode::SoftLight => b"sLit",
        BlendMode::HardLight => b"hLit",
        BlendMode::VividLight => b"vLit",
        BlendMode::LinearLight => b"lLit",
        BlendMode::PinLight => b"pLit",
        BlendMode::HardMix => b"hMix",
        BlendMode::Difference => b"diff",
        BlendMode::Exclusion => b"smud",
        BlendMode::Subtract => b"fsub",
        BlendMode::Divide => b"fdiv",
        BlendMode::Hue => b"hue ",
        BlendMode::Saturation => b"sat ",
        BlendMode::Color => b"colr",
        BlendMode::Luminosity => b"lum ",
    }
}

/// An action descriptor (Photoshop's structured data): an empty name, class `null`, and items
/// of 4-byte types.
fn descriptor(items: &[(&[u8], &[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut out = 16u32.to_be_bytes().to_vec();
    out.extend(1u32.to_be_bytes());
    out.extend([0, 0]);
    out.extend(0u32.to_be_bytes());
    out.extend(b"null");
    out.extend((items.len() as u32).to_be_bytes());
    for (key, ty, value) in items {
        let length = if key.len() == 4 { 0 } else { key.len() as u32 };
        out.extend(length.to_be_bytes());
        out.extend(*key);
        out.extend(*ty);
        out.extend(value);
    }
    out
}

fn long(v: i32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/// The tagged blocks of an adjustment layer (what `psd::layers` reads back).
fn adjustment_blocks(adjustment: &Adjustment) -> Vec<([u8; 4], Vec<u8>)> {
    let be16 = |values: &[i16]| {
        values
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect::<Vec<u8>>()
    };
    let round = |v: f32| v.round() as i16;
    match *adjustment {
        Adjustment::Levels {
            input_black,
            input_white,
            gamma,
            output_black,
            output_white,
        } => {
            let code = |v: f32| (v * 255.0).round() as i16;
            let mut b = be16(&[2]);
            b.extend(be16(&[
                code(input_black),
                code(input_white),
                code(output_black),
                code(output_white),
                (gamma * 100.0).round() as i16,
            ]));
            // The channel records: unchanged.
            for _ in 1..29 {
                b.extend(be16(&[0, 255, 0, 255, 100]));
            }
            vec![(*b"levl", b)]
        }
        Adjustment::Exposure {
            exposure,
            offset,
            gamma,
        } => {
            let mut b = be16(&[1]);
            for v in [exposure, offset, gamma] {
                b.extend(v.to_be_bytes());
            }
            vec![(*b"expA", b)]
        }
        Adjustment::HueSaturation {
            hue,
            saturation,
            lightness,
        } => {
            let mut b = be16(&[2]);
            b.extend([0, 0]);
            b.extend(be16(&[
                0,
                25,
                0,
                round(hue),
                round(saturation),
                round(lightness),
            ]));
            // Photoshop's six default color ranges, unchanged.
            for start in [315i16, 15, 75, 135, 195, 255] {
                let wrap = |v: i16| if v >= 360 { v - 360 } else { v };
                b.extend(be16(&[
                    start,
                    wrap(start + 30),
                    wrap(start + 60),
                    wrap(start + 90),
                    0,
                    0,
                    0,
                ]));
            }
            vec![(*b"hue2", b)]
        }
        Adjustment::BrightnessContrast {
            brightness,
            contrast,
        } => {
            let mut b = be16(&[round(brightness), round(contrast), 127]);
            b.extend([0, 0]);
            let d = descriptor(&[
                (b"Vrsn", b"long", long(1)),
                (b"Brgh", b"long", long(brightness.round() as i32)),
                (b"Cntr", b"long", long(contrast.round() as i32)),
                (b"means", b"long", long(127)),
                (b"Lab ", b"bool", vec![0]),
                (b"useLegacy", b"bool", vec![0]),
                (b"Auto", b"bool", vec![0]),
            ]);
            vec![(*b"brit", b), (*b"CgEd", d)]
        }
        Adjustment::Vibrance {
            vibrance,
            saturation,
        } => vec![(
            *b"vibA",
            descriptor(&[
                (b"vibrance", b"long", long(vibrance.round() as i32)),
                (b"Strt", b"long", long(saturation.round() as i32)),
            ]),
        )],
        Adjustment::Invert => vec![(*b"nvrt", Vec::new())],
        Adjustment::Posterize { levels } => vec![(*b"post", be16(&[round(levels), 0]))],
        Adjustment::Threshold { level } => {
            vec![(*b"thrs", be16(&[(level * 255.0).round() as i16, 0]))]
        }
    }
}

fn header(out: &mut Vec<u8>, size: Size, depth: PsdDepth) {
    out.extend(b"8BPS");
    out.extend(1u16.to_be_bytes());
    out.extend([0; 6]);
    // Red, green, blue and the composite's transparency.
    out.extend(4u16.to_be_bytes());
    out.extend(size.height.to_be_bytes());
    out.extend(size.width.to_be_bytes());
    out.extend(
        match depth {
            PsdDepth::U8 => 8u16,
            PsdDepth::U16 => 16,
        }
        .to_be_bytes(),
    );
    // RGB.
    out.extend(3u16.to_be_bytes());
}

/// Image resources: the ICC profile (resource 1039).
fn image_resources(out: &mut Vec<u8>, profile: &[u8]) {
    let mut resources = Vec::new();
    resources.extend(b"8BIM");
    resources.extend(1039u16.to_be_bytes());
    // An empty Pascal name, padded to an even length.
    resources.extend([0, 0]);
    resources.extend((profile.len() as u32).to_be_bytes());
    resources.extend(profile);
    if profile.len() % 2 == 1 {
        resources.push(0);
    }
    out.extend((resources.len() as u32).to_be_bytes());
    out.extend(resources);
}

/// A tagged block, padded to an even length.
fn block(out: &mut Vec<u8>, key: &[u8; 4], data: &[u8]) {
    out.extend(b"8BIM");
    out.extend(key);
    let padded = data.len().next_multiple_of(2);
    out.extend((padded as u32).to_be_bytes());
    out.extend(data);
    out.resize(out.len() + padded - data.len(), 0);
}

/// The layer info: the count (negative: the composite's first alpha channel is its
/// transparency), the records, then every channel's data.
fn layer_info(records: &[Record]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend((-(records.len() as i16)).to_be_bytes());
    for r in records {
        let b = r.bounds;
        for v in [b.y, b.x, b.y + b.height, b.x + b.width] {
            out.extend((v as i32).to_be_bytes());
        }
        out.extend((r.channels.len() as u16).to_be_bytes());
        for (id, plane) in &r.channels {
            out.extend(id.to_be_bytes());
            let len = plane.as_ref().map_or(2, Plane::len);
            out.extend((len as u32).to_be_bytes());
        }
        out.extend(b"8BIM");
        out.extend(r.blend_key);
        out.push(r.opacity);
        out.push(u8::from(r.clipped));
        // Flags: bit 1 hidden; bit 3 set, bit 4 meaningful (pixel data irrelevant: no).
        out.push(if r.hidden { 2 } else { 0 } | 8);
        out.push(0);
        let mut extra = Vec::new();
        match r.mask {
            Some((area, disabled)) => {
                extra.extend(20u32.to_be_bytes());
                for v in [area.y, area.x, area.y + area.height, area.x + area.width] {
                    extra.extend((v as i32).to_be_bytes());
                }
                // Default color 0 (hidden outside), flags (bit 1: disabled), padding.
                extra.push(0);
                extra.push(if disabled { 2 } else { 0 });
                extra.extend([0, 0]);
            }
            None => extra.extend(0u32.to_be_bytes()),
        }
        // Blending ranges: none.
        extra.extend(0u32.to_be_bytes());
        let latin: Vec<u8> = r
            .name
            .chars()
            .map(|c| {
                if c.is_ascii() && !c.is_control() {
                    c as u8
                } else {
                    b'?'
                }
            })
            .take(255)
            .collect();
        extra.push(latin.len() as u8);
        extra.extend(&latin);
        extra.resize(
            extra.len() + (1 + latin.len()).next_multiple_of(4) - 1 - latin.len(),
            0,
        );
        let units: Vec<u16> = r.name.encode_utf16().collect();
        let mut luni = (units.len() as u32).to_be_bytes().to_vec();
        for u in units {
            luni.extend(u.to_be_bytes());
        }
        block(&mut extra, b"luni", &luni);
        for (key, data) in &r.blocks {
            block(&mut extra, key, data);
        }
        out.extend((extra.len() as u32).to_be_bytes());
        out.extend(extra);
    }
    for r in records {
        for (_, plane) in &r.channels {
            match plane {
                Some(plane) => plane.write(&mut out),
                None => out.extend(0u16.to_be_bytes()),
            }
        }
    }
    // A multiple of 4: Photoshop aligns the global `Lr16` block so, and readers follow it.
    out.resize(out.len().next_multiple_of(4), 0);
    out
}

/// The layer and mask information section. 16-bit documents keep their layers in an `Lr16`
/// block, as Photoshop writes them.
fn layer_and_mask_info(out: &mut Vec<u8>, records: &[Record], depth: PsdDepth) {
    let info = layer_info(records);
    let mut section = Vec::new();
    match depth {
        PsdDepth::U8 => {
            section.extend((info.len() as u32).to_be_bytes());
            section.extend(&info);
            // Global layer mask info: none.
            section.extend(0u32.to_be_bytes());
        }
        PsdDepth::U16 => {
            section.extend(0u32.to_be_bytes());
            section.extend(0u32.to_be_bytes());
            block(&mut section, b"Lr16", &info);
        }
    }
    out.extend((section.len() as u32).to_be_bytes());
    out.extend(section);
}

/// The merged composite: RLE, every row length of every channel, then the rows.
fn merged_image(out: &mut Vec<u8>, composite: &Planes) {
    out.extend(1u16.to_be_bytes());
    for plane in &composite.channels {
        for count in &plane.counts {
            out.extend(count.to_be_bytes());
        }
    }
    for plane in &composite.channels {
        out.extend(&plane.data);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_bits_round_trips() {
        let unpack = |mut data: &[u8], len: usize| {
            let mut out = Vec::new();
            while out.len() < len {
                let n = data[0] as i8;
                if n >= 0 {
                    out.extend(&data[1..2 + n as usize]);
                    data = &data[2 + n as usize..];
                } else {
                    out.extend(std::iter::repeat_n(data[1], (1 - n as i32) as usize));
                    data = &data[2..];
                }
            }
            out
        };
        for src in [
            vec![],
            vec![7],
            vec![1, 1],
            vec![1, 2, 3, 3, 3, 4],
            (0..300u32).map(|i| (i / 7) as u8).collect::<Vec<u8>>(),
            (0..300u32)
                .map(|i| (i * 31 % 251) as u8)
                .collect::<Vec<u8>>(),
            vec![9; 1000],
        ] {
            let mut packed = Vec::new();
            pack_bits(&src, &mut packed);
            assert_eq!(unpack(&packed, src.len()), src);
        }
    }

    #[test]
    fn blend_keys_are_distinct() {
        let keys: std::collections::HashSet<[u8; 4]> =
            BlendMode::ALL.iter().map(|&m| blend_key(m)).collect();
        assert_eq!(keys.len(), BlendMode::ALL.len());
    }
}
