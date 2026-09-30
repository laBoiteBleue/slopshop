use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use slopshop_core::color::{
    AlphaMode, ChannelLayout, ColorSpace, LinearRgba, PixelFormat, SampleType,
};
use slopshop_core::document::{Layer, LayerContent};
use slopshop_core::geom::Size;
use slopshop_core::raster::RasterImage;
use slopshop_core::{BlendMode, BlendSpace, Document, Edit};

use super::format::{HEADER_LEN, SLOT_LEN, SLOT_OFFSETS, Slot};
use super::*;
use crate::atomic::temp_files;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("slopshop-slop-{}-{name}", std::process::id()))
}

/// Deterministic noise (incompressible enough to exercise raw storage too).
fn noise(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    (0..len)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u8
        })
        .collect()
}

fn smooth(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i / 997) as u8).collect()
}

fn image(
    size: Size,
    layout: ChannelLayout,
    sample: SampleType,
    pixels: Vec<u8>,
) -> Arc<RasterImage> {
    let format = PixelFormat {
        layout,
        sample,
        color_space: ColorSpace::DISPLAY_P3,
        alpha: if layout.has_alpha() {
            AlphaMode::Premultiplied
        } else {
            AlphaMode::Straight
        },
    };
    assert_eq!(
        pixels.len(),
        size.pixel_count() as usize * format.bytes_per_pixel() as usize
    );
    Arc::new(RasterImage::from_pixels(size, format, &pixels).unwrap())
}

fn push(doc: &mut Document, name: &str, content: LayerContent, opacity: f32) -> u64 {
    let id = doc.allocate_layer_id();
    let index = doc.layers().len();
    let layer = Layer {
        clipped: false,
        id,
        name: name.to_owned(),
        visible: true,
        opacity,
        blend_mode: BlendMode::Normal,
        mask: None,
        content,
    };
    Edit::InsertLayer {
        parent: None,
        index,
        layer,
    }
    .apply(doc)
    .unwrap();
    id.get()
}

/// A document with every sample type, a shared image, fills and a hidden layer.
fn sample_document() -> Document {
    let size = Size::new(300, 270);
    let mut doc = Document::new(size);
    push(
        &mut doc,
        "fond",
        LayerContent::Fill {
            color: LinearRgba::new(0.1, 0.2, 0.3, 1.0),
        },
        1.0,
    );
    let pixels = |bpp: usize, seed| {
        let len = size.pixel_count() as usize * bpp;
        if seed % 2 == 0 {
            smooth(len)
        } else {
            noise(len, seed)
        }
    };
    let rgb8 = image(size, ChannelLayout::Rgb, SampleType::U8, pixels(3, 2));
    push(
        &mut doc,
        "photo",
        LayerContent::Raster {
            image: rgb8.clone(),
        },
        0.8,
    );
    push(
        &mut doc,
        "rgba16",
        LayerContent::Raster {
            image: image(size, ChannelLayout::Rgba, SampleType::U16, pixels(8, 3)),
        },
        1.0,
    );
    // Half floats from noise: includes NaN and infinity bit patterns, kept bit-exact.
    push(
        &mut doc,
        "gray f16",
        LayerContent::Raster {
            image: image(
                Size::new(40, 520),
                ChannelLayout::GrayAlpha,
                SampleType::F16,
                noise(40 * 520 * 4, 5),
            ),
        },
        0.5,
    );
    push(
        &mut doc,
        "rgba f32",
        LayerContent::Raster {
            image: image(
                Size::new(17, 9),
                ChannelLayout::Rgba,
                SampleType::F32,
                noise(17 * 9 * 16, 7),
            ),
        },
        1.0,
    );
    // The same image in a second layer: stored once, shared again after loading.
    let hidden = push(
        &mut doc,
        "photo (copie)",
        LayerContent::Raster { image: rgb8 },
        0.25,
    );
    Edit::SetLayerVisible {
        id: slopshop_core::LayerId::from_raw(hidden),
        visible: false,
    }
    .apply(&mut doc)
    .unwrap();
    push(
        &mut doc,
        "teinte HDR",
        LayerContent::Fill {
            color: LinearRgba::new(2.5, -0.125, 1e-7, 0.5),
        },
        0.3,
    );
    // A removed layer: its id is never reused.
    let removed = push(
        &mut doc,
        "supprimé",
        LayerContent::Fill {
            color: LinearRgba::new(0.0, 0.0, 0.0, 1.0),
        },
        1.0,
    );
    Edit::RemoveLayer {
        id: slopshop_core::LayerId::from_raw(removed),
    }
    .apply(&mut doc)
    .unwrap();
    doc
}

/// Same layers, ids, names, visibility, opacity, fills, sizes, formats and every tile byte.
fn assert_same(a: &Document, b: &Document) {
    assert_eq!(a.size(), b.size());
    assert_eq!(a.working_space(), b.working_space());
    assert_eq!(a.blend_space(), b.blend_space());
    assert_eq!(a.next_layer_id(), b.next_layer_id());
    assert_same_layers(a.layers(), b.layers());
}

fn assert_same_layers(a: &[Layer], b: &[Layer]) {
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b) {
        assert_eq!((x.id, &x.name, x.visible), (y.id, &y.name, y.visible));
        assert_eq!(x.clipped, y.clipped, "{}", x.name);
        assert_eq!(x.opacity.to_bits(), y.opacity.to_bits(), "{}", x.name);
        assert_eq!(x.blend_mode, y.blend_mode, "{}", x.name);
        match (&x.mask, &y.mask) {
            (None, None) => {}
            (Some(m), Some(n)) => {
                assert_eq!(
                    (m.enabled, m.replaces_alpha),
                    (n.enabled, n.replaces_alpha),
                    "{}",
                    x.name
                );
                assert_eq!(m.image.format(), n.image.format(), "{}", x.name);
                for (l, k) in m.image.levels().iter().zip(n.image.levels()) {
                    for (s, t) in l.tiles().iter().zip(k.tiles()) {
                        assert!(s[..] == t[..], "{}: mask tile differs", x.name);
                    }
                }
            }
            _ => panic!("{}: mask presence differs", x.name),
        }
        match (&x.content, &y.content) {
            (LayerContent::Fill { color: c }, LayerContent::Fill { color: d }) => {
                let bits = |c: &LinearRgba| [c.r, c.g, c.b, c.a].map(f32::to_bits);
                assert_eq!(bits(c), bits(d), "{}", x.name);
            }
            (LayerContent::Raster { image: i }, LayerContent::Raster { image: j }) => {
                assert_eq!(i.size(), j.size(), "{}", x.name);
                assert_eq!(i.format(), j.format(), "{}", x.name);
                assert_eq!(i.levels().len(), j.levels().len(), "{}", x.name);
                for (l, m) in i.levels().iter().zip(j.levels()) {
                    for (s, t) in l.tiles().iter().zip(m.tiles()) {
                        assert!(s[..] == t[..], "{}: tile differs", x.name);
                    }
                }
            }
            (
                LayerContent::Group {
                    children: c,
                    pass_through: p,
                },
                LayerContent::Group {
                    children: d,
                    pass_through: q,
                },
            ) => {
                assert_eq!(p, q, "{}", x.name);
                assert_same_layers(c, d);
            }
            _ => panic!("{}: content kind differs", x.name),
        }
    }
}

#[test]
fn documents_round_trip_bit_exact() {
    let path = temp_path("round-trip.slop");
    let doc = sample_document();
    let created = SlopFile::create(&path, &doc).unwrap();
    assert!(temp_files(&path).is_empty());
    assert_eq!(created.generation(), 1);
    assert_eq!(fs::metadata(&path).unwrap().len(), created.len());
    assert!(is_slop_file(&path).unwrap());

    let (loaded, file) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    assert_eq!(file.generation(), 1);
    // The shared image is one image again.
    let photos: Vec<&Arc<RasterImage>> = loaded
        .layers()
        .iter()
        .filter_map(|l| match &l.content {
            LayerContent::Raster { image } if l.name.starts_with("photo") => Some(image),
            _ => None,
        })
        .collect();
    assert_eq!(photos.len(), 2);
    assert!(Arc::ptr_eq(photos[0], photos[1]));
    fs::remove_file(&path).ok();
}

#[test]
fn saving_without_changes_appends_only_metadata() {
    let path = temp_path("metadata.slop");
    let doc = sample_document();
    let mut file = SlopFile::create(&path, &doc).unwrap();
    let first = file.len();
    let report = file.save(&doc).unwrap();
    assert!(!report.compacted);
    // Manifest, index and commit record: a few kilobytes, no pixels.
    assert!(report.bytes_written < 16 * 1024, "{report:?}");
    assert_eq!(file.len(), first + report.bytes_written);
    assert_eq!(file.generation(), 2);
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    fs::remove_file(&path).ok();
}

#[test]
fn a_new_layer_appends_only_its_tiles_and_the_session_carries_on() {
    let path = temp_path("new-layer.slop");
    let mut doc = sample_document();
    let (_, mut file) = {
        SlopFile::create(&path, &doc).unwrap();
        SlopFile::open(&path).unwrap()
    };
    let size = Size::new(512, 512);
    let added = image(
        size,
        ChannelLayout::Rgba,
        SampleType::U8,
        noise(512 * 512 * 4, 11),
    );
    push(
        &mut doc,
        "nouveau",
        LayerContent::Raster { image: added },
        1.0,
    );
    let before = file.len();
    let report = file.save(&doc).unwrap();
    // Four level-0 tiles and one pyramid tile of noise (stored raw), plus metadata.
    let tile = 256 * 256 * 4;
    assert!(report.bytes_written > 4 * tile as u64, "{report:?}");
    assert!(
        report.bytes_written < 6 * tile as u64 + 64 * 1024,
        "{report:?}"
    );
    assert_eq!(file.len(), before + report.bytes_written);
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    // Ids continue from the saved counter.
    let mut loaded = loaded;
    assert_eq!(loaded.allocate_layer_id().get(), doc.next_layer_id());
    fs::remove_file(&path).ok();
}

#[test]
fn removed_data_is_compacted_away_once_it_dominates() {
    let path = temp_path("compact.slop");
    let size = Size::new(600, 600);
    let mut doc = Document::new(size);
    let big = push(
        &mut doc,
        "gros",
        LayerContent::Raster {
            image: image(
                size,
                ChannelLayout::Rgba,
                SampleType::U8,
                noise(600 * 600 * 4, 13),
            ),
        },
        1.0,
    );
    push(
        &mut doc,
        "fond",
        LayerContent::Fill {
            color: LinearRgba::new(1.0, 1.0, 1.0, 1.0),
        },
        1.0,
    );
    let mut file = SlopFile::create(&path, &doc).unwrap();
    let full = file.len();
    Edit::RemoveLayer {
        id: slopshop_core::LayerId::from_raw(big),
    }
    .apply(&mut doc)
    .unwrap();
    // Most of the file is now dead: the save rewrites it compact.
    let report = file.save(&doc).unwrap();
    assert!(report.compacted, "{report:?}");
    assert!(file.len() < full / 10, "{} vs {full}", file.len());
    assert_eq!(file.dead_bytes(), 0);
    assert!(temp_files(&path).is_empty());
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    fs::remove_file(&path).ok();
}

/// Two generations of one document, and the bytes of the file after each.
fn two_generations(name: &str) -> (PathBuf, Document, Document, Vec<u8>, Vec<u8>) {
    let path = temp_path(name);
    let first = sample_document();
    let mut file = SlopFile::create(&path, &first).unwrap();
    let gen1 = fs::read(&path).unwrap();
    let mut second = first.clone();
    push(
        &mut second,
        "ajout",
        LayerContent::Raster {
            image: image(
                Size::new(300, 10),
                ChannelLayout::Rgb,
                SampleType::U8,
                noise(300 * 10 * 3, 17),
            ),
        },
        1.0,
    );
    file.save(&second).unwrap();
    let gen2 = fs::read(&path).unwrap();
    (path, first, second, gen1, gen2)
}

#[test]
fn a_crash_during_a_save_leaves_the_previous_generation_readable() {
    let (path, first, _second, gen1, gen2) = two_generations("crash.slop");
    assert!(gen2.len() > gen1.len());
    // The new records are appended and synced before the slot is written: a crash leaves the
    // old header with any prefix of them. Every cut must open as generation 1.
    let header = &gen1[..HEADER_LEN as usize];
    let step = ((gen2.len() - gen1.len()) / 40).max(1);
    let mut cut = gen1.len();
    while cut <= gen2.len() {
        let mut crashed = header.to_vec();
        crashed.extend_from_slice(&gen2[HEADER_LEN as usize..cut]);
        fs::write(&path, &crashed).unwrap();
        let (loaded, file) = SlopFile::open(&path).unwrap_or_else(|e| panic!("cut {cut}: {e}"));
        assert_eq!(file.generation(), 1, "cut {cut}");
        assert_same(&first, &loaded);
        cut += step;
    }
    fs::remove_file(&path).ok();
}

#[test]
fn junk_after_the_committed_length_is_removed_by_the_next_save() {
    let (path, _first, second, _gen1, gen2) = two_generations("junk.slop");
    let mut junk = gen2.clone();
    junk.extend_from_slice(&noise(5000, 3));
    fs::write(&path, &junk).unwrap();
    let (loaded, mut file) = SlopFile::open(&path).unwrap();
    assert_same(&second, &loaded);
    file.save(&loaded).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), file.len());
    let (again, _) = SlopFile::open(&path).unwrap();
    assert_same(&second, &again);
    fs::remove_file(&path).ok();
}

#[test]
fn a_torn_slot_falls_back_to_the_other_one() {
    let (path, first, _second, _gen1, mut gen2) = two_generations("torn.slop");
    // Generation 2 is in slot 0 (2 % 2); damage it.
    let at = SLOT_OFFSETS[0] as usize + 17;
    gen2[at] ^= 0xFF;
    fs::write(&path, &gen2).unwrap();
    let (loaded, file) = SlopFile::open(&path).unwrap();
    assert_eq!(file.generation(), 1);
    assert_same(&first, &loaded);
    fs::remove_file(&path).ok();
}

#[test]
fn commit_records_recover_the_newest_generation_when_both_slots_are_lost() {
    let (path, _first, second, _gen1, mut gen2) = two_generations("recover.slop");
    for offset in SLOT_OFFSETS {
        let at = offset as usize;
        gen2[at..at + SLOT_LEN].fill(0xAB);
    }
    fs::write(&path, &gen2).unwrap();
    let (loaded, file) = SlopFile::open(&path).unwrap();
    assert_eq!(file.generation(), 2);
    assert_same(&second, &loaded);
    fs::remove_file(&path).ok();
}

#[test]
fn a_concurrent_save_is_a_conflict_not_a_silent_overwrite() {
    let path = temp_path("conflict.slop");
    let doc = sample_document();
    SlopFile::create(&path, &doc).unwrap();
    let (_, mut a) = SlopFile::open(&path).unwrap();
    let (_, mut b) = SlopFile::open(&path).unwrap();
    a.save(&doc).unwrap();
    let error = b.save(&doc).unwrap_err();
    assert_eq!(error.code(), "conflict");
    // A's work is intact.
    let (_, file) = SlopFile::open(&path).unwrap();
    assert_eq!(file.generation(), 2);
    fs::remove_file(&path).ok();
}

#[test]
fn damaged_files_are_errors_never_panics() {
    let path = temp_path("damaged.slop");
    let mut doc = Document::new(Size::new(40, 30));
    push(
        &mut doc,
        "image",
        LayerContent::Raster {
            image: image(
                Size::new(40, 30),
                ChannelLayout::Rgb,
                SampleType::U8,
                noise(40 * 30 * 3, 19),
            ),
        },
        1.0,
    );
    SlopFile::create(&path, &doc).unwrap();
    let good = fs::read(&path).unwrap();
    // Flip one byte at a time across the records (and a few in the header).
    let positions: Vec<usize> = (0..40)
        .chain((HEADER_LEN as usize..good.len()).step_by(7))
        .collect();
    let mut errors = 0;
    for at in positions {
        let mut bad = good.clone();
        bad[at] ^= 0x5A;
        fs::write(&path, &bad).unwrap();
        match SlopFile::open(&path) {
            // A flipped padding byte or unused slot byte changes nothing.
            Ok((loaded, _)) => assert_same(&doc, &loaded),
            Err(_) => errors += 1,
        }
    }
    assert!(errors > 0);
    // Truncations too.
    for len in [
        0,
        7,
        100,
        HEADER_LEN as usize,
        good.len() / 2,
        good.len() - 1,
    ] {
        fs::write(&path, &good[..len]).unwrap();
        assert!(SlopFile::open(&path).is_err(), "truncated to {len}");
    }
    fs::remove_file(&path).ok();
}

#[test]
fn files_from_newer_versions_are_refused_with_their_own_error() {
    let path = temp_path("newer.slop");
    let doc = sample_document();
    SlopFile::create(&path, &doc).unwrap();
    let mut bytes = fs::read(&path).unwrap();
    bytes[8] = 1; // major version 1
    fs::write(&path, &bytes).unwrap();
    assert_eq!(SlopFile::open(&path).unwrap_err().code(), "newerVersion");
    fs::write(
        &path,
        b"GIF89a, not a document at all, longer than the magic",
    )
    .unwrap();
    assert_eq!(SlopFile::open(&path).unwrap_err().code(), "notASlopFile");
    assert!(!is_slop_file(&path).unwrap());
    fs::remove_file(&path).ok();
}

#[test]
fn slots_alternate_between_generations() {
    let (path, _first, _second, gen1, gen2) = two_generations("slots.slop");
    let slot = |bytes: &[u8], i: usize| {
        let at = SLOT_OFFSETS[i] as usize;
        Slot::decode(&bytes[at..at + SLOT_LEN]).map(|s| s.generation)
    };
    assert_eq!((slot(&gen1, 0), slot(&gen1, 1)), (None, Some(1)));
    assert_eq!((slot(&gen2, 0), slot(&gen2, 1)), (Some(2), Some(1)));
    fs::remove_file(&path).ok();
}

#[test]
fn save_as_writes_a_compact_copy_and_continues_with_it() {
    let (path, _first, second, _gen1, _gen2) = two_generations("save-as.slop");
    let (loaded, mut file) = SlopFile::open(&path).unwrap();
    let copy = temp_path("save-as-copy.slop");
    file.save_as(&copy, &loaded).unwrap();
    assert_eq!(file.path(), copy.as_path());
    assert_eq!(file.generation(), 1);
    assert!(file.len() <= fs::metadata(&path).unwrap().len());
    let (again, _) = SlopFile::open(&copy).unwrap();
    assert_same(&second, &again);
    fs::remove_file(&path).ok();
    fs::remove_file(&copy).ok();
}

/// The document of the golden fixture of schema 0.5: the schema 0.4 one, its hidden top layer
/// clipped to the group below it.
fn golden_document() -> Document {
    let mut doc = golden_document_v0_4();
    let top = doc.layers().last().unwrap().id;
    Edit::SetLayerClipped {
        id: top,
        clipped: true,
    }
    .apply(&mut doc)
    .unwrap();
    doc
}

/// The document of the golden fixture of schema 0.4: the schema 0.3 one, with its "Tint" layer
/// inside an isolated screen group at 80 %, itself inside a masked pass-through group.
fn golden_document_v0_4() -> Document {
    let mut doc = golden_document_v0_3();
    let tint = doc.layers()[2].id;
    let masked_by = match &doc.layers()[1].content {
        LayerContent::Raster { image } => slopshop_core::LayerMask::from_transparency(image),
        _ => None,
    };
    let folder = doc.allocate_layer_id();
    let inner = doc.allocate_layer_id();
    let group = |id, name: &str, pass_through, mode, opacity, mask| Layer {
        clipped: false,
        id,
        name: name.to_owned(),
        visible: true,
        opacity,
        blend_mode: mode,
        mask,
        content: LayerContent::Group {
            children: Vec::new(),
            pass_through,
        },
    };
    Edit::Batch(vec![
        Edit::InsertLayer {
            parent: None,
            index: 3,
            layer: group(folder, "Folder", true, BlendMode::Normal, 1.0, masked_by),
        },
        Edit::InsertLayer {
            parent: Some(folder),
            index: 0,
            layer: group(inner, "Inner", false, BlendMode::Screen, 0.8, None),
        },
        Edit::MoveLayer {
            id: tint,
            parent: Some(inner),
            index: 0,
        },
    ])
    .apply(&mut doc)
    .unwrap();
    doc
}

/// The document of the golden fixture of schema 0.3 (0.1 and 0.2 without what came later):
/// every node type, an 8-bit and a half-float image, one of them shared by two layers, a hidden
/// layer, partial opacity, blend modes and a mask.
fn golden_document_v0_3() -> Document {
    let size = Size::new(40, 20);
    let mut doc = Document::new(size);
    let gradient: Vec<u8> = (0..size.pixel_count())
        .flat_map(|i| [(i % 40 * 6) as u8, (i / 40 * 12) as u8, 200])
        .collect();
    let gradient = image(size, ChannelLayout::Rgb, SampleType::U8, gradient);
    let ramp: Vec<u8> = (0..size.pixel_count())
        .flat_map(|i| {
            let gray = half::f16::from_f32(i as f32 / 800.0).to_le_bytes();
            let alpha = half::f16::from_f32(1.0).to_le_bytes();
            [gray, alpha].concat()
        })
        .collect();
    let ramp = image(size, ChannelLayout::GrayAlpha, SampleType::F16, ramp);
    let raster = |image: &Arc<RasterImage>| LayerContent::Raster {
        image: image.clone(),
    };
    push(&mut doc, "Gradient", raster(&gradient), 1.0);
    push(&mut doc, "Ramp", raster(&ramp), 0.75);
    let color = LinearRgba::new(0.25, 0.5, 1.0, 0.5);
    push(&mut doc, "Tint", LayerContent::Fill { color }, 0.5);
    let hidden = push(&mut doc, "Gradient again", raster(&gradient), 1.0);
    // A mask from transparency (schema 0.3) on the half-float gray + alpha layer, disabled.
    let ramp_layer = doc.layers()[1].clone();
    if let LayerContent::Raster { image } = &ramp_layer.content {
        let mut mask = slopshop_core::LayerMask::from_transparency(image).unwrap();
        mask.enabled = false;
        Edit::SetLayerMask {
            id: ramp_layer.id,
            mask: Some(mask),
        }
        .apply(&mut doc)
        .unwrap();
    }
    // Blend modes (schema 0.2): by id, in stack order.
    let ids: Vec<_> = doc.layers().iter().map(|l| l.id).collect();
    for (id, mode) in ids.into_iter().zip([
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::SoftLight,
        BlendMode::Luminosity,
    ]) {
        Edit::SetLayerBlendMode { id, mode }
            .apply(&mut doc)
            .unwrap();
    }
    let id = doc
        .layers()
        .iter()
        .find(|l| l.id.get() == hidden)
        .unwrap()
        .id;
    Edit::SetLayerVisible { id, visible: false }
        .apply(&mut doc)
        .unwrap();
    doc
}

/// What the schema 0.2 fixture holds: no masks then.
fn golden_document_v0_2() -> Document {
    let mut doc = golden_document_v0_3();
    for layer in doc.layers().to_vec() {
        Edit::SetLayerMask {
            id: layer.id,
            mask: None,
        }
        .apply(&mut doc)
        .unwrap();
    }
    doc
}

/// What the schema 0.1 fixture holds: no blend modes either, and linear compositing.
fn golden_document_v0_1() -> Document {
    let mut doc = golden_document_v0_2();
    for layer in doc.layers().to_vec() {
        Edit::SetLayerBlendMode {
            id: layer.id,
            mode: BlendMode::Normal,
        }
        .apply(&mut doc)
        .unwrap();
    }
    Edit::SetBlendSpace {
        space: BlendSpace::Linear,
    }
    .apply(&mut doc)
    .unwrap();
    doc
}

fn golden_path(version: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/slop/fixtures")
        .join(format!("v{version}.slop"))
}

#[test]
fn golden_fixtures_still_open_identically() {
    let (loaded, file) = SlopFile::open(&golden_path("0.1")).unwrap();
    assert_same(&golden_document_v0_1(), &loaded);
    assert_eq!(file.generation(), 1);
    let (loaded, _) = SlopFile::open(&golden_path("0.2")).unwrap();
    assert_same(&golden_document_v0_2(), &loaded);
    let (loaded, _) = SlopFile::open(&golden_path("0.3")).unwrap();
    assert_same(&golden_document_v0_3(), &loaded);
    let (loaded, _) = SlopFile::open(&golden_path("0.4")).unwrap();
    assert_same(&golden_document_v0_4(), &loaded);
    let (loaded, _) = SlopFile::open(&golden_path("0.5")).unwrap();
    assert_same(&golden_document(), &loaded);
}

/// Writes the fixture of the current schema version. Run once when the schema changes, and
/// commit the file: fixtures of older versions are never rewritten.
#[test]
#[ignore = "writes a golden fixture into the source tree"]
fn write_golden_fixture() {
    let path = golden_path(&format!(
        "{}.{}",
        manifest::SCHEMA_MAJOR,
        manifest::SCHEMA_MINOR
    ));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    SlopFile::create(&path, &golden_document()).unwrap();
}

#[test]
fn blend_modes_and_space_round_trip_and_unknown_ones_are_refused() {
    let path = temp_path("blend.slop");
    let mut doc = sample_document();
    let ids: Vec<_> = doc.layers().iter().map(|l| l.id).collect();
    for (id, mode) in ids.iter().zip(BlendMode::ALL.iter().cycle().skip(3)) {
        Edit::SetLayerBlendMode {
            id: *id,
            mode: *mode,
        }
        .apply(&mut doc)
        .unwrap();
    }
    for space in [BlendSpace::Linear, BlendSpace::Perceptual] {
        Edit::SetBlendSpace { space }.apply(&mut doc).unwrap();
        SlopFile::create(&path, &doc).unwrap();
        let (loaded, _) = SlopFile::open(&path).unwrap();
        assert_same(&doc, &loaded);
    }

    // Older nodes have no mode; a mode or a space from a newer SlopShop is "newer version",
    // not corruption.
    let node = |version: u32, params: &str| {
        let json = format!(
            r#"{{"type":"slopshop.fill","version":{version},"name":"n","visible":true,"opacity":1.0,"params":{params},"inputs":[]}}"#
        );
        read::node_blend_mode(&serde_json::from_str(&json).unwrap()).map_err(|e| e.code())
    };
    assert_eq!(node(1, "{}"), Ok(BlendMode::Normal));
    assert_eq!(node(2, r#"{"blend_mode":"screen"}"#), Ok(BlendMode::Screen));
    assert_eq!(node(2, r#"{"blend_mode":"pinkify"}"#), Err("newerVersion"));
    assert_eq!(node(2, "{}"), Err("corrupt"));
    let space = |extra: &str| {
        let json = format!(
            r#"{{"size":[1,1],"working_space":{{"primaries":{{"r":[0.708,0.292],"g":[0.17,0.797],"b":[0.131,0.046],"w":[0.3127,0.329]}},"transfer":{{"kind":"linear"}}}},"next_node_id":1,"stack":[]{extra}}}"#
        );
        read::document_blend_space(&serde_json::from_str(&json).unwrap()).map_err(|e| e.code())
    };
    assert_eq!(space(""), Ok(BlendSpace::Linear));
    assert_eq!(
        space(r#","blend_space":"perceptual""#),
        Ok(BlendSpace::Perceptual)
    );
    assert_eq!(space(r#","blend_space":"cmyk""#), Err("newerVersion"));
    fs::remove_file(&path).ok();
}

#[test]
fn masks_round_trip_and_share_their_tiles() {
    let path = temp_path("masks.slop");
    let mut doc = sample_document();
    for layer in doc.layers().to_vec() {
        if let LayerContent::Raster { image } = &layer.content
            && let Some(mask) = slopshop_core::LayerMask::from_transparency(image)
        {
            Edit::SetLayerMask {
                id: layer.id,
                mask: Some(mask),
            }
            .apply(&mut doc)
            .unwrap();
        }
    }
    assert!(doc.layers().iter().any(|l| l.mask.is_some()));
    SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    fs::remove_file(&path).ok();
}

#[test]
fn groups_round_trip_through_saves() {
    let path = temp_path("groups.slop");
    let doc = golden_document();
    let mut file = SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    // Moving the grouped layer out and saving again (an incremental save) keeps the tree.
    let mut session = slopshop_core::Session::new(loaded);
    let tint = session
        .document()
        .all_layers()
        .find(|l| l.name == "Tint")
        .unwrap()
        .id;
    session
        .perform(Edit::MoveLayer {
            id: tint,
            parent: None,
            index: 0,
        })
        .unwrap();
    file.save(session.document()).unwrap();
    let (again, _) = SlopFile::open(&path).unwrap();
    assert_same(session.document(), &again);
    fs::remove_file(&path).ok();
}

#[test]
fn damaged_layer_trees_are_refused() {
    // A manifest with these nodes and this stack.
    let read = |nodes: &str, stack: &str| {
        let json = format!(
            r#"{{"schema":{{"major":0,"minor":4}},"writer":{{"app":"test","version":"0"}},
            "document":{{"size":[1,1],"working_space":{{"primaries":{{"r":[0.708,0.292],"g":[0.17,0.797],"b":[0.131,0.046],"w":[0.3127,0.329]}},"transfer":{{"kind":"linear"}}}},"next_node_id":100,"stack":{stack}}},
            "nodes":{{{nodes}}},"images":{{}}}}"#
        );
        let manifest: manifest::Manifest = serde_json::from_str(&json).unwrap();
        let mut used = std::collections::HashSet::new();
        let mut residue = Residue::default();
        let stack: Vec<u64> = serde_json::from_str(stack).unwrap();
        stack
            .iter()
            .map(|&id| {
                read::read_node(
                    id,
                    0,
                    &manifest,
                    &std::collections::HashMap::new(),
                    &mut residue,
                    &mut used,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.code())
    };
    let group = |id: u64, inputs: &str| {
        format!(
            r#""{id}":{{"type":"slopshop.group","version":3,"name":"g","visible":true,"opacity":1.0,"params":{{"blend_mode":"normal","pass_through":true}},"inputs":{inputs}}}"#
        )
    };
    let fill = |id: u64| {
        format!(
            r#""{id}":{{"type":"slopshop.fill","version":3,"name":"f","visible":true,"opacity":1.0,"params":{{"blend_mode":"normal","color":[0,0,0,1]}},"inputs":[]}}"#
        )
    };
    let valid = read(&[group(1, "[2]"), fill(2)].join(","), "[1]").unwrap();
    assert_eq!(valid[0].children().unwrap()[0].id.get(), 2);
    assert_eq!(read(&group(1, "[1]"), "[1]"), Err("corrupt"), "a cycle");
    assert_eq!(
        read(
            &[group(1, "[3]"), group(2, "[3]"), fill(3)].join(","),
            "[1,2]"
        ),
        Err("corrupt"),
        "a shared child"
    );
    assert_eq!(
        read(&group(1, "[9]"), "[1]"),
        Err("corrupt"),
        "a missing child"
    );
    let no_flag = r#""1":{"type":"slopshop.group","version":3,"name":"g","visible":true,"opacity":1.0,"params":{"blend_mode":"normal"},"inputs":[]}"#;
    assert_eq!(read(no_flag, "[1]"), Err("corrupt"));
    // Deeper than the engine allows: refused before recursing further.
    let depth = slopshop_core::document::MAX_GROUP_DEPTH as u64 + 1;
    let chain: Vec<String> = (1..=depth)
        .map(|id| {
            if id == depth {
                group(id, "[]")
            } else {
                group(id, &format!("[{}]", id + 1))
            }
        })
        .collect();
    assert_eq!(read(&chain.join(","), "[1]"), Err("corrupt"));
}
