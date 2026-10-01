//! The helper run as the importer runs it, on a synthetic DNG whose colors are known.

use std::path::Path;
use std::process::Command;

fn run(file: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_slopshop-raw"))
        .arg(file)
        .output()
        .expect("the helper runs")
}

#[test]
fn a_dng_develops_to_its_colors_in_linear_rec2020() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/tiny.dng");
    let output = run(&fixture);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = output.stdout;
    assert_eq!(&out[..8], b"SLOPRAW1");
    let u32_at = |i: usize| u32::from_le_bytes(out[i..i + 4].try_into().unwrap());
    let u16_at = |i: usize| u16::from_le_bytes(out[i..i + 2].try_into().unwrap());
    let (width, height) = (u32_at(8), u32_at(12));
    assert_eq!((u16_at(16), u16_at(18), u32_at(20)), (1, 3, 0));
    let samples: Vec<f32> = out[24..]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    assert_eq!(samples.len(), (width * height * 3) as usize);
    let pixel = |x: u32, y: u32| {
        let i = ((y * width + x) * 3) as usize;
        [samples[i], samples[i + 1], samples[i + 2]]
    };
    // Away from the edges and from the boundary between the halves, where demosaicing mixes.
    let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 2e-3);
    let (y, quarter) = (height / 2, width / 4);
    assert!(
        close(pixel(quarter, y), [0.5, 0.25, 0.125]),
        "{:?}",
        pixel(quarter, y)
    );
    assert!(
        close(pixel(width - quarter, y), [0.1, 0.4, 0.2]),
        "{:?}",
        pixel(width - quarter, y)
    );
}

#[test]
fn files_it_cannot_read_fail_with_a_message() {
    let path = std::env::temp_dir().join(format!("slopshop-raw-{}.cr2", std::process::id()));
    std::fs::write(&path, b"not a raw file").unwrap();
    let output = run(&path);
    std::fs::remove_file(&path).ok();
    assert!(!output.status.success());
    assert!(!output.stderr.is_empty());
    assert!(output.stdout.is_empty());
}
