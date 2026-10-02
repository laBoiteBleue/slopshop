//! The `slopshop-ai` helper (ADR 0025): runs the AI models for the editor, through ONNX Runtime
//! loaded at run time, and answers its requests (see the library's protocol) on its standard
//! input and output. Diagnostics go to standard error.
//!
//!     slopshop-ai --runtime <onnxruntime library> --models <folder> [--provider auto|directml|coreml|cpu]

use std::io::{self, BufReader, BufWriter};
use std::path::PathBuf;

use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use slopshop_ai::{
    MASK_SIDE, PROTOCOL_VERSION, Point, Request, Response, SUBJECT_SIDE, read_frame, write_frame,
};

/// SAM 2.1's input side.
const SIDE: usize = 1024;
/// SAM 2.1's models, by size, in the models folder (as downloaded from Hugging Face).
const SAM_BASE_PLUS: &str = "onnx-community/sam2.1-hiera-base-plus-ONNX/onnx";
const SAM_TINY: &str = "onnx-community/sam2.1-hiera-tiny-ONNX/onnx";
/// BiRefNet, by size: the full model on a GPU, the lite one on the CPU (in the models folder).
const BIREFNET: &str = "onnx-community/BiRefNet-ONNX/onnx";
const BIREFNET_LITE: &str = "onnx-community/BiRefNet_lite-ONNX/onnx";

/// The models for a provider (as the app's manifest installs them): half precision on
/// DirectML, single on Core ML (half precision falls back to the CPU there), the small
/// models on the CPU. `(SAM's folder, its files' suffix, BiRefNet's file)`.
fn models_for(provider: &str) -> (&'static str, &'static str, String) {
    match provider {
        "directml" => (
            SAM_BASE_PLUS,
            "_fp16",
            format!("{BIREFNET}/model_fp16.onnx"),
        ),
        "coreml" => (SAM_BASE_PLUS, "", format!("{BIREFNET}/model.onnx")),
        _ => (SAM_TINY, "", format!("{BIREFNET_LITE}/model.onnx")),
    }
}
/// ViTMatte's base and small models, in the models folder.
const VITMATTE_BASE: &str = "Xenova/vitmatte-base-composition-1k/onnx/model.onnx";
const VITMATTE_SMALL: &str = "Xenova/vitmatte-small-composition-1k/onnx/model.onnx";

/// The ViTMatte to run on `provider`: the base model on a GPU, the small one on the CPU (ADR
/// 0025), else whichever is installed.
fn vitmatte(models: &std::path::Path, provider: &str) -> std::path::PathBuf {
    let preferred = match provider {
        "directml" | "coreml" => [VITMATTE_BASE, VITMATTE_SMALL],
        _ => [VITMATTE_SMALL, VITMATTE_BASE],
    };
    preferred
        .iter()
        .map(|path| models.join(path))
        .find(|path| path.is_file())
        .unwrap_or_else(|| models.join(preferred[0]))
}

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
        "directml" => vec!["directml"],
        "coreml" => vec!["coreml"],
        "cpu" => vec!["cpu"],
        _ if cfg!(target_os = "macos") => vec!["coreml", "cpu"],
        _ => vec!["directml", "cpu"],
    }
}

/// A session on `provider`, or why it cannot run there.
fn session(path: &std::path::Path, provider: &str) -> Result<Session, String> {
    fn build(path: &std::path::Path, provider: &str) -> ort::Result<Session> {
        let builder =
            Session::builder()?.with_optimization_level(GraphOptimizationLevel::Level3)?;
        let mut builder = match provider {
            // ML Program: Core ML 5's format, the one with the widest operator coverage. Core
            // ML takes only the parts of a graph with fixed shapes (SAM's decoder has a varying
            // number of points, which Core ML cannot compile): the rest runs on the CPU.
            "coreml" => builder.with_execution_providers([ort::ep::CoreML::default()
                .with_model_format(ort::ep::coreml::ModelFormat::MLProgram)
                .with_static_input_shapes(true)
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
    match build(path, provider) {
        Ok(session) => Ok(session),
        // A model Core ML cannot compile still runs, on the CPU (the same model file).
        Err(e) if provider == "coreml" => {
            eprintln!(
                "slopshop-ai: Core ML cannot run {}: {e}; on the CPU",
                path.display()
            );
            build(path, "cpu").map_err(|e| format!("{}: {e}", path.display()))
        }
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
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
            let (sam, suffix, _) = models_for(provider);
            let dir = models.join(sam);
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
        // Several clicks pin the object down: SAM's first mask; one click is ambiguous.
        let plane = MASK_SIDE * MASK_SIDE;
        let candidates = scores.len().min(masks.len() / plane);
        let best = if count > 1 || boxed.is_some() {
            0
        } else {
            clicked_object(&scores[..candidates], masks, plane)
        };
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
    /// BiRefNet on the first provider of `choices` that runs it (full on a GPU, lite on the CPU;
    /// see `models_for`).
    fn load(models: &std::path::Path, choices: &[&'static str]) -> Result<Self, String> {
        let mut errors = Vec::new();
        for &provider in choices {
            let path = models.join(models_for(provider).2);
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

/// ViTMatte: mattes an image guided by a trimap (Refine Edge).
struct Matte {
    session: Session,
}

impl Matte {
    /// ViTMatte on the first provider of `choices` that runs it.
    fn load(models: &std::path::Path, choices: &[&'static str]) -> Result<Self, String> {
        let mut errors = Vec::new();
        for &provider in choices {
            match session(&vitmatte(models, provider), provider) {
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
    // The models load on the first request that needs them: greeting stays instant. BiRefNet
    // and ViTMatte are never needed together and each holds gigabytes of GPU memory while
    // loaded (BiRefNet about 10 GB on DirectML): loading one unloads the other, else the second
    // spills into shared memory and runs several times slower. SAM is small and stays.
    let mut sam: Option<Sam> = None;
    let mut matte: Option<Matte> = None;
    let mut subject: Option<Subject> = None;
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
                subject = None;
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
                matte = None;
                let loaded = match subject.as_mut() {
                    Some(subject) => Ok(subject),
                    None => Subject::load(&options.models, &choices).map(|s| subject.insert(s)),
                };
                match loaded.and_then(|s| s.run(width, height, &rgb)) {
                    Ok(logits) => Response::SubjectMask(logits),
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

/// Of SAM's masks for one click, the object the click means, as Photoshop's Object Selection
/// picks: SAM's first mask is the whole object (a person), the others parts of it (the hair,
/// the jacket). The whole object when SAM is fairly sure of it and its edge is stable (a click
/// on its hair or its sleeve); otherwise the mask SAM rates best (a click on a hand, a hat, the
/// face, or on the background). Measured on 29 clicks over 3 photos: 24 as intended, against
/// 18 for the best-rated mask alone and 16 for the first mask alone.
fn clicked_object(scores: &[f32], masks: &[f32], plane: usize) -> usize {
    /// SAM's confidence in the whole object, and the stability of its edge, that make it the
    /// answer.
    const WHOLE_SCORE: f32 = 0.2;
    const WHOLE_STABILITY: f32 = 0.85;
    let best = (0..scores.len())
        .max_by(|&a, &b| scores[a].total_cmp(&scores[b]))
        .unwrap_or(0);
    let Some(whole) = masks.get(..plane) else {
        return best;
    };
    // Stability: the area well inside over the area barely inside (logits above 1 and -1).
    let strong = whole.iter().filter(|&&v| v > 1.0).count();
    let weak = whole.iter().filter(|&&v| v > -1.0).count();
    let stability = if weak > 0 {
        strong as f32 / weak as f32
    } else {
        0.0
    };
    if scores.first().is_some_and(|&s| s >= WHOLE_SCORE) && stability >= WHOLE_STABILITY {
        0
    } else {
        best
    }
}

#[cfg(test)]
mod tests {
    use super::clicked_object;

    #[test]
    fn a_click_takes_the_whole_object_only_when_sam_stands_by_it() {
        // Three 2×2 masks: the whole object (crisp), a part, a smaller part.
        let masks = [
            5.0, 5.0, 5.0, 5.0, //
            5.0, 5.0, -5.0, -5.0, //
            5.0, -5.0, -5.0, -5.0,
        ];
        // A click on the hair: SAM fairly sure of the whole person.
        assert_eq!(clicked_object(&[0.6, 0.9, 0.7], &masks, 4), 0);
        // A click on a hand: SAM barely believes in the whole person.
        assert_eq!(clicked_object(&[0.1, 0.9, 0.7], &masks, 4), 1);
        // A whole object with a soft, uncertain edge: the best-rated part.
        let soft = [
            0.5, 0.5, 0.5, -0.5, 5.0, 5.0, -5.0, -5.0, 5.0, -5.0, -5.0, -5.0,
        ];
        assert_eq!(clicked_object(&[0.6, 0.7, 0.9], &soft, 4), 2);
    }
}
