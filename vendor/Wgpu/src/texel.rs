//! Texels as colors on the CPU: the float formats mip levels and format
//! conversions render in, read back as RGBA colors, and colors packed
//! into the texels of any color format a shader reads as floating point.

use half::f16;
use pwrs::prelude::*;
use wgpu::TextureFormat as F;

use crate::error::invalid;
use crate::texture::format_name;

/// The float format `format`'s texels render in, with its channels: 16-bit
/// floats for formats of 10 bits a channel or fewer and for half floats,
/// 32-bit floats for 16-bit norm formats and floats. None for a format a
/// shader does not read as floating-point colors: integer, depth, stencil
/// and planar formats, and block-compressed ones, which decode first.
pub(crate) fn render_format(format: F) -> Option<F> {
    let wide = match format {
        F::R8Unorm
        | F::R8Snorm
        | F::Rg8Unorm
        | F::Rg8Snorm
        | F::Rgba8Unorm
        | F::Rgba8UnormSrgb
        | F::Rgba8Snorm
        | F::Bgra8Unorm
        | F::Bgra8UnormSrgb
        | F::Rgb10a2Unorm
        | F::Rg11b10Ufloat
        | F::Rgb9e5Ufloat
        | F::R16Float
        | F::Rg16Float
        | F::Rgba16Float => false,
        F::R16Unorm | F::R16Snorm | F::Rg16Unorm | F::Rg16Snorm | F::Rgba16Unorm | F::Rgba16Snorm | F::R32Float | F::Rg32Float | F::Rgba32Float => true,
        _ => return None,
    };
    Some(match (format.components(), wide) {
        (1, false) => F::R16Float,
        (2, false) => F::Rg16Float,
        (_, false) => F::Rgba16Float,
        (1, true) => F::R32Float,
        (2, true) => F::Rg32Float,
        (_, true) => F::Rgba32Float,
    })
}

/// The texels of `format`, a float format `render_format` names, as RGBA
/// colors, read as a shader reads them: a channel the format lacks is 0,
/// and a missing alpha 1.
pub(crate) fn colors(bytes: &[u8], format: F) -> PsResult<Vec<[f32; 4]>> {
    let values: Vec<f32> = match format {
        F::R16Float | F::Rg16Float | F::Rgba16Float => bytes.as_chunks::<2>().0.iter().map(|c| f16::from_le_bytes(*c).to_f32()).collect(),
        F::R32Float | F::Rg32Float | F::Rgba32Float => bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect(),
        other => return Err(invalid("WgpuTexels", format!("{} texels are not read as colors on the CPU", format_name(other)))),
    };
    let channels = format.components() as usize;
    Ok(values
        .chunks_exact(channels)
        .map(|c| {
            let mut rgba = [0.0, 0.0, 0.0, 1.0];
            rgba[..channels].copy_from_slice(c);
            rgba
        })
        .collect())
}

fn unorm8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn snorm8(v: f32) -> u8 {
    (v.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8
}

fn unorm16(v: f32) -> [u8; 2] {
    ((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_le_bytes()
}

fn snorm16(v: f32) -> [u8; 2] {
    ((v.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()
}

fn half(v: f32) -> [u8; 2] {
    f16::from_f32(v).to_le_bytes()
}

/// A linear value encoded as an 8-bit sRGB one.
fn srgb8(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let encoded = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    unorm8(encoded)
}

/// `v` as an unsigned float with a 5-bit exponent biased by 15 and
/// `mantissa` bits of mantissa: 6 for an 11-bit float, 5 for a 10-bit
/// one. Rounded to the nearest representable finite value, ties to even;
/// a negative value is 0, +Inf stays infinite and NaN stays NaN, as the
/// Vulkan specification converts them.
fn small_float(v: f32, mantissa: u32) -> u32 {
    let infinite = 0x1F << mantissa;
    if v.is_nan() {
        return infinite | 1;
    }
    if v == f32::INFINITY {
        return infinite;
    }
    if v.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return 0;
    }
    // The exponent `v` takes, held at that of the smallest normal value so
    // smaller values become subnormals, and `v` in units of the last
    // mantissa bit at that exponent.
    let exponent = (((v.to_bits() >> 23) & 0xFF) as i32 - 127).max(-14);
    let units = (f64::from(v) * 2f64.powi(mantissa as i32 - exponent)).round_ties_even() as u32;
    // A rounding that carries past the mantissa lands in the exponent.
    let bits = (((exponent + 14) as u32) << mantissa) + units;
    bits.min(infinite - 1)
}

/// An RGB color as the shared-exponent Rgb9e5Ufloat texel, as the Vulkan
/// specification encodes it: each channel clamped to 0 through the largest
/// value, NaN as 0, then rounded at the exponent the largest needs.
fn rgb9e5(c: [f32; 4]) -> u32 {
    const LARGEST: f32 = 511.0 / 512.0 * 65536.0;
    let clamp = |v: f32| if v.is_nan() { 0.0 } else { v.clamp(0.0, LARGEST) };
    let (r, g, b) = (clamp(c[0]), clamp(c[1]), clamp(c[2]));
    let top = r.max(g).max(b);
    let floor_log2 = if top > 0.0 { ((top.to_bits() >> 23) & 0xFF) as i32 - 127 } else { -16 };
    let mut shared = floor_log2.max(-16) + 16;
    let scale = |exp: i32| 2f64.powi(exp - 24);
    let at = |v: f32, exp: i32| (f64::from(v) / scale(exp) + 0.5).floor() as u32;
    if at(top, shared) == 512 {
        shared += 1;
    }
    at(r, shared) | (at(g, shared) << 9) | (at(b, shared) << 18) | ((shared as u32) << 27)
}

/// `colors` packed into texels of `format`, any color format a shader
/// reads as floating point: each channel the format has, rounded to
/// nearest, clamped to the range a norm format holds, sRGB-encoded for an
/// sRGB format.
pub(crate) fn pack(colors: &[[f32; 4]], format: F) -> PsResult<Vec<u8>> {
    let size = format.block_copy_size(None).ok_or_else(|| invalid("WgpuTexels", format!("{} has no texel layout", format_name(format))))? as usize;
    let mut out = Vec::with_capacity(colors.len() * size);
    for &c in colors {
        match format {
            F::R8Unorm => out.push(unorm8(c[0])),
            F::Rg8Unorm => out.extend([unorm8(c[0]), unorm8(c[1])]),
            F::Rgba8Unorm => out.extend(c.map(unorm8)),
            F::Rgba8UnormSrgb => out.extend([srgb8(c[0]), srgb8(c[1]), srgb8(c[2]), unorm8(c[3])]),
            F::Bgra8Unorm => out.extend([unorm8(c[2]), unorm8(c[1]), unorm8(c[0]), unorm8(c[3])]),
            F::Bgra8UnormSrgb => out.extend([srgb8(c[2]), srgb8(c[1]), srgb8(c[0]), unorm8(c[3])]),
            F::R8Snorm => out.push(snorm8(c[0])),
            F::Rg8Snorm => out.extend([snorm8(c[0]), snorm8(c[1])]),
            F::Rgba8Snorm => out.extend(c.map(snorm8)),
            F::R16Unorm => out.extend(unorm16(c[0])),
            F::Rg16Unorm => out.extend(c[..2].iter().flat_map(|v| unorm16(*v))),
            F::Rgba16Unorm => out.extend(c.iter().flat_map(|v| unorm16(*v))),
            F::R16Snorm => out.extend(snorm16(c[0])),
            F::Rg16Snorm => out.extend(c[..2].iter().flat_map(|v| snorm16(*v))),
            F::Rgba16Snorm => out.extend(c.iter().flat_map(|v| snorm16(*v))),
            F::R16Float => out.extend(half(c[0])),
            F::Rg16Float => out.extend(c[..2].iter().flat_map(|v| half(*v))),
            F::Rgba16Float => out.extend(c.iter().flat_map(|v| half(*v))),
            F::R32Float => out.extend(c[0].to_le_bytes()),
            F::Rg32Float => out.extend(c[..2].iter().flat_map(|v| v.to_le_bytes())),
            F::Rgba32Float => out.extend(c.iter().flat_map(|v| v.to_le_bytes())),
            F::Rgb10a2Unorm => {
                let q = |v: f32, max: f32| (v.clamp(0.0, 1.0) * max).round() as u32;
                out.extend((q(c[0], 1023.0) | (q(c[1], 1023.0) << 10) | (q(c[2], 1023.0) << 20) | (q(c[3], 3.0) << 30)).to_le_bytes());
            }
            F::Rg11b10Ufloat => out.extend((small_float(c[0], 6) | (small_float(c[1], 6) << 11) | (small_float(c[2], 5) << 22)).to_le_bytes()),
            F::Rgb9e5Ufloat => out.extend(rgb9e5(c).to_le_bytes()),
            other => return Err(invalid("WgpuTexels", format!("{} texels are not packed from colors on the CPU", format_name(other)))),
        }
    }
    Ok(out)
}

/// Whether every texel of `format` in `bytes` has full alpha: true for a
/// format with no alpha channel.
pub(crate) fn opaque(bytes: &[u8], format: F) -> bool {
    match format {
        F::Rgba8Unorm | F::Rgba8UnormSrgb | F::Bgra8Unorm | F::Bgra8UnormSrgb => bytes.as_chunks::<4>().0.iter().all(|t| t[3] == 255),
        F::Rgba8Snorm => bytes.as_chunks::<4>().0.iter().all(|t| t[3] as i8 == 127),
        F::Rgb10a2Unorm => bytes.as_chunks::<4>().0.iter().all(|t| t[3] >> 6 == 3),
        F::Rgba16Unorm => bytes.as_chunks::<8>().0.iter().all(|t| u16::from_le_bytes([t[6], t[7]]) == u16::MAX),
        F::Rgba16Snorm => bytes.as_chunks::<8>().0.iter().all(|t| i16::from_le_bytes([t[6], t[7]]) == i16::MAX),
        F::Rgba16Float => bytes.as_chunks::<8>().0.iter().all(|t| f16::from_le_bytes([t[6], t[7]]).to_f32() >= 1.0),
        F::Rgba32Float => bytes.as_chunks::<16>().0.iter().all(|t| f32::from_le_bytes([t[12], t[13], t[14], t[15]]) >= 1.0),
        other => other.components() < 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An 11-bit float by the Vulkan specification: 5 exponent bits biased
    /// by 15 above 6 mantissa bits, subnormal at exponent 0.
    fn eleven(bits: u32) -> f64 {
        let (e, m) = (bits >> 6, f64::from(bits & 0x3F));
        if e == 0 { m / 64.0 * 2f64.powi(-14) } else { (1.0 + m / 64.0) * 2f64.powi(e as i32 - 15) }
    }

    #[test]
    fn packs_eleven_bit_floats_to_the_nearest_value() {
        assert_eq!(small_float(1.0, 6), 15 << 6);
        assert_eq!(small_float(0.0, 6), 0);
        assert_eq!(small_float(-3.0, 6), 0);
        assert_eq!(small_float(f32::INFINITY, 6), 0x1F << 6);
        assert_eq!(small_float(f32::NAN, 6) >> 6, 0x1F);
        assert_ne!(small_float(f32::NAN, 6) & 0x3F, 0);
        // The largest finite 11-bit float, 65024, also takes everything past it.
        assert_eq!(small_float(1e9, 6), (0x1E << 6) | 0x3F);
        assert_eq!(eleven(small_float(1e9, 6)), 65024.0);
        // Every value lands on the nearest representable one.
        for i in 0..20_000 {
            let v = (i as f32 * 0.002).exp2() * 1e-7;
            if f64::from(v) > 65024.0 {
                break;
            }
            let bits = small_float(v, 6);
            let got = eleven(bits);
            let next_up = eleven(bits + 1);
            let next_down = if bits == 0 { 0.0 } else { eleven(bits - 1) };
            let err = (got - f64::from(v)).abs();
            assert!(err <= (next_up - f64::from(v)).abs() && err <= (next_down - f64::from(v)).abs(), "{v} packed to {got}");
        }
        // Subnormals: the smallest is 2^-20, and half of it ties to even, 0.
        assert_eq!(small_float(2f32.powi(-20), 6), 1);
        assert_eq!(small_float(2f32.powi(-21), 6), 0);
        assert_eq!(small_float(3.0 * 2f32.powi(-21), 6), 2);
    }

    /// The Vulkan specification's worked bounds of RGB9E5: the largest
    /// value, a value that needs the next exponent after rounding, and
    /// zero.
    #[test]
    fn packs_shared_exponent_texels() {
        let decode = |bits: u32| {
            let e = (bits >> 27) as i32;
            let scale = 2f64.powi(e - 24);
            [(bits & 0x1FF) as f64 * scale, ((bits >> 9) & 0x1FF) as f64 * scale, ((bits >> 18) & 0x1FF) as f64 * scale]
        };
        assert_eq!(decode(rgb9e5([1e9, 0.0, -1.0, 1.0])), [65408.0, 0.0, 0.0]);
        assert_eq!(rgb9e5([0.0, 0.0, 0.0, 1.0]), 0);
        assert_eq!(decode(rgb9e5([1.0, 0.5, 0.25, 1.0])), [1.0, 0.5, 0.25]);
        // 511.75 rounds to 512 units at the exponent its leading bit gives,
        // past what 9 mantissa bits hold, so it takes the next exponent:
        // 256 units of 2, biased exponent 25.
        let bits = rgb9e5([511.75, 0.0, 0.0, 1.0]);
        assert_eq!(bits >> 27, 25);
        assert_eq!(decode(bits)[0], 512.0);
    }

    #[test]
    fn packs_every_channel_layout() {
        let c = [[1.0, -1.0, 0.5, 0.25]];
        assert_eq!(pack(&c, F::Rgba8Unorm).unwrap(), [255, 0, 128, 64]);
        assert_eq!(pack(&c, F::Bgra8Unorm).unwrap(), [128, 0, 255, 64]);
        assert_eq!(pack(&c, F::Rgba8Snorm).unwrap(), [127, 0x81, 64, 32]);
        assert_eq!(pack(&c, F::Rg8Snorm).unwrap(), [127, 0x81]);
        assert_eq!(pack(&c, F::R16Unorm).unwrap(), [0xFF, 0xFF]);
        assert_eq!(pack(&c, F::Rg16Snorm).unwrap(), [0xFF, 0x7F, 0x01, 0x80]);
        assert_eq!(pack(&c, F::Rgba16Float).unwrap(), [0x00, 0x3C, 0x00, 0xBC, 0x00, 0x38, 0x00, 0x34]);
        assert_eq!(pack(&c, F::Rg32Float).unwrap(), [0, 0, 0x80, 0x3F, 0, 0, 0x80, 0xBF]);
        assert_eq!(pack(&c, F::Rgb10a2Unorm).unwrap(), (1023u32 | (512 << 20) | (1 << 30)).to_le_bytes());
        // sRGB: linear 0.5 encodes as 188, and alpha stays linear.
        assert_eq!(pack(&[[0.5, 0.0, 1.0, 0.5]], F::Rgba8UnormSrgb).unwrap(), [188, 0, 255, 128]);
        assert!(pack(&c, F::Rgba8Uint).is_err());
    }

    #[test]
    fn reads_float_texels_as_colors_with_missing_channels_filled() {
        let r: Vec<u8> = [0.5f32, -2.0].iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(colors(&r, F::R32Float).unwrap(), [[0.5, 0.0, 0.0, 1.0], [-2.0, 0.0, 0.0, 1.0]]);
        let rg: Vec<u8> = [0.25f32, 0.75].iter().flat_map(|v| f16::from_f32(*v).to_le_bytes()).collect();
        assert_eq!(colors(&rg, F::Rg16Float).unwrap(), [[0.25, 0.75, 0.0, 1.0]]);
    }

    #[test]
    fn tells_full_alpha_from_any_less() {
        assert!(opaque(&[1, 2, 3, 255, 4, 5, 6, 255], F::Rgba8Unorm));
        assert!(!opaque(&[1, 2, 3, 255, 4, 5, 6, 254], F::Rgba8Unorm));
        assert!(opaque(&[9, 9], F::Rg8Unorm));
        assert!(opaque(&[0, 0, 0, 0xC0], F::Rgb10a2Unorm));
        assert!(!opaque(&[0, 0, 0, 0x80], F::Rgb10a2Unorm));
    }

    #[test]
    fn renders_each_format_through_a_float_format_that_holds_it() {
        assert_eq!(render_format(F::R8Unorm), Some(F::R16Float));
        assert_eq!(render_format(F::Rg8Snorm), Some(F::Rg16Float));
        assert_eq!(render_format(F::Rgb9e5Ufloat), Some(F::Rgba16Float));
        assert_eq!(render_format(F::R16Unorm), Some(F::R32Float));
        assert_eq!(render_format(F::Rgba32Float), Some(F::Rgba32Float));
        assert_eq!(render_format(F::Rgba8Uint), None);
        assert_eq!(render_format(F::Depth32Float), None);
    }
}
