// Filters on the GPU (ADR 0035): a Gaussian blur of an 8-bit RGBA sRGB region in a perceptual
// document, whose stored values are the blend values, and the filters made from it. Rows first,
// decoding the 8-bit straight pixels into premultiplied values; then columns, each pixel made
// from itself and its blur as the CPU makes it (`Filter::finish`), encoded back into 8-bit
// straight pixels as the CPU writes them. The region's edges repeat outward (the CPU's rule).

struct Params {
    width: u32,
    height: u32,
    // Half the kernel's width: `weights` holds 2 * reach + 1 values.
    reach: u32,
    // What a pixel becomes from its blur: 0 the blur (Gaussian Blur), 1 Unsharp Mask, 2 High
    // Pass, 4 kept in `aux` for the next pass (Texture's fine blur), 5 Clarity and Texture from
    // it and this blur. (`line_main`, Motion Blur, has its own entry point: `reach` is its
    // samples' count, `weights` their offsets, x then y; so has `noise_main`, Add Noise:
    // `weights` is the crop's map to the document, a to f; and `median_main`, Dust & Scratches:
    // `reach` is its radius.)
    mode: u32,
    // Unsharp Mask's and Add Noise's amount (percent), Unsharp Mask's and Dust & Scratches'
    // threshold (levels of 8 bits); for Clarity and Texture, Texture's strength and Clarity's
    // (its fraction of 1 times `CLARITY_STRENGTH`).
    amount: f32,
    threshold: f32,
    // Add Noise's seed, and its flags: 1 Gaussian, 2 monochromatic.
    seed: u32,
    flags: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> weights: array<f32>;
@group(0) @binding(2) var<storage, read> packed: array<u32>;
// The rows' blur; past `width * height`, a blur kept between two passes (Texture's, for
// Clarity's pass): devices may bind no more than four storage buffers.
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

// A premultiplied color's straight color; black where transparent.
fn straight(p: vec4<f32>) -> vec3<f32> {
    if p.a > 0.0 {
        return p.rgb / p.a;
    }
    return vec3<f32>(0.0);
}

// The pixel `original` made from its blur `blurred` (`Filter::finish`): the colors straight,
// the pixel's alpha kept.
fn finish(original: vec4<f32>, blurred: vec4<f32>) -> vec4<f32> {
    let o = straight(original);
    let b = straight(blurred);
    let alpha = original.a;
    if params.mode == 1u {
        // Below the threshold on every channel, the pixel is left as it is.
        let level = params.threshold / 255.0;
        if alpha <= 0.0 || all(abs(o - b) < vec3<f32>(level)) {
            return original;
        }
        let k = params.amount / 100.0;
        return vec4<f32>((o + k * (o - b)) * alpha, alpha);
    }
    return vec4<f32>((o - b + 0.5) * alpha, alpha);
}

// Clarity and Texture (`Filter::finish`): the color pushed from its fine blur by Texture, and
// from its broad blur by Clarity in the midtones (Rec. 709 luma), alpha kept.
fn clarity(original: vec4<f32>, fine: vec4<f32>, broad: vec4<f32>) -> vec4<f32> {
    let alpha = original.a;
    if alpha <= 0.0 {
        return original;
    }
    let o = straight(original);
    let luma = dot(o, vec3<f32>(0.2126, 0.7152, 0.0722));
    let midtones = clamp(1.0 - (2.0 * luma - 1.0) * (2.0 * luma - 1.0), 0.0, 1.0);
    let c = o + params.amount * (o - straight(fine)) + params.threshold * midtones * (o - straight(broad));
    return vec4<f32>(c * alpha, alpha);
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
    let i = id.y * params.width + id.x;
    let kept = params.width * params.height + i;
    if params.mode == 4u {
        rows[kept] = sum;
        return;
    }
    if params.mode == 5u {
        sum = clarity(premultiplied(packed[i]), rows[kept], sum);
    } else if params.mode != 0u {
        sum = finish(premultiplied(packed[i]), sum);
    }
    output[i] = encoded(sum);
}

// A premultiplied value as an 8-bit straight pixel; transparent stays all zero.
fn encoded(sum: vec4<f32>) -> u32 {
    let alpha = clamp(sum.a, 0.0, 1.0);
    if alpha <= 0.0 {
        return 0u;
    }
    let color = sum.rgb / sum.a;
    return byte(color.r) | (byte(color.g) << 8u) | (byte(color.b) << 16u) | (byte(alpha) << 24u);
}

// The premultiplied pixel (`x`, `y`) of the input, the region's edges repeating outward.
fn input(x: i32, y: i32) -> vec4<f32> {
    let cx = u32(clamp(x, 0, i32(params.width) - 1));
    let cy = u32(clamp(y, 0, i32(params.height) - 1));
    return premultiplied(packed[cy * params.width + cx]);
}

// Motion Blur: the average of the line's samples, each read bilinearly (`Line` on the CPU).
@compute @workgroup_size(16, 16)
fn line_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let last = vec2<f32>(f32(params.width - 1u), f32(params.height - 1u));
    var sum = vec4<f32>(0.0);
    for (var i = 0u; i < params.reach; i++) {
        let at = clamp(
            vec2<f32>(f32(id.x) + weights[2u * i], f32(id.y) + weights[2u * i + 1u]),
            vec2<f32>(0.0),
            last,
        );
        let p0 = floor(at);
        let f = at - p0;
        let x0 = i32(p0.x);
        let y0 = i32(p0.y);
        let top = mix(input(x0, y0), input(x0 + 1, y0), f.x);
        let bottom = mix(input(x0, y0 + 1), input(x0 + 1, y0 + 1), f.x);
        sum += mix(top, bottom, f.y);
    }
    output[id.y * params.width + id.x] = encoded(sum / f32(params.reach));
}

// A 32-bit integer hash, as the CPU's (`filter::hash`).
fn hash(x: u32) -> u32 {
    let h = x * 747796405u + 2891336453u;
    let m = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    return (m >> 22u) ^ m;
}

// A uniform value of (0, 1) for document pixel `at`, `channel` and `draw` (`filter::noise`).
fn uniform_at(at: vec2<i32>, channel: u32, draw: u32) -> f32 {
    let h = hash(
        bitcast<u32>(at.x) ^ hash(bitcast<u32>(at.y) ^ hash(params.seed ^ hash(channel * 2u + draw))),
    );
    return (f32(h >> 8u) + 0.5) / 16777216.0;
}

// The noise of one channel: uniform within +-0.5, or Gaussian of the same variance.
fn noise_of(at: vec2<i32>, channel: u32) -> f32 {
    if (params.flags & 1u) != 0u {
        let u = uniform_at(at, channel, 0u);
        let v = uniform_at(at, channel, 1u);
        return sqrt(-2.0 * log(u)) * cos(6.283185307179586 * v) / sqrt(12.0);
    }
    return uniform_at(at, channel, 0u) - 0.5;
}

// Add Noise: each color moved by the noise of the document pixel it shows, alpha kept.
@compute @workgroup_size(16, 16)
fn noise_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let original = premultiplied(packed[id.y * params.width + id.x]);
    var result = original;
    if original.a > 0.0 {
        let x = f32(id.x) + 0.5;
        let y = f32(id.y) + 0.5;
        let at = vec2<i32>(
            i32(floor(weights[0] * x + weights[2] * y + weights[4])),
            i32(floor(weights[1] * x + weights[3] * y + weights[5])),
        );
        var n = vec3<f32>(noise_of(at, 0u));
        if (params.flags & 2u) == 0u {
            n = vec3<f32>(n.x, noise_of(at, 1u), noise_of(at, 2u));
        }
        let color = straight(original) + params.amount / 100.0 * n;
        result = vec4<f32>(color * original.a, original.a);
    }
    output[id.y * params.width + id.x] = encoded(result);
}

// Dust & Scratches: each channel's median of the square of `reach` around the pixel (the CPU's
// `Kernel::Median`, exact): a bisection on the value, the count of the window's values at or
// below it telling which side the median is on, then the smallest value above the lower bound.
// After 24 halvings the bounds are closer than any two values of 8-bit pixels apart, so that
// value is the median. The pixel becomes it where it differs by more than the threshold.
@compute @workgroup_size(16, 16)
fn median_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let r = i32(params.reach);
    let x = i32(id.x);
    let y = i32(id.y);
    let side = u32(2 * r + 1);
    let middle = vec4<u32>(side * side / 2u);
    var lo = vec4<f32>(-1.0);
    var hi = vec4<f32>(2.0);
    for (var step = 0; step < 24; step++) {
        let mid = (lo + hi) * 0.5;
        var count = vec4<u32>(0u);
        for (var dy = -r; dy <= r; dy++) {
            for (var dx = -r; dx <= r; dx++) {
                count += select(vec4<u32>(0u), vec4<u32>(1u), input(x + dx, y + dy) <= mid);
            }
        }
        let above = count > middle;
        hi = select(hi, mid, above);
        lo = select(mid, lo, above);
    }
    var median = vec4<f32>(3.0);
    for (var dy = -r; dy <= r; dy++) {
        for (var dx = -r; dx <= r; dx++) {
            let p = input(x + dx, y + dy);
            median = select(median, min(median, p), p > lo);
        }
    }
    let original = input(x, y);
    let level = params.threshold / 255.0;
    let differs = any(abs(original - median) > vec4<f32>(level));
    output[id.y * params.width + id.x] = encoded(select(original, median, differs));
}

// Maximum and Minimum (`Kernel::Extreme` on the CPU, exact): each channel's largest (flag 1) or
// smallest value of the row's `2 reach + 1` pixels, then of the column's, the edges repeating
// outward.
fn extreme(a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    if (params.flags & 1u) != 0u {
        return max(a, b);
    }
    return min(a, b);
}

@compute @workgroup_size(16, 16)
fn extreme_rows_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let r = i32(params.reach);
    var best = input(i32(id.x), i32(id.y));
    for (var k = -r; k <= r; k++) {
        best = extreme(best, input(i32(id.x) + k, i32(id.y)));
    }
    rows[id.y * params.width + id.x] = best;
}

@compute @workgroup_size(16, 16)
fn extreme_columns_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let r = i32(params.reach);
    let last = i32(params.height) - 1;
    var best = rows[id.y * params.width + id.x];
    for (var k = -r; k <= r; k++) {
        let y = u32(clamp(i32(id.y) + k, 0, last));
        best = extreme(best, rows[y * params.width + id.x]);
    }
    output[id.y * params.width + id.x] = encoded(best);
}

// Bilinear between the input's pixel centers (`relief` on the CPU), the edges repeating.
fn bilinear(x: f32, y: f32) -> vec4<f32> {
    let x0 = floor(x);
    let y0 = floor(y);
    let a = x - x0;
    let b = y - y0;
    let i = i32(x0);
    let j = i32(y0);
    let top = input(i, j) * (1.0 - a) + input(i + 1, j) * a;
    let bottom = input(i, j + 1) * (1.0 - a) + input(i + 1, j + 1) * a;
    return top * (1.0 - b) + bottom * b;
}

// Stylize (`Filter::finish` on the CPU), by `mode`: 0 Solarize (colors above the middle
// inverted), 1 Find Edges (each channel's Sobel gradient, dark lines on white), 2 Emboss (middle
// gray moved by the difference of the pixels `weights[0..2]` after and before, `amount`
// percent); the pixel's alpha kept.
@compute @workgroup_size(16, 16)
fn stylize_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let x = i32(id.x);
    let y = i32(id.y);
    let original = input(x, y);
    let alpha = original.a;
    var result = original;
    if alpha > 0.0 {
        if params.mode == 0u {
            let o = straight(original);
            let c = select(o, vec3<f32>(1.0) - o, o > vec3<f32>(0.5));
            result = vec4<f32>(c * alpha, alpha);
        } else if params.mode == 1u {
            let gx = (input(x + 1, y - 1) + 2.0 * input(x + 1, y) + input(x + 1, y + 1))
                - (input(x - 1, y - 1) + 2.0 * input(x - 1, y) + input(x - 1, y + 1));
            let gy = (input(x - 1, y + 1) + 2.0 * input(x, y + 1) + input(x + 1, y + 1))
                - (input(x - 1, y - 1) + 2.0 * input(x, y - 1) + input(x + 1, y - 1));
            let g = sqrt(gx * gx + gy * gy) / 4.0;
            let c = vec3<f32>(1.0) - clamp(g.rgb / alpha, vec3<f32>(0.0), vec3<f32>(1.0));
            result = vec4<f32>(c * alpha, alpha);
        } else {
            let fx = f32(x);
            let fy = f32(y);
            let d = bilinear(fx + weights[0], fy + weights[1]) - bilinear(fx - weights[0], fy - weights[1]);
            let c = vec3<f32>(0.5) + params.amount / 100.0 * d.rgb / alpha;
            result = vec4<f32>(c * alpha, alpha);
        }
    }
    output[id.y * params.width + id.x] = encoded(result);
}

// Mosaic (`Kernel::Cells` on the CPU): the cell of side `weights[0]` (pixels of the crop) each
// pixel lies in, cells from the layer's origin (`weights[1..3]`: the crop's first pixel from it,
// `weights[3..5]`: the layer's size in pixels of the crop), cut by the layer's edges; averaged
// along the row, then along the column.
fn cell_span(v: i32, axis: u32) -> vec2<i32> {
    let cell = weights[0];
    let origin = i32(weights[1u + axis]);
    let size = i32(ceil(weights[3u + axis]));
    let at = clamp(origin + v, 0, size - 1);
    let index = floor(f32(at) / cell);
    let start = max(i32(ceil(index * cell)), 0);
    let end = max(min(i32(ceil((index + 1.0) * cell)), size), start + 1);
    return vec2<i32>(start - origin, end - origin);
}

@compute @workgroup_size(16, 16)
fn cells_rows_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let span = cell_span(i32(id.x), 0u);
    var sum = vec4<f32>(0.0);
    for (var x = span.x; x < span.y; x++) {
        sum += input(x, i32(id.y));
    }
    rows[id.y * params.width + id.x] = sum / f32(span.y - span.x);
}

@compute @workgroup_size(16, 16)
fn cells_columns_main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= params.width || id.y >= params.height {
        return;
    }
    let span = cell_span(i32(id.y), 1u);
    let last = i32(params.height) - 1;
    var sum = vec4<f32>(0.0);
    for (var y = span.x; y < span.y; y++) {
        sum += rows[u32(clamp(y, 0, last)) * params.width + id.x];
    }
    output[id.y * params.width + id.x] = encoded(sum / f32(span.y - span.x));
}
