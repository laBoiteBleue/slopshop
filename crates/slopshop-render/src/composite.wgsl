// Compositor: one invocation per output pixel.
//
// Layers are composited in the working space (linear Rec.2020, unbounded, premultiplied alpha;
// ADR 0007), each combined with what is below by its blend mode, in the document's blend space
// (ADR 0012; see `blend_layer`). Raster tiles are sampled in their source encoding and converted here: transfer
// function decode, then a 3×3 matrix to the working space.
//
// Two entry points share that code:
// - `main`, the viewport: the result is converted to the display space (linear sRGB),
//   composited over a transparency checkerboard and encoded to sRGB 8-bit. Clipping only happens
//   at that last step: it is a *view transform*, the document is never converted.
// - `export_main`, export (ADR 0008): the working-space values themselves, as f32, one level-0
//   texel per output pixel, finite values never clamped. Non-finite values are replaced (see
//   `finite` and `saturated`) and counted in `export_non_finite`, like the CPU reference
//   compositor (slopshop_core::composite) does.

struct Params {
    // Document coordinate of the output's top-left corner.
    origin: vec2<f32>,
    // Document pixels per output pixel.
    scale: f32,
    layer_count: u32,
    out_size: vec2<u32>,
    doc_size: vec2<u32>,
    // Working space → display (linear sRGB), rows of a 3×3 matrix.
    display0: vec4<f32>,
    display1: vec4<f32>,
    display2: vec4<f32>,
}

const KIND_FILL: u32 = 0u;
const KIND_RASTER: u32 = 1u;
const NO_TILE: u32 = 0xffffffffu;
const TILE_SIZE: u32 = 256u;

// Tile storage classes (GpuTileFormat in tiles.rs).
const FORMAT_UNORM8: u32 = 0u;
const FORMAT_UINT16: u32 = 1u;
const FORMAT_FLOAT16: u32 = 2u;

// Layer flags (keep in sync with lib.rs).
const FLAG_PREMULTIPLIED: u32 = 1u;
// Gray source: only the first color channel of a texel is a stored sample (the others are
// copies of it, see gpu_texels in tiles.rs).
const FLAG_GRAY: u32 = 2u;

// Transfer function kinds (transfer_fields in lib.rs).
const TF_LINEAR: u32 = 0u;
const TF_SRGB: u32 = 1u;
const TF_GAMMA: u32 = 2u;
const TF_REC709: u32 = 3u;
const TF_PARAMETRIC: u32 = 4u;
const TF_PQ: u32 = 5u;
const TF_HLG: u32 = 6u;

// 192 bytes; keep in sync with `LayerFields` in lib.rs.
struct Layer {
    // Fill: working-space linear RGBA, premultiplied, opacity applied. Raster: unused.
    color: vec4<f32>,
    kind: u32,
    opacity: f32,
    // Raster: document pixels per pixel of the sampled pyramid level (2^level).
    level_scale: f32,
    // Raster: where this layer's tile table starts in `tile_table`.
    table_offset: u32,
    // Raster: visible tile range of the level, and the level size in pixels.
    tile_origin: vec2<u32>,
    tile_count: vec2<u32>,
    level_size: vec2<u32>,
    format: u32,
    flags: u32,
    // Raster: source transfer function (kind, g, a, b) and (c, d, e, f).
    transfer: vec4<f32>,
    transfer2: vec4<f32>,
    // Raster: source linear RGB → working space, rows of a 3×3 matrix.
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    // The enabled mask (FLAG_MASK, ADR 0014): a gray linear raster, planned like a layer.
    mask_tile_origin: vec2<u32>,
    mask_tile_count: vec2<u32>,
    mask_level_size: vec2<u32>,
    mask_table_offset: u32,
    mask_level_scale: f32,
    mask_format: u32,
    // Where the raster's origin, and the mask's, are in the document: whole pixels (ADR 0017).
    offset: vec2<i32>,
    mask_offset: vec2<i32>,
}

@group(0) @binding(0) var<uniform> params: Params;
// Visible layers, bottom to top.
@group(0) @binding(1) var<storage, read> layers: array<Layer>;
// Output pixels, tightly packed, RGBA8 sRGB (one u32 per pixel, R in the lowest byte).
@group(0) @binding(2) var<storage, read_write> output: array<u32>;
// Raster layers: cache slot of each visible tile, row-major within the layer's tile range.
@group(0) @binding(3) var<storage, read> tile_table: array<u32>;
// Cached tiles, one array per storage class, values as stored in the source.
@group(0) @binding(4) var tiles_unorm8: texture_2d_array<f32>;
@group(0) @binding(5) var tiles_uint16: texture_2d_array<u32>;
@group(0) @binding(6) var tiles_float16: texture_2d_array<f32>;
@group(0) @binding(7) var tiles_float32: texture_2d_array<f32>;

// Export: a region of the document at full resolution (32 bytes; keep in sync with
// `export_params_bytes` in region.rs).
struct ExportParams {
    // Document pixel of the output's top-left corner.
    origin: vec2<u32>,
    size: vec2<u32>,
    layer_count: u32,
}

@group(0) @binding(8) var<uniform> export_params: ExportParams;
// Output pixels, row-major: premultiplied working-space RGBA.
@group(0) @binding(9) var<storage, read_write> export_output: array<vec4<f32>>;
// Non-finite values replaced in the dispatch, as a 64-bit count: low word, then high word.
// Cleared before each dispatch.
@group(0) @binding(10) var<storage, read_write> export_non_finite: array<atomic<u32>, 2>;

// Linear sRGB display colors.
const PASTEBOARD = vec3<f32>(0.0144, 0.0144, 0.0168);
const CHECKER_LIGHT = vec3<f32>(0.527, 0.527, 0.527);
const CHECKER_DARK = vec3<f32>(0.314, 0.314, 0.314);
// Checker squares are sized in output pixels so they look the same at every zoom level.
const CHECKER_SIZE = 8u;

fn srgb_encode(linear: vec3<f32>) -> vec3<f32> {
    let low = linear * 12.92;
    let high = 1.055 * pow(linear, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, linear <= vec3<f32>(0.0031308));
}

// Display bound on sample magnitudes (MAX_FINITE_SAMPLE in core): sums and matrices stay far
// from overflow, so an opaque layer still hides what is below it. Export maps ±inf samples to it.
const MAX_FINITE = 65504.0;
// Largest finite f32: what export saturates overflowing composites to.
const F32_MAX = 0x1.fffffep+127f;

// Rec.709 constants at full precision (same as core).
const REC709_ALPHA = 1.0992968;
const REC709_BETA = 0.018053968;

// Components that are ±inf or NaN. Tests the bits: `v != v` may be optimized away.
fn non_finite(v: vec4<f32>) -> vec4<bool> {
    let bits = bitcast<vec4<u32>>(v);
    return (bits & vec4<u32>(0x7f800000u)) == vec4<u32>(0x7f800000u);
}

// Components that are NaN.
fn is_nan(v: vec4<f32>) -> vec4<bool> {
    return non_finite(v) & ((bitcast<vec4<u32>>(v) & vec4<u32>(0x007fffffu)) != vec4<u32>(0u));
}

// Number of non-finite components of `v` among those selected by `mask`.
fn count_non_finite(v: vec4<f32>, mask: vec4<bool>) -> u32 {
    let n = select(vec4<u32>(0u), vec4<u32>(1u), non_finite(v) & mask);
    return n.x + n.y + n.z + n.w;
}

// Sample values: NaN → 0 and ±inf → ±MAX_FINITE. For display, finite values are clamped to
// ±MAX_FINITE too; for export (`unbounded`, ADR 0008), they are kept. ±inf does not become
// ±F32_MAX: through a color matrix it would swamp the pixel's other channels (f32 rounding at
// 1e38 is about 1e31). Same rule as the CPU reference compositor.
fn finite(v: vec4<f32>, unbounded: bool) -> vec4<f32> {
    var mapped = select(v, sign(v) * MAX_FINITE, non_finite(v));
    if !unbounded {
        mapped = clamp(mapped, vec4<f32>(-MAX_FINITE), vec4<f32>(MAX_FINITE));
    }
    return select(mapped, vec4<f32>(0.0), is_nan(v));
}

// Export: computed values that overflowed the f32 range saturate to ±F32_MAX (NaN, as inf − inf,
// to 0), counted in `count`.
fn saturated(v: vec4<f32>, count: ptr<function, u32>) -> vec4<f32> {
    *count += count_non_finite(v, vec4<bool>(true));
    let mapped = select(v, sign(v) * F32_MAX, non_finite(v));
    return select(mapped, vec4<f32>(0.0), is_nan(v));
}

// ICC parametric curve for x ≥ 0: g = t.y, a = t.z, b = t.w, c = t2.x, d = t2.y, e = t2.z,
// f = t2.w.
fn parametric(x: vec3<f32>, t: vec4<f32>, t2: vec4<f32>) -> vec3<f32> {
    let curve = pow(max(t.z * x + t.w, vec3<f32>(0.0)), vec3<f32>(t.y)) + t2.z;
    return select(curve, t2.x * x + t2.w, x < vec3<f32>(t2.y));
}

// Encoded → linear, mirrored for negative values (see TransferFunction::decode in core).
fn decode_transfer(v: vec3<f32>, t: vec4<f32>, t2: vec4<f32>) -> vec3<f32> {
    if u32(t.x) == TF_PARAMETRIC {
        // Mirrored about the value at 0, which offsets make non-zero.
        let y0 = parametric(vec3<f32>(0.0), t, t2);
        return select(parametric(v, t, t2), 2.0 * y0 - parametric(-v, t, t2), v < vec3<f32>(0.0));
    }
    let x = abs(v);
    var y = x;
    switch u32(t.x) {
        case TF_SRGB: {
            y = select(pow((x + 0.055) / 1.055, vec3<f32>(2.4)), x / 12.92, x <= vec3<f32>(0.04045));
        }
        case TF_GAMMA: {
            y = pow(x, vec3<f32>(t.y));
        }
        case TF_REC709: {
            let high = pow((x + (REC709_ALPHA - 1.0)) / REC709_ALPHA, vec3<f32>(1.0 / 0.45));
            y = select(high, x / 4.5, x < vec3<f32>(4.5 * REC709_BETA));
        }
        case TF_PQ: {
            let m1 = 2610.0 / 16384.0;
            let m2 = 2523.0 / 4096.0 * 128.0;
            let c1 = 3424.0 / 4096.0;
            let c2 = 2413.0 / 4096.0 * 32.0;
            let c3 = 2392.0 / 4096.0 * 32.0;
            // Defined on [0, 1]: saturate above, where the formula would divide by ≤ 0.
            let p = pow(min(x, vec3<f32>(1.0)), vec3<f32>(1.0 / m2));
            let nits = pow(max(p - c1, vec3<f32>(0.0)) / (c2 - c3 * p), vec3<f32>(1.0 / m1)) * 10000.0;
            y = nits / 203.0;
        }
        case TF_HLG: {
            let a = 0.17883277;
            let b = 1.0 - 4.0 * a;
            let c = 0.5599107;
            let scene = select((exp((x - c) / a) + b) / 12.0, x * x / 3.0, x <= vec3<f32>(0.5));
            y = scene / 0.26496252;
        }
        default: {}
    }
    return select(y, -y, v < vec3<f32>(0.0));
}

fn load_texel(layer: Layer, texel: vec2<u32>, slot: u32) -> vec4<f32> {
    let local = texel % TILE_SIZE;
    switch layer.format {
        case FORMAT_UNORM8: {
            return textureLoad(tiles_unorm8, local, slot, 0);
        }
        case FORMAT_UINT16: {
            return vec4<f32>(textureLoad(tiles_uint16, local, slot, 0)) / 65535.0;
        }
        case FORMAT_FLOAT16: {
            return textureLoad(tiles_float16, local, slot, 0);
        }
        default: {
            return textureLoad(tiles_float32, local, slot, 0);
        }
    }
}

// Premultiplied working-space color of one texel of a raster layer's planned level.
// Transparent outside the image or where no tile is resident. `unbounded` (export): see
// `finite`; the non-finite values replaced are added to `count` (stored samples and decoded
// values, like the CPU reference compositor). Display does not count.
fn texel_color(layer: Layer, at: vec2<i32>, unbounded: bool, count: ptr<function, u32>) -> vec4<f32> {
    if any(at < vec2<i32>(0)) || any(at >= vec2<i32>(layer.level_size)) {
        return vec4<f32>(0.0);
    }
    let texel = vec2<u32>(at);
    let tile = texel / TILE_SIZE;
    if any(tile < layer.tile_origin) || any(tile >= layer.tile_origin + layer.tile_count) {
        return vec4<f32>(0.0);
    }
    let local = tile - layer.tile_origin;
    let slot = tile_table[layer.table_offset + local.y * layer.tile_count.x + local.x];
    if slot == NO_TILE {
        return vec4<f32>(0.0);
    }
    let stored = load_texel(layer, texel, slot);
    let raw = finite(stored, unbounded);
    let a = clamp(raw.a, 0.0, 1.0);
    let premultiplied = (layer.flags & FLAG_PREMULTIPLIED) != 0u;
    let unpremultiply_first = premultiplied && u32(layer.transfer.x) != TF_LINEAR;
    var encoded = raw.rgb;
    if unpremultiply_first {
        // The file multiplied *encoded* values by alpha: decode the straight value, then
        // premultiply again (the matrix below is linear, so the order does not matter).
        encoded = select(vec3<f32>(0.0), raw.rgb / a, a > 0.0);
    }
    // Decoding can overflow (HLG is exponential, and a huge value raised to a power): mapped
    // like a sample, before the matrix.
    let decoded = vec4<f32>(decode_transfer(encoded, layer.transfer, layer.transfer2), 1.0);
    var linear = finite(decoded, unbounded).rgb;
    if unpremultiply_first || !premultiplied {
        linear = linear * a;
    }
    if unbounded {
        // Gray: one stored color sample, copied to green and blue.
        let gray = (layer.flags & FLAG_GRAY) != 0u;
        let samples = vec4<bool>(true, !gray, !gray, true);
        *count += count_non_finite(stored, samples) + count_non_finite(decoded, samples);
    }
    let rgb = vec3<f32>(dot(layer.m0.xyz, linear), dot(layer.m1.xyz, linear), dot(layer.m2.xyz, linear));
    let color = vec4<f32>(rgb, a);
    if unbounded {
        // Values near the f32 limit can still overflow through the matrix.
        return saturated(color, count);
    }
    return finite(color, false);
}

// Most texels read per axis for one output pixel. The planned level has 1 to 2 texels per output
// pixel, so a footprint spans at most 3; only a document zoomed out beyond its coarsest level
// needs more, and is then subsampled.
const MAX_FOOTPRINT_TEXELS = 4;

// Premultiplied working-space color of a raster layer over an output pixel's footprint (the
// document rectangle `lo`–`hi`): an area (box) filter. The texels it covers are averaged, each
// weighted by the area it covers, in linear light. Exact at 100% (one texel), free of aliasing
// when zoomed out, and pixels keep sharp, even edges when zoomed in.
fn sample_raster(layer: Layer, lo: vec2<f32>, hi: vec2<f32>) -> vec4<f32> {
    var uncounted = 0u;
    let origin = vec2<f32>(layer.offset);
    let a = (lo - origin) / layer.level_scale;
    let b = (hi - origin) / layer.level_scale;
    let first = vec2<i32>(floor(a));
    let last = min(vec2<i32>(ceil(b)) - 1, first + (MAX_FOOTPRINT_TEXELS - 1));
    var sum = vec4<f32>(0.0);
    var weight = 0.0;
    for (var y = first.y; y <= last.y; y++) {
        let wy = min(f32(y + 1), b.y) - max(f32(y), a.y);
        for (var x = first.x; x <= last.x; x++) {
            let w = (min(f32(x + 1), b.x) - max(f32(x), a.x)) * wy;
            sum += texel_color(layer, vec2<i32>(x, y), false, &uncounted) * w;
            weight += w;
        }
    }
    return sum / max(weight, 1e-12);
}

// Share of the output pixel that falls inside a raster layer's image (at its offset): 0 or 1
// for an exact texel, the covered area otherwise.
fn inside_raster(layer: Layer, footprint: Footprint) -> f32 {
    if footprint.exact {
        let t = footprint.texel - layer.offset;
        let inside = all(t >= vec2<i32>(0)) && all(t < vec2<i32>(layer.level_size));
        return select(0.0, 1.0, inside);
    }
    let origin = vec2<f32>(layer.offset);
    let extent = origin + vec2<f32>(layer.level_size) * layer.level_scale;
    let covered = max(min(footprint.hi, extent) - max(footprint.lo, origin), vec2<f32>(0.0));
    let area = max(footprint.hi - footprint.lo, vec2<f32>(1e-12));
    return (covered.x * covered.y) / (area.x * area.y);
}

// A layer's mask seen as a raster layer: gray, linear, opaque, so that its texels read as
// (coverage, coverage, coverage, 1).
fn mask_view(layer: Layer) -> Layer {
    var m = layer;
    m.kind = KIND_RASTER;
    m.opacity = 1.0;
    m.level_scale = layer.mask_level_scale;
    m.table_offset = layer.mask_table_offset;
    m.tile_origin = layer.mask_tile_origin;
    m.tile_count = layer.mask_tile_count;
    m.level_size = layer.mask_level_size;
    m.format = layer.mask_format;
    m.offset = layer.mask_offset;
    m.flags = FLAG_GRAY;
    m.transfer = vec4<f32>(f32(TF_LINEAR), 0.0, 0.0, 0.0);
    m.transfer2 = vec4<f32>(0.0);
    m.m0 = vec4<f32>(1.0, 0.0, 0.0, 0.0);
    m.m1 = vec4<f32>(0.0, 1.0, 0.0, 0.0);
    m.m2 = vec4<f32>(0.0, 0.0, 1.0, 0.0);
    return m;
}

// The mask's coverage over the footprint, in [0, 1]; 0 outside the mask image. Non-finite mask
// samples are not counted (like the CPU reference).
fn mask_coverage(layer: Layer, footprint: Footprint) -> f32 {
    let mask = mask_view(layer);
    var uncounted = 0u;
    var value = 0.0;
    if footprint.exact {
        value = texel_color(mask, footprint.texel - mask.offset, true, &uncounted).r;
    } else {
        value = sample_raster(mask, footprint.lo, footprint.hi).r;
    }
    return clamp(value, 0.0, 1.0);
}

// Blend modes (ADR 0012), the same math as slopshop_core::blend in f32. The MODE_* constants,
// FLAG_PERCEPTUAL, BLEND_SHIFT, DIVISION_EPSILON and the TO_BLEND* / FROM_BLEND* matrices (working space ↔
// linear sRGB, rows) are generated by `shader_source` in lib.rs.

// The sRGB curve extended to negative values by symmetry.
fn blend_encode(v: f32) -> f32 {
    let a = abs(v);
    var e = a * 12.92;
    if a > 0.0031308 {
        e = 1.055 * pow(a, 1.0 / 2.4) - 0.055;
    }
    return select(e, -e, v < 0.0);
}

fn blend_decode(e: f32) -> f32 {
    let a = abs(e);
    var v = a / 12.92;
    if a > 0.04045 {
        v = pow((a + 0.055) / 1.055, 2.4);
    }
    return select(v, -v, e < 0.0);
}

fn to_blend(c: vec3<f32>, perceptual: bool) -> vec3<f32> {
    if !perceptual {
        return c;
    }
    let s = vec3<f32>(dot(TO_BLEND0, c), dot(TO_BLEND1, c), dot(TO_BLEND2, c));
    return vec3<f32>(blend_encode(s.x), blend_encode(s.y), blend_encode(s.z));
}

fn from_blend(e: vec3<f32>, perceptual: bool) -> vec3<f32> {
    if !perceptual {
        return e;
    }
    let c = vec3<f32>(blend_decode(e.x), blend_decode(e.y), blend_decode(e.z));
    return vec3<f32>(dot(FROM_BLEND0, c), dot(FROM_BLEND1, c), dot(FROM_BLEND2, c));
}

fn unpremultiply(c: vec4<f32>) -> vec3<f32> {
    if c.a > 0.0 {
        return c.rgb / c.a;
    }
    return vec3<f32>(0.0);
}

fn screen_channel(b: f32, s: f32) -> f32 {
    return b + s - b * s;
}

fn hard_light_channel(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        return b * 2.0 * s;
    }
    return screen_channel(b, 2.0 * s - 1.0);
}

fn soft_light_channel(b: f32, s: f32) -> f32 {
    if s <= 0.5 {
        return 2.0 * b * s + b * b * (1.0 - 2.0 * s);
    }
    return 2.0 * b * (1.0 - s) + sqrt(max(b, 0.0)) * (2.0 * s - 1.0);
}

fn color_dodge_channel(b: f32, s: f32) -> f32 {
    if b <= DIVISION_EPSILON {
        return 0.0;
    }
    if s >= 1.0 - DIVISION_EPSILON {
        return 1.0;
    }
    return min(b / (1.0 - s), 1.0);
}

fn color_burn_channel(b: f32, s: f32) -> f32 {
    if b >= 1.0 - DIVISION_EPSILON {
        return 1.0;
    }
    if s <= DIVISION_EPSILON {
        return 0.0;
    }
    return 1.0 - min((1.0 - b) / s, 1.0);
}

// Modes that leave [0, 1] with in-range inputs: clamped in perceptual space, at 0 in linear.
fn clamp_mode(v: f32, perceptual: bool) -> f32 {
    if perceptual {
        return clamp(v, 0.0, 1.0);
    }
    return max(v, 0.0);
}

fn separable_channel(mode: u32, b: f32, s: f32, perceptual: bool) -> f32 {
    switch mode {
        case MODE_DARKEN: { return min(b, s); }
        case MODE_MULTIPLY: { return b * s; }
        case MODE_COLOR_BURN: { return color_burn_channel(b, s); }
        case MODE_LINEAR_BURN: { return clamp_mode(b + s - 1.0, perceptual); }
        case MODE_LIGHTEN: { return max(b, s); }
        case MODE_SCREEN: { return screen_channel(b, s); }
        case MODE_COLOR_DODGE: { return color_dodge_channel(b, s); }
        case MODE_LINEAR_DODGE: { return clamp_mode(b + s, perceptual); }
        case MODE_OVERLAY: { return hard_light_channel(s, b); }
        case MODE_SOFT_LIGHT: { return soft_light_channel(b, s); }
        case MODE_HARD_LIGHT: { return hard_light_channel(b, s); }
        case MODE_VIVID_LIGHT: {
            if s <= 0.5 {
                return color_burn_channel(b, 2.0 * s);
            }
            return color_dodge_channel(b, 2.0 * s - 1.0);
        }
        case MODE_LINEAR_LIGHT: { return clamp_mode(b + 2.0 * s - 1.0, perceptual); }
        case MODE_PIN_LIGHT: {
            if s <= 0.5 {
                return min(b, 2.0 * s);
            }
            return max(b, 2.0 * s - 1.0);
        }
        case MODE_HARD_MIX: { return select(0.0, 1.0, b + s >= 1.0); }
        case MODE_DIFFERENCE: { return abs(b - s); }
        case MODE_EXCLUSION: { return b + s - 2.0 * b * s; }
        case MODE_SUBTRACT: { return clamp_mode(b - s, perceptual); }
        case MODE_DIVIDE: {
            if s > DIVISION_EPSILON {
                return clamp_mode(b / s, perceptual);
            }
            // Division by zero: white, as Photoshop.
            return select(0.0, 1.0, b > DIVISION_EPSILON);
        }
        default: { return s; }
    }
}

fn lum(c: vec3<f32>) -> f32 {
    return dot(vec3<f32>(0.3, 0.59, 0.11), c);
}

fn sat(c: vec3<f32>) -> f32 {
    return max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b);
}

fn clip_color(c: vec3<f32>, perceptual: bool) -> vec3<f32> {
    let l = lum(c);
    let n = min(min(c.r, c.g), c.b);
    let x = max(max(c.r, c.g), c.b);
    var r = c;
    if n < 0.0 && l - n > 0.0 {
        r = l + (r - l) * l / (l - n);
    }
    if perceptual && x > 1.0 && x - l > 0.0 {
        r = l + (r - l) * (1.0 - l) / (x - l);
    }
    return r;
}

fn set_lum(c: vec3<f32>, l: f32, perceptual: bool) -> vec3<f32> {
    return clip_color(c + (l - lum(c)), perceptual);
}

fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
    let hi = max(max(c.r, c.g), c.b);
    let lo = min(min(c.r, c.g), c.b);
    if hi <= lo {
        return vec3<f32>(0.0);
    }
    return (c - lo) * s / (hi - lo);
}

fn blend_color(mode: u32, cb: vec3<f32>, cs: vec3<f32>, perceptual: bool) -> vec3<f32> {
    let one = vec3<f32>(1.0);
    switch mode {
        case MODE_DARKER_COLOR: { return select(cb, cs, dot(cs, one) < dot(cb, one)); }
        case MODE_LIGHTER_COLOR: { return select(cb, cs, dot(cs, one) > dot(cb, one)); }
        case MODE_HUE: { return set_lum(set_sat(cs, sat(cb)), lum(cb), perceptual); }
        case MODE_SATURATION: { return set_lum(set_sat(cb, sat(cs)), lum(cb), perceptual); }
        case MODE_COLOR: { return set_lum(cs, lum(cb), perceptual); }
        case MODE_LUMINOSITY: { return set_lum(cb, lum(cs), perceptual); }
        default: {
            return vec3<f32>(
                separable_channel(mode, cb.r, cs.r, perceptual),
                separable_channel(mode, cb.g, cs.g, perceptual),
                separable_channel(mode, cb.b, cs.b, perceptual),
            );
        }
    }
}

// Combine a layer (`src`, premultiplied working space, opacity applied) with what is below.
fn blend_layer(src: vec4<f32>, dst: vec4<f32>, flags: u32) -> vec4<f32> {
    let mode = (flags >> BLEND_SHIFT) & 0xffu;
    let perceptual = (flags & FLAG_PERCEPTUAL) != 0u;
    // Exact paths: linear normal is "over"; an opaque normal layer, or one over nothing, is
    // its own color in either space.
    // Dissolve blends like normal: its pixels were already chosen (`dissolve`).
    if (mode == MODE_NORMAL || mode == MODE_DISSOLVE)
        && (!perceptual || src.a >= 1.0 || dst.a <= 0.0) {
        return src + dst * (1.0 - src.a);
    }
    if src.a <= 0.0 {
        return dst;
    }
    let alpha_s = min(src.a, 1.0);
    let alpha_b = clamp(dst.a, 0.0, 1.0);
    let cs = to_blend(unpremultiply(src), perceptual);
    let cb = to_blend(unpremultiply(dst), perceptual);
    let mixed = blend_color(mode, cb, cs, perceptual);
    let alpha_o = alpha_s + alpha_b - alpha_s * alpha_b;
    var co = vec3<f32>(0.0);
    if alpha_o > 0.0 {
        co = (alpha_s * (1.0 - alpha_b) * cs + alpha_b * (1.0 - alpha_s) * cb
            + alpha_s * alpha_b * mixed) / alpha_o;
    }
    return vec4<f32>(from_blend(co, perceptual) * alpha_o, alpha_o);
}

// Combine a clipped layer atop its clipping group (ADR 0016): blended like `blend_layer`, the
// result keeping `dst`'s coverage (Blender::blend_atop in core).
fn blend_atop(src: vec4<f32>, dst: vec4<f32>, flags: u32) -> vec4<f32> {
    if dst.a <= 0.0 {
        return dst;
    }
    let mode = (flags >> BLEND_SHIFT) & 0xffu;
    let perceptual = (flags & FLAG_PERCEPTUAL) != 0u;
    if (mode == MODE_NORMAL || mode == MODE_DISSOLVE) && (!perceptual || src.a >= 1.0) {
        return src * dst.a + dst * (1.0 - src.a);
    }
    if src.a <= 0.0 {
        return dst;
    }
    let alpha_s = min(src.a, 1.0);
    let alpha_b = clamp(dst.a, 0.0, 1.0);
    let cs = to_blend(unpremultiply(src), perceptual);
    let cb = to_blend(unpremultiply(dst), perceptual);
    let mixed = blend_color(mode, cb, cs, perceptual);
    let co = alpha_s * (1.0 - alpha_b) * cs + alpha_s * alpha_b * mixed + (1.0 - alpha_s) * cb;
    return vec4<f32>(from_blend(co, perceptual) * alpha_b, alpha_b);
}

// Blend `src` onto `dst` as its flags say: atop for a clipped layer.
fn combine(src: vec4<f32>, dst: vec4<f32>, flags: u32) -> vec4<f32> {
    if (flags & FLAG_ATOP) != 0u {
        return blend_atop(src, dst, flags);
    }
    return blend_layer(src, dst, flags);
}

// Fade from `below` to `above` by `t` (a pass-through group's opacity and mask, ADR 0015): a
// premultiplied mix in the blend space, exact at 0 and 1 (Blender::fade in core).
fn fade(below: vec4<f32>, above: vec4<f32>, t: f32, perceptual: bool) -> vec4<f32> {
    if t >= 1.0 {
        return above;
    }
    if !(t > 0.0) {
        return below;
    }
    if !perceptual {
        return below + (above - below) * t;
    }
    let alpha_b = clamp(below.a, 0.0, 1.0);
    let alpha_a = clamp(above.a, 0.0, 1.0);
    let alpha = alpha_b + (alpha_a - alpha_b) * t;
    if alpha <= 0.0 {
        return vec4<f32>(0.0);
    }
    let cb = to_blend(unpremultiply(below), true);
    let ca = to_blend(unpremultiply(above), true);
    let co = (cb * alpha_b * (1.0 - t) + ca * alpha_a * t) / alpha;
    return vec4<f32>(from_blend(co, true) * alpha, alpha);
}

// Dissolve's noise at a document pixel, in [0, 1): the same hash as
// slopshop_core::blend::dissolve_noise (lowbias32), 24 bits so that it is exact in f32.
fn dissolve_noise(p: vec2<u32>) -> f32 {
    var h = (p.x * 0x8da6b343u) ^ (p.y * 0xd8163841u);
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h >> 8u) / 16777216.0;
}

// Dissolve: the layer's pixel kept whole (straight color, alpha 1) or dropped, by its coverage
// against the noise of the document pixel. Views where an output pixel covers several document
// pixels show the average instead, which is normal blending.
fn dissolve(src: vec4<f32>, footprint: Footprint) -> vec4<f32> {
    var pixel: vec2<i32>;
    if footprint.exact {
        pixel = footprint.texel;
    } else {
        let size = footprint.hi - footprint.lo;
        if max(size.x, size.y) > 1.0 {
            return src;
        }
        pixel = vec2<i32>(floor((footprint.lo + footprint.hi) * 0.5));
    }
    if any(pixel < vec2<i32>(0)) || !(dissolve_noise(vec2<u32>(pixel)) < min(src.a, 1.0)) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(unpremultiply(src), 1.0);
}

// Where an output pixel samples the document.
struct Footprint {
    // Display: the document rectangle covered, area-filtered at each layer's planned level.
    lo: vec2<f32>,
    hi: vec2<f32>,
    // Export (`exact`): exactly this level-0 texel, with the unbounded `finite` rule.
    texel: vec2<i32>,
    exact: bool,
}

// Premultiplied working-space color of the first `layer_count` layers ("over", bottom to top).
// Groups (ADR 0015) push the accumulator and pop it back, combined with what they made.
// Export adds the non-finite values it replaces to `count`.
fn composite(footprint: Footprint, layer_count: u32, count: ptr<function, u32>) -> vec4<f32> {
    var acc = vec4<f32>(0.0);
    var stack: array<vec4<f32>, MAX_GROUP_DEPTH>;
    var depth = 0u;
    for (var i = 0u; i < layer_count; i++) {
        let layer = layers[i];
        if layer.kind == KIND_GROUP_BEGIN {
            // The engine never nests deeper than MAX_GROUP_DEPTH.
            stack[min(depth, MAX_GROUP_DEPTH - 1u)] = acc;
            depth++;
            if (layer.flags & FLAG_ISOLATED) != 0u {
                acc = vec4<f32>(0.0);
            }
            continue;
        }
        if layer.kind == KIND_GROUP_END {
            if depth == 0u {
                continue;
            }
            depth--;
            let below = stack[min(depth, MAX_GROUP_DEPTH - 1u)];
            var coverage = layer.opacity;
            if (layer.flags & FLAG_MASK) != 0u {
                coverage *= mask_coverage(layer, footprint);
            }
            if (layer.flags & FLAG_ISOLATED) != 0u {
                var src = acc * coverage;
                if ((layer.flags >> BLEND_SHIFT) & 0xffu) == MODE_DISSOLVE {
                    src = dissolve(src, footprint);
                }
                acc = combine(src, below, layer.flags);
            } else {
                acc = fade(below, acc, coverage, (layer.flags & FLAG_PERCEPTUAL) != 0u);
            }
            if footprint.exact {
                acc = saturated(acc, count);
            }
            continue;
        }
        var src = layer.color;
        if layer.kind == KIND_RASTER {
            if footprint.exact {
                src = texel_color(layer, footprint.texel - layer.offset, true, count);
            } else {
                src = sample_raster(layer, footprint.lo, footprint.hi);
            }
            // A mask made from the layer's transparency replaces its alpha (ADR 0014).
            // Only where the image is: outside it the layer stays transparent.
            if (layer.flags & FLAG_IGNORE_ALPHA) != 0u {
                src = vec4<f32>(unpremultiply(src), 1.0) * inside_raster(layer, footprint);
            }
            src = src * layer.opacity;
        }
        if (layer.flags & FLAG_MASK) != 0u {
            src = src * mask_coverage(layer, footprint);
        }
        if ((layer.flags >> BLEND_SHIFT) & 0xffu) == MODE_DISSOLVE {
            src = dissolve(src, footprint);
        }
        acc = combine(src, acc, layer.flags);
        if footprint.exact {
            // Unbounded values can overflow here (color above alpha, premultiplied): keep them
            // finite, or an opaque layer above would compute inf × 0 = NaN.
            acc = saturated(acc, count);
        }
    }
    return acc;
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.out_size.x || id.y >= params.out_size.y {
        return;
    }

    // The output pixel's footprint in the document, clipped to the document: layers are averaged
    // over the part inside it, and the document is blended with the pasteboard by the share of
    // the pixel it covers. Edge pixels are then antialiased against the pasteboard, never
    // against the checkerboard (whose squares would make the edges shimmer while navigating).
    let corner = params.origin + vec2<f32>(id.xy) * params.scale;
    let doc = vec2<f32>(params.doc_size);
    let lo = max(corner, vec2<f32>(0.0));
    let hi = min(corner + params.scale, doc);
    let inside = max(hi - lo, vec2<f32>(0.0));
    let coverage = (inside.x * inside.y) / (params.scale * params.scale);

    var color = PASTEBOARD;
    if coverage > 0.0 {
        var uncounted = 0u;
        let acc = composite(Footprint(lo, hi, vec2<i32>(0), false), params.layer_count, &uncounted);
        // Working space → display (a linear map, so it commutes with premultiplied "over").
        let display = vec3<f32>(
            dot(params.display0.xyz, acc.rgb),
            dot(params.display1.xyz, acc.rgb),
            dot(params.display2.xyz, acc.rgb),
        );
        let checker = ((id.x / CHECKER_SIZE) + (id.y / CHECKER_SIZE)) % 2u;
        let background = select(CHECKER_DARK, CHECKER_LIGHT, checker == 0u);
        color = mix(PASTEBOARD, display + background * (1.0 - acc.a), min(coverage, 1.0));
    }

    // Clipping only happens here, at the display boundary.
    let encoded = srgb_encode(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)));
    output[id.y * params.out_size.x + id.x] = pack4x8unorm(vec4<f32>(encoded, 1.0));
}

// Export: premultiplied working-space RGBA of a document region at full resolution. Every raster
// layer is planned at level 0, so one output pixel is exactly one texel (no filtering). No
// display matrix, background or clipping.
@compute @workgroup_size(8, 8)
fn export_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= export_params.size.x || id.y >= export_params.size.y {
        return;
    }
    let texel = vec2<i32>(export_params.origin + id.xy);
    let footprint = Footprint(vec2<f32>(0.0), vec2<f32>(0.0), texel, true);
    var count = 0u;
    export_output[id.y * export_params.size.x + id.x] =
        composite(footprint, export_params.layer_count, &count);
    if count > 0u {
        // 64-bit add: carry into the high word when the low word wraps.
        let low = atomicAdd(&export_non_finite[0], count);
        if low > 0xffffffffu - count {
            atomicAdd(&export_non_finite[1], 1u);
        }
    }
}
