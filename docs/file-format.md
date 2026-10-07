# The `.slop` document format, version 0.6

The byte-level specification of SlopShop documents. The design and its reasons are in
[ADR 0009](adr/0009-document-file-format.md). The reference implementation is
`crates/slopshop-io/src/slop/`. Before 1.0 the format may change in incompatible ways: a reader
refuses files with a newer major version.

This specification is licensed under [CC BY 4.0](../LICENSES/CC-BY-4.0.txt), not under the
GPL of SlopShop's code, so that other software can implement the format. The golden files in
`crates/slopshop-io/src/slop/fixtures/` are under [CC0 1.0](../LICENSES/CC0-1.0.txt).

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
  "schema": { "major": 0, "minor": 6 },
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
    "blend_space": "perceptual",
    "resolution": 300
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
  `resolution` (0.11): pixels per inch, how large the document prints ([ADR
  0028](adr/0028-resolution.md)); metadata only, in `[1, 100000]`; absent in older files,
  which read as 72 (Photoshop's default).
  `selections` (0.16): the selections saved by name (Select > Save Selection), in the order
  they were saved, each `{ "id": 2, "name": "Hair", "image": "b3:…" }`: a unique id below
  `next_selection_id`, and the key of a gray coverage image (the selection's mask, at the
  document origin, normally the canvas's size). Absent: none. `next_selection_id` (0.16): the
  next saved selection id to allocate; absent, one above the largest id. Their images are
  stored like the layers'. A writer of an older schema drops them (it keeps unknown fields but
  not images no node references): before 1.0, compatibility may break.
  `guides` (0.22): the guides (View > Rulers), in the order they were placed, each
  `{ "axis": "vertical", "position": 120 }`: `vertical` at `position` document pixels from the
  canvas's left edge, `horizontal` from its top edge; a finite number within 1e9 of the
  origin, possibly fractional or outside the canvas; at most 10 000 guides. Absent: none. An
  unknown `axis` is refused as made by a newer SlopShop. A writer of an older schema keeps
  them as an unknown field, without moving them when the canvas changes.
  `sources` (0.26, [ADR 0040](adr/0040-sources.md)): what raster nodes show, kept once, each
  `{ "name": "photo.jpg", "image": "b3:…" }` (`name` absent when nothing names it) and named
  by its index in this list. Absent: none. Before schema 0.26, a reader gives each raster
  node's original a source of its own, one per image (nodes of the same image share it),
  named after the first node. A writer of an older schema drops the references and keeps the
  list as an unknown field; a reader then ignores it, the file being of the older schema.
- **Color spaces**: CIE xy chromaticities of the primaries and white point, and a transfer
  function with `kind` one of `linear`, `srgb`, `gamma` (`gamma`), `rec709`, `parametric`
  (ICC parametric curve `g a b c d e f`), `pq`, `hlg`. `id_hint` is informative only.
- **Nodes** are keyed by id (decimal string). `type` and `version` select the parameters:
  - `slopshop.raster` v1 and v2: `params.image` is the key of an entry of `images`.
    From schema 0.26, at any version, `params.source` is the index of its source in
    `document.sources` (absent: a layer made empty, with none). Without a stack, the source's
    image is `params.image`. A layer painted beyond its source grows around it: its original
    (`params.image`) holds the source at `params.source_offset`, `[columns, rows]` of whole
    tiles (absent: `[0, 0]`), within it, and its stack is written even without entries
    (`"stack": []`). A missing source, or one outside the original, makes the file invalid.
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
  - v5 (schema 0.6, [ADR 0017](adr/0017-non-destructive-transforms.md)) adds `params.transform`,
    `[a, b, c, d, e, f]`: the point `(x, y)` of the node's content goes to
    `(a·x + c·y + e, b·x + d·y + f)` in its parent (the document or its group). Only nodes that
    are not at the identity are written at v5. The transform must be finite and invertible
    (magnitudes below 1e9, |a·d − b·c| ≥ 1e-9, [ADR 0018](adr/0018-resampling.md)); earlier
    versions of SlopShop read only whole-pixel translations and refuse the others.
  - v6 (schema 0.10, [ADR 0027](adr/0027-painting.md)) adds paint: `params.original` on a
    raster node is the key of its unpainted image, `params.image` being its painted pixels (what
    it shows), and `params.mask.original` likewise for a painted mask. Both images have the same
    size and share their unpainted tiles in the file. Only painted nodes are written at v6, so
    that readers without paint refuse them instead of losing the original. A v6 raster's paint
    reads as one paint entry of its stack (v7), exactly.
  - v7 (schema 0.12, [ADR 0029](adr/0029-layer-stack.md)) gives a raster node its stack:
    `params.image` is the original, never written, and `params.stack` lists what is applied to
    it, bottom to top. A paint entry is `{"paint": {"color", "keep", "space"}}`: `color` the key
    of `P`, an image of the original's format with an alpha channel, straight, holding the
    paint's color; `keep` the key of `k`, a gray image of `P`'s sample type (`f32` for float
    layers), linear; `space` the blend space it was laid in. The layer's result is
    `P + k·B` over what is below (`B`), computed on premultiplied values of that space; tiles
    the paint did not touch are `P = 0`, `k = 1` (one tile, stored once). An effect entry is
    `{"effect": [steps]}`, steps of one kind applied in order: each an adjustment's parameters
    (as adjustment nodes store them), `selection` (the key of a gray coverage image at the
    document origin, or `null`), `transform` (six numbers: the layer's pixels to the document
    when it was applied, where the selection is read) and `space`. The result is rounded to the
    original's format after each paint and each step. The result itself is not stored: readers
    evaluate it. Only nodes with a stack are written at v7; an entry of another kind comes from
    a newer SlopShop.
  - v8 (schema 0.17, [ADR 0032](adr/0032-layer-styles.md)) gives a raster or fill node, and
    from schema 0.19 a group node, its style: `params.style` is `{"fill_opacity", "drop_shadow", "outer_glow", "inner_shadow",
    "inner_glow", "color_overlay", "stroke"}`, the effects not added absent. `drop_shadow` holds `enabled`, `color` (linear working-space RGB,
    as a fill's), `mode` (a blend mode), `opacity`, `angle` (degrees, where the light comes
    from), `distance`, `spread` (percent) and `size` (pixels), as `inner_shadow` (its `spread`
    being Photoshop's Choke); `outer_glow` and `inner_glow` hold `enabled`, `color`, `mode`,
    `opacity`, `spread` (percent, Choke inside) and `size`; `color_overlay` holds `enabled`,
    `color`, `mode` and `opacity`; `stroke` holds `enabled`, `size`, `position` (`inside`,
    `center` or `outside`), `color`, `mode` and `opacity`. What the effects draw is not stored:
    readers draw it. Only styled nodes are written at v8 (a styled fill or group too: they skip
    v6 and v7); a style out of range is refused. A group's effects are drawn from what it holds.
  - v9 (schema 0.18) is v8 with `outer_glow`, `inner_shadow` or `inner_glow` in the style: only
    those nodes are written at v9, so that readers of v8 refuse them rather than drop these
    effects.
  - v10 (schema 0.20, [ADR 0034](adr/0034-editable-operations.md)) is a raster node whose stack
    has an entry hidden by its eye: `"hidden": true` on that entry (absent: shown). A hidden
    entry is kept but skipped when the result is evaluated. Only those nodes are written at v10,
    so that older readers refuse them rather than show the entry; `hidden` on a node of an
    earlier version is refused.
  - v11 (schema 0.25, [ADR 0038](adr/0038-projective-transforms.md)) is a raster node placed in
    perspective: its `params.transform`, or a stack step's `transform`, has nine numbers
    `[a, b, c, d, e, f, g, h, i]`, the projective map sending `(x, y)` to
    `((a·x + c·y + e) / w, (b·x + d·y + f) / w)` with `w = g·x + h·y + i` (`i` ≠ 0; the same map
    whatever the scale of the nine). Every pixel of the node's image and mask is on the side of
    the horizon line where `w > 0`; the image is resampled through the map (each sample's
    ellipse from the map's Jacobian there). Affine maps are still written with six numbers.
    Only those nodes are written at v11, so that older readers refuse them rather than misplace
    them; nine numbers on a node of an earlier version are refused.
  - v12 (schema 0.27) is a raster, fill or group node whose style has a Gradient Overlay:
    `params.style.gradient_overlay` holds `enabled`, `gradient` (stops as Gradient Map's,
    `[location 0–4096, r, g, b]` sRGB-encoded), `reverse`, `shape` (`linear` or `radial`),
    `angle` (degrees, counterclockwise from the right: at 90 the gradient starts at the bottom),
    `scale` (10 to 150, percent), `align` (across the box of the layer's pixels, else the
    canvas), `mode` and `opacity`. Linear, the gradient runs through the box's center along the
    angle from edge to edge (`(w·|cos| + h·|sin|) × scale`); radial, from the center out to half
    the box's longer side × scale. It fills the layer's shape, under a Color Overlay. Only those
    nodes are written at v12, so that older readers refuse them rather than drop the effect.
  - v13 (schema 0.28) is a raster, fill or group node whose style has a Satin:
    `params.style.satin` holds `enabled`, `color`, `mode`, `opacity`, `angle` (degrees,
    counterclockwise from the right), `distance` and `size` (0 to 250 pixels) and `invert`. The
    shape is blurred by a Gaussian of `size / 2` pixels; at each pixel, the blurred shape half
    the distance before and after it along the angle (whole pixels) differ by `d`; the effect's
    coverage is `d`, or `1 − d` inverted, within the shape, above the overlays and under Inner
    Glow. Only those nodes are written at v13.
  - v14 (schema 0.29) is a raster, fill or group node whose style has a Bevel and Emboss:
    `params.style.bevel` holds `enabled`, `style` (`innerBevel`, `outerBevel`, `emboss` or
    `pillowEmboss`), `depth` (1 to 1000, percent), `up`, `size` (0 to 250 pixels), `soften`
    (0 to 16 pixels), `angle` and `altitude` (degrees, the light), and `highlight` and `shadow`,
    each `{ "color", "mode", "opacity" }`. The shape blurred by a Gaussian of `size / 2` (`a`,
    0.5 on the edge) gives a height: `clamp(2a − 1, 0, 1)` (Inner), `clamp(2a, 0, 1)` (Outer),
    `a` (Emboss), `|2a − 1|` (Pillow), negated when not `up`; its slope (central differences)
    times `size × depth / 100` gives the surface's normal, lit by the unit vector toward the
    light. Where the light falls on it more than on the flat plane, the highlight's coverage
    `(lit − flat) / (1 − flat)`; less, the shadow's `(flat − lit) / flat`; each blurred by
    `soften / 2`; drawn above everything else, shadow then highlight. Only those nodes are
    written at v14.
  - From schema 0.21 ([ADR 0034](adr/0034-editable-operations.md)), a stack entry may be a
    filter entry, `{"filter": [steps]}`: steps of one kind applied in order, each
    `{"filter", "values", "selection", "transform", "space"}`: `filter` its identifier
    (`gaussianBlur`), `values` its parameters (Gaussian Blur: the radius, the standard deviation
    in the layer's pixels, 0.1 to 1000), the others as an effect step's. A filter reads around
    each pixel: the result of the entries below it is filtered whole (the layer's edges
    repeating outward, in premultiplied values of `space`), mixed with what it was by the
    selection's coverage, and rounded to the original's format. Older readers refuse such an
    entry as coming from a newer SlopShop, and so does a reader that does not know the filter.
  - From schema 0.23 ([ADR 0037](adr/0037-liquify.md)), a stack entry may be a Liquify entry,
    `{"liquify": {"displacement", "frozen", "cell", "space"}}`: `displacement` the key of
    an image of one pixel per node, `ceil(width / cell) × ceil(height / cell)` nodes of the
    layer, in the format gray + alpha of `f32` samples (linear sRGB, straight alpha: the
    channels are `dx` and `dy`, never converted, only stored), and `frozen` the key of a gray
    8-bit image of the same size (0 free, 255 frozen). `cell` is the layer's pixels a node
    spans (a power of two up to 256: 1, 2 or 4 as writers choose from the layer's size), `space`
    the blend space the pixels are interpolated in. Pixel (`x`, `y`) of the result reads the
    result of the entries below it at (`x + ½ + dx`, `y + ½ + dy`) (the layer's pixels, `dx`
    and `dy` interpolated bilinearly between the nodes, whose centers are at
    `((i + ½) × cell, (j + ½) × cell)`), bilinearly in premultiplied values of `space`, the
    layer's edges repeating outward, and is rounded to the original's format. Both images are
    stored with a pyramid as every image is, and a tile no stroke touched is all zero bytes
    (one shared tile). The entry takes `hidden` like the others. Older readers refuse such an
    entry as coming from a newer SlopShop.
  - `slopshop.adjustment` (schema 0.7, [ADR 0020](adr/0020-adjustment-layers.md)): an adjustment
    layer. `params.adjustment` is `exposure`, `hueSaturation`, `levels`, `brightnessContrast`,
    `vibrance`, `invert`, `posterize` or `threshold`, and from schema 0.8 `blackWhite`,
    `colorBalance`, `photoFilter` or `channelMixer`, from schema 0.9 `curves`, and from schema
    0.14 `gradientMap`, from schema 0.15 `selectiveColor`. `params.values` holds its parameters in
    this order: exposure, offset, gamma; hue, saturation, lightness; input black, input
    white, gamma, output black, output white; brightness, contrast; vibrance, saturation;
    nothing (invert); levels (posterize); level in [0, 1] (threshold); the reds, yellows,
    greens, cyans, blues and magentas weights, tint (flag), tint hue, tint saturation (black
    and white); the cyan–red, magenta–green and yellow–blue shifts of the shadows, the
    midtones and the highlights, preserve luminosity (flag) (color balance); the filter color
    (sRGB-encoded r, g, b in [0, 1]), density, preserve luminosity (flag) (photo filter); the
    red, green and blue output rows (red, green and blue weights, then the constant, in %),
    monochrome (flag) (channel mixer). Flags are 0 or 1. Schema 0.7 writes five numbers, the
    unused ones 0; from 0.8, at least five (so that 0.7 readers still read the first eight
    adjustments) and at most 16, missing ones read as 0. From schema 0.13, at most 20: `levels`
    may have fifteen more, the red, green and blue channels' own input black, input white,
    gamma, output black and output white, applied before the five of the composite (five
    values: the channels unchanged). From schema 0.9, `curves` has no
    values of its own but `params.curves`: four lists (composite, red, green, blue) of 2 to 16
    points `[input, output]`, integers 0–255 with strictly increasing inputs; the curve through
    them is a natural cubic spline, flat outside its points.
    From schema 0.14, `gradientMap` has one value, reverse (flag), and `params.gradient`: 2 to
    16 stops `[location, r, g, b]`, the location an integer 0–4096 never decreasing along the
    list, the color sRGB-encoded 0–255; between two stops the color is interpolated linearly
    (on sRGB-encoded values), outside them it stays the end stop's; the luminance mapped is
    0.299 r + 0.587 g + 0.114 b of the sRGB-encoded color, whatever the blend space.
    From schema 0.15, at most 37 values: `selectiveColor` has the cyan, magenta, yellow and
    black (integers, −100 to 100 %) of the reds, yellows, greens, cyans, blues, magentas,
    whites, neutrals and blacks, then the method (flag: absolute).
    Parameters out of range make the file invalid; an unknown adjustment comes from a newer
    SlopShop. No `inputs`.
  - `slopshop.gradientFill` (schema 0.24): a gradient fill layer, with the versions and the
    parameters of `slopshop.fill` (blend mode, mask, clipping, transform, style) but for its
    color: `params.gradient`, stops as Gradient Map's; `params.shape`, `linear` or `radial`;
    `params.from` and `params.to`, two different points `[x, y]` (finite) of the node's content
    (placed by its transform); `params.alpha`, the opacity `[start, end]` in [0, 1]. Point `p`
    of the content is at `t` along the gradient: linear, the projection of `p − from` on
    `to − from` divided by its squared length; radial, `|p − from| / |to − from|`; `t` clamped
    to [0, 1]. Its color is the gradient's at `t` (sRGB-encoded), its opacity interpolated
    linearly between the two. It covers the whole canvas, like a fill. Older readers refuse the
    unknown type.
  - `opacity` is in [0, 1]. `inputs` is empty for rasters, fills (solid or gradient) and
    adjustments.
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
