# 0009 — Document file format v0

Status: accepted (2026-09-29), not implemented yet. Research:
[document-format.md](../research/document-format.md).

## Context

SlopShop needs save/load (roadmap Phase 1, 🔶). Documents reach hundreds of megapixels in
8/16-bit integer and 16/32-bit float, i.e. 1–5 GB of level-0 tiles in memory (ADR 0005), plus a
derived display pyramid (+35 % of level 0, measured on the 233 MP test image). Pixels are
immutable 256 px tiles in the source format, shared through `Arc` by layers, snapshots and undo
(ADR 0003, 0005). Colour spaces are explicit per raster (ADR 0007). The model must grow from a
layer stack into a DAG with AI nodes whose results are expensive and not reliably reproducible.
Saves must be fast, never leave the user's file half-written, and never assume the document fits
in RAM. `slopshop-core` stays dependency-free.

Prior art (research notes, 2026-09-29):
- Zip-of-layers formats (ORA, KRA, Procreate) rewrite the whole file on every save.
- Monolithic formats (PSD, XCF, .blend) all had to break for 32-bit size fields.
- Serial compression dominates Photoshop save times (about 20x slower).
- Formats that dump in-memory structures age badly (Blender DNA, Natron/Boost).
- What works: a versioned manifest plus immutable chunks with an index and an atomic commit
  (Zarr, git, Adobe DCX, LMDB/redb commit slots).

Measured on the 233 MP test image (reference machine, NVMe):
- zstd-1 per tile: 0.19–0.25 of raw size for RGBA8.
- BLAKE3 over all tiles: 15 ms per GB.
- Durable append: about 1.9 GB/s.
- Metadata-only commit: about 0.5 ms.
- Pyramid rebuild: 1.0 s (RGBA8) to 2.3 s (RGBA32F), and it needs all of level 0.

## Decision

Maintainer decisions (2026-09-29): incremental in-place saves (1–2 below), zstd despite being C
code (4), extension `.slop` (1), every pyramid level stored (5), newer files refused before 1.0
(7), AI results kept in the document (9), no undo history or view state in v0 (8).

1. **Container: one file, append-only inside**, little-endian, 64-bit offsets and lengths
   everywhere. Extension `.slop`, media type `application/vnd.slopshop.document` (unregistered
   for now; no `x-` prefix, per RFC 6648). Open detects the file by its magic, not its extension.

   | Offset | Content |
   |---|---|
   | 0 | magic `89 53 4C 50 0D 0A 1A 0A` (PNG style: catches text-mode and 7-bit damage) |
   | 8 | `major u16`, `minor u16`, `header_len u32` (4096) |
   | 16 | `incompat u64`, `ro_compat u64`, `compat u64` feature flags (ext4 model): an unknown incompat flag means refuse, an unknown ro_compat flag means open read-only, an unknown compat flag is ignored |
   | 1024, 2048 | two commit slots: `generation u64, manifest {offset, len, hash}, index {offset, len, hash}, committed_len u64, writer version`, then BLAKE3-128 of the slot |
   | 4096… | append-only records |

   A **record** is a 56-byte header followed by the payload. The header holds `kind` (tile,
   table, manifest, index, commit, metadata), `codec`, `filter`, `flags`, `stored_len u64`,
   `raw_len u64` and `hash [32]`. Records are 8-byte aligned. 4 KiB alignment was rejected: it
   costs about 4 % on RGBA8 and would only help mmap or direct I/O, which v0 does not use.

2. **Commit protocol (Save).**
   - Take a snapshot: `end_gesture`, then `Document::clone`. No pixels are copied.
   - On worker threads (`std::thread::scope`), hash the tiles. Hashes are cached per `ImageId`,
     which is valid because images are immutable and the id is unique in the process.
   - Compress only the blobs the file lacks, and append them after `committed_len`. Anything
     beyond `committed_len` (left by a crash) is truncated first.
   - Append the tables, the manifest, the index and a commit record, then `sync_data`.
   - Write the *inactive* slot with `generation + 1`, then `sync_data` again.
   - Open picks the valid slot with the higher generation. A torn slot fails its checksum and
     the other slot is used. The commit record duplicated in the log allows recovery even if
     both slots are damaged.
   - Concurrent writers are handled optimistically, without OS locks: before committing, the
     header is read again, and if its generation changed since open, the save fails and
     suggests Save As.

3. **Full write (Save As, compaction, first save).** Write a compact file to
   `<name>.slopshop-tmp` in the destination directory, fsync it, rename it over the
   destination, then fsync the parent directory on Unix. The helper is shared with export and
   also fixes export's missing directory fsync. Compressed blobs are copied verbatim.
   Compaction runs on Save when dead bytes exceed 50 % of the file (threshold provisional), or
   on explicit request.

4. **Tiles.**
   - Stored exactly as in memory: the stored format (RGB stored as RGBA with opaque alpha), 256
     px, edge padding included, little-endian samples. Loading is decompress plus copy, and it
     is bit-exact for every value, NaN payloads and infinities included.
   - Identity is **BLAKE3-256 of the raw, unfiltered tile bytes**. The key does not depend on
     codec or filter, and deduplication across layers, images and saves is automatic.
     Identical uniform or transparent tiles are stored once, so no separate "fill value"
     mechanism is needed.
   - On load, the decompressed bytes are re-hashed and checked against the key: this is the
     integrity check, and no CRC is needed.
   - Each image level has a **tile table** (row-major list of tile hashes), stored as a blob
     and therefore content-addressed too.
   - The **image key** is BLAKE3 over the canonical format descriptor, the size, the tile size
     and the level-0 table hash. It persists across sessions and will serve as the AI cache
     key input.
   - Codec and filter are recorded per blob, so defaults can change without a format break.
     v0 defaults (provisional until tested on real 16-bit and HDR data):

     | Sample type | Filter | Codec |
     |---|---|---|
     | U8 | byte-plane shuffle + left delta | zstd-1 |
     | U16 | shuffle + delta | zstd-1 |
     | F16, F32 | shuffle only (delta hurts floats) | zstd-1 |

     A blob is stored raw when compression gains less than 5 %. zstd is C code (libzstd,
     BSD-3-Clause, through the `zstd` crate): an accepted exception to "no C code in
     slopshop-io", for its ratio and speed (pure-Rust lz4_flex is ~40 % larger on 8-bit tiles).
     The codec is recorded per blob, so it can change without a format break. Filters are
     defined byte for byte in the spec.

5. **Pyramid.** Levels 1 and up are stored as **derived blobs**, tagged with the algorithm id
   `slopshop.pyramid.box-linear-premul@1`. When they are missing or the tag is unknown, they
   are rebuilt, tile by tile from 2×2 children if a test proves that matches `from_pixels`
   exactly. A "strip derived data" option exists.

6. **Manifest.** JSON (serde_json), compressed with zstd, written through explicit DTOs in
   `slopshop-io`, never serde derives on core types. Sketch:

   ```json
   {
     "schema": { "major": 0, "minor": 1 },
     "writer": { "app": "slopshop", "version": "0.1.0" },
     "document": {
       "size": [21600, 10800],
       "working_space": { "primaries": { "r": [0.708, 0.292], "g": [...], "b": [...], "w": [...] },
                          "transfer": { "kind": "linear" }, "id_hint": "rec2020-linear" },
       "next_node_id": 6,
       "stack": [3, 5]
     },
     "nodes": {
       "3": { "type": "slopshop.raster", "version": 1, "name": "Background", "visible": true,
              "opacity": 1.0, "params": { "image": "b3:9f2c…" }, "inputs": [] },
       "5": { "type": "slopshop.fill", "version": 1, "name": "Tint", "visible": true,
              "opacity": 0.5, "params": { "color": [0.2, 0.1, 0.0, 1.0] }, "inputs": [] }
     },
     "images": {
       "b3:9f2c…": {
         "size": [21600, 10800], "tile_size": 256,
         "source_format": { "layout": "rgb", "sample": "u8", "alpha": "straight",
                            "color_space": { "...": "..." } },
         "stored_format": { "layout": "rgba", "sample": "u8", "alpha": "straight" },
         "levels": [ { "table": "b3:…" }, { "table": "b3:…", "derived": true } ],
         "pyramid_algorithm": "slopshop.pyramid.box-linear-premul@1",
         "warnings": ["iccCurveApproximated"],
         "metadata": { }
       }
     },
     "sections": { }
   }
   ```

   - Nodes are keyed by stable id, which is today's `LayerId`. `next_node_id` is today's
     `next_layer_id` and is always saved, so ids are never reused after a reload.
   - Each node has a type name plus a per-type parameter version, upgraded step by step on
     load (the darktable and Blender pattern).
   - The stack is a document-level ordered list, bottom to top. The DAG step later turns it
     into a root node's inputs, as a schema migration.
   - Colour spaces are stored as full numbers; `id_hint` is only a hint.
   - `revision` and `ImageId` are never stored.
   - `metadata` is reserved for verbatim ICC, EXIF and XMP blob references (ADR 0006 decision
     2) and stays empty until import keeps them.
   - The index (hash → offset, length, codec, filter, raw length) is a fixed little-endian
     binary record, not JSON.

7. **Compatibility.**
   - Readers accept every older schema and upgrade it in memory. Writers write only the
     latest schema.
   - Minor versions only add fields that are safe to copy. Readers keep unknown fields and
     unknown optional `sections` verbatim (a residue keyed by node id or image key) and write
     them back.
   - Before 1.0, a newer *major* version or an unknown node type is refused with a typed
     "made by a newer SlopShop" error. Placeholder nodes that are preserved and marked stale
     come with the first new node type in Phase 2.
   - From 1.0 on, the forward-compatibility guarantees are frozen.
   - Every schema version gets a golden fixture file in the repository.

8. **Not in v0.**
   - Undo history (a section type is reserved).
   - View state (reserved section `slopshop.view`).
   - Thumbnail (reserved; it needs a composite at a coarse level, since the CPU compositor
     works only at level 0 today).
   - Autosave and recovery file.
   - Lazy or out-of-core loading (the format supports it: open is eager in v0).
   - Snapshots or versions visible to users.
   - Encryption and signatures.
   - Big-endian hosts (refused).
   - Any promise of interoperability: interchange goes through export (ORA, PSD and OME-Zarr
     later).


9. **AI results and costly caches** (later nodes) are kept in the document as source-like
   blobs, with an explicit per-node purge, because they are not reliably reproducible. A side
   cache directory would be lost when the file is moved.

## Alternatives

- **SQLite**: proven atomic commits and incremental pages, but C code, a `-journal` side file,
  VACUUM to reclaim space, and blobs at or above its documented 100 KB break-even. WAL mode is
  2.5x slower for GB-sized saves and adds `-wal` and `-shm` files next to the document.
  Runner-up.
- **Zip + manifest (ORA/KRA style)**: easy to inspect, but every save rewrites GBs (in-place
  append was shown to leave an unreadable archive after a crash). Kept for a layered export.
- **Full rewrite on every save, with our container**: simplest and always compact, but save
  cost grows with document size (0.4–2.5 s for 1–5 GB on NVMe, several times more on HDD or
  USB), and the whole file is rewritten for a one-layer change.
- **Directory bundle or Zarr store**: many small files (slow with NTFS and antivirus),
  awkward to share on Windows.
- **redb, sled, fjall, bincode, rkyv or postcard for the manifest**: format churn, beta
  status, license failure, unmaintained, or weak schema evolution.
- **Positional tile keys (image, level, col, row) instead of hashes**: no hashing, but no
  deduplication, harder incremental saves, and no persistent cache key for AI nodes.

## Consequences

- Save costs grow with what changed. A first save of a 233 MP RGBA8 document is estimated
  at about 0.5 s and 0.19–0.25 GB (+35 % with the pyramid). A save with no new pixels takes a
  few milliseconds plus serializing the manifest and index. HDD or USB saves are disk-bound.
- The user's file is modified in place, append-only, with a two-phase commit. A crash or power
  loss leaves the previous generation readable. This relies on fsync being honest (some
  consumer drives acknowledge early).
- Files grow between compactions. Dead bytes are tracked and shown by `slopshop-cli inspect`.
- We own a specification (`docs/file-format.md`), a fuzz target for the reader, and
  fault-injection tests.
- Core needs `Document::restore`, `RasterImage::from_tiles` and a pyramid rebuild from tiles.
  Serialization stays in `slopshop-io`.
- Tile residency can become lazy later (Phase 4) without a format change. The `tile()` API
  will need to change then (see ADR 0005).
- New dependencies in `slopshop-io`: serde and serde_json (already in the lockfile), blake3,
  and the codec (zstd or lz4_flex).
