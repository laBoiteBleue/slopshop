use std::io::Write;
use std::path::PathBuf;

use flate2::Compression;
use flate2::write::ZlibEncoder;
use slopshop_core::color::{ColorSpace, TransferFunction};
use slopshop_core::tile::TileCoord;

use super::*;
use crate::{ImportError, ImportWarning, icc, open_image};

/// A Photoshop document to write, planes as stored (big-endian rows).
struct Doc {
    big: bool,
    mode: u16,
    depth: u16,
    width: u32,
    height: u32,
    planes: Vec<Vec<u8>>,
    compression: u16,
    layer_count: i16,
    icc: Option<Vec<u8>>,
    merged: bool,
    palette: Option<Vec<u8>>,
    transparent_index: Option<u16>,
    layers: Vec<TestLayer>,
    /// Layers in an `Lr16`/`Lr32` tagged block, as 16/32-bit documents store them.
    layers_in_block: bool,
    layer_compression: u16,
}

/// A layer to write. Channels are stored planes (big-endian rows) over the layer's bounds, or
/// over the mask's bounds for a mask channel.
struct TestLayer {
    name: &'static str,
    /// Top, left, bottom, right.
    bounds: [i32; 4],
    channels: Vec<(i16, Vec<u8>)>,
    blend: [u8; 4],
    opacity: u8,
    hidden: bool,
    clipping: bool,
    /// Bounds, default color, flags.
    mask: Option<([i32; 4], u8, u8)>,
    blocks: Vec<([u8; 4], Vec<u8>)>,
}

impl TestLayer {
    fn new(name: &'static str, bounds: [i32; 4], channels: Vec<(i16, Vec<u8>)>) -> Self {
        Self {
            name,
            bounds,
            channels,
            blend: *b"norm",
            opacity: 255,
            hidden: false,
            clipping: false,
            mask: None,
            blocks: Vec::new(),
        }
    }

    /// A group's record (`kind` 1) or divider (`kind` 3), without pixels.
    fn section(name: &'static str, kind: u32) -> Self {
        let mut layer = Self::new(name, [0; 4], Vec::new());
        layer.blocks.push((*b"lsct", kind.to_be_bytes().to_vec()));
        layer
    }
}

impl Doc {
    fn new(mode: u16, depth: u16, width: u32, height: u32, planes: Vec<Vec<u8>>) -> Self {
        Self {
            big: false,
            mode,
            depth,
            width,
            height,
            planes,
            compression: 0,
            layer_count: 0,
            icc: None,
            merged: true,
            palette: None,
            transparent_index: None,
            layers: Vec::new(),
            layers_in_block: false,
            layer_compression: 0,
        }
    }

    fn row_bytes(&self) -> usize {
        match self.depth {
            1 => (self.width as usize).div_ceil(8),
            d => self.width as usize * usize::from(d / 8),
        }
    }

    fn write(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(b"8BPS");
        out.extend(if self.big { 2u16 } else { 1u16 }.to_be_bytes());
        out.extend([0u8; 6]);
        out.extend((self.planes.len() as u16).to_be_bytes());
        out.extend(self.height.to_be_bytes());
        out.extend(self.width.to_be_bytes());
        out.extend(self.depth.to_be_bytes());
        out.extend(self.mode.to_be_bytes());

        let palette = self.palette.clone().unwrap_or_default();
        out.extend((palette.len() as u32).to_be_bytes());
        out.extend(&palette);

        let mut resources = Vec::new();
        let mut block = |id: u16, data: &[u8]| {
            resources.extend(b"8BIM");
            resources.extend(id.to_be_bytes());
            resources.extend([0u8, 0]); // empty name, padded
            resources.extend((data.len() as u32).to_be_bytes());
            resources.extend(data);
            if data.len() % 2 == 1 {
                resources.push(0);
            }
        };
        if let Some(icc) = &self.icc {
            block(1039, icc);
        }
        if let Some(index) = self.transparent_index {
            block(1047, &index.to_be_bytes());
        }
        block(1057, &[0, 0, 0, 1, u8::from(self.merged), 0, 0, 0, 0]);
        out.extend((resources.len() as u32).to_be_bytes());
        out.extend(&resources);

        // Layer and mask information: a layer info block holding only the count.
        let length = |out: &mut Vec<u8>, n: u64| {
            if self.big {
                out.extend(n.to_be_bytes());
            } else {
                out.extend((n as u32).to_be_bytes());
            }
        };
        if !self.layers.is_empty() {
            let info = self.layer_info();
            let mut section = Vec::new();
            if self.layers_in_block {
                length(&mut section, 0);
                section.extend(0u32.to_be_bytes()); // global layer mask info
                section.extend(b"8BIM");
                section.extend(if self.depth == 32 { b"Lr32" } else { b"Lr16" });
                length(&mut section, info.len() as u64);
                section.extend(&info);
            } else {
                length(&mut section, info.len() as u64);
                section.extend(&info);
                section.extend(0u32.to_be_bytes()); // global layer mask info
            }
            length(&mut out, section.len() as u64);
            out.extend(section);
        } else if self.layer_count == 0 {
            length(&mut out, 0);
        } else {
            let field = if self.big { 8 } else { 4 };
            length(&mut out, field + 2);
            length(&mut out, 2);
            out.extend(self.layer_count.to_be_bytes());
        }

        out.extend(self.compression.to_be_bytes());
        let row_bytes = self.row_bytes();
        match self.compression {
            0 => self.planes.iter().for_each(|p| out.extend(p)),
            1 => {
                let packed: Vec<Vec<u8>> = self
                    .planes
                    .iter()
                    .flat_map(|p| p.chunks(row_bytes).map(pack_bits))
                    .collect();
                for row in &packed {
                    if self.big {
                        out.extend((row.len() as u32).to_be_bytes());
                    } else {
                        out.extend((row.len() as u16).to_be_bytes());
                    }
                }
                packed.iter().for_each(|row| out.extend(row));
            }
            3 => {
                let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
                for plane in &self.planes {
                    for row in plane.chunks(row_bytes) {
                        z.write_all(&predict(row, self.depth, self.width as usize))
                            .unwrap();
                    }
                }
                out.extend(z.finish().unwrap());
            }
            _ => unreachable!(),
        }
        out
    }
}

impl Doc {
    /// The layer info body: count, records, channels (padded to an even length).
    fn layer_info(&self) -> Vec<u8> {
        let sample_bytes = usize::from(self.depth / 8);
        let mut records = Vec::new();
        let mut channel_data = Vec::new();
        for layer in &self.layers {
            let [top, left, bottom, right] = layer.bounds;
            let mut stored = Vec::new();
            for (id, plane) in &layer.channels {
                let width = match (*id, layer.mask) {
                    (-2 | -3, Some(([_, l, _, r], _, _))) => (r - l) as usize,
                    _ => (right - left) as usize,
                };
                let row_bytes = width * sample_bytes;
                let mut data = self.layer_compression.to_be_bytes().to_vec();
                if !plane.is_empty() {
                    match self.layer_compression {
                        1 => {
                            let rows: Vec<Vec<u8>> =
                                plane.chunks(row_bytes).map(pack_bits).collect();
                            for row in &rows {
                                if self.big {
                                    data.extend((row.len() as u32).to_be_bytes());
                                } else {
                                    data.extend((row.len() as u16).to_be_bytes());
                                }
                            }
                            rows.iter().for_each(|r| data.extend(r));
                        }
                        3 => {
                            let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
                            for row in plane.chunks(row_bytes) {
                                z.write_all(&predict(row, self.depth, width)).unwrap();
                            }
                            data.extend(z.finish().unwrap());
                        }
                        _ => data.extend(plane),
                    }
                }
                stored.push((*id, data));
            }

            for v in [top, left, bottom, right] {
                records.extend(v.to_be_bytes());
            }
            records.extend((stored.len() as u16).to_be_bytes());
            for (id, data) in &stored {
                records.extend(id.to_be_bytes());
                if self.big {
                    records.extend((data.len() as u64).to_be_bytes());
                } else {
                    records.extend((data.len() as u32).to_be_bytes());
                }
            }
            records.extend(b"8BIM");
            records.extend(layer.blend);
            records.extend([
                layer.opacity,
                u8::from(layer.clipping),
                if layer.hidden { 2 } else { 0 },
                0,
            ]);
            let mut extra = Vec::new();
            match layer.mask {
                Some((bounds, default, flags)) => {
                    extra.extend(20u32.to_be_bytes());
                    bounds.iter().for_each(|v| extra.extend(v.to_be_bytes()));
                    extra.extend([default, flags, 0, 0]);
                }
                None => extra.extend(0u32.to_be_bytes()),
            }
            extra.extend(0u32.to_be_bytes()); // blending ranges: default
            extra.push(layer.name.len() as u8);
            extra.extend(layer.name.as_bytes());
            while extra.len() % 4 != 0 {
                extra.push(0);
            }
            for (key, data) in &layer.blocks {
                extra.extend(b"8BIM");
                extra.extend(key);
                extra.extend((data.len().next_multiple_of(2) as u32).to_be_bytes());
                extra.extend(data);
                if data.len() % 2 == 1 {
                    extra.push(0);
                }
            }
            records.extend((extra.len() as u32).to_be_bytes());
            records.extend(extra);
            channel_data.extend(stored.into_iter().flat_map(|(_, d)| d));
        }
        let mut info = (self.layers.len() as i16).to_be_bytes().to_vec();
        info.extend(records);
        info.extend(channel_data);
        if info.len() % 2 == 1 {
            info.push(0);
        }
        info
    }
}

/// PackBits with runs for repeated bytes and literals otherwise.
fn pack_bits(row: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < row.len() {
        let mut run = 1;
        while i + run < row.len() && row[i + run] == row[i] && run < 128 {
            run += 1;
        }
        if run >= 2 {
            out.push((1 - run as i32) as i8 as u8);
            out.push(row[i]);
            i += run;
        } else {
            let start = i;
            while i < row.len() && i - start < 128 && (i + 1 >= row.len() || row[i + 1] != row[i]) {
                i += 1;
            }
            if i == start {
                i += 1;
            }
            out.push((i - start - 1) as u8);
            out.extend(&row[start..i]);
        }
    }
    out
}

fn predict(row: &[u8], depth: u16, width: usize) -> Vec<u8> {
    match depth {
        16 => {
            let mut out = Vec::with_capacity(row.len());
            let mut previous = 0u16;
            for pair in row.as_chunks::<2>().0 {
                let v = u16::from_be_bytes([pair[0], pair[1]]);
                out.extend(v.wrapping_sub(previous).to_be_bytes());
                previous = v;
            }
            out
        }
        32 => {
            let mut planar = vec![0u8; row.len()];
            for x in 0..width {
                for byte in 0..4 {
                    planar[byte * width + x] = row[x * 4 + byte];
                }
            }
            let mut out = planar.clone();
            for i in 1..out.len() {
                out[i] = planar[i].wrapping_sub(planar[i - 1]);
            }
            out
        }
        _ => {
            let mut out = row.to_vec();
            for i in 1..out.len() {
                out[i] = row[i].wrapping_sub(row[i - 1]);
            }
            out
        }
    }
}

fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let path = std::env::temp_dir().join(format!("slopshop-psd-{}-{name}", std::process::id()));
    std::fs::write(&path, bytes).unwrap();
    path
}

fn open(name: &str, doc: &Doc) -> Result<crate::Imported, ImportError> {
    let path = temp_file(name, &doc.write());
    let result = open_image(&path);
    std::fs::remove_file(&path).ok();
    result
}

/// Level-0 pixel (x, y) of an imported image, as stored bytes.
fn pixel(image: &slopshop_core::RasterImage, x: u32, y: u32) -> Vec<u8> {
    let bpp = image.stored_format().bytes_per_pixel() as usize;
    let tile = image.levels()[0]
        .tile(TileCoord {
            col: x / 256,
            row: y / 256,
        })
        .unwrap();
    let at = ((y % 256) * 256 + x % 256) as usize * bpp;
    tile[at..at + bpp].to_vec()
}

fn rgb8_planes(width: u32, height: u32) -> (Vec<Vec<u8>>, impl Fn(u32, u32) -> [u8; 3]) {
    let color = |x: u32, y: u32| {
        [
            (x * 13 % 256) as u8,
            (y * 7 % 256) as u8,
            ((x ^ y) % 256) as u8,
        ]
    };
    let planes = (0..3)
        .map(|c| {
            (0..height)
                .flat_map(|y| (0..width).map(move |x| color(x, y)[c]))
                .collect()
        })
        .collect();
    (planes, color)
}

#[test]
fn rgb_composites_open_in_every_compression_and_version() {
    let (width, height) = (300, 70);
    let (planes, color) = rgb8_planes(width, height);
    for big in [false, true] {
        for compression in [0, 1, 3] {
            let mut doc = Doc::new(3, 8, width, height, planes.clone());
            doc.big = big;
            doc.compression = compression;
            let imported = open(&format!("rgb-{big}-{compression}.psd"), &doc).unwrap();
            let image = &imported.image;
            assert_eq!(image.size(), Size::new(width, height));
            assert_eq!(image.format().layout, ChannelLayout::Rgb);
            assert_eq!(image.format().color_space, ColorSpace::SRGB);
            for (x, y) in [(0, 0), (299, 69), (257, 3), (41, 66)] {
                assert_eq!(
                    pixel(image, x, y)[..3],
                    color(x, y),
                    "{big} {compression} ({x}, {y})"
                );
            }
            assert!(imported.warnings.is_empty());
        }
    }
}

#[test]
fn sixteen_bit_gray_and_thirty_two_bit_rgb_keep_their_samples() {
    let (width, height) = (40, 9);
    let gray: Vec<u16> = (0..width * height)
        .map(|i| (i * 1789 % 65536) as u16)
        .collect();
    let plane: Vec<u8> = gray.iter().flat_map(|v| v.to_be_bytes()).collect();
    for compression in [0, 1, 3] {
        let mut doc = Doc::new(1, 16, width, height, vec![plane.clone()]);
        doc.compression = compression;
        let image = open(&format!("gray16-{compression}.psd"), &doc)
            .unwrap()
            .image;
        assert_eq!(image.format().sample, SampleType::U16);
        assert_eq!(image.format().layout, ChannelLayout::Gray);
        let px = pixel(&image, 17, 5);
        assert_eq!(u16::from_ne_bytes([px[0], px[1]]), gray[5 * 40 + 17]);
    }

    let value = |x: u32, c: u32| x as f32 / 7.0 + c as f32 * 2.5 - 1.0;
    let planes: Vec<Vec<u8>> = (0..3)
        .map(|c| {
            (0..height)
                .flat_map(|_| (0..width).flat_map(move |x| value(x, c).to_be_bytes()))
                .collect()
        })
        .collect();
    let mut doc = Doc::new(3, 32, width, height, planes);
    doc.compression = 3;
    // A Display P3 profile: 32-bit documents keep its primaries with a linear transfer.
    doc.icc = Some(icc::write_matrix_trc(&ColorSpace::DISPLAY_P3).unwrap());
    let image = open("rgb32.psd", &doc).unwrap().image;
    let format = image.format();
    assert_eq!(format.sample, SampleType::F32);
    assert_eq!(format.color_space.transfer, TransferFunction::Linear);
    assert_eq!(
        format.color_space.primaries,
        ColorSpace::DISPLAY_P3.primaries
    );
    let px = pixel(&image, 33, 2);
    for c in 0..3u32 {
        let at = c as usize * 4;
        let v = f32::from_ne_bytes([px[at], px[at + 1], px[at + 2], px[at + 3]]);
        assert_eq!(v, value(33, c));
    }
}

#[test]
fn transparency_is_read_and_the_white_matte_removed() {
    let (width, height) = (16, 4);
    let straight = [200u8, 40, 90];
    let alpha = |x: u32| (x * 16) as u8;
    // Photoshop stores colors matted against white: c' = c·a + (1 − a).
    let matted = |c: u8, a: u8| {
        let (c, a) = (f64::from(c) / 255.0, f64::from(a) / 255.0);
        ((c * a + (1.0 - a)) * 255.0).round() as u8
    };
    let mut planes: Vec<Vec<u8>> = (0..3)
        .map(|c| {
            (0..height)
                .flat_map(|_| (0..width).map(move |x| matted(straight[c], alpha(x))))
                .collect()
        })
        .collect();
    planes.push((0..height).flat_map(|_| (0..width).map(alpha)).collect());
    let mut doc = Doc::new(3, 8, width, height, planes);
    doc.layer_count = -2;
    let imported = open("transparent.psd", &doc).unwrap();
    assert_eq!(imported.image.format().layout, ChannelLayout::Rgba);
    assert_eq!(imported.warnings, [ImportWarning::LayersFlattened]);
    for x in [6, 10, 15] {
        let px = pixel(&imported.image, x, 1);
        assert_eq!(px[3], alpha(x));
        for c in 0..3 {
            assert!(
                px[c].abs_diff(straight[c]) <= 6,
                "x {x} channel {c}: {} vs {}",
                px[c],
                straight[c]
            );
        }
    }
    // Fully transparent pixels have no color left.
    assert_eq!(pixel(&imported.image, 0, 0), [0, 0, 0, 0]);
}

#[test]
fn bitmap_and_indexed_documents_open_as_gray_and_rgb() {
    // 1-bit: 1 is black.
    let doc = Doc::new(
        0,
        1,
        10,
        2,
        vec![vec![0b1010_0000, 0b0100_0000, 0xff, 0xff]],
    );
    let image = open("bitmap.psd", &doc).unwrap().image;
    assert_eq!(image.format().layout, ChannelLayout::Gray);
    let row: Vec<u8> = (0..10).map(|x| pixel(&image, x, 0)[0]).collect();
    assert_eq!(row, [0, 255, 0, 255, 255, 255, 255, 255, 255, 0]);

    // Indexed: 256 reds, 256 greens, 256 blues; index 2 transparent.
    let mut palette = vec![0u8; 768];
    for i in 0..256 {
        palette[i] = i as u8;
        palette[256 + i] = 255 - i as u8;
        palette[512 + i] = 7;
    }
    let mut doc = Doc::new(2, 8, 4, 1, vec![vec![0, 1, 2, 200]]);
    doc.palette = Some(palette);
    doc.transparent_index = Some(2);
    let image = open("indexed.psd", &doc).unwrap().image;
    assert_eq!(image.format().layout, ChannelLayout::Rgba);
    assert_eq!(pixel(&image, 1, 0), [1, 254, 7, 255]);
    assert_eq!(pixel(&image, 2, 0)[3], 0);
    assert_eq!(pixel(&image, 3, 0), [200, 55, 7, 255]);
}

#[test]
fn unsupported_and_damaged_documents_are_errors() {
    let cmyk = Doc::new(4, 8, 2, 2, vec![vec![0; 4]; 4]);
    assert!(matches!(
        open("cmyk.psd", &cmyk),
        Err(ImportError::NotYetSupported(_))
    ));
    let mut no_composite = Doc::new(3, 8, 2, 2, vec![vec![0; 4]; 3]);
    no_composite.merged = false;
    assert!(matches!(
        open("empty.psd", &no_composite),
        Err(ImportError::PsdWithoutComposite)
    ));

    // Every truncation of valid files is an error, never a panic.
    let (planes, _) = rgb8_planes(20, 5);
    for compression in [0, 1, 3] {
        let mut doc = Doc::new(3, 8, 20, 5, planes.clone());
        doc.compression = compression;
        doc.layer_count = 3;
        let bytes = doc.write();
        for cut in (0..bytes.len()).step_by(3) {
            let path = temp_file(&format!("cut-{compression}.psd"), &bytes[..cut]);
            assert!(
                open_image(&path).is_err(),
                "compression {compression}, cut at {cut}"
            );
            std::fs::remove_file(&path).ok();
        }
    }
    // Absurd sizes are refused before any allocation.
    let mut huge = Doc::new(3, 8, 1, 1, vec![vec![0]; 3]);
    huge.width = 30_001;
    assert!(matches!(
        open("huge.psd", &huge),
        Err(ImportError::TooLarge { .. })
    ));
}

#[test]
fn packbits_round_trips_and_rejects_overflows() {
    let row: Vec<u8> = (0..300)
        .map(|i| if i % 50 < 20 { 9 } else { (i * 7) as u8 })
        .collect();
    let mut out = vec![0u8; row.len()];
    unpack_bits(&pack_bits(&row), &mut out).unwrap();
    assert_eq!(out, row);
    let mut small = [0u8; 2];
    assert!(
        unpack_bits(&[0xfd, 1], &mut small).is_err(),
        "a run longer than the row"
    );
    assert!(
        unpack_bits(&[1, 5], &mut small).is_err(),
        "a truncated literal"
    );
}

fn open_file_of(name: &str, doc: &Doc) -> Result<crate::Opened, ImportError> {
    let path = temp_file(name, &doc.write());
    let result = crate::open_file(&path);
    std::fs::remove_file(&path).ok();
    result
}

fn open_layers(name: &str, doc: &Doc) -> crate::ImportedLayers {
    match open_file_of(name, doc).unwrap() {
        crate::Opened::Layers(layers) => layers,
        crate::Opened::Image(_) => panic!("{name}: opened as an image"),
    }
}

fn raster(layer: &slopshop_core::Layer) -> &slopshop_core::RasterImage {
    match &layer.content {
        slopshop_core::LayerContent::Raster { image, .. } => image
            .ready_image()
            .expect("imported pixels are there from the start"),
        other => panic!("not a raster: {other:?}"),
    }
}

/// An 8-bit plane of `width` × `height`.
fn plane8(width: u32, height: u32, value: impl Fn(u32, u32) -> u8) -> Vec<u8> {
    (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .map(|(x, y)| value(x, y))
        .collect()
}

fn luni(name: &str) -> Vec<u8> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let mut data = (units.len() as u32).to_be_bytes().to_vec();
    units.iter().for_each(|u| data.extend(u.to_be_bytes()));
    data
}

#[test]
fn layers_open_with_their_bounds_modes_masks_and_names() {
    let (width, height) = (300, 200);
    for big in [false, true] {
        for compression in [0, 1, 3] {
            let mut doc = Doc::new(3, 8, width, height, rgb8_planes(width, height).0);
            doc.big = big;
            doc.layer_compression = compression;
            let background = TestLayer::new(
                "Background",
                [0, 0, 200, 300],
                vec![
                    (0, plane8(300, 200, |x, _| x as u8)),
                    (1, plane8(300, 200, |_, y| y as u8)),
                    (2, plane8(300, 200, |_, _| 9)),
                ],
            );
            // 100 × 100 at (40, 50), with a 40 × 40 mask at (50, 60).
            let mut square = TestLayer::new(
                "square",
                [50, 40, 150, 140],
                vec![
                    (-1, plane8(100, 100, |_, _| 200)),
                    (0, plane8(100, 100, |_, _| 255)),
                    (1, plane8(100, 100, |x, _| x as u8)),
                    (2, plane8(100, 100, |_, _| 0)),
                    (-2, plane8(40, 40, |_, _| 30)),
                ],
            );
            square.blend = *b"mul ";
            square.opacity = 128;
            square.mask = Some(([60, 50, 100, 90], 255, 0));
            square.blocks.push((*b"luni", luni("Carré rouge")));
            // 60 × 40 at (-30, -20): only its bottom-right quarter is on the canvas.
            let off = TestLayer::new(
                "off",
                [-20, -30, 20, 30],
                (-1..3)
                    .map(|c| (c, plane8(60, 40, move |_, _| if c < 0 { 255 } else { 77 })))
                    .collect(),
            );
            let mut hidden = TestLayer::new(
                "hidden",
                [0, 0, 10, 10],
                (-1..3).map(|c| (c, plane8(10, 10, |_, _| 1))).collect(),
            );
            hidden.hidden = true;
            doc.layers = vec![background, square, off, hidden];

            let opened = open_layers(&format!("layers-{big}-{compression}.psd"), &doc);
            let document = &opened.document;
            assert_eq!(document.size(), Size::new(width, height));
            assert_eq!(
                document.blend_space(),
                slopshop_core::BlendSpace::Perceptual
            );
            let layers = document.layers();
            let names: Vec<&str> = layers.iter().map(|l| l.name.as_str()).collect();
            assert_eq!(names, ["Background", "Carré rouge", "off", "hidden"]);
            assert!(opened.warnings.is_empty());

            let background = raster(&layers[0]);
            assert_eq!(background.format().layout, ChannelLayout::Rgb);
            assert_eq!(pixel(background, 123, 45)[..3], [123, 45, 9]);

            let square = &layers[1];
            assert_eq!(square.blend_mode, slopshop_core::BlendMode::Multiply);
            assert!((square.opacity - 128.0 / 255.0).abs() < 1e-6);
            let image = raster(square);
            assert_eq!(image.format().layout, ChannelLayout::Rgba);
            assert_eq!(pixel(image, 50, 70), [255, 10, 0, 200]);
            assert_eq!(pixel(image, 10, 10), [0, 0, 0, 0], "outside its bounds");
            let mask = square.mask.as_ref().unwrap();
            assert!(mask.enabled && !mask.replaces_alpha);
            assert_eq!(pixel(&mask.image, 55, 65), [30]);
            assert_eq!(
                pixel(&mask.image, 10, 10),
                [255],
                "the default color outside"
            );

            let off = raster(&layers[2]);
            assert_eq!(pixel(off, 0, 0), [77, 77, 77, 255]);
            assert_eq!(pixel(off, 29, 19), [77, 77, 77, 255]);
            assert_eq!(pixel(off, 30, 0)[3], 0);
            assert!(!layers[3].visible && layers[2].visible);
            assert_eq!(
                opened.layer_warnings,
                [
                    vec![],
                    vec![],
                    vec![ImportWarning::PixelsOutsideCanvas],
                    vec![]
                ]
            );
        }
    }
}

#[test]
fn deep_layers_are_read_from_their_tagged_block() {
    // 16-bit gray: a 10 × 4 layer at (3, 2).
    let value = |x: u32, y: u32| (x * 1000 + y) as u16;
    let plane16 = |f: &dyn Fn(u32, u32) -> u16| -> Vec<u8> {
        (0..4)
            .flat_map(|y| (0..10).map(move |x| (x, y)))
            .flat_map(|(x, y)| f(x, y).to_be_bytes())
            .collect()
    };
    for compression in [1, 3] {
        let mut doc = Doc::new(1, 16, 20, 10, vec![vec![0; 400]]);
        doc.layers_in_block = true;
        doc.layer_compression = compression;
        doc.layers = vec![TestLayer::new(
            "gray",
            [2, 3, 6, 13],
            vec![(-1, plane16(&|_, _| 65535)), (0, plane16(&value))],
        )];
        let opened = open_layers(&format!("gray16-layers-{compression}.psd"), &doc);
        let image = raster(&opened.document.layers()[0]);
        assert_eq!(image.format().sample, SampleType::U16);
        assert_eq!(image.format().layout, ChannelLayout::GrayAlpha);
        let px = pixel(image, 7, 3);
        assert_eq!(u16::from_ne_bytes([px[0], px[1]]), value(4, 1));
        assert_eq!(u16::from_ne_bytes([px[2], px[3]]), 65535);
    }

    // 32-bit RGB covering the canvas: linear light, blended linearly.
    let value = |x: u32, c: i16| x as f32 * 0.25 - 0.5 + f32::from(c);
    let channels = (-1..3)
        .map(|c| {
            let plane = (0..4)
                .flat_map(|_| (0..8).map(move |x| if c < 0 { 1.0 } else { value(x, c) }))
                .flat_map(f32::to_be_bytes)
                .collect();
            (c, plane)
        })
        .collect();
    let mut doc = Doc::new(3, 32, 8, 4, vec![vec![0; 128]; 3]);
    doc.layers_in_block = true;
    doc.layer_compression = 3;
    doc.layers = vec![TestLayer::new("hdr", [0, 0, 4, 8], channels)];
    let opened = open_layers("rgb32-layers.psd", &doc);
    assert_eq!(
        opened.document.blend_space(),
        slopshop_core::BlendSpace::Linear
    );
    let image = raster(&opened.document.layers()[0]);
    assert_eq!(
        image.format().color_space.transfer,
        TransferFunction::Linear
    );
    let px = pixel(image, 5, 2);
    for c in 0..3 {
        let at = c * 4;
        let v = f32::from_ne_bytes([px[at], px[at + 1], px[at + 2], px[at + 3]]);
        assert_eq!(v, value(5, c as i16));
    }
}

#[test]
fn groups_adjustments_clipping_and_styles_are_reported() {
    let full = |name: &'static str| {
        TestLayer::new(
            name,
            [0, 0, 16, 16],
            (-1..3).map(|c| (c, plane8(16, 16, |_, _| 100))).collect(),
        )
    };
    let mut divider = TestLayer::section("</Layer group>", 3);
    divider.channels = (-1..3).map(|c| (c, Vec::new())).collect();
    let mut group = TestLayer::section("Group", 1);
    group.opacity = 128;
    group.hidden = true;
    // Pass-through, as Photoshop writes it: in the section block, the record saying normal.
    group.blocks[0].1 = [&1u32.to_be_bytes()[..], b"8BIM", b"pass"].concat();
    let mut levels = TestLayer::new("Levels", [0; 4], Vec::new());
    levels.blocks.push((*b"levl", vec![0; 4]));
    let mut clipped = full("clipped");
    clipped.clipping = true;
    let mut text = full("text");
    text.blocks.push((*b"TySh", vec![0; 4]));
    let mut styled = full("styled");
    styled.blend = *b"fsub";
    styled.blocks.push((*b"lfx2", vec![0; 4]));
    styled.blocks.push((*b"iOpa", vec![128]));
    let mut vivid = full("vivid");
    vivid.blend = *b"vLit";
    vivid.blocks.push((*b"iOpa", vec![128]));

    let mut doc = Doc::new(3, 8, 16, 16, rgb8_planes(16, 16).0);
    doc.layers = vec![
        divider,
        full("child"),
        group,
        levels,
        clipped,
        text,
        styled,
        vivid,
    ];
    let opened = open_layers("groups.psd", &doc);
    let layers = opened.document.layers();
    let names: Vec<&str> = layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Group", "clipped", "text", "styled", "vivid"]);
    // The group, hidden at 50 %, passing through, holds its layer.
    let group = &layers[0];
    assert!(!group.visible);
    assert!((group.opacity - 128.0 / 255.0).abs() < 1e-6);
    let slopshop_core::LayerContent::Group {
        children,
        pass_through,
    } = &group.content
    else {
        panic!("a group expected");
    };
    assert!(*pass_through);
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "child");
    assert!(children[0].visible && children[0].opacity == 1.0);
    // Fill is the style's Fill Opacity (ADR 0032), the layer's opacity untouched.
    assert_eq!(layers[3].opacity, 1.0);
    let fill = layers[3].style.as_ref().unwrap().settings().fill_opacity;
    assert!((fill - 128.0 / 255.0).abs() < 1e-6, "fill opacity");
    assert_eq!(layers[3].blend_mode, slopshop_core::BlendMode::Subtract);
    // Clipped to the group below it (ADR 0016).
    assert!(layers[1].clipped && !layers[2].clipped);
    assert_eq!(opened.warnings, [ImportWarning::AdjustmentLayersSkipped]);
    use ImportWarning::*;
    // In the order of `all_layers`: the group, its layer, then the others.
    assert_eq!(
        opened.layer_warnings,
        [
            vec![],
            vec![],
            vec![],
            vec![LayersRasterized],
            vec![LayerStylesIgnored],
            // Vivid light is one of the modes where fill is not opacity.
            vec![LayerStylesIgnored],
        ]
    );
}

#[test]
fn unreadable_layers_fall_back_to_the_composite() {
    let (planes, color) = rgb8_planes(20, 10);
    let layer = || {
        TestLayer::new(
            "layer",
            [0, 0, 10, 20],
            (-1..3).map(|c| (c, plane8(20, 10, |_, _| 50))).collect(),
        )
    };
    // An unknown channel compression: the layers cannot be read, the composite can.
    let mut doc = Doc::new(3, 8, 20, 10, planes);
    doc.layers = vec![layer()];
    doc.layer_compression = 9;
    match open_file_of("bad-layers.psd", &doc).unwrap() {
        crate::Opened::Image(imported) => {
            assert_eq!(imported.warnings, [ImportWarning::LayersFlattened]);
            assert_eq!(pixel(&imported.image, 4, 7)[..3], color(4, 7));
        }
        crate::Opened::Layers(_) => panic!("the layers are damaged"),
    }
    // Without a composite, the layers are all there is.
    doc.merged = false;
    assert!(matches!(
        open_file_of("bad-layers-no-composite.psd", &doc),
        Err(ImportError::PsdWithoutComposite)
    ));
    doc.layer_compression = 1;
    assert!(matches!(
        open_file_of("layers-no-composite.psd", &doc),
        Ok(crate::Opened::Layers(_))
    ));
    // Asked for an image, a layered document gives its composite.
    doc.merged = true;
    let imported = open("layers-as-image.psd", &doc).unwrap();
    assert_eq!(imported.warnings, [ImportWarning::LayersFlattened]);

    // Every truncation is an error or a fallback, never a panic.
    let bytes = doc.write();
    for cut in (0..bytes.len()).step_by(7) {
        let path = temp_file("layers-cut.psd", &bytes[..cut]);
        let _ = crate::open_file(&path);
        std::fs::remove_file(&path).ok();
    }
}

#[test]
fn solid_color_fill_layers_become_fill_layers() {
    // A descriptor holding the color as Photoshop writes it (the values found by key).
    let mut soco = vec![0, 0, 0, 16];
    soco.extend(b"...Clr Objc...RGBC");
    for (key, value) in [
        (b"Rd  doub", 255.0f64),
        (b"Grn doub", 0.0),
        (b"Bl  doub", 0.0),
    ] {
        soco.extend([0, 0, 0, 0]);
        soco.extend(key);
        soco.extend(value.to_be_bytes());
    }
    // No pixels (empty bounds), a 4 × 2 mask at (5, 6).
    let mut fill = TestLayer::new("Color Fill 1", [0; 4], Vec::new());
    fill.channels = (-1..3).map(|c| (c, Vec::new())).collect();
    fill.channels.push((-2, plane8(4, 2, |x, _| x as u8 * 60)));
    fill.mask = Some(([6, 5, 8, 9], 0, 0));
    fill.blocks.push((*b"SoCo", soco.clone()));
    // A vector-only shape: nothing to import without rendering its outline.
    let mut shape = TestLayer::new("Shape 1", [0; 4], Vec::new());
    shape.blocks.push((*b"SoCo", soco));
    shape.blocks.push((*b"vmsk", vec![0; 8]));

    let mut doc = Doc::new(3, 8, 16, 16, rgb8_planes(16, 16).0);
    doc.layers = vec![fill, shape];
    let opened = open_layers("solid.psd", &doc);
    assert_eq!(opened.warnings, [ImportWarning::AdjustmentLayersSkipped]);
    let [layer] = opened.document.layers() else {
        panic!("one layer expected");
    };
    let slopshop_core::LayerContent::Fill { color } = &layer.content else {
        panic!("a fill layer expected");
    };
    // Back from the working space to linear sRGB: pure red.
    let to_srgb = slopshop_core::color::WORKING_SPACE.matrix_to(&ColorSpace::LINEAR_SRGB);
    let srgb = color.transform(&to_srgb);
    let (r, g, b) = (srgb.r, srgb.g, srgb.b);
    assert!((r - 1.0).abs() < 1e-3 && g.abs() < 1e-3 && b.abs() < 1e-3);
    let mask = layer.mask.as_ref().unwrap();
    assert_eq!(pixel(&mask.image, 7, 7), [120]);
    assert_eq!(pixel(&mask.image, 0, 0), [0]);
    assert_eq!(opened.layer_warnings, [Vec::<ImportWarning>::new()]);
}

#[test]
fn adjustment_layers_are_imported_with_their_settings() {
    use slopshop_core::adjust::Adjustment;
    // An adjustment layer: no pixels, one tagged block.
    let adjustment = |name: &'static str, key: &[u8; 4], block: Vec<u8>| {
        let mut layer = TestLayer::new(name, [0; 4], Vec::new());
        layer.channels = (-1..3).map(|c| (c, Vec::new())).collect();
        layer.blocks.push((*key, block));
        layer
    };
    let be16 = |values: &[i16]| {
        values
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect::<Vec<u8>>()
    };
    // Levels: version, the composite record, then channel records (identity, except one).
    let mut levels = be16(&[2, 7, 235, 0, 255, 72]);
    levels.extend(be16(&[0, 255, 0, 255, 100]));
    let mut per_channel = levels.clone();
    per_channel.extend(be16(&[10, 255, 0, 255, 100]));
    // Exposure: version, then three floats.
    let mut exposure = be16(&[1]);
    for v in [-0.39f32, 0.0168, 0.91] {
        exposure.extend(v.to_be_bytes());
    }
    // Hue/Saturation: version, colorize, padding, colorization, master, six empty ranges.
    let hue = |colorize: u8| {
        let mut b = be16(&[2]);
        b.extend([colorize, 0]);
        b.extend(be16(&[0, 25, 0, -17, 19, 4]));
        b.extend(vec![0; 6 * 14]);
        b
    };
    // Brightness/Contrast: zeros in `brit`, the values in its descriptor.
    let mut cged = vec![0, 0, 0, 16];
    for (key, value) in [(b"Brgh", 34i32), (b"Cntr", 18)] {
        cged.extend([0, 0, 0, 0]);
        cged.extend(key);
        cged.extend(b"long");
        cged.extend(value.to_be_bytes());
    }
    cged.extend([0, 0, 0, 9]);
    cged.extend(b"useLegacy");
    cged.extend(b"bool");
    cged.push(0);
    let mut brightness = adjustment("Brightness/Contrast 1", b"brit", be16(&[0, 0, 127, 0]));
    brightness.blocks.push((*b"CgEd", cged));
    let mut vibrance = vec![0, 0, 0, 16];
    for (key, value) in [(&b"vibrance"[..], -6i32), (&b"Strt"[..], 2)] {
        let length = if key.len() == 4 {
            0u32
        } else {
            key.len() as u32
        };
        vibrance.extend(length.to_be_bytes());
        vibrance.extend(key);
        vibrance.extend(b"long");
        vibrance.extend(value.to_be_bytes());
    }

    let mut doc = Doc::new(3, 8, 16, 16, rgb8_planes(16, 16).0);
    let background = TestLayer::new(
        "Background",
        [0, 0, 16, 16],
        (0..3)
            .map(|c| (c, plane8(16, 16, |x, _| x as u8 * 10)))
            .collect(),
    );
    doc.layers = vec![
        background,
        brightness,
        adjustment("Levels 1", b"levl", levels),
        adjustment("Levels 2", b"levl", per_channel),
        adjustment("Exposure 1", b"expA", exposure),
        adjustment("Vibrance 1", b"vibA", vibrance),
        adjustment("Hue/Saturation 1", b"hue2", hue(0)),
        adjustment("Colorize", b"hue2", hue(1)),
        adjustment("Invert 1", b"nvrt", Vec::new()),
        adjustment("Posterize 1", b"post", be16(&[4, 0])),
        adjustment("Threshold 1", b"thrs", be16(&[128, 0])),
        adjustment("Selective Color 1", b"selc", {
            // Absolute; the unused record, then reds and blacks set, the others 0.
            let mut b = be16(&[1, 1, 0, 0, 0, 0, 20, -10, 0, 5]);
            b.extend(be16(&[0; 4 * 7]));
            b.extend(be16(&[0, 0, 0, -30]));
            b
        }),
    ];
    let opened = open_layers("adjustments.psd", &doc);
    // Colorize is not reproduced yet.
    assert_eq!(opened.warnings, [ImportWarning::AdjustmentLayersSkipped]);
    let adjustments: Vec<(String, Adjustment)> = opened
        .document
        .layers()
        .iter()
        .filter_map(|l| match l.content {
            slopshop_core::LayerContent::Adjustment { adjustment } => {
                Some((l.name.clone(), adjustment))
            }
            _ => None,
        })
        .collect();
    let names: Vec<&str> = adjustments.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        [
            "Brightness/Contrast 1",
            "Levels 1",
            "Levels 2",
            "Exposure 1",
            "Vibrance 1",
            "Hue/Saturation 1",
            "Invert 1",
            "Posterize 1",
            "Threshold 1",
            "Selective Color 1"
        ]
    );
    let find = |name: &str| {
        adjustments
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, a)| *a)
            .unwrap()
    };
    assert_eq!(
        find("Brightness/Contrast 1"),
        Adjustment::BrightnessContrast {
            brightness: 34.0,
            contrast: 18.0
        }
    );
    assert_eq!(
        find("Levels 1"),
        Adjustment::Levels {
            input_black: 7.0 / 255.0,
            input_white: 235.0 / 255.0,
            gamma: 0.72,
            output_black: 0.0,
            channels: [slopshop_core::adjust::LEVELS_IDENTITY; 3],
            output_white: 1.0
        }
    );
    assert_eq!(
        find("Exposure 1"),
        Adjustment::Exposure {
            exposure: -0.39,
            offset: 0.0168,
            gamma: 0.91
        }
    );
    assert_eq!(
        find("Vibrance 1"),
        Adjustment::Vibrance {
            vibrance: -6.0,
            saturation: 2.0
        }
    );
    assert_eq!(
        find("Hue/Saturation 1"),
        Adjustment::HueSaturation {
            hue: -17.0,
            saturation: 19.0,
            lightness: 4.0
        }
    );
    assert_eq!(find("Posterize 1"), Adjustment::Posterize { levels: 4.0 });
    assert_eq!(
        find("Threshold 1"),
        Adjustment::Threshold {
            level: 128.0 / 255.0
        }
    );
    let mut ranges = [[0i16; 4]; slopshop_core::adjust::SELECTIVE_RANGES];
    ranges[0] = [20, -10, 0, 5];
    ranges[8] = [0, 0, 0, -30];
    assert_eq!(
        find("Selective Color 1"),
        Adjustment::SelectiveColor {
            ranges,
            absolute: true
        }
    );
    // Levels with a channel of its own: the green one, exactly.
    assert_eq!(
        find("Levels 2"),
        Adjustment::Levels {
            input_black: 7.0 / 255.0,
            input_white: 235.0 / 255.0,
            gamma: 0.72,
            output_black: 0.0,
            output_white: 1.0,
            channels: [
                slopshop_core::adjust::LEVELS_IDENTITY,
                [10.0 / 255.0, 1.0, 1.0, 0.0, 1.0],
                slopshop_core::adjust::LEVELS_IDENTITY,
            ],
        }
    );
    let warnings_of = |name: &str| {
        opened
            .document
            .all_layers()
            .zip(&opened.layer_warnings)
            .find(|(l, _)| l.name == name)
            .map(|(_, w)| w.clone())
            .unwrap()
    };
    assert!(warnings_of("Levels 2").is_empty());
    assert!(warnings_of("Levels 1").is_empty());
}

#[test]
fn adjustments_of_many_settings_are_imported() {
    use slopshop_core::adjust::Adjustment;
    let adjustment = |name: &'static str, key: &[u8; 4], block: Vec<u8>| {
        let mut layer = TestLayer::new(name, [0; 4], Vec::new());
        layer.channels = (-1..3).map(|c| (c, Vec::new())).collect();
        layer.blocks.push((*key, block));
        layer
    };
    let be16 = |values: &[i16]| {
        values
            .iter()
            .flat_map(|v| v.to_be_bytes())
            .collect::<Vec<u8>>()
    };
    // Black & White, as Photoshop writes it (fill_adjustments.psd of psd-tools), tinted.
    let item = |out: &mut Vec<u8>, key: &[u8], ty: &[u8; 4]| {
        let length = if key.len() == 4 {
            0u32
        } else {
            key.len() as u32
        };
        out.extend(length.to_be_bytes());
        out.extend(key);
        out.extend(ty);
    };
    let mut blwh = vec![0, 0, 0, 16];
    for (key, value) in [
        (&b"Rd  "[..], 40i32),
        (b"Yllw", 60),
        (b"Grn ", 40),
        (b"Cyn ", -60),
        (b"Bl  ", 20),
        (b"Mgnt", 280),
    ] {
        item(&mut blwh, key, b"long");
        blwh.extend(value.to_be_bytes());
    }
    item(&mut blwh, b"useTint", b"bool");
    blwh.push(1);
    item(&mut blwh, b"tintColor", b"Objc");
    for (key, value) in [(b"Rd  ", 225.0f64), (b"Grn ", 211.0), (b"Bl  ", 179.0)] {
        item(&mut blwh, key, b"doub");
        blwh.extend(value.to_be_bytes());
    }
    // Color Balance: psd-tools' sample values.
    let mut blnc = be16(&[-4, 2, -5, 10, 4, -9, 1, -9, -3]);
    blnc.extend([1, 0, 0, 0]);
    // Photo Filter, version 2 in Lab (Photoshop's Warming Filter 85: out of sRGB, clipped).
    let mut phfl = be16(&[2, 7, 6706, 3200, 12000, 0]);
    phfl.extend(25u32.to_be_bytes());
    phfl.push(1);
    // Curves, as Photoshop writes them (psd-tools' sample): the composite curve only, points
    // as (output, input), then the same in a `Crv ` block.
    let mut curv = vec![0, 0, 1, 0, 0, 0, 1];
    let points = be16(&[3, 5, 0, 102, 131, 236, 248]);
    curv.extend(&points);
    curv.extend(b"Crv ");
    curv.extend(be16(&[4, 0, 1, 0]));
    curv.extend(&points);
    // Channel Mixer: four records (red, green, blue, unused).
    let mut mixr = be16(&[1, 0]);
    mixr.extend(be16(&[80, 30, -10, 0, 5]));
    mixr.extend(be16(&[0, 100, 0, 0, 0]));
    mixr.extend(be16(&[0, 0, 100, 0, -20]));
    mixr.extend(be16(&[0, 0, 0, 100, 0]));

    let mut doc = Doc::new(3, 8, 16, 16, rgb8_planes(16, 16).0);
    let background = TestLayer::new(
        "Background",
        [0, 0, 16, 16],
        (0..3)
            .map(|c| (c, plane8(16, 16, |x, _| x as u8 * 10)))
            .collect(),
    );
    doc.layers = vec![
        background,
        adjustment("Black & White 1", b"blwh", blwh),
        adjustment("Color Balance 1", b"blnc", blnc),
        adjustment("Photo Filter 1", b"phfl", phfl),
        adjustment("Channel Mixer 1", b"mixr", mixr),
        adjustment("Curves 1", b"curv", curv),
    ];
    let opened = open_layers("more-adjustments.psd", &doc);
    let found: Vec<Adjustment> = opened
        .document
        .layers()
        .iter()
        .filter_map(|l| match l.content {
            slopshop_core::LayerContent::Adjustment { adjustment } => Some(adjustment),
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 5);
    assert_eq!(
        found[0],
        Adjustment::BlackWhite {
            weights: [40.0, 60.0, 40.0, -60.0, 20.0, 280.0],
            tint: true,
            tint_hue: 42.0,
            tint_saturation: 20.0,
        }
    );
    assert_eq!(
        found[1],
        Adjustment::ColorBalance {
            shadows: [-4.0, 2.0, -5.0],
            midtones: [10.0, 4.0, -9.0],
            highlights: [1.0, -9.0, -3.0],
            preserve_luminosity: true,
        }
    );
    let Adjustment::PhotoFilter {
        color,
        density,
        preserve_luminosity,
    } = found[2]
    else {
        panic!("{:?}", found[2]);
    };
    // Photoshop shows this filter as (236, 138, 0).
    let expected = [236.0 / 255.0, 138.0 / 255.0, 0.0];
    assert!(
        color
            .iter()
            .zip(expected)
            .all(|(c, e)| (c - e).abs() < 3.0 / 255.0),
        "{color:?}"
    );
    assert_eq!((density, preserve_luminosity), (25.0, true));
    assert_eq!(
        found[3],
        Adjustment::ChannelMixer {
            red: [80.0, 30.0, -10.0, 5.0],
            green: [0.0, 100.0, 0.0, 0.0],
            blue: [0.0, 0.0, 100.0, -20.0],
            monochrome: false,
        }
    );
    use slopshop_core::curve::Curve;
    assert_eq!(
        found[4],
        Adjustment::Curves {
            rgb: Curve::new(&[[0, 5], [131, 102], [248, 236]]).unwrap(),
            red: Curve::IDENTITY,
            green: Curve::IDENTITY,
            blue: Curve::IDENTITY,
        }
    );
    // Only the tint is approximated.
    let approximated: Vec<bool> = opened.layer_warnings[1..]
        .iter()
        .map(|w| w.contains(&ImportWarning::AdjustmentsApproximated))
        .collect();
    assert_eq!(approximated, [true, false, false, false, false]);
}
