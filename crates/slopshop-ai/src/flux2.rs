//! FLUX.2 [klein] 4B as ONNX graphs written here, from the published diffusers weights
//! (`Flux2Transformer2DModel`, `AutoencoderKLFlux2`): the transformer, the VAE's encoder and its
//! decoder. Each builder returns the serialized model and the buffer of its weights, which ONNX
//! Runtime is given as the external data file [`WEIGHTS`] (nothing is written to disk).
//!
//! Precision (DirectML has no bfloat16): the transformer's residual streams, norms, rotary
//! embedding and modulation stay in single precision; its large projections and the attention
//! run in half precision. The bfloat16 weights convert to half precision exactly. A LoRA is
//! merged into the weights it targets (`W + B·A`, scale 1). The VAE runs in single precision.

use std::io;
use std::path::Path;

use crate::numeric::f32_to_f16;
use crate::onnx::{Attr, DataType, Dim, Graph};
use crate::safetensors::SafeTensors;

/// The name the graphs give their external data.
pub const WEIGHTS: &str = "weights.bin";
/// Prompt tokens (the precomputed embedding's length).
pub const TEXT_TOKENS: i64 = 512;
const DIM: i64 = 3072;
const HEADS: i64 = 24;
const HEAD_DIM: i64 = 128;
const MLP: i64 = 9216;
const DOUBLE_BLOCKS: usize = 5;
const SINGLE_BLOCKS: usize = 20;
const EPS: f32 = 1e-6;
/// Alignment of each weight in the buffer.
const ALIGN: usize = 4096;

/// Extra dimensions of each attention head that mask the padding tokens.
const KEY_BIAS: i64 = 8;
/// The score given to a padding token: its weight after the softmax is exactly zero.
pub const PADDING_SCORE: f32 = -32768.0;
/// The transformer's sequence is padded to a multiple of this (DirectML's fast half-precision
/// matrix products need it: an odd length runs about twice as slow).
pub const SEQUENCE_MULTIPLE: usize = 8;

/// A model and its weights.
#[derive(Debug)]
pub struct Built {
    pub model: Vec<u8>,
    pub weights: Vec<u8>,
}

/// A LoRA's factors for one layer: `A` `[r, in]`, `B` `[out, r]` and the rank `r`.
type Factors = (Vec<f32>, Vec<f32>, usize);

/// A LoRA in diffusers keys (`transformer.<module>.lora_A.weight`, `…lora_B.weight`).
#[derive(Debug)]
pub struct Lora {
    file: SafeTensors,
    prefix: &'static str,
}

impl Lora {
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            file: SafeTensors::open(path)?,
            prefix: "transformer.",
        })
    }

    /// `(A [r, in], B [out, r], r)` for the module `name` (without `.weight`), if targeted.
    fn factors(&mut self, module: &str) -> io::Result<Option<Factors>> {
        let a = format!("{}{module}.lora_A.weight", self.prefix);
        let b = format!("{}{module}.lora_B.weight", self.prefix);
        if !self.file.tensors.contains_key(&a) {
            return Ok(None);
        }
        let rank = self.file.info(&a)?.shape[0];
        Ok(Some((self.file.f32s(&a)?, self.file.f32s(&b)?, rank)))
    }

    /// Every module the LoRA targets.
    pub fn modules(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .file
            .tensors
            .keys()
            .filter_map(|k| k.strip_suffix(".lora_A.weight"))
            .filter_map(|k| k.strip_prefix(self.prefix))
            .map(str::to_string)
            .collect();
        out.sort();
        out
    }
}

struct Builder<'a> {
    g: Graph,
    weights: Vec<u8>,
    file: &'a mut SafeTensors,
    lora: Option<&'a mut Lora>,
    merged: usize,
}

impl Builder<'_> {
    fn place(&mut self, name: &str, dtype: DataType, dims: &[i64], data: &[u8]) -> String {
        let offset = self.weights.len().div_ceil(ALIGN) * ALIGN;
        self.weights.resize(offset, 0);
        self.weights.extend_from_slice(data);
        self.g
            .external(name, dtype, dims, WEIGHTS, offset, data.len())
    }

    /// A linear layer's weight `[out, in]` as the `[in, out]` right operand of a `MatMul`, the
    /// LoRA merged, in `dtype`.
    fn matrix(&mut self, module: &str, dtype: DataType) -> io::Result<String> {
        let (w, out, inp) = self.merged_weight(module)?;
        let data = transpose_to(&w, out, inp, dtype);
        Ok(self.place(
            &format!("{module}.weight"),
            dtype,
            &[inp as i64, out as i64],
            &data,
        ))
    }

    /// The weight `[out, in]` of a linear layer in single precision, the LoRA merged.
    fn merged_weight(&mut self, module: &str) -> io::Result<(Vec<f32>, usize, usize)> {
        let name = format!("{module}.weight");
        let shape = self.file.info(&name)?.shape.clone();
        let [out, inp] = shape[..] else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{name} is not a matrix"),
            ));
        };
        let mut w = self.file.f32s(&name)?;
        if let Some(lora) = self.lora.as_deref_mut()
            && let Some((a, b, rank)) = lora.factors(module)?
        {
            merge_lora(&mut w, out, inp, &a, &b, rank);
            self.merged += 1;
        }
        Ok((w, out, inp))
    }

    /// A fused projection's weight cut along its outputs into `sizes` (e.g. q, k, v and the
    /// MLP's halves): one `MatMul` each, so no large output is split afterwards.
    fn matrix_rows(
        &mut self,
        module: &str,
        sizes: &[usize],
        dtype: DataType,
    ) -> io::Result<Vec<String>> {
        let (w, out, inp) = self.merged_weight(module)?;
        debug_assert_eq!(sizes.iter().sum::<usize>(), out);
        let mut names = Vec::new();
        let mut first = 0;
        for (i, &rows) in sizes.iter().enumerate() {
            let data = transpose_to(&w[first * inp..(first + rows) * inp], rows, inp, dtype);
            let dims = [inp as i64, rows as i64];
            names.push(self.place(&format!("{module}.weight.{i}"), dtype, &dims, &data));
            first += rows;
        }
        Ok(names)
    }

    /// A projection's weight cut along its inputs into `sizes`: the projection of a
    /// concatenation is the sum of the parts' projections.
    fn matrix_columns(
        &mut self,
        module: &str,
        sizes: &[usize],
        dtype: DataType,
    ) -> io::Result<Vec<String>> {
        let (w, out, inp) = self.merged_weight(module)?;
        debug_assert_eq!(sizes.iter().sum::<usize>(), inp);
        let mut names = Vec::new();
        let mut first = 0;
        for (i, &cols) in sizes.iter().enumerate() {
            let part: Vec<f32> = w
                .chunks_exact(inp)
                .flat_map(|row| &row[first..first + cols])
                .copied()
                .collect();
            let data = transpose_to(&part, out, cols, dtype);
            let dims = [cols as i64, out as i64];
            names.push(self.place(&format!("{module}.weight.{i}"), dtype, &dims, &data));
            first += cols;
        }
        Ok(names)
    }

    /// A vector (norm weight, bias) in single precision, reshaped to `dims`.
    fn vector(&mut self, name: &str, dims: &[i64]) -> io::Result<String> {
        let v = self.file.f32s(name)?;
        let data: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
        Ok(self.place(name, DataType::F32, dims, &data))
    }

    /// A convolution kernel in single precision, as published (`[out, in, kh, kw]`).
    fn kernel(&mut self, name: &str) -> io::Result<String> {
        let dims: Vec<i64> = self
            .file
            .info(name)?
            .shape
            .iter()
            .map(|&d| d as i64)
            .collect();
        self.vector(name, &dims)
    }

    // ---------------------------------------------------------------- operators

    fn op(&mut self, op: &str, inputs: &[&str]) -> String {
        self.g.node(op, inputs, &[])
    }

    fn cast(&mut self, x: &str, to: DataType) -> String {
        self.g.node("Cast", &[x], &[Attr::Int("to", to as i64)])
    }

    fn silu(&mut self, x: &str) -> String {
        let s = self.op("Sigmoid", &[x]);
        self.op("Mul", &[x, &s])
    }

    fn reshape(&mut self, x: &str, dims: &[i64]) -> String {
        let shape = self.g.ints(dims);
        self.op("Reshape", &[x, &shape])
    }

    fn transpose(&mut self, x: &str, perm: &[i64]) -> String {
        self.g
            .node("Transpose", &[x], &[Attr::Ints("perm", perm.to_vec())])
    }

    fn slice(&mut self, x: &str, axis: i64, start: i64, end: i64) -> String {
        let (s, e, a) = (
            self.g.ints(&[start]),
            self.g.ints(&[end]),
            self.g.ints(&[axis]),
        );
        self.op("Slice", &[x, &s, &e, &a])
    }

    fn split(&mut self, x: &str, axis: i64, sizes: &[i64]) -> Vec<String> {
        let s = self.g.ints(sizes);
        self.g
            .node_n("Split", &[x, &s], sizes.len(), &[Attr::Int("axis", axis)])
    }

    fn concat(&mut self, xs: &[&str], axis: i64) -> String {
        self.g.node("Concat", xs, &[Attr::Int("axis", axis)])
    }

    fn linear(&mut self, x: &str, module: &str, dtype: DataType) -> io::Result<String> {
        let w = self.matrix(module, dtype)?;
        Ok(self.op("MatMul", &[x, &w]))
    }

    /// A projection in half precision of a single-precision value, back in single precision.
    fn linear16(&mut self, x16: &str, module: &str) -> io::Result<String> {
        let y = self.linear(x16, module, DataType::F16)?;
        Ok(self.cast(&y, DataType::F32))
    }

    /// `LayerNorm(x) · (1 + scale) + shift`, no affine parameters of its own.
    fn modulate(&mut self, x: &str, ones: &str, shift: &str, scale1: &str) -> String {
        let n = self.g.node(
            "LayerNormalization",
            &[x, ones],
            &[Attr::Int("axis", -1), Attr::Float("epsilon", EPS)],
        );
        let m = self.op("Mul", &[&n, scale1]);
        self.op("Add", &[&m, shift])
    }

    /// RMSNorm over the last axis, with its weight.
    fn rms(&mut self, x: &str, weight: &str) -> io::Result<String> {
        let w = self.vector(weight, &[HEAD_DIM])?;
        let sq = self.op("Mul", &[x, x]);
        let axes = self.g.ints(&[-1]);
        let mean = self
            .g
            .node("ReduceMean", &[&sq, &axes], &[Attr::Int("keepdims", 1)]);
        let eps = self.g.scalar(EPS);
        let var = self.op("Add", &[&mean, &eps]);
        let rstd = self.op("Sqrt", &[&var]);
        let n = self.op("Div", &[x, &rstd]);
        Ok(self.op("Mul", &[&n, &w]))
    }

    /// `x·cos + swap(x)·sin` over `[L, heads, 128]` (tables `[L, 1, 128]`).
    fn rope(&mut self, x: &str, cos: &str, sin: &str) -> String {
        let pairs = self.reshape(x, &[0, HEADS, HEAD_DIM / 2, 2]);
        let halves = self.g.node_n(
            "Split",
            &[&pairs],
            2,
            &[Attr::Int("axis", -1), Attr::Int("num_outputs", 2)],
        );
        let swapped = self.concat(&[&halves[1], &halves[0]], -1);
        let swapped = self.reshape(&swapped, &[0, HEADS, HEAD_DIM]);
        let a = self.op("Mul", &[x, cos]);
        let b = self.op("Mul", &[&swapped, sin]);
        self.op("Add", &[&a, &b])
    }

    /// Attention of `q`, `k` (`[L, 24, 128]`, single precision, rotated) and `v` (`[L, 3072]`,
    /// half precision): the heads' outputs `[L, 3072]` in half precision. `key_bias`
    /// (`[L, 24, 8]`, half precision) extends each key by 8 dimensions and each query by 8 ones:
    /// zero for the real tokens, a large negative score for the padding (see [`transformer_graph`]).
    fn attention(&mut self, q: &str, k: &str, v16: &str, key_bias: &str, chunks: usize) -> String {
        let scale = self.g.scalar(1.0 / (HEAD_DIM as f32).sqrt());
        let q = self.op("Mul", &[q, &scale]);
        let q16 = self.cast(&q, DataType::F16);
        let pads = self.g.ints(&[0, 0, 0, 0, 0, KEY_BIAS]);
        let one = self
            .g
            .constant(DataType::F16, &[], &f32_to_f16(1.0).to_le_bytes());
        let q16 = self.op("Pad", &[&q16, &pads, &one]); // [L, 24, 136]
        let k16 = self.cast(k, DataType::F16);
        let k16 = self.concat(&[&k16, key_bias], -1);
        let qh = self.transpose(&q16, &[1, 0, 2]); // [24, L, 136]
        let kt = self.transpose(&k16, &[1, 2, 0]); // [24, 136, L]
        let v = self.reshape(v16, &[-1, HEADS, HEAD_DIM]);
        let vh = self.transpose(&v, &[1, 0, 2]); // [24, L, 128]

        // Chunk i holds the queries [L·i/n, L·(i+1)/n), whatever the length L: only one chunk's
        // scores are in memory at a time.
        let chunks = chunks.max(1);
        let shape = self.op("Shape", &[&qh]);
        let length = self.slice(&shape, 0, 1, 2);
        let count = self.g.ints(&[chunks as i64]);
        let axis = self.g.ints(&[1]);
        let mut bounds = Vec::with_capacity(chunks + 1);
        for i in 0..=chunks {
            let i = self.g.ints(&[i as i64]);
            let scaled = self.op("Mul", &[&length, &i]);
            bounds.push(self.op("Div", &[&scaled, &count]));
        }
        let mut heads = Vec::with_capacity(chunks);
        for i in 0..chunks {
            let part = self.op("Slice", &[&qh, &bounds[i], &bounds[i + 1], &axis]);
            let s = self.op("MatMul", &[&part, &kt]);
            let p = self.g.node("Softmax", &[&s], &[Attr::Int("axis", -1)]);
            heads.push(self.op("MatMul", &[&p, &vh]));
        }
        let refs: Vec<&str> = heads.iter().map(String::as_str).collect();
        let o = self.concat(&refs, 1);
        let o = self.transpose(&o, &[1, 0, 2]);
        self.reshape(&o, &[-1, DIM])
    }

    /// SwiGLU in half precision: `silu(x·W_gate) · (x·W_up)`, the fused projection's two halves
    /// computed apart.
    fn swiglu(&mut self, x16: &str, module: &str) -> io::Result<String> {
        let w = self.matrix_rows(module, &[MLP as usize; 2], DataType::F16)?;
        let gate = self.op("MatMul", &[x16, &w[0]]);
        let up = self.op("MatMul", &[x16, &w[1]]);
        let gate = self.silu(&gate);
        Ok(self.op("Mul", &[&gate, &up]))
    }
}

/// Padding tokens to append after `image` tokens for the joint sequence (`text` + `image`) to be a
/// multiple of [`SEQUENCE_MULTIPLE`].
pub fn padding(text: usize, image: usize) -> usize {
    (text + image).next_multiple_of(SEQUENCE_MULTIPLE) - text - image
}

/// The `key_bias` input for a sequence of `tokens` whose last `padding` are padding.
pub fn key_bias(tokens: usize, padding: usize) -> Vec<f32> {
    let mut bias = vec![0.0; tokens * KEY_BIAS as usize];
    bias[(tokens - padding) * KEY_BIAS as usize..].fill(PADDING_SCORE / KEY_BIAS as f32);
    bias
}

/// `w [out, in] += b [out, r] · a [r, in]`, rows split across threads.
fn merge_lora(w: &mut [f32], out: usize, inp: usize, a: &[f32], b: &[f32], rank: usize) {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = out.div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for (chunk, block) in w.chunks_mut(rows * inp).enumerate() {
            s.spawn(move || {
                for (j, row) in block.chunks_exact_mut(inp).enumerate() {
                    let o = chunk * rows + j;
                    for r in 0..rank {
                        let coef = b[o * rank + r];
                        for (x, &ar) in row.iter_mut().zip(&a[r * inp..(r + 1) * inp]) {
                            *x += coef * ar;
                        }
                    }
                }
            });
        }
    });
}

/// `w [out, in]` as `[in, out]` little-endian bytes of `dtype`, rows split across threads.
fn transpose_to(w: &[f32], out: usize, inp: usize, dtype: DataType) -> Vec<u8> {
    let size = dtype.size();
    let mut data = vec![0u8; w.len() * size];
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = inp.div_ceil(threads).max(1);
    const TILE: usize = 64;
    std::thread::scope(|s| {
        for (chunk, block) in data.chunks_mut(rows * out * size).enumerate() {
            s.spawn(move || {
                let first = chunk * rows;
                let count = block.len() / (out * size);
                for o0 in (0..out).step_by(TILE) {
                    for i in 0..count {
                        for o in o0..(o0 + TILE).min(out) {
                            let v = w[o * inp + first + i];
                            let at = (i * out + o) * size;
                            match dtype {
                                DataType::F16 => {
                                    block[at..at + 2].copy_from_slice(&f32_to_f16(v).to_le_bytes())
                                }
                                _ => block[at..at + 4].copy_from_slice(&v.to_le_bytes()),
                            }
                        }
                    }
                }
            });
        }
    });
    data
}

/// The transformer: inputs `img` `[S, 128]` (target, photo and mask tokens, then padding),
/// `txt` `[512, 7680]` (prompt embedding), `tproj` `[1, 256]` (the timestep's projection),
/// `rope_cos` and `rope_sin` `[512 + S, 128]`, `key_bias` `[512 + S, 8]` (0 for a token,
/// [`PADDING_SCORE`] / 8 for padding); output `velocity` `[S, 128]` (only the target's rows are
/// used). Padding the sequence to a multiple of [`SEQUENCE_MULTIPLE`] keeps DirectML on its fast
/// path; the padding takes part in no other token's attention.
fn transformer_graph(b: &mut Builder, chunks: usize) -> io::Result<()> {
    let (f32_, f16_) = (DataType::F32, DataType::F16);
    let img =
        b.g.input("img", f32_, &[Dim::Named("image_tokens"), Dim::Fixed(128)]);
    let txt =
        b.g.input("txt", f32_, &[Dim::Fixed(TEXT_TOKENS), Dim::Fixed(7680)]);
    let tproj = b.g.input("tproj", f32_, &[Dim::Fixed(1), Dim::Fixed(256)]);
    let cos = b.g.input(
        "rope_cos",
        f32_,
        &[Dim::Named("tokens"), Dim::Fixed(HEAD_DIM)],
    );
    let sin = b.g.input(
        "rope_sin",
        f32_,
        &[Dim::Named("tokens"), Dim::Fixed(HEAD_DIM)],
    );
    let cos = b.reshape(&cos, &[-1, 1, HEAD_DIM]);
    let key_bias = b.g.input(
        "key_bias",
        f32_,
        &[Dim::Named("tokens"), Dim::Fixed(KEY_BIAS)],
    );
    let sin = b.reshape(&sin, &[-1, 1, HEAD_DIM]);
    let key_bias = b.cast(&key_bias, f16_);
    let key_bias = b.reshape(&key_bias, &[-1, 1, KEY_BIAS]);
    let heads_shape = b.g.ints(&[1, HEADS, 1]);
    let key_bias = b.op("Expand", &[&key_bias, &heads_shape]); // [L, 24, 8]
    // The text part of the tables, then the image part.
    let (cos_txt, cos_img) = (
        b.slice(&cos, 0, 0, TEXT_TOKENS),
        b.slice(&cos, 0, TEXT_TOKENS, i64::MAX),
    );
    let (sin_txt, sin_img) = (
        b.slice(&sin, 0, 0, TEXT_TOKENS),
        b.slice(&sin, 0, TEXT_TOKENS, i64::MAX),
    );
    let ones = b.g.floats(&vec![1.0; DIM as usize]);
    let one = b.g.scalar(1.0);

    // Timestep embedding and the modulations it drives.
    let t = b.linear(
        &tproj,
        "time_guidance_embed.timestep_embedder.linear_1",
        f32_,
    )?;
    let t = b.silu(&t);
    let temb = b.linear(&t, "time_guidance_embed.timestep_embedder.linear_2", f32_)?;
    let act = b.silu(&temb);
    let mods = |b: &mut Builder, module: &str, sets: usize| -> io::Result<Vec<[String; 3]>> {
        let m = b.linear(&act, module, f32_)?;
        let parts = b.split(&m, -1, &vec![DIM; 3 * sets]);
        Ok(parts
            .chunks(3)
            .map(|p| {
                let scale1 = b.op("Add", &[&p[1], &one]);
                [p[0].clone(), scale1, p[2].clone()] // shift, 1 + scale, gate
            })
            .collect())
    };
    let mod_img = mods(b, "double_stream_modulation_img.linear", 2)?;
    let mod_txt = mods(b, "double_stream_modulation_txt.linear", 2)?;
    let mod_single = mods(b, "single_stream_modulation.linear", 1)?;
    let out_mod = b.linear(&act, "norm_out.linear", f32_)?;
    let out_parts = b.split(&out_mod, -1, &[DIM, DIM]); // scale, then shift
    let out_scale1 = b.op("Add", &[&out_parts[0], &one]);

    let mut x = b.linear(&img, "x_embedder", f32_)?;
    let mut c = b.linear(&txt, "context_embedder", f32_)?;

    for i in 0..DOUBLE_BLOCKS {
        let p = format!("transformer_blocks.{i}");
        let [shift, scale1, gate] = &mod_img[0];
        let nx = b.modulate(&x, &ones, shift, scale1);
        let nx = b.cast(&nx, f16_);
        let [cshift, cscale1, cgate] = &mod_txt[0];
        let nc = b.modulate(&c, &ones, cshift, cscale1);
        let nc = b.cast(&nc, f16_);

        let heads = |b: &mut Builder, x16: &str, proj: &str, norm: &str| -> io::Result<String> {
            let y = b.linear16(x16, &format!("{p}.attn.{proj}"))?;
            let y = b.reshape(&y, &[-1, HEADS, HEAD_DIM]);
            b.rms(&y, &format!("{p}.attn.{norm}.weight"))
        };
        let q = heads(b, &nx, "to_q", "norm_q")?;
        let k = heads(b, &nx, "to_k", "norm_k")?;
        let eq = heads(b, &nc, "add_q_proj", "norm_added_q")?;
        let ek = heads(b, &nc, "add_k_proj", "norm_added_k")?;
        let v = b.linear(&nx, &format!("{p}.attn.to_v"), f16_)?;
        let ev = b.linear(&nc, &format!("{p}.attn.add_v_proj"), f16_)?;
        let q = b.rope(&q, &cos_img, &sin_img);
        let k = b.rope(&k, &cos_img, &sin_img);
        let eq = b.rope(&eq, &cos_txt, &sin_txt);
        let ek = b.rope(&ek, &cos_txt, &sin_txt);
        let q = b.concat(&[&eq, &q], 0);
        let k = b.concat(&[&ek, &k], 0);
        let v = b.concat(&[&ev, &v], 0);
        let att = b.attention(&q, &k, &v, &key_bias, chunks);
        let catt = b.slice(&att, 0, 0, TEXT_TOKENS);
        let xatt = b.slice(&att, 0, TEXT_TOKENS, i64::MAX);

        let xo = b.linear16(&xatt, &format!("{p}.attn.to_out.0"))?;
        let xo = b.op("Mul", &[&xo, gate]);
        x = b.op("Add", &[&x, &xo]);
        let co = b.linear16(&catt, &format!("{p}.attn.to_add_out"))?;
        let co = b.op("Mul", &[&co, cgate]);
        c = b.op("Add", &[&c, &co]);

        let ff = |b: &mut Builder, h: &str, m: &[String; 3], module: &str| -> io::Result<String> {
            let n = b.modulate(h, &ones, &m[0], &m[1]);
            let n = b.cast(&n, f16_);
            let y = b.swiglu(&n, &format!("{p}.{module}.linear_in"))?;
            let y = b.linear16(&y, &format!("{p}.{module}.linear_out"))?;
            let y = b.op("Mul", &[&y, &m[2]]);
            Ok(b.op("Add", &[h, &y]))
        };
        x = ff(b, &x, &mod_img[1], "ff")?;
        c = ff(b, &c, &mod_txt[1], "ff_context")?;
    }

    let cos_all = cos;
    let sin_all = sin;
    let mut h = b.concat(&[&c, &x], 0);
    for i in 0..SINGLE_BLOCKS {
        let p = format!("single_transformer_blocks.{i}");
        let [shift, scale1, gate] = &mod_single[0];
        let n = b.modulate(&h, &ones, shift, scale1);
        let n = b.cast(&n, f16_);
        // The fused input projection, cut by its outputs: q, k, v, the MLP's gate and up halves.
        let (d, m) = (DIM as usize, MLP as usize);
        let w = b.matrix_rows(&format!("{p}.attn.to_qkv_mlp_proj"), &[d, d, d, m, m], f16_)?;
        let proj: Vec<String> = w.iter().map(|w| b.op("MatMul", &[&n, w])).collect();
        let head = |b: &mut Builder, y16: &str, norm: &str| -> io::Result<String> {
            let y = b.cast(y16, f32_);
            let y = b.reshape(&y, &[-1, HEADS, HEAD_DIM]);
            let y = b.rms(&y, &format!("{p}.attn.{norm}.weight"))?;
            Ok(b.rope(&y, &cos_all, &sin_all))
        };
        let q = head(b, &proj[0], "norm_q")?;
        let k = head(b, &proj[1], "norm_k")?;
        let att = b.attention(&q, &k, &proj[2], &key_bias, chunks);
        let gate_mlp = b.silu(&proj[3]);
        let mlp = b.op("Mul", &[&gate_mlp, &proj[4]]);
        // The fused output projection, cut by its inputs: attention part + MLP part.
        let w = b.matrix_columns(&format!("{p}.attn.to_out"), &[d, m], f16_)?;
        let oa = b.op("MatMul", &[&att, &w[0]]);
        let om = b.op("MatMul", &[&mlp, &w[1]]);
        let oa = b.cast(&oa, f32_);
        let om = b.cast(&om, f32_);
        let o = b.op("Add", &[&oa, &om]);
        let o = b.op("Mul", &[&o, gate]);
        h = b.op("Add", &[&h, &o]);
    }

    let x = b.slice(&h, 0, TEXT_TOKENS, i64::MAX);
    let x = b.modulate(&x, &ones, &out_parts[1], &out_scale1);
    let v = b.linear(&x, "proj_out", f32_)?;
    b.g.output(
        &v,
        "velocity",
        f32_,
        &[Dim::Named("image_tokens"), Dim::Fixed(128)],
    );
    Ok(())
}

// -------------------------------------------------------------------------------- VAE

const VAE_EPS: f32 = 1e-6;
const VAE_GROUPS: i64 = 32;

impl Builder<'_> {
    fn conv(&mut self, x: &str, prefix: &str, stride: i64, pad: i64) -> io::Result<String> {
        let w = self.kernel(&format!("{prefix}.weight"))?;
        let out = self.file.info(&format!("{prefix}.weight"))?.shape[0] as i64;
        let k = self.file.info(&format!("{prefix}.weight"))?.shape[2] as i64;
        let bias = self.vector(&format!("{prefix}.bias"), &[out])?;
        Ok(self.g.node(
            "Conv",
            &[x, &w, &bias],
            &[
                Attr::Ints("kernel_shape", vec![k, k]),
                Attr::Ints("pads", vec![pad; 4]),
                Attr::Ints("strides", vec![stride, stride]),
            ],
        ))
    }

    /// GroupNorm (32 groups) with its affine parameters, as InstanceNormalization over groups.
    fn group_norm(&mut self, x: &str, prefix: &str) -> io::Result<String> {
        let channels = self.file.info(&format!("{prefix}.weight"))?.shape[0] as i64;
        let shape = self.op("Shape", &[x]);
        let grouped = self.reshape(x, &[0, VAE_GROUPS, -1]);
        let ones = self.g.floats(&vec![1.0; VAE_GROUPS as usize]);
        let zeros = self.g.floats(&vec![0.0; VAE_GROUPS as usize]);
        let n = self.g.node(
            "InstanceNormalization",
            &[&grouped, &ones, &zeros],
            &[Attr::Float("epsilon", VAE_EPS)],
        );
        let n = self.op("Reshape", &[&n, &shape]);
        let gamma = self.vector(&format!("{prefix}.weight"), &[channels, 1, 1])?;
        let beta = self.vector(&format!("{prefix}.bias"), &[channels, 1, 1])?;
        let n = self.op("Mul", &[&n, &gamma]);
        Ok(self.op("Add", &[&n, &beta]))
    }

    fn resnet(&mut self, x: &str, prefix: &str) -> io::Result<String> {
        let h = self.group_norm(x, &format!("{prefix}.norm1"))?;
        let h = self.silu(&h);
        let h = self.conv(&h, &format!("{prefix}.conv1"), 1, 1)?;
        let h = self.group_norm(&h, &format!("{prefix}.norm2"))?;
        let h = self.silu(&h);
        let h = self.conv(&h, &format!("{prefix}.conv2"), 1, 1)?;
        let shortcut = format!("{prefix}.conv_shortcut");
        let skip = if self
            .file
            .tensors
            .contains_key(&format!("{shortcut}.weight"))
        {
            self.conv(x, &shortcut, 1, 0)?
        } else {
            x.to_string()
        };
        Ok(self.op("Add", &[&skip, &h]))
    }

    /// The mid block's single-head self-attention over all positions, with its residual.
    fn vae_attention(&mut self, x: &str, prefix: &str) -> io::Result<String> {
        let shape = self.op("Shape", &[x]);
        let h = self.group_norm(x, &format!("{prefix}.group_norm"))?;
        let h = self.reshape(&h, &[0, 0, -1]);
        let h = self.transpose(&h, &[0, 2, 1]); // [1, HW, C]
        let channels = self.file.info(&format!("{prefix}.to_q.weight"))?.shape[0] as i64;
        let proj = |b: &mut Self, name: &str, x: &str| -> io::Result<String> {
            let y = b.linear(x, &format!("{prefix}.{name}"), DataType::F32)?;
            let bias = b.vector(&format!("{prefix}.{name}.bias"), &[channels])?;
            Ok(b.op("Add", &[&y, &bias]))
        };
        let q = proj(self, "to_q", &h)?;
        let k = proj(self, "to_k", &h)?;
        let v = proj(self, "to_v", &h)?;
        let kt = self.transpose(&k, &[0, 2, 1]);
        let s = self.op("MatMul", &[&q, &kt]);
        let scale = self.g.scalar(1.0 / (channels as f32).sqrt());
        let s = self.op("Mul", &[&s, &scale]);
        let p = self.g.node("Softmax", &[&s], &[Attr::Int("axis", -1)]);
        let o = self.op("MatMul", &[&p, &v]);
        let o = proj(self, "to_out.0", &o)?;
        let o = self.transpose(&o, &[0, 2, 1]);
        let o = self.op("Reshape", &[&o, &shape]);
        Ok(self.op("Add", &[&o, x]))
    }

    fn mid_block(&mut self, x: &str, prefix: &str) -> io::Result<String> {
        let x = self.resnet(x, &format!("{prefix}.resnets.0"))?;
        let x = self.vae_attention(&x, &format!("{prefix}.attentions.0"))?;
        self.resnet(&x, &format!("{prefix}.resnets.1"))
    }

    fn tail(&mut self, x: &str, prefix: &str) -> io::Result<String> {
        let x = self.group_norm(x, &format!("{prefix}.conv_norm_out"))?;
        let x = self.silu(&x);
        self.conv(&x, &format!("{prefix}.conv_out"), 1, 1)
    }
}

/// The VAE's encoder: `pixels` `[1, 3, H, W]` in [−1, 1] → `mean` `[1, 32, H/8, W/8]` (the
/// latent distribution's mean: no sampling).
fn encoder_graph(b: &mut Builder) -> io::Result<()> {
    let dims = [
        Dim::Fixed(1),
        Dim::Fixed(3),
        Dim::Named("height"),
        Dim::Named("width"),
    ];
    let x = b.g.input("pixels", DataType::F32, &dims);
    let mut x = b.conv(&x, "encoder.conv_in", 1, 1)?;
    for i in 0..4 {
        for r in 0..2 {
            x = b.resnet(&x, &format!("encoder.down_blocks.{i}.resnets.{r}"))?;
        }
        if i < 3 {
            // Asymmetric padding (right and bottom), then a stride of 2.
            let pads = b.g.ints(&[0, 0, 0, 0, 0, 0, 1, 1]);
            let padded = b.op("Pad", &[&x, &pads]);
            x = b.conv(
                &padded,
                &format!("encoder.down_blocks.{i}.downsamplers.0.conv"),
                2,
                0,
            )?;
        }
    }
    let x = b.mid_block(&x, "encoder.mid_block")?;
    let x = b.tail(&x, "encoder")?;
    let x = b.conv(&x, "quant_conv", 1, 0)?;
    let mean = b.slice(&x, 1, 0, 32);
    let out = [
        Dim::Fixed(1),
        Dim::Fixed(32),
        Dim::Named("h8"),
        Dim::Named("w8"),
    ];
    b.g.output(&mean, "mean", DataType::F32, &out);
    Ok(())
}

/// The VAE's decoder: `latent` `[1, 32, h, w]` → `image` `[1, 3, 8h, 8w]` in about [−1, 1].
fn decoder_graph(b: &mut Builder) -> io::Result<()> {
    let dims = [
        Dim::Fixed(1),
        Dim::Fixed(32),
        Dim::Named("h8"),
        Dim::Named("w8"),
    ];
    let z = b.g.input("latent", DataType::F32, &dims);
    let z = b.conv(&z, "post_quant_conv", 1, 0)?;
    let x = b.conv(&z, "decoder.conv_in", 1, 1)?;
    let mut x = b.mid_block(&x, "decoder.mid_block")?;
    for i in 0..4 {
        for r in 0..3 {
            x = b.resnet(&x, &format!("decoder.up_blocks.{i}.resnets.{r}"))?;
        }
        if i < 3 {
            let scales = b.g.floats(&[1.0, 1.0, 2.0, 2.0]);
            let up = b.g.node(
                "Resize",
                &[&x, "", &scales],
                &[
                    Attr::Str("mode", "nearest"),
                    Attr::Str("coordinate_transformation_mode", "asymmetric"),
                    Attr::Str("nearest_mode", "floor"),
                ],
            );
            x = b.conv(
                &up,
                &format!("decoder.up_blocks.{i}.upsamplers.0.conv"),
                1,
                1,
            )?;
        }
    }
    let x = b.tail(&x, "decoder")?;
    let out = [
        Dim::Fixed(1),
        Dim::Fixed(3),
        Dim::Named("height"),
        Dim::Named("width"),
    ];
    b.g.output(&x, "image", DataType::F32, &out);
    Ok(())
}

/// The whole model in one graph: the VAE's encoder (`pixels` → `mean`), the transformer (`img`,
/// `txt`, `tproj`, `rope_cos`, `rope_sin` → `velocity`) and the VAE's decoder (`latent` →
/// `image`). Each run asks for one output and computes only what it needs; in one session, the
/// three share their memory (the decoder reuses what the transformer's steps freed).
///
/// Returns the model and how many layers the LoRA was merged into.
pub fn pipeline(
    transformer: &mut SafeTensors,
    lora: Option<&mut Lora>,
    vae: &mut SafeTensors,
    chunks: usize,
) -> io::Result<(Built, usize)> {
    let total: usize = [&*transformer, &*vae]
        .iter()
        .flat_map(|f| f.tensors.values())
        .map(|t| t.end - t.start)
        .sum();
    let mut b = Builder {
        g: Graph::new(),
        weights: Vec::with_capacity(total + total / 8),
        file: transformer,
        lora,
        merged: 0,
    };
    transformer_graph(&mut b, chunks)?;
    b.file = vae;
    b.lora = None;
    encoder_graph(&mut b)?;
    decoder_graph(&mut b)?;
    let merged = b.merged;
    Ok((
        Built {
            model: b.g.model("flux2-klein", 18),
            weights: b.weights,
        },
        merged,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sequence_is_padded_to_a_multiple_of_8() {
        assert_eq!(padding(512, 3 * 3072), 0); // 1024×768
        assert_eq!(padding(512, 3 * 4015), 3); // 1168×880: 12557 → 12560
        let bias = key_bias(4, 1);
        assert_eq!(bias.len(), 4 * 8);
        assert!(bias[..24].iter().all(|&b| b == 0.0));
        // The 8 extra dimensions of a padding key sum to the padding score (queries add 1 × each).
        assert_eq!(bias[24..].iter().sum::<f32>(), PADDING_SCORE);
        // In half precision the score plus a large real score stays finite, and the padding's
        // weight after the softmax underflows to zero.
        assert!(crate::numeric::f16_to_f32(f32_to_f16(PADDING_SCORE - 30000.0)).is_finite());
        assert_eq!((PADDING_SCORE + 100.0 - 100.0).exp(), 0.0);
    }

    #[test]
    fn a_lora_adds_b_times_a() {
        // w [2, 3], a [1, 3], b [2, 1].
        let mut w = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        merge_lora(&mut w, 2, 3, &[1.0, 0.0, -1.0], &[2.0, 0.5], 1);
        assert_eq!(w, vec![3.0, 2.0, 1.0, 4.5, 5.0, 5.5]);
    }

    #[test]
    fn matrices_are_transposed_for_matmul() {
        let w: Vec<f32> = (0..6 * 70).map(|i| i as f32).collect(); // [out 6, in 70]
        let t = transpose_to(&w, 6, 70, DataType::F32);
        let at = |i: usize, o: usize| {
            let b = &t[(i * 6 + o) * 4..][..4];
            f32::from_le_bytes([b[0], b[1], b[2], b[3]])
        };
        assert_eq!(at(0, 0), 0.0);
        assert_eq!(at(69, 5), w[5 * 70 + 69]);
        assert_eq!(at(3, 2), w[2 * 70 + 3]);
        let h = transpose_to(&w, 6, 70, DataType::F16);
        assert_eq!(h.len(), 6 * 70 * 2);
        assert_eq!(
            u16::from_le_bytes([h[(3 * 6 + 2) * 2], h[(3 * 6 + 2) * 2 + 1]]),
            f32_to_f16(143.0)
        );
    }
}
