//! The helper end to end: started as a process, SAM 2.1 selects a shape on a synthetic image.
//! Needs ONNX Runtime and the models, so it runs only when they are given:
//! `SLOPSHOP_AI_RUNTIME` (the ONNX Runtime library), `SLOPSHOP_AI_MODELS` (the models folder,
//! as `experiments/ai-bench/fetch_models.py` lays it out), and optionally `SLOPSHOP_AI_PROVIDER`
//! and `SLOPSHOP_AI_LIBRARY_PATHS` (NVIDIA's libraries, `;`- or `:`-separated). Skipped
//! otherwise (CI has neither).

use std::path::{Path, PathBuf};

use slopshop_ai::{Client, Launch, MASK_SIDE, Point};

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
    if Path::new(&models)
        .join("Xenova/vitmatte-small-composition-1k/onnx/model.onnx")
        .is_file()
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
    if Path::new(&models)
        .join("onnx-community/BiRefNet-ONNX/onnx/model_fp16.onnx")
        .is_file()
    {
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
    // SAM 3: "circle" names the disc.
    if Path::new(&models)
        .join("wkentaro/sam3-onnx-models-v0.3.0/sam3_decoder.onnx")
        .is_file()
    {
        let start = std::time::Instant::now();
        let (count, probabilities) = client
            .semantic(9, w as u32, h as u32, rgb_again.clone(), "yellow circle")
            .expect("SAM 3 finds it");
        eprintln!(
            "semantic (with loading): {:?}, {count} instance(s)",
            start.elapsed()
        );
        let side = slopshop_ai::SEMANTIC_SIDE;
        assert!(count >= 1);
        assert!(
            probabilities[side / 2 * side + side / 2] > 0.5,
            "the disc's center"
        );
        assert!(probabilities[side * 5 + 5] < 0.5, "a corner");
        let start = std::time::Instant::now();
        client
            .semantic(9, w as u32, h as u32, rgb_again.clone(), "background")
            .expect("SAM 3 again, same image");
        eprintln!("semantic, image already encoded: {:?}", start.elapsed());
    }
    // Another key than the encoded image's is refused.
    assert!(client.sam_decode(2, vec![click], None).is_err());
}
