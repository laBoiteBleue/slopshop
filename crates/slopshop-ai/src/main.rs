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
use slopshop_ai::{MASK_SIDE, PROTOCOL_VERSION, Point, Request, Response, read_frame, write_frame};

/// SAM 2.1's input side.
const SIDE: usize = 1024;
/// SAM 2.1's models, by size, in the models folder (as downloaded from Hugging Face).
const SAM_GPU: &str = "onnx-community/sam2.1-hiera-base-plus-ONNX/onnx";
const SAM_CPU: &str = "onnx-community/sam2.1-hiera-tiny-ONNX/onnx";

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
            "cuda" => builder
                .with_execution_providers([ort::ep::CUDA::default().build().error_on_failure()])?,
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
