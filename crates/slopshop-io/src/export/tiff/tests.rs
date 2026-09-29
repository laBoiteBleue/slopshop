use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};

use ::tiff::decoder::{Decoder, DecodingResult};
use ::tiff::encoder::{Compression, colortype};
use slopshop_core::color::ColorSpace;
use slopshop_core::convert::{ConversionReport, ConvertOptions, Converter};
use slopshop_core::{CancelToken, Rect};

use super::*;
use crate::export::{ExportFormat, ExportReport, ExportSpec, TiffSample, export_image, temp_files};
use crate::open_image;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("slopshop-tiff-{}-{name}", std::process::id()))
}

/// Heights not multiple of the band or strip height, widths spanning several tiles.
const ODD_SIZE: Size = Size::new(300, 520);

/// A deterministic image: premultiplied working-space RGBA within [0, 1], alpha > 0.
fn pattern(region: Rect, out: &mut [f32]) -> Result<u64, String> {
    for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let x = region.x + i as u32 % region.width;
        let y = region.y + i as u32 / region.width;
        let alpha = 0.25 + ((x * 7 + y * 3) % 64) as f32 / 84.0;
        let r = ((x * 13 + y) % 97) as f32 / 96.0;
        let g = (y % 256) as f32 / 255.0;
        let b = ((x ^ y) % 200) as f32 / 199.0;
        *px = [r * alpha, g * alpha, b * alpha, alpha];
    }
    Ok(0)
}

/// The samples the file must hold: the pattern through the same conversion.
fn expected(size: Size, spec: &ExportSpec) -> Vec<u8> {
    let options = ConvertOptions {
        dither: spec.dither,
        big_endian: false,
    };
    let converter = Converter::new(spec.target_format(), options).unwrap();
    let row_bytes = size.width as usize * converter.bytes_per_pixel();
    let mut out = vec![0; row_bytes * size.height as usize];
    let mut src = vec![0.0; size.width as usize * 4];
    for (y, dst) in out.chunks_exact_mut(row_bytes).enumerate() {
        let y = y as u32;
        pattern(Rect::new(0, y, size.width, 1), &mut src).unwrap();
        let mut report = ConversionReport::default();
        converter.convert_row(&src, 0, y, dst, &mut report).unwrap();
    }
    out
}

fn tiff_spec(sample: TiffSample, compression: TiffCompression, keep_alpha: bool) -> ExportSpec {
    ExportSpec {
        format: ExportFormat::Tiff {
            sample,
            compression,
        },
        // Rec.2020 primaries, like the working space: nothing out of gamut.
        space: match sample {
            TiffSample::F32 => ColorSpace::LINEAR_REC2020,
            TiffSample::U8 | TiffSample::U16 => ColorSpace::REC2020,
        },
        keep_alpha,
        dither: true,
    }
}

fn export_pattern(path: &Path, size: Size, spec: &ExportSpec) -> ExportReport {
    export_image(path, size, spec, pattern, &CancelToken::new(), &mut |_| {}).unwrap()
}

fn decoder(path: &Path) -> Decoder<BufReader<File>> {
    Decoder::new(BufReader::new(File::open(path).unwrap())).unwrap()
}

/// Samples decoded by tiff, as little-endian bytes.
fn decode(decoder: &mut Decoder<BufReader<File>>) -> Vec<u8> {
    match decoder.read_image().unwrap() {
        DecodingResult::U8(v) => v,
        DecodingResult::U16(v) => v.iter().flat_map(|s| s.to_le_bytes()).collect(),
        DecodingResult::F32(v) => v.iter().flat_map(|s| s.to_le_bytes()).collect(),
        _ => panic!("unexpected sample type"),
    }
}

/// Tag, type and count of each entry of the first directory of a little-endian (Big)TIFF.
fn entries(bytes: &[u8]) -> Vec<(u16, u16, u64)> {
    let u16_at = |i: usize| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap());
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    let u64_at = |i: usize| u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
    assert_eq!(&bytes[..2], b"II");
    let big = u16_at(2) == 43;
    let (count, first, len) = if big {
        let ifd = u64_at(8) as usize;
        (u64_at(ifd) as usize, ifd + 8, 20)
    } else {
        let ifd = u32_at(4) as usize;
        (usize::from(u16_at(ifd)), ifd + 2, 12)
    };
    (0..count)
        .map(|i| {
            let entry = first + i * len;
            let count = if big {
                u64_at(entry + 4)
            } else {
                u64::from(u32_at(entry + 4))
            };
            (u16_at(entry), u16_at(entry + 2), count)
        })
        .collect()
}

const STRIP_OFFSETS: u16 = 273;
const ICC_PROFILE: u16 = 34675;
const UNDEFINED: u16 = 7;
const LONG8: u16 = 16;

#[test]
fn every_combination_round_trips() {
    let compressions = [
        TiffCompression::None,
        TiffCompression::Deflate,
        TiffCompression::Lzw,
    ];
    for sample in [TiffSample::U8, TiffSample::U16, TiffSample::F32] {
        for keep_alpha in [false, true] {
            let mut uncompressed_len = 0;
            for compression in compressions {
                let case = format!("{sample:?} {compression:?} alpha {keep_alpha}");
                let spec = tiff_spec(sample, compression, keep_alpha);
                let target = spec.target_format();
                let path = temp_path("round-trip.tif");
                let report = export_pattern(&path, ODD_SIZE, &spec);
                // Nothing clipped, classic TIFF.
                assert_eq!(report, ExportReport::default(), "{case}");

                // Through tiff: the exact samples, and the tags.
                let mut tiff = decoder(&path);
                assert_eq!(decode(&mut tiff), expected(ODD_SIZE, &spec), "{case}");
                let channels = if keep_alpha { 4 } else { 3 };
                let (bits, format) = match sample {
                    TiffSample::U8 => (8, 1),
                    TiffSample::U16 => (16, 1),
                    TiffSample::F32 => (32, 3),
                };
                let (method, predictor) = match compression {
                    TiffCompression::None => (1, 1),
                    TiffCompression::Lzw => (5, 2),
                    TiffCompression::Deflate => (8, 2),
                };
                let predictor = if sample == TiffSample::F32 {
                    1
                } else {
                    predictor
                };
                let mut tag = |tag| tiff.get_tag_u16_vec(tag).unwrap();
                assert_eq!(tag(Tag::BitsPerSample), vec![bits; channels], "{case}");
                assert_eq!(tag(Tag::SampleFormat), vec![format; channels], "{case}");
                assert_eq!(tag(Tag::Compression), [method], "{case}");
                assert_eq!(tag(Tag::Predictor), [predictor], "{case}");
                assert_eq!(tag(Tag::PhotometricInterpretation), [2], "{case}");
                assert_eq!(tag(Tag::PlanarConfiguration), [1], "{case}");
                let extra = match (keep_alpha, target.alpha) {
                    (false, _) => None,
                    (true, AlphaMode::Straight) => Some(vec![2]),
                    (true, AlphaMode::Premultiplied) => Some(vec![1]),
                };
                let found = tiff.find_tag_unsigned_vec::<u16>(Tag::ExtraSamples);
                assert_eq!(found.unwrap(), extra, "{case}");

                // Every tag written explicitly, the profile as UNDEFINED.
                let bytes = std::fs::read(&path).unwrap();
                assert_eq!(&bytes[..4], b"II*\0", "{case}");
                let tags: Vec<u16> = entries(&bytes).iter().map(|e| e.0).collect();
                let mut expected_tags = vec![
                    256,
                    257,
                    258,
                    259,
                    262,
                    273,
                    277,
                    278,
                    279,
                    282,
                    283,
                    284,
                    296,
                    317,
                    339,
                    ICC_PROFILE,
                ];
                if keep_alpha {
                    expected_tags.push(338);
                    expected_tags.sort_unstable();
                }
                assert_eq!(tags, expected_tags, "{case}");
                let icc = entries(&bytes).into_iter().find(|e| e.0 == ICC_PROFILE);
                assert_eq!(icc.map(|e| e.1), Some(UNDEFINED), "{case}");

                // The strips really are compressed (see `upstream_write_strip_ignores_compression`;
                // LZW may expand the noisy low bits of 16-bit samples).
                match compression {
                    TiffCompression::None => uncompressed_len = bytes.len(),
                    TiffCompression::Deflate => assert!(bytes.len() < uncompressed_len, "{case}"),
                    TiffCompression::Lzw => {}
                }

                // Through our importer: the same format and space.
                let imported = open_image(&path).unwrap();
                let format = imported.image.format();
                assert_eq!(format.layout.has_alpha(), keep_alpha, "{case}");
                assert_eq!(format.sample, spec.format.sample_type(), "{case}");
                if keep_alpha {
                    assert_eq!(format.alpha, target.alpha, "{case}");
                }
                assert_eq!(format.color_space, spec.space, "{case}");
                assert!(imported.warnings.is_empty(), "{case}");
                std::fs::remove_file(&path).ok();
            }
        }
    }
}

/// Write the expected samples of `spec` with a writer that always chooses BigTIFF.
fn write_big(path: &Path, size: Size, spec: &ExportSpec) -> Vec<ExportNotice> {
    let ExportFormat::Tiff { compression, .. } = spec.format else {
        panic!("not a TIFF spec");
    };
    let target = spec.target_format();
    let file = File::create(path).unwrap();
    let mut writer =
        TiffWriter::with_big_tiff_threshold(file, size, target, compression, 0).unwrap();
    let samples = expected(size, spec);
    let band_bytes = BAND_ROWS as usize * size.width as usize * target.bytes_per_pixel() as usize;
    for (i, band) in samples.chunks(band_bytes).enumerate() {
        writer.write_rows(i as u32 * BAND_ROWS, band).unwrap();
    }
    writer.finish().unwrap()
}

#[test]
fn forced_big_tiff_round_trips() {
    for (sample, compression) in [
        (TiffSample::U16, TiffCompression::Deflate),
        (TiffSample::U8, TiffCompression::Lzw),
        (TiffSample::F32, TiffCompression::None),
    ] {
        let case = format!("{sample:?} {compression:?}");
        let spec = tiff_spec(sample, compression, true);
        let path = temp_path("big.tif");
        let notices = write_big(&path, ODD_SIZE, &spec);
        assert_eq!(notices, [ExportNotice::BigTiff], "{case}");
        let bytes = std::fs::read(&path).unwrap();
        // "II", version 43, 8-byte offsets.
        assert_eq!(bytes[..6], [0x49, 0x49, 0x2B, 0x00, 0x08, 0x00], "{case}");
        let offsets = entries(&bytes).into_iter().find(|e| e.0 == STRIP_OFFSETS);
        assert_eq!(offsets.map(|e| e.1), Some(LONG8), "{case}");

        let decoded = decode(&mut decoder(&path));
        assert_eq!(decoded, expected(ODD_SIZE, &spec), "{case}");
        let imported = open_image(&path).unwrap();
        assert_eq!(imported.image.format().color_space, spec.space, "{case}");
        assert!(imported.warnings.is_empty(), "{case}");
        std::fs::remove_file(&path).ok();
    }
}

/// Offset of the first directory of a little-endian (Big)TIFF, and the offset of each value it
/// stores out of line, with its tag.
fn value_offsets(bytes: &[u8]) -> (u64, Vec<(u16, u64)>) {
    let u16_at = |i: usize| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap());
    let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
    let u64_at = |i: usize| u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
    let big = u16_at(2) == 43;
    let (ifd, first, len, inline) = if big {
        let ifd = u64_at(8);
        (ifd, ifd as usize + 8, 20, 8)
    } else {
        let ifd = u64::from(u32_at(4));
        (ifd, ifd as usize + 2, 12, 4)
    };
    let values = entries(bytes)
        .into_iter()
        .enumerate()
        .filter_map(|(i, (tag, ty, count))| {
            let size = match ty {
                1 | 2 | 6 | 7 => 1,
                3 | 8 => 2,
                4 | 9 | 11 => 4,
                5 | 10 | 12 | 16 => 8,
                other => panic!("unexpected type {other}"),
            };
            let field = first + i * len + if big { 12 } else { 8 };
            (count * size > inline).then(|| {
                let offset = if big {
                    u64_at(field)
                } else {
                    u64::from(u32_at(field))
                };
                (tag, offset)
            })
        })
        .collect();
    (ifd, values)
}

#[test]
fn directory_and_values_are_word_aligned() {
    // Odd width, 8-bit RGB: rows of an odd number of bytes, so that the strips often end at an
    // odd offset, compressed or not. TIFF 6.0 wants the directory and its values at even ones.
    let mut odd_strip_ends = 0;
    for compression in [
        TiffCompression::None,
        TiffCompression::Lzw,
        TiffCompression::Deflate,
    ] {
        for height in [1, 2, 3, 5] {
            let case = format!("{compression:?} height {height}");
            let size = Size::new(301, height);
            let spec = tiff_spec(TiffSample::U8, compression, false);
            let path = temp_path("aligned.tif");
            export_pattern(&path, size, &spec);
            let bytes = std::fs::read(&path).unwrap();
            let mut tiff = decoder(&path);
            let offsets = tiff.get_tag_u64_vec(Tag::StripOffsets).unwrap();
            let counts = tiff.get_tag_u64_vec(Tag::StripByteCounts).unwrap();
            let strip_end = offsets.last().unwrap() + counts.last().unwrap();
            odd_strip_ends += strip_end % 2;

            let (ifd, values) = value_offsets(&bytes);
            assert_eq!(ifd % 2, 0, "{case}: directory at {ifd}");
            // At least BitsPerSample, the resolutions, SampleFormat and the ICC profile.
            assert!(values.len() >= 5, "{case}: {values:?}");
            for (tag, offset) in values {
                assert_eq!(offset % 2, 0, "{case}: tag {tag} at {offset}");
            }
            // Still read correctly.
            assert_eq!(decode(&mut tiff), expected(size, &spec), "{case}");
            std::fs::remove_file(&path).ok();
        }
    }
    // The padding really was exercised.
    assert!(odd_strip_ends > 0);

    // BigTIFF likewise.
    let spec = tiff_spec(TiffSample::U8, TiffCompression::None, false);
    let path = temp_path("aligned-big.tif");
    write_big(&path, Size::new(301, 3), &spec);
    let (ifd, values) = value_offsets(&std::fs::read(&path).unwrap());
    assert_eq!(ifd % 2, 0, "BigTIFF directory at {ifd}");
    assert!(!values.is_empty());
    for (tag, offset) in values {
        assert_eq!(offset % 2, 0, "BigTIFF tag {tag} at {offset}");
    }
    std::fs::remove_file(&path).ok();
}

#[test]
fn icc_profile_reads_back_as_the_same_space() {
    let size = Size::new(8, 4);
    for space in [
        ColorSpace::SRGB,
        ColorSpace::DISPLAY_P3,
        ColorSpace::ADOBE_RGB,
        ColorSpace::PROPHOTO,
        ColorSpace::REC2020,
        ColorSpace::LINEAR_REC2020,
        ColorSpace::LINEAR_SRGB,
    ] {
        for sample in [TiffSample::U16, TiffSample::F32] {
            let spec = ExportSpec {
                space,
                ..tiff_spec(sample, TiffCompression::Deflate, false)
            };
            let path = temp_path("icc.tif");
            export_pattern(&path, size, &spec);
            let imported = open_image(&path).unwrap();
            assert_eq!(imported.image.format().color_space, space, "{space:?}");
            assert!(imported.warnings.is_empty(), "{space:?}");
            std::fs::remove_file(&path).ok();
        }
    }
}

#[test]
fn big_tiff_is_chosen_from_the_size_bound() {
    let rgb8 = PixelFormat {
        layout: ChannelLayout::Rgb,
        sample: SampleType::U8,
        color_space: ColorSpace::SRGB,
        alpha: AlphaMode::Straight,
    };
    let bound = |size, compression| {
        let layout = Layout::new(size, rgb8, compression).unwrap();
        size_bound(&layout, 1000).unwrap()
    };
    // 1.2 GB and 4.8 GB of samples.
    let (medium, huge) = (Size::new(20_000, 20_000), Size::new(40_000, 40_000));
    for compression in [TiffCompression::None, TiffCompression::Deflate] {
        assert!(bound(medium, compression) < BIG_TIFF_THRESHOLD);
        assert!(bound(huge, compression) > BIG_TIFF_THRESHOLD);
    }
    // 3.2 GB of samples: LZW may expand them by half.
    let large = Size::new(40_000, 26_667);
    assert!(bound(large, TiffCompression::Deflate) < BIG_TIFF_THRESHOLD);
    assert!(bound(large, TiffCompression::Lzw) > BIG_TIFF_THRESHOLD);
    // Beyond u64.
    let layout = Layout::new(Size::new(u32::MAX, u32::MAX), rgb8, TiffCompression::None);
    assert_eq!(size_bound(&layout.unwrap(), 0), None);
}

#[test]
fn strips_are_power_of_two_rows_of_at_most_1_mib() {
    assert_eq!(strip_rows(1), MAX_STRIP_ROWS);
    assert_eq!(strip_rows(1200), MAX_STRIP_ROWS);
    // 1 MiB / 40 000 = 26.2.
    assert_eq!(strip_rows(40_000), 16);
    assert_eq!(strip_rows(1 << 20), 1);
    assert_eq!(strip_rows(3 << 20), 1);
    for row_bytes in [1, 3, 1000, 70_000, 1 << 19, 1 << 25] {
        let rows = strip_rows(row_bytes);
        assert!(BAND_ROWS.is_multiple_of(rows));
        assert!(rows == 1 || u64::from(rows) * row_bytes <= STRIP_BYTES);
    }
}

#[test]
fn horizontal_predictor_differences_each_channel() {
    let mut row = [10, 20, 30, 15, 25, 5, 16, 26, 6];
    predict_row(&mut row, SampleType::U8, 3);
    assert_eq!(row, [10, 20, 30, 5, 5, 231, 1, 1, 1]);

    let samples: [u16; 4] = [1000, 60_000, 900, 60_100];
    let mut row: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    predict_row(&mut row, SampleType::U16, 4);
    let predicted: Vec<u16> = row
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&b| u16::from_le_bytes(b))
        .collect();
    assert_eq!(predicted, [1000, 60_000, 900u16.wrapping_sub(1000), 100]);
}

#[test]
fn rows_must_come_in_order_and_in_whole_strips() {
    let size = Size::new(64, 100);
    let target = tiff_spec(TiffSample::U8, TiffCompression::Deflate, false).target_format();
    let rows = |count: usize| vec![0; count * 64 * 3];
    let path = temp_path("rows.tif");
    let new_writer = || {
        let file = File::create(&path).unwrap();
        TiffWriter::new(file, size, target, TiffCompression::Deflate).unwrap()
    };

    // Strips of MAX_STRIP_ROWS (32) rows.
    let mut writer = new_writer();
    assert!(writer.write_rows(0, &rows(33)).is_err());
    assert!(writer.write_rows(32, &rows(32)).is_err());
    writer.write_rows(0, &rows(32)).unwrap();
    assert!(writer.write_rows(32, &[0; 10]).is_err());
    assert!(writer.write_rows(32, &rows(96)).is_err());
    // The last rows may fill a strip partially.
    writer.write_rows(32, &rows(68)).unwrap();
    writer.finish().unwrap();

    // Finishing early is an error, and closes the file so that it can be deleted.
    let mut writer = new_writer();
    writer.write_rows(0, &rows(32)).unwrap();
    assert_eq!(writer.finish().unwrap_err().code(), "encode");
    std::fs::remove_file(&path).unwrap();

    // So does dropping the writer.
    let mut writer = new_writer();
    writer.write_rows(0, &rows(32)).unwrap();
    drop(writer);
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn cancelling_a_tiff_export_leaves_no_file() {
    let path = temp_path("cancel.tif");
    let cancel = CancelToken::new();
    let result = export_image(
        &path,
        Size::new(16, 1000),
        &tiff_spec(TiffSample::U16, TiffCompression::Lzw, true),
        pattern,
        &cancel,
        // Cancel after the first band.
        &mut |_| cancel.cancel(),
    );
    assert_eq!(result.unwrap_err().code(), "cancelled");
    assert!(!path.exists());
    assert!(temp_files(&path).is_empty());
}

#[test]
fn unsupported_targets_are_rejected() {
    let path = temp_path("unsupported.tif");
    let target = |layout, sample| PixelFormat {
        layout,
        sample,
        color_space: ColorSpace::LINEAR_SRGB,
        alpha: AlphaMode::Premultiplied,
    };
    let pq = PixelFormat {
        color_space: ColorSpace::REC2100_PQ,
        ..target(ChannelLayout::Rgb, SampleType::U16)
    };
    for (format, code) in [
        (target(ChannelLayout::Gray, SampleType::U8), "invalidSpec"),
        (target(ChannelLayout::Rgba, SampleType::F16), "invalidSpec"),
        (pq, "unsupportedSpace"),
    ] {
        let file = File::create(&path).unwrap();
        let result = TiffWriter::new(file, Size::new(2, 2), format, TiffCompression::None);
        assert_eq!(result.err().map(|e| e.code()), Some(code), "{format:?}");
    }
    let file = File::create(&path).unwrap();
    let rgb = target(ChannelLayout::Rgb, SampleType::U8);
    let result = TiffWriter::new(file, Size::new(0, 2), rgb, TiffCompression::None);
    assert_eq!(result.err().map(|e| e.code()), Some("invalidSpec"));
    std::fs::remove_file(&path).ok();
}

/// tiff 0.11.3's `ImageEncoder::write_strip` declares the compression but writes the strips
/// uncompressed: this is why the writer compresses its own strips. If this test fails,
/// upstream fixed it, and the workaround may be reconsidered.
#[test]
fn upstream_write_strip_ignores_compression() {
    let (width, height) = (64, 16);
    // Very compressible.
    let data: Vec<u8> = (0..width * height * 3).map(|i| (i % 7) as u8).collect();
    let mut file = Cursor::new(Vec::new());
    {
        let mut encoder = ::tiff::encoder::TiffEncoder::new(&mut file)
            .unwrap()
            .with_compression(Compression::Lzw);
        let mut image = encoder.new_image::<colortype::RGB8>(width, height).unwrap();
        image.rows_per_strip(height).unwrap();
        image.write_strip(&data).unwrap();
        image.finish().unwrap();
    }
    let bytes = file.into_inner();
    // Declared LZW, stored raw: the file holds the samples as they are, and does not decode.
    assert!(bytes.windows(data.len()).any(|w| w == data));
    let mut decoder = Decoder::new(Cursor::new(&bytes)).unwrap();
    assert_eq!(decoder.get_tag_u32(Tag::Compression).unwrap(), 5);
    let decoded = decoder.read_image();
    assert!(!matches!(decoded, Ok(DecodingResult::U8(v)) if v == data));
}
