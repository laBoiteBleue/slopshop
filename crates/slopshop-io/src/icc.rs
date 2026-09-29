//! Minimal ICC profile reader for matrix/TRC profiles (ADR 0007).
//!
//! Turns an RGB (or gray) ICC profile into a [`ColorSpace`]: primaries recovered from the
//! colorant tags (undoing the chromatic adaptation to the D50 PCS, described by the `chad` tag
//! in v4 profiles, or implied by a non-D50 `wtpt` in v2 profiles) and a transfer function from the tone curves. Well-known spaces are recognized and snapped
//! to their exact definition. LUT-based profiles (CMYK, Lab, device links, A2B-only) are
//! reported as unsupported; they will go through lcms2 later.
//!
//! The input is untrusted: every read is bounds-checked, nothing panics on malformed data.

use slopshop_core::color::{
    ColorSpace, D50, Mat3, RgbPrimaries, TransferFunction, bradford, mat_inverse, mat_vec,
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
/// exact definition (and a recognizable name).
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
        .unwrap_or(space)
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
pub(crate) mod tests {
    use super::*;
    use slopshop_core::color::{RgbPrimaries, mat_mul};

    /// Build a matrix/TRC ICC profile (v4 style: colorants adapted to D50 with Bradford, plus
    /// `chad`) for `space`, with the tone curve written as `curve` tag bytes.
    pub(crate) fn profile(space: ColorSpace, curve: &[u8], gray: bool) -> Vec<u8> {
        build(space, curve, gray, false)
    }

    /// Same, v2 style: no `chad`, the source white in `wtpt`.
    fn profile_v2(space: ColorSpace, curve: &[u8]) -> Vec<u8> {
        build(space, curve, false, true)
    }

    fn build(space: ColorSpace, curve: &[u8], gray: bool, v2: bool) -> Vec<u8> {
        let mut tags: Vec<([u8; 4], Vec<u8>)> = Vec::new();
        let fixed = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
        let xyz_tag = |v: [f64; 3]| {
            let mut d = b"XYZ \0\0\0\0".to_vec();
            for c in v {
                d.extend(fixed(c));
            }
            d
        };
        if gray {
            tags.push((*b"kTRC", curve.to_vec()));
        } else {
            let to_xyz = space.primaries.to_xyz();
            let chad = bradford(space.primaries.white, slopshop_core::color::D50);
            let adapted = mat_mul(&chad, &to_xyz);
            for (i, sig) in [b"rXYZ", b"gXYZ", b"bXYZ"].into_iter().enumerate() {
                tags.push((*sig, xyz_tag([adapted[0][i], adapted[1][i], adapted[2][i]])));
            }
            for sig in [b"rTRC", b"gTRC", b"bTRC"] {
                tags.push((*sig, curve.to_vec()));
            }
            if v2 {
                let white = mat_vec(&to_xyz, [1.0, 1.0, 1.0]);
                tags.push((*b"wtpt", xyz_tag(white)));
            } else {
                let mut sf32 = b"sf32\0\0\0\0".to_vec();
                for v in chad.iter().flatten() {
                    sf32.extend(fixed(*v));
                }
                tags.push((*b"chad", sf32));
            }
        }
        let mut out = vec![0u8; 128];
        out[16..20].copy_from_slice(if gray { b"GRAY" } else { b"RGB " });
        out[20..24].copy_from_slice(b"XYZ ");
        out.extend((tags.len() as u32).to_be_bytes());
        let mut offset = 132 + tags.len() * 12;
        let mut data: Vec<u8> = Vec::new();
        for (sig, bytes) in &tags {
            out.extend(sig);
            out.extend((offset as u32).to_be_bytes());
            out.extend((bytes.len() as u32).to_be_bytes());
            offset += bytes.len();
            data.extend(bytes);
        }
        out.extend(data);
        let size = out.len() as u32;
        out[0..4].copy_from_slice(&size.to_be_bytes());
        out
    }

    pub(crate) fn para_srgb() -> Vec<u8> {
        let fixed = |v: f64| ((v * 65536.0).round() as i32).to_be_bytes();
        let mut d = b"para\0\0\0\0".to_vec();
        d.extend(3u16.to_be_bytes());
        d.extend(0u16.to_be_bytes());
        for v in [2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045] {
            d.extend(fixed(v));
        }
        d
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
