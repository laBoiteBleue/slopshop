//! The `slopshop-ai` helper (ADR 0025): runs the AI models for the editor, through ONNX Runtime
//! loaded at run time, and answers its requests (see the library's protocol) on its standard
//! input and output. Diagnostics go to standard error.
//!
//!     slopshop-ai --runtime <onnxruntime library> --models <folder> [--provider auto|cuda|directml|cpu]

use std::io::{self, BufReader, BufWriter};
use std::path::PathBuf;

use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use slopshop_ai::{
    MASK_SIDE, PROTOCOL_VERSION, Point, Request, Response, SEMANTIC_SIDE, SUBJECT_SIDE, read_frame,
    write_frame,
};

/// SAM 2.1's input side.
const SIDE: usize = 1024;
/// SAM 2.1's models, by size, in the models folder (as downloaded from Hugging Face).
const SAM_GPU: &str = "onnx-community/sam2.1-hiera-base-plus-ONNX/onnx";
const SAM_CPU: &str = "onnx-community/sam2.1-hiera-tiny-ONNX/onnx";
/// BiRefNet, by size: the full model on a GPU, the lite one on the CPU (in the models folder).
const BIREFNET_GPU: &str = "onnx-community/BiRefNet-ONNX/onnx/model_fp16.onnx";
const BIREFNET_CPU: &str = "onnx-community/BiRefNet_lite-ONNX/onnx/model_fp16.onnx";
/// SAM 3 (Kentaro Wada's ONNX export) and CLIP's merges for its text, in the models folder.
const SAM3: &str = "wkentaro/sam3-onnx-models-v0.3.0";
const CLIP_MERGES: &str = "openai/CLIP/bpe_simple_vocab_16e6.txt.gz";
/// SAM 3's input side and text length.
const SAM3_SIDE: usize = 1008;
const SAM3_TEXT: usize = 32;
/// ViTMatte-S, in the models folder.
const VITMATTE: &str = "Xenova/vitmatte-small-composition-1k/onnx/model.onnx";

struct Options {
    runtime: PathBuf,
    models: PathBuf,
    provider: String,
}

fn options() -> Result<Options, String> {
    let mut runtime = None;
    let mut models = None;
    let mut provider = "auto".to_owned();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let value = args.next().ok_or_else(|| format!("{arg} needs a value"))?;
        match arg.as_str() {
            "--runtime" => runtime = Some(PathBuf::from(value)),
            "--models" => models = Some(PathBuf::from(value)),
            "--provider" => provider = value,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(Options {
        runtime: runtime.ok_or("--runtime is required")?,
        models: models.ok_or("--models is required")?,
        provider,
    })
}

/// The execution providers to try, best first, for `--provider`.
fn providers(choice: &str) -> Vec<&'static str> {
    match choice {
        "cuda" => vec!["cuda"],
        "directml" => vec!["directml"],
        "cpu" => vec!["cpu"],
        _ => vec!["cuda", "directml", "cpu"],
    }
}

/// A session on `provider`, or why it cannot run there.
fn session(path: &std::path::Path, provider: &str) -> Result<Session, String> {
    fn build(path: &std::path::Path, provider: &str) -> ort::Result<Session> {
        let builder =
            Session::builder()?.with_optimization_level(GraphOptimizationLevel::Level3)?;
        let mut builder = match provider {
            // The memory arena grows by what is asked, not by powers of two: SAM 3 alone takes
            // most of a 16 GB card, and an arena doubled past the card spills into system
            // memory (Windows' fallback), which made each SAM 3 prompt take seconds.
            "cuda" => builder.with_execution_providers([ort::ep::CUDA::default()
                .with_arena_extend_strategy(ort::ep::ArenaExtendStrategy::SameAsRequested)
                .build()
                .error_on_failure()])?,
            // DirectML's requirements (ONNX Runtime's documentation).
            "directml" => builder
                .with_memory_pattern(false)?
                .with_parallel_execution(false)?
                .with_execution_providers([ort::ep::DirectML::default()
                    .build()
                    .error_on_failure()])?,
            _ => builder,
        };
        builder.commit_from_file(path)
    }
    build(path, provider).map_err(|e| format!("{}: {e}", path.display()))
}

/// SAM 2.1: its encoder, its prompt decoder, and the last image's embeddings.
struct Sam {
    encoder: Session,
    decoder: Session,
    /// Under the image's key: its three embeddings and its size.
    image: Option<(u64, [Tensor<f32>; 3], u32, u32)>,
}

impl Sam {
    /// SAM on the first provider of `choices` that runs it (base+ on a GPU, tiny on the CPU).
    fn load(
        models: &std::path::Path,
        choices: &[&'static str],
    ) -> Result<(Self, &'static str), String> {
        let mut errors = Vec::new();
        for &provider in choices {
            let dir = models.join(if provider == "cpu" { SAM_CPU } else { SAM_GPU });
            let suffix = if provider == "cpu" { "" } else { "_fp16" };
            let encoder = session(&dir.join(format!("vision_encoder{suffix}.onnx")), provider);
            let decoder = session(
                &dir.join(format!("prompt_encoder_mask_decoder{suffix}.onnx")),
                provider,
            );
            match (encoder, decoder) {
                (Ok(encoder), Ok(decoder)) => {
                    let sam = Sam {
                        encoder,
                        decoder,
                        image: None,
                    };
                    return Ok((sam, provider));
                }
                (Err(e), _) | (_, Err(e)) => errors.push(format!("{provider}: {e}")),
            }
        }
        Err(format!("SAM could not start: {}", errors.join("; ")))
    }

    fn encode(&mut self, key: u64, width: u32, height: u32, rgb: &[u8]) -> Result<(), String> {
        let pixels = normalized(width as usize, height as usize, rgb);
        let input =
            Tensor::from_array(([1usize, 3, SIDE, SIDE], pixels)).map_err(|e| e.to_string())?;
        let outputs = self
            .encoder
            .run(ort::inputs!["pixel_values" => input])
            .map_err(|e| e.to_string())?;
        let take = |name: &str| -> Result<Tensor<f32>, String> {
            let (shape, data) = outputs[name]
                .try_extract_tensor::<f32>()
                .map_err(|e| e.to_string())?;
            let dims: Vec<usize> = shape.iter().map(|&d| d.max(0) as usize).collect();
            Tensor::from_array((dims, data.to_vec())).map_err(|e| e.to_string())
        };
        let embeddings = [
            take("image_embeddings.0")?,
            take("image_embeddings.1")?,
            take("image_embeddings.2")?,
        ];
        self.image = Some((key, embeddings, width, height));
        Ok(())
    }

    fn decode(
        &mut self,
        key: u64,
        points: &[Point],
        boxed: Option<[f32; 4]>,
    ) -> Result<Response, String> {
        let Some((current, embeddings, width, height)) = &self.image else {
            return Err("no image encoded".into());
        };
        if *current != key {
            return Err("another image is encoded".into());
        }
        let (sx, sy) = (SIDE as f32 / *width as f32, SIDE as f32 / *height as f32);
        // At least one point: SAM needs a prompt; a box alone gets a padding point.
        let mut coords: Vec<f32> = points.iter().flat_map(|p| [p.x * sx, p.y * sy]).collect();
        let mut labels: Vec<i64> = points.iter().map(|p| i64::from(p.positive)).collect();
        if coords.is_empty() {
            coords.extend([0.0, 0.0]);
            labels.push(-10);
        }
        let count = labels.len();
        let boxes: Vec<f32> = boxed
            .map(|[x0, y0, x1, y1]| vec![x0 * sx, y0 * sy, x1 * sx, y1 * sy])
            .unwrap_or_default();
        let box_count = boxes.len() / 4;
        let tensor = |dims: Vec<usize>, data: Vec<f32>| {
            Tensor::from_array((dims, data)).map_err(|e| e.to_string())
        };
        let outputs = self
            .decoder
            .run(ort::inputs![
                "input_points" => tensor(vec![1, 1, count, 2], coords)?,
                "input_labels" => Tensor::from_array(([1usize, 1, count], labels)).map_err(|e| e.to_string())?,
                "input_boxes" => tensor(vec![1, box_count, 4], boxes)?,
                "image_embeddings.0" => embeddings[0].view(),
                "image_embeddings.1" => embeddings[1].view(),
                "image_embeddings.2" => embeddings[2].view(),
            ])
            .map_err(|e| e.to_string())?;
        let (_, scores) = outputs["iou_scores"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        let (_, masks) = outputs["pred_masks"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        // Several clicks pin the object down: SAM's first mask; one click is ambiguous: the
        // mask it rates best.
        let candidates = scores.len().min(masks.len() / (MASK_SIDE * MASK_SIDE));
        let best = if count > 1 || boxed.is_some() {
            0
        } else {
            (0..candidates)
                .max_by(|&a, &b| scores[a].total_cmp(&scores[b]))
                .unwrap_or(0)
        };
        let plane = MASK_SIDE * MASK_SIDE;
        let logits = masks
            .get(best * plane..(best + 1) * plane)
            .ok_or("SAM returned no mask")?
            .to_vec();
        Ok(Response::SamMask {
            logits,
            score: scores.get(best).copied().unwrap_or(0.0),
        })
    }
}

/// BiRefNet: the main subject of an image (Select Subject).
struct Subject {
    session: Session,
}

impl Subject {
    /// BiRefNet on the first provider of `choices` that runs it (full on a GPU, lite on the CPU).
    fn load(models: &std::path::Path, choices: &[&'static str]) -> Result<Self, String> {
        let mut errors = Vec::new();
        for &provider in choices {
            let path = models.join(if provider == "cpu" {
                BIREFNET_CPU
            } else {
                BIREFNET_GPU
            });
            match session(&path, provider) {
                Ok(session) => {
                    eprintln!("slopshop-ai: BiRefNet on {provider}");
                    return Ok(Subject { session });
                }
                Err(e) => errors.push(format!("{provider}: {e}")),
            }
        }
        Err(format!("BiRefNet could not start: {}", errors.join("; ")))
    }

    /// The subject's 1024² logits (positive: inside) over the whole image.
    fn run(&mut self, width: u32, height: u32, rgb: &[u8]) -> Result<Vec<f32>, String> {
        let pixels = normalized(width as usize, height as usize, rgb);
        let input =
            Tensor::from_array(([1usize, 3, SIDE, SIDE], pixels)).map_err(|e| e.to_string())?;
        let input_name = self.session.inputs()[0].name().to_owned();
        let output_name = self.session.outputs()[0].name().to_owned();
        let outputs = self
            .session
            .run(ort::inputs![input_name.as_str() => input])
            .map_err(|e| e.to_string())?;
        let (_, logits) = outputs[output_name.as_str()]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        logits
            .get(..SUBJECT_SIDE * SUBJECT_SIDE)
            .map(<[f32]>::to_vec)
            .ok_or_else(|| "BiRefNet returned a smaller mask".to_owned())
    }
}

/// SAM 3: every instance of what a text names (semantic selection).
struct Sam3 {
    image_encoder: Session,
    text_encoder: Session,
    decoder: Session,
    tokenizer: slopshop_ai::clip::ClipTokenizer,
    /// The last image's key and the encoder outputs the decoder reads.
    image: Option<(u64, [Tensor<f32>; 4])>,
}

impl Sam3 {
    /// SAM 3 on the first provider of `choices` that runs it.
    fn load(models: &std::path::Path, choices: &[&'static str]) -> Result<Self, String> {
        let merges = std::fs::File::open(models.join(CLIP_MERGES)).map_err(|e| e.to_string())?;
        let mut text = String::new();
        std::io::Read::read_to_string(&mut flate2::read::GzDecoder::new(merges), &mut text)
            .map_err(|e| e.to_string())?;
        let tokenizer = slopshop_ai::clip::ClipTokenizer::from_merges(&text)?;
        let dir = models.join(SAM3);
        let mut errors = Vec::new();
        for &provider in choices {
            let sessions = (|| {
                Ok::<_, String>((
                    session(&dir.join("sam3_image_encoder.onnx"), provider)?,
                    session(&dir.join("sam3_language_encoder.onnx"), provider)?,
                    session(&dir.join("sam3_decoder.onnx"), provider)?,
                ))
            })();
            match sessions {
                Ok((image_encoder, text_encoder, decoder)) => {
                    eprintln!("slopshop-ai: SAM 3 on {provider}");
                    return Ok(Sam3 {
                        image_encoder,
                        text_encoder,
                        decoder,
                        tokenizer,
                        image: None,
                    });
                }
                Err(e) => errors.push(format!("{provider}: {e}")),
            }
        }
        Err(format!("SAM 3 could not start: {}", errors.join("; ")))
    }

    /// What `text` names: how many instances, and their union (probabilities, 288²).
    fn run(
        &mut self,
        key: u64,
        width: u32,
        height: u32,
        rgb: &[u8],
        text: &str,
    ) -> Result<(u32, Vec<f32>), String> {
        let err = |e: ort::Error| e.to_string();
        if self.image.as_ref().map(|(k, _)| *k) != Some(key) {
            // The image stretched to 1008² (bilinear), 8-bit, planar; the graph normalizes.
            let (w, h) = (width as usize, height as usize);
            let plane = SAM3_SIDE * SAM3_SIDE;
            let mut pixels = vec![0u8; 3 * plane];
            let (sx, sy) = (w as f32 / SAM3_SIDE as f32, h as f32 / SAM3_SIDE as f32);
            for y in 0..SAM3_SIDE {
                let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (h - 1) as f32);
                let (y0, ty) = (fy.floor() as usize, fy.fract());
                let y1 = (y0 + 1).min(h - 1);
                for x in 0..SAM3_SIDE {
                    let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (w - 1) as f32);
                    let (x0, tx) = (fx.floor() as usize, fx.fract());
                    let x1 = (x0 + 1).min(w - 1);
                    for c in 0..3 {
                        let at = |x: usize, y: usize| f32::from(rgb[(y * w + x) * 3 + c]);
                        let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
                        let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
                        pixels[c * plane + y * SAM3_SIDE + x] =
                            (top * (1.0 - ty) + bottom * ty).round() as u8;
                    }
                }
            }
            let input =
                Tensor::from_array(([3usize, SAM3_SIDE, SAM3_SIDE], pixels)).map_err(err)?;
            let outputs = self
                .image_encoder
                .run(ort::inputs!["image" => input])
                .map_err(err)?;
            let take = |name: &str| -> Result<Tensor<f32>, String> {
                let (shape, data) = outputs[name]
                    .try_extract_tensor::<f32>()
                    .map_err(|e| e.to_string())?;
                let dims: Vec<usize> = shape.iter().map(|&d| d.max(0) as usize).collect();
                Tensor::from_array((dims, data.to_vec())).map_err(|e| e.to_string())
            };
            let encoded = [
                take("vision_pos_enc_2")?,
                take("backbone_fpn_0")?,
                take("backbone_fpn_1")?,
                take("backbone_fpn_2")?,
            ];
            self.image = Some((key, encoded));
        }
        let tokens = self.tokenizer.tokens(text, SAM3_TEXT);
        let tokens = Tensor::from_array(([1usize, SAM3_TEXT], tokens)).map_err(err)?;
        let language = self
            .text_encoder
            .run(ort::inputs!["tokens" => tokens])
            .map_err(err)?;
        let (_, mask) = language["text_attention_mask"]
            .try_extract_tensor::<bool>()
            .map_err(err)?;
        let language_mask =
            Tensor::from_array(([1usize, SAM3_TEXT], mask.to_vec())).map_err(err)?;
        let (shape, features) = language["text_memory"]
            .try_extract_tensor::<f32>()
            .map_err(err)?;
        let dims: Vec<usize> = shape.iter().map(|&d| d.max(0) as usize).collect();
        let language_features = Tensor::from_array((dims, features.to_vec())).map_err(err)?;
        let Some((_, image)) = &self.image else {
            return Err("no image encoded".into());
        };
        // Text only: the box prompt is masked out.
        let outputs = self
            .decoder
            .run(ort::inputs![
                "vision_pos_enc_2" => image[0].view(),
                "backbone_fpn_0" => image[1].view(),
                "backbone_fpn_1" => image[2].view(),
                "backbone_fpn_2" => image[3].view(),
                "language_mask" => language_mask,
                "language_features" => language_features,
                "box_coords" => Tensor::from_array(([1usize, 1, 4], vec![0f32; 4])).map_err(err)?,
                "box_labels" => Tensor::from_array(([1usize, 1], vec![1i64])).map_err(err)?,
                "box_masks" => Tensor::from_array(([1usize, 1], vec![true])).map_err(err)?,
            ])
            .map_err(err)?;
        let (_, masks) = outputs["masks"].try_extract_tensor::<f32>().map_err(err)?;
        let plane = SEMANTIC_SIDE * SEMANTIC_SIDE;
        let count = masks.len() / plane;
        let mut union = vec![0f32; plane];
        for instance in masks.chunks_exact(plane) {
            for (u, &p) in union.iter_mut().zip(instance) {
                *u = u.max(p);
            }
        }
        Ok((count as u32, union))
    }
}

/// ViTMatte-S: mattes an image guided by a trimap (Refine Edge).
struct Matte {
    session: Session,
}

impl Matte {
    /// ViTMatte on the first provider of `choices` that runs it.
    fn load(models: &std::path::Path, choices: &[&'static str]) -> Result<Self, String> {
        let mut errors = Vec::new();
        for &provider in choices {
            match session(&models.join(VITMATTE), provider) {
                Ok(session) => {
                    eprintln!("slopshop-ai: ViTMatte on {provider}");
                    return Ok(Matte { session });
                }
                Err(e) => errors.push(format!("{provider}: {e}")),
            }
        }
        Err(format!("ViTMatte could not start: {}", errors.join("; ")))
    }

    /// The matte of `width × height` RGB pixels guided by `trimap` (0 out, 255 in, else to
    /// decide), as 16-bit coverage. The model wants sides that are multiples of 32: the image is
    /// padded by repeating its edges (outside, for the trimap) and the matte cropped back.
    fn run(
        &mut self,
        width: u32,
        height: u32,
        rgb: &[u8],
        trimap: &[u8],
    ) -> Result<Vec<u16>, String> {
        let (w, h) = (width as usize, height as usize);
        let (pw, ph) = (w.div_ceil(32) * 32, h.div_ceil(32) * 32);
        let plane = pw * ph;
        let mut input = vec![0f32; 4 * plane];
        for y in 0..ph {
            let sy = y.min(h - 1);
            for x in 0..pw {
                let sx = x.min(w - 1);
                let i = sy * w + sx;
                for c in 0..3 {
                    input[c * plane + y * pw + x] = (f32::from(rgb[i * 3 + c]) / 255.0 - 0.5) / 0.5;
                }
                input[3 * plane + y * pw + x] = if x < w && y < h {
                    f32::from(trimap[i]) / 255.0
                } else {
                    0.0
                };
            }
        }
        let tensor = Tensor::from_array(([1usize, 4, ph, pw], input)).map_err(|e| e.to_string())?;
        let input_name = self.session.inputs()[0].name().to_owned();
        let output_name = self.session.outputs()[0].name().to_owned();
        let outputs = self
            .session
            .run(ort::inputs![input_name.as_str() => tensor])
            .map_err(|e| e.to_string())?;
        let (_, alpha) = outputs[output_name.as_str()]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        if alpha.len() < plane {
            return Err("ViTMatte returned a smaller matte".into());
        }
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                let a = alpha[y * pw + x].clamp(0.0, 1.0);
                out.push((a * f32::from(u16::MAX)).round() as u16);
            }
        }
        Ok(out)
    }
}

/// An RGB image resized (bilinear) to SAM's 1024² input and normalized with ImageNet's mean and
/// standard deviation, planar.
fn normalized(width: usize, height: usize, rgb: &[u8]) -> Vec<f32> {
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let plane = SIDE * SIDE;
    let mut out = vec![0f32; 3 * plane];
    let (sx, sy) = (width as f32 / SIDE as f32, height as f32 / SIDE as f32);
    for y in 0..SIDE {
        let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (height - 1) as f32);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(height - 1);
        for x in 0..SIDE {
            let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (width - 1) as f32);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(width - 1);
            for c in 0..3 {
                let at = |x: usize, y: usize| f32::from(rgb[(y * width + x) * 3 + c]);
                let top = at(x0, y0) * (1.0 - tx) + at(x1, y0) * tx;
                let bottom = at(x0, y1) * (1.0 - tx) + at(x1, y1) * tx;
                let v = (top * (1.0 - ty) + bottom * ty) / 255.0;
                out[c * plane + y * SIDE + x] = (v - MEAN[c]) / STD[c];
            }
        }
    }
    out
}

fn main() {
    let options = match options() {
        Ok(options) => options,
        Err(e) => {
            eprintln!("slopshop-ai: {e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = ort::init_from(&options.runtime).map(|b| b.commit()) {
        eprintln!("slopshop-ai: ONNX Runtime could not be loaded: {e}");
        std::process::exit(3);
    }
    let choices = providers(&options.provider);
    // The models load on the first request that needs them: greeting stays instant.
    let mut sam: Option<Sam> = None;
    let mut matte: Option<Matte> = None;
    let mut subject: Option<Subject> = None;
    let mut sam3: Option<Sam3> = None;
    let mut provider = "none";
    let mut input = BufReader::new(io::stdin().lock());
    let mut output = BufWriter::new(io::stdout().lock());
    loop {
        let frame = match read_frame(&mut input) {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(e) => {
                eprintln!("slopshop-ai: {e}");
                break;
            }
        };
        let response = match Request::decode(&frame) {
            Err(e) => Response::Failed(e.to_string()),
            Ok(Request::Quit) => break,
            Ok(Request::Hello) => Response::Hello {
                version: PROTOCOL_VERSION,
                provider: provider.to_owned(),
            },
            Ok(Request::SamEncode {
                key,
                width,
                height,
                rgb,
            }) => {
                let loaded = match sam.as_mut() {
                    Some(sam) => Ok(sam),
                    None => Sam::load(&options.models, &choices).map(|(loaded, used)| {
                        provider = used;
                        eprintln!("slopshop-ai: SAM 2.1 on {used}");
                        sam.insert(loaded)
                    }),
                };
                match loaded.and_then(|sam| sam.encode(key, width, height, &rgb)) {
                    Ok(()) => Response::Done,
                    Err(e) => Response::Failed(e),
                }
            }
            Ok(Request::Matte {
                width,
                height,
                rgb,
                trimap,
            }) => {
                let loaded = match matte.as_mut() {
                    Some(matte) => Ok(matte),
                    None => Matte::load(&options.models, &choices).map(|m| matte.insert(m)),
                };
                match loaded.and_then(|m| m.run(width, height, &rgb, &trimap)) {
                    Ok(alpha) => Response::Alpha(alpha),
                    Err(e) => Response::Failed(e),
                }
            }
            Ok(Request::Subject { width, height, rgb }) => {
                let loaded = match subject.as_mut() {
                    Some(subject) => Ok(subject),
                    None => {
                        // One large model in memory at a time.
                        sam3 = None;
                        Subject::load(&options.models, &choices).map(|s| subject.insert(s))
                    }
                };
                match loaded.and_then(|s| s.run(width, height, &rgb)) {
                    Ok(logits) => Response::SubjectMask(logits),
                    Err(e) => Response::Failed(e),
                }
            }
            Ok(Request::Semantic {
                key,
                width,
                height,
                rgb,
                text,
            }) => {
                let loaded = match sam3.as_mut() {
                    Some(sam3) => Ok(sam3),
                    None => {
                        // One large model in memory at a time.
                        subject = None;
                        Sam3::load(&options.models, &choices).map(|s| sam3.insert(s))
                    }
                };
                match loaded.and_then(|s| s.run(key, width, height, &rgb, &text)) {
                    Ok((count, probabilities)) => Response::SemanticMask {
                        count,
                        probabilities,
                    },
                    Err(e) => Response::Failed(e),
                }
            }
            Ok(Request::SamDecode { key, points, boxed }) => match sam.as_mut() {
                Some(sam) => sam
                    .decode(key, &points, boxed)
                    .unwrap_or_else(Response::Failed),
                None => Response::Failed("no image encoded".into()),
            },
        };
        if write_frame(&mut output, &response.encode()).is_err() {
            break;
        }
    }
}
