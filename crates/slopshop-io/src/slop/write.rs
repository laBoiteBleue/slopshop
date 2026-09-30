//! Writing `.slop` files: a compact new file, or one more generation appended in place.

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value, json};
use slopshop_core::color::SampleType;
use slopshop_core::document::{Document, LayerContent};
use slopshop_core::raster::{ImageId, RasterImage, TILE_SIZE};

use super::format::{
    Codec, Encoding, Filter, HEADER_LEN, Hash, Header, IndexEntry, Kind, RECORD_HEADER_LEN,
    RecordHeader, RecordRef, SLOT_LEN, SLOT_OFFSETS, Slot, encode_blob, encode_index, record_span,
};
use super::manifest::{
    ColorSpaceDto, DocumentDto, FormatDto, ImageDto, LevelDto, Manifest, NODE_ADJUSTMENT,
    NODE_FILL, NODE_GROUP, NODE_RASTER, NODE_VERSION, NODE_VERSION_CLIPPED,
    NODE_VERSION_TRANSFORMED, NodeDto, PYRAMID_ALGORITHM, SCHEMA_MAJOR, SCHEMA_MINOR, Schema,
    Writer,
};
use super::read::best_slot;
use super::{FileError, ImageRecord, Residue, SaveReport, SlopFile};
use crate::atomic::TempFile;

/// zstd levels: fast for tiles (most of the bytes), a little denser for the small manifest.
const TILE_LEVEL: i32 = 1;
const MANIFEST_LEVEL: i32 = 3;

/// What one generation needs from the file and the session.
struct State {
    /// Blobs already in the file (updated as blobs are written).
    index: HashMap<Hash, IndexEntry>,
    images: HashMap<ImageId, Arc<ImageRecord>>,
    residue: Residue,
}

/// What writing a generation produced.
struct Written {
    slot: Slot,
    /// Bytes of the records the generation uses.
    live_bytes: u64,
}

/// A new compact file at `path`, through a temporary file renamed over it.
pub(super) fn create(
    path: &Path,
    document: &Document,
    images: HashMap<ImageId, Arc<ImageRecord>>,
    residue: Residue,
) -> Result<SlopFile, FileError> {
    let (temp, file) = TempFile::create(path)?;
    let mut out = BufWriter::new(file);
    out.write_all(&Header::current().encode())?;
    let mut state = State {
        index: HashMap::new(),
        images,
        residue,
    };
    let written = write_generation(&mut out, HEADER_LEN, document, &mut state, 1)?;
    let mut file = out.into_inner().map_err(|e| e.into_error())?;
    write_slot(&mut file, &written.slot)?;
    // Durable before it replaces the destination.
    file.sync_all()?;
    drop(file);
    temp.persist(path)?;
    Ok(SlopFile {
        path: path.to_owned(),
        generation: written.slot.generation,
        committed_len: written.slot.committed_len,
        index: state.index,
        images: kept_images(state.images, document),
        residue: state.residue,
        dead_bytes: dead_bytes(&written),
        read_only: false,
    })
}

/// One more generation appended to `file` (ADR 0009, commit protocol).
pub(super) fn append(file: &mut SlopFile, document: &Document) -> Result<SaveReport, FileError> {
    let mut handle = OpenOptions::new().read(true).write(true).open(&file.path)?;
    // Optimistic concurrency: someone else saved since we opened or saved.
    let mut header = vec![0u8; HEADER_LEN as usize];
    handle.read_exact(&mut header)?;
    let current = best_slot(&header, handle.metadata()?.len());
    if current.map(|slot| (slot.generation, slot.committed_len))
        != Some((file.generation, file.committed_len))
    {
        return Err(FileError::Conflict);
    }
    // Junk from an interrupted save goes away first.
    handle.set_len(file.committed_len)?;
    handle.seek(SeekFrom::Start(file.committed_len))?;

    // Work on copies: if anything fails, the session still describes the committed file.
    let mut state = State {
        index: file.index.clone(),
        images: file.images.clone(),
        residue: file.residue.clone(),
    };
    let mut out = BufWriter::new(handle);
    let written = write_generation(
        &mut out,
        file.committed_len,
        document,
        &mut state,
        file.generation + 1,
    )?;
    let mut handle = out.into_inner().map_err(|e| e.into_error())?;
    // The data must be on disk before the slot that points to it.
    handle.sync_data()?;
    write_slot(&mut handle, &written.slot)?;
    handle.sync_data()?;

    let report = SaveReport {
        bytes_written: written.slot.committed_len - file.committed_len,
        compacted: false,
    };
    file.generation = written.slot.generation;
    file.committed_len = written.slot.committed_len;
    file.index = state.index;
    file.images = kept_images(state.images, document);
    file.residue = state.residue;
    file.dead_bytes = dead_bytes(&written);
    Ok(report)
}

fn dead_bytes(written: &Written) -> u64 {
    (written.slot.committed_len - HEADER_LEN).saturating_sub(written.live_bytes)
}

/// The cached records of the images still in the document.
fn kept_images(
    mut images: HashMap<ImageId, Arc<ImageRecord>>,
    document: &Document,
) -> HashMap<ImageId, Arc<ImageRecord>> {
    let present: HashSet<ImageId> = rasters(document).map(|image| image.id()).collect();
    images.retain(|id, _| present.contains(id));
    images
}

/// Every image of the document: layer rasters and masks, groups included.
fn rasters(document: &Document) -> impl Iterator<Item = &Arc<RasterImage>> {
    document.all_layers().flat_map(|layer| {
        let content = match &layer.content {
            LayerContent::Raster { image } => Some(image),
            _ => None,
        };
        content
            .into_iter()
            .chain(layer.mask.as_ref().map(|m| &m.image))
    })
}

fn write_slot(file: &mut File, slot: &Slot) -> Result<(), FileError> {
    file.seek(SeekFrom::Start(
        SLOT_OFFSETS[(slot.generation % 2) as usize],
    ))?;
    file.write_all(&slot.encode())?;
    file.flush()?;
    Ok(())
}

/// Append the records of one generation at `start` (where `out` is positioned): the blobs the
/// file lacks, the manifest, the index and the commit record.
fn write_generation(
    out: &mut BufWriter<File>,
    start: u64,
    document: &Document,
    state: &mut State,
    generation: u64,
) -> Result<Written, FileError> {
    let mut pos = start;

    // Every image once, hashed unless already known.
    let mut images: Vec<(&Arc<RasterImage>, Arc<ImageRecord>)> = Vec::new();
    for image in rasters(document) {
        if images.iter().any(|(known, _)| known.id() == image.id()) {
            continue;
        }
        let record = match state.images.get(&image.id()) {
            Some(record) => record.clone(),
            None => {
                let record = Arc::new(hash_image(image));
                state.images.insert(image.id(), record.clone());
                record
            }
        };
        images.push((image, record));
    }

    // The tiles the file lacks, once each.
    let mut queued = HashSet::new();
    let mut tiles: Vec<(Hash, &Arc<[u8]>, Encoding)> = Vec::new();
    for (image, record) in &images {
        let encoding = tile_encoding(image);
        for (level, (_, hashes)) in image.levels().iter().zip(&record.levels) {
            for (tile, hash) in level.tiles().iter().zip(hashes) {
                if !state.index.contains_key(hash) && queued.insert(*hash) {
                    tiles.push((*hash, tile, encoding));
                }
            }
        }
    }
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    // Bounded memory: a few compressed tiles per thread at a time.
    for batch in tiles.chunks(threads * 4) {
        let encoded = parallel_map(batch, threads, |(_, tile, encoding)| {
            encode_blob(tile, *encoding, TILE_LEVEL)
        });
        for ((hash, tile, _), (stored, encoding)) in batch.iter().zip(encoded) {
            let entry = IndexEntry {
                kind: Kind::Tile,
                offset: pos,
                stored_len: stored.len() as u64,
                raw_len: tile.len() as u64,
                encoding,
            };
            pos = write_record(out, pos, Kind::Tile, encoding, tile.len(), *hash, &stored)?;
            state.index.insert(*hash, entry);
        }
    }

    // Tile tables (hashes do not compress).
    for (_, record) in &images {
        for (table, hashes) in &record.levels {
            if state.index.contains_key(table) {
                continue;
            }
            let raw: Vec<u8> = hashes.iter().flat_map(|h| h.0).collect();
            let entry = IndexEntry {
                kind: Kind::Table,
                offset: pos,
                stored_len: raw.len() as u64,
                raw_len: raw.len() as u64,
                encoding: Encoding::RAW,
            };
            pos = write_record(
                out,
                pos,
                Kind::Table,
                Encoding::RAW,
                raw.len(),
                *table,
                &raw,
            )?;
            state.index.insert(*table, entry);
        }
    }

    // What the generation uses, for the dead-bytes count.
    let mut live_bytes = 0u64;
    let mut counted = HashSet::new();
    for (_, record) in &images {
        for (table, hashes) in &record.levels {
            for hash in std::iter::once(table).chain(hashes) {
                if counted.insert(*hash)
                    && let Some(entry) = state.index.get(hash)
                {
                    live_bytes += record_span(entry.stored_len);
                }
            }
        }
    }

    // Manifest.
    let manifest = build_manifest(document, &images, &state.residue);
    let json = serde_json::to_vec(&manifest)
        .map_err(|e| FileError::Corrupt(format!("cannot serialize the manifest: {e}")))?;
    let manifest_hash = Hash::of(&json);
    let wanted = Encoding {
        codec: Codec::Zstd,
        filter: Filter::None,
        stride: 0,
    };
    let (stored, encoding) = encode_blob(&json, wanted, MANIFEST_LEVEL);
    let manifest_ref = RecordRef {
        offset: pos,
        len: stored.len() as u64,
        hash: manifest_hash,
    };
    pos = write_record(
        out,
        pos,
        Kind::Manifest,
        encoding,
        json.len(),
        manifest_hash,
        &stored,
    )?;
    live_bytes += record_span(stored.len() as u64);

    // Index.
    let index = encode_index(&state.index);
    let index_hash = Hash::of(&index);
    let (stored, encoding) = encode_blob(&index, wanted, TILE_LEVEL);
    let index_ref = RecordRef {
        offset: pos,
        len: stored.len() as u64,
        hash: index_hash,
    };
    pos = write_record(
        out,
        pos,
        Kind::Index,
        encoding,
        index.len(),
        index_hash,
        &stored,
    )?;
    live_bytes += record_span(stored.len() as u64);

    // Commit record: the slot, also kept in the log for recovery.
    let slot = Slot {
        generation,
        manifest: manifest_ref,
        index: index_ref,
        committed_len: pos + record_span(SLOT_LEN as u64),
        writer: writer_version(),
    };
    let payload = slot.encode();
    let end = write_record(
        out,
        pos,
        Kind::Commit,
        Encoding::RAW,
        SLOT_LEN,
        Hash::of(&payload),
        &payload,
    )?;
    debug_assert_eq!(end, slot.committed_len);
    live_bytes += record_span(SLOT_LEN as u64);
    out.flush()?;
    Ok(Written { slot, live_bytes })
}

/// Write one record at `pos` (header, payload, padding to the record alignment); the position
/// after it.
fn write_record(
    out: &mut BufWriter<File>,
    pos: u64,
    kind: Kind,
    encoding: Encoding,
    raw_len: usize,
    hash: Hash,
    stored: &[u8],
) -> Result<u64, FileError> {
    let header = RecordHeader {
        kind,
        encoding,
        stored_len: stored.len() as u64,
        raw_len: raw_len as u64,
        hash,
    };
    out.write_all(&header.encode())?;
    out.write_all(stored)?;
    let span = record_span(stored.len() as u64);
    let padding = span - (RECORD_HEADER_LEN + stored.len()) as u64;
    out.write_all(&[0u8; 8][..padding as usize])?;
    Ok(pos + span)
}

/// Byte-plane filter on whole pixels; delta helps integer samples, hurts floats (ADR 0009).
fn tile_encoding(image: &RasterImage) -> Encoding {
    let stored = image.stored_format();
    Encoding {
        codec: Codec::Zstd,
        filter: match stored.sample {
            SampleType::U8 | SampleType::U16 => Filter::ShuffleDelta,
            SampleType::F16 | SampleType::F32 => Filter::Shuffle,
        },
        stride: stored.bytes_per_pixel() as u8,
    }
}

/// Hash every tile of every level (in parallel), and derive the tables and the image key.
fn hash_image(image: &RasterImage) -> ImageRecord {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let levels: Vec<(Hash, Vec<Hash>)> = image
        .levels()
        .iter()
        .map(|level| {
            let hashes = parallel_map(level.tiles(), threads, |tile| Hash::of(tile));
            let table: Vec<u8> = hashes.iter().flat_map(|h| h.0).collect();
            (Hash::of(&table), hashes)
        })
        .collect();
    let key = image_key(image, levels[0].0);
    ImageRecord { key, levels }
}

/// The image key: BLAKE3 over the format, the size, the tile size and the level-0 table.
pub(super) fn image_key(image: &RasterImage, level0_table: Hash) -> Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"slopshop.image.v1\0");
    // Field order is fixed by the DTO, so the JSON is canonical.
    let format = serde_json::to_vec(&FormatDto::new(&image.format())).unwrap_or_default();
    hasher.update(&format);
    hasher.update(&image.size().width.to_le_bytes());
    hasher.update(&image.size().height.to_le_bytes());
    hasher.update(&TILE_SIZE.to_le_bytes());
    hasher.update(&level0_table.0);
    Hash(*hasher.finalize().as_bytes())
}

fn build_manifest(
    document: &Document,
    images: &[(&Arc<RasterImage>, Arc<ImageRecord>)],
    residue: &Residue,
) -> Manifest {
    let key_of = |image: &Arc<RasterImage>| {
        images
            .iter()
            .find(|(known, _)| known.id() == image.id())
            .map(|(_, record)| record.key)
    };
    let mut nodes = std::collections::BTreeMap::new();
    for layer in document.all_layers() {
        let id = layer.id.get();
        let mut inputs = Vec::new();
        let (kind, params) = match &layer.content {
            LayerContent::Raster { image } => {
                let key = key_of(image).map(Hash::to_key).unwrap_or_default();
                (NODE_RASTER, json!({ "image": key }))
            }
            LayerContent::Fill { color } => (
                NODE_FILL,
                json!({ "color": [color.r, color.g, color.b, color.a] }),
            ),
            LayerContent::Group {
                children,
                pass_through,
            } => {
                inputs = children.iter().map(|child| child.id.get()).collect();
                (NODE_GROUP, json!({ "pass_through": pass_through }))
            }
            LayerContent::Adjustment { adjustment } => {
                // At least five values: what readers of schema 0.7 expect.
                let used = adjustment.param_count().max(5);
                let values = &adjustment.params()[..used];
                (
                    NODE_ADJUSTMENT,
                    json!({ "adjustment": adjustment.id(), "values": values }),
                )
            }
        };
        let mut params = match params {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        params.insert("blend_mode".to_owned(), Value::from(layer.blend_mode.id()));
        if layer.clipped {
            params.insert("clipped".to_owned(), Value::from(true));
        }
        if !layer.transform.is_identity() {
            params.insert(
                "transform".to_owned(),
                Value::from(layer.transform.to_array().to_vec()),
            );
        }
        if let Some(mask) = &layer.mask {
            let key = key_of(&mask.image).map(Hash::to_key).unwrap_or_default();
            params.insert(
                "mask".to_owned(),
                json!({
                    "image": key,
                    "enabled": mask.enabled,
                    "replaces_alpha": mask.replaces_alpha,
                }),
            );
        }
        nodes.insert(
            id.to_string(),
            NodeDto {
                kind: kind.to_owned(),
                version: if !layer.transform.is_identity() {
                    NODE_VERSION_TRANSFORMED
                } else if layer.clipped {
                    NODE_VERSION_CLIPPED
                } else {
                    NODE_VERSION
                },
                name: layer.name.clone(),
                visible: layer.visible,
                opacity: layer.opacity,
                params,
                inputs,
                extra: residue.nodes.get(&id).cloned().unwrap_or_default(),
            },
        );
    }
    let mut image_dtos = std::collections::BTreeMap::new();
    for (image, record) in images {
        let size = image.size();
        image_dtos.insert(
            record.key.to_key(),
            ImageDto {
                size: [size.width, size.height],
                tile_size: TILE_SIZE,
                source_format: FormatDto::new(&image.format()),
                levels: record
                    .levels
                    .iter()
                    .enumerate()
                    .map(|(level, (table, _))| LevelDto {
                        table: table.to_key(),
                        derived: level > 0,
                    })
                    .collect(),
                pyramid_algorithm: PYRAMID_ALGORITHM.to_owned(),
                extra: residue.images.get(&record.key).cloned().unwrap_or_default(),
            },
        );
    }
    let size = document.size();
    Manifest {
        schema: Schema {
            major: SCHEMA_MAJOR,
            minor: SCHEMA_MINOR,
        },
        writer: Writer::current(),
        document: DocumentDto {
            size: [size.width, size.height],
            working_space: ColorSpaceDto::new(&document.working_space()),
            next_node_id: document.next_layer_id(),
            stack: document.layers().iter().map(|l| l.id.get()).collect(),
            blend_space: Some(document.blend_space().id().to_owned()),
            extra: residue.document.clone(),
        },
        nodes,
        images: image_dtos,
        sections: residue.sections.clone(),
        extra: residue.manifest.clone(),
    }
}

/// The writer's version, as the slot records it.
fn writer_version() -> [u16; 4] {
    let mut parts = env!("CARGO_PKG_VERSION")
        .split('.')
        .map(|part| part.parse::<u16>().unwrap_or(0));
    [
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        0,
    ]
}

/// `f` over `items` on up to `threads` scoped threads, results in order.
pub(super) fn parallel_map<T: Sync, R: Send>(
    items: &[T],
    threads: usize,
    f: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    if items.len() <= 1 || threads <= 1 {
        return items.iter().map(f).collect();
    }
    let chunk = items.len().div_ceil(threads);
    std::thread::scope(|scope| {
        let f = &f;
        let workers: Vec<_> = items
            .chunks(chunk)
            .map(|part| scope.spawn(move || part.iter().map(f).collect::<Vec<R>>()))
            .collect();
        workers
            .into_iter()
            // Invariant: `f` does not panic (hashing and compression of owned bytes).
            .flat_map(|worker| worker.join().expect("worker panicked"))
            .collect()
    })
}
