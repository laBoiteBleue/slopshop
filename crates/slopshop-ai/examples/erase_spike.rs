//! Spike of the Erase tool (FLUX.2 [klein] 4B turbo + the `erase_v1` LoRA) on our own ONNX
//! graphs: runs the integration's test vectors through ONNX Runtime with DirectML and reports the
//! protocol's conformity criteria (final latent, PSNR inside the selection, pixels outside it)
//! with timings and VRAM.
//!
//! ```sh
//! cargo run --release -p slopshop-ai --features helper --example erase_spike -- \
//!     --integration <folder with erase/> --models <folder with black-forest-labs/> \
//!     --runtime <onnxruntime.dll> [--chunks 16] [--weights half|channels|<block>] [--steps 4] [--profile] [--out <folder>] [case…]
//! ```

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ort::session::builder::GraphOptimizationLevel;
use ort::session::{OutputSelector, RunOptions, Session, SessionInputValue};
use ort::value::Tensor;
use slopshop_ai::erase::{self, LatentStats, Region, pil};
use slopshop_ai::flux2::{self, Built, Lora, Storage};
use slopshop_ai::safetensors::SafeTensors;

type Error = Box<dyn std::error::Error>;

struct Options {
    integration: PathBuf,
    models: PathBuf,
    runtime: PathBuf,
    chunks: usize,
    storage: Storage,
    out: Option<PathBuf>,
    cases: Vec<String>,
    level: GraphOptimizationLevel,
    profile: bool,
    steps: usize,
    crop: bool,
    dilate: f64,
}

fn options() -> Result<Options, Error> {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let ai = Path::new(&local).join("dev.slopshop.app").join("ai");
    let mut o = Options {
        integration: PathBuf::from(r"C:\Users\Proprietaire\Downloads\integration"),
        models: ai.join("models"),
        runtime: ai.join("runtime").join("directml").join("onnxruntime.dll"),
        chunks: 16,
        storage: Storage::Half,
        out: None,
        cases: Vec::new(),
        level: GraphOptimizationLevel::Level3,
        profile: false,
        steps: erase::STEPS,
        crop: false,
        dilate: 0.0,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--integration" => o.integration = value()?.into(),
            "--models" => o.models = value()?.into(),
            "--runtime" => o.runtime = value()?.into(),
            "--chunks" => o.chunks = value()?.parse()?,
            "--weights" => {
                o.storage = match value()?.as_str() {
                    "half" => Storage::Half,
                    "channels" => Storage::Channels,
                    block => Storage::Blocks(block.parse()?),
                }
            }
            "--profile" => o.profile = true,
            "--crop" => o.crop = true,
            "--dilate" => o.dilate = value()?.parse()?,
            "--steps" => o.steps = value()?.parse()?,
            "--out" => o.out = Some(value()?.into()),
            "--level" => {
                o.level = match value()?.as_str() {
                    "0" => GraphOptimizationLevel::Disable,
                    "1" => GraphOptimizationLevel::Level1,
                    "2" => GraphOptimizationLevel::Level2,
                    _ => GraphOptimizationLevel::Level3,
                }
            }
            case => o.cases.push(case.to_string()),
        }
    }
    Ok(o)
}

fn session(
    built: Built,
    level: GraphOptimizationLevel,
    profile: Option<&Path>,
) -> Result<Session, Error> {
    let Built { model, weights } = built;
    let mut builder = Session::builder()?;
    if let Some(path) = profile {
        builder = builder.with_profiling(path)?;
    }
    let session = builder
        .with_optimization_level(level)?
        .with_memory_pattern(false)?
        .with_parallel_execution(false)?
        .with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()])?
        .with_external_initializer_file_in_memory(flux2::WEIGHTS, Cow::Owned(weights))?
        .commit_from_memory(&model)?;
    Ok(session)
}

fn vram() -> String {
    std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "?".into())
}

fn load_png(path: &Path) -> Result<(Vec<u8>, usize, usize, usize), Error> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("PNG too large")?];
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Grayscale => 1,
        other => return Err(format!("{}: unexpected {other:?}", path.display()).into()),
    };
    if info.bit_depth != png::BitDepth::Eight {
        return Err(format!("{}: not 8-bit", path.display()).into());
    }
    Ok((buf, info.width as usize, info.height as usize, channels))
}

fn save_png(path: &Path, rgb: &[u8], width: usize, height: usize) -> Result<(), Error> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgb)?;
    Ok(())
}

/// Runs the model for one of its outputs, computing only what that output needs.
fn fetch(
    session: &mut Session,
    inputs: Vec<(&'static str, Tensor<f32>)>,
    output: &str,
) -> Result<Vec<f32>, Error> {
    let options = RunOptions::new()?.with_outputs(OutputSelector::no_default().with(output));
    // ONNX Runtime runs the whole graph: the parts not asked for get tiny inputs.
    let mut inputs = inputs;
    let given: Vec<&str> = inputs.iter().map(|(k, _)| *k).collect();
    let mut dummies: Vec<(&'static str, Tensor<f32>)> = Vec::new();
    if !given.contains(&"pixels") {
        dummies.push((
            "pixels",
            Tensor::from_array(([1, 3, 8, 8], vec![0.0; 192]))?,
        ));
    }
    if !given.contains(&"latent") {
        dummies.push((
            "latent",
            Tensor::from_array(([1, 32, 1, 1], vec![0.0; 32]))?,
        ));
    }
    if !given.contains(&"img") {
        let text = flux2::TEXT_TOKENS as usize;
        dummies.push(("img", Tensor::from_array(([8, 128], vec![0.0; 8 * 128]))?));
        dummies.push((
            "txt",
            Tensor::from_array(([text, 7680], vec![0.0; text * 7680]))?,
        ));
        dummies.push(("tproj", Tensor::from_array(([1, 256], vec![0.0; 256]))?));
        dummies.push((
            "rope_cos",
            Tensor::from_array(([text + 8, 128], vec![1.0; (text + 8) * 128]))?,
        ));
        dummies.push((
            "rope_sin",
            Tensor::from_array(([text + 8, 128], vec![0.0; (text + 8) * 128]))?,
        ));
        dummies.push((
            "key_bias",
            Tensor::from_array(([text + 8, 8], vec![0.0; (text + 8) * 8]))?,
        ));
    }
    inputs.extend(dummies);
    let inputs: Vec<(Cow<'static, str>, SessionInputValue)> = inputs
        .into_iter()
        .map(|(k, v)| (Cow::Borrowed(k), v.into()))
        .collect();
    let outputs = session.run_with_options(inputs, &options)?;
    let (_, values) = outputs[output].try_extract_tensor::<f32>()?;
    Ok(values.to_vec())
}

fn main() -> Result<(), Error> {
    let o = options()?;
    ort::init_from(&o.runtime)?.commit();
    let erase_dir = o.integration.join("erase");
    let klein = o.models.join("black-forest-labs").join("FLUX.2-klein-4B");

    // The VAE's batch-norm statistics of the patched latent, then the whole model in one graph.
    let mut vae = SafeTensors::open(
        &klein
            .join("vae")
            .join("diffusion_pytorch_model.safetensors"),
    )?;
    let stats = LatentStats::new(
        vae.f32s("bn.running_mean")?,
        &vae.f32s("bn.running_var")?,
        1e-4,
    );
    let t = Instant::now();
    let mut weights = SafeTensors::open(
        &klein
            .join("transformer")
            .join("diffusion_pytorch_model.safetensors"),
    )?;
    let mut lora = Lora::open(
        &erase_dir
            .join("lora")
            .join("erase_v1_diffusers.safetensors"),
    )?;
    let targeted = lora.modules().len();
    let (built, merged) =
        flux2::pipeline(&mut weights, Some(&mut lora), &mut vae, o.chunks, o.storage)?;
    if merged != targeted {
        return Err(format!("LoRA: {merged} of {targeted} modules merged").into());
    }
    println!(
        "graph: {:.1} MB, weights {:.2} GB, {merged} LoRA layers merged, {:.1} s",
        built.model.len() as f64 / 1e6,
        built.weights.len() as f64 / 1e9,
        t.elapsed().as_secs_f64()
    );
    if let Some(out) = &o.out {
        std::fs::create_dir_all(out)?;
        std::fs::write(out.join("flux2-klein.onnx"), &built.model)?;
    }
    let t = Instant::now();
    let profile = o
        .profile
        .then(|| o.out.clone().unwrap_or_default().join("pipeline"));
    let mut model = session(built, o.level, profile.as_deref())?;
    println!(
        "session: {:.1} s, VRAM {}",
        t.elapsed().as_secs_f64(),
        vram()
    );

    let mut embeds = SafeTensors::open(&erase_dir.join("prompt_embeds.safetensors"))?;
    let txt = embeds.f32s("prompt_embeds")?;
    let text = flux2::TEXT_TOKENS as usize;

    let mut cases: Vec<String> = std::fs::read_dir(erase_dir.join("test_vectors"))?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|name| o.cases.is_empty() || o.cases.iter().any(|c| name.contains(c)))
        .collect();
    cases.sort();
    let mut all_pass = true;
    for case in cases {
        let dir = erase_dir.join("test_vectors").join(&case);
        println!("\n== {case}");
        let (full_photo, full_width, full_height, _) = load_png(&dir.join("input.png"))?;
        let (full_selection, ..) = load_png(&dir.join("selection.png"))?;
        let full_selection: Vec<u8> = full_selection
            .iter()
            .map(|&s| if s > 127 { 255 } else { 0 })
            .collect();
        // `--dilate f`: the selection grown by a disk of f × the image's larger side.
        let full_selection = if o.dilate > 0.0 {
            let radius = (o.dilate * full_width.max(full_height) as f64).round() as i64;
            dilate(&full_selection, full_width, full_height, radius)
        } else {
            full_selection
        };
        // With `--crop`, the model works on the region around the selection only.
        let region = if o.crop {
            let bbox = bounding_box(&full_selection, full_width, full_height);
            erase::work_region(full_width as u32, full_height as u32, bbox)
        } else {
            Region {
                x: 0,
                y: 0,
                width: full_width as u32,
                height: full_height as u32,
            }
        };
        let photo = crop(&full_photo, 3, full_width, region);
        let selection = crop(&full_selection, 1, full_width, region);
        let (width, height) = (region.width as usize, region.height as usize);
        let (w, h) = erase::working_size(width as u32, height as u32);
        let (w, h) = (w as usize, h as usize);
        let (lh, lw) = (h / 16, w / 16);
        let n = lh * lw;
        let photo_s = pil::resize_lanczos(&photo, 3, (width, height), (w, h));
        let selection_s = pil::resize_nearest(&selection, 1, (width, height), (w, h));
        let mask = erase::mask_rgb(&selection_s);

        let total = Instant::now();
        let t = Instant::now();
        let mut refs = Vec::with_capacity(2 * n * 128);
        for image in [&photo_s, &mask] {
            let pixels = Tensor::from_array(([1, 3, h, w], erase::vae_input(image, w, h)))?;
            let latent = fetch(&mut model, vec![("pixels", pixels)], "mean")?;
            refs.extend(erase::pack(&latent, lh, lw, &stats));
        }
        // The reference images enter the transformer in bfloat16.
        refs.iter_mut()
            .for_each(|v| *v = slopshop_ai::numeric::round_bf16(*v));
        let encode = t.elapsed().as_secs_f64();

        let mut vectors = SafeTensors::open(&dir.join("tensors.safetensors"))?;
        let noise = if o.crop {
            erase::noise(0, 128 * n)
        } else {
            vectors.f32s("noise")?
        };
        let mut x = erase::pack_noise(&noise, lh, lw);
        let expected = vectors.f32s("final_latent")?;
        let sigmas = erase::sigmas(n);
        let (mut cos, mut sin) = erase::rope_tables(text, lh, lw);
        // The sequence padded for DirectML: identity rotations, masked out of the attention.
        let extra = flux2::padding(text, 3 * n);
        let tokens = text + 3 * n + extra;
        let key_bias = flux2::key_bias(tokens, extra);
        cos.resize(tokens * 128, 1.0);
        sin.resize(tokens * 128, 0.0);
        let mut steps = Vec::new();
        for i in 0..o.steps {
            let t = Instant::now();
            let mut img = x.clone();
            img.extend_from_slice(&refs);
            img.resize((3 * n + extra) * 128, 0.0);
            let inputs = vec![
                ("img", Tensor::from_array(([3 * n + extra, 128], img))?),
                ("txt", Tensor::from_array(([text, 7680], txt.clone()))?),
                (
                    "tproj",
                    Tensor::from_array(([1, 256], erase::timestep_projection(sigmas[i]).to_vec()))?,
                ),
                (
                    "rope_cos",
                    Tensor::from_array(([tokens, 128], cos.clone()))?,
                ),
                (
                    "rope_sin",
                    Tensor::from_array(([tokens, 128], sin.clone()))?,
                ),
                (
                    "key_bias",
                    Tensor::from_array(([tokens, 8], key_bias.clone()))?,
                ),
            ];
            let v = fetch(&mut model, inputs, "velocity")?;
            let v = &v[..n * 128];
            let bad = v.iter().filter(|v| !v.is_finite()).count();
            if bad > 0 {
                println!("  step {i}: {bad} non-finite values in the velocity");
            }
            erase::euler_step(&mut x, v, sigmas[i], sigmas[i + 1]);
            steps.push(t.elapsed().as_secs_f64());
        }
        let vram_after = vram();
        let num: f64 = x
            .iter()
            .zip(&expected)
            .map(|(a, b)| f64::from(a - b).powi(2))
            .sum();
        let den: f64 = expected.iter().map(|b| f64::from(*b).powi(2)).sum();
        let latent_error = if o.crop { 0.0 } else { (num / den).sqrt() };

        let t = Instant::now();
        let latent =
            Tensor::from_array(([1, 32, 2 * lh, 2 * lw], erase::unpack(&x, lh, lw, &stats)))?;
        let image = fetch(&mut model, vec![("latent", latent)], "image")?;
        let decode = t.elapsed().as_secs_f64();
        let total = total.elapsed().as_secs_f64();
        let out_s = erase::vae_output(&image, w, h);
        let out = pil::resize_lanczos(&out_s, 3, (w, h), (width, height));
        let result = erase::composite(&photo, &selection, &out);
        let (result, photo, selection, width) = (
            paste(&full_photo, full_width, &result, region),
            full_photo,
            full_selection,
            full_width,
        );
        let height = full_height;
        if o.crop {
            println!("  work region {region:?}");
        }

        let (expected_png, ..) = load_png(&dir.join("expected_output.png"))?;
        let (mut se, mut count) = (0.0f64, 0usize);
        let mut outside_changed = 0usize;
        for (i, &s) in selection.iter().enumerate() {
            let (a, b) = (&result[3 * i..3 * i + 3], &expected_png[3 * i..3 * i + 3]);
            if s > 127 {
                se += a
                    .iter()
                    .zip(b)
                    .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                    .sum::<f64>();
                count += 3;
            } else if a != &photo[3 * i..3 * i + 3] {
                outside_changed += 1;
            }
        }
        let psnr = 10.0 * (255.0f64.powi(2) / (se / count as f64)).log10();
        let pass = latent_error < 0.03 && psnr > 40.0 && outside_changed == 0;
        all_pass &= pass;
        println!(
            "  {width}×{height} → {w}×{h}, {n} tokens, sigmas {:?}",
            &sigmas[..4]
        );
        println!(
            "  VAE encode {encode:.2} s, steps {steps:.2?} s, decode {decode:.2} s, total {total:.1} s, VRAM {vram_after}"
        );
        println!(
            "  final latent error {:.2} % (< 3 %), PSNR in the selection {psnr:.1} dB (> 40), outside changed {outside_changed} px (0): {}",
            100.0 * latent_error,
            if pass { "PASS" } else { "FAIL" }
        );
        if let Some(dir) = &o.out {
            save_png(&dir.join(format!("{case}.png")), &result, width, height)?;
        }
    }
    if o.profile {
        println!("profile: {}", model.end_profiling()?);
    }
    println!(
        "\n{}",
        if all_pass {
            "all cases pass"
        } else {
            "some cases FAIL"
        }
    );
    Ok(())
}

/// The bounding box of a mask's set pixels (the whole image when none is set).
fn bounding_box(mask: &[u8], width: usize, height: usize) -> Region {
    let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0, 0);
    for (i, &m) in mask.iter().enumerate() {
        if m > 127 {
            let (x, y) = (i % width, i / width);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    }
    if x1 == 0 {
        return Region {
            x: 0,
            y: 0,
            width: width as u32,
            height: height as u32,
        };
    }
    Region {
        x: x0 as u32,
        y: y0 as u32,
        width: (x1 - x0) as u32,
        height: (y1 - y0) as u32,
    }
}

fn crop(pixels: &[u8], channels: usize, width: usize, r: Region) -> Vec<u8> {
    let (x, w) = (r.x as usize * channels, r.width as usize * channels);
    (r.y as usize..(r.y + r.height) as usize)
        .flat_map(|y| &pixels[y * width * channels + x..][..w])
        .copied()
        .collect()
}

fn paste(full: &[u8], width: usize, part: &[u8], r: Region) -> Vec<u8> {
    let mut out = full.to_vec();
    let (x, w) = (r.x as usize * 3, r.width as usize * 3);
    for (row, y) in (r.y as usize..(r.y + r.height) as usize).enumerate() {
        out[y * width * 3 + x..][..w].copy_from_slice(&part[row * w..][..w]);
    }
    out
}

/// The mask grown by a disk of `radius` pixels (stamped on its boundary pixels).
fn dilate(mask: &[u8], width: usize, height: usize, radius: i64) -> Vec<u8> {
    let mut out = mask.to_vec();
    let set = |x: i64, y: i64| {
        x >= 0
            && y >= 0
            && (x as usize) < width
            && (y as usize) < height
            && mask[y as usize * width + x as usize] > 127
    };
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            if !set(x, y) || (set(x - 1, y) && set(x + 1, y) && set(x, y - 1) && set(x, y + 1)) {
                continue;
            }
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let (px, py) = (x + dx, y + dy);
                    if dx * dx + dy * dy <= radius * radius
                        && px >= 0
                        && py >= 0
                        && (px as usize) < width
                        && (py as usize) < height
                    {
                        out[py as usize * width + px as usize] = 255;
                    }
                }
            }
        }
    }
    out
}
