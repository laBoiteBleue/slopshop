//! The Erase tool's protocol around the model: everything but the transformer and the VAE
//! themselves. FLUX.2 [klein] 4B turbo with the `erase_v1` LoRA replaces the selection by the
//! background in 4 transformer passes; the steps here follow the tool's integration protocol
//! (`PROTOCOL.md` v1.0.0 and its executable reference, `reference.py`): working size, the two
//! reference images, position ids, the sigma schedule, the Euler update, latent (un)packing and
//! the strict composite (no pixel outside the selection changes).

pub mod pil;

use crate::numeric::round_bf16;

/// At most this many pixels at the model's resolution (never upscaled).
pub const MAX_PIXELS: f64 = 1_048_576.0;
/// Both working sides are multiples of this (8 for the VAE, then 2×2 patches).
pub const SIZE_MULTIPLE: u32 = 16;
/// Denoising passes.
pub const STEPS: usize = 4;
/// Channels of a packed latent token (32 VAE channels × a 2×2 patch).
pub const TOKEN_CHANNELS: usize = 128;
/// VAE latent channels, before patching.
pub const LATENT_CHANNELS: usize = 32;
/// Time ids of the reference images: the photo, then the mask (the target is 0).
pub const REFERENCE_TIMES: [u32; 2] = [10, 20];
/// Width of a rotary axis: four axes (T, H, W, L) of 32 dimensions make a head of 128.
const ROPE_AXIS: usize = 32;
const ROPE_THETA: f64 = 2000.0;

/// The model's resolution for a photo of `width` × `height`: at most about 1 Mpx, both sides a
/// multiple of 16 (never larger than the photo).
pub fn working_size(width: u32, height: u32) -> (u32, u32) {
    let s = (MAX_PIXELS / (f64::from(width) * f64::from(height)))
        .sqrt()
        .min(1.0);
    let side = |v: u32| (f64::from(v) * s) as u32 / SIZE_MULTIPLE * SIZE_MULTIPLE;
    (side(width), side(height))
}

/// A rectangle of the image, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Context kept around the selection's box, on each side: this fraction of its larger side.
const CONTEXT: f64 = 0.5;
/// The region's smaller sides are raised to this (when the image allows): below about 512 px,
/// the model sees too little of the scene.
const MIN_SIDE: u32 = 512;

/// The part of an image of `width` × `height` the model works on to erase a selection whose
/// bounding box is `selection`: the box, plus context on each side (half its larger side), at
/// least 512 px a side, within the image and centred on the box where the edges allow. Its sides
/// are multiples of 16 when that keeps the box inside, so that a region under 1 Mpx is worked on
/// unscaled. A region over 1 Mpx is reduced to it (see [`working_size`]), as the whole image was.
pub fn work_region(width: u32, height: u32, selection: Region) -> Region {
    let margin = (f64::from(selection.width.max(selection.height)) * CONTEXT).ceil() as u32;
    let side = |extent: u32, image: u32| -> u32 {
        let wanted = (extent + 2 * margin).max(MIN_SIDE).min(image);
        let snapped = wanted.next_multiple_of(SIZE_MULTIPLE);
        if snapped <= image {
            snapped
        } else if wanted / SIZE_MULTIPLE * SIZE_MULTIPLE >= extent {
            wanted / SIZE_MULTIPLE * SIZE_MULTIPLE
        } else {
            wanted
        }
    };
    let place = |start: u32, extent: u32, size: u32, image: u32| -> u32 {
        let centre = f64::from(start) + f64::from(extent) / 2.0;
        let origin = (centre - f64::from(size) / 2.0).round().max(0.0) as u32;
        origin.min(image - size)
    };
    let w = side(selection.width, width);
    let h = side(selection.height, height);
    Region {
        x: place(selection.x, selection.width, w, width),
        y: place(selection.y, selection.height, h, height),
        width: w,
        height: h,
    }
}

/// Standard normal noise, `count` values from `seed` (SplitMix64 then Box–Muller): the starting
/// latent, reproducible from the seed a result keeps.
pub fn noise(seed: u64, count: usize) -> Vec<f32> {
    let mut state = seed;
    let mut next = move || {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    };
    // A uniform in (0, 1]: never 0, whose logarithm is infinite.
    let mut uniform = move || ((next() >> 11) as f64 + 1.0) / (1u64 << 53) as f64;
    let mut out = Vec::with_capacity(count + 1);
    while out.len() < count {
        let (u, v) = (uniform(), uniform());
        let r = (-2.0 * u.ln()).sqrt();
        let a = std::f64::consts::TAU * v;
        out.push((r * a.cos()) as f32);
        out.push((r * a.sin()) as f32);
    }
    out.truncate(count);
    out
}

/// The 4 sampling sigmas, then 0, for `tokens` latent tokens (the target's, `(h/16)·(w/16)`):
/// FLUX.2's resolution-dependent exponential time shift.
pub fn sigmas(tokens: usize) -> [f64; STEPS + 1] {
    let (a1, b1, a2, b2) = (8.73809524e-05, 1.89833333, 0.00016927, 0.45666666);
    let n = tokens as f64;
    let mu = if tokens > 4300 {
        a2 * n + b2
    } else {
        let (m200, m10) = (a2 * n + b2, a1 * n + b1);
        let a = (m200 - m10) / 190.0;
        a * STEPS as f64 + (m200 - 200.0 * a)
    };
    let mut out = [0.0; STEPS + 1];
    for (i, sigma) in out.iter_mut().take(STEPS).enumerate() {
        let base = 1.0 - i as f64 / STEPS as f64; // 1, 0.75, 0.5, 0.25
        *sigma = mu.exp() / (mu.exp() + (1.0 / base - 1.0));
    }
    out
}

/// The sinusoidal projection of a step's timestep (256 values), as the model computes it from
/// the sigma it is given: `sigma × 1000` in bfloat16, frequencies `10000^(-i/128)`, cosines then
/// sines, rounded to bfloat16.
pub fn timestep_projection(sigma: f64) -> [f32; 256] {
    let t = round_bf16(round_bf16(sigma as f32) * 1000.0);
    let mut out = [0.0; 256];
    for i in 0..128 {
        let exponent = -(10000f32).ln() * i as f32 / 128.0;
        let angle = t * exponent.exp();
        out[i] = round_bf16(angle.cos());
        out[128 + i] = round_bf16(angle.sin());
    }
    out
}

/// Rotary tables for the transformer's joint sequence: `text` prompt tokens (ids `(0, 0, 0, l)`),
/// then the target, the photo and the mask, each `lh × lw` tokens in row order (ids
/// `(T, y, x, 0)`). Returns `cos` and `sin` of `sequence × 128`, the sine signed for a rotation
/// written `x·cos + swap(x)·sin` where `swap` exchanges each pair of dimensions: the model's
/// `(x₀, x₁) → (x₀cos − x₁sin, x₁cos + x₀sin)`.
pub fn rope_tables(text: usize, lh: usize, lw: usize) -> (Vec<f32>, Vec<f32>) {
    let mut ids: Vec<[u32; 4]> = (0..text as u32).map(|l| [0, 0, 0, l]).collect();
    for t in [0].into_iter().chain(REFERENCE_TIMES) {
        for y in 0..lh as u32 {
            ids.extend((0..lw as u32).map(|x| [t, y, x, 0]));
        }
    }
    let freqs: Vec<f64> = (0..ROPE_AXIS / 2)
        .map(|j| 1.0 / ROPE_THETA.powf((2 * j) as f64 / ROPE_AXIS as f64))
        .collect();
    let width = 4 * ROPE_AXIS;
    let mut cos = vec![0.0; ids.len() * width];
    let mut sin = vec![0.0; ids.len() * width];
    for (s, id) in ids.iter().enumerate() {
        for (axis, &pos) in id.iter().enumerate() {
            for (j, f) in freqs.iter().enumerate() {
                let angle = f64::from(pos) * f;
                let at = s * width + axis * ROPE_AXIS + 2 * j;
                let (c, sn) = (angle.cos() as f32, angle.sin() as f32);
                cos[at] = c;
                cos[at + 1] = c;
                sin[at] = -sn;
                sin[at + 1] = sn;
            }
        }
    }
    (cos, sin)
}

/// The VAE's input for 8-bit RGB pixels: planar `[3, h, w]` in [−1, 1].
pub fn vae_input(rgb: &[u8], width: usize, height: usize) -> Vec<f32> {
    let plane = width * height;
    let mut out = vec![0.0; 3 * plane];
    for (i, px) in rgb.as_chunks::<3>().0.iter().enumerate() {
        for c in 0..3 {
            out[c * plane + i] = f32::from(px[c]) / 127.5 - 1.0;
        }
    }
    out
}

/// The VAE's output `[3, h, w]` in [−1, 1] as 8-bit RGB (rounded half to even, as PyTorch).
pub fn vae_output(planar: &[f32], width: usize, height: usize) -> Vec<u8> {
    let plane = width * height;
    let mut out = vec![0; 3 * plane];
    for i in 0..plane {
        for c in 0..3 {
            let v = (planar[c * plane + i].clamp(-1.0, 1.0) + 1.0) * 127.5;
            out[3 * i + c] = v.round_ties_even().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// The VAE's batch-norm statistics over the 128 patched channels: `(mean, sqrt(var + eps))`.
#[derive(Debug, Clone)]
pub struct LatentStats {
    pub mean: Vec<f32>,
    pub std: Vec<f32>,
}

impl LatentStats {
    pub fn new(mean: Vec<f32>, var: &[f32], eps: f32) -> Self {
        let std = var.iter().map(|v| (v + eps).sqrt()).collect();
        Self { mean, std }
    }
}

/// A VAE latent `[32, 2·lh, 2·lw]` as normalized tokens `[lh·lw, 128]` in row order, each
/// token's channels ordered `(c, dy, dx)` (2×2 patchify, then the batch-norm statistics).
pub fn pack(latent: &[f32], lh: usize, lw: usize, stats: &LatentStats) -> Vec<f32> {
    let (h8, w8) = (2 * lh, 2 * lw);
    let mut out = vec![0.0; lh * lw * TOKEN_CHANNELS];
    for y in 0..lh {
        for x in 0..lw {
            let token = &mut out[(y * lw + x) * TOKEN_CHANNELS..][..TOKEN_CHANNELS];
            for c in 0..LATENT_CHANNELS {
                for (d, (dy, dx)) in [(0, 0), (0, 1), (1, 0), (1, 1)].into_iter().enumerate() {
                    let ch = c * 4 + d;
                    let v = latent[(c * h8 + 2 * y + dy) * w8 + 2 * x + dx];
                    token[ch] = (v - stats.mean[ch]) / stats.std[ch];
                }
            }
        }
    }
    out
}

/// The inverse of [`pack`]: normalized tokens back to a VAE latent `[32, 2·lh, 2·lw]`.
pub fn unpack(tokens: &[f32], lh: usize, lw: usize, stats: &LatentStats) -> Vec<f32> {
    let (h8, w8) = (2 * lh, 2 * lw);
    let mut out = vec![0.0; LATENT_CHANNELS * h8 * w8];
    for y in 0..lh {
        for x in 0..lw {
            let token = &tokens[(y * lw + x) * TOKEN_CHANNELS..][..TOKEN_CHANNELS];
            for c in 0..LATENT_CHANNELS {
                for (d, (dy, dx)) in [(0, 0), (0, 1), (1, 0), (1, 1)].into_iter().enumerate() {
                    let ch = c * 4 + d;
                    out[(c * h8 + 2 * y + dy) * w8 + 2 * x + dx] =
                        token[ch] * stats.std[ch] + stats.mean[ch];
                }
            }
        }
    }
    out
}

/// Noise `[128, lh, lw]` (channel planes) as packed tokens `[lh·lw, 128]`, rounded to bfloat16
/// as the reference keeps the denoised state.
pub fn pack_noise(noise: &[f32], lh: usize, lw: usize) -> Vec<f32> {
    let plane = lh * lw;
    let mut out = vec![0.0; plane * TOKEN_CHANNELS];
    for ch in 0..TOKEN_CHANNELS {
        for i in 0..plane {
            out[i * TOKEN_CHANNELS + ch] = round_bf16(noise[ch * plane + i]);
        }
    }
    out
}

/// One Euler step of the flow: `x += (σ_next − σ) · v`, the state kept in bfloat16.
pub fn euler_step(x: &mut [f32], velocity: &[f32], sigma: f64, next: f64) {
    let dt = (next - sigma) as f32;
    for (x, v) in x.iter_mut().zip(velocity) {
        *x = round_bf16(*x + dt * v);
    }
}

/// The selection as the model's second reference: white (to erase) or black, on 3 channels.
pub fn mask_rgb(selection: &[u8]) -> Vec<u8> {
    selection.iter().flat_map(|&s| [s; 3]).collect()
}

/// The strict composite: the model's output inside the selection (`selection[i] > 127`), the
/// photo's own pixels everywhere else.
pub fn composite(photo: &[u8], selection: &[u8], output: &[u8]) -> Vec<u8> {
    let mut out = photo.to_vec();
    for (i, &s) in selection.iter().enumerate() {
        if s > 127 {
            out[3 * i..3 * i + 3].copy_from_slice(&output[3 * i..3 * i + 3]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_size_caps_at_a_megapixel_in_multiples_of_16() {
        assert_eq!(working_size(1024, 768), (1024, 768));
        assert_eq!(working_size(2048, 1536), (1168, 880)); // test vector 03
        assert_eq!(working_size(500, 333), (496, 320)); // never upscaled
        let (w, h) = working_size(8000, 1000);
        assert!(w * h <= 1_048_576 && w % 16 == 0 && h % 16 == 0);
    }

    fn contains(outer: Region, inner: Region) -> bool {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.x + inner.width <= outer.x + outer.width
            && inner.y + inner.height <= outer.y + outer.height
    }

    #[test]
    fn the_work_region_frames_the_selection_with_context() {
        // A 300 × 200 object in a 6000 × 4000 photo: 150 px of context each side → 600 × 500,
        // snapped to 608 × 512, centred on the object.
        let object = Region {
            x: 3000,
            y: 2000,
            width: 300,
            height: 200,
        };
        let r = work_region(6000, 4000, object);
        assert_eq!((r.width, r.height), (608, 512));
        assert_eq!((r.x, r.y), (3150 - 304, 2100 - 256));
        assert!(contains(r, object));
        // Under 1 Mpx and in multiples of 16: worked on unscaled.
        assert_eq!(working_size(r.width, r.height), (r.width, r.height));
    }

    #[test]
    fn the_work_region_stays_in_the_image() {
        // A small object in a corner: the region is pushed inside, the object still in it.
        let object = Region {
            x: 5,
            y: 3990,
            width: 40,
            height: 10,
        };
        let r = work_region(6000, 4000, object);
        assert_eq!((r.x, r.y, r.width, r.height), (0, 4000 - 512, 512, 512));
        assert!(contains(r, object));
        // A large selection: the region is the whole image, reduced to 1 Mpx by working_size.
        let large = Region {
            x: 100,
            y: 100,
            width: 1800,
            height: 1300,
        };
        let r = work_region(2048, 1536, large);
        assert_eq!(
            r,
            Region {
                x: 0,
                y: 0,
                width: 2048,
                height: 1536
            }
        );
        // An image smaller than the minimum side, and of a side that is not a multiple of 16.
        let tiny = Region {
            x: 10,
            y: 10,
            width: 20,
            height: 20,
        };
        assert_eq!(
            work_region(300, 250, tiny),
            Region {
                x: 0,
                y: 0,
                width: 288,
                height: 240
            }
        );
        let wide = Region {
            x: 0,
            y: 0,
            width: 1001,
            height: 20,
        };
        let r = work_region(1001, 700, wide);
        assert_eq!((r.x, r.width), (0, 1001)); // 992 would cut the selection: kept unsnapped
        assert!(contains(r, wide));
    }

    #[test]
    fn noise_is_standard_normal_and_reproducible() {
        let n = noise(42, 200_001);
        assert_eq!(n.len(), 200_001);
        assert_eq!(n, noise(42, 200_001));
        assert_ne!(n[..8], noise(43, 8)[..]);
        let mean = n.iter().map(|&v| f64::from(v)).sum::<f64>() / n.len() as f64;
        let var = n
            .iter()
            .map(|&v| (f64::from(v) - mean).powi(2))
            .sum::<f64>()
            / n.len() as f64;
        assert!(
            mean.abs() < 0.01 && (var - 1.0).abs() < 0.01,
            "{mean} {var}"
        );
        // About 4.6 % of a normal distribution lies beyond two standard deviations.
        let tails = n.iter().filter(|v| v.abs() > 2.0).count() as f64 / n.len() as f64;
        assert!((tails - 0.0455).abs() < 0.003, "{tails}");
    }

    #[test]
    fn sigmas_match_the_test_vectors() {
        // case.json of 01/02 (1024×768) and 03 (1168×880).
        let expected = [
            (
                3072,
                [
                    1.0,
                    0.964530289207372,
                    0.9006394477286384,
                    0.7513336040514659,
                    0.0,
                ],
            ),
            (
                4015,
                [
                    1.0,
                    0.9671665781883478,
                    0.9075693627662733,
                    0.7659709894512238,
                    0.0,
                ],
            ),
        ];
        for (tokens, want) in expected {
            for (got, want) in sigmas(tokens).iter().zip(want) {
                assert!((got - want).abs() < 1e-6, "{tokens}: {got} vs {want}");
            }
        }
        // Above 4300 tokens, mu is linear in the token count.
        let mu = (0.00016927f64 * 4500.0 + 0.45666666).exp();
        assert_eq!(sigmas(4500)[1], mu / (mu + 1.0 / 3.0));
    }

    #[test]
    fn the_timestep_is_rounded_as_the_model_sees_it() {
        // σ = 0.96453 is 0.96484375 in bfloat16, × 1000 = 964.84 → 964 in bfloat16.
        let p = timestep_projection(0.964530289207372);
        assert_eq!(p[0], round_bf16(964f32.cos()));
        assert_eq!(p[128], round_bf16(964f32.sin()));
        let p = timestep_projection(1.0);
        assert_eq!(p[0], round_bf16(1000f32.cos()));
        assert_eq!(
            p[127 + 128],
            round_bf16((1000.0 * (-(10000f32).ln() * 127.0 / 128.0).exp()).sin())
        );
    }

    #[test]
    fn rope_tables_follow_the_ids() {
        let (lh, lw) = (2, 3);
        let (cos, sin) = rope_tables(4, lh, lw);
        let seq = 4 + 3 * lh * lw;
        assert_eq!((cos.len(), sin.len()), (seq * 128, seq * 128));
        let at = |s: usize, d: usize| (cos[s * 128 + d], sin[s * 128 + d]);
        // Text token 3: only the L axis (dims 96..128) turns, by 3 × θ^(-2j/32).
        assert_eq!(at(3, 0), (1.0, -0.0));
        assert_eq!(at(3, 96), ((3f64).cos() as f32, -((3f64).sin() as f32)));
        assert_eq!(at(3, 97), ((3f64).cos() as f32, (3f64).sin() as f32));
        // The mask's last token: T = 20, y = 1, x = 2.
        let last = seq - 1;
        assert_eq!(at(last, 0).0, (20f64).cos() as f32);
        assert_eq!(at(last, 32).0, (1f64).cos() as f32);
        assert_eq!(
            at(last, 66).0,
            (2.0 / 2000f64.powf(2.0 / 32.0)).cos() as f32
        );
        assert_eq!(at(last, 96), (1.0, -0.0));
    }

    #[test]
    fn packing_round_trips_and_orders_channels_c_dy_dx() {
        let (lh, lw) = (2, 3);
        let latent: Vec<f32> = (0..LATENT_CHANNELS * 4 * lh * lw)
            .map(|i| i as f32)
            .collect();
        let stats = LatentStats::new(vec![0.0; 128], &[1.0; 128], 0.0);
        let tokens = pack(&latent, lh, lw, &stats);
        // Token (1, 2), channel (c = 1, dy = 1, dx = 0): latent pixel (3, 4) of plane 1.
        let (h8, w8) = (2 * lh, 2 * lw);
        assert_eq!(tokens[5 * 128 + 4 + 2], latent[(h8 + 3) * w8 + 4]);
        assert_eq!(unpack(&tokens, lh, lw, &stats), latent);
        let stats = LatentStats::new(vec![1.0; 128], &[3.0; 128], 1.0);
        let tokens = pack(&latent, lh, lw, &stats);
        assert_eq!(tokens[1], (latent[1] - 1.0) / 2.0);
        assert_eq!(unpack(&tokens, lh, lw, &stats), latent);
    }

    #[test]
    fn noise_packs_by_token() {
        let noise: Vec<f32> = (0..128 * 6).map(|i| i as f32).collect();
        let x = pack_noise(&noise, 2, 3);
        assert_eq!(x[4 * 128 + 7], round_bf16(noise[7 * 6 + 4]));
    }

    #[test]
    fn euler_steps_keep_the_state_in_bfloat16() {
        let mut x = vec![1.0, -0.5];
        euler_step(&mut x, &[0.5, 1.0], 1.0, 0.75);
        assert_eq!(x, vec![round_bf16(0.875), round_bf16(-0.75)]);
    }

    #[test]
    fn the_composite_keeps_every_pixel_outside_the_selection() {
        let photo = [10, 20, 30, 40, 50, 60];
        let output = [1, 2, 3, 4, 5, 6];
        assert_eq!(
            composite(&photo, &[0, 255], &output),
            vec![10, 20, 30, 4, 5, 6]
        );
        assert_eq!(mask_rgb(&[0, 255]), vec![0, 0, 0, 255, 255, 255]);
    }

    #[test]
    fn vae_values_map_to_and_from_pixels() {
        let rgb = [0, 255, 128];
        let x = vae_input(&rgb, 1, 1);
        assert_eq!(x, vec![-1.0, 1.0, 128.0 / 127.5 - 1.0]);
        assert_eq!(vae_output(&x, 1, 1), rgb);
        // Clamped to [−1, 1]; 0.5 → 191.25 → 191.
        assert_eq!(vae_output(&[2.0, -3.0, 0.5], 1, 1), [255, 0, 191]);
    }
}
