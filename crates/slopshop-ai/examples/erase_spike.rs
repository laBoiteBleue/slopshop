//! The Erase tool (FLUX.2 [klein] 4B turbo + the `erase_v1` LoRA, ADR 0045) on the
//! integration's test vectors, through ONNX Runtime with DirectML: the protocol's conformity
//! criteria (PSNR inside the selection, pixels outside it), timings and VRAM.
//!
//! ```sh
//! cargo run --release -p slopshop-ai --features helper --example erase_spike -- \
//!     [--integration <folder with erase/>] [--models <folder>] [--runtime <onnxruntime.dll>] \
//!     [--weights half|channels|<block>] [--crop] [--dilate <fraction>] [--out <folder>] [case…]
//! ```
//!
//! With `--crop` (the work region around the selection) or `--dilate`, the noise is drawn from a
//! seed instead of the vectors': the PSNR then measures another sample.

use std::path::{Path, PathBuf};
use std::time::Instant;

use slopshop_ai::erase::{self, Region, pil};
use slopshop_ai::eraser::{Eraser, Files};
use slopshop_ai::flux2::Storage;
use slopshop_ai::safetensors::SafeTensors;

type Error = Box<dyn std::error::Error>;

struct Options {
    integration: PathBuf,
    models: PathBuf,
    runtime: PathBuf,
    storage: Storage,
    crop: bool,
    dilate: f64,
    /// The colors matched to the photo at the selection's edge (`--match`).
    edges: bool,
    out: Option<PathBuf>,
    cases: Vec<String>,
}

fn options() -> Result<Options, Error> {
    let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
    let ai = Path::new(&local).join("dev.slopshop.app").join("ai");
    let mut o = Options {
        integration: PathBuf::from(r"C:\Users\Proprietaire\Downloads\integration"),
        models: ai.join("models"),
        runtime: ai.join("runtime").join("directml").join("onnxruntime.dll"),
        storage: Storage::Blocks(64),
        crop: false,
        dilate: 0.0,
        edges: false,
        out: None,
        cases: Vec::new(),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--integration" => o.integration = value()?.into(),
            "--models" => o.models = value()?.into(),
            "--runtime" => o.runtime = value()?.into(),
            "--weights" => {
                o.storage = match value()?.as_str() {
                    "half" => Storage::Half,
                    "channels" => Storage::Channels,
                    block => Storage::Blocks(block.parse()?),
                }
            }
            "--crop" => o.crop = true,
            "--dilate" => o.dilate = value()?.parse()?,
            "--match" => o.edges = true,
            "--out" => o.out = Some(value()?.into()),
            case => o.cases.push(case.to_string()),
        }
    }
    Ok(o)
}

fn vram() -> String {
    std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.used", "--format=csv,noheader"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "?".into())
}

fn main() -> Result<(), Error> {
    let o = options()?;
    ort::init_from(&o.runtime)?.commit();
    let erase_dir = o.integration.join("erase");
    let klein = o.models.join("black-forest-labs").join("FLUX.2-klein-4B");
    let t = Instant::now();
    let mut eraser = Eraser::load(
        Files {
            transformer: &klein
                .join("transformer")
                .join("diffusion_pytorch_model.safetensors"),
            vae: &klein
                .join("vae")
                .join("diffusion_pytorch_model.safetensors"),
            lora: &erase_dir
                .join("lora")
                .join("erase_v1_diffusers.safetensors"),
            embedding: &erase_dir.join("prompt_embeds.safetensors"),
        },
        o.storage,
        &mut |_, _, _| {},
    )?;
    println!(
        "loaded in {:.1} s ({:?}), VRAM {}",
        t.elapsed().as_secs_f64(),
        o.storage,
        vram()
    );
    if let Some(out) = &o.out {
        std::fs::create_dir_all(out)?;
    }

    let mut cases: Vec<String> = std::fs::read_dir(erase_dir.join("test_vectors"))?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .filter(|name| o.cases.is_empty() || o.cases.iter().any(|c| name.contains(c)))
        .collect();
    cases.sort();
    let mut all_pass = true;
    for case in cases {
        let dir = erase_dir.join("test_vectors").join(&case);
        println!("\n== {case}");
        let (photo, width, height) = load_png(&dir.join("input.png"))?;
        let (selection, ..) = load_png(&dir.join("selection.png"))?;
        let mut selection: Vec<u8> = selection
            .iter()
            .map(|&s| if s > 127 { 255 } else { 0 })
            .collect();
        if o.dilate > 0.0 {
            let radius = (o.dilate * width.max(height) as f64).round() as i64;
            selection = dilate(&selection, width, height, radius);
        }
        let region = if o.crop {
            let bbox = bounding_box(&selection, width, height);
            erase::work_region(width as u32, height as u32, bbox)
        } else {
            Region {
                x: 0,
                y: 0,
                width: width as u32,
                height: height as u32,
            }
        };
        let (rw, rh) = (region.width as usize, region.height as usize);
        let region_photo = crop(&photo, 3, width, region);
        let region_selection = crop(&selection, 1, width, region);

        let (w, h) = erase::working_size(region.width, region.height);
        let (w, h) = (w as usize, h as usize);
        let n = (h / 16) * (w / 16);
        let photo_s = pil::resize_lanczos(&region_photo, 3, (rw, rh), (w, h));
        let selection_s = pil::resize_nearest(&region_selection, 1, (rw, rh), (w, h));
        let given = !o.crop && o.dilate == 0.0;
        let noise = if given {
            SafeTensors::open(&dir.join("tensors.safetensors"))?.f32s("noise")?
        } else {
            erase::noise(0, 128 * n)
        };

        let t = Instant::now();
        let mut stages = Vec::new();
        let mut report = |stage, done, total| {
            let at = t.elapsed().as_secs_f64();
            stages.push(format!("{stage:?} {done}/{total} at {at:.2} s"));
        };
        let mut out_s = eraser.run((w, h), &photo_s, &selection_s, &noise, &mut report)?;
        if o.edges {
            erase::edges::match_edges(&mut out_s, &photo_s, &selection_s, w, h);
        }
        let total = t.elapsed().as_secs_f64();
        let out = pil::resize_lanczos(&out_s, 3, (w, h), (rw, rh));
        // The model's output against the photo where it should reproduce it: everywhere
        // outside the selection, and in a ring of 24 px around it (the seam).
        {
            let near = dilate(&selection_s, w, h, 24);
            let (mut all, mut ring, mut na, mut nr) = ([0f64; 3], [0f64; 3], 0f64, 0f64);
            for i in 0..w * h {
                if selection_s[i] > 127 {
                    continue;
                }
                for c in 0..3 {
                    let d = f64::from(out_s[3 * i + c]) - f64::from(photo_s[3 * i + c]);
                    all[c] += d;
                    if near[i] > 127 {
                        ring[c] += d;
                    }
                }
                na += 1.0;
                if near[i] > 127 {
                    nr += 1.0;
                }
            }
            let mean = |s: [f64; 3], n: f64| s.map(|v| format!("{:+.1}", v / n.max(1.0)));
            println!(
                "  output − input (RGB): outside {:?}, ring {:?}",
                mean(all, na),
                mean(ring, nr)
            );
        }
        let fill = erase::composite(&region_photo, &region_selection, &out);
        let result = paste(&photo, width, &fill, region);

        // The tool returns the image, not its final latent: the latent was checked by the
        // earlier spike (ADR 0045); here the image is.
        let (expected, ..) = load_png(&dir.join("expected_output.png"))?;
        let (mut se, mut count, mut outside_changed) = (0.0f64, 0usize, 0usize);
        for (i, &s) in selection.iter().enumerate() {
            let (a, b) = (&result[3 * i..3 * i + 3], &expected[3 * i..3 * i + 3]);
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
        let pass = (!given || psnr > 40.0) && outside_changed == 0;
        all_pass &= pass;
        println!("  {width}×{height}, region {region:?} → {w}×{h}, {n} tokens");
        println!("  {total:.1} s, VRAM {}; {}", vram(), stages.join(", "));
        println!(
            "  PSNR in the selection {psnr:.1} dB{}, outside changed {outside_changed} px: {}",
            if given {
                " (> 40)"
            } else {
                " (another sample)"
            },
            if pass { "PASS" } else { "FAIL" }
        );
        if let Some(out) = &o.out {
            save_png(&out.join(format!("{case}.png")), &result, width, height)?;
        }
    }
    let verdict = if all_pass {
        "all cases pass"
    } else {
        "some cases FAIL"
    };
    println!("\n{verdict}");
    Ok(())
}

fn load_png(path: &Path) -> Result<(Vec<u8>, usize, usize), Error> {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path)?));
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("PNG too large")?];
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());
    let rgb_or_gray = matches!(
        info.color_type,
        png::ColorType::Rgb | png::ColorType::Grayscale
    );
    if info.bit_depth != png::BitDepth::Eight || !rgb_or_gray {
        return Err(format!("{}: not 8-bit RGB or gray", path.display()).into());
    }
    Ok((buf, info.width as usize, info.height as usize))
}

fn save_png(path: &Path, rgb: &[u8], width: usize, height: usize) -> Result<(), Error> {
    let file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(rgb)?;
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
        (x0, y0, x1, y1) = (0, 0, width, height);
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
    let (w, h) = (width as i64, height as i64);
    let inside = |x: i64, y: i64| (0..w).contains(&x) && (0..h).contains(&y);
    let set = |x: i64, y: i64| inside(x, y) && mask[(y * w + x) as usize] > 127;
    let mut out = mask.to_vec();
    for y in 0..h {
        for x in 0..w {
            let edge = !(set(x - 1, y) && set(x + 1, y) && set(x, y - 1) && set(x, y + 1));
            if !set(x, y) || !edge {
                continue;
            }
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    if dx * dx + dy * dy <= radius * radius && inside(x + dx, y + dy) {
                        out[((y + dy) * w + x + dx) as usize] = 255;
                    }
                }
            }
        }
    }
    out
}
