// Compositor: one invocation per output pixel.
//
// Layers are composited in the working space (linear Rec.2020, unbounded, premultiplied alpha;
// ADR 0007). Raster tiles are sampled in their source encoding and converted here: transfer
// function decode, then a 3×3 matrix to the working space.
//
// Two entry points share that code:
// - `main`, the viewport: the result is converted to the display space (linear sRGB),
//   composited over a transparency checkerboard and encoded to sRGB 8-bit. Clipping only happens
//   at that last step: it is a *view transform*, the document is never converted.
// - `export_main`, export (ADR 0008): the working-space values themselves, as f32, one level-0
//   texel per output pixel, finite values never clamped.

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

const FLAG_PREMULTIPLIED: u32 = 1u;

// Transfer function kinds (transfer_fields in lib.rs).
const TF_LINEAR: u32 = 0u;
const TF_SRGB: u32 = 1u;
const TF_GAMMA: u32 = 2u;
const TF_REC709: u32 = 3u;
const TF_PARAMETRIC: u32 = 4u;
const TF_PQ: u32 = 5u;
const TF_HLG: u32 = 6u;

// 144 bytes; keep in sync with `LayerFields` in lib.rs.
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
// from overflow, so an opaque layer still hides what is below it.
const MAX_FINITE = 65504.0;
// Largest finite f32: what export maps ±inf to.
const F32_MAX = 0x1.fffffep+127f;

// Rec.709 constants at full precision (same as core).
const REC709_ALPHA = 1.0992968;
const REC709_BETA = 0.018053968;

// NaN → 0. Then, for display, every value (finite or not) is clamped to ±MAX_FINITE; for
// export (`unbounded`, ADR 0008), only ±inf are mapped, to ±F32_MAX, and finite values are
// kept. Tests the bits: `v != v` may be optimized away.
fn finite(v: vec4<f32>, unbounded: bool) -> vec4<f32> {
    let bits = bitcast<vec4<u32>>(v);
    let special = (bits & vec4<u32>(0x7f800000u)) == vec4<u32>(0x7f800000u);
    let nan = special & ((bits & vec4<u32>(0x007fffffu)) != vec4<u32>(0u));
    var mapped: vec4<f32>;
    if unbounded {
        mapped = select(v, sign(v) * F32_MAX, special);
    } else {
        let clamped = clamp(v, vec4<f32>(-MAX_FINITE), vec4<f32>(MAX_FINITE));
        mapped = select(clamped, sign(v) * MAX_FINITE, special);
    }
    return select(mapped, vec4<f32>(0.0), nan);
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
// Transparent outside the image or where no tile is resident. `unbounded`: see `finite`.
fn texel_color(layer: Layer, at: vec2<i32>, unbounded: bool) -> vec4<f32> {
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
    let raw = finite(load_texel(layer, texel, slot), unbounded);
    let a = clamp(raw.a, 0.0, 1.0);
    let premultiplied = (layer.flags & FLAG_PREMULTIPLIED) != 0u;
    var linear: vec3<f32>;
    if premultiplied && u32(layer.transfer.x) != TF_LINEAR {
        // The file multiplied *encoded* values by alpha: decode the straight value, then
        // premultiply again (the matrix below is linear, so the order does not matter).
        let straight = select(vec3<f32>(0.0), raw.rgb / a, a > 0.0);
        linear = decode_transfer(straight, layer.transfer, layer.transfer2) * a;
    } else {
        linear = decode_transfer(raw.rgb, layer.transfer, layer.transfer2);
        if !premultiplied {
            linear = linear * a;
        }
    }
    let rgb = vec3<f32>(dot(layer.m0.xyz, linear), dot(layer.m1.xyz, linear), dot(layer.m2.xyz, linear));
    // Decoding can still overflow (HLG is exponential).
    return finite(vec4<f32>(rgb, a), unbounded);
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
    let a = lo / layer.level_scale;
    let b = hi / layer.level_scale;
    let first = vec2<i32>(floor(a));
    let last = min(vec2<i32>(ceil(b)) - 1, first + (MAX_FOOTPRINT_TEXELS - 1));
    var sum = vec4<f32>(0.0);
    var weight = 0.0;
    for (var y = first.y; y <= last.y; y++) {
        let wy = min(f32(y + 1), b.y) - max(f32(y), a.y);
        for (var x = first.x; x <= last.x; x++) {
            let w = (min(f32(x + 1), b.x) - max(f32(x), a.x)) * wy;
            sum += texel_color(layer, vec2<i32>(x, y), false) * w;
            weight += w;
        }
    }
    return sum / max(weight, 1e-12) * layer.opacity;
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
fn composite(footprint: Footprint, layer_count: u32) -> vec4<f32> {
    var acc = vec4<f32>(0.0);
    for (var i = 0u; i < layer_count; i++) {
        let layer = layers[i];
        var src = layer.color;
        if layer.kind == KIND_RASTER {
            if footprint.exact {
                src = texel_color(layer, footprint.texel, true) * layer.opacity;
            } else {
                src = sample_raster(layer, footprint.lo, footprint.hi);
            }
        }
        acc = src + acc * (1.0 - src.a);
        if footprint.exact {
            // Unbounded values can overflow here (color above alpha, premultiplied): keep them
            // finite, or an opaque layer above would compute inf × 0 = NaN.
            acc = finite(acc, true);
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
        let acc = composite(Footprint(lo, hi, vec2<i32>(0), false), params.layer_count);
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
    export_output[id.y * export_params.size.x + id.x] =
        composite(footprint, export_params.layer_count);
}
