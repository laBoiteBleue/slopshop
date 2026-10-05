use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use slopshop_core::color::{
    AlphaMode, ChannelLayout, ColorSpace, LinearRgba, PixelFormat, SampleType,
};
use slopshop_core::document::{Layer, LayerContent};
use slopshop_core::geom::Size;
use slopshop_core::raster::RasterImage;
use slopshop_core::selection::Selection;
use slopshop_core::stack::{Effect, Entry, LayerStack, PaintEntry, PaintOp};
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
        style: None,
        transform: slopshop_core::Affine::IDENTITY.into(),
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
    // Not the default: the resolution must round-trip (ADR 0028).
    Edit::SetResolution { ppi: 240.0 }.apply(&mut doc).unwrap();
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
            stack: None,
            image: slopshop_core::stack::Pixels::ready(rgb8.clone()),
        },
        0.8,
    );
    push(
        &mut doc,
        "rgba16",
        LayerContent::Raster {
            stack: None,
            image: slopshop_core::stack::Pixels::ready(image(
                size,
                ChannelLayout::Rgba,
                SampleType::U16,
                pixels(8, 3),
            )),
        },
        1.0,
    );
    // Half floats from noise: includes NaN and infinity bit patterns, kept bit-exact.
    push(
        &mut doc,
        "gray f16",
        LayerContent::Raster {
            stack: None,
            image: slopshop_core::stack::Pixels::ready(image(
                Size::new(40, 520),
                ChannelLayout::GrayAlpha,
                SampleType::F16,
                noise(40 * 520 * 4, 5),
            )),
        },
        0.5,
    );
    push(
        &mut doc,
        "rgba f32",
        LayerContent::Raster {
            stack: None,
            image: slopshop_core::stack::Pixels::ready(image(
                Size::new(17, 9),
                ChannelLayout::Rgba,
                SampleType::F32,
                noise(17 * 9 * 16, 7),
            )),
        },
        1.0,
    );
    // The same image in a second layer: stored once, shared again after loading.
    let hidden = push(
        &mut doc,
        "photo (copie)",
        LayerContent::Raster {
            image: slopshop_core::stack::Pixels::ready(rgb8),
            stack: None,
        },
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
    assert_eq!(a.resolution(), b.resolution());
    assert_eq!(a.next_layer_id(), b.next_layer_id());
    assert_same_layers(a.layers(), b.layers());
    assert_eq!(a.next_saved_selection_id(), b.next_saved_selection_id());
    assert_eq!(a.saved_selections().len(), b.saved_selections().len());
    for (s, t) in a.saved_selections().iter().zip(b.saved_selections()) {
        assert_eq!((s.id, &s.name), (t.id, &t.name));
        assert_same_image(s.selection.image(), t.selection.image(), &s.name);
    }
}

/// Same size, format and tile bytes at every level.
fn assert_same_image(a: &RasterImage, b: &RasterImage, what: &str) {
    assert_eq!(a.size(), b.size(), "{what}");
    assert_eq!(a.format(), b.format(), "{what}");
    assert_eq!(a.levels().len(), b.levels().len(), "{what}");
    for (l, m) in a.levels().iter().zip(b.levels()) {
        for (s, t) in l.tiles().iter().zip(m.tiles()) {
            assert!(s[..] == t[..], "{what}: tile differs");
        }
    }
}

/// Both `None`, or the same image.
fn assert_same_original(a: Option<&Arc<RasterImage>>, b: Option<&Arc<RasterImage>>, what: &str) {
    match (a, b) {
        (None, None) => {}
        (Some(a), Some(b)) => assert_same_image(a, b, what),
        _ => panic!("{what}: paint differs"),
    }
}

/// Both `None`, or the same original and entries (ADR 0029).
fn assert_same_stack(a: Option<&LayerStack>, b: Option<&LayerStack>, what: &str) {
    let (a, b) = match (a, b) {
        (None, None) => return,
        (Some(a), Some(b)) => (a, b),
        _ => panic!("{what}: stack presence differs"),
    };
    assert_same_image(a.original(), b.original(), &format!("{what} original"));
    assert_eq!(a.entries().len(), b.entries().len(), "{what}");
    for (e, f) in a.entries().iter().zip(b.entries()) {
        assert_eq!(e.hidden(), f.hidden(), "{what}: an entry's eye");
        match (e, f) {
            (Entry::Paint(p), Entry::Paint(q)) => {
                assert_eq!(p.space(), q.space(), "{what}");
                let ((pc, pk), (qc, qk)) = (p.images().unwrap(), q.images().unwrap());
                assert_same_image(&pc, &qc, &format!("{what} paint"));
                assert_same_image(&pk, &qk, &format!("{what} paint"));
            }
            (Entry::Effect(p), Entry::Effect(q)) => {
                assert_eq!(p.steps().len(), q.steps().len(), "{what}");
                for (s, t) in p.steps().iter().zip(q.steps()) {
                    assert_eq!(s.adjustment, t.adjustment, "{what}");
                    assert_eq!(s.to_document, t.to_document, "{what}");
                    assert_eq!(s.space, t.space, "{what}");
                    match (&s.selection, &t.selection) {
                        (None, None) => {}
                        (Some(s), Some(t)) => {
                            assert_same_image(s.image(), t.image(), &format!("{what} selection"))
                        }
                        _ => panic!("{what}: an effect's selection differs"),
                    }
                }
            }
            (Entry::Filter(p), Entry::Filter(q)) => {
                assert_eq!(p.steps().len(), q.steps().len(), "{what}");
                for (s, t) in p.steps().iter().zip(q.steps()) {
                    assert_eq!(s.filter, t.filter, "{what}");
                    assert_eq!(s.to_document, t.to_document, "{what}");
                    assert_eq!(s.space, t.space, "{what}");
                    assert_eq!(s.selection.is_some(), t.selection.is_some(), "{what}");
                }
            }
            (Entry::Liquify(p), Entry::Liquify(q)) => {
                assert_eq!(p.space(), q.space(), "{what}");
                assert_eq!(p.field().cell(), q.field().cell(), "{what}");
                let ((pd, pf), (qd, qf)) = (p.images().unwrap(), q.images().unwrap());
                assert_same_image(&pd, &qd, &format!("{what} liquify field"));
                assert_same_image(&pf, &qf, &format!("{what} liquify freeze"));
            }
            _ => panic!("{what}: entries differ"),
        }
    }
}

fn assert_same_layers(a: &[Layer], b: &[Layer]) {
    assert_eq!(a.len(), b.len());
    for (x, y) in a.iter().zip(b) {
        assert_eq!((x.id, &x.name, x.visible), (y.id, &y.name, y.visible));
        assert_eq!(x.clipped, y.clipped, "{}", x.name);
        assert_eq!(x.transform, y.transform, "{}", x.name);
        assert_eq!(x.opacity.to_bits(), y.opacity.to_bits(), "{}", x.name);
        assert_eq!(x.blend_mode, y.blend_mode, "{}", x.name);
        assert_eq!(x.style, y.style, "{}", x.name);
        match (&x.mask, &y.mask) {
            (None, None) => {}
            (Some(m), Some(n)) => {
                assert_eq!(
                    (m.enabled, m.replaces_alpha),
                    (n.enabled, n.replaces_alpha),
                    "{}",
                    x.name
                );
                assert_same_image(&m.image, &n.image, &format!("{} mask", x.name));
                let what = format!("{} mask original", x.name);
                assert_same_original(m.original.as_ref(), n.original.as_ref(), &what);
            }
            _ => panic!("{}: mask presence differs", x.name),
        }
        match (&x.content, &y.content) {
            (LayerContent::Fill { color: c }, LayerContent::Fill { color: d }) => {
                let bits = |c: &LinearRgba| [c.r, c.g, c.b, c.a].map(f32::to_bits);
                assert_eq!(bits(c), bits(d), "{}", x.name);
            }
            (
                LayerContent::Raster { image: i, stack: s },
                LayerContent::Raster { image: j, stack: t },
            ) => {
                assert_same_image(&i.get(), &j.get(), &x.name);
                assert_same_stack(s.as_ref(), t.as_ref(), &x.name);
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
            (
                LayerContent::Adjustment { adjustment: a },
                LayerContent::Adjustment { adjustment: b },
            ) => {
                assert_eq!(a.id(), b.id(), "{}", x.name);
                let bits = |a: &slopshop_core::adjust::Adjustment| a.params().map(f32::to_bits);
                assert_eq!(bits(a), bits(b), "{}", x.name);
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
    let photos: Vec<Arc<RasterImage>> = loaded
        .layers()
        .iter()
        .filter_map(|l| match &l.content {
            LayerContent::Raster { image, .. } if l.name.starts_with("photo") => Some(image.get()),
            _ => None,
        })
        .collect();
    assert_eq!(photos.len(), 2);
    assert!(Arc::ptr_eq(&photos[0], &photos[1]));
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
fn an_image_sharing_tiles_with_a_saved_one_reuses_their_hashes() {
    // A painted image (ADR 0027) shares the tiles it did not change with its original: saving
    // it writes the changed tiles only, and reads back the same.
    let path = temp_path("shared-tiles.slop");
    let mut doc = sample_document();
    let size = Size::new(512, 512);
    let original = image(
        size,
        ChannelLayout::Rgba,
        SampleType::U8,
        noise(512 * 512 * 4, 21),
    );
    push(
        &mut doc,
        "original",
        LayerContent::Raster {
            stack: None,
            image: slopshop_core::stack::Pixels::ready(Arc::clone(&original)),
        },
        1.0,
    );
    let mut file = SlopFile::create(&path, &doc).unwrap();
    let mut tiles = original.levels()[0].tiles().to_vec();
    tiles[3] = noise(tiles[3].len(), 22).into();
    let shared = RasterImage::from_level0_tiles(size, original.format(), tiles).unwrap();
    push(
        &mut doc,
        "shared",
        LayerContent::Raster {
            stack: None,
            image: slopshop_core::stack::Pixels::ready(Arc::new(shared)),
        },
        1.0,
    );
    let report = file.save(&doc).unwrap();
    // One new level-0 tile, one new pyramid tile, metadata.
    let tile = 256 * 256 * 4;
    assert!(
        report.bytes_written < 2 * tile as u64 + 64 * 1024,
        "{report:?}"
    );
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
        LayerContent::Raster {
            image: slopshop_core::stack::Pixels::ready(added),
            stack: None,
        },
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
            stack: None,
            image: slopshop_core::stack::Pixels::ready(image(
                size,
                ChannelLayout::Rgba,
                SampleType::U8,
                noise(600 * 600 * 4, 13),
            )),
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
            stack: None,
            image: slopshop_core::stack::Pixels::ready(image(
                Size::new(300, 10),
                ChannelLayout::Rgb,
                SampleType::U8,
                noise(300 * 10 * 3, 17),
            )),
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
            stack: None,
            image: slopshop_core::stack::Pixels::ready(image(
                Size::new(40, 30),
                ChannelLayout::Rgb,
                SampleType::U8,
                noise(40 * 30 * 3, 19),
            )),
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

/// The document of the golden fixture of schema 0.12: the schema 0.10 one with an Invert
/// within a selection then paint over it on its first raster layer (ADR 0029).
fn golden_document() -> Document {
    let mut doc = golden_document_v0_10();
    let (id, stack) = doc
        .all_layers()
        .find_map(|l| match &l.content {
            LayerContent::Raster { .. } => Some((l.id, l.content.stack()?)),
            _ => None,
        })
        .expect("a raster layer");
    let size = doc.size();
    // A selection of a rectangle, its edge soft.
    let coverage: Vec<u8> = (0..size.height)
        .flat_map(|y| {
            (0..size.width).flat_map(move |x| {
                let v: u16 = match (x, y) {
                    (2..9, 1..6) => u16::MAX,
                    (9, 1..6) => 30000,
                    _ => 0,
                };
                v.to_ne_bytes()
            })
        })
        .collect();
    let selection =
        RasterImage::from_pixels(size, slopshop_core::selection::SELECTION_FORMAT, &coverage)
            .unwrap();
    let inverted = stack
        .with_effect(Effect {
            adjustment: slopshop_core::adjust::Adjustment::Invert,
            selection: Selection::new(Arc::new(selection)),
            to_document: slopshop_core::Affine::IDENTITY.into(),
            space: doc.blend_space(),
        })
        .unwrap();
    let empty = PaintEntry::empty(
        inverted.original().format(),
        inverted.original().size(),
        doc.blend_space(),
    );
    let coord = slopshop_core::tile::TileCoord { col: 0, row: 0 };
    let tile = empty.painted_tile(
        coord,
        PaintOp::Color(LinearRgba::new(0.2, 0.5, 0.1, 1.0)),
        |x, y| ((x + y) % 5) as f32 / 4.0,
    );
    let paint = empty.with_tiles(vec![(coord, tile)]).unwrap();
    Edit::SetLayerStack {
        id,
        stack: inverted.with_top_paint(Arc::new(paint)).unwrap(),
        shown: None,
    }
    .apply(&mut doc)
    .unwrap();
    doc
}

/// The document of the golden fixture of schema 0.10: the schema 0.9 one with paint on the
/// first raster layer and on the first mask (ADR 0027), the layer's paint read as a stack
/// (ADR 0029).
fn golden_document_v0_10() -> Document {
    let mut doc = golden_document_v0_9();
    let raster = doc
        .all_layers()
        .find_map(|l| match &l.content {
            LayerContent::Raster { image, .. } => Some((l.id, image.get())),
            _ => None,
        })
        .expect("a raster layer");
    let masked = doc
        .all_layers()
        .find_map(|l| l.mask.as_ref().map(|m| (l.id, Arc::clone(&m.image))))
        .expect("a mask");
    // Paint: the original with its first tile replaced by the last one.
    let painted = |image: &RasterImage| {
        let mut tiles = image.levels()[0].tiles().to_vec();
        tiles[0] = Arc::clone(tiles.last().unwrap());
        Arc::new(RasterImage::from_level0_tiles(image.size(), image.format(), tiles).unwrap())
    };
    let paint =
        PaintEntry::from_painted(&raster.1, &painted(&raster.1), doc.blend_space()).unwrap();
    Edit::SetLayerStack {
        id: raster.0,
        stack: LayerStack::new(Arc::clone(&raster.1))
            .with_top_paint(Arc::new(paint))
            .unwrap(),
        shown: None,
    }
    .apply(&mut doc)
    .unwrap();
    Edit::SetMaskPaint {
        id: masked.0,
        painted: Some(painted(&masked.1)),
    }
    .apply(&mut doc)
    .unwrap();
    doc
}

/// The document of the golden fixture of schema 0.9: the schema 0.8 one with Curves on top.
fn golden_document_v0_9() -> Document {
    use slopshop_core::curve::Curve;
    let mut doc = golden_document_v0_8();
    let id = doc.allocate_layer_id();
    let index = doc.layers().len();
    Edit::InsertLayer {
        parent: None,
        index,
        layer: Layer {
            style: None,
            id,
            name: "Curves".into(),
            visible: true,
            opacity: 0.8,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            transform: slopshop_core::Affine::IDENTITY.into(),
            content: LayerContent::Adjustment {
                adjustment: slopshop_core::adjust::Adjustment::Curves {
                    rgb: Curve::new(&[[0, 10], [100, 140], [255, 240]]).unwrap(),
                    red: Curve::new(&[[0, 0], [128, 150], [255, 255]]).unwrap(),
                    green: Curve::IDENTITY,
                    blue: Curve::new(&[[20, 0], [255, 255]]).unwrap(),
                },
            },
        },
    }
    .apply(&mut doc)
    .unwrap();
    doc
}

/// The document of the golden fixture of schema 0.8: the schema 0.7 one with a Channel Mixer
/// on top, an adjustment of more than five values.
fn golden_document_v0_8() -> Document {
    let mut doc = golden_document_v0_7();
    let id = doc.allocate_layer_id();
    let index = doc.layers().len();
    Edit::InsertLayer {
        parent: None,
        index,
        layer: Layer {
            style: None,
            id,
            name: "Channel Mixer".into(),
            visible: true,
            opacity: 0.5,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            transform: slopshop_core::Affine::IDENTITY.into(),
            content: LayerContent::Adjustment {
                adjustment: slopshop_core::adjust::Adjustment::ChannelMixer {
                    red: [80.0, 30.0, -10.0, 0.0],
                    green: [0.0, 100.0, 0.0, 5.5],
                    blue: [10.0, 0.0, 90.0, 0.0],
                    monochrome: false,
                },
            },
        },
    }
    .apply(&mut doc)
    .unwrap();
    doc
}

/// The document of the golden fixture of schema 0.7: the schema 0.6 one with a Hue/Saturation
/// adjustment layer at 70 % on top (ADR 0020).
fn golden_document_v0_7() -> Document {
    let mut doc = golden_document_v0_6();
    let id = doc.allocate_layer_id();
    let index = doc.layers().len();
    Edit::InsertLayer {
        parent: None,
        index,
        layer: Layer {
            style: None,
            id,
            name: "Hue/Saturation".into(),
            visible: true,
            opacity: 0.7,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            transform: slopshop_core::Affine::IDENTITY.into(),
            content: LayerContent::Adjustment {
                adjustment: slopshop_core::adjust::Adjustment::HueSaturation {
                    hue: 30.0,
                    saturation: -25.5,
                    lightness: 10.0,
                },
            },
        },
    }
    .apply(&mut doc)
    .unwrap();
    doc
}

/// The document of the golden fixture of schema 0.6: the schema 0.5 one, its gradient layer
/// moved by (-7, 3) and its folder by (5, 0) (ADR 0017).
fn golden_document_v0_6() -> Document {
    let mut doc = golden_document_v0_5();
    let gradient = doc.layers()[0].id;
    let folder = doc.layers()[2].id;
    for (id, x, y) in [(gradient, -7.0, 3.0), (folder, 5.0, 0.0)] {
        Edit::SetLayerTransform {
            id,
            transform: slopshop_core::Affine::translation(x, y).into(),
        }
        .apply(&mut doc)
        .unwrap();
    }
    doc
}

/// The document of the golden fixture of schema 0.5: the schema 0.4 one, its hidden top layer
/// clipped to the group below it.
fn golden_document_v0_5() -> Document {
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
        LayerContent::Raster { image, .. } => {
            slopshop_core::LayerMask::from_transparency(&image.get())
        }
        _ => None,
    };
    let folder = doc.allocate_layer_id();
    let inner = doc.allocate_layer_id();
    let group = |id, name: &str, pass_through, mode, opacity, mask| Layer {
        style: None,
        transform: slopshop_core::Affine::IDENTITY.into(),
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
        stack: None,
        image: slopshop_core::stack::Pixels::ready(image.clone()),
    };
    push(&mut doc, "Gradient", raster(&gradient), 1.0);
    push(&mut doc, "Ramp", raster(&ramp), 0.75);
    let color = LinearRgba::new(0.25, 0.5, 1.0, 0.5);
    push(&mut doc, "Tint", LayerContent::Fill { color }, 0.5);
    let hidden = push(&mut doc, "Gradient again", raster(&gradient), 1.0);
    // A mask from transparency (schema 0.3) on the half-float gray + alpha layer, disabled.
    let ramp_layer = doc.layers()[1].clone();
    if let LayerContent::Raster { image, .. } = &ramp_layer.content {
        let mut mask = slopshop_core::LayerMask::from_transparency(&image.get()).unwrap();
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
    assert_same(&golden_document_v0_5(), &loaded);
    let (loaded, _) = SlopFile::open(&golden_path("0.6")).unwrap();
    assert_same(&golden_document_v0_6(), &loaded);
    let (loaded, _) = SlopFile::open(&golden_path("0.7")).unwrap();
    assert_same(&golden_document_v0_7(), &loaded);
    let (loaded, _) = SlopFile::open(&golden_path("0.8")).unwrap();
    assert_same(&golden_document_v0_8(), &loaded);
    let (loaded, _) = SlopFile::open(&golden_path("0.9")).unwrap();
    assert_same(&golden_document_v0_9(), &loaded);
    let (loaded, _) = SlopFile::open(&golden_path("0.10")).unwrap();
    assert_same(&golden_document_v0_10(), &loaded);
    let (loaded, _) = SlopFile::open(&golden_path("0.12")).unwrap();
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
fn saved_selections_round_trip_with_their_names_order_and_ids() {
    use slopshop_core::document::{SavedSelection, SavedSelectionId};
    let mut doc = sample_document();
    let size = doc.size();
    let coverage = |x0: u32, x1: u32| -> Vec<u8> {
        (0..size.height)
            .flat_map(|_| {
                (0..size.width).flat_map(move |x| {
                    let v: u16 = if (x0..x1).contains(&x) {
                        u16::MAX
                    } else {
                        x as u16 * 7
                    };
                    v.to_ne_bytes()
                })
            })
            .collect()
    };
    let mask = |x0, x1| {
        let image = RasterImage::from_pixels(
            size,
            slopshop_core::selection::SELECTION_FORMAT,
            &coverage(x0, x1),
        )
        .unwrap();
        Selection::new(Arc::new(image)).unwrap()
    };
    // Saved, then one removed: ids keep their gaps, the counter goes on.
    for (name, x0, x1) in [("Hair", 1, 4), ("Shirt", 3, 9), ("Sky", 0, 2)] {
        let id = doc.allocate_saved_selection_id();
        let index = doc.saved_selections().len();
        let saved = SavedSelection {
            id,
            name: name.into(),
            selection: mask(x0, x1),
        };
        Edit::InsertSavedSelection { index, saved }
            .apply(&mut doc)
            .unwrap();
    }
    Edit::RemoveSavedSelection {
        id: SavedSelectionId::from_raw(2),
    }
    .apply(&mut doc)
    .unwrap();
    let path = temp_path("saved-selections.slop");
    SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    let names: Vec<&str> = loaded
        .saved_selections()
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(names, ["Hair", "Sky"]);
    assert_eq!(loaded.next_saved_selection_id(), 4);
    // A document without any writes none, and reads back none.
    let plain = sample_document();
    SlopFile::create(&path, &plain).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert!(loaded.saved_selections().is_empty());
    fs::remove_file(&path).ok();
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
    // Resolution: 72 ppi before schema 0.11 (ADR 0028).
    let resolution = |extra: &str| {
        let json = format!(
            r#"{{"size":[1,1],"working_space":{{"primaries":{{"r":[0.708,0.292],"g":[0.17,0.797],"b":[0.131,0.046],"w":[0.3127,0.329]}},"transfer":{{"kind":"linear"}}}},"next_node_id":1,"stack":[]{extra}}}"#
        );
        read::document_resolution(&serde_json::from_str(&json).unwrap())
    };
    assert_eq!(resolution(""), 72.0);
    assert_eq!(resolution(r#","resolution":300.5"#), 300.5);
    fs::remove_file(&path).ok();
}

#[test]
fn masks_round_trip_and_share_their_tiles() {
    let path = temp_path("masks.slop");
    let mut doc = sample_document();
    for layer in doc.layers().to_vec() {
        if let LayerContent::Raster { image, .. } = &layer.content
            && let Some(mask) = slopshop_core::LayerMask::from_transparency(&image.get())
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

    // Transforms (ADR 0017, 0018): any finite invertible affine, nothing degenerate.
    let moved = |transform: &str| {
        format!(
            r#""1":{{"type":"slopshop.fill","version":5,"name":"f","visible":true,"opacity":1.0,"params":{{"blend_mode":"normal","color":[0,0,0,1],"transform":{transform}}},"inputs":[]}}"#
        )
    };
    let rotated = read(&moved("[0.8,0.6,-0.6,0.8,2.5,-1.25]"), "[1]").unwrap();
    assert_eq!(
        rotated[0].transform.to_array(),
        [0.8, 0.6, -0.6, 0.8, 2.5, -1.25, 0.0, 0.0, 1.0]
    );
    assert_eq!(read(&moved("[1,0,2,0,0,0]"), "[1]"), Err("corrupt"));
    assert_eq!(read(&moved("[1,0,0,1,0]"), "[1]"), Err("corrupt"));
    // Nine numbers (a perspective, ADR 0038) need node version 11.
    assert_eq!(
        read(&moved("[1,0,0,1,0,0,0.001,0,1]"), "[1]"),
        Err("corrupt")
    );

    // Adjustment layers (ADR 0020): a known adjustment with five values; others are refused.
    let adjustment = |params: &str| {
        format!(
            r#""1":{{"type":"slopshop.adjustment","version":3,"name":"a","visible":true,"opacity":1.0,"params":{{"blend_mode":"normal",{params}}},"inputs":[]}}"#
        )
    };
    let exposure = read(
        &adjustment(r#""adjustment":"exposure","values":[1.5,0,1,0,0]"#),
        "[1]",
    )
    .unwrap();
    assert_eq!(
        exposure[0].content,
        LayerContent::Adjustment {
            adjustment: slopshop_core::adjust::Adjustment::Exposure {
                exposure: 1.5,
                offset: 0.0,
                gamma: 1.0
            }
        }
    );
    assert_eq!(
        read(
            &adjustment(r#""adjustment":"colorLookup","values":[0,0,0,0,0]"#),
            "[1]"
        ),
        Err("newerVersion"),
        "an adjustment from a newer SlopShop"
    );
    // Schema 0.8: missing values read as 0; more than 37 are refused (16 before schema 0.13,
    // 20 before 0.15).
    assert!(
        read(
            &adjustment(r#""adjustment":"exposure","values":[1,0,1]"#),
            "[1]"
        )
        .is_ok()
    );
    assert_eq!(
        read(
            &adjustment(
                r#""adjustment":"exposure","values":[1,0,1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]"#
            ),
            "[1]"
        ),
        Err("corrupt")
    );
}

#[test]
fn levels_keep_their_channels_and_write_five_values_without_them() {
    use slopshop_core::adjust::{Adjustment, LEVELS_IDENTITY};
    let composite = Adjustment::Levels {
        input_black: 0.1,
        input_white: 0.9,
        gamma: 1.2,
        output_black: 0.0,
        output_white: 1.0,
        channels: [LEVELS_IDENTITY; 3],
    };
    let channels = Adjustment::Levels {
        input_black: 0.1,
        input_white: 0.9,
        gamma: 1.2,
        output_black: 0.0,
        output_white: 1.0,
        channels: [
            [0.05, 0.95, 0.8, 0.0, 1.0],
            LEVELS_IDENTITY,
            [0.0, 0.7, 1.0, 0.1, 0.9],
        ],
    };
    // Without channels of their own: the five numbers earlier readers read.
    let values = |a: &Adjustment| {
        super::write::adjustment_params(a)["values"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(values(&composite), 5);
    assert_eq!(values(&channels), 20);

    let path = temp_path("levels-channels.slop");
    let mut doc = Document::new(Size::new(4, 4));
    for adjustment in [composite, channels] {
        push(
            &mut doc,
            "levels",
            LayerContent::Adjustment { adjustment },
            1.0,
        );
    }
    SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    fs::remove_file(&path).ok();
}

#[test]
fn gradient_maps_keep_their_stops_and_reverse() {
    use slopshop_core::adjust::Adjustment;
    use slopshop_core::gradient::{Gradient, GradientStop};
    let gradient = Gradient::new(&[
        GradientStop {
            location: 0,
            color: [10, 20, 30],
        },
        GradientStop {
            location: 1500,
            color: [200, 0, 90],
        },
        GradientStop {
            location: 4096,
            color: [255, 250, 245],
        },
    ])
    .unwrap();
    let path = temp_path("gradient-map.slop");
    let mut doc = Document::new(Size::new(4, 4));
    let adjustment = Adjustment::GradientMap {
        gradient,
        reverse: true,
    };
    push(
        &mut doc,
        "map",
        LayerContent::Adjustment { adjustment },
        1.0,
    );
    SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    match &loaded.layers()[0].content {
        LayerContent::Adjustment { adjustment: back } => assert_eq!(*back, adjustment),
        _ => panic!("not an adjustment layer"),
    }
    fs::remove_file(&path).ok();
}

#[test]
fn gradient_fills_keep_their_gradient_and_place() {
    use slopshop_core::gradient::{Gradient, GradientField, GradientShape, GradientStop};
    let gradient = Gradient::new(&[
        GradientStop {
            location: 0,
            color: [10, 20, 30],
        },
        GradientStop {
            location: 3000,
            color: [255, 250, 245],
        },
    ])
    .unwrap();
    let path = temp_path("gradient-fill.slop");
    let mut doc = Document::new(Size::new(4, 4));
    let fields = [
        GradientField {
            gradient,
            alpha: [1.0, 0.25],
            shape: GradientShape::Radial,
            from: [1.5, 2.0],
            to: [-3.25, 7.0],
        },
        GradientField {
            gradient: gradient.reversed(),
            alpha: [1.0, 1.0],
            shape: GradientShape::Linear,
            from: [0.0, 4.0],
            to: [0.0, 0.0],
        },
    ];
    for field in fields {
        push(
            &mut doc,
            "gradient",
            LayerContent::GradientFill { field },
            1.0,
        );
    }
    SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    for (layer, field) in loaded.layers().iter().zip(fields) {
        match &layer.content {
            LayerContent::GradientFill { field: back } => assert_eq!(*back, field),
            _ => panic!("not a gradient fill layer"),
        }
    }
    fs::remove_file(&path).ok();
}

#[test]
fn selective_color_keeps_its_ranges_and_method() {
    use slopshop_core::adjust::{Adjustment, SELECTIVE_RANGES};
    let mut ranges = [[0i16; 4]; SELECTIVE_RANGES];
    ranges[3] = [-100, 50, 25, 100];
    ranges[7] = [5, 0, -5, 0];
    let adjustment = Adjustment::SelectiveColor {
        ranges,
        absolute: true,
    };
    let path = temp_path("selective-color.slop");
    let mut doc = Document::new(Size::new(4, 4));
    push(
        &mut doc,
        "selective",
        LayerContent::Adjustment { adjustment },
        1.0,
    );
    SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    match &loaded.layers()[0].content {
        LayerContent::Adjustment { adjustment: back } => assert_eq!(*back, adjustment),
        _ => panic!("not an adjustment layer"),
    }
    fs::remove_file(&path).ok();
}

#[test]
fn layer_styles_round_trip_and_bad_ones_are_refused() {
    use slopshop_core::selection::StrokeLocation;
    use slopshop_core::style::{ColorOverlay, DropShadow, Glow, LayerStyle, Stroke};
    let mut doc = Document::new(Size::new(40, 30));
    let format = PixelFormat {
        layout: ChannelLayout::Rgba,
        sample: SampleType::U8,
        color_space: ColorSpace::SRGB,
        alpha: AlphaMode::Straight,
    };
    let pixels = noise(40 * 30 * 4, 7);
    let raster = Arc::new(RasterImage::from_pixels(Size::new(40, 30), format, &pixels).unwrap());
    let photo = push(&mut doc, "photo", LayerContent::raster(raster), 1.0);
    let fill = push(
        &mut doc,
        "fill",
        LayerContent::Fill {
            color: LinearRgba::new(0.2, 0.4, 0.6, 1.0),
        },
        0.5,
    );
    let group = push(
        &mut doc,
        "group",
        LayerContent::Group {
            children: Vec::new(),
            pass_through: true,
        },
        1.0,
    );
    let styles = [
        LayerStyle {
            fill_opacity: 0.3,
            drop_shadow: Some(DropShadow {
                enabled: false,
                angle: -33.5,
                spread: 12.0,
                ..DropShadow::default()
            }),
            stroke: Some(Stroke {
                position: StrokeLocation::Center,
                size: 7.25,
                ..Stroke::default()
            }),
            ..LayerStyle::default()
        },
        LayerStyle {
            color_overlay: Some(ColorOverlay {
                mode: BlendMode::Screen,
                opacity: 0.125,
                ..ColorOverlay::default()
            }),
            outer_glow: Some(Glow {
                spread: 40.0,
                ..Glow::default()
            }),
            inner_shadow: Some(DropShadow {
                distance: 11.0,
                ..DropShadow::default()
            }),
            inner_glow: Some(Glow {
                enabled: false,
                size: 21.5,
                ..Glow::default()
            }),
            ..LayerStyle::default()
        },
        LayerStyle {
            fill_opacity: 0.6,
            drop_shadow: Some(DropShadow::default()),
            ..LayerStyle::default()
        },
    ];
    for (id, style) in [photo, fill, group].into_iter().zip(styles) {
        Edit::SetLayerStyle {
            id: slopshop_core::LayerId::from_raw(id),
            style: Some(Box::new(style)),
        }
        .apply(&mut doc)
        .unwrap();
    }
    let path = temp_path("styles.slop");
    SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    fs::remove_file(&path).ok();

    // Styles are refused when they are out of range.
    let mut bad = serde_json::json!({ "fill_opacity": 2.0 });
    assert_eq!(super::style::from_json(&bad), None);
    bad = serde_json::json!({ "fill_opacity": 1.0, "stroke": { "size": 3 } });
    assert_eq!(super::style::from_json(&bad), None);
    assert_eq!(
        super::style::from_json(&serde_json::json!({ "fill_opacity": 1.0 })),
        Some(LayerStyle::default())
    );
}

#[test]
fn hidden_entries_round_trip() {
    let mut doc = golden_document();
    let (id, stack) = doc
        .all_layers()
        .find_map(|l| match &l.content {
            LayerContent::Raster {
                stack: Some(stack), ..
            } => Some((l.id, stack.clone())),
            _ => None,
        })
        .expect("a layer with a stack");
    // The Invert below the paint hidden, the paint shown.
    assert_eq!(stack.entries().len(), 2);
    Edit::set_entry(&doc, id, 0, None, true)
        .unwrap()
        .apply(&mut doc)
        .unwrap();
    let path = temp_path("hidden-entries.slop");
    SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    let LayerContent::Raster {
        stack: Some(read), ..
    } = &loaded.layer(id).unwrap().content
    else {
        panic!("a layer with a stack");
    };
    assert!(read.entries()[0].hidden());
    assert!(!read.entries()[1].hidden());
    fs::remove_file(&path).ok();
}

#[test]
fn layers_in_perspective_round_trip_at_node_version_11() {
    let mut doc = golden_document();
    let (id, size) = doc
        .all_layers()
        .find_map(|l| match &l.content {
            LayerContent::Raster {
                image,
                stack: Some(_),
            } => Some((l.id, image.size())),
            _ => None,
        })
        .expect("a layer with a stack");
    let (w, h) = (f64::from(size.width), f64::from(size.height));
    let keystone = slopshop_core::Projective::from_rect_to_quad(
        [0.0, 0.0, w, h],
        [(w * 0.25, 0.0), (w * 0.75, 0.0), (w, h), (0.0, h)],
    )
    .unwrap();
    Edit::SetLayerTransform {
        id,
        transform: keystone,
    }
    .apply(&mut doc)
    .unwrap();
    // Applied in perspective: the effect keeps that placement.
    Edit::apply_effect(&doc, &[id], slopshop_core::adjust::Adjustment::Invert)
        .unwrap()
        .apply(&mut doc)
        .unwrap();
    let path = temp_path("perspective.slop");
    SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    let layer = loaded.layer(id).unwrap();
    assert_eq!(layer.transform, keystone);
    let LayerContent::Raster {
        stack: Some(read), ..
    } = &layer.content
    else {
        panic!("a layer with a stack");
    };
    let Some(Entry::Effect(effect)) = read.entries().last() else {
        panic!("an effect entry");
    };
    assert!(!effect.steps()[0].to_document.is_affine());
    fs::remove_file(&path).ok();
}

#[test]
fn filter_entries_round_trip() {
    let mut doc = golden_document();
    let id = doc
        .all_layers()
        .find_map(|l| match &l.content {
            LayerContent::Raster { stack: Some(_), .. } => Some(l.id),
            _ => None,
        })
        .expect("a layer with a stack");
    for radius in [3.5, 1.25] {
        Edit::apply_filter(
            &doc,
            id,
            slopshop_core::filter::Filter::GaussianBlur { radius },
        )
        .unwrap()
        .apply(&mut doc)
        .unwrap();
    }
    let path = temp_path("filter-entries.slop");
    SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    let LayerContent::Raster {
        stack: Some(read), ..
    } = &loaded.layer(id).unwrap().content
    else {
        panic!("a layer with a stack");
    };
    // Two blurs in a row: one blur, on top of the Invert and the paint.
    assert_eq!(read.entries().len(), 3);
    let Entry::Filter(filter) = &read.entries()[2] else {
        panic!("a filter entry");
    };
    assert_eq!(filter.steps().len(), 1);
    fs::remove_file(&path).ok();
}

/// A field that pushed pixels around and froze a spot, over a layer of `size`.
fn liquified_field(size: slopshop_core::Size) -> Arc<slopshop_core::liquify::Field> {
    use slopshop_core::liquify::{Brush, Field, Stroke, Tool};
    let mut field = Field::new(size);
    let brush = Brush {
        size: 16.0,
        density: 100.0,
        pressure: 100.0,
        rate: 100.0,
    };
    for (tool, from, to) in [
        (Tool::ForwardWarp, [8.0, 10.0], [24.0, 12.0]),
        (Tool::Freeze, [30.0, 22.0], [30.0, 22.0]),
    ] {
        let mut stroke = Stroke::new(tool, brush);
        stroke.move_to(&mut field, from);
        stroke.move_to(&mut field, to);
        stroke.finish(&mut field);
    }
    Arc::new(field)
}

#[test]
fn liquify_entries_round_trip() {
    let mut doc = golden_document();
    let id = doc
        .all_layers()
        .find_map(|l| match &l.content {
            LayerContent::Raster { stack: Some(_), .. } => Some(l.id),
            _ => None,
        })
        .expect("a layer with a stack");
    let size = match &doc.layer(id).unwrap().content {
        LayerContent::Raster { stack: Some(s), .. } => s.original().size(),
        _ => unreachable!(),
    };
    let field = liquified_field(size);
    Edit::apply_liquify(&doc, id, Arc::clone(&field), None)
        .unwrap()
        .apply(&mut doc)
        .unwrap();
    // A second one, hidden: its eye is kept too.
    Edit::apply_liquify(&doc, id, liquified_field(size), None)
        .unwrap()
        .apply(&mut doc)
        .unwrap();
    let count = match &doc.layer(id).unwrap().content {
        LayerContent::Raster { stack: Some(s), .. } => s.entries().len(),
        _ => unreachable!(),
    };
    Edit::set_entry(&doc, id, count - 1, None, true)
        .unwrap()
        .apply(&mut doc)
        .unwrap();
    let path = temp_path("liquify-entries.slop");
    SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    let LayerContent::Raster {
        stack: Some(read), ..
    } = &loaded.layer(id).unwrap().content
    else {
        panic!("a layer with a stack");
    };
    let [.., Entry::Liquify(first), Entry::Liquify(second)] = read.entries() else {
        panic!("two liquify entries");
    };
    assert!(!first.hidden() && second.hidden());
    // What was read is what was stored: the field, its freeze, its cell.
    assert!(!first.field().is_identity() && first.field().has_frozen());
    for p in [[16.0, 11.0], [30.0, 22.0], [2.0, 2.0]] {
        assert_eq!(
            first.field().displacement_at(p),
            field.displacement_at(p),
            "{p:?}"
        );
        assert_eq!(first.field().frozen_at(p), field.frozen_at(p));
    }
    // Saved again unchanged: nothing new to write for it (the images are the same).
    let (again, mut file) = SlopFile::open(&path).unwrap();
    let report = file.save(&again).unwrap();
    assert!(
        report.bytes_written < 64 * 1024,
        "{} bytes written for nothing new",
        report.bytes_written
    );
    fs::remove_file(&path).ok();
}

#[test]
fn a_liquify_entry_with_images_that_do_not_fit_is_refused() {
    use slopshop_core::liquify::Field;
    // The images of a field over a larger layer: a corrupt entry, not a panic.
    let big = Field::new(slopshop_core::Size::new(900, 900));
    let (displacement, frozen) = big.to_images().unwrap();
    assert!(
        Field::from_images(
            slopshop_core::Size::new(100, 100),
            1,
            &displacement,
            &frozen
        )
        .is_err()
    );
}

#[test]
fn entries_applied_several_times_in_older_files_are_read_as_an_entry_each() {
    use slopshop_core::adjust::Adjustment;
    use slopshop_core::stack::EffectEntry;
    let mut doc = golden_document();
    let (id, stack) = doc
        .all_layers()
        .find_map(|l| match &l.content {
            LayerContent::Raster {
                stack: Some(stack), ..
            } => Some((l.id, stack.clone())),
            _ => None,
        })
        .expect("a layer with a stack");
    let step = |levels| {
        Arc::new(Effect {
            adjustment: Adjustment::Posterize { levels },
            selection: None,
            to_document: slopshop_core::Affine::IDENTITY.into(),
            space: doc.blend_space(),
        })
    };
    // ×2, as files written before ADR 0034's combining kept it.
    let mut entries = stack.entries().to_vec();
    entries.push(Entry::Effect(Arc::new(
        EffectEntry::new(vec![step(4.0), step(9.0)]).unwrap(),
    )));
    let twice = LayerStack::with_entries(Arc::clone(stack.original()), entries).unwrap();
    Edit::SetLayerStack {
        id,
        stack: twice,
        shown: None,
    }
    .apply(&mut doc)
    .unwrap();
    let path = temp_path("applied-twice.slop");
    SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    let LayerContent::Raster {
        stack: Some(read), ..
    } = &loaded.layer(id).unwrap().content
    else {
        panic!("a layer with a stack");
    };
    assert_eq!(read.entries().len(), stack.entries().len() + 2);
    for entry in &read.entries()[stack.entries().len()..] {
        let Entry::Effect(effect) = entry else {
            panic!("an effect entry");
        };
        assert_eq!(effect.steps().len(), 1);
    }
    fs::remove_file(&path).ok();
}

#[test]
fn guides_round_trip_in_their_order_and_unknown_axes_are_newer() {
    use slopshop_core::document::{Guide, GuideAxis};
    let mut doc = sample_document();
    let guides = vec![
        Guide {
            axis: GuideAxis::Vertical,
            position: 12.0,
        },
        Guide {
            axis: GuideAxis::Horizontal,
            position: -3.5,
        },
        Guide {
            axis: GuideAxis::Vertical,
            position: 1.25,
        },
    ];
    Edit::SetGuides {
        guides: guides.clone(),
    }
    .apply(&mut doc)
    .unwrap();
    let path = temp_path("guides.slop");
    SlopFile::create(&path, &doc).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert_same(&doc, &loaded);
    assert_eq!(loaded.guides(), guides.as_slice());
    // A document without any reads back none.
    SlopFile::create(&path, &sample_document()).unwrap();
    let (loaded, _) = SlopFile::open(&path).unwrap();
    assert!(loaded.guides().is_empty());
    fs::remove_file(&path).ok();

    let read = |extra: &str| {
        let json = format!(
            r#"{{"size":[1,1],"working_space":{{"primaries":{{"r":[0.708,0.292],"g":[0.17,0.797],"b":[0.131,0.046],"w":[0.3127,0.329]}},"transfer":{{"kind":"linear"}}}},"next_node_id":1,"stack":[]{extra}}}"#
        );
        read::document_guides(&serde_json::from_str(&json).unwrap()).map_err(|e| e.code())
    };
    assert_eq!(read(""), Ok(Vec::new()));
    assert_eq!(
        read(r#","guides":[{"axis":"horizontal","position":4}]"#),
        Ok(vec![Guide {
            axis: GuideAxis::Horizontal,
            position: 4.0
        }])
    );
    assert_eq!(
        read(r#","guides":[{"axis":"diagonal","position":4}]"#),
        Err("newerVersion")
    );
}
