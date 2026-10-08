//! The Erase tool on ONNX Runtime (ADR 0045): FLUX.2 [klein] with the `erase_v1` LoRA, as one
//! graph ([`crate::flux2::pipeline`]) in one DirectML session. Given the photo and the selection
//! at the model's working size, it returns the image with the selection replaced by its
//! background; resizing and compositing around it are the caller's.

use std::borrow::Cow;
use std::path::Path;

use ort::session::builder::GraphOptimizationLevel;
use ort::session::{OutputSelector, RunOptions, Session, SessionInputValue};
use ort::value::Tensor;

use crate::Stage;
use crate::erase::{self, LatentStats};
use crate::flux2::{self, Built, Lora, Storage};
use crate::numeric::round_bf16;
use crate::safetensors::SafeTensors;

/// The files the tool needs.
#[derive(Debug, Clone, Copy)]
pub struct Files<'a> {
    pub transformer: &'a Path,
    pub vae: &'a Path,
    pub lora: &'a Path,
    pub embedding: &'a Path,
}

/// Query chunks of the attention (ADR 0045).
const CHUNKS: usize = 16;
/// Loading's progress, in thousandths: the weights read up to [`READ`], then the session.
const LOADING: u32 = 1000;
const READ: u32 = 950;

/// The loaded model.
#[derive(Debug)]
pub struct Eraser {
    session: Session,
    stats: LatentStats,
    /// The prompt's embedding `[512, 7680]`.
    text: Vec<f32>,
}

fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A DirectML session on a built model, its weights given from memory.
pub fn session(
    built: Built,
    level: GraphOptimizationLevel,
    profile: Option<&Path>,
) -> ort::Result<Session> {
    let Built { model, weights } = built;
    let mut builder = Session::builder()?;
    if let Some(path) = profile {
        builder = builder.with_profiling(path)?;
    }
    builder
        .with_optimization_level(level)?
        .with_memory_pattern(false)?
        .with_parallel_execution(false)?
        .with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()])?
        .with_external_initializer_file_in_memory(flux2::WEIGHTS, Cow::Owned(weights))?
        .commit_from_memory(&model)
}

impl Eraser {
    /// Builds the graph (the LoRA merged, the weights stored as `storage` says) and its session,
    /// telling `progress` how far it is ([`Stage::Loading`], in thousandths: the weights read
    /// up to 950, then the session, about a third of the time, at once).
    pub fn load(
        files: Files<'_>,
        storage: Storage,
        progress: &mut dyn FnMut(Stage, u32, u32),
    ) -> Result<Self, String> {
        progress(Stage::Loading, 0, LOADING);
        let mut vae = SafeTensors::open(files.vae).map_err(error)?;
        let stats = LatentStats::new(
            vae.f32s("bn.running_mean").map_err(error)?,
            &vae.f32s("bn.running_var").map_err(error)?,
            1e-4,
        );
        let mut transformer = SafeTensors::open(files.transformer).map_err(error)?;
        let mut lora = Lora::open(files.lora).map_err(error)?;
        let targeted = lora.modules().len();
        let mut reported = 0;
        let mut read = |done: u64, total: u64| {
            let at = (done * u64::from(READ) / total.max(1)) as u32;
            // Every 1 % at most: one frame each to the editor.
            if at >= reported + LOADING / 100 {
                reported = at;
                progress(Stage::Loading, at, LOADING);
            }
        };
        let (built, merged) = flux2::pipeline(
            &mut transformer,
            Some(&mut lora),
            &mut vae,
            CHUNKS,
            storage,
            &mut read,
        )
        .map_err(error)?;
        progress(Stage::Loading, READ, LOADING);
        if merged != targeted || merged == 0 {
            return Err(format!("LoRA: {merged} of {targeted} layers merged"));
        }
        let text = SafeTensors::open(files.embedding)
            .and_then(|mut e| e.f32s("prompt_embeds"))
            .map_err(error)?;
        if text.len() != flux2::TEXT_TOKENS as usize * 7680 {
            return Err("unexpected prompt embedding".into());
        }
        let session = session(built, GraphOptimizationLevel::Level3, None).map_err(error)?;
        progress(Stage::Loading, LOADING, LOADING);
        Ok(Self {
            session,
            stats,
            text,
        })
    }

    /// Runs the model for one of its outputs. ONNX Runtime runs the whole graph: the parts not
    /// asked for get tiny inputs.
    fn fetch(
        &mut self,
        inputs: Vec<(&'static str, Tensor<f32>)>,
        output: &str,
    ) -> Result<Vec<f32>, String> {
        let options = RunOptions::new()
            .map_err(error)?
            .with_outputs(OutputSelector::no_default().with(output));
        let given: Vec<&str> = inputs.iter().map(|(k, _)| *k).collect();
        let mut all = inputs;
        let tensor = |dims: Vec<usize>, value: f32| {
            let len = dims.iter().product();
            Tensor::from_array((dims, vec![value; len])).map_err(error)
        };
        if !given.contains(&"pixels") {
            all.push(("pixels", tensor(vec![1, 3, 8, 8], 0.0)?));
        }
        if !given.contains(&"latent") {
            all.push(("latent", tensor(vec![1, 32, 1, 1], 0.0)?));
        }
        if !given.contains(&"img") {
            let text = flux2::TEXT_TOKENS as usize;
            all.push(("img", tensor(vec![8, 128], 0.0)?));
            all.push(("txt", tensor(vec![text, 7680], 0.0)?));
            all.push(("tproj", tensor(vec![1, 256], 0.0)?));
            all.push(("rope_cos", tensor(vec![text + 8, 128], 1.0)?));
            all.push(("rope_sin", tensor(vec![text + 8, 128], 0.0)?));
            all.push(("key_bias", tensor(vec![text + 8, 8], 0.0)?));
        }
        let all: Vec<(Cow<'static, str>, SessionInputValue)> = all
            .into_iter()
            .map(|(k, v)| (Cow::Borrowed(k), v.into()))
            .collect();
        let outputs = self
            .session
            .run_with_options(all, &options)
            .map_err(error)?;
        let (_, values) = outputs[output].try_extract_tensor::<f32>().map_err(error)?;
        Ok(values.to_vec())
    }

    /// Erases the selection of an 8-bit RGB image at the working size (`width`, `height`
    /// multiples of 16; `mask` above 127 to erase), from `noise` (`[128, h/16, w/16]`, standard
    /// normal). Returns the model's whole output at that size: the caller composites it through
    /// the selection.
    pub fn run(
        &mut self,
        (width, height): (usize, usize),
        rgb: &[u8],
        mask: &[u8],
        noise: &[f32],
        progress: &mut dyn FnMut(Stage, u32, u32),
    ) -> Result<Vec<u8>, String> {
        let (lh, lw) = (height / 16, width / 16);
        let n = lh * lw;
        if width % 16 != 0 || height % 16 != 0 || noise.len() != n * 128 {
            return Err("Erase: bad sizes".into());
        }
        progress(Stage::Encoding, 0, 2);
        let mut refs = Vec::with_capacity(2 * n * 128);
        let mask_rgb = erase::mask_rgb(mask);
        for (i, image) in [rgb, &mask_rgb].into_iter().enumerate() {
            let pixels = Tensor::from_array((
                [1, 3, height, width],
                erase::vae_input(image, width, height),
            ))
            .map_err(error)?;
            let latent = self.fetch(vec![("pixels", pixels)], "mean")?;
            refs.extend(erase::pack(&latent, lh, lw, &self.stats));
            progress(Stage::Encoding, i as u32 + 1, 2);
        }
        // The reference images enter the transformer in bfloat16.
        refs.iter_mut().for_each(|v| *v = round_bf16(*v));

        let text = flux2::TEXT_TOKENS as usize;
        let mut x = erase::pack_noise(noise, lh, lw);
        let sigmas = erase::sigmas(n);
        // The sequence padded for DirectML: identity rotations, masked out of the attention.
        let (mut cos, mut sin) = erase::rope_tables(text, lh, lw);
        let extra = flux2::padding(text, 3 * n);
        let tokens = text + 3 * n + extra;
        cos.resize(tokens * 128, 1.0);
        sin.resize(tokens * 128, 0.0);
        let key_bias = flux2::key_bias(tokens, extra);
        let steps = erase::STEPS as u32;
        progress(Stage::Denoising, 0, steps);
        for i in 0..erase::STEPS {
            let mut img = x.clone();
            img.extend_from_slice(&refs);
            img.resize((3 * n + extra) * 128, 0.0);
            let tensor = |dims: Vec<usize>, data: Vec<f32>| -> Result<Tensor<f32>, String> {
                Tensor::from_array((dims, data)).map_err(error)
            };
            let inputs = vec![
                ("img", tensor(vec![3 * n + extra, 128], img)?),
                ("txt", tensor(vec![text, 7680], self.text.clone())?),
                (
                    "tproj",
                    tensor(vec![1, 256], erase::timestep_projection(sigmas[i]).to_vec())?,
                ),
                ("rope_cos", tensor(vec![tokens, 128], cos.clone())?),
                ("rope_sin", tensor(vec![tokens, 128], sin.clone())?),
                ("key_bias", tensor(vec![tokens, 8], key_bias.clone())?),
            ];
            let v = self.fetch(inputs, "velocity")?;
            let v = &v[..n * 128];
            if v.iter().any(|v| !v.is_finite()) {
                return Err("Erase: the model diverged".into());
            }
            erase::euler_step(&mut x, v, sigmas[i], sigmas[i + 1]);
            progress(Stage::Denoising, i as u32 + 1, steps);
        }

        progress(Stage::Decoding, 0, 1);
        let latent = Tensor::from_array((
            [1, 32, 2 * lh, 2 * lw],
            erase::unpack(&x, lh, lw, &self.stats),
        ))
        .map_err(error)?;
        let image = self.fetch(vec![("latent", latent)], "image")?;
        progress(Stage::Decoding, 1, 1);
        Ok(erase::vae_output(&image, width, height))
    }
}
