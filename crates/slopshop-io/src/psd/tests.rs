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
        if self.layer_count == 0 {
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
