//! The helper end to end: started as a process, SAM 2.1 selects a shape on a synthetic image.
//! Needs ONNX Runtime and the models, so it runs only when they are given:
//! `SLOPSHOP_AI_RUNTIME` (the ONNX Runtime library), `SLOPSHOP_AI_MODELS` (the models folder,
//! as `experiments/ai-bench/fetch_models.py` lays it out), and optionally `SLOPSHOP_AI_PROVIDER`
//! and `SLOPSHOP_AI_LIBRARY_PATHS` (the runtime's libraries, `;`- or `:`-separated). Skipped
//! otherwise (CI has neither).

use std::path::{Path, PathBuf};

use slopshop_ai::{Client, EraseModel, Launch, MASK_SIDE, Point, Request, Stage};

#[test]
fn sam_selects_the_clicked_shape() {
    let (Some(runtime), Some(models)) = (
        std::env::var_os("SLOPSHOP_AI_RUNTIME"),
        std::env::var_os("SLOPSHOP_AI_MODELS"),
    ) else {
        eprintln!("SLOPSHOP_AI_RUNTIME / SLOPSHOP_AI_MODELS not set: skipped");
        return;
    };
    let provider = std::env::var("SLOPSHOP_AI_PROVIDER").unwrap_or_else(|_| "auto".into());
    let library_paths: Vec<PathBuf> = std::env::var_os("SLOPSHOP_AI_LIBRARY_PATHS")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    let library_paths: Vec<&Path> = library_paths.iter().map(PathBuf::as_path).collect();
    let mut client = Client::start(&Launch {
        executable: Path::new(env!("CARGO_BIN_EXE_slopshop-ai")),
        runtime: Path::new(&runtime),
        models: Path::new(&models),
        provider: &provider,
        library_paths: &library_paths,
    })
    .expect("the helper starts");

    // A light disc on a dark background, 640×480.
    let (w, h) = (640usize, 480usize);
    let mut rgb = vec![30u8; w * h * 3];
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 - 320.0, y as f32 - 240.0);
            if dx * dx + dy * dy < 120.0 * 120.0 {
                rgb[(y * w + x) * 3..(y * w + x) * 3 + 3].copy_from_slice(&[230, 200, 60]);
            }
        }
    }
    let start = std::time::Instant::now();
    client
        .sam_encode(1, w as u32, h as u32, rgb.clone())
        .expect("SAM encodes");
    eprintln!("first encode (with loading): {:?}", start.elapsed());
    let start = std::time::Instant::now();
    let rgb_again = rgb.clone();
    client
        .sam_encode(1, w as u32, h as u32, rgb)
        .expect("SAM encodes");
    eprintln!("encode: {:?}", start.elapsed());
    eprintln!("provider: {}", client.provider);
    let click = Point {
        x: 320.0,
        y: 240.0,
        positive: true,
    };
    let start = std::time::Instant::now();
    let (logits, score) = client
        .sam_decode(1, vec![click], None)
        .expect("SAM decodes");
    eprintln!("decode: {:?}", start.elapsed());
    assert_eq!(logits.len(), MASK_SIDE * MASK_SIDE);
    let at = |x: f32, y: f32| {
        let (mx, my) = (
            (x / w as f32 * MASK_SIDE as f32) as usize,
            (y / h as f32 * MASK_SIDE as f32) as usize,
        );
        logits[my * MASK_SIDE + mx]
    };
    assert!(at(320.0, 240.0) > 0.0, "the disc's center is inside");
    assert!(at(20.0, 20.0) < 0.0, "a corner is outside");
    assert!(score > 0.5, "score {score}");
    // Warm: the first run of a model tunes its kernels.
    let start = std::time::Instant::now();
    let near = Point {
        x: 300.0,
        y: 250.0,
        positive: true,
    };
    client
        .sam_decode(1, vec![click, near], None)
        .expect("SAM decodes");
    eprintln!("decode, warm: {:?}", start.elapsed());
    // ViTMatte on the disc's edge: a trimap undecided on a ring around it.
    let vitmatte = [
        "Xenova/vitmatte-small-composition-1k/onnx/model.onnx",
        "Xenova/vitmatte-base-composition-1k/onnx/model.onnx",
    ];
    if vitmatte
        .iter()
        .any(|path| Path::new(&models).join(path).is_file())
    {
        let trimap: Vec<u8> = (0..w * h)
            .map(|i| {
                let (dx, dy) = ((i % w) as f32 - 320.0, (i / w) as f32 - 240.0);
                let r = (dx * dx + dy * dy).sqrt();
                if r < 100.0 {
                    255
                } else if r > 140.0 {
                    0
                } else {
                    128
                }
            })
            .collect();
        let start = std::time::Instant::now();
        let alpha = client
            .matte(w as u32, h as u32, rgb_again.clone(), trimap)
            .expect("ViTMatte mattes");
        eprintln!("matte (with loading): {:?}", start.elapsed());
        assert_eq!(alpha.len(), w * h);
        let at = |x: usize, y: usize| f32::from(alpha[y * w + x]) / 65535.0;
        assert!(
            at(320 + 110, 240) > 0.8,
            "inside the disc's edge: {}",
            at(430, 240)
        );
        assert!(at(320 + 130, 240) < 0.2, "outside it: {}", at(450, 240));
    }
    // BiRefNet: the disc is the subject.
    // The model the helper takes for the provider (see `models_for`).
    let birefnet = match provider.as_str() {
        "directml" => "onnx-community/BiRefNet-ONNX/onnx/model_fp16.onnx",
        "coreml" => "onnx-community/BiRefNet-ONNX/onnx/model.onnx",
        _ => "onnx-community/BiRefNet_lite-ONNX/onnx/model.onnx",
    };
    if Path::new(&models).join(birefnet).is_file() {
        let start = std::time::Instant::now();
        let logits = client
            .subject(w as u32, h as u32, rgb_again.clone())
            .expect("BiRefNet finds the subject");
        eprintln!("subject (with loading): {:?}", start.elapsed());
        let side = slopshop_ai::SUBJECT_SIDE;
        assert_eq!(logits.len(), side * side);
        assert!(
            logits[side / 2 * side + side / 2] > 0.0,
            "the disc's center"
        );
        assert!(logits[side * 10 + 10] < 0.0, "a corner");
    }
    // Another key than the encoded image's is refused.
    assert!(client.sam_decode(2, vec![click], None).is_err());
}

/// The Erase tool (ADR 0045) through the helper: a red square on a smooth background is replaced
/// by that background. Also needs the Erase files in the models folder (skipped without them).
#[test]
fn erase_replaces_the_selection_by_its_surroundings() {
    let (Some(runtime), Some(models)) = (
        std::env::var_os("SLOPSHOP_AI_RUNTIME"),
        std::env::var_os("SLOPSHOP_AI_MODELS"),
    ) else {
        eprintln!("SLOPSHOP_AI_RUNTIME / SLOPSHOP_AI_MODELS not set: skipped");
        return;
    };
    let models = PathBuf::from(models);
    if !models
        .join("slopshop/erase-v1/erase_v1_diffusers.safetensors")
        .is_file()
    {
        eprintln!("the Erase files are not installed: skipped");
        return;
    }
    let library_paths: Vec<PathBuf> = std::env::var_os("SLOPSHOP_AI_LIBRARY_PATHS")
        .map(|paths| std::env::split_paths(&paths).collect())
        .unwrap_or_default();
    let library_paths: Vec<&Path> = library_paths.iter().map(PathBuf::as_path).collect();
    let mut client = Client::start(&Launch {
        executable: Path::new(env!("CARGO_BIN_EXE_slopshop-ai")),
        runtime: Path::new(&runtime),
        models: &models,
        provider: "directml",
        library_paths: &library_paths,
    })
    .expect("the helper starts");

    // A sky-blue to grass-green gradient, a red square in the middle, its mask a little larger.
    let (w, h) = (512usize, 512usize);
    let mut rgb = Vec::with_capacity(w * h * 3);
    let mut mask = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let t = y as f32 / h as f32;
            let background = [
                (120.0 - 60.0 * t) as u8,
                (170.0 + 30.0 * t) as u8,
                (230.0 - 170.0 * t) as u8,
            ];
            let square = (192..320).contains(&x) && (192..320).contains(&y);
            rgb.extend_from_slice(if square { &[220, 20, 20] } else { &background });
            mask.push(if (176..336).contains(&x) && (176..336).contains(&y) {
                255
            } else {
                0
            });
        }
    }
    let mut stages = Vec::new();
    let start = std::time::Instant::now();
    let result = client
        .erase(
            &Request::Erase {
                width: w as u32,
                height: h as u32,
                rgb,
                mask,
                seed: 7,
                model: EraseModel::Turbo,
            },
            &mut |stage, done, total| {
                stages.push((stage, done, total));
                true
            },
        )
        .expect("Erase runs");
    eprintln!(
        "erase (with loading): {:?}, {} progress reports",
        start.elapsed(),
        stages.len()
    );
    assert_eq!(result.len(), w * h * 3);
    assert_eq!(stages.first(), Some(&(Stage::Loading, 0, 1000)));
    // Loading reports its way (at most every 1 %, a large tensor at a time), up to its end.
    let loading = stages.iter().filter(|s| s.0 == Stage::Loading).count();
    assert!(loading > 20, "{loading} loading reports");
    assert!(stages.contains(&(Stage::Loading, 1000, 1000)));
    assert!(stages.contains(&(Stage::Denoising, 4, 4)));
    assert_eq!(stages.last(), Some(&(Stage::Decoding, 1, 1)));
    // Inside the square: no red left, the background's colors instead.
    let (mut red, mut blue, mut count) = (0f64, 0f64, 0f64);
    for y in 200..312 {
        for x in 200..312 {
            let p = &result[(y * w + x) * 3..][..3];
            red += f64::from(p[0]);
            blue += f64::from(p[2]);
            count += 1.0;
        }
    }
    let (red, blue) = (red / count, blue / count);
    eprintln!("inside the square: mean red {red:.0}, blue {blue:.0}");
    assert!(red < 150.0 && blue > 80.0, "red {red}, blue {blue}");
}
