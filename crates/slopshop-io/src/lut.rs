//! Color lookup table files (Image > Adjustments > Color Lookup): Adobe's and Resolve's `.cube`
//! (3D or 1D) and Autodesk's `.3dl` (3D), read into a [`ColorLut`]; `.cube` written too (PSD
//! files embed the table as a file). The files are untrusted: sizes are bounded before anything
//! is allocated, every number is checked, and damaged files are refused, never guessed at.

use std::path::Path;
use std::sync::Arc;

use slopshop_core::lut::{ColorLut, LutDimension, LutError, MAX_1D_SIZE, MAX_3D_SIZE};

/// The largest file read: a 129³ `.cube` written with long numbers is about 100 MB.
pub const MAX_FILE_BYTES: u64 = 192 << 20;

/// A lookup table file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LutFormat {
    Cube,
    ThreeDl,
}

impl LutFormat {
    /// The format of a file name's extension (`cube`, `3dl`, any case).
    pub fn of_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            "cube" => Some(LutFormat::Cube),
            "3dl" => Some(LutFormat::ThreeDl),
            _ => None,
        }
    }
}

/// Why a lookup table file cannot be read.
#[derive(Debug, Clone, PartialEq)]
pub enum LutFileError {
    Io(String),
    /// Not a `.cube` or `.3dl` file name.
    UnknownFormat,
    TooLarge,
    /// Not text (UTF-8 or ASCII).
    NotText,
    /// What is wrong, at line `line` (1-based; 0: the file as a whole).
    Invalid {
        line: usize,
        reason: String,
    },
    /// Valid, but asking for what SlopShop does not do.
    Unsupported(String),
    Table(LutError),
}

impl std::fmt::Display for LutFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LutFileError::Io(e) => write!(f, "cannot read the lookup table: {e}"),
            LutFileError::UnknownFormat => write!(f, "not a .cube or .3dl lookup table"),
            LutFileError::TooLarge => write!(f, "the lookup table file is too large"),
            LutFileError::NotText => write!(f, "the lookup table file is not text"),
            LutFileError::Invalid { line: 0, reason } => {
                write!(f, "invalid lookup table: {reason}")
            }
            LutFileError::Invalid { line, reason } => {
                write!(f, "invalid lookup table, line {line}: {reason}")
            }
            LutFileError::Unsupported(what) => write!(f, "unsupported lookup table: {what}"),
            LutFileError::Table(e) => write!(f, "invalid lookup table: {e}"),
        }
    }
}

impl std::error::Error for LutFileError {}

impl From<LutError> for LutFileError {
    fn from(e: LutError) -> Self {
        LutFileError::Table(e)
    }
}

fn invalid(line: usize, reason: impl Into<String>) -> LutFileError {
    LutFileError::Invalid {
        line,
        reason: reason.into(),
    }
}

/// The table in the file at `path`, named after the file, its format told by its extension.
pub fn read_lut(path: &Path) -> Result<Arc<ColorLut>, LutFileError> {
    let format = LutFormat::of_path(path).ok_or(LutFileError::UnknownFormat)?;
    let length = std::fs::metadata(path)
        .map_err(|e| LutFileError::Io(e.to_string()))?
        .len();
    if length > MAX_FILE_BYTES {
        return Err(LutFileError::TooLarge);
    }
    let bytes = std::fs::read(path).map_err(|e| LutFileError::Io(e.to_string()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    parse_lut(&bytes, format, name)
}

/// The table in `bytes`, a file of `format`, named `name`.
pub fn parse_lut(
    bytes: &[u8],
    format: LutFormat,
    name: impl Into<String>,
) -> Result<Arc<ColorLut>, LutFileError> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(LutFileError::TooLarge);
    }
    // A UTF-8 byte order mark is allowed (some Windows tools write one).
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).map_err(|_| LutFileError::NotText)?;
    match format {
        LutFormat::Cube => parse_cube(text, name.into()),
        LutFormat::ThreeDl => parse_3dl(text, name.into()),
    }
}

/// The lines of `text` without comments (`#` to the end of the line) and surrounding blanks,
/// numbered from 1, empty ones left out.
fn lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines().enumerate().filter_map(|(i, line)| {
        let line = line.split('#').next().unwrap_or("").trim();
        (!line.is_empty()).then_some((i + 1, line))
    })
}

/// A finite number.
fn number(token: &str, line: usize) -> Result<f32, LutFileError> {
    token
        .parse::<f32>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| invalid(line, format!("{token:?} is not a number")))
}

/// Exactly `N` numbers from the tokens of a line.
fn numbers<const N: usize>(
    mut tokens: std::str::SplitWhitespace<'_>,
    line: usize,
) -> Result<[f32; N], LutFileError> {
    let mut out = [0.0; N];
    for v in &mut out {
        *v = number(
            tokens
                .next()
                .ok_or_else(|| invalid(line, format!("{N} numbers expected")))?,
            line,
        )?;
    }
    if tokens.next().is_some() {
        return Err(invalid(line, format!("{N} numbers expected")));
    }
    Ok(out)
}

/// A size keyword's value, within `max`.
fn size(tokens: std::str::SplitWhitespace<'_>, line: usize, max: u32) -> Result<u32, LutFileError> {
    let [v] = numbers::<1>(tokens, line)?;
    if v.fract() != 0.0 || !(2.0..=max as f32).contains(&v) {
        return Err(LutFileError::Unsupported(format!(
            "a table of size {v} (2 to {max})"
        )));
    }
    Ok(v as u32)
}

/// A `.cube` file (Adobe's Cube LUT Specification 1.0, with Resolve's `LUT_1D_INPUT_RANGE` and
/// `LUT_3D_INPUT_RANGE`): keywords, then the table, red changing fastest.
pub fn parse_cube(text: &str, name: String) -> Result<Arc<ColorLut>, LutFileError> {
    let mut dimension: Option<(LutDimension, u32)> = None;
    let mut domain = ColorLut::UNIT_DOMAIN;
    let mut values: Vec<f32> = Vec::new();
    let mut expected = 0;
    for (line, content) in lines(text) {
        let mut tokens = content.split_whitespace();
        let Some(first) = tokens.clone().next() else {
            continue;
        };
        let starts_number = first
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_digit() || matches!(b, b'-' | b'+' | b'.'));
        if starts_number {
            if dimension.is_none() {
                return Err(invalid(line, "values before LUT_3D_SIZE or LUT_1D_SIZE"));
            }
            if values.len() >= expected {
                return Err(invalid(line, "more values than the table's size"));
            }
            values.extend(numbers::<3>(tokens, line)?);
            continue;
        }
        if !values.is_empty() {
            return Err(invalid(line, format!("{first} after the table's values")));
        }
        tokens.next();
        match first {
            // Its text is free (quoted).
            "TITLE" => {}
            "LUT_3D_SIZE" | "LUT_1D_SIZE" => {
                let (kind, max) = if first == "LUT_3D_SIZE" {
                    (LutDimension::Three, MAX_3D_SIZE)
                } else {
                    (LutDimension::One, MAX_1D_SIZE)
                };
                if dimension.is_some() {
                    // Resolve's shaper (a 1D table before a 3D one) is not read.
                    return Err(LutFileError::Unsupported(
                        "both a 1D and a 3D table, or two sizes".into(),
                    ));
                }
                let n = size(tokens, line, max)?;
                expected = kind.entries(n) * 3;
                // Bounded by the size's limits (129³ × 3 values).
                values.reserve_exact(expected);
                dimension = Some((kind, n));
            }
            "DOMAIN_MIN" => domain[0] = numbers::<3>(tokens, line)?,
            "DOMAIN_MAX" => domain[1] = numbers::<3>(tokens, line)?,
            "LUT_1D_INPUT_RANGE" | "LUT_3D_INPUT_RANGE" => {
                let [lo, hi] = numbers::<2>(tokens, line)?;
                domain = [[lo; 3], [hi; 3]];
            }
            // They change what the values mean (video levels): not silently ignored.
            "LUT_IN_VIDEO_RANGE" | "LUT_OUT_VIDEO_RANGE" => {
                return Err(LutFileError::Unsupported(first.to_string()));
            }
            // Other keywords (other software's) say nothing about the table.
            keyword
                if keyword
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b == b'_' || b.is_ascii_digit()) => {}
            _ => return Err(invalid(line, format!("{first:?} is not a keyword"))),
        }
    }
    let (kind, n) = dimension.ok_or_else(|| invalid(0, "no LUT_3D_SIZE or LUT_1D_SIZE"))?;
    if values.len() != expected {
        return Err(invalid(
            0,
            format!(
                "{} values for a table of {}",
                values.len() / 3,
                expected / 3
            ),
        ));
    }
    Ok(ColorLut::new(name, kind, n, domain, values)?)
}

/// The smallest even bit depth from 8 to 16 holding `max` (the convention of `.3dl` readers:
/// files do not say their bit depths).
fn bit_depth(max: f64) -> Option<u32> {
    (8..=16)
        .step_by(2)
        .find(|bits| max <= f64::from((1u32 << bits) - 1))
}

/// A `.3dl` file (Autodesk Lustre and Flame): an optional `3DMESH` and `Mesh <input bits>
/// <output bits>` header, then the mesh, the input codes of the table's points (the same for
/// each channel), then the table's output codes, blue changing fastest. The codes are integers
/// of a bit depth the file does not say unless with `Mesh`: the input's is the smallest even one
/// holding the mesh's last point (8 to 16), the output's that of the largest value. The mesh must
/// be evenly spaced, its last point possibly one less than its step says (Lustre's `0 64 … 960
/// 1023`: 1024 does not fit in 10 bits); that point is read where the step puts it.
pub fn parse_3dl(text: &str, name: String) -> Result<Arc<ColorLut>, LutFileError> {
    let mut output_bits: Option<u32> = None;
    let mut mesh: Option<Vec<f64>> = None;
    let mut table: Vec<f32> = Vec::new();
    let mut expected = 0;
    for (line, content) in lines(text) {
        let mut tokens = content.split_whitespace();
        let first = tokens.clone().next().unwrap_or("");
        if mesh.is_none() {
            if first.eq_ignore_ascii_case("3DMESH") {
                continue;
            }
            if first.eq_ignore_ascii_case("Mesh") {
                tokens.next();
                let [_, out] = numbers::<2>(tokens, line)?;
                if out.fract() != 0.0 || !(8.0..=16.0).contains(&out) {
                    return Err(LutFileError::Unsupported(format!("{out}-bit output")));
                }
                output_bits = Some(out as u32);
                continue;
            }
            // The mesh: at most MAX_3D_SIZE points.
            let points: Vec<f64> = tokens
                .take(MAX_3D_SIZE as usize + 1)
                .map(|t| number(t, line).map(f64::from))
                .collect::<Result<_, _>>()?;
            let n = points.len();
            if n > MAX_3D_SIZE as usize {
                return Err(LutFileError::Unsupported(format!(
                    "a table of more than {MAX_3D_SIZE} points a side"
                )));
            }
            if n < 2 {
                return Err(invalid(line, "a mesh of at least two points expected"));
            }
            expected = n * n * n * 3;
            table.reserve_exact(expected);
            mesh = Some(points);
            continue;
        }
        if table.len() >= expected {
            return Err(invalid(line, "more values than the mesh's size"));
        }
        let rgb = numbers::<3>(tokens, line)?;
        if rgb.iter().any(|&v| v < 0.0) {
            return Err(invalid(line, "a negative code"));
        }
        table.extend(rgb);
    }
    let mesh = mesh.ok_or_else(|| invalid(0, "no mesh"))?;
    if table.len() != expected {
        return Err(invalid(
            0,
            format!("{} values for a table of {}", table.len() / 3, expected / 3),
        ));
    }
    let n = mesh.len();
    let last = mesh[n - 1];
    let step = mesh[1] - mesh[0];
    let even = mesh[0] == 0.0
        && step > 0.0
        && step.fract() == 0.0
        && mesh[..n - 1]
            .iter()
            .enumerate()
            .all(|(k, &p)| p == k as f64 * step);
    let full = (n - 1) as f64 * step;
    if !even || (last != full && last != full - 1.0) {
        return Err(LutFileError::Unsupported(
            "a mesh not evenly spaced from 0".into(),
        ));
    }
    let input_max = bit_depth(last)
        .map(|bits| f64::from((1u32 << bits) - 1))
        .ok_or_else(|| LutFileError::Unsupported(format!("a mesh up to {last}")))?;
    let largest = table.iter().fold(0.0f32, |m, &v| m.max(v));
    let output_bits = match output_bits {
        Some(bits) => bits,
        None => bit_depth(f64::from(largest))
            .ok_or_else(|| LutFileError::Unsupported(format!("output codes up to {largest}")))?,
    };
    let output_max = ((1u32 << output_bits) - 1) as f32;
    if largest > output_max {
        return Err(invalid(0, format!("codes above {output_max}")));
    }
    // Blue fastest in the file, red fastest in the table.
    let mut values = vec![0.0; expected];
    for r in 0..n {
        for g in 0..n {
            for b in 0..n {
                let from = ((r * n + g) * n + b) * 3;
                let to = ((b * n + g) * n + r) * 3;
                for c in 0..3 {
                    values[to + c] = table[from + c] / output_max;
                }
            }
        }
    }
    let domain_max = (full / input_max) as f32;
    Ok(ColorLut::new(
        name,
        LutDimension::Three,
        n as u32,
        [[0.0; 3], [domain_max; 3]],
        values,
    )?)
}

/// `lut` as a `.cube` file (what PSD files embed): its name as the title, its domain, its
/// values written exactly (shortest round-trip decimals).
pub fn write_cube(lut: &ColorLut) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let title: String = lut
        .name()
        .chars()
        .filter(|&c| c != '"' && c != '\n')
        .collect();
    // Writing to a String does not fail.
    let _ = writeln!(out, "TITLE \"{title}\"");
    let keyword = match lut.dimension() {
        LutDimension::One => "LUT_1D_SIZE",
        LutDimension::Three => "LUT_3D_SIZE",
    };
    let _ = writeln!(out, "{keyword} {}", lut.size());
    let [lo, hi] = lut.domain();
    let _ = writeln!(out, "DOMAIN_MIN {} {} {}", lo[0], lo[1], lo[2]);
    let _ = writeln!(out, "DOMAIN_MAX {} {} {}", hi[0], hi[1], hi[2]);
    for v in lut.values().as_chunks::<3>().0 {
        let _ = writeln!(out, "{} {} {}", v[0], v[1], v[2]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(text: &str) -> Result<Arc<ColorLut>, LutFileError> {
        parse_lut(text.as_bytes(), LutFormat::Cube, "test.cube")
    }

    fn three_dl(text: &str) -> Result<Arc<ColorLut>, LutFileError> {
        parse_lut(text.as_bytes(), LutFormat::ThreeDl, "test.3dl")
    }

    /// A 2-point 3D `.cube` of `f` (red fastest).
    fn cube_of(f: impl Fn([u8; 3]) -> [f32; 3]) -> String {
        let mut text = String::from("LUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    let v = f([r, g, b]);
                    text += &format!("{} {} {}\n", v[0], v[1], v[2]);
                }
            }
        }
        text
    }

    #[test]
    fn cube_files_are_read_with_their_keywords_and_comments() {
        let text = "# Made by hand\nTITLE \"Swap # not a comment start\"\n\n\
                    DOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 2 1\nLUT_3D_SIZE 2\n\
                    0 0 0\n1 0 0 # red\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n";
        let lut = cube(text).unwrap();
        assert_eq!(lut.dimension(), LutDimension::Three);
        assert_eq!(lut.size(), 2);
        assert_eq!(lut.domain(), [[0.0; 3], [1.0, 2.0, 1.0]]);
        assert_eq!(lut.name(), "test.cube");
        assert_eq!(lut.values()[3..6], [1.0, 0.0, 0.0]);
        // Green 1 is the middle of its domain.
        let c = lut.apply([0.25, 1.0, 0.75]);
        assert!((c[0] - 0.25).abs() < 1e-9 && (c[1] - 0.5).abs() < 1e-9);
        // A 1D table, Resolve's input range, CRLF line ends and a byte order mark.
        let lut = parse_lut(
            b"\xEF\xBB\xBFLUT_1D_SIZE 2\r\nLUT_1D_INPUT_RANGE -0.5 1.5\r\n0 1 0\r\n1 0 1\r\n",
            LutFormat::Cube,
            "ramp.cube",
        )
        .unwrap();
        assert_eq!(lut.dimension(), LutDimension::One);
        assert_eq!(lut.domain(), [[-0.5; 3], [1.5; 3]]);
        // Other software's keywords are ignored.
        assert!(
            cube(&format!(
                "LUT_3D_SIZE_PRIVATE 7\n{}",
                cube_of(|c| c.map(f32::from))
            ))
            .is_ok()
        );
    }

    #[test]
    fn a_cube_comes_back_exactly_from_what_is_written() {
        let lut = cube(&cube_of(|[r, g, b]| {
            [
                0.1 + f32::from(r) / 3.0,
                f32::from(g) * 0.7,
                1.0 - f32::from(b),
            ]
        }))
        .unwrap();
        let back = cube(&write_cube(&lut)).unwrap();
        assert_eq!(back.values(), lut.values());
        assert_eq!(back.domain(), lut.domain());
        let ramp = ColorLut::new(
            "a \"quoted\" name",
            LutDimension::One,
            3,
            [[-0.25; 3], [1.25, 2.0, 3.0]],
            vec![0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8],
        )
        .unwrap();
        let back = cube(&write_cube(&ramp)).unwrap();
        assert_eq!(
            (back.dimension(), back.size(), back.domain(), back.values()),
            (ramp.dimension(), ramp.size(), ramp.domain(), ramp.values())
        );
    }

    #[test]
    fn damaged_cube_files_are_refused() {
        let good = cube_of(|c| c.map(f32::from));
        let cases: Vec<String> = vec![
            String::new(),
            "TITLE \"nothing\"\n".into(),
            // A value missing, one too many, two numbers on a line, a word for a number.
            good.lines().take(8).collect::<Vec<_>>().join("\n"),
            format!("{good}1 1 1\n"),
            good.replacen("0 0 0", "0 0", 1),
            good.replacen("0 0 0", "0 0 0 0", 1),
            good.replacen("0 0 0", "0 zero 0", 1),
            good.replacen("0 0 0", "0 NaN 0", 1),
            good.replacen("0 0 0", "0 inf 0", 1),
            good.replacen("0 0 0", "0 1e39 0", 1),
            // Values before the size, keywords after them, unusable sizes and domains.
            format!("0 0 0\n{good}"),
            format!("{good}DOMAIN_MIN 0 0 0\n"),
            good.replace("LUT_3D_SIZE 2", "LUT_3D_SIZE 1"),
            good.replace("LUT_3D_SIZE 2", "LUT_3D_SIZE 2.5"),
            good.replace("LUT_3D_SIZE 2", "LUT_3D_SIZE 4096"),
            good.replace("LUT_3D_SIZE 2", "LUT_3D_SIZE -3"),
            good.replace("LUT_3D_SIZE 2", "LUT_3D_SIZE"),
            format!("DOMAIN_MIN 0 0 0\nDOMAIN_MAX 1 0 1\n{good}"),
            format!("DOMAIN_MAX 1 1\n{good}"),
            format!("LUT_1D_SIZE 2\n{good}"),
            format!("LUT_IN_VIDEO_RANGE\n{good}"),
            format!("lowercase keyword\n{good}"),
        ];
        for text in cases {
            assert!(cube(&text).is_err(), "{text:?}");
        }
        // Cut anywhere: refused or read, never a panic.
        for end in 0..good.len() {
            let _ = cube(&good[..end]);
        }
        assert_eq!(
            parse_lut(b"\xff\xfeL\0", LutFormat::Cube, "x.cube").map(|_| ()),
            Err(LutFileError::NotText)
        );
        // A huge size alone allocates nothing beyond the limit, and is refused.
        assert!(matches!(
            cube("LUT_3D_SIZE 100000\n0 0 0\n"),
            Err(LutFileError::Unsupported(_))
        ));
    }

    /// A 3DL file of `n` points a side on a mesh up to `last`, its output `f` of the input codes
    /// (blue fastest).
    fn three_dl_of(header: &str, mesh: &[u32], f: impl Fn([u32; 3]) -> [u32; 3]) -> String {
        let mut text = String::from(header);
        text += &mesh
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        text += "\n";
        for &r in mesh {
            for &g in mesh {
                for &b in mesh {
                    let v = f([r, g, b]);
                    text += &format!("{} {} {}\n", v[0], v[1], v[2]);
                }
            }
        }
        text
    }

    #[test]
    fn three_dl_files_are_read_blue_fastest_with_their_bit_depths() {
        // 10-bit in, 12-bit out (from the largest value), red and blue swapped.
        let mesh = [0, 341, 682, 1023];
        let text = three_dl_of("# Lustre\n", &mesh, |[r, g, b]| [b * 4, g * 4, r * 4]);
        let lut = three_dl(&text).unwrap();
        assert_eq!((lut.dimension(), lut.size()), (LutDimension::Three, 4));
        assert_eq!(lut.domain(), [[0.0; 3], [1.0; 3]]);
        // The first point after black in red is red's output 341·4 in blue.
        let c = lut.apply([341.0 / 1023.0, 0.0, 0.0]);
        let expected = 341.0 * 4.0 / 4095.0;
        assert!(c[0].abs() < 1e-6 && (c[2] - expected).abs() < 1e-6, "{c:?}");
        // Lustre's mesh, its last point written one less, and a Mesh header saying 16 bits out.
        let mesh: Vec<u32> = (0..=16).map(|k| (k * 64).min(1023)).collect();
        let text = three_dl_of("3DMESH\nMesh 4 16\n", &mesh, |[r, g, b]| [r, g, b]);
        let lut = three_dl(&text).unwrap();
        assert_eq!(lut.size(), 17);
        assert_eq!(lut.domain()[1], [1024.0 / 1023.0; 3]);
        // Code 64 is the second point: its output 64 / 65535.
        let c = lut.apply([64.0 / 1023.0, 0.0, 0.0]);
        assert!((c[0] - 64.0 / 65535.0).abs() < 1e-7, "{c:?}");
    }

    #[test]
    fn damaged_three_dl_files_are_refused() {
        let mesh = [0, 512, 1023];
        let good = three_dl_of("", &mesh, |[r, g, b]| [r, g, b]);
        let cases: Vec<String> = vec![
            String::new(),
            "3DMESH\n".into(),
            "0\n".into(),
            good.lines().take(10).collect::<Vec<_>>().join("\n"),
            format!("{good}0 0 0\n"),
            good.replacen("\n0 0 0", "\n0 0", 1),
            good.replacen("\n0 0 0", "\n0 -1 0", 1),
            good.replacen("\n0 0 0", "\n0 x 0", 1),
            // A mesh not evenly spaced, not from 0, beyond 16 bits; codes beyond 16 bits.
            three_dl_of("", &[0, 300, 1023], |c| c),
            three_dl_of("", &[10, 512, 1014], |c| c),
            three_dl_of("", &[0, 70000, 140000], |c| c),
            three_dl_of("", &mesh, |[r, ..]| [r * 100, 0, 0]),
            format!("Mesh 2 20\n{good}"),
            format!("Mesh 2\n{good}"),
            // More points than the largest table.
            format!(
                "{}\n",
                (0..200)
                    .map(|k| (k * 4).to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
        ];
        for text in cases {
            assert!(three_dl(&text).is_err(), "{text:?}");
        }
        for end in 0..good.len() {
            let _ = three_dl(&good[..end]);
        }
    }

    #[test]
    fn files_are_told_by_their_extension() {
        assert_eq!(
            LutFormat::of_path(Path::new("a/Film.CUBE")),
            Some(LutFormat::Cube)
        );
        assert_eq!(
            LutFormat::of_path(Path::new("b.3dl")),
            Some(LutFormat::ThreeDl)
        );
        assert_eq!(LutFormat::of_path(Path::new("c.look")), None);
        assert_eq!(
            read_lut(Path::new("missing.look")).map(|_| ()),
            Err(LutFileError::UnknownFormat)
        );
        assert!(matches!(
            read_lut(Path::new("surely/missing/file.cube")),
            Err(LutFileError::Io(_))
        ));
    }
}
