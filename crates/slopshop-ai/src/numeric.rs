//! Conversions between the floating-point formats of model weights: bfloat16 (how the FLUX.2
//! weights are published), half precision (what DirectML computes in) and single precision.

/// A bfloat16 value (its bits) as single precision: exact, bfloat16 is the top half of an `f32`.
pub fn bf16_to_f32(bits: u16) -> f32 {
    f32::from_bits(u32::from(bits) << 16)
}

/// Rounds to the nearest bfloat16 (ties to even), as PyTorch's `.to(torch.bfloat16)`.
pub fn f32_to_bf16(value: f32) -> u16 {
    let bits = value.to_bits();
    if value.is_nan() {
        return ((bits >> 16) as u16) | 0x40;
    }
    let rounding = 0x7fff + ((bits >> 16) & 1);
    (bits.wrapping_add(rounding) >> 16) as u16
}

/// `value` rounded through bfloat16 and back.
pub fn round_bf16(value: f32) -> f32 {
    bf16_to_f32(f32_to_bf16(value))
}

/// Rounds to the nearest half-precision value (ties to even); beyond its range, infinity.
pub fn f32_to_f16(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xff) as i32;
    let mantissa = bits & 0x007f_ffff;
    if exponent == 0xff {
        // Infinity, or NaN (kept quiet).
        return sign | 0x7c00 | if mantissa != 0 { 0x200 } else { 0 };
    }
    let unbiased = exponent - 127;
    if unbiased > 15 {
        return sign | 0x7c00;
    }
    if unbiased >= -14 {
        // Normal: 10 bits of mantissa, the 13 others rounded (a carry may raise the exponent,
        // up to infinity, which is the right rounding).
        let half = (((unbiased + 15) as u32) << 10) | (mantissa >> 13);
        let rest = mantissa & 0x1fff;
        let round = rest > 0x1000 || (rest == 0x1000 && half & 1 == 1);
        return sign | (half + u32::from(round)) as u16;
    }
    if unbiased < -25 {
        return sign;
    }
    // Subnormal: the implicit bit joins the mantissa, shifted down to 2^-24 units.
    let full = mantissa | 0x0080_0000;
    let shift = (-1 - unbiased) as u32; // 14 + 13 - (unbiased + 14) … between 14 and 24
    let half = full >> shift;
    let rest = full & ((1 << shift) - 1);
    let midpoint = 1 << (shift - 1);
    let round = rest > midpoint || (rest == midpoint && half & 1 == 1);
    sign | (half + u32::from(round)) as u16
}

/// A half-precision value (its bits) as single precision: exact.
pub fn f16_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = u32::from((bits >> 10) & 0x1f);
    let mantissa = u32::from(bits & 0x3ff);
    let magnitude = match exponent {
        // Subnormal: mantissa × 2^-24, exact in single precision.
        0 => (mantissa as f32 * (1.0 / 16_777_216.0)).to_bits(),
        0x1f => 0x7f80_0000 | (mantissa << 13),
        _ => ((exponent + 112) << 23) | (mantissa << 13),
    };
    f32::from_bits(sign | magnitude)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bfloat16_round_trips_and_rounds_to_even() {
        for v in [0.0f32, 1.0, -2.5, 3.140625, 65504.0, 1e-30, -1e30] {
            assert_eq!(bf16_to_f32(f32_to_bf16(round_bf16(v))), round_bf16(v));
        }
        // 1 + 2^-8 is halfway between 1 and 1 + 2^-7: ties go to the even one, 1.
        assert_eq!(round_bf16(1.0 + 1.0 / 256.0), 1.0);
        assert_eq!(round_bf16(1.0 + 3.0 / 256.0), 1.0 + 4.0 / 256.0);
        assert!(round_bf16(f32::NAN).is_nan());
    }

    #[test]
    fn half_precision_matches_its_definition() {
        // Every half value converts to single precision and back unchanged.
        for bits in 0..=u16::MAX {
            let v = f16_to_f32(bits);
            if v.is_nan() {
                assert!(f16_to_f32(f32_to_f16(v)).is_nan());
            } else {
                assert_eq!(f32_to_f16(v), bits, "{bits:#06x} = {v}");
            }
        }
        assert_eq!(f16_to_f32(f32_to_f16(1.0 + 1.0 / 2048.0)), 1.0); // tie to even
        assert_eq!(
            f16_to_f32(f32_to_f16(1.0 + 3.0 / 2048.0)),
            1.0 + 4.0 / 2048.0
        );
        assert_eq!(f16_to_f32(f32_to_f16(65520.0)), f32::INFINITY); // rounds past the largest
        assert_eq!(f16_to_f32(f32_to_f16(65519.0)), 65504.0);
        assert_eq!(f16_to_f32(f32_to_f16(1e-8)), 0.0);
        assert_eq!(f16_to_f32(f32_to_f16(3e-8)), 1.0 / 16_777_216.0); // nearest: 2^-24
        // Every bfloat16 weight converts exactly when it is within half precision's normal range.
        for v in [0.0078125f32, -0.5, 3.0, 1000.0] {
            let w = round_bf16(v);
            assert_eq!(f16_to_f32(f32_to_f16(w)), w);
        }
    }
}
