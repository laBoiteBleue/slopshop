# The `.slop` document format, version 0.5

The byte-level specification of SlopShop documents. The design and its reasons are in
[ADR 0009](adr/0009-document-file-format.md). The reference implementation is
`crates/slopshop-io/src/slop/`. Before 1.0 the format may change in incompatible ways: a reader
refuses files with a newer major version.

## Conventions

- All integers are little-endian and unsigned. Offsets and lengths are `u64` byte counts from
  the start of the file.
- A **hash** is BLAKE3-256 (32 bytes). In JSON it is written as a **key**: `b3:` followed by 64
  lowercase hexadecimal digits.
- Readers must bound every length they read by the file length and by the limits below before
  allocating anything.

## Layout

```
0      header (4096 bytes, including the two commit slots)
4096   records, appended one after the other, each aligned to 8 bytes
```

The file is append-only after the header: a save appends new records, then overwrites one of the
two commit slots. Bytes after the committed length of the valid slot (left by an interrupted
save) are ignored, and truncated by the next save.

## Header

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | magic `89 53 4C 50 0D 0A 1A 0A` (`\x89SLP\r\n\x1a\n`) |
| 8 | 2 | `major` = 0 |
| 10 | 2 | `minor` = 1 |
| 12 | 4 | `header_len` = 4096 |
| 16 | 8 | `incompat` feature flags |
| 24 | 8 | `ro_compat` feature flags |
| 32 | 8 | `compat` feature flags |
| 40 | 984 | zero |
| 1024 | 136 | commit slot 0 |
| 1160 | 888 | zero |
| 2048 | 136 | commit slot 1 |
| 2184 | 1912 | zero |

A reader:
- refuses a file whose magic differs (not a SlopShop document);
- refuses a `major` greater than its own ("made by a newer SlopShop");
- refuses a file with an `incompat` flag it does not know;
- opens a file with an unknown `ro_compat` flag read-only (it may read, never save in place);
- ignores unknown `compat` flags.

No flag is defined yet: writers write 0 in all three fields.

## Commit slots

A slot says which manifest and index make up the document, and where the file ends.

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | `generation`: 1 for the first save, +1 for each save |
| 8 | 8 | manifest record: offset |
| 16 | 8 | manifest record: stored length (payload only) |
| 24 | 32 | manifest record: hash |
| 56 | 8 | index record: offset |
| 64 | 8 | index record: stored length |
| 72 | 32 | index record: hash |
| 104 | 8 | `committed_len`: end of this generation's commit record, i.e. the length of the valid file |
| 112 | 8 | writer version: four `u16` (major, minor, patch, 0) |
| 120 | 16 | the first 16 bytes of BLAKE3 over bytes 0–119 of the slot |

Generation `g` is written to slot `g mod 2`. A slot is **intact** when it is not all zero, its
checksum matches and `4096 ≤ committed_len ≤ file length`. A reader uses the intact slot with the
highest generation. If loading that generation fails, it tries the other slot, then
**recovery**: it walks the records from offset 4096 (stopping at the first record header that
does not decode or overruns the file) and collects every commit record whose payload is an intact
slot whose `committed_len` equals the end of that commit record, and tries them newest first.

### Saving

An incremental save of generation `g + 1`:
1. reads the header again and fails with a conflict if the best slot's generation or committed
   length changed since the file was opened or last saved (another program saved meanwhile);
2. truncates the file to `committed_len` of generation `g`;
3. appends the records the file lacks, the tables, the manifest, the index and a commit record;
4. flushes the file data to the device (`fdatasync`/`FlushFileBuffers`);
5. writes the slot `(g + 1) mod 2`, and flushes again.

A crash at any point leaves generation `g` or `g + 1` readable. A full write (first save, Save
As, compaction) writes a compact file of generation 1 to a temporary file next to the destination,
flushes it and renames it over the destination (then flushes the directory on Unix).

## Records

| Offset | Size | Field |
|---|---|---|
| 0 | 1 | `kind`: 1 tile, 2 table, 3 manifest, 4 index, 5 commit |
| 1 | 1 | `codec`: 0 raw, 1 zstd |
| 2 | 1 | `filter`: 0 none, 1 shuffle, 2 shuffle + delta |
| 3 | 1 | `stride`: element size of the filter, in bytes (0 without filter) |
| 4 | 4 | zero |
| 8 | 8 | `stored_len`: payload length |
| 16 | 8 | `raw_len`: length after decompression |
| 24 | 32 | `hash` |
| 56 | `stored_len` | payload |
| … | 0–7 | zero padding to a multiple of 8 bytes |

The record spans `(56 + stored_len)` rounded up to a multiple of 8. Unknown kinds, codecs or
filters, or a filter with `stride` 0, make the record invalid.

**Decoding a payload:** decompress it (zstd frame; raw: as is), check that the result is
`raw_len` bytes, then undo the filter. The **hash** of a tile, table, manifest or index record is
the hash of these decoded bytes, and a reader checks it: that is the integrity check of the
format. The hash of a commit record is the hash of its 136-byte payload.

Limits on `raw_len` (anything larger is corrupt): tile 1 MiB, table 64 MiB, manifest 256 MiB,
index 1 GiB, commit 136 bytes.

A writer stores a payload raw (codec 0, filter 0, stride 0) when compression saves less than 5 %
of `raw_len`.

### Filters

With `n = raw_len / stride` whole elements and `r = raw_len mod stride` remaining bytes:
- **shuffle** writes byte `k` of element `i` at position `k·n + i` (byte planes), followed by the
  `r` remaining bytes unchanged. With `stride` 1 it changes nothing.
- **shuffle + delta** shuffles, then splits the result into consecutive chunks of `n` bytes (the
  planes; a last, shorter chunk holds the `r` remaining bytes) and replaces each byte of a chunk,
  except its first, by its difference with the previous byte, modulo 256.

Decoding undoes these steps in reverse order: running sums per chunk, then unshuffle.

## Tiles and tables

An image is a pyramid of levels. Level 0 is the image; level `k + 1` is half of level `k` (rounded
up) in each dimension, until a level fits in one tile. Each level is cut into 256×256 tiles, row-major. A **tile**
record holds one tile exactly as it is in memory: 256 rows of 256 pixels in the image's *stored
format* (the source format, except that RGB is stored as RGBA with opaque alpha), samples
little-endian (`u8`, `u16`, IEEE 754 `f16`, `f32`), padding beyond the image edge included. Its
hash is the hash of these bytes, so identical tiles are stored once in the whole file.

SlopShop writes tiles with zstd level 1, filter shuffle + delta for integer samples and shuffle
for float samples, and `stride` = bytes per stored pixel. Readers must accept any combination.

A **table** record lists the hashes of the tiles of one level, row-major, 32 bytes each. It is
stored raw by SlopShop and addressed by its own hash.

The **image key** is BLAKE3 over, in order: the bytes `slopshop.image.v1\0`; the JSON of the
image's `source_format` exactly as written in the manifest (see below, fields in the order given
there, no whitespace); width and height as `u32`; the tile size (256) as `u32`; and the hash of
the level-0 table.

## Index

The index maps every hash the generation may refer to (tiles, tables) to its record:

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | `count` |
| 8 + 64·i | 32 | hash |
| +32 | 8 | record offset |
| +40 | 8 | stored length |
| +48 | 8 | raw length |
| +56 | 1 | kind |
| +57 | 1 | codec |
| +58 | 1 | filter |
| +59 | 1 | stride |
| +60 | 4 | zero |

Entries are sorted by hash. The index may list records that the current manifest no longer uses
(they become dead bytes, reclaimed by the next compaction). SlopShop compresses it with zstd
level 1 and no filter.

## Manifest

UTF-8 JSON, compressed by SlopShop with zstd level 3 and no filter. Example (hashes shortened):

```json
{
  "schema": { "major": 0, "minor": 5 },
  "writer": { "app": "slopshop", "version": "0.1.0" },
  "document": {
    "size": [21600, 10800],
    "working_space": {
      "primaries": { "r": [0.708, 0.292], "g": [0.17, 0.797], "b": [0.131, 0.046], "w": [0.3127, 0.329] },
      "transfer": { "kind": "linear" },
      "id_hint": "linear-rec2020"
    },
    "next_node_id": 8,
    "stack": [3, 7],
    "blend_space": "perceptual"
  },
  "nodes": {
    "3": { "type": "slopshop.raster", "version": 3, "name": "Background", "visible": true,
           "opacity": 1.0, "params": { "image": "b3:9f2c…", "blend_mode": "normal",
           "mask": { "image": "b3:41d7…", "enabled": true, "replaces_alpha": true } },
           "inputs": [] },
    "5": { "type": "slopshop.fill", "version": 3, "name": "Tint", "visible": true,
           "opacity": 0.5, "params": { "color": [0.2, 0.1, 0.0, 1.0], "blend_mode": "multiply" },
           "inputs": [] },
    "7": { "type": "slopshop.group", "version": 3, "name": "Folder", "visible": true,
           "opacity": 1.0, "params": { "blend_mode": "normal", "pass_through": true },
           "inputs": [5] }
  },
  "images": {
    "b3:9f2c…": {
      "size": [21600, 10800],
      "tile_size": 256,
      "source_format": {
        "layout": "rgb", "sample": "u8", "alpha": "straight",
        "color_space": { "primaries": { "…": "…" }, "transfer": { "kind": "srgb" }, "id_hint": "srgb" }
      },
      "levels": [ { "table": "b3:…" }, { "table": "b3:…", "derived": true } ],
      "pyramid_algorithm": "slopshop.pyramid.box-linear-premul@1"
    }
  }
}
```

- `schema`: a reader accepts any version with its own major and upgrades it in memory.
- `document.size`: canvas width and height in pixels. `working_space`: the compositing color
  space (linear transfer only so far). `next_node_id`: the next id to allocate, greater
  than every node id. `stack`: the top-level node ids, bottom to top. Every node of `nodes` is
  used exactly once, in the stack or as the input of a group: they form a tree.
  `blend_space` (0.2): where layers blend, `perceptual` or `linear` ([ADR
  0012](adr/0012-blend-modes.md)); absent in 0.1 files, which read as `linear`.
- **Color spaces**: CIE xy chromaticities of the primaries and white point, and a transfer
  function with `kind` one of `linear`, `srgb`, `gamma` (`gamma`), `rec709`, `parametric`
  (ICC parametric curve `g a b c d e f`), `pq`, `hlg`. `id_hint` is informative only.
- **Nodes** are keyed by id (decimal string). `type` and `version` select the parameters:
  - `slopshop.raster` v1 and v2: `params.image` is the key of an entry of `images`.
  - `slopshop.fill` v1 and v2: `params.color` is a linear, straight-alpha RGBA color in the
    working space.
  - v2 (schema 0.2) adds `params.blend_mode`, one of `normal`, `darken`, `multiply`,
    `colorBurn`, `linearBurn`, `darkerColor`, `lighten`, `screen`, `colorDodge`,
    `linearDodge`, `lighterColor`, `overlay`, `softLight`, `hardLight`, `vividLight`,
    `linearLight`, `pinLight`, `hardMix`, `difference`, `exclusion`, `subtract`, `divide`,
    `hue`, `saturation`, `color`, `luminosity`. v1 nodes are in normal mode. An unknown mode
    or blend space is refused like an unknown node type.
  - v3 (schema 0.3) adds the optional `params.mask` ([ADR 0014](adr/0014-layer-masks.md)):
    `image`, the key of a gray entry of `images` whose samples are coverage (read linearly);
    `enabled`; `replaces_alpha` (the layer's own alpha is ignored while the mask exists). Nodes
    below v3 have no mask.
  - `slopshop.group` v3 (schema 0.4, [ADR 0015](adr/0015-layer-groups.md)): `inputs` are the
    group's layers, bottom to top; `params.pass_through` (the layers blend through the group,
    faded by its opacity and mask; else they are composited on their own and blended as one
    layer with `blend_mode`), `params.blend_mode` and the optional `params.mask`. Groups nest at
    most 16 deep.
  - v4 (schema 0.5, [ADR 0016](adr/0016-clipping-masks.md)) adds `params.clipped`: the node is
    clipped to the nearest sibling below it that is not clipped. Only clipped nodes are written
    at v4 (others stay at v3), so that readers without clipping refuse them.
  - `opacity` is in [0, 1]. `inputs` is empty for rasters and fills.
  - A reader refuses a node type or version it does not know ("made by a newer SlopShop").
- **Images** are keyed by image key. `layout` is `gray`, `gray-alpha`, `rgb` or `rgba`; `sample`
  is `u8`, `u16`, `f16` or `f32`; `alpha` is `straight` or `premultiplied`. `levels` lists the
  tables of the pyramid levels, finest first. Levels marked `derived` are computed from level 0
  with `pyramid_algorithm`; a reader that does not know that algorithm, or finds derived levels
  missing, may rebuild them. SlopShop checks the image key on load.
- **Unknown data is kept:** unknown fields of the manifest, the document, nodes and images, and
  every entry of the optional top-level `sections` object, are written back unchanged by a
  save.

Not stored yet: undo history, view state, thumbnail, ICC/EXIF/XMP metadata.
