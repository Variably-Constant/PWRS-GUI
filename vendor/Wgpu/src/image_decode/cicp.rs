//! A file's CICP values, from an AVIF's colr box or a JPEG XL's color
//! encoding: as a profile for the module's conversion, and the light of a
//! PQ or HLG transfer as linear light in sRGB's primaries.

use image::DynamicImage;
use moxcms::{adapt_to_illuminant_d, CicpColorPrimaries, CicpProfile, ColorPrimaries, ColorProfile, TransferCharacteristics, XyY};

use super::{filled, rgb_to_xyz, with_alpha, with_colorants, D65};

/// `a` times `b`.
fn product(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    [0, 1, 2].map(|i| [0, 1, 2].map(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

/// The transfer of an HDR file's CICP values.
#[derive(Clone, Copy)]
pub(super) enum HdrTransfer {
    /// SMPTE ST 2084's perceptual quantizer, whose signal is absolute light
    /// up to 10,000 nits.
    Pq,
    /// BT.2100's hybrid log-gamma, whose signal is scene light.
    Hlg,
}

impl HdrTransfer {
    pub(super) fn of(cicp: &CicpProfile) -> Option<HdrTransfer> {
        match cicp.transfer_characteristics {
            TransferCharacteristics::Smpte2084 => Some(HdrTransfer::Pq),
            TransferCharacteristics::Hlg => Some(HdrTransfer::Hlg),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            HdrTransfer::Pq => "PQ",
            HdrTransfer::Hlg => "HLG",
        }
    }
}

/// BT.2408's HDR reference white, in nits: the light an HDR file's pixels
/// load as 1 at.
const REFERENCE_WHITE: f64 = 203.0;

/// The display peak, in nits, of BT.2100's reference display for HLG,
/// where its system gamma is 1.2.
const HLG_PEAK: f64 = 1000.0;

/// The light, in nits, a PQ signal from 0 to 1 means, as SMPTE ST 2084's
/// EOTF gives it.
fn pq_nits(signal: f64) -> f64 {
    let (m1, m2): (f64, f64) = (2610.0 / 16384.0, 2523.0 / 4096.0 * 128.0);
    let (c1, c2, c3): (f64, f64, f64) = (3424.0 / 4096.0, 2413.0 / 4096.0 * 32.0, 2392.0 / 4096.0 * 32.0);
    let p = signal.clamp(0.0, 1.0).powf(1.0 / m2);
    10000.0 * ((p - c1).max(0.0) / (c2 - c3 * p)).powf(1.0 / m1)
}

/// The scene light, from 0 to 1, of an HLG signal from 0 to 1, as BT.2100's
/// inverse OETF gives it.
fn hlg_scene(signal: f64) -> f64 {
    let (a, b, c): (f64, f64, f64) = (0.178_832_77, 0.284_668_92, 0.559_910_73);
    let e = signal.clamp(0.0, 1.0);
    if e <= 0.5 { e * e / 3.0 } else { (((e - c) / a).exp() + b) / 12.0 }
}

/// An HDR file's pixels as linear light in sRGB's primaries, with
/// BT.2408's HDR reference white of 203 nits at 1: PQ's nits as ST 2084
/// gives them, HLG's through BT.2100's inverse OETF and its OOTF for a
/// display of 1,000 nits, where BT.2408 puts that white at 75 percent of
/// the signal. Light in colors sRGB lacks is clipped at 0, and light above
/// the reference white is kept. The transfer's name comes with them.
pub(super) fn hdr_linear(image: &DynamicImage, cicp: &CicpProfile) -> Result<(DynamicImage, &'static str), String> {
    use DynamicImage as D;
    let transfer = HdrTransfer::of(cicp).ok_or("its CICP transfer is neither PQ nor HLG")?;
    let primaries = ColorPrimaries::try_from(cicp.color_primaries).map_err(|e| format!("its CICP primaries are not ones moxcms knows: {e}"))?;
    let white = cicp_white(cicp.color_primaries).ok_or("its CICP primaries name no white point")?;
    let to_xyz = rgb_to_xyz(primaries, white);
    // Light under a white other than sRGB's is adapted to sRGB's by Bradford.
    let to_d65 = adapt_to_illuminant_d(to_xyz, white, D65.to_xyz());
    let to_srgb = product(&rgb_to_xyz(ColorPrimaries::BT_709, D65).inverse().v, &to_d65.v);
    let luminance = to_xyz.v[1];
    let light = |signal: [f64; 3]| -> [f32; 3] {
        let nits = match transfer {
            HdrTransfer::Pq => signal.map(pq_nits),
            HdrTransfer::Hlg => {
                let scene = signal.map(hlg_scene);
                let y = (0..3).map(|i| luminance[i] * scene[i]).sum::<f64>().max(0.0);
                // BT.2100's OOTF: the peak times the scene light, scaled by
                // its luminance to the power of the system gamma, 1.2, less 1.
                let gain = HLG_PEAK * y.powf(0.2);
                scene.map(|e| e * gain)
            }
        };
        to_srgb.map(|row| ((row[0] * nits[0] + row[1] * nits[1] + row[2] * nits[2]) / REFERENCE_WHITE).max(0.0) as f32)
    };
    let rgb = |p: &[u16]| light([0, 1, 2].map(|i| f64::from(p[i]) / 65535.0));
    let (w, h) = (image.width(), image.height());
    let converted = match image {
        D::ImageRgb16(b) => D::ImageRgb32F(filled(w, h, b.as_raw().as_chunks::<3>().0.iter().flat_map(|p| rgb(p)).collect())?),
        other => {
            let rgba = other.to_rgba16();
            D::ImageRgba32F(filled(w, h, rgba.as_raw().as_chunks::<4>().0.iter().flat_map(|p| with_alpha(rgb(p), f32::from(p[3]) / 65535.0)).collect())?)
        }
    };
    Ok((converted, transfer.name()))
}

/// The white of a CICP code's primaries, as H.273 gives it, or `None` for
/// a code that names none.
fn cicp_white(primaries: CicpColorPrimaries) -> Option<XyY> {
    use CicpColorPrimaries as P;
    match primaries {
        P::Bt709 | P::Bt470Bg | P::Bt601 | P::Smpte240 | P::Bt2020 | P::Smpte432 | P::Ebu3213 => Some(D65),
        P::Bt470M | P::GenericFilm => Some(XyY { x: 0.310, y: 0.316, yb: 1.0 }),
        P::Xyz => Some(XyY { x: 1.0 / 3.0, y: 1.0 / 3.0, yb: 1.0 }),
        P::Smpte431 => Some(XyY { x: 0.314, y: 0.351, yb: 1.0 }),
        P::Reserved | P::Unspecified => None,
    }
}

/// CICP values as a profile, its colorants built as the sRGB the conversion
/// goes to is, so a file that says it is sRGB keeps its bytes.
pub(super) fn cicp_profile(cicp: CicpProfile) -> ColorProfile {
    let profile = ColorProfile::new_from_cicp(cicp);
    match (ColorPrimaries::try_from(cicp.color_primaries), cicp_white(cicp.color_primaries)) {
        (Ok(primaries), Some(white)) => with_colorants(profile, primaries, white),
        _ => profile,
    }
}
