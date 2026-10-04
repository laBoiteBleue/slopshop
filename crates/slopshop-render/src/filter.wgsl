// Filters on the GPU (ADR 0035): a Gaussian blur of an 8-bit RGBA sRGB region in a perceptual
// document, whose stored values are the blend values. Rows first, decoding the 8-bit straight
// pixels into premultiplied values; then columns, encoding them back into 8-bit straight pixels
// as the CPU writes them. The region's edges repeat outward (the CPU's rule).

struct Params {
    width: u32,
    height: u32,
    // Half the kernel's width: `weights` holds 2 * reach + 1 values.
    reach: u32,
    _pad: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> weights: array<f32>;
@group(0) @binding(2) var<storage, read> packed: array<u32>;
@group(0) @binding(3) var<storage, read_write> rows: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> output: array<u32>;

fn premultiplied(pixel: u32) -> vec4<f32> {
    let c = unpack4x8unorm(pixel);
    return vec4<f32>(c.rgb * c.a, c.a);
}

@compute @workgroup_size(16, 16)
fn rows_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let reach = i32(params.reach);
    let last = i32(params.width) - 1;
    let line = id.y * params.width;
    var sum = vec4<f32>(0.0);
    for (var k = -reach; k <= reach; k++) {
        let x = u32(clamp(i32(id.x) + k, 0, last));
        sum += weights[u32(k + reach)] * premultiplied(packed[line + x]);
    }
    rows[line + id.x] = sum;
}

// A byte as the CPU rounds it: half away from zero, after clamping.
fn byte(v: f32) -> u32 {
    return u32(floor(clamp(v, 0.0, 1.0) * 255.0 + 0.5));
}

@compute @workgroup_size(16, 16)
fn columns_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let reach = i32(params.reach);
    let last = i32(params.height) - 1;
    var sum = vec4<f32>(0.0);
    for (var k = -reach; k <= reach; k++) {
        let y = u32(clamp(i32(id.y) + k, 0, last));
        sum += weights[u32(k + reach)] * rows[y * params.width + id.x];
    }
    // Straight again: transparent stays all zero.
    var pixel = 0u;
    let alpha = clamp(sum.a, 0.0, 1.0);
    if alpha > 0.0 {
        let color = sum.rgb / sum.a;
        pixel = byte(color.r) | (byte(color.g) << 8u) | (byte(color.b) << 16u) | (byte(alpha) << 24u);
    }
    output[id.y * params.width + id.x] = pixel;
}
