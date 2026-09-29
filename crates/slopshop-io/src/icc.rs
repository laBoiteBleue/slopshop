//! Minimal ICC profile reader and writer for matrix/TRC profiles (ADR 0007, ADR 0008).
//!
//! Reading turns an RGB (or gray) ICC profile into a [`ColorSpace`]: primaries recovered from
//! the colorant tags (undoing the chromatic adaptation to the D50 PCS, described by the `chad`
//! tag in v4 profiles, or implied by a non-D50 `wtpt` in v2 profiles) and a transfer function
//! from the tone curves. Well-known spaces are recognized and snapped to their exact
//! definition. LUT-based profiles (CMYK, Lab, device links, A2B-only) are reported as
//! unsupported; they will go through lcms2 later. The input is untrusted: every read is
//! bounds-checked, nothing panics on malformed data.
//!
//! Writing does the reverse for export: an ICC v4.4 RGB display profile whose tone curves are
//! exact parametric curves, so that it reads back as the same space.

use slopshop_core::color::{
    ColorSpace, D50, Mat3, RgbPrimaries, TransferFunction, bradford, mat_inverse, mat_mul, mat_vec,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct IccColor {
    pub space: ColorSpace,
    /// The tone curve had to be approximated (tables that match no parametric curve).
    pub approximated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IccError {
    Malformed,
    /// Not an RGB or gray profile with an XYZ connection space.
    UnsupportedColorModel,
    /// No matrix/TRC description (LUT-based profile).
    LutBased,
    /// Writing: the transfer function has no ICC tone curve (PQ and HLG, which are tagged with
    /// cICP instead) or its parameters are outside the ICC number range.
    UnsupportedTransfer,
    /// Writing: degenerate primaries, or colorants outside the ICC number range.
    InvalidPrimaries,
}

/// D50, the ICC profile connection space white, as XYZ.
const D50_XYZ: [f64; 3] = [0.9642, 1.0, 0.8249];

pub(crate) fn parse(bytes: &[u8]) -> Result<IccColor, IccError> {
    let data_space = bytes.get(16..20).ok_or(IccError::Malformed)?;
    let pcs = bytes.get(20..24).ok_or(IccError::Malformed)?;
    if pcs != b"XYZ " {
        return Err(IccError::UnsupportedColorModel);
    }
    let tags = TagTable::read(bytes)?;
    match data_space {
        b"GRAY" => {
            let (transfer, approximated) = tags.curve(b"kTRC")?.ok_or(IccError::LutBased)?;
            Ok(IccColor {
                space: snap(ColorSpace {
                    primaries: RgbPrimaries::REC709,
                    transfer,
                }),
                approximated,
            })
        }
        b"RGB " => {
            let (Some(r), Some(g), Some(b)) =
                (tags.xyz(b"rXYZ")?, tags.xyz(b"gXYZ")?, tags.xyz(b"bXYZ")?)
            else {
                return Err(IccError::LutBased);
            };
            let curves = [
                tags.curve(b"rTRC")?,
                tags.curve(b"gTRC")?,
                tags.curve(b"bTRC")?,
            ];
            let [Some(rc), Some(gc), Some(bc)] = curves else {
                return Err(IccError::LutBased);
            };
            // One transfer function per space: different channel curves are approximated by
            // the green one (the most visible).
            let differs = !transfer_close(rc.0, gc.0) || !transfer_close(bc.0, gc.0);
            let (transfer, fitted) = gc;
            let approximated = fitted || rc.1 || bc.1 || differs;

            // Colorants are adapted to D50. Undo the adaptation so that the original white point
            // is recovered: v4 profiles say how (`chad`); v2 profiles without it (HP sRGB,
            // Adobe RGB 1998) give the media white (`wtpt`), conventionally adapted with
            // Bradford. Otherwise the space really is D50-based.
            let adaptation = match tags.matrix(b"chad")? {
                Some(chad) => Some(chad),
                None => match tags.xyz(b"wtpt")? {
                    Some(wtpt) if !close_xy(xy(wtpt)?, D50) => Some(bradford(xy(wtpt)?, D50)),
                    _ => None,
                },
            };
            let (colorants, white) = match adaptation {
                Some(adapt) => {
                    let undo = mat_inverse(&adapt).ok_or(IccError::Malformed)?;
                    (
                        [r, g, b].map(|c| mat_vec(&undo, c)),
                        mat_vec(&undo, D50_XYZ),
                    )
                }
                None => ([r, g, b], D50_XYZ),
            };
            let primaries = RgbPrimaries {
                red: xy(colorants[0])?,
                green: xy(colorants[1])?,
                blue: xy(colorants[2])?,
                white: xy(white)?,
            };
            if !primaries.is_valid() {
                return Err(IccError::Malformed);
            }
            Ok(IccColor {
                space: snap(ColorSpace {
                    primaries,
                    transfer,
                }),
                approximated,
            })
        }
        _ => Err(IccError::UnsupportedColorModel),
    }
}

fn close_xy(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 0.002 && (a[1] - b[1]).abs() < 0.002
}

fn xy([x, y, z]: [f64; 3]) -> Result<[f64; 2], IccError> {
    let sum = x + y + z;
    if !sum.is_finite() || sum.abs() < 1e-9 {
        return Err(IccError::Malformed);
    }
    Ok([x / sum, y / sum])
}

struct TagTable<'a> {
    bytes: &'a [u8],
    entries: Vec<([u8; 4], usize, usize)>,
}

impl<'a> TagTable<'a> {
    fn read(bytes: &'a [u8]) -> Result<Self, IccError> {
        let count = be_u32(bytes, 128)? as usize;
        // A profile with thousands of tags is corrupt; don't allocate for it.
        if count > 1024 {
            return Err(IccError::Malformed);
        }
        let mut entries = Vec::with_capacity(count);
        for i in 0..count {
            let at = 132 + i * 12;
            let signature: [u8; 4] = bytes
                .get(at..at + 4)
                .ok_or(IccError::Malformed)?
                .try_into()
                .map_err(|_| IccError::Malformed)?;
            let offset = be_u32(bytes, at + 4)? as usize;
            let size = be_u32(bytes, at + 8)? as usize;
            entries.push((signature, offset, size));
        }
        Ok(Self { bytes, entries })
    }

    fn data(&self, signature: &[u8; 4]) -> Result<Option<&'a [u8]>, IccError> {
        let Some(&(_, offset, size)) = self.entries.iter().find(|(s, _, _)| s == signature) else {
            return Ok(None);
        };
        let end = offset.checked_add(size).ok_or(IccError::Malformed)?;
        self.bytes
            .get(offset..end)
            .map(Some)
            .ok_or(IccError::Malformed)
    }

    fn xyz(&self, signature: &[u8; 4]) -> Result<Option<[f64; 3]>, IccError> {
        let Some(data) = self.data(signature)? else {
            return Ok(None);
        };
        if data.get(0..4) != Some(b"XYZ ") {
            return Err(IccError::Malformed);
        }
        Ok(Some([
            s15f16(data, 8)?,
            s15f16(data, 12)?,
            s15f16(data, 16)?,
        ]))
    }

    fn matrix(&self, signature: &[u8; 4]) -> Result<Option<Mat3>, IccError> {
        let Some(data) = self.data(signature)? else {
            return Ok(None);
        };
        if data.get(0..4) != Some(b"sf32") {
            return Err(IccError::Malformed);
        }
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = s15f16(data, 8 + (i * 3 + j) * 4)?;
            }
        }
        Ok(Some(m))
    }

    /// A tone curve and whether it had to be approximated.
    fn curve(&self, signature: &[u8; 4]) -> Result<Option<(TransferFunction, bool)>, IccError> {
        let Some(data) = self.data(signature)? else {
            return Ok(None);
        };
        match data.get(0..4) {
            Some(b"curv") => {
                let count = be_u32(data, 8)? as usize;
                match count {
                    0 => Ok(Some((TransferFunction::Linear, false))),
                    1 => {
                        let gamma = f32::from(be_u16(data, 12)?) / 256.0;
                        Ok(Some((gamma_or_linear(gamma), false)))
                    }
                    _ => {
                        let table: Vec<f32> = (0..count)
                            .map(|i| be_u16(data, 12 + i * 2).map(|v| f32::from(v) / 65535.0))
                            .collect::<Result<_, _>>()?;
                        Ok(Some(fit_table(&table)))
                    }
                }
            }
            Some(b"para") => {
                let kind = be_u16(data, 8)?;
                let p = |i: usize| s15f16(data, 12 + i * 4).map(|v| v as f32);
                let tf = match kind {
                    0 => gamma_or_linear(p(0)?),
                    1 => {
                        let (g, a, b) = (p(0)?, p(1)?, p(2)?);
                        parametric(g, a, b, 0.0, threshold(a, b), 0.0, 0.0)
                    }
                    2 => {
                        let (g, a, b, c) = (p(0)?, p(1)?, p(2)?, p(3)?);
                        parametric(g, a, b, 0.0, threshold(a, b), c, c)
                    }
                    3 => parametric(p(0)?, p(1)?, p(2)?, p(3)?, p(4)?, 0.0, 0.0),
                    4 => parametric(p(0)?, p(1)?, p(2)?, p(3)?, p(4)?, p(5)?, p(6)?),
                    _ => return Err(IccError::Malformed),
                };
                Ok(Some((tf, false)))
            }
            _ => Err(IccError::LutBased),
        }
    }
}

fn threshold(a: f32, b: f32) -> f32 {
    if a != 0.0 { -b / a } else { 0.0 }
}

fn parametric(g: f32, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> TransferFunction {
    TransferFunction::Parametric {
        g,
        a,
        b,
        c,
        d,
        e,
        f,
    }
}

fn gamma_or_linear(gamma: f32) -> TransferFunction {
    if (gamma - 1.0).abs() < 1e-4 {
        TransferFunction::Linear
    } else {
        TransferFunction::Gamma(gamma)
    }
}

/// Fit a sampled tone curve: exactly linear or sRGB when it matches, else a pure gamma
/// (least squares in log space), reporting whether the fit is only approximate.
fn fit_table(table: &[f32]) -> (TransferFunction, bool) {
    let n = table.len();
    let samples = || {
        table
            .iter()
            .enumerate()
            .map(move |(i, &y)| (i as f32 / (n - 1) as f32, y))
    };
    let max_error = |tf: TransferFunction| {
        samples()
            .map(|(x, y)| (tf.decode(x) - y).abs())
            .fold(0.0, f32::max)
    };
    const EXACT: f32 = 1.0 / 1024.0;
    for tf in [TransferFunction::Linear, TransferFunction::Srgb] {
        if max_error(tf) < EXACT {
            return (tf, false);
        }
    }
    let (mut num, mut den) = (0.0f64, 0.0f64);
    for (x, y) in samples() {
        if x > 0.05 && y > 0.0 {
            let (lx, ly) = (f64::from(x).ln(), f64::from(y).ln());
            num += lx * ly;
            den += lx * lx;
        }
    }
    let gamma = if den > 0.0 { (num / den) as f32 } else { 1.0 };
    let tf = gamma_or_linear(gamma);
    (tf, max_error(tf) >= EXACT)
}

/// Whether two transfer functions agree within display precision.
fn transfer_close(a: TransferFunction, b: TransferFunction) -> bool {
    (0..=16).all(|i| {
        let x = i as f32 / 16.0;
        (a.decode(x) - b.decode(x)).abs() < 1e-3
    })
}

/// Replace a color space by the named one it matches, so that well-known profiles get their
/// exact definition (and a recognizable name). Otherwise, a tone curve that matches a named
/// one (sRGB or Rec.709 written as a parametric curve) still gets its exact definition.
pub(crate) fn snap(space: ColorSpace) -> ColorSpace {
    const NAMED: [ColorSpace; 7] = [
        ColorSpace::SRGB,
        ColorSpace::LINEAR_SRGB,
        ColorSpace::DISPLAY_P3,
        ColorSpace::ADOBE_RGB,
        ColorSpace::PROPHOTO,
        ColorSpace::REC2020,
        ColorSpace::LINEAR_REC2020,
    ];
    NAMED
        .into_iter()
        .find(|named| {
            let (p, q) = (&named.primaries, &space.primaries);
            close_xy(p.red, q.red)
                && close_xy(p.green, q.green)
                && close_xy(p.blue, q.blue)
                && close_xy(p.white, q.white)
                && transfer_close(named.transfer, space.transfer)
        })
        .unwrap_or_else(|| {
            let transfer = [TransferFunction::Srgb, TransferFunction::Rec709]
                .into_iter()
                .find(|named| transfer_close(*named, space.transfer))
                .unwrap_or(space.transfer);
            ColorSpace { transfer, ..space }
        })
}

/// Header version field: ICC.1:2022, profile version 4.4.
const VERSION_4_4: u32 = 0x0440_0000;

/// Creation date and time written in every profile (year, month, day, hour, minute, second).
/// Fixed, so that a color space always gives the same bytes (reproducible exports).
const CREATION_DATE: [u16; 6] = [2026, 9, 29, 0, 0, 0];

const COPYRIGHT: &str = "No copyright, use freely";

// ITU-R BT.709 OETF constants at full precision (the engine's decoder uses the same curve).
const REC709_ALPHA: f64 = 1.099_296_826_809_44;
const REC709_BETA: f64 = 0.018_053_968_510_807;

/// Build an ICC v4.4 RGB display profile (matrix/TRC) describing `space`: colorants adapted to
/// the D50 connection space with Bradford (the adaptation stored in `chad`), and tone curves as
/// exact `para` curves (the parameters are only rounded to the ICC 16.16 fixed-point format).
/// PQ and HLG have no ICC tone curve: files in those spaces are tagged with cICP instead.
// Used by the exporter (ADR 0008), which lands separately.
#[allow(dead_code)]
pub(crate) fn write_matrix_trc(space: &ColorSpace) -> Result<Vec<u8>, IccError> {
    let primaries = &space.primaries;
    if !primaries.is_valid() {
        return Err(IccError::InvalidPrimaries);
    }
    let curve = para(space.transfer)?;
    let chad = bradford(primaries.white, D50);
    let colorants = mat_mul(&chad, &primaries.to_xyz());
    let colorant = |i: usize| {
        xyz_type([colorants[0][i], colorants[1][i], colorants[2][i]])
            .ok_or(IccError::InvalidPrimaries)
    };
    let tags = [
        (*b"desc", mluc(description(space))),
        (*b"cprt", mluc(COPYRIGHT)),
        // v4: the media white of a display profile is the PCS white.
        (
            *b"wtpt",
            xyz_type(D50_XYZ).ok_or(IccError::InvalidPrimaries)?,
        ),
        (
            *b"chad",
            sf32_type(&chad).ok_or(IccError::InvalidPrimaries)?,
        ),
        (*b"rXYZ", colorant(0)?),
        (*b"gXYZ", colorant(1)?),
        (*b"bXYZ", colorant(2)?),
        (*b"rTRC", curve.clone()),
        (*b"gTRC", curve.clone()),
        (*b"bTRC", curve),
    ];
    Ok(assemble(b"RGB ", &tags))
}

/// English name of the space, for the `desc` tag (file metadata, not UI text).
fn description(space: &ColorSpace) -> &'static str {
    match space.id() {
        Some("srgb") => "sRGB",
        Some("linear-srgb") => "Linear sRGB",
        Some("display-p3") => "Display P3",
        Some("adobe-rgb") => "Adobe RGB (1998) compatible",
        Some("prophoto") => "ProPhoto RGB (ROMM RGB)",
        Some("rec2020") => "Rec. 2020",
        Some("linear-rec2020") => "Linear Rec. 2020",
        _ => "Custom RGB",
    }
}

/// A tone curve as a `para` tag: type 0 (pure power) when it is one, type 3 (power with a
/// linear segment) or type 4 (with offsets) otherwise.
fn para(transfer: TransferFunction) -> Result<Vec<u8>, IccError> {
    let (kind, params): (u16, Vec<f64>) = match transfer {
        TransferFunction::Linear => (0, vec![1.0]),
        TransferFunction::Gamma(g) if g.is_finite() && g > 0.0 => (0, vec![f64::from(g)]),
        TransferFunction::Srgb => (
            3,
            vec![2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045],
        ),
        // Inverse of V = α·L^0.45 − (α − 1) above the knee, V = 4.5·L below.
        TransferFunction::Rec709 => (
            3,
            vec![
                1.0 / 0.45,
                1.0 / REC709_ALPHA,
                (REC709_ALPHA - 1.0) / REC709_ALPHA,
                1.0 / 4.5,
                4.5 * REC709_BETA,
            ],
        ),
        TransferFunction::Parametric {
            g,
            a,
            b,
            c,
            d,
            e,
            f,
        } => {
            if e == 0.0 && f == 0.0 {
                (3, [g, a, b, c, d].map(f64::from).to_vec())
            } else {
                (4, [g, a, b, c, d, e, f].map(f64::from).to_vec())
            }
        }
        TransferFunction::Gamma(_) | TransferFunction::Pq | TransferFunction::Hlg => {
            return Err(IccError::UnsupportedTransfer);
        }
    };
    let mut data = b"para\0\0\0\0".to_vec();
    data.extend(kind.to_be_bytes());
    data.extend(0u16.to_be_bytes());
    for p in params {
        data.extend(fixed(p).ok_or(IccError::UnsupportedTransfer)?);
    }
    Ok(data)
}

/// A `mluc` tag holding one English (en-US) string.
fn mluc(text: &str) -> Vec<u8> {
    let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut data = b"mluc\0\0\0\0".to_vec();
    data.extend(1u32.to_be_bytes()); // one record
    data.extend(12u32.to_be_bytes()); // record size
    data.extend(b"enUS");
    data.extend((utf16.len() as u32).to_be_bytes());
    data.extend(28u32.to_be_bytes()); // string offset, from the start of the tag
    data.extend(utf16);
    data
}

fn xyz_type(xyz: [f64; 3]) -> Option<Vec<u8>> {
    let mut data = b"XYZ \0\0\0\0".to_vec();
    for v in xyz {
        data.extend(fixed(v)?);
    }
    Some(data)
}

fn sf32_type(m: &Mat3) -> Option<Vec<u8>> {
    let mut data = b"sf32\0\0\0\0".to_vec();
    for v in m.iter().flatten() {
        data.extend(fixed(*v)?);
    }
    Some(data)
}

/// `s15Fixed16Number`, `None` outside its range.
fn fixed(v: f64) -> Option<[u8; 4]> {
    let scaled = (v * 65536.0).round();
    (scaled >= f64::from(i32::MIN) && scaled <= f64::from(i32::MAX))
        .then(|| (scaled as i32).to_be_bytes())
}

/// Serialize a display profile: header, tag table, then the tag data, each element starting on
/// a 4-byte boundary (and the profile padded to one). Tags with identical data share it, as
/// the three tone curves usually do.
fn assemble(data_space: &[u8; 4], tags: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![0u8; 128];
    out[8..12].copy_from_slice(&VERSION_4_4.to_be_bytes());
    out[12..16].copy_from_slice(b"mntr");
    out[16..20].copy_from_slice(data_space);
    out[20..24].copy_from_slice(b"XYZ ");
    for (i, v) in CREATION_DATE.iter().enumerate() {
        out[24 + i * 2..26 + i * 2].copy_from_slice(&v.to_be_bytes());
    }
    out[36..40].copy_from_slice(b"acsp");
    // Rendering intent (64..68): 0, perceptual. Profile ID (84..100): 0, not computed.
    for (i, v) in D50_XYZ.into_iter().enumerate() {
        // Invariant: D50 is well inside the fixed-point range.
        let bytes = fixed(v).unwrap_or_default();
        out[68 + i * 4..72 + i * 4].copy_from_slice(&bytes);
    }
    out.extend((tags.len() as u32).to_be_bytes());
    let table = out.len();
    out.resize(table + tags.len() * 12, 0);
    for (i, (signature, data)) in tags.iter().enumerate() {
        let shared = tags[..i].iter().position(|(_, earlier)| earlier == data);
        let offset = match shared {
            Some(j) => {
                let at = table + j * 12 + 4;
                u32::from_be_bytes([out[at], out[at + 1], out[at + 2], out[at + 3]])
            }
            None => {
                let offset = out.len() as u32;
                out.extend(data);
                out.resize(out.len().next_multiple_of(4), 0);
                offset
            }
        };
        let entry = table + i * 12;
        out[entry..entry + 4].copy_from_slice(signature);
        out[entry + 4..entry + 8].copy_from_slice(&offset.to_be_bytes());
        out[entry + 8..entry + 12].copy_from_slice(&(data.len() as u32).to_be_bytes());
    }
    let size = out.len() as u32;
    out[0..4].copy_from_slice(&size.to_be_bytes());
    out
}

fn be_u32(bytes: &[u8], at: usize) -> Result<u32, IccError> {
    let b = bytes.get(at..at + 4).ok_or(IccError::Malformed)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn be_u16(bytes: &[u8], at: usize) -> Result<u16, IccError> {
    let b = bytes.get(at..at + 2).ok_or(IccError::Malformed)?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

fn s15f16(bytes: &[u8], at: usize) -> Result<f64, IccError> {
    Ok(f64::from(be_u32(bytes, at)? as i32) / 65536.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Named spaces that an ICC profile can describe (all but PQ and HLG).
    const WRITABLE: [ColorSpace; 7] = [
        ColorSpace::SRGB,
        ColorSpace::LINEAR_SRGB,
        ColorSpace::DISPLAY_P3,
        ColorSpace::ADOBE_RGB,
        ColorSpace::PROPHOTO,
        ColorSpace::REC2020,
        ColorSpace::LINEAR_REC2020,
    ];

    /// A matrix/TRC profile for `space` with the tone curve given as `curve` tag bytes, to test
    /// the reader on encodings the writer never produces (`curv` gammas and tables, gray). v4
    /// style: colorants adapted to D50 with Bradford, plus `chad`.
    fn profile(space: ColorSpace, curve: &[u8], gray: bool) -> Vec<u8> {
        build(space, curve, gray, false)
    }

    /// Same, v2 style: no `chad`, the source white in `wtpt`.
    fn profile_v2(space: ColorSpace, curve: &[u8]) -> Vec<u8> {
        build(space, curve, false, true)
    }

    fn build(space: ColorSpace, curve: &[u8], gray: bool, v2: bool) -> Vec<u8> {
        let mut tags: Vec<([u8; 4], Vec<u8>)> = Vec::new();
        if gray {
            tags.push((*b"kTRC", curve.to_vec()));
        } else {
            let to_xyz = space.primaries.to_xyz();
            let chad = bradford(space.primaries.white, D50);
            let adapted = mat_mul(&chad, &to_xyz);
            for (i, sig) in [b"rXYZ", b"gXYZ", b"bXYZ"].into_iter().enumerate() {
                let colorant = [adapted[0][i], adapted[1][i], adapted[2][i]];
                tags.push((*sig, xyz_type(colorant).unwrap()));
            }
            for sig in [b"rTRC", b"gTRC", b"bTRC"] {
                tags.push((*sig, curve.to_vec()));
            }
            if v2 {
                let white = mat_vec(&to_xyz, [1.0, 1.0, 1.0]);
                tags.push((*b"wtpt", xyz_type(white).unwrap()));
            } else {
                tags.push((*b"chad", sf32_type(&chad).unwrap()));
            }
        }
        let mut out = assemble(if gray { b"GRAY" } else { b"RGB " }, &tags);
        if v2 {
            out[8..12].copy_from_slice(&0x0210_0000u32.to_be_bytes());
        }
        out
    }

    fn para_srgb() -> Vec<u8> {
        para(TransferFunction::Srgb).unwrap()
    }

    /// The English string of a `mluc` tag.
    fn mluc_text(data: &[u8]) -> String {
        assert_eq!(&data[0..4], b"mluc");
        assert_eq!(&data[16..20], b"enUS");
        let len = be_u32(data, 20).unwrap() as usize;
        let offset = be_u32(data, 24).unwrap() as usize;
        let units: Vec<u16> = data[offset..offset + len]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_be_bytes(*b))
            .collect();
        String::from_utf16(&units).unwrap()
    }

    #[test]
    fn written_profiles_read_back_as_the_same_space() {
        for space in WRITABLE {
            let color = parse(&write_matrix_trc(&space).unwrap()).unwrap();
            assert_eq!(
                color,
                IccColor {
                    space,
                    approximated: false
                },
                "{:?}",
                space.id()
            );
        }
    }

    #[test]
    fn written_tone_curves_are_exact() {
        for space in WRITABLE {
            let bytes = write_matrix_trc(&space).unwrap();
            let tags = TagTable::read(&bytes).unwrap();
            for sig in [b"rTRC", b"gTRC", b"bTRC"] {
                let (curve, approximated) = tags.curve(sig).unwrap().unwrap();
                assert!(!approximated);
                // Only the 16.16 fixed-point rounding of the parameters differs.
                for i in 0..=1024 {
                    let x = i as f32 / 1024.0;
                    let (got, want) = (curve.decode(x), space.transfer.decode(x));
                    assert!((got - want).abs() < 2e-5, "{:?} at {x}", space.id());
                }
            }
        }
    }

    #[test]
    fn custom_spaces_round_trip() {
        // A non-D65 white exercises the chromatic adaptation.
        let primaries = RgbPrimaries {
            red: [0.66, 0.33],
            green: [0.25, 0.68],
            blue: [0.14, 0.07],
            white: [0.32, 0.335],
        };
        let parametric = |e, f| TransferFunction::Parametric {
            g: 2.6,
            a: 0.9,
            b: 0.1,
            c: 0.2,
            d: 0.05,
            e,
            f,
        };
        for transfer in [
            TransferFunction::Linear,
            TransferFunction::Gamma(2.0),
            TransferFunction::Srgb,
            TransferFunction::Rec709,
            parametric(0.0, 0.0),
            parametric(0.01, 0.005),
        ] {
            let space = ColorSpace {
                primaries,
                transfer,
            };
            let color = parse(&write_matrix_trc(&space).unwrap()).unwrap();
            assert!(!color.approximated);
            let p = color.space.primaries;
            let got = [p.red, p.green, p.blue, p.white];
            let want = [
                primaries.red,
                primaries.green,
                primaries.blue,
                primaries.white,
            ];
            for (g, w) in got.iter().zip(want) {
                assert!((g[0] - w[0]).abs() < 1e-4 && (g[1] - w[1]).abs() < 1e-4);
            }
            // Named curves come back exactly, others up to the fixed-point rounding.
            if let TransferFunction::Parametric { .. } = transfer {
                assert!(transfer_close(color.space.transfer, transfer));
            } else {
                assert_eq!(color.space.transfer, transfer);
            }
        }
    }

    #[test]
    fn spaces_without_a_matrix_trc_description_are_errors() {
        for space in [ColorSpace::REC2100_PQ, ColorSpace::REC2100_HLG] {
            assert_eq!(write_matrix_trc(&space), Err(IccError::UnsupportedTransfer));
        }
        for transfer in [
            TransferFunction::Gamma(f32::NAN),
            TransferFunction::Gamma(0.0),
            TransferFunction::Parametric {
                g: 2.0,
                a: 1e6,
                b: 0.0,
                c: 0.0,
                d: 0.0,
                e: 0.0,
                f: 0.0,
            },
        ] {
            let space = ColorSpace {
                transfer,
                ..ColorSpace::SRGB
            };
            assert_eq!(write_matrix_trc(&space), Err(IccError::UnsupportedTransfer));
        }
        let degenerate = ColorSpace {
            primaries: RgbPrimaries {
                red: [0.3, 0.3],
                green: [0.3, 0.3],
                blue: [0.3, 0.3],
                white: slopshop_core::color::D65,
            },
            transfer: TransferFunction::Linear,
        };
        assert_eq!(
            write_matrix_trc(&degenerate),
            Err(IccError::InvalidPrimaries)
        );
    }

    #[test]
    fn written_profiles_are_well_formed() {
        for space in WRITABLE {
            let bytes = write_matrix_trc(&space).unwrap();
            let be = |at| be_u32(&bytes, at).unwrap();
            assert_eq!(be(0) as usize, bytes.len(), "size field");
            assert_eq!(bytes.len() % 4, 0);
            assert_eq!(be(8), 0x0440_0000, "version 4.4");
            assert_eq!(&bytes[12..24], b"mntrRGB XYZ ");
            assert_eq!(bytes[24..26], 2026u16.to_be_bytes());
            assert_eq!(&bytes[36..40], b"acsp");
            assert_eq!(be(64), 0, "perceptual intent");
            let d50 = [0, 0, 0xf6, 0xd6, 0, 1, 0, 0, 0, 0, 0xd3, 0x2d];
            assert_eq!(bytes[68..80], d50, "D50 illuminant");
            let count = be(128) as usize;
            let data_start = 132 + count * 12;
            let mut signatures = Vec::new();
            for i in 0..count {
                let at = 132 + i * 12;
                let (offset, size) = (be(at + 4) as usize, be(at + 8) as usize);
                assert_eq!(offset % 4, 0, "tag data alignment");
                assert!(offset >= data_start && offset + size <= bytes.len());
                signatures.push(&bytes[at..at + 4]);
            }
            for required in [
                b"desc", b"cprt", b"wtpt", b"chad", b"rXYZ", b"gXYZ", b"bXYZ", b"rTRC", b"gTRC",
                b"bTRC",
            ] {
                assert!(signatures.contains(&&required[..]));
            }
            let tags = TagTable::read(&bytes).unwrap();
            let desc = mluc_text(tags.data(b"desc").unwrap().unwrap());
            assert_eq!(desc, description(&space));
            assert_ne!(desc, "Custom RGB");
            assert_eq!(mluc_text(tags.data(b"cprt").unwrap().unwrap()), COPYRIGHT);
            let wtpt = tags.xyz(b"wtpt").unwrap().unwrap();
            assert!((0..3).all(|i| (wtpt[i] - D50_XYZ[i]).abs() < 1e-4));
        }
        let p3 = write_matrix_trc(&ColorSpace::DISPLAY_P3).unwrap();
        let desc = TagTable::read(&p3).unwrap().data(b"desc").unwrap().unwrap();
        assert_eq!(mluc_text(desc), "Display P3");
    }

    fn curv_gamma(gamma: f32) -> Vec<u8> {
        let mut d = b"curv\0\0\0\0".to_vec();
        d.extend(1u32.to_be_bytes());
        d.extend(((gamma * 256.0).round() as u16).to_be_bytes());
        d
    }

    fn curv_table(tf: TransferFunction, n: usize) -> Vec<u8> {
        let mut d = b"curv\0\0\0\0".to_vec();
        d.extend((n as u32).to_be_bytes());
        for i in 0..n {
            let x = i as f32 / (n - 1) as f32;
            d.extend(((tf.decode(x).clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes());
        }
        d
    }

    #[test]
    fn well_known_profiles_are_recognized() {
        let cases = [
            (ColorSpace::SRGB, para_srgb()),
            (ColorSpace::DISPLAY_P3, para_srgb()),
            (ColorSpace::ADOBE_RGB, curv_gamma(563.0 / 256.0)),
            (ColorSpace::PROPHOTO, curv_gamma(1.8)),
        ];
        for (space, curve) in cases {
            let color = parse(&profile(space, &curve, false)).unwrap();
            assert_eq!(color.space, space, "{:?}", space.id());
            assert!(!color.approximated);
        }
    }

    #[test]
    fn v2_profiles_without_chad_use_the_media_white() {
        for (space, curve) in [
            (ColorSpace::SRGB, para_srgb()),
            (ColorSpace::ADOBE_RGB, curv_gamma(563.0 / 256.0)),
        ] {
            let color = parse(&profile_v2(space, &curve)).unwrap();
            assert_eq!(color.space, space, "{:?}", space.id());
        }
        // A D50 white (ProPhoto) needs no adaptation.
        let color = parse(&profile_v2(ColorSpace::PROPHOTO, &curv_gamma(1.8))).unwrap();
        assert_eq!(color.space, ColorSpace::PROPHOTO);
    }

    #[test]
    fn srgb_given_as_a_table_is_recognized_exactly() {
        let color = parse(&profile(
            ColorSpace::SRGB,
            &curv_table(TransferFunction::Srgb, 1024),
            false,
        ))
        .unwrap();
        assert_eq!(color.space, ColorSpace::SRGB);
        assert!(!color.approximated);
    }

    #[test]
    fn odd_tables_are_approximated_and_reported() {
        // A curve that is neither sRGB nor a pure gamma.
        let odd = TransferFunction::Parametric {
            g: 3.0,
            a: 0.8,
            b: 0.2,
            c: 0.5,
            d: 0.3,
            e: 0.0,
            f: 0.0,
        };
        let color = parse(&profile(ColorSpace::SRGB, &curv_table(odd, 256), false)).unwrap();
        assert!(color.approximated);
    }

    #[test]
    fn custom_primaries_survive() {
        let custom = ColorSpace {
            primaries: RgbPrimaries {
                red: [0.66, 0.33],
                green: [0.25, 0.68],
                blue: [0.14, 0.07],
                white: slopshop_core::color::D65,
            },
            transfer: TransferFunction::Gamma(2.0),
        };
        let color = parse(&profile(custom, &curv_gamma(2.0), false)).unwrap();
        let p = color.space.primaries;
        assert!((p.red[0] - 0.66).abs() < 1e-3 && (p.green[1] - 0.68).abs() < 1e-3);
        assert!((p.white[0] - 0.3127).abs() < 1e-3);
    }

    #[test]
    fn gray_profiles() {
        let color = parse(&profile(ColorSpace::SRGB, &curv_gamma(2.2), true)).unwrap();
        assert_eq!(color.space.transfer, TransferFunction::Gamma(563.0 / 256.0));
    }

    #[test]
    fn malformed_and_unsupported_profiles_are_errors_not_panics() {
        assert_eq!(parse(&[]), Err(IccError::Malformed));
        let mut p = profile(ColorSpace::SRGB, &para_srgb(), false);
        p[20..24].copy_from_slice(b"Lab ");
        assert_eq!(parse(&p), Err(IccError::UnsupportedColorModel));
        let mut p = profile(ColorSpace::SRGB, &para_srgb(), false);
        p[16..20].copy_from_slice(b"CMYK");
        assert_eq!(parse(&p), Err(IccError::UnsupportedColorModel));
        // Truncated anywhere: never panics.
        let full = profile(ColorSpace::DISPLAY_P3, &para_srgb(), false);
        for len in 0..full.len() {
            let _ = parse(&full[..len]);
        }
        // A tag table pointing outside the data.
        let mut p = full.clone();
        p[136..140].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(parse(&p), Err(IccError::Malformed));
    }
}
