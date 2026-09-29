# Document file format v0: research notes

**Bottom line.** I recommend a custom single-file container for `.slop` (name to be confirmed). The file is append-only inside and holds three things:
- **Tiles**, content-addressed by BLAKE3 hash, each compressed on its own and kept bit-exact in the source format.
- **A JSON manifest** of node-shaped records with stable ids.
- **Two checksummed commit slots** in the header, which make each save atomic.

A save appends only the tiles the file does not already hold, and a full rewrite happens only on Save As or compaction.

I checked the decisive claims about the code myself (listed in §0). The web sources, crate facts and benchmark numbers come from the three reports; I did not re-run or re-fetch them.

---

## 0. Cross-check against the repository (checked by me)

These report claims hold:
- **Core has no dependencies.** `crates/slopshop-core/Cargo.toml` has an empty `[dependencies]` with the comment "Intentionally dependency-free".
- **`Document` is cheap to snapshot.** It is `#[derive(Clone)]` with fields `size, working_space, layers, next_layer_id, revision` (document.rs). `Document::new` always starts at `next_layer_id: 1`, and `is_allocated` requires `id != 0 && id < next_layer_id`. There is no constructor that rebuilds a document from saved ids.
- **`ImageId` cannot be saved.** It comes from a process-wide `AtomicU64` (raster.rs:36-40).
- **Tile access is a synchronous borrow.** `RasterLevel::tile()` returns `Option<&Arc<[u8]>>` (raster.rs:70). Its callers outside tests are composite.rs:177, render/lib.rs:648 and io/lib.rs:679. ADR 0005 says residency can become lazy "without changing callers"; that is optimistic.
- **The compositor works at full resolution only** ("pyramid level 0, never coarser", composite.rs:4). A thumbnail therefore needs a new composite-at-level path.
- **Export's commit helper is incomplete.** `TempFile::persist` just calls `fs::rename`, so the parent directory is never fsynced. A temp file left by a crash stays behind: its name `<name>.slopshop-tmp` is fixed, so only a later export to the same name overwrites it.
- **Import warnings live only in the app shell** (`app/src-tauri/src/lib.rs:98`, `layer_warnings: HashMap<LayerId, Vec<&'static str>>`).
- **The public API is enough to write a serializer.** `ColorSpace { pub primaries, pub transfer }` is public, and so are `RasterImage::{size, format, levels, id}`.
- **deny.toml allows every license the plan needs**: MIT, Apache-2.0, BSD-3-Clause, CC0-1.0, Zlib, MPL-2.0. BSL-1.0 is not allowed, which confirms the fjall rejection.
- **The workspace MSRV is 1.93** and `unsafe_code = "deny"`.

These report claims needed correcting:
1. **ADR 0008 already exists** ("Export", accepted 2026-09-29, listed in `docs/adr/README.md`). The prior-art report said there was none. The new ADR is therefore **0009**.
2. **rayon is not a direct dependency of any engine crate.** It appears in Cargo.lock only through other crates. The engine runs its workers on `std::thread::scope` (raster.rs:524, composite.rs:118, export/mod.rs:616/704). The format code should do the same, so no new dependency is needed.
3. **The reports contradicted each other on three points**, resolved here:
   - *Store the pyramid or not.* The requirements report measured it; the crates report did not. I follow the measurements: store it.
   - *Float filters.* Shuffle alone helped F32 (0.643 → 0.537), while shuffle + delta hurt it (3.36× → 2.88×). The results are consistent: use no delta on floats.
   - *How a save is committed.* Scanning back from the end for the last trailer (PDF style) versus two header slots (LMDB/redb style). I chose header slots: open is O(1) and junk appended at the end is never ambiguous.
4. **zstd licensing.** The benchmark used zstd 0.13.3 (MIT); 0.14.0 is BSD-3-Clause, and zip 8.6 still pins `^0.13.3`. Both licenses are allowed. Pick one version consistently.

Inferred and not checked:
- blake3's default build compiles bundled assembly or C SIMD code through `cc`. A `pure` feature is believed to avoid this; check `build.rs` before deciding on the C question.
- `std::fs::File::lock` is believed to be stable since Rust 1.89. The plan below avoids needing a lock anyway.

---

## 1. Container comparison

Scale: ++ excellent, + good, 0 neutral, − weak, −− disqualifying. **M** = measured by the reports (NVMe, 20 threads). **I** = inference.

| Criterion | **A. Custom single file, append-only** (header slots, blobs, JSON manifest) | **B. SQLite single file** (rusqlite, rollback journal) | **C. Zip + JSON manifest + tile entries** | **D. Directory bundle** (Zarr-like) |
|---|---|---|---|---|
| Full save speed | ++ (M) 944 MB durable in 0.49–0.55 s | + (M) 0.60–0.65 s; WAL mode 1.47 s | + (M) 0.56–0.64 s, stored entries | 0 (I) thousands of files; NTFS plus Defender costs per file |
| Incremental save | ++ (M) only changed tiles are appended; a metadata-only commit takes about 0.5 ms | ++ only changed pages are written; freed space needs VACUUM | −− in-place append is not crash-safe (verified by test), so every save rewrites GBs | + new tile files plus a manifest rename |
| Crash safety | + by design (checksums plus a fallback slot); we must prove it with fault-injection tests and fuzzing | ++ proven; but a `-journal` side file exists during saves, and sync folders are a risk | + only with temp file + rename (full rewrite) | 0 consistency across many files; users copy half a folder |
| Open speed, lazy loading | ++ index plus positional reads; lazy loading needs no format change | + positional blob reads; blobs over a page become overflow chains (about the 100 KB break-even) | + central directory gives random access | + |
| Simplicity | 0 our own spec, recovery and compaction code | + SQLite handles atomicity; pragmas and schema are subtle | ++ | ++ |
| Interoperability, debugging | − needs a written spec plus `slopshop-cli inspect`; the JSON manifest helps | + `sqlite3` CLI | ++ any unzip tool | ++ locally, − to share |
| Growth to a DAG and cached AI results | ++ content-addressed blobs; node records; results kept as blobs | ++ | + but growing AI caches get rewritten on every save | + |
| Dependency and license cost | ++ codec + hash + serde only | 0 rusqlite 0.40.2 MIT plus bundled C SQLite (public domain), a C toolchain, C-side advisories | + zip 8.6.0 MIT with default features turned off | ++ |

Rejected outright:
- **redb 4.3.0**: format churn (v2 support dropped in 3.0), a data-loss fix in 4.0, files 2.2× larger (M), and the slowest save (M).
- **sled**: still beta.
- **fjall**: depends on xxhash-rust, which is BSL-1.0 and fails deny.
- **bincode**: RUSTSEC-2025-0141, unmaintained.
- **memmap2**: its constructors are `unsafe`, and I/O errors become SIGBUS or access violations. Also no gain, since tiles are owned `Arc<[u8]>`.

Zip remains the right choice for a future **ORA-style layered export**.

---

## 2. Decision

See [ADR 0009](../adr/0009-document-file-format.md), written from this study.

---

## 3. Decisions (taken 2026-09-29: the recommendation for each)

1. **Save mode.** This is the hardest to reverse because it changes crash behaviour.
   - (a) Incremental in-place append with a two-slot commit. Fastest, costs grow with what changed. Every save touches the user's file, and we own the recovery logic.
   - (b) Same container, but every save is a full compact rewrite (temp file + rename). Simpler and never mutates the file in place, but 0.4–2.5 s or more per save on NVMe and far slower on HDD. Incremental writes could then serve only a future recovery file.
   - (c) SQLite.
   - Recommendation: **(a)**, gated by the fault-injection test suite. (b) is a safe fallback, and moving from (b) to (a) later needs no format break.
2. **Native C code for the codec (and blake3's SIMD build).** The `slopshop-io` Cargo comment says "no C code".
   - (a) zstd 0.14 (BSD-3, libzstd 1.5.7): best ratio and speed.
   - (b) lz4_flex 0.14 (MIT, pure Rust): within 5–10 % on shuffled 16-bit and float data, but about 40 % larger on 8-bit (0.35 vs 0.19–0.25).
   - (c) LZ4 by default, zstd behind a feature.
   - Recommendation: **(a)**, stated in ADR 0009 as a justified exception, with blake3 on its default build. Because the codec is recorded per blob, switching later is not a break.
3. **Pyramid on disk.**
   - (a) All levels: +35 %, fastest open.
   - (b) Levels 3 and up only: +2.5 %, the rest rebuilt.
   - (c) None: 1–2.3 s rebuild, which blocks lazy open.
   - Recommendation: **(a)** by default, with a "strip derived data" option.
4. **Compatibility promise before 1.0.**
   - (a) Older readers refuse newer majors and unknown node types; stability is promised from 1.0.
   - (b) Placeholder nodes that are preserved and marked stale, starting in v0.
   - Recommendation: **(a)**. It is honest and cheap, and (b) arrives with the first new node type.
5. **AI results and costly caches** (fixes the manifest's blob classes now, used later).
   - (a) Kept in the document as source-like blobs, with an explicit per-node purge.
   - (b) A side cache directory (Nuke style), lost when the file is moved.
   - Recommendation: **(a)**. Results are not reliably reproducible.
6. **Extension, MIME type and magic.**
   - Options: `.slop` or `.slopshop`, with `application/vnd.slopshop.document`.
   - Recommendation: `.slop`. This is branding, so it is the maintainer's call.
7. **Undo history and view state in the file.**
   - Recommendation: neither in v0, with section types reserved. Confirm.

Already recommended and **not** a maintainer decision:
- JSON manifest.
- BLAKE3-256.
- DTOs in `slopshop-io`.
- Positional reads instead of mmap.
- Tile size recorded in the file (256 today).
- Little-endian byte order.

---

## 4. Implementation plan

Each step is small and testable, and each leaves `fmt`, `clippy -D warnings`, `test` and `deny` passing.

| # | Where | What | Tests |
|---|---|---|---|
| 0 | docs | Maintainer decisions, then ADR 0009 plus a row in the ADR index; `docs/file-format.md` byte spec; update `architecture.md` (open questions table) and `roadmap.md` | — |
| 1 | core `document.rs` | `Document::restore(size, working_space, layers, next_layer_id) -> Result<_, RestoreError>`; `next_layer_id()` getter | Rejects duplicate ids, id 0, id ≥ next, opacity outside [0,1], non-finite fill; restore(clone) ≡ original |
| 2 | core `raster.rs` | `RasterImage::from_tiles(size, format, levels)` (validates tile count and length) and `with_level0_tiles(...)` that rebuilds the pyramid from tiles | from_tiles(from_pixels(x) tiles) bit-identical for every `PixelFormat`; **tile-local pyramid rebuild == from_pixels** on odd sizes (settles the inference) |
| 3 | io `commit.rs` (shared) | Extract `TempFile` from export into an `atomic_replace` helper: fsync the file, rename, fsync the directory on Unix, remove stale temp files on open or save | Existing export tests; failure midway leaves the destination untouched |
| 4 | io `document_file/format.rs` | Header, slots, record framing, index encoding, shuffle/delta filters, codec wrapper, BLAKE3 keys; pure functions, typed `FileError` | Filter roundtrip per sample type including NaN payloads and ±Inf; slot checksum; torn slot → other slot |
| 5 | io `document_file/write.rs` | Full compact write from a `Document` snapshot; parallel hashing and compression on `thread::scope`; bounded memory (streams per tile) | Roundtrip bit-exact for every format; Arc sharing kept (two layers → one image → `Arc::ptr_eq` after load); ids and `next_layer_id` kept; golden fixture `v0.1.slop` |
| 6 | io `document_file/read.rs` | Eager open with parallel decompression and hash verification; upgrade chain skeleton; unknown fields and sections kept as residue | Golden fixture opens; corrupted blob → typed error, never a panic; newer major refused; **fuzz target** on the reader (a `libfuzzer-sys` fuzzing setup exists per the lockfile) |
| 7 | io `document_file/session.rs` | `DocumentFile` handle: path, generation, index, `ImageId` → hash cache, dead-byte counter; incremental `save`; compaction threshold; optimistic concurrency check | Re-save without change adds only metadata (assert growth ≤ manifest + index); adding a layer appends only its tiles; **crash injection**: truncate at every record boundary and at random offsets during the last save → previous generation opens; junk after `committed_len` removed by the next save; external generation change → conflict error |
| 8 | io + app | Move import warnings into image records (`warnings` ids) instead of `app/src-tauri` | Warnings survive save and load |
| 9 | cli | `slopshop-cli save <images…> -o doc.slop`, `open`/`export` from `.slop`, `inspect` (header, generations, manifest JSON, dead bytes), `compact` | CLI integration tests; headless proof |
| 10 | app | `async` commands `open_document` (by magic), `save_document`, `save_document_as` on `spawn_blocking`, with progress events; dirty flag = `revision != saved_revision`; Ctrl+S and Ctrl+Shift+S; unsaved-changes prompt on close; i18n keys in en and fr | `npm run check`; manual tests (below) |
| 11 | bench (ignored test or CLI) | Synthetic 233 MP RGBA8 and 100 MP RGBA32F: first save, save after a 1 % change, open; NVMe and HDD/USB; Defender on; add a real 16-bit TIFF and an HDR EXR to validate filters | Record numbers in ADR 0009; targets: incremental save with no pixels < 50 ms; 1 GB first save < 1 s; open < 1 s per GB compressed |

Later, not v0:
- Autosave and recovery file, in the same format under the app data directory.
- Thumbnail, which needs a composite at a coarse pyramid level.
- Lazy residency (Phase 4, changes the `tile()` signature).
- History section.
- ORA export through zip.

Manual test checklist for the maintainer (after step 10):
- Open `out/default.jpg`, add a fill layer, save as `test.slop`, close, reopen. Layers, ids, opacity and pixels should be identical, and a new layer's id should continue from the saved counter.
- Make a small change and save: it should be near-instant and the file should grow only a little.
- Kill the app during a save: the file should reopen at the previous save.

New dependencies (all for `slopshop-io`; mention them in the commit message):

| Crate | Version | License | Why | Why not std or an existing dependency |
|---|---|---|---|---|
| serde (+derive) | 1.0.229 | MIT OR Apache-2.0 | Manifest DTOs | Already in the lockfile via Tauri |
| serde_json | 1.0.151 | MIT OR Apache-2.0 | Readable manifest that keeps unknown fields | Already in the lockfile (app dev-dependency) |
| blake3 | 1.8.7 | CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception | Tile identity and integrity; 15 ms per GB | std has no cryptographic hash; xxh3 collisions can be crafted, and xxhash-rust fails deny |
| zstd | 0.14.0 (zstd-sys 2.1.0+zstd.1.5.7) | BSD-3-Clause, C | Default codec | flate2 deflate-1 is strictly worse (M); ruzstd encodes 6–8× slower and ≥25 % larger |
| *or* lz4_flex | 0.14.0 | MIT, pure Rust | Codec if pure Rust is chosen | — |

Not needed: rayon (use `thread::scope`), crc32fast (BLAKE3 covers integrity), tempfile, fs4, memmap2.

---

## 5. Sources

- **Repository (checked by me):** `crates/slopshop-core/src/document.rs`, `raster.rs` (24, 36-40, 70, 121, 190-215, 524), `composite.rs` (4, 118, 177), `color.rs` (342-345), `slopshop-io/src/export/mod.rs` (12, 76, 549, 739-771), `slopshop-io/Cargo.toml`, `app/src-tauri/src/lib.rs:98`, `deny.toml`, `Cargo.toml` (MSRV 1.93, `unsafe_code = "deny"`), `docs/adr/0003`, `0005`, `0006`, `0008`, `docs/adr/README.md`, `docs/architecture.md:154`, `docs/roadmap.md:43-44`.
- **Web (cited by the reports, not re-fetched):**
  - https://www.sqlite.org/appfileformat.html , /intern-v-extern-blob.html , /wal.html , /lts.html
  - https://docs.rs/zip/8.6.0 (and the reports' `new_append` crash test)
  - https://github.com/cberner/redb (CHANGELOG)
  - https://zarr-specs.readthedocs.io/en/latest/v3/codecs/sharding-indexed/index.html
  - https://git-scm.com/book/en/v2/Git-Internals-Git-Objects
  - https://www.kernel.org/doc/html/latest/filesystems/ext4/super.html
  - https://www.w3.org/TR/png-3/#5PNG-file-signature
  - https://developer.blender.org/docs/features/core/dna/
  - https://docs.krita.org/en/general_concepts/file_formats/file_kra.html
  - https://www.figma.com/blog/making-multiplayer-more-reliable/
  - https://aras-p.info/blog/2023/02/01/Float-Compression-3-Filters/
  - https://docs.rs/memmap2/0.9.11
  - RUSTSEC-2025-0141 (bincode)
  - man fsync(2)
  - RFC 6648 (x- prefix deprecated)
- **Benchmarks:** run by the research agents on the maintainer's machine (NVMe, 20 threads, warm cache); scripts not kept in the repository. The 16-bit and float compression data are synthetic.
