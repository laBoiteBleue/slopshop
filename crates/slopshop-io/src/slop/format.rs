//! The byte layout of `.slop` files (ADR 0009, `docs/file-format.md`): header, commit slots,
//! record framing, the binary index, blob filters and codecs. Pure functions over bytes; the
//! reader and writer do the I/O.
//!
//! Everything is little-endian, offsets and lengths are 64-bit.
//!
//! ```text
//! 0     magic 89 53 4C 50 0D 0A 1A 0A
//! 8     major u16, minor u16, header_len u32 (4096)
//! 16    incompat u64, ro_compat u64, compat u64
//! 1024  commit slot 0 (136 bytes)
//! 2048  commit slot 1 (136 bytes)
//! 4096  records, each 8-byte aligned: 56-byte header, payload, zero padding
//! ```

use std::collections::HashMap;
use std::fmt;

use super::FileError;

pub(crate) const MAGIC: [u8; 8] = [0x89, b'S', b'L', b'P', 0x0D, 0x0A, 0x1A, 0x0A];
pub(crate) const MAJOR: u16 = 0;
pub(crate) const MINOR: u16 = 1;
pub(crate) const HEADER_LEN: u64 = 4096;
/// Offsets of the two commit slots. Generation `g` is written to slot `g % 2`, so the previous
/// generation stays in the other one.
pub(crate) const SLOT_OFFSETS: [u64; 2] = [1024, 2048];
pub(crate) const SLOT_LEN: usize = 136;
pub(crate) const RECORD_HEADER_LEN: usize = 56;
/// Records start on multiples of this.
pub(crate) const RECORD_ALIGN: u64 = 8;
/// Bytes of one index entry.
pub(crate) const INDEX_ENTRY_LEN: usize = 64;
/// A blob is stored compressed only when that saves at least this fraction.
const MIN_GAIN: f64 = 0.05;

/// BLAKE3-256 of raw (decoded, unfiltered) bytes: the identity of every blob.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Hash(pub [u8; 32]);

impl Hash {
    pub(crate) fn of(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// `b3:` and 64 hex digits, as the manifest writes blob references.
    pub(crate) fn to_key(self) -> String {
        let mut key = String::with_capacity(67);
        key.push_str("b3:");
        for byte in self.0 {
            key.push_str(&format!("{byte:02x}"));
        }
        key
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        let hex = key.strip_prefix("b3:")?;
        if hex.len() != 64 || !hex.is_ascii() {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(Self(bytes))
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_key())
    }
}

/// Feature flags this version understands (none yet).
pub(crate) const KNOWN_INCOMPAT: u64 = 0;
pub(crate) const KNOWN_RO_COMPAT: u64 = 0;

/// The fixed part of the header (the first 40 bytes; the rest up to the slots is zero).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Header {
    pub major: u16,
    pub minor: u16,
    pub incompat: u64,
    pub ro_compat: u64,
    pub compat: u64,
}

impl Header {
    pub(crate) fn current() -> Self {
        Self {
            major: MAJOR,
            minor: MINOR,
            incompat: 0,
            ro_compat: 0,
            compat: 0,
        }
    }

    /// The whole header area (4096 bytes), with both slots empty.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut bytes = vec![0u8; HEADER_LEN as usize];
        bytes[..8].copy_from_slice(&MAGIC);
        bytes[8..10].copy_from_slice(&self.major.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.minor.to_le_bytes());
        bytes[12..16].copy_from_slice(&(HEADER_LEN as u32).to_le_bytes());
        bytes[16..24].copy_from_slice(&self.incompat.to_le_bytes());
        bytes[24..32].copy_from_slice(&self.ro_compat.to_le_bytes());
        bytes[32..40].copy_from_slice(&self.compat.to_le_bytes());
        bytes
    }

    /// Parse and check the header: `NotASlopFile` without the magic, `NewerVersion` for a
    /// newer major version, `UnsupportedFeatures` for unknown incompatible features.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, FileError> {
        if bytes.len() < 40 || bytes[..8] != MAGIC {
            return Err(FileError::NotASlopFile);
        }
        let header = Self {
            major: u16_at(bytes, 8),
            minor: u16_at(bytes, 10),
            incompat: u64_at(bytes, 16),
            ro_compat: u64_at(bytes, 24),
            compat: u64_at(bytes, 32),
        };
        if header.major > MAJOR {
            return Err(FileError::NewerVersion {
                major: header.major,
                minor: header.minor,
            });
        }
        if u32_at(bytes, 12) as u64 != HEADER_LEN {
            return Err(FileError::Corrupt("unexpected header length".to_owned()));
        }
        if header.incompat & !KNOWN_INCOMPAT != 0 {
            return Err(FileError::UnsupportedFeatures(header.incompat));
        }
        Ok(header)
    }

    /// Unknown read-only-compatible features: the file can be read, not saved.
    pub(crate) fn read_only(&self) -> bool {
        self.ro_compat & !KNOWN_RO_COMPAT != 0
    }
}

/// Where a record is and what it holds, as a commit slot refers to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RecordRef {
    /// Offset of the record header.
    pub offset: u64,
    /// Length of the stored payload.
    pub len: u64,
    /// Hash of the raw payload.
    pub hash: Hash,
}

/// One generation of the file: what a reader opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Slot {
    pub generation: u64,
    pub manifest: RecordRef,
    pub index: RecordRef,
    /// Everything after this is junk from an interrupted save.
    pub committed_len: u64,
    /// Version of the writer (major, minor, patch, 0).
    pub writer: [u16; 4],
}

impl Slot {
    pub(crate) fn encode(&self) -> [u8; SLOT_LEN] {
        let mut bytes = [0u8; SLOT_LEN];
        bytes[0..8].copy_from_slice(&self.generation.to_le_bytes());
        let mut at = 8;
        for record in [self.manifest, self.index] {
            bytes[at..at + 8].copy_from_slice(&record.offset.to_le_bytes());
            bytes[at + 8..at + 16].copy_from_slice(&record.len.to_le_bytes());
            bytes[at + 16..at + 48].copy_from_slice(&record.hash.0);
            at += 48;
        }
        bytes[104..112].copy_from_slice(&self.committed_len.to_le_bytes());
        for (i, part) in self.writer.iter().enumerate() {
            bytes[112 + i * 2..114 + i * 2].copy_from_slice(&part.to_le_bytes());
        }
        let check = blake3::hash(&bytes[..120]);
        bytes[120..].copy_from_slice(&check.as_bytes()[..16]);
        bytes
    }

    /// A slot, or `None` when it is empty or fails its checksum (torn by a crash).
    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < SLOT_LEN || bytes[..SLOT_LEN].iter().all(|&b| b == 0) {
            return None;
        }
        let check = blake3::hash(&bytes[..120]);
        if bytes[120..SLOT_LEN] != check.as_bytes()[..16] {
            return None;
        }
        let record = |at: usize| RecordRef {
            offset: u64_at(bytes, at),
            len: u64_at(bytes, at + 8),
            hash: Hash(bytes[at + 16..at + 48].try_into().unwrap_or([0; 32])),
        };
        Some(Self {
            generation: u64_at(bytes, 0),
            manifest: record(8),
            index: record(56),
            committed_len: u64_at(bytes, 104),
            writer: [0, 1, 2, 3].map(|i| u16_at(bytes, 112 + i * 2)),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum Kind {
    Tile = 1,
    Table = 2,
    Manifest = 3,
    Index = 4,
    /// A copy of the commit slot, for recovery when both slots are damaged.
    Commit = 5,
}

impl Kind {
    fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            1 => Kind::Tile,
            2 => Kind::Table,
            3 => Kind::Manifest,
            4 => Kind::Index,
            5 => Kind::Commit,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum Codec {
    Raw = 0,
    Zstd = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub(crate) enum Filter {
    None = 0,
    /// Byte planes: every byte `k` of each element, then every byte `k + 1`… (element size in
    /// the record's `stride`).
    Shuffle = 1,
    /// [`Filter::Shuffle`], then each plane replaced by the differences of consecutive bytes
    /// (wrapping), its first byte kept.
    ShuffleDelta = 2,
}

/// How a blob is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Encoding {
    pub codec: Codec,
    pub filter: Filter,
    /// Element size of the filter, in bytes (the pixel size for tiles); 0 without filter.
    pub stride: u8,
}

impl Encoding {
    pub(crate) const RAW: Encoding = Encoding {
        codec: Codec::Raw,
        filter: Filter::None,
        stride: 0,
    };
}

/// The 56-byte header in front of every record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RecordHeader {
    pub kind: Kind,
    pub encoding: Encoding,
    pub stored_len: u64,
    pub raw_len: u64,
    pub hash: Hash,
}

impl RecordHeader {
    pub(crate) fn encode(&self) -> [u8; RECORD_HEADER_LEN] {
        let mut bytes = [0u8; RECORD_HEADER_LEN];
        bytes[0] = self.kind as u8;
        bytes[1] = self.encoding.codec as u8;
        bytes[2] = self.encoding.filter as u8;
        bytes[3] = self.encoding.stride;
        bytes[8..16].copy_from_slice(&self.stored_len.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.raw_len.to_le_bytes());
        bytes[24..56].copy_from_slice(&self.hash.0);
        bytes
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, FileError> {
        if bytes.len() < RECORD_HEADER_LEN {
            return Err(corrupt("truncated record header"));
        }
        Ok(Self {
            kind: Kind::from_byte(bytes[0]).ok_or_else(|| corrupt("unknown record kind"))?,
            encoding: decode_encoding(bytes[1], bytes[2], bytes[3])?,
            stored_len: u64_at(bytes, 8),
            raw_len: u64_at(bytes, 16),
            hash: Hash(bytes[24..56].try_into().unwrap_or([0; 32])),
        })
    }
}

fn decode_encoding(codec: u8, filter: u8, stride: u8) -> Result<Encoding, FileError> {
    let codec = match codec {
        0 => Codec::Raw,
        1 => Codec::Zstd,
        _ => return Err(corrupt("unknown codec")),
    };
    let filter = match filter {
        0 => Filter::None,
        1 => Filter::Shuffle,
        2 => Filter::ShuffleDelta,
        _ => return Err(corrupt("unknown filter")),
    };
    if filter != Filter::None && stride == 0 {
        return Err(corrupt("filter without an element size"));
    }
    Ok(Encoding {
        codec,
        filter,
        stride,
    })
}

/// The largest raw payload a record of `kind` can have: lengths read from a damaged file are
/// checked against it before anything is allocated.
pub(crate) fn max_raw_len(kind: Kind) -> u64 {
    match kind {
        // 256 × 256 pixels of 16 bytes (RGBA f32).
        Kind::Tile => 1 << 20,
        Kind::Table => 64 << 20,
        Kind::Manifest => 256 << 20,
        Kind::Index => 1 << 30,
        Kind::Commit => SLOT_LEN as u64,
    }
}

/// Bytes a record takes in the file, padding included.
pub(crate) fn record_span(stored_len: u64) -> u64 {
    (RECORD_HEADER_LEN as u64 + stored_len).next_multiple_of(RECORD_ALIGN)
}

/// Where a blob (tile or table) is, as the index lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct IndexEntry {
    pub kind: Kind,
    /// Offset of the record header.
    pub offset: u64,
    pub stored_len: u64,
    pub raw_len: u64,
    pub encoding: Encoding,
}

/// The index payload: entry count, then entries sorted by hash.
pub(crate) fn encode_index(entries: &HashMap<Hash, IndexEntry>) -> Vec<u8> {
    let mut sorted: Vec<(&Hash, &IndexEntry)> = entries.iter().collect();
    sorted.sort_by_key(|(hash, _)| **hash);
    let mut bytes = Vec::with_capacity(8 + sorted.len() * INDEX_ENTRY_LEN);
    bytes.extend_from_slice(&(sorted.len() as u64).to_le_bytes());
    for (hash, entry) in sorted {
        bytes.extend_from_slice(&hash.0);
        bytes.extend_from_slice(&entry.offset.to_le_bytes());
        bytes.extend_from_slice(&entry.stored_len.to_le_bytes());
        bytes.extend_from_slice(&entry.raw_len.to_le_bytes());
        bytes.extend_from_slice(&[
            entry.kind as u8,
            entry.encoding.codec as u8,
            entry.encoding.filter as u8,
            entry.encoding.stride,
            0,
            0,
            0,
            0,
        ]);
    }
    bytes
}

pub(crate) fn decode_index(bytes: &[u8]) -> Result<HashMap<Hash, IndexEntry>, FileError> {
    if bytes.len() < 8 {
        return Err(corrupt("truncated index"));
    }
    let count = u64_at(bytes, 0);
    let expected = count
        .checked_mul(INDEX_ENTRY_LEN as u64)
        .and_then(|len| len.checked_add(8))
        .ok_or_else(|| corrupt("index too large"))?;
    if bytes.len() as u64 != expected {
        return Err(corrupt("index length does not match its count"));
    }
    let mut entries = HashMap::with_capacity(count as usize);
    for chunk in bytes[8..].as_chunks::<INDEX_ENTRY_LEN>().0 {
        let hash = Hash(chunk[..32].try_into().unwrap_or([0; 32]));
        let entry = IndexEntry {
            offset: u64_at(chunk, 32),
            stored_len: u64_at(chunk, 40),
            raw_len: u64_at(chunk, 48),
            kind: Kind::from_byte(chunk[56]).ok_or_else(|| corrupt("unknown kind in index"))?,
            encoding: decode_encoding(chunk[57], chunk[58], chunk[59])?,
        };
        entries.insert(hash, entry);
    }
    Ok(entries)
}

/// `raw` filtered and compressed as `wanted` asks, or stored raw when compression gains less
/// than 5 %: the stored bytes and how they are encoded.
pub(crate) fn encode_blob(raw: &[u8], wanted: Encoding, level: i32) -> (Vec<u8>, Encoding) {
    if wanted.codec == Codec::Raw || raw.is_empty() {
        return (raw.to_vec(), Encoding::RAW);
    }
    let filtered = apply_filter(raw, wanted.filter, wanted.stride);
    match zstd::bulk::compress(&filtered, level) {
        Ok(compressed) if (compressed.len() as f64) <= raw.len() as f64 * (1.0 - MIN_GAIN) => {
            (compressed, wanted)
        }
        _ => (raw.to_vec(), Encoding::RAW),
    }
}

/// The raw bytes of a stored blob, checked against its length.
pub(crate) fn decode_blob(
    stored: &[u8],
    encoding: Encoding,
    raw_len: u64,
    kind: Kind,
) -> Result<Vec<u8>, FileError> {
    // Never trust a length read from the file with an allocation.
    if raw_len > max_raw_len(kind) {
        return Err(corrupt("blob length out of range"));
    }
    let raw_len = usize::try_from(raw_len).map_err(|_| corrupt("blob too large"))?;
    let decoded = match encoding.codec {
        Codec::Raw => stored.to_vec(),
        Codec::Zstd => zstd::bulk::decompress(stored, raw_len)
            .map_err(|e| corrupt(&format!("cannot decompress a blob: {e}")))?,
    };
    if decoded.len() != raw_len {
        return Err(corrupt("blob length does not match its header"));
    }
    Ok(unapply_filter(decoded, encoding.filter, encoding.stride))
}

fn apply_filter(raw: &[u8], filter: Filter, stride: u8) -> Vec<u8> {
    let stride = stride as usize;
    if filter == Filter::None || stride == 0 || stride <= 1 && filter == Filter::Shuffle {
        return raw.to_vec();
    }
    let mut out = shuffle(raw, stride);
    if filter == Filter::ShuffleDelta {
        let n = raw.len() / stride;
        for plane in out.chunks_mut(n.max(1)) {
            for i in (1..plane.len()).rev() {
                plane[i] = plane[i].wrapping_sub(plane[i - 1]);
            }
        }
    }
    out
}

fn unapply_filter(mut data: Vec<u8>, filter: Filter, stride: u8) -> Vec<u8> {
    let stride = stride as usize;
    if filter == Filter::None || stride == 0 || stride <= 1 && filter == Filter::Shuffle {
        return data;
    }
    if filter == Filter::ShuffleDelta {
        let n = data.len() / stride;
        for plane in data.chunks_mut(n.max(1)) {
            for i in 1..plane.len() {
                plane[i] = plane[i].wrapping_add(plane[i - 1]);
            }
        }
    }
    unshuffle(&data, stride)
}

/// Byte planes of `stride`-byte elements; a tail shorter than an element is kept as is.
fn shuffle(raw: &[u8], stride: usize) -> Vec<u8> {
    let n = raw.len() / stride;
    let mut out = vec![0u8; raw.len()];
    for (i, element) in raw.chunks_exact(stride).enumerate() {
        for (k, &byte) in element.iter().enumerate() {
            out[k * n + i] = byte;
        }
    }
    out[n * stride..].copy_from_slice(&raw[n * stride..]);
    out
}

fn unshuffle(planes: &[u8], stride: usize) -> Vec<u8> {
    let n = planes.len() / stride;
    let mut out = vec![0u8; planes.len()];
    for (i, element) in out.chunks_exact_mut(stride).enumerate() {
        for (k, byte) in element.iter_mut().enumerate() {
            *byte = planes[k * n + i];
        }
    }
    out[n * stride..].copy_from_slice(&planes[n * stride..]);
    out
}

pub(crate) fn corrupt(what: &str) -> FileError {
    FileError::Corrupt(what.to_owned())
}

pub(crate) fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

pub(crate) fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

pub(crate) fn u64_at(bytes: &[u8], at: usize) -> u64 {
    let mut value = [0u8; 8];
    value.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_round_trip_every_byte() {
        // Includes NaN payloads and infinities as float bit patterns, and a partial element.
        let mut raw: Vec<u8> = (0..4099u32).map(|i| (i * 7919 % 251) as u8).collect();
        raw.extend_from_slice(&f32::NAN.to_bits().to_le_bytes());
        raw.extend_from_slice(&f32::INFINITY.to_le_bytes());
        raw.extend_from_slice(&(-0.0f32).to_le_bytes());
        for filter in [Filter::None, Filter::Shuffle, Filter::ShuffleDelta] {
            for stride in [1u8, 2, 3, 4, 8, 16] {
                let filtered = apply_filter(&raw, filter, stride);
                assert_eq!(filtered.len(), raw.len());
                assert_eq!(
                    unapply_filter(filtered, filter, stride),
                    raw,
                    "{filter:?} stride {stride}"
                );
            }
        }
    }

    #[test]
    fn blobs_round_trip_and_incompressible_ones_stay_raw() {
        let smooth: Vec<u8> = (0..65536u32).map(|i| (i / 256) as u8).collect();
        let wanted = Encoding {
            codec: Codec::Zstd,
            filter: Filter::ShuffleDelta,
            stride: 4,
        };
        let (stored, encoding) = encode_blob(&smooth, wanted, 1);
        assert_eq!(encoding, wanted);
        assert!(stored.len() < smooth.len() / 10);
        assert_eq!(
            decode_blob(&stored, encoding, smooth.len() as u64, Kind::Table).unwrap(),
            smooth
        );

        let noise: Vec<u8> = (0..4096u32)
            .map(|i| {
                *blake3::hash(&i.to_le_bytes())
                    .as_bytes()
                    .first()
                    .unwrap_or(&0)
            })
            .collect();
        let (stored, encoding) = encode_blob(&noise, wanted, 1);
        assert_eq!(encoding, Encoding::RAW);
        assert_eq!(stored, noise);
        // A wrong length is corruption, not a panic.
        assert!(decode_blob(&stored, encoding, 10, Kind::Table).is_err());
        assert!(decode_blob(b"not zstd", wanted, 100, Kind::Table).is_err());
        // A huge length from a damaged header is refused before any allocation.
        assert!(decode_blob(b"x", wanted, u64::MAX / 3, Kind::Tile).is_err());
    }

    #[test]
    fn slots_detect_torn_writes() {
        let slot = Slot {
            generation: 7,
            manifest: RecordRef {
                offset: 4096,
                len: 100,
                hash: Hash::of(b"manifest"),
            },
            index: RecordRef {
                offset: 8192,
                len: 64,
                hash: Hash::of(b"index"),
            },
            committed_len: 9000,
            writer: [0, 1, 0, 0],
        };
        let bytes = slot.encode();
        assert_eq!(Slot::decode(&bytes), Some(slot));
        for at in [0, 50, 119, 125] {
            let mut torn = bytes;
            torn[at] ^= 1;
            assert_eq!(Slot::decode(&torn), None, "byte {at}");
        }
        assert_eq!(Slot::decode(&[0u8; SLOT_LEN]), None);
    }

    #[test]
    fn header_checks_magic_version_and_features() {
        let bytes = Header::current().encode();
        assert_eq!(bytes.len(), HEADER_LEN as usize);
        assert_eq!(Header::decode(&bytes).unwrap(), Header::current());
        assert!(matches!(
            Header::decode(b"\x89PNG\r\n\x1a\n and more bytes than forty"),
            Err(FileError::NotASlopFile)
        ));
        let mut newer = bytes.clone();
        newer[8] = 1;
        assert!(matches!(
            Header::decode(&newer),
            Err(FileError::NewerVersion { major: 1, .. })
        ));
        let mut feature = bytes.clone();
        feature[16] = 1;
        assert!(matches!(
            Header::decode(&feature),
            Err(FileError::UnsupportedFeatures(1))
        ));
        let mut read_only = bytes;
        read_only[24] = 1;
        assert!(Header::decode(&read_only).unwrap().read_only());
    }

    #[test]
    fn index_and_record_headers_round_trip() {
        let mut entries = HashMap::new();
        for i in 0..5u8 {
            entries.insert(
                Hash::of(&[i]),
                IndexEntry {
                    kind: if i % 2 == 0 { Kind::Tile } else { Kind::Table },
                    offset: 4096 + u64::from(i) * 100,
                    stored_len: 90,
                    raw_len: 262_144,
                    encoding: Encoding {
                        codec: Codec::Zstd,
                        filter: Filter::Shuffle,
                        stride: 8,
                    },
                },
            );
        }
        let bytes = encode_index(&entries);
        assert_eq!(bytes.len(), 8 + 5 * INDEX_ENTRY_LEN);
        assert_eq!(decode_index(&bytes).unwrap(), entries);
        assert!(decode_index(&bytes[..bytes.len() - 1]).is_err());

        let header = RecordHeader {
            kind: Kind::Manifest,
            encoding: Encoding::RAW,
            stored_len: 1234,
            raw_len: 1234,
            hash: Hash::of(b"x"),
        };
        assert_eq!(RecordHeader::decode(&header.encode()).unwrap(), header);
        assert_eq!(record_span(1), 64);
        assert_eq!(record_span(8), 64);
        assert_eq!(record_span(9), 72);
    }

    #[test]
    fn keys_round_trip() {
        let hash = Hash::of(b"tile");
        let key = hash.to_key();
        assert!(key.starts_with("b3:") && key.len() == 67);
        assert_eq!(Hash::from_key(&key), Some(hash));
        assert_eq!(Hash::from_key("b3:zz"), None);
        assert_eq!(Hash::from_key("md5:00"), None);
    }
}
