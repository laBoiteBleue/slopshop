//! Reading `.slop` files: pick the newest intact generation, verify every blob against its
//! hash, and rebuild the document.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Map, Value};
use slopshop_core::adjust::PARAM_COUNT;
use slopshop_core::color::LinearRgba;
use slopshop_core::document::{Document, Layer, LayerContent, LayerId, LayerMask, MAX_GROUP_DEPTH};
use slopshop_core::geom::Size;
use slopshop_core::raster::{ImageId, RasterImage, TILE_SIZE};
use slopshop_core::{BlendMode, BlendSpace};

use super::format::{
    HEADER_LEN, Hash, Header, IndexEntry, Kind, RECORD_HEADER_LEN, RecordHeader, RecordRef,
    SLOT_LEN, SLOT_OFFSETS, Slot, corrupt, decode_blob, decode_index, record_span,
};
use super::manifest::{
    DocumentDto, Manifest, NODE_ADJUSTMENT, NODE_FILL, NODE_GROUP, NODE_RASTER,
    NODE_VERSION_CLIPPED, NODE_VERSION_PAINTED, NODE_VERSION_TRANSFORMED, NodeDto,
    PYRAMID_ALGORITHM, SCHEMA_MAJOR,
};
use super::write::{image_key, parallel_map};
use super::{FileError, ImageRecord, Residue, SlopFile};

/// The newest intact slot of a header, whose committed length fits in the file.
pub(super) fn best_slot(header: &[u8], file_len: u64) -> Option<Slot> {
    slots(header, file_len).into_iter().next()
}

/// Intact slots, newest first.
fn slots(header: &[u8], file_len: u64) -> Vec<Slot> {
    let mut slots: Vec<Slot> = SLOT_OFFSETS
        .iter()
        .filter_map(|&offset| {
            let at = offset as usize;
            header.get(at..at + SLOT_LEN).and_then(Slot::decode)
        })
        .filter(|slot| (HEADER_LEN..=file_len).contains(&slot.committed_len))
        .collect();
    slots.sort_by_key(|slot| std::cmp::Reverse(slot.generation));
    slots
}

/// A file and its length, read once: every read is checked against it.
struct Source<'a> {
    file: &'a File,
    len: u64,
}

pub(super) fn open(path: &Path) -> Result<(Document, SlopFile), FileError> {
    let handle = File::open(path)?;
    let file_len = handle.metadata()?.len();
    let source = Source {
        file: &handle,
        len: file_len,
    };
    let file = &source;
    let header_len = HEADER_LEN.min(file_len) as usize;
    let header_bytes = read_at(file, 0, header_len)?;
    let header = Header::decode(&header_bytes)?;
    if (header_bytes.len() as u64) < HEADER_LEN {
        return Err(corrupt("truncated header"));
    }

    // The slots first; if neither leads to a readable generation, the commit records.
    let mut candidates = slots(&header_bytes, file_len);
    let mut first_error = None;
    let mut tried = HashSet::new();
    let mut scanned = false;
    loop {
        let Some(slot) = candidates.first().copied() else {
            if scanned {
                break;
            }
            scanned = true;
            candidates = scan_commits(file, file_len);
            candidates.retain(|slot| !tried.contains(&slot.generation));
            continue;
        };
        candidates.remove(0);
        tried.insert(slot.generation);
        match load(file, &slot) {
            Ok((document, index, images, residue)) => {
                let live: u64 = live_bytes(&index, &slot);
                let file = SlopFile {
                    path: path.to_owned(),
                    generation: slot.generation,
                    committed_len: slot.committed_len,
                    index,
                    images,
                    residue,
                    dead_bytes: (slot.committed_len - HEADER_LEN).saturating_sub(live),
                    read_only: header.read_only(),
                };
                return Ok((document, file));
            }
            // Never hide a version problem behind an older generation.
            Err(e @ (FileError::NewerVersion { .. } | FileError::UnknownNodeType(_))) => {
                return Err(e);
            }
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    Err(first_error.unwrap_or_else(|| corrupt("no readable generation")))
}

/// An estimate of the bytes the generation uses: every indexed blob (unused blobs are rare
/// between compactions) plus the manifest, index and commit records.
fn live_bytes(index: &HashMap<Hash, IndexEntry>, slot: &Slot) -> u64 {
    let blobs: u64 = index.values().map(|e| record_span(e.stored_len)).sum();
    blobs
        + record_span(slot.manifest.len)
        + record_span(slot.index.len)
        + record_span(SLOT_LEN as u64)
}

/// Commit records found by walking the records from the header on, newest first: recovery
/// when both slots are damaged. Stops at the first record that does not parse.
fn scan_commits(file: &Source<'_>, file_len: u64) -> Vec<Slot> {
    let mut found = Vec::new();
    let mut at = HEADER_LEN;
    while at + RECORD_HEADER_LEN as u64 <= file_len {
        let Ok(bytes) = read_at(file, at, RECORD_HEADER_LEN) else {
            break;
        };
        let Ok(header) = RecordHeader::decode(&bytes) else {
            break;
        };
        let span = record_span(header.stored_len);
        if at.checked_add(span).is_none_or(|end| end > file_len) {
            break;
        }
        if header.kind == Kind::Commit
            && header.stored_len == SLOT_LEN as u64
            && let Ok(payload) = read_at(file, at + RECORD_HEADER_LEN as u64, SLOT_LEN)
            && let Some(slot) = Slot::decode(&payload)
            && slot.committed_len == at + span
        {
            found.push(slot);
        }
        at += span;
    }
    found.sort_by_key(|slot| std::cmp::Reverse(slot.generation));
    found
}

type Loaded = (
    Document,
    HashMap<Hash, IndexEntry>,
    HashMap<ImageId, Arc<ImageRecord>>,
    Residue,
);

/// Everything of one generation: the index, the manifest, the images and the document.
fn load(file: &Source<'_>, slot: &Slot) -> Result<Loaded, FileError> {
    let index = decode_index(&read_record(file, &slot.index, Kind::Index)?)?;
    let manifest_json = read_record(file, &slot.manifest, Kind::Manifest)?;
    let manifest: Manifest = serde_json::from_slice(&manifest_json)
        .map_err(|e| corrupt(&format!("invalid manifest: {e}")))?;
    if manifest.schema.major > SCHEMA_MAJOR {
        return Err(FileError::NewerVersion {
            major: manifest.schema.major as u16,
            minor: manifest.schema.minor as u16,
        });
    }

    // Images, each tile read once even when shared.
    let mut images = HashMap::new();
    let mut records = HashMap::new();
    for (key, dto) in &manifest.images {
        let key = Hash::from_key(key).ok_or_else(|| corrupt("invalid image key"))?;
        if dto.tile_size != TILE_SIZE {
            return Err(corrupt("unsupported tile size"));
        }
        let format = dto.source_format.to_format()?;
        let size = Size::new(dto.size[0], dto.size[1]);
        let levels = RasterImage::level_sizes(size);
        // Stored derived levels are used only when they come from our algorithm.
        let use_pyramid =
            dto.pyramid_algorithm == PYRAMID_ALGORITHM && dto.levels.len() == levels.len();
        let wanted = if use_pyramid { dto.levels.len() } else { 1 };
        let mut tables = Vec::with_capacity(wanted);
        for level in dto.levels.iter().take(wanted) {
            let table = Hash::from_key(&level.table).ok_or_else(|| corrupt("invalid table key"))?;
            let raw = read_blob(file, &index, table, Kind::Table)?;
            if raw.len() % 32 != 0 {
                return Err(corrupt("invalid tile table"));
            }
            let hashes: Vec<Hash> = raw.as_chunks::<32>().0.iter().map(|h| Hash(*h)).collect();
            tables.push((table, hashes));
        }
        if tables.is_empty() {
            return Err(corrupt("image without levels"));
        }
        images.insert(key, (size, format, tables, use_pyramid));
    }
    let wanted_tiles: Vec<Hash> = {
        let mut seen = HashSet::new();
        images
            .values()
            .flat_map(|(_, _, tables, _)| tables.iter().flat_map(|(_, hashes)| hashes))
            .filter(|hash| seen.insert(**hash))
            .copied()
            .collect()
    };
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let read = parallel_map(&wanted_tiles, threads, |hash| {
        read_blob(file, &index, *hash, Kind::Tile).map(|raw| (*hash, Arc::<[u8]>::from(raw)))
    });
    let mut tiles: HashMap<Hash, Arc<[u8]>> = HashMap::with_capacity(read.len());
    for result in read {
        let (hash, tile) = result?;
        tiles.insert(hash, tile);
    }

    let mut rasters: HashMap<Hash, Arc<RasterImage>> = HashMap::new();
    for (key, (size, format, tables, use_pyramid)) in images {
        let tile_bytes = RasterImage::tile_bytes(format);
        let level_tiles = |hashes: &[Hash]| -> Result<Vec<Arc<[u8]>>, FileError> {
            hashes
                .iter()
                .map(|hash| {
                    let tile = tiles.get(hash).ok_or_else(|| corrupt("missing tile"))?;
                    if tile.len() != tile_bytes {
                        return Err(corrupt("tile of the wrong size"));
                    }
                    Ok(tile.clone())
                })
                .collect()
        };
        let image = if use_pyramid {
            let levels = tables
                .iter()
                .map(|(_, hashes)| level_tiles(hashes))
                .collect::<Result<Vec<_>, _>>()?;
            RasterImage::from_tiles(size, format, levels)
        } else {
            RasterImage::from_level0_tiles(size, format, level_tiles(&tables[0].1)?)
        }
        .map_err(FileError::Image)?;
        if image_key(&image, tables[0].0) != key {
            return Err(corrupt("image key does not match the image"));
        }
        // What saves need to skip hashing again (only when every level came from the file).
        if use_pyramid {
            records.insert(
                image.id(),
                Arc::new(ImageRecord {
                    key,
                    levels: tables,
                }),
            );
        }
        rasters.insert(key, Arc::new(image));
    }

    // Nodes, in stack order.
    let doc = &manifest.document;
    let mut residue = Residue {
        manifest: manifest.extra.clone(),
        sections: manifest.sections.clone(),
        document: doc.extra.clone(),
        ..Residue::default()
    };
    let mut used = HashSet::new();
    let mut layers = Vec::with_capacity(doc.stack.len());
    for id in &doc.stack {
        layers.push(read_node(
            *id,
            0,
            &manifest,
            &rasters,
            &mut residue,
            &mut used,
        )?);
    }
    if used.len() != manifest.nodes.len() {
        return Err(corrupt("nodes outside the layer stack"));
    }
    for (key, dto) in &manifest.images {
        if !dto.extra.is_empty()
            && let Some(key) = Hash::from_key(key)
        {
            residue.images.insert(key, dto.extra.clone());
        }
    }
    let blend_space = document_blend_space(doc)?;
    let document = Document::restore(
        Size::new(doc.size[0], doc.size[1]),
        doc.working_space.to_space(),
        blend_space,
        layers,
        doc.next_node_id,
    )
    .and_then(|document| document.with_resolution(document_resolution(doc)))
    .map_err(FileError::Document)?;
    Ok((document, index, records, residue))
}

/// The layer of node `id` (`depth`: groups around it), a group with its children. Each node may
/// be used once: the stack and the groups form a tree.
pub(super) fn read_node(
    id: u64,
    depth: usize,
    manifest: &Manifest,
    rasters: &HashMap<Hash, Arc<RasterImage>>,
    residue: &mut Residue,
    used: &mut HashSet<u64>,
) -> Result<Layer, FileError> {
    let node = manifest
        .nodes
        .get(&id.to_string())
        .ok_or_else(|| corrupt("the stack refers to a missing node"))?;
    if !used.insert(id) {
        return Err(corrupt("a node is used twice"));
    }
    let versioned = || format!("{}@{}", node.kind, node.version);
    let known_version = (1..=NODE_VERSION_PAINTED).contains(&node.version);
    let content = match node.kind.as_str() {
        NODE_RASTER if known_version => {
            let key = node
                .params
                .get("image")
                .and_then(Value::as_str)
                .and_then(Hash::from_key)
                .ok_or_else(|| corrupt("raster node without an image"))?;
            let image = rasters
                .get(&key)
                .ok_or_else(|| corrupt("raster node with a missing image"))?;
            LayerContent::Raster {
                image: image.clone(),
                original: painted_original(node, &node.params, rasters)?,
            }
        }
        NODE_FILL if known_version => {
            let color = node
                .params
                .get("color")
                .and_then(Value::as_array)
                .filter(|c| c.len() == 4)
                .and_then(|c| {
                    c.iter()
                        .map(|v| v.as_f64().map(|v| v as f32))
                        .collect::<Option<Vec<f32>>>()
                })
                .ok_or_else(|| corrupt("fill node without a color"))?;
            LayerContent::Fill {
                color: LinearRgba::new(color[0], color[1], color[2], color[3]),
            }
        }
        NODE_GROUP if (3..=NODE_VERSION_PAINTED).contains(&node.version) => {
            // Checked before going deeper: the file is untrusted.
            if depth >= MAX_GROUP_DEPTH {
                return Err(corrupt("groups nested too deep"));
            }
            let pass_through = node
                .params
                .get("pass_through")
                .and_then(Value::as_bool)
                .ok_or_else(|| corrupt("group node without pass_through"))?;
            let children = node
                .inputs
                .iter()
                .map(|&child| read_node(child, depth + 1, manifest, rasters, residue, used))
                .collect::<Result<_, _>>()?;
            LayerContent::Group {
                children,
                pass_through,
            }
        }
        NODE_ADJUSTMENT if (3..=NODE_VERSION_PAINTED).contains(&node.version) => {
            let id = node
                .params
                .get("adjustment")
                .and_then(Value::as_str)
                .ok_or_else(|| corrupt("adjustment node without an adjustment"))?;
            let values: Vec<f32> = node
                .params
                .get("values")
                .and_then(Value::as_array)
                .and_then(|v| {
                    v.iter()
                        .map(|v| v.as_f64().map(|v| v as f32))
                        .collect::<Option<Vec<f32>>>()
                })
                .filter(|v| v.len() <= PARAM_COUNT)
                .ok_or_else(|| corrupt("adjustment node without its values"))?;
            // An adjustment this version does not know comes from a newer SlopShop.
            let mut adjustment = slopshop_core::adjust::Adjustment::from_params(id, &values)
                .ok_or_else(|| FileError::UnknownNodeType(format!("adjustment {id}")))?;
            if adjustment.curves().is_some() {
                adjustment = node_curves(node)?;
            }
            LayerContent::Adjustment { adjustment }
        }
        _ => return Err(FileError::UnknownNodeType(versioned())),
    };
    let blend_mode = node_blend_mode(node)?;
    let mask = node_mask(node, rasters)?;
    // Version 4 may be clipped (ADR 0016).
    let clipped = node.version >= NODE_VERSION_CLIPPED
        && node
            .params
            .get("clipped")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let transform = node_transform(node)?;
    if !node.extra.is_empty() {
        residue.nodes.insert(id, node.extra.clone());
    }
    Ok(Layer {
        transform,
        clipped,
        id: LayerId::from_raw(id),
        name: node.name.clone(),
        visible: node.visible,
        opacity: node.opacity,
        blend_mode,
        mask,
        content,
    })
}

/// A node's transform (version 5; the identity before): finite and invertible (ADR 0018).
fn node_transform(node: &NodeDto) -> Result<slopshop_core::Affine, FileError> {
    let values = match node.params.get("transform") {
        None | Some(Value::Null) => return Ok(slopshop_core::Affine::IDENTITY),
        Some(Value::Array(values)) if node.version >= NODE_VERSION_TRANSFORMED => values,
        Some(_) => return Err(corrupt("invalid transform")),
    };
    let numbers: Vec<f64> = values.iter().filter_map(Value::as_f64).collect();
    let array: [f64; 6] = numbers
        .try_into()
        .map_err(|_| corrupt("a transform has six numbers"))?;
    let transform = slopshop_core::Affine::from_array(array);
    if !transform.is_valid_layer_transform() {
        return Err(corrupt("invalid transform"));
    }
    Ok(transform)
}

/// Curves' points (schema 0.9): four lists of `[input, output]` on 0–255 (composite, red,
/// green, blue), each 2 to 16 points with increasing inputs.
fn node_curves(node: &NodeDto) -> Result<slopshop_core::adjust::Adjustment, FileError> {
    use slopshop_core::curve::Curve;
    let invalid = || corrupt("curves adjustment without valid curves");
    let lists = node
        .params
        .get("curves")
        .and_then(Value::as_array)
        .filter(|lists| lists.len() == 4)
        .ok_or_else(invalid)?;
    let mut curves = [Curve::IDENTITY; 4];
    for (curve, list) in curves.iter_mut().zip(lists) {
        let points = list
            .as_array()
            .and_then(|points| {
                points
                    .iter()
                    .map(|p| {
                        let pair = p.as_array().filter(|pair| pair.len() == 2)?;
                        let byte = |v: &Value| v.as_u64().and_then(|v| u8::try_from(v).ok());
                        Some([byte(&pair[0])?, byte(&pair[1])?])
                    })
                    .collect::<Option<Vec<[u8; 2]>>>()
            })
            .ok_or_else(invalid)?;
        *curve = Curve::new(&points).ok_or_else(invalid)?;
    }
    let [rgb, red, green, blue] = curves;
    Ok(slopshop_core::adjust::Adjustment::Curves {
        rgb,
        red,
        green,
        blue,
    })
}

/// A node's blend mode. Version 1 had none: normal. A mode this version does not know comes
/// from a newer SlopShop.
pub(super) fn node_blend_mode(node: &NodeDto) -> Result<BlendMode, FileError> {
    match node.params.get("blend_mode") {
        None if node.version == 1 => Ok(BlendMode::Normal),
        Some(Value::String(id)) => BlendMode::from_id(id)
            .ok_or_else(|| FileError::UnknownNodeType(format!("blend mode {id}"))),
        _ => Err(corrupt("node without a blend mode")),
    }
}

/// A node's mask (version 3; none before): `{ image, enabled, replaces_alpha }`.
fn node_mask(
    node: &NodeDto,
    rasters: &HashMap<Hash, Arc<RasterImage>>,
) -> Result<Option<LayerMask>, FileError> {
    let mask = match node.params.get("mask") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Object(mask)) if node.version >= 3 => mask,
        Some(_) => return Err(corrupt("invalid mask")),
    };
    let image = mask
        .get("image")
        .and_then(Value::as_str)
        .and_then(Hash::from_key)
        .and_then(|key| rasters.get(&key))
        .ok_or_else(|| corrupt("mask with a missing image"))?;
    let flag = |name: &str| {
        mask.get(name)
            .and_then(Value::as_bool)
            .ok_or_else(|| corrupt("mask without its flags"))
    };
    Ok(Some(LayerMask {
        image: image.clone(),
        enabled: flag("enabled")?,
        replaces_alpha: flag("replaces_alpha")?,
        original: painted_original(node, mask, rasters)?,
    }))
}

/// The unpainted image named by `params.original` (version 6, ADR 0027), if any.
fn painted_original(
    node: &NodeDto,
    params: &Map<String, Value>,
    rasters: &HashMap<Hash, Arc<RasterImage>>,
) -> Result<Option<Arc<RasterImage>>, FileError> {
    match params.get("original") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(key)) if node.version >= NODE_VERSION_PAINTED => {
            Hash::from_key(key.as_str())
                .and_then(|key| rasters.get(&key))
                .cloned()
                .map(Some)
                .ok_or_else(|| corrupt("paint with a missing original"))
        }
        Some(_) => Err(corrupt("invalid original")),
    }
}

/// The document's blend space. Schema 0.1 documents composited in linear light.
pub(super) fn document_blend_space(doc: &DocumentDto) -> Result<BlendSpace, FileError> {
    match &doc.blend_space {
        None => Ok(BlendSpace::Linear),
        Some(id) => BlendSpace::from_id(id)
            .ok_or_else(|| FileError::UnknownNodeType(format!("blend space {id}"))),
    }
}

/// The document's resolution, pixels per inch; 72 before schema 0.11 (ADR 0028). Out of range,
/// it is refused when the document is restored.
pub(super) fn document_resolution(doc: &DocumentDto) -> f64 {
    doc.resolution
        .unwrap_or(slopshop_core::document::DEFAULT_RESOLUTION)
}

/// The raw payload of the record a slot refers to, checked against its kind and hash.
fn read_record(file: &Source<'_>, record: &RecordRef, kind: Kind) -> Result<Vec<u8>, FileError> {
    let len = usize::try_from(record.len).map_err(|_| corrupt("record too large"))?;
    let bytes = read_at(file, record.offset, RECORD_HEADER_LEN + len)?;
    let header = RecordHeader::decode(&bytes)?;
    if header.kind != kind || header.stored_len != record.len || header.hash != record.hash {
        return Err(corrupt("a commit slot does not match its record"));
    }
    let raw = decode_blob(
        &bytes[RECORD_HEADER_LEN..],
        header.encoding,
        header.raw_len,
        kind,
    )?;
    if Hash::of(&raw) != record.hash {
        return Err(corrupt("a record does not match its hash"));
    }
    Ok(raw)
}

/// The raw bytes of an indexed blob, checked against its kind, header and hash.
fn read_blob(
    file: &Source<'_>,
    index: &HashMap<Hash, IndexEntry>,
    hash: Hash,
    kind: Kind,
) -> Result<Vec<u8>, FileError> {
    let entry = index
        .get(&hash)
        .filter(|entry| entry.kind == kind)
        .ok_or_else(|| corrupt("a blob is missing from the index"))?;
    let len = usize::try_from(entry.stored_len).map_err(|_| corrupt("blob too large"))?;
    let bytes = read_at(file, entry.offset, RECORD_HEADER_LEN + len)?;
    let header = RecordHeader::decode(&bytes)?;
    if header.kind != kind
        || header.hash != hash
        || header.stored_len != entry.stored_len
        || header.raw_len != entry.raw_len
        || header.encoding != entry.encoding
    {
        return Err(corrupt("a blob does not match its index entry"));
    }
    let raw = decode_blob(
        &bytes[RECORD_HEADER_LEN..],
        header.encoding,
        header.raw_len,
        kind,
    )?;
    // The integrity check: no CRC needed on top of the content address.
    if Hash::of(&raw) != hash {
        return Err(corrupt("a blob does not match its hash"));
    }
    Ok(raw)
}

/// `len` bytes at `offset`, from any thread (positional read: no shared cursor). A short file
/// is corruption.
fn read_at(file: &Source<'_>, offset: u64, len: usize) -> Result<Vec<u8>, FileError> {
    // Lengths come from the file: check them before allocating.
    if offset
        .checked_add(len as u64)
        .is_none_or(|end| end > file.len)
    {
        return Err(corrupt("a record goes past the end of the file"));
    }
    let mut buffer = vec![0u8; len];
    let mut done = 0;
    while done < len {
        let read = positional_read(file, &mut buffer[done..], offset + done as u64)?;
        if read == 0 {
            return Err(corrupt("unexpected end of file"));
        }
        done += read;
    }
    Ok(buffer)
}

#[cfg(windows)]
fn positional_read(file: &Source<'_>, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file.file, buffer, offset)
}

#[cfg(unix)]
fn positional_read(file: &Source<'_>, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file.file, buffer, offset)
}
