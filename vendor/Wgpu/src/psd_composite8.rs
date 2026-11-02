//! Photoshop 2026's compositing of 8-bit layers, as measured on documents
//! it composited itself: every step in whole levels, each product of two
//! levels divided by 255 and rounded half up.
//!
//! A layer's opacity and fill opacity combine first, then scale each
//! pixel's alpha, but for eight modes that take the fill inside the mode
//! instead (`fill_inside`). A blend mode gives its result from the source
//! and backdrop levels, and the result alpha is the two alphas' union. The
//! color takes one of two orders. Normal and the eight mix their result
//! with the plain source by the backdrop's alpha, then mix that into the
//! backdrop by the source's share of the union. The other modes mix their
//! result into the backdrop by the source's alpha, as over an opaque
//! backdrop, then mix the plain source into that by the share of the union
//! the source covers alone.

/// A product of levels over 255, rounded half up; `v` at least 0.
fn r(v: i32) -> i32 {
    (v + 127) / 255
}

/// `a / b` rounded half up; `a` at least 0, `b` above 0.
fn ratio(a: i32, b: i32) -> i32 {
    (2 * a + b) / (2 * b)
}

/// Whether the module composites a blend mode in whole levels: normal and
/// the separable modes, each measured on every pair of levels.
pub(crate) fn takes(mode: &[u8; 4]) -> bool {
    matches!(
        mode,
        b"norm" | b"dark" | b"lite" | b"mul " | b"scrn" | b"lbrn" | b"lddg" | b"diff" | b"smud" | b"fsub" | b"fdiv" | b"idiv" | b"div " | b"over" | b"hLit" | b"lLit" | b"hMix" | b"pLit" | b"vLit" | b"sLit"
    )
}

/// Vivid and pin light's doubled source below the middle: 2s scaled by
/// 255/256, rounded.
fn doubled_low(s: i32) -> i32 {
    (510 * s + 128) >> 8
}

/// Vivid and pin light's doubled source from the middle up: s - 128
/// stretched over 0 to 255, rounded.
fn doubled_high(s: i32) -> i32 {
    ratio((s - 128).max(0) * 255, 127)
}

/// Vivid light below the middle: color burn by the doubled source `low`.
fn vivid_burn(low: i32, d: i32) -> i32 {
    if low == 0 { 0 } else { (255 - ratio((255 - d) * 255, low)).max(0) }
}

/// Vivid light from the middle up: color dodge by the doubled source
/// `high`.
fn vivid_dodge(high: i32, d: i32) -> i32 {
    if high >= 255 {
        255
    } else if d == 0 {
        0
    } else {
        ratio(d * 255, 255 - high).min(255)
    }
}

/// Soft light's curve of the backdrop at the top half: the W3C curve, a
/// polynomial up to a quarter and the square root past it, held to whole
/// levels.
fn soft_curve(d: i32) -> i32 {
    let x = f64::from(d) / 255.0;
    let g = if x <= 0.25 { ((16.0 * x - 12.0) * x + 4.0) * x } else { x.sqrt() };
    (g * 255.0 + 0.5).floor() as i32
}

/// A blend mode's result for source level `s` over backdrop level `d`; a
/// mode `takes` does not take gives the source.
pub(crate) fn blend(mode: &[u8; 4], s: i32, d: i32) -> i32 {
    match mode {
        b"dark" => s.min(d),
        b"lite" => s.max(d),
        b"mul " => r(s * d),
        b"scrn" => 255 - r((255 - s) * (255 - d)),
        b"lbrn" => (s + d - 255).max(0),
        b"lddg" => (s + d).min(255),
        b"diff" => (s - d).abs(),
        b"smud" => s + d - 2 * r(s * d),
        b"fsub" => (d - s).max(0),
        b"fdiv" => {
            if d == 0 {
                0
            } else if s == 0 {
                255
            } else {
                ratio(d * 255, s).min(255)
            }
        }
        b"idiv" => {
            if d == 255 {
                255
            } else if s == 0 {
                0
            } else {
                (255 - ratio((255 - d) * 255, s)).max(0)
            }
        }
        b"div " => {
            if d == 0 {
                0
            } else if s == 255 {
                255
            } else {
                ratio(d * 255, 255 - s).min(255)
            }
        }
        b"over" => {
            if d < 128 {
                r(2 * s * d)
            } else {
                255 - r(2 * (255 - s) * (255 - d))
            }
        }
        b"hLit" => {
            if s < 128 {
                r(2 * s * d)
            } else {
                255 - r((511 - 2 * s) * (255 - d))
            }
        }
        b"lLit" => (d + 2 * s - 256).clamp(0, 255),
        b"hMix" => {
            if (s < 128 && s + d >= 255) || (s >= 128 && s + d > 255) {
                255
            } else {
                0
            }
        }
        b"pLit" => {
            if s < 128 {
                d.min(doubled_low(s))
            } else {
                d.max(doubled_high(s))
            }
        }
        b"vLit" => {
            if s < 128 {
                vivid_burn(doubled_low(s), d)
            } else {
                vivid_dodge(doubled_high(s), d)
            }
        }
        b"sLit" => {
            if s < 128 {
                d - r((255 - 2 * s) * r(d * (255 - d)))
            } else {
                d + r((2 * s - 256) * (soft_curve(d) - d))
            }
        }
        _ => s,
    }
}

/// Whether a blend mode takes a layer's fill opacity inside the mode,
/// moving the source toward the level the mode leaves the backdrop at,
/// rather than scaling the layer's alpha by it: color burn, linear burn,
/// color dodge, linear dodge, vivid light, linear light, hard mix and
/// difference.
fn fill_inside(mode: &[u8; 4]) -> bool {
    matches!(mode, b"idiv" | b"lbrn" | b"div " | b"lddg" | b"vLit" | b"lLit" | b"hMix" | b"diff")
}

/// The result of a mode `fill_inside` takes for source level `s` over
/// backdrop level `d` at fill opacity `fill`; at 255 it is `blend`'s. The
/// burns move the source toward 255 by the fill, the dodges and difference
/// toward 0, and vivid light each half's doubled source toward its own
/// mode's level. Linear light adds (2s - 255) scaled by the fill, less a
/// level. Hard mix is linear burn at the fill taken onto 0 to 254, then
/// stretched by 255 over what that leaves.
fn blend_at_fill(mode: &[u8; 4], s: i32, d: i32, fill: i32) -> i32 {
    let toward = |level: i32, neutral: i32| r(level * fill + neutral * (255 - fill));
    match mode {
        b"idiv" | b"lbrn" => blend(mode, toward(s, 255), d),
        b"div " | b"lddg" | b"diff" => blend(mode, toward(s, 0), d),
        b"vLit" => {
            if s < 128 {
                vivid_burn(toward(doubled_low(s), 255), d)
            } else {
                vivid_dodge(toward(doubled_high(s), 0), d)
            }
        }
        b"lLit" => (d + ((2 * s - 255) * fill - 255 + 127).div_euclid(255)).clamp(0, 255),
        b"hMix" => {
            let g = (255 * fill + 127) >> 8;
            ratio((d + r(s * g) - g).max(0) * 255, 255 - g).min(255)
        }
        _ => blend(mode, s, d),
    }
}

/// How a layer's pixels composite: its blend mode, the opacity in levels
/// its pixels' alpha is scaled by, and the fill opacity a mode
/// `fill_inside` takes moves the source by, 255 for any other mode.
pub(crate) struct Paint {
    mode: [u8; 4],
    k: i32,
    fill: i32,
}

impl Paint {
    /// A layer in blend mode `mode` at `opacity` and fill opacity `fill`,
    /// in levels: the two combined into the one opacity, or for a mode
    /// `fill_inside` takes, the opacity alone with the fill kept apart.
    pub(crate) fn new(mode: [u8; 4], opacity: u8, fill: u8) -> Paint {
        let (opacity, fill) = (i32::from(opacity), i32::from(fill));
        if fill_inside(&mode) { Paint { mode, k: opacity, fill } } else { Paint { mode, k: r(opacity * fill), fill: 255 } }
    }
}

/// Whether a blend mode's result is mixed into the backdrop by the source's
/// alpha before the source over nothing joins it: every separable mode but
/// color burn, linear burn, color dodge, linear dodge, vivid light, linear
/// light, hard mix and difference, which composite in normal's order.
fn result_into_backdrop_first(mode: &[u8; 4]) -> bool {
    matches!(mode, b"dark" | b"lite" | b"mul " | b"scrn" | b"over" | b"sLit" | b"hLit" | b"pLit" | b"smud" | b"fsub" | b"fdiv")
}

/// One pixel of a layer composited onto the canvas: the source's color
/// levels `source` at alpha `a` (in levels, before the layer's opacity),
/// painted as `paint` says, onto the backdrop's `color` and `alpha`, which
/// it updates.
pub(crate) fn over(paint: &Paint, source: &[i32], a: i32, color: &mut [i32], alpha: &mut i32) {
    let a = r(a * paint.k);
    if paint.fill < 255 {
        over_at_fill(paint, source, a, color, alpha);
        return;
    }
    if a == 0 {
        return;
    }
    let mode = &paint.mode;
    let b = *alpha;
    let t = b + a - r(b * a);
    if result_into_backdrop_first(mode) {
        let alone = ((255 * (t - b) + t / 2) / t).min(255);
        for (d, &s) in color.iter_mut().zip(source) {
            let into_backdrop = r(blend(mode, s, *d) * a + *d * (255 - a));
            *d = r(s * alone + into_backdrop * (255 - alone));
        }
    } else {
        let share = ((255 * a + t / 2) / t).min(255);
        for (d, &s) in color.iter_mut().zip(source) {
            let mixed = if b > 0 { r(blend(mode, s, *d) * b + s * (255 - b)) } else { s };
            *d = r(mixed * share + *d * (255 - share));
        }
    }
    *alpha = t;
}

/// A pixel of a layer in a mode `fill_inside` takes at a fill opacity below
/// 255, at alpha `a` after its opacity. The source over nothing covers that
/// alpha scaled by the fill; where the backdrop is, the mode's result at
/// the fill mixes into it by the alpha alone. The two mix by the shares of
/// the union they cover, then into the backdrop by their share of it. That
/// is Photoshop's composite exactly over an opaque backdrop and over
/// nothing, and within 2 levels of its stored composite and 1 of its alpha
/// where the backdrop is partly transparent.
fn over_at_fill(paint: &Paint, source: &[i32], a: i32, color: &mut [i32], alpha: &mut i32) {
    let b = *alpha;
    let alone = r(a * paint.fill);
    let overlap = r(a * b);
    let covered = overlap + alone - r(alone * b);
    if covered == 0 {
        return;
    }
    let t = b + alone - r(b * alone);
    let toward_result = ((255 * overlap + covered / 2) / covered).min(255);
    let share = ((255 * covered + t / 2) / t).min(255);
    for (d, &s) in color.iter_mut().zip(source) {
        let mixed = r(blend_at_fill(&paint.mode, s, *d, paint.fill) * toward_result + s * (255 - toward_result));
        *d = r(mixed * share + *d * (255 - share));
    }
    *alpha = t;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Normal compositing at full alpha replaces the backdrop; at half
    /// opacity over an opaque backdrop it mixes with the measured rounding.
    #[test]
    fn mixes_by_rounded_levels() {
        let (mut color, mut alpha) = (vec![10, 20, 30], 255);
        over(&Paint::new(*b"norm", 255, 255), &[200, 100, 50], 255, &mut color, &mut alpha);
        assert_eq!((color.clone(), alpha), (vec![200, 100, 50], 255));
        let half = Paint::new(*b"norm", 128, 255);
        assert_eq!(half.k, 128);
        over(&half, &[0, 0, 0], 255, &mut color, &mut alpha);
        assert_eq!(color, vec![r(200 * 127), r(100 * 127), r(50 * 127)]);
    }

    /// Two half-transparent layers over nothing: the alphas' rounded union,
    /// the color mixed by the source's rounded share.
    #[test]
    fn composites_over_transparency() {
        let (mut color, mut alpha) = (vec![255, 255, 255], 0);
        let normal = Paint::new(*b"norm", 255, 255);
        over(&normal, &[60, 160, 220], 128, &mut color, &mut alpha);
        over(&normal, &[255, 128, 0], 128, &mut color, &mut alpha);
        assert_eq!(alpha, 192);
        let matted: Vec<i32> = color.iter().map(|&c| r(c * alpha + 255 * (255 - alpha))).collect();
        assert_eq!(matted, vec![206, 168, 118]);
    }

    /// Fill opacity joins opacity for most modes and stays apart for the
    /// eight that take it inside.
    #[test]
    fn keeps_fill_apart_only_for_the_modes_that_take_it_inside() {
        let multiply = Paint::new(*b"mul ", 200, 77);
        assert_eq!((multiply.k, multiply.fill), (r(200 * 77), 255));
        let difference = Paint::new(*b"diff", 200, 77);
        assert_eq!((difference.k, difference.fill), (200, 77));
    }

    /// At full fill each mode that takes the fill inside gives the result
    /// measured for it opaque, hard mix's two thresholds among them, for
    /// every pair of levels.
    #[test]
    fn gives_the_opaque_results_at_full_fill() {
        for mode in [b"idiv", b"lbrn", b"div ", b"lddg", b"vLit", b"lLit", b"hMix", b"diff"] {
            for s in 0..=255 {
                for d in 0..=255 {
                    assert_eq!(blend_at_fill(mode, s, d, 255), blend(mode, s, d), "{} at {s}, {d}", String::from_utf8_lossy(mode));
                }
            }
        }
    }
}
