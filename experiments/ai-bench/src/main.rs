//! Experiment: how fast do the candidate AI selection models run through ONNX Runtime, per
//! execution provider (see docs/research/ai-selection.md, section 7)? Not part of SlopShop.
//!
//! For every stage of every model (an encoder, a decoder…), with synthetic inputs of the shapes
//! the app would send: session creation, first run, then warm runs (median and 95th
//! percentile), and the GPU memory the session added (nvidia-smi, when there is one). Latency
//! does not depend on the values, so no image or tokenizer is needed here; quality is another
//! experiment.
//!
//!     cargo run --release --features directml|webgpu -- [--models <dir>]
//!         [--ep cpu|directml|webgpu]... [--only <model>] [--runs N] [--opt basic|none]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::{DynValue, Tensor, TensorElementType, ValueType};

/// One ONNX graph of a model, with the sizes of its dynamic dimensions.
struct Stage {
    name: &'static str,
    file: String,
    /// Input name → shape, for inputs with dynamic dimensions (others keep theirs).
    shapes: Vec<(&'static str, Vec<i64>)>,
}

struct Model {
    name: &'static str,
    repo: &'static str,
    stages: Vec<Stage>,
}

fn sam2(name: &'static str, repo: &'static str, suffix: &str) -> Model {
    Model {
        name,
        repo,
        stages: vec![
            Stage {
                name: "image encoder",
                file: format!("onnx/vision_encoder{suffix}.onnx"),
                shapes: vec![("pixel_values", vec![1, 3, 1024, 1024])],
            },
            Stage {
                name: "prompt decoder (1 click)",
                file: format!("onnx/prompt_encoder_mask_decoder{suffix}.onnx"),
                shapes: vec![
                    ("input_points", vec![1, 1, 1, 2]),
                    ("input_labels", vec![1, 1, 1]),
                    ("input_boxes", vec![1, 0, 4]),
                    ("image_embeddings.0", vec![1, 32, 256, 256]),
                    ("image_embeddings.1", vec![1, 64, 128, 128]),
                    ("image_embeddings.2", vec![1, 256, 64, 64]),
                ],
            },
        ],
    }
}

fn birefnet(name: &'static str, repo: &'static str, sides: &[i64]) -> Model {
    Model {
        name,
        repo,
        stages: sides
            .iter()
            .map(|&side| Stage {
                name: if side == 1024 { "1024²" } else { "2048²" },
                file: "onnx/model_fp16.onnx".to_owned(),
                shapes: vec![("input_image", vec![1, 3, side, side])],
            })
            .collect(),
    }
}

fn models() -> Vec<Model> {
    vec![
        sam2("sam2.1-tiny", "onnx-community/sam2.1-hiera-tiny-ONNX", ""),
        sam2(
            "sam2.1-tiny-fp16",
            "onnx-community/sam2.1-hiera-tiny-ONNX",
            "_fp16",
        ),
        sam2(
            "sam2.1-small-fp16",
            "onnx-community/sam2.1-hiera-small-ONNX",
            "_fp16",
        ),
        sam2(
            "sam2.1-base-plus-fp16",
            "onnx-community/sam2.1-hiera-base-plus-ONNX",
            "_fp16",
        ),
        birefnet(
            "birefnet-lite-fp16",
            "onnx-community/BiRefNet_lite-ONNX",
            &[1024],
        ),
        birefnet("birefnet-fp16", "onnx-community/BiRefNet-ONNX", &[1024]),
        birefnet(
            "birefnet-dynamic-fp16",
            "onnx-community/BiRefNet_dynamic-1024x1024-ONNX",
            &[1024, 2048],
        ),
        Model {
            name: "sam3",
            repo: "wkentaro/sam3-onnx-models-v0.3.0",
            stages: vec![
                Stage {
                    name: "image encoder",
                    file: "sam3_image_encoder.onnx".to_owned(),
                    shapes: vec![("image", vec![3, 1008, 1008])],
                },
                Stage {
                    name: "text encoder",
                    file: "sam3_language_encoder.onnx".to_owned(),
                    shapes: vec![("tokens", vec![1, 32])],
                },
                Stage {
                    name: "decoder (text, no box)",
                    file: "sam3_decoder.onnx".to_owned(),
                    shapes: vec![
                        ("box_coords", vec![1, 1, 4]),
                        ("box_labels", vec![1, 1]),
                        ("box_masks", vec![1, 1]),
                        ("language_mask", vec![1, 32]),
                        ("language_features", vec![32, 1, 256]),
                    ],
                },
            ],
        },
        Model {
            name: "grounding-dino-tiny-fp16",
            repo: "onnx-community/grounding-dino-tiny-ONNX",
            stages: vec![Stage {
                name: "text → boxes (16 tokens)",
                file: "onnx/model_fp16.onnx".to_owned(),
                shapes: vec![
                    ("pixel_values", vec![1, 3, 800, 800]),
                    ("pixel_mask", vec![1, 800, 800]),
                    ("input_ids", vec![1, 16]),
                    ("token_type_ids", vec![1, 16]),
                    ("attention_mask", vec![1, 16]),
                ],
            }],
        },
    ]
}

#[derive(Clone, Copy, PartialEq)]
enum Ep {
    Cpu,
    #[cfg(feature = "directml")]
    DirectMl,
    #[cfg(feature = "webgpu")]
    WebGpu,
}

impl Ep {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "cpu" => Some(Ep::Cpu),
            #[cfg(feature = "directml")]
            "directml" => Some(Ep::DirectMl),
            #[cfg(feature = "webgpu")]
            "webgpu" => Some(Ep::WebGpu),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Ep::Cpu => "CPU",
            #[cfg(feature = "directml")]
            Ep::DirectMl => "DirectML",
            #[cfg(feature = "webgpu")]
            Ep::WebGpu => "WebGPU",
        }
    }
}

fn session(path: &Path, ep: Ep, level: GraphOptimizationLevel) -> ort::Result<Session> {
    let builder = Session::builder()?.with_optimization_level(level)?;
    let mut builder = match ep {
        Ep::Cpu => builder,
        #[cfg(feature = "webgpu")]
        Ep::WebGpu => builder
            .with_execution_providers([ort::ep::WebGPU::default().build().error_on_failure()])?,
        #[cfg(feature = "directml")]
        Ep::DirectMl => builder
            // DirectML's requirements (ONNX Runtime documentation).
            .with_memory_pattern(false)?
            .with_parallel_execution(false)?
            .with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()])?,
    };
    builder.commit_from_file(path)
}

/// Synthetic inputs of the right types and shapes: the values do not change the latency.
fn inputs(session: &Session, stage: &Stage) -> Result<Vec<(String, DynValue)>, String> {
    session
        .inputs()
        .iter()
        .map(|input| {
            let name = input.name().to_owned();
            let ValueType::Tensor { ty, shape, .. } = input.dtype() else {
                return Err(format!("{name}: not a tensor input"));
            };
            let wanted = stage
                .shapes
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, s)| s);
            let dims: Vec<usize> = match wanted {
                Some(dims) => dims.iter().map(|&d| d as usize).collect(),
                None => shape
                    .iter()
                    .map(|&d| if d < 0 { 1 } else { d as usize })
                    .collect(),
            };
            let count: usize = dims.iter().product();
            let err = |e: ort::Error| format!("{name}: {e}");
            let value = match ty {
                TensorElementType::Float32 => {
                    let fill: f32 = if name.contains("point") { 512.0 } else { 0.5 };
                    Tensor::from_array((dims, vec![fill; count]))
                        .map_err(err)?
                        .into_dyn()
                }
                TensorElementType::Int64 => Tensor::from_array((dims, vec![1i64; count]))
                    .map_err(err)?
                    .into_dyn(),
                TensorElementType::Uint8 => Tensor::from_array((dims, vec![128u8; count]))
                    .map_err(err)?
                    .into_dyn(),
                TensorElementType::Bool => Tensor::from_array((dims, vec![true; count]))
                    .map_err(err)?
                    .into_dyn(),
                other => return Err(format!("{name}: input type {other:?} not handled")),
            };
            Ok((name, value))
        })
        .collect()
}

/// GPU memory in use (MB), when nvidia-smi answers.
fn gpu_memory_mb() -> Option<u64> {
    let out = Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"])
        .output()
        .ok()?;
    String::from_utf8(out.stdout)
        .ok()?
        .lines()
        .next()?
        .trim()
        .parse()
        .ok()
}

fn ms(d: Duration) -> String {
    format!("{:.1}", d.as_secs_f64() * 1000.0)
}

#[allow(clippy::too_many_arguments)]
fn bench(
    dir: &Path,
    model: &Model,
    stage: &Stage,
    ep: Ep,
    runs: usize,
    level: GraphOptimizationLevel,
) -> Result<String, String> {
    let path = dir.join(model.repo).join(&stage.file);
    if !path.exists() {
        return Err(format!("missing {} (run fetch_models.py)", path.display()));
    }
    let before = gpu_memory_mb();
    let start = Instant::now();
    let mut session = session(&path, ep, level).map_err(|e| e.to_string())?;
    let build = start.elapsed();
    let values = inputs(&session, stage)?;
    let feed = || {
        values
            .iter()
            .map(|(n, v)| (n.as_str(), v))
            .collect::<Vec<_>>()
    };
    let start = Instant::now();
    session.run(feed()).map_err(|e| e.to_string())?;
    let first = start.elapsed();
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        session.run(feed()).map_err(|e| e.to_string())?;
        times.push(start.elapsed());
    }
    times.sort();
    let at = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
    let vram = match (before, gpu_memory_mb()) {
        (Some(a), Some(b)) => format!("{}", b.saturating_sub(a)),
        _ => "—".to_owned(),
    };
    Ok(format!(
        "| {} | {} | {} | {} | {} | {} | {} | {} |",
        model.name,
        stage.name,
        ep.name(),
        ms(build),
        ms(first),
        ms(at(0.5)),
        ms(at(0.95)),
        vram
    ))
}

fn default_dir() -> PathBuf {
    match std::env::var_os("LOCALAPPDATA") {
        Some(base) => PathBuf::from(base),
        None => std::env::home_dir().unwrap_or_default().join(".cache"),
    }
    .join("slopshop")
    .join("bench-models")
}

fn main() {
    let mut dir = default_dir();
    let mut eps = Vec::new();
    let mut only = None;
    let mut runs = 20;
    let mut level = GraphOptimizationLevel::Level3;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().unwrap_or_default();
        match arg.as_str() {
            "--models" => dir = PathBuf::from(value()),
            "--ep" => match Ep::parse(&value()) {
                Some(ep) => eps.push(ep),
                None => return eprintln!("unknown execution provider"),
            },
            "--only" => only = Some(value()),
            "--runs" => runs = value().parse().unwrap_or(20).max(1),
            // Graph optimizations: all (default), basic, or none.
            "--opt" => {
                level = match value().as_str() {
                    "basic" => GraphOptimizationLevel::Level1,
                    "none" => GraphOptimizationLevel::Disable,
                    _ => GraphOptimizationLevel::Level3,
                }
            }
            other => return eprintln!("unknown argument {other}"),
        }
    }
    if eps.is_empty() {
        #[cfg(feature = "directml")]
        eps.push(Ep::DirectMl);
        #[cfg(feature = "webgpu")]
        eps.push(Ep::WebGpu);
        eps.push(Ep::Cpu);
    }
    println!(
        "| Model | Stage | EP | Session (ms) | First run (ms) | p50 (ms) | p95 (ms) | VRAM (MB) |"
    );
    println!("|---|---|---|---|---|---|---|---|");
    for model in models() {
        if only.as_deref().is_some_and(|o| o != model.name) {
            continue;
        }
        for &ep in &eps {
            // The CPU is slow: fewer runs there.
            let runs = if ep == Ep::Cpu { runs.min(5) } else { runs };
            for stage in &model.stages {
                match bench(&dir, &model, stage, ep, runs, level) {
                    Ok(row) => println!("{row}"),
                    Err(e) => println!(
                        "| {} | {} | {} | failed: {e} |||||",
                        model.name,
                        stage.name,
                        ep.name()
                    ),
                }
            }
        }
    }
}
