// Viewport compositor: one invocation per output pixel.
//
// Layers are composited in linear light (premultiplied alpha) over a transparency
// checkerboard, then encoded to sRGB 8-bit for display. The encoding is a *view transform*:
// the document itself is never converted.

struct Params {
    // Document coordinate of the output's top-left corner.
    origin: vec2<f32>,
    // Document pixels per output pixel.
    scale: f32,
    layer_count: u32,
    out_size: vec2<u32>,
    doc_size: vec2<u32>,
}

@group(0) @binding(0) var<uniform> params: Params;
// Visible layers, bottom to top: linear RGBA, premultiplied, opacity already applied.
@group(0) @binding(1) var<storage, read> layers: array<vec4<f32>>;
// Output pixels, tightly packed, RGBA8 sRGB (one u32 per pixel, R in the lowest byte).
@group(0) @binding(2) var<storage, read_write> output: array<u32>;

// Linear-light display colors.
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

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.out_size.x || id.y >= params.out_size.y {
        return;
    }

    let p = params.origin + (vec2<f32>(id.xy) + 0.5) * params.scale;
    let doc = vec2<f32>(params.doc_size);

    var color = PASTEBOARD;
    if all(p >= vec2<f32>(0.0)) && all(p < doc) {
        var acc = vec4<f32>(0.0);
        for (var i = 0u; i < params.layer_count; i++) {
            let src = layers[i];
            acc = src + acc * (1.0 - src.a);
        }
        let checker = ((id.x / CHECKER_SIZE) + (id.y / CHECKER_SIZE)) % 2u;
        let background = select(CHECKER_DARK, CHECKER_LIGHT, checker == 0u);
        color = acc.rgb + background * (1.0 - acc.a);
    }

    // Clipping only happens here, at the display boundary.
    let encoded = srgb_encode(clamp(color, vec3<f32>(0.0), vec3<f32>(1.0)));
    output[id.y * params.out_size.x + id.x] = pack4x8unorm(vec4<f32>(encoded, 1.0));
}
