//! `slopshop save` and `slopshop inspect`: `.slop` document files (ADR 0009) without the app.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use slopshop_core::color::ColorSpace;
use slopshop_core::{BlendMode, Document, Edit, Layer, LayerContent, Size};
use slopshop_io::slop::SlopFile;

/// `slopshop save <IMAGE>... --out <FILE.slop> [--bench]`: one layer per image, bottom first.
pub fn save(args: &[String]) -> Result<(), String> {
    let mut inputs = Vec::new();
    let mut out = None;
    let mut bench = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--out" => out = Some(PathBuf::from(it.next().ok_or("missing value for --out")?)),
            "--bench" => bench = true,
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option `{other}`"));
            }
            path => inputs.push(PathBuf::from(path)),
        }
    }
    let out = out.ok_or("missing --out <FILE.slop>")?;
    if inputs.is_empty() {
        return Err("expected at least one image to save".to_owned());
    }

    let started = Instant::now();
    let mut images = Vec::new();
    for input in &inputs {
        let imported = slopshop_io::open_image(input)
            .map_err(|e| format!("cannot open {}: {e}", input.display()))?;
        for warning in &imported.warnings {
            println!("import warning ({}): {}", input.display(), warning.id());
        }
        images.push((input, imported.image));
    }
    let opened = started.elapsed();
    let size = images.iter().fold(Size::new(0, 0), |size, (_, image)| {
        Size::new(
            size.width.max(image.size().width),
            size.height.max(image.size().height),
        )
    });
    let mut document = Document::new(size);
    for (input, image) in images {
        let id = document.allocate_layer_id();
        let index = document.layers().len();
        let layer = Layer {
            transform: slopshop_core::Affine::IDENTITY,
            clipped: false,
            id,
            name: layer_name(input),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            content: LayerContent::Raster {
                image: Arc::new(image),
            },
        };
        Edit::InsertLayer {
            parent: None,
            index,
            layer,
        }
        .apply(&mut document)
        .map_err(|e| e.to_string())?;
    }

    let started = Instant::now();
    let file = SlopFile::create(&out, &document)
        .map_err(|e| format!("cannot save {}: {e}", out.display()))?;
    let saved = started.elapsed();
    println!(
        "saved {} layer(s), {}x{}, to {} ({})",
        document.layers().len(),
        size.width,
        size.height,
        out.display(),
        bytes(file.len())
    );
    if bench {
        println!("bench: open  {:.3} s", opened.as_secs_f64());
        println!(
            "bench: save  {:.3} s ({:.0} MB/s of pixels)",
            saved.as_secs_f64(),
            pixel_bytes(&document) as f64 / 1e6 / saved.as_secs_f64().max(f64::MIN_POSITIVE)
        );
    }
    Ok(())
}

/// `slopshop inspect <FILE.slop> [--bench]`: the file and the document it holds.
pub fn inspect(args: &[String]) -> Result<(), String> {
    let mut path = None;
    let mut bench = false;
    for arg in args {
        match arg.as_str() {
            "--bench" => bench = true,
            other if other.starts_with('-') && other != "-" => {
                return Err(format!("unknown option `{other}`"));
            }
            other if path.is_none() => path = Some(PathBuf::from(other)),
            _ => return Err("expected one document".to_owned()),
        }
    }
    let path = path.ok_or("expected a .slop document")?;
    let started = Instant::now();
    let (document, file) =
        SlopFile::open(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let opened = started.elapsed();
    println!("file: {}", path.display());
    println!(
        "generation {}, {} ({} unused, reclaimed by the next compaction)",
        file.generation(),
        bytes(file.len()),
        bytes(file.dead_bytes())
    );
    let size = document.size();
    println!(
        "document: {}x{}, {} blending, next layer id {}",
        size.width,
        size.height,
        document.blend_space().id(),
        document.next_layer_id()
    );
    println!("layers, bottom to top (groups before their layers):");
    print_layers(document.layers(), 1);
    if bench {
        println!(
            "bench: open {:.3} s ({:.0} MB/s of pixels)",
            opened.as_secs_f64(),
            pixel_bytes(&document) as f64 / 1e6 / opened.as_secs_f64().max(f64::MIN_POSITIVE)
        );
    }
    Ok(())
}

/// One line per layer, the layers of a group indented under it.
fn print_layers(layers: &[slopshop_core::Layer], depth: usize) {
    for layer in layers {
        let content = match &layer.content {
            LayerContent::Raster { image } => {
                let format = image.format();
                format!(
                    "image {}x{} {:?} {:?} {}",
                    image.size().width,
                    image.size().height,
                    format.layout,
                    format.sample,
                    space_name(&format.color_space)
                )
            }
            LayerContent::Adjustment { adjustment } => {
                format!("adjustment {} {:?}", adjustment.id(), adjustment.params())
            }
            LayerContent::Fill { color } => {
                format!("fill ({}, {}, {}, {})", color.r, color.g, color.b, color.a)
            }
            LayerContent::Group {
                children,
                pass_through,
            } => format!(
                "group of {} ({})",
                children.len(),
                if *pass_through {
                    "pass-through"
                } else {
                    "isolated"
                }
            ),
        };
        let placed = match layer.transform.integer_translation() {
            _ if layer.transform.is_identity() => String::new(),
            Some((x, y)) => format!(", moved by ({x}, {y})"),
            None => format!(", transform {:?}", layer.transform.to_array()),
        };
        println!(
            "{}#{} {:?}: {content}, {} at opacity {}{}{placed}",
            "  ".repeat(depth),
            layer.id.get(),
            layer.name,
            layer.blend_mode.id(),
            layer.opacity,
            if layer.visible { "" } else { ", hidden" }
        );
        if let Some(children) = layer.children() {
            print_layers(children, depth + 1);
        }
    }
}

/// Bytes of the tiles of every image (each once), all levels.
fn pixel_bytes(document: &Document) -> u64 {
    let mut seen = Vec::new();
    let mut total = 0;
    for layer in document.all_layers() {
        if let LayerContent::Raster { image } = &layer.content
            && !seen.contains(&image.id())
        {
            seen.push(image.id());
            total += image.memory_bytes();
        }
    }
    total
}

fn bytes(n: u64) -> String {
    if n >= 1 << 30 {
        format!("{:.2} GiB", n as f64 / (1u64 << 30) as f64)
    } else if n >= 1 << 20 {
        format!("{:.1} MiB", n as f64 / (1u64 << 20) as f64)
    } else {
        format!("{n} bytes")
    }
}

fn space_name(space: &ColorSpace) -> String {
    space
        .id()
        .map_or_else(|| "custom".to_owned(), str::to_owned)
}

fn layer_name(path: &Path) -> String {
    path.file_stem()
        .map_or_else(|| "Image".to_owned(), |s| s.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// A PNG of one repeated pixel.
    fn write_png(path: &Path, width: u32, height: u32, color: png::ColorType, pixel: &[u8]) {
        let file = std::io::BufWriter::new(fs::File::create(path).unwrap());
        let mut encoder = png::Encoder::new(file, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&pixel.repeat((width * height) as usize))
            .unwrap();
    }

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn saves_images_as_layers_and_inspects_them() {
        let dir = std::env::temp_dir().join(format!("slopshop-cli-doc-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.png");
        let b = dir.join("b.png");
        write_png(&a, 300, 20, png::ColorType::Rgb, &[10, 20, 30]);
        write_png(&b, 40, 280, png::ColorType::Rgba, &[1, 2, 3, 128]);
        let out = dir.join("doc.slop");
        save(&strings(&[
            a.to_str().unwrap(),
            b.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]))
        .unwrap();
        let (document, _) = SlopFile::open(&out).unwrap();
        assert_eq!(document.size(), Size::new(300, 280));
        let names: Vec<&str> = document.layers().iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        inspect(&strings(&[out.to_str().unwrap(), "--bench"])).unwrap();

        assert!(save(&strings(&[a.to_str().unwrap()])).is_err(), "no --out");
        assert!(
            save(&strings(&["--out", out.to_str().unwrap()])).is_err(),
            "no image"
        );
        assert!(
            inspect(&strings(&[a.to_str().unwrap()])).is_err(),
            "not a document"
        );
        fs::remove_dir_all(&dir).ok();
    }
}
