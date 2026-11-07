//! Palette images: a picture's colors brought to a palette of at most 256
//! entries, exactly where it has no more colors than the palette holds, and
//! otherwise by NeuQuant or Heckbert's median cut; each pixel then takes its
//! nearest entry, the difference spread by error diffusion or an ordered
//! Bayer pattern as asked, or not at all.

use std::collections::{BTreeMap, BTreeSet};

use pwrs::prelude::*;

/// How a palette image spreads the difference between a pixel's color and
/// its palette color: by error diffusion, which passes each pixel's error
/// on to the pixels after it, by an ordered pattern, which adds a fixed
/// threshold to each pixel before it takes its nearest color, or not at all.
#[psenum(name = "Wgpu.DitherMethod")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DitherMethod {
    /// Floyd and Steinberg's error diffusion, to four neighbors.
    #[default]
    FloydSteinberg,
    /// Jarvis, Judice and Ninke's, to twelve neighbors in this row and the
    /// two below.
    JarvisJudiceNinke,
    /// Stucki's, to the same twelve neighbors, weighed more toward the
    /// pixel.
    Stucki,
    /// Atkinson's, which passes three quarters of the error to six
    /// neighbors.
    Atkinson,
    /// Burkes's, to seven neighbors in this row and the next.
    Burkes,
    /// Sierra's three-row filter, to ten neighbors.
    Sierra,
    /// Sierra's two-row filter, to seven neighbors.
    TwoRowSierra,
    /// Sierra's lite filter, to three neighbors.
    SierraLite,
    /// An ordered pattern of Bayer's 2x2 matrix.
    Bayer2x2,
    /// An ordered pattern of Bayer's 4x4 matrix.
    Bayer4x4,
    /// An ordered pattern of Bayer's 8x8 matrix.
    Bayer8x8,
    /// An ordered pattern of Bayer's 16x16 matrix.
    Bayer16x16,
    /// No dithering: each pixel takes its nearest palette color.
    None,
}

/// How a palette image finds its palette when the picture has more colors
/// than the palette holds.
#[psenum(name = "Wgpu.Quantizer")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Quantizer {
    /// Median cut for a palette of fewer than 64 colors, NeuQuant for one
    /// of 64 or more.
    #[default]
    Auto,
    /// Dekker's NeuQuant, a neural network trained on the pixels, which
    /// keeps gradients smooth in a large palette.
    NeuQuant,
    /// Heckbert's median cut: the box of colors with the widest range is
    /// cut at its median pixel until there are as many boxes as colors, and
    /// each gives its mean.
    MedianCut,
}

/// Floyd and Steinberg's error-diffusion kernel and the others, as
/// hitherdither lists them: each neighbor across and down from the pixel,
/// and the share of its error the neighbor takes.
const FLOYD_STEINBERG: [(i64, i64, f64); 4] = [(1, 0, 7.0 / 16.0), (-1, 1, 3.0 / 16.0), (0, 1, 5.0 / 16.0), (1, 1, 1.0 / 16.0)];
const JARVIS_JUDICE_NINKE: [(i64, i64, f64); 12] = [
    (1, 0, 7.0 / 48.0),
    (2, 0, 5.0 / 48.0),
    (-2, 1, 3.0 / 48.0),
    (-1, 1, 5.0 / 48.0),
    (0, 1, 7.0 / 48.0),
    (1, 1, 5.0 / 48.0),
    (2, 1, 3.0 / 48.0),
    (-2, 2, 1.0 / 48.0),
    (-1, 2, 3.0 / 48.0),
    (0, 2, 5.0 / 48.0),
    (1, 2, 3.0 / 48.0),
    (2, 2, 1.0 / 48.0),
];
const STUCKI: [(i64, i64, f64); 12] = [
    (1, 0, 8.0 / 42.0),
    (2, 0, 4.0 / 42.0),
    (-2, 1, 2.0 / 42.0),
    (-1, 1, 4.0 / 42.0),
    (0, 1, 8.0 / 42.0),
    (1, 1, 4.0 / 42.0),
    (2, 1, 2.0 / 42.0),
    (-2, 2, 1.0 / 42.0),
    (-1, 2, 2.0 / 42.0),
    (0, 2, 4.0 / 42.0),
    (1, 2, 2.0 / 42.0),
    (2, 2, 1.0 / 42.0),
];
const ATKINSON: [(i64, i64, f64); 6] = [(1, 0, 1.0 / 8.0), (2, 0, 1.0 / 8.0), (-1, 1, 1.0 / 8.0), (0, 1, 1.0 / 8.0), (1, 1, 1.0 / 8.0), (0, 2, 1.0 / 8.0)];
const BURKES: [(i64, i64, f64); 7] = [(1, 0, 8.0 / 32.0), (2, 0, 4.0 / 32.0), (-2, 1, 2.0 / 32.0), (-1, 1, 4.0 / 32.0), (0, 1, 8.0 / 32.0), (1, 1, 4.0 / 32.0), (2, 1, 2.0 / 32.0)];
const SIERRA: [(i64, i64, f64); 10] = [
    (1, 0, 5.0 / 32.0),
    (2, 0, 3.0 / 32.0),
    (-2, 1, 2.0 / 32.0),
    (-1, 1, 4.0 / 32.0),
    (0, 1, 5.0 / 32.0),
    (1, 1, 4.0 / 32.0),
    (2, 1, 2.0 / 32.0),
    (-1, 2, 2.0 / 32.0),
    (0, 2, 3.0 / 32.0),
    (1, 2, 2.0 / 32.0),
];
const TWO_ROW_SIERRA: [(i64, i64, f64); 7] = [(1, 0, 4.0 / 16.0), (2, 0, 3.0 / 16.0), (-2, 1, 1.0 / 16.0), (-1, 1, 2.0 / 16.0), (0, 1, 3.0 / 16.0), (1, 1, 2.0 / 16.0), (2, 1, 1.0 / 16.0)];
const SIERRA_LITE: [(i64, i64, f64); 3] = [(1, 0, 2.0 / 4.0), (-1, 1, 1.0 / 4.0), (0, 1, 1.0 / 4.0)];

impl DitherMethod {
    /// An error-diffusion method's kernel.
    fn kernel(self) -> Option<&'static [(i64, i64, f64)]> {
        use DitherMethod as D;
        Some(match self {
            D::FloydSteinberg => &FLOYD_STEINBERG,
            D::JarvisJudiceNinke => &JARVIS_JUDICE_NINKE,
            D::Stucki => &STUCKI,
            D::Atkinson => &ATKINSON,
            D::Burkes => &BURKES,
            D::Sierra => &SIERRA,
            D::TwoRowSierra => &TWO_ROW_SIERRA,
            D::SierraLite => &SIERRA_LITE,
            D::Bayer2x2 | D::Bayer4x4 | D::Bayer8x8 | D::Bayer16x16 | D::None => return None,
        })
    }

    /// The side of an ordered method's Bayer matrix.
    fn bayer(self) -> Option<usize> {
        match self {
            DitherMethod::Bayer2x2 => Some(2),
            DitherMethod::Bayer4x4 => Some(4),
            DitherMethod::Bayer8x8 => Some(8),
            DitherMethod::Bayer16x16 => Some(16),
            _ => None,
        }
    }
}

/// Bayer's index matrix of side `n`, a power of 2 from 2, rows from the
/// top: [[0, 2], [3, 1]], and each larger one four copies of the one half
/// its side, times 4, plus 0, 2, 3 and 1, as hitherdither builds it.
fn bayer_index(n: usize) -> Vec<u32> {
    let mut index = vec![0, 2, 3, 1];
    let mut side = 2;
    while side < n {
        let next = side * 2;
        let mut out = vec![0; next * next];
        for y in 0..side {
            for x in 0..side {
                let v = 4 * index[y * side + x];
                out[y * next + x] = v;
                out[y * next + x + side] = v + 2;
                out[(y + side) * next + x] = v + 3;
                out[(y + side) * next + x + side] = v + 1;
            }
        }
        index = out;
        side = next;
    }
    index
}

/// The spread of an ordered pattern for a palette of `colors`: the step
/// between the levels of each channel of a uniform palette of as many
/// colors, the cube root of `colors` and at least 2 of them.
fn ordered_spread(colors: usize) -> f64 {
    let levels = (colors as f64).cbrt().max(2.0);
    255.0 / (levels - 1.0)
}

/// The entry of `palette` nearest `pixel`, by the distance across its
/// first `channels` samples; the first of entries as near.
fn nearest(pixel: &[f64], palette: &[[f64; 4]], channels: usize) -> usize {
    let mut best = (0, f64::INFINITY);
    for (at, color) in palette.iter().enumerate() {
        let mut distance = 0.0;
        for c in 0..channels {
            let d = pixel[c] - color[c];
            distance += d * d;
        }
        if distance < best.1 {
            best = (at, distance);
        }
    }
    best.0
}

/// How a picture is dithered: by `method` at `amount`, an ordered pattern
/// spread over `spread` sample values.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Dithering {
    method: DitherMethod,
    amount: f64,
    spread: f64,
}

/// Each pixel's palette entry, of `samples` `channels` to a pixel and
/// `width` pixels to a row, as `dithering` says. Error diffusion runs as
/// hitherdither runs it: in rows from the top, each from the left, each
/// pixel clipped to 0 to 255 before it takes its nearest entry and its
/// error passed on. An ordered pattern adds its value, centered on 0, to
/// each pixel. A pixel `skipped` marks takes entry 0, passes no error on
/// and drops what it is given.
fn entries(samples: &[f64], width: usize, channels: usize, palette: &[[f64; 4]], dithering: Dithering, skipped: Option<&[bool]>) -> Vec<u8> {
    let Dithering { method, amount, spread } = dithering;
    let count = samples.len() / channels;
    let height = count / width.max(1);
    let skip = |at: usize| skipped.is_some_and(|marks| marks[at]);
    let mut out = vec![0u8; count];
    if let Some(kernel) = method.kernel() {
        let mut values = samples.to_vec();
        for y in 0..height {
            for x in 0..width {
                let at = y * width + x;
                if skip(at) {
                    continue;
                }
                let pixel = &mut values[at * channels..(at + 1) * channels];
                for v in pixel.iter_mut() {
                    *v = v.clamp(0.0, 255.0);
                }
                let entry = nearest(pixel, palette, channels);
                out[at] = entry as u8;
                let mut error = [0.0; 4];
                for c in 0..channels {
                    error[c] = pixel[c] - palette[entry][c];
                }
                for &(dx, dy, share) in kernel {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                        continue;
                    }
                    let next = ny as usize * width + nx as usize;
                    for c in 0..channels {
                        values[next * channels + c] += error[c] * share * amount;
                    }
                }
            }
        }
        return out;
    }
    let pattern = method.bayer().map(|n| (n, bayer_index(n)));
    for at in 0..count {
        if skip(at) {
            continue;
        }
        let pixel = &samples[at * channels..(at + 1) * channels];
        let entry = match &pattern {
            Some((n, index)) => {
                let (x, y) = (at % width, at / width);
                let b = (1.0 + f64::from(index[(y % n) * n + x % n])) / (1.0 + (n * n) as f64);
                let offset = (b - 0.5) * spread * amount;
                let mut shifted = [0.0; 4];
                for (to, v) in shifted.iter_mut().zip(pixel) {
                    *to = v + offset;
                }
                nearest(&shifted, palette, channels)
            }
            None => nearest(pixel, palette, channels),
        };
        out[at] = entry as u8;
    }
    out
}

/// What a palette image is asked for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Paletting {
    /// The most colors the palette holds, from 2 to 256.
    pub(crate) colors: usize,
    pub(crate) quantizer: Quantizer,
    pub(crate) dither: DitherMethod,
    /// How much of the dithering is applied, from 0 to 1.
    pub(crate) amount: f64,
    /// Makes each pixel opaque or transparent, as a GIF's are: transparent
    /// below alpha 128, and opaque from it.
    pub(crate) binary_alpha: bool,
}

/// How a palette was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// The pixels' own colors, as few as the palette holds.
    Exact,
    NeuQuant,
    MedianCut,
}

/// Frames brought to one palette.
pub(crate) struct Paletted {
    /// The palette's colors, RGBA: a GIF's transparent entry, where it has
    /// one, last.
    pub(crate) palette: Vec<[u8; 4]>,
    /// Each frame's pixels as palette entries, rows from the top.
    pub(crate) frames: Vec<Vec<u8>>,
    /// The entry a GIF's transparent pixels take.
    pub(crate) transparent: Option<u8>,
    pub(crate) found: Found,
}

/// `frames`, RGBA of 8 bits a sample and `width` pixels to a row, brought to
/// one palette as `asked` says: of their own colors where they have no more
/// than it holds, a GIF's transparent entry counted among them, and
/// otherwise found by the quantizer asked for, each pixel then taking its
/// entry by the dithering asked for. A GIF's pixels of alpha below 128 take
/// its transparent entry, and the rest are opaque.
pub(crate) fn paletted(frames: &[&[u8]], width: usize, asked: &Paletting) -> Paletted {
    let channels = if asked.binary_alpha { 3 } else { 4 };
    let opaque = |pixel: &[u8]| !asked.binary_alpha || pixel[3] >= 128;
    let color_of = |pixel: &[u8]| if asked.binary_alpha { [pixel[0], pixel[1], pixel[2], 255] } else { [pixel[0], pixel[1], pixel[2], pixel[3]] };
    let pixels = || frames.iter().flat_map(|frame| frame.as_chunks::<4>().0.iter());
    let any_transparent = pixels().any(|pixel| !opaque(pixel));
    let room = asked.colors - usize::from(any_transparent);
    let mut distinct = BTreeSet::new();
    for pixel in pixels().filter(|pixel| opaque(pixel.as_slice())) {
        distinct.insert(color_of(pixel));
        if distinct.len() > room {
            break;
        }
    }
    let (mut palette, found) = if distinct.len() <= room {
        (distinct.into_iter().collect::<Vec<_>>(), Found::Exact)
    } else {
        let median = match asked.quantizer {
            Quantizer::Auto => asked.colors < 64,
            Quantizer::MedianCut => true,
            Quantizer::NeuQuant => false,
        };
        if median {
            (median_cut(pixels().filter(|pixel| opaque(pixel.as_slice())).map(|pixel| color_of(pixel)), room, channels), Found::MedianCut)
        } else {
            let samples: Vec<u8> = pixels().filter(|pixel| opaque(pixel.as_slice())).flat_map(|pixel| color_of(pixel)).collect();
            let network = color_quant::NeuQuant::new(1, room, &samples);
            let map = network.color_map_rgba();
            (map.as_chunks::<4>().0.iter().map(|&[r, g, b, a]| if asked.binary_alpha { [r, g, b, 255] } else { [r, g, b, a] }).collect(), Found::NeuQuant)
        }
    };
    let entries_of: Vec<[f64; 4]> = palette.iter().map(|color| color.map(f64::from)).collect();
    let spread = ordered_spread(asked.colors);
    let transparent = any_transparent.then_some(palette.len() as u8);
    // An exact palette holds every pixel's color, so the nearest entry is
    // the pixel's own and there is no error to dither.
    let dither = if found == Found::Exact { DitherMethod::None } else { asked.dither };
    let mapped = frames
        .iter()
        .map(|frame| {
            let marks: Vec<bool> = frame.as_chunks::<4>().0.iter().map(|pixel| !opaque(pixel)).collect();
            let mut samples = Vec::with_capacity(frame.len() / 4 * channels);
            for pixel in frame.as_chunks::<4>().0 {
                samples.extend(color_of(pixel)[..channels].iter().map(|&v| f64::from(v)));
            }
            let indices = entries(&samples, width, channels, &entries_of, Dithering { method: dither, amount: asked.amount, spread }, Some(&marks));
            match transparent {
                Some(entry) => indices.into_iter().zip(&marks).map(|(index, &clear)| if clear { entry } else { index }).collect(),
                None => indices,
            }
        })
        .collect();
    if any_transparent {
        palette.push([0, 0, 0, 0]);
    }
    Paletted { palette, frames: mapped, transparent, found }
}

/// Heckbert's median cut of `colors`, a pixel each, into at most `count`
/// boxes over their first `channels` samples: the box whose colors span
/// the widest range in one channel is cut there at its median pixel, until
/// there are `count` boxes or none holds two colors; each box gives the
/// mean of its pixels, rounded.
fn median_cut(colors: impl Iterator<Item = [u8; 4]>, count: usize, channels: usize) -> Vec<[u8; 4]> {
    let mut histogram: BTreeMap<[u8; 4], u64> = BTreeMap::new();
    for color in colors {
        *histogram.entry(color).or_insert(0) += 1;
    }
    let mut boxes: Vec<Vec<([u8; 4], u64)>> = vec![histogram.into_iter().collect()];
    let widest = |entries: &[([u8; 4], u64)]| -> (usize, u8) {
        let mut best = (0, 0);
        for c in 0..channels {
            let (low, high) = entries.iter().fold((u8::MAX, u8::MIN), |(low, high), (color, _)| (low.min(color[c]), high.max(color[c])));
            if high.saturating_sub(low) > best.1 {
                best = (c, high - low);
            }
        }
        best
    };
    while boxes.len() < count {
        let Some((at, channel)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, entries)| entries.len() > 1)
            .map(|(at, entries)| (at, widest(entries)))
            .max_by(|a, b| a.1.1.cmp(&b.1.1).then(b.0.cmp(&a.0)))
            .map(|(at, (channel, _))| (at, channel))
        else {
            break;
        };
        let mut entries = boxes.swap_remove(at);
        entries.sort_by(|a, b| a.0[channel].cmp(&b.0[channel]).then(a.0.cmp(&b.0)));
        let total: u64 = entries.iter().map(|(_, n)| n).sum();
        let mut seen = 0;
        let mut cut = entries.len() - 1;
        for (i, (_, n)) in entries.iter().enumerate() {
            seen += n;
            if seen * 2 >= total {
                cut = (i + 1).clamp(1, entries.len() - 1);
                break;
            }
        }
        let upper = entries.split_off(cut);
        boxes.push(entries);
        boxes.push(upper);
    }
    let mut out: Vec<[u8; 4]> = boxes
        .iter()
        .map(|entries| {
            let total: u64 = entries.iter().map(|(_, n)| n).sum();
            let mut mean = [0u8; 4];
            for (c, value) in mean.iter_mut().enumerate() {
                let sum: u64 = entries.iter().map(|(color, n)| u64::from(color[c]) * n).sum();
                *value = ((sum as f64) / (total as f64)).round() as u8;
            }
            mean
        })
        .collect();
    out.sort_unstable();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::palette_reference as r;

    fn reference_palette() -> Vec<[f64; 4]> {
        r::PALETTE.as_chunks::<3>().0.iter().map(|&[red, green, blue]| [f64::from(red), f64::from(green), f64::from(blue), 255.0]).collect()
    }

    fn reference_samples() -> Vec<f64> {
        assert_eq!(r::IMAGE.len(), r::WIDTH * r::HEIGHT * 3, "the test image's size");
        r::IMAGE.iter().map(|&v| f64::from(v)).collect()
    }

    /// Every error-diffusion kernel gives the entries hitherdither's does,
    /// and nearest color alone the entries hitherdither's search gives.
    #[test]
    fn diffuses_errors_as_hitherdither_does() {
        let (samples, palette) = (reference_samples(), reference_palette());
        let cases: [(DitherMethod, &[u8]); 9] = [
            (DitherMethod::None, &r::NEAREST),
            (DitherMethod::FloydSteinberg, &r::FLOYD_STEINBERG),
            (DitherMethod::JarvisJudiceNinke, &r::JARVIS_JUDICE_NINKE),
            (DitherMethod::Stucki, &r::STUCKI),
            (DitherMethod::Atkinson, &r::ATKINSON),
            (DitherMethod::Burkes, &r::BURKES),
            (DitherMethod::Sierra, &r::SIERRA),
            (DitherMethod::TwoRowSierra, &r::TWO_ROW_SIERRA),
            (DitherMethod::SierraLite, &r::SIERRA_LITE),
        ];
        for (method, wanted) in cases {
            assert_eq!(entries(&samples, r::WIDTH, 3, &palette, Dithering { method, amount: 1.0, spread: r::SPREAD }, None), wanted, "{method:?}");
        }
    }

    /// Bayer's index matrices are hitherdither's, and each ordered pattern,
    /// centered, gives the entries hitherdither's search gives the pixels
    /// it shifts.
    #[test]
    fn orders_as_hitherdithers_bayer_matrices_centered() {
        let (samples, palette) = (reference_samples(), reference_palette());
        let cases: [(DitherMethod, &[u16], &[u8]); 4] = [
            (DitherMethod::Bayer2x2, &r::BAYER2_INDEX, &r::BAYER2),
            (DitherMethod::Bayer4x4, &r::BAYER4_INDEX, &r::BAYER4),
            (DitherMethod::Bayer8x8, &r::BAYER8_INDEX, &r::BAYER8),
            (DitherMethod::Bayer16x16, &r::BAYER16_INDEX, &r::BAYER16),
        ];
        for (method, index, wanted) in cases {
            let n = method.bayer().expect("an ordered method");
            let built: Vec<u16> = bayer_index(n).into_iter().map(|v| v as u16).collect();
            assert_eq!(built, index, "{method:?}'s matrix");
            assert_eq!(entries(&samples, r::WIDTH, 3, &palette, Dithering { method, amount: 1.0, spread: r::SPREAD }, None), wanted, "{method:?}");
        }
    }

    /// An amount of 0 dithers nothing, for diffusion and patterns alike.
    #[test]
    fn an_amount_of_0_dithers_nothing() {
        let (samples, palette) = (reference_samples(), reference_palette());
        for method in [DitherMethod::FloydSteinberg, DitherMethod::Atkinson, DitherMethod::Bayer8x8] {
            assert_eq!(entries(&samples, r::WIDTH, 3, &palette, Dithering { method, amount: 0.0, spread: r::SPREAD }, None), r::NEAREST, "{method:?}");
        }
    }

    #[test]
    fn spreads_a_pattern_over_a_uniform_palettes_step() {
        assert_eq!(ordered_spread(2), 255.0);
        assert_eq!(ordered_spread(8), 255.0);
        assert!((ordered_spread(64) - 85.0).abs() < 1e-9);
        assert!((ordered_spread(256) - 255.0 / (256f64.cbrt() - 1.0)).abs() < 1e-12);
    }

    fn rgba(pixels: &[[u8; 4]]) -> Vec<u8> {
        pixels.iter().flatten().copied().collect()
    }

    fn asked(colors: usize, binary_alpha: bool) -> Paletting {
        Paletting { colors, quantizer: Quantizer::Auto, dither: DitherMethod::FloydSteinberg, amount: 1.0, binary_alpha }
    }

    /// Colors as few as the palette holds make it exactly, each pixel its
    /// own color, with no dithering.
    #[test]
    fn keeps_as_few_colors_as_the_palette_holds_exactly() {
        let frame = rgba(&[[10, 20, 30, 255], [200, 0, 0, 255], [10, 20, 30, 255], [0, 0, 255, 128]]);
        let out = paletted(&[&frame], 2, &asked(4, false));
        assert_eq!(out.found, Found::Exact);
        assert_eq!(out.palette, [[0, 0, 255, 128], [10, 20, 30, 255], [200, 0, 0, 255]]);
        assert_eq!(out.frames, [vec![1, 2, 1, 0]]);
        assert_eq!(out.transparent, None);
    }

    /// A GIF's pixels below alpha 128 take its transparent entry, last in
    /// the palette, and the rest are opaque, the entry counted among the
    /// colors.
    #[test]
    fn cuts_a_gifs_alpha_at_128() {
        let frame = rgba(&[[255, 0, 0, 127], [0, 255, 0, 128], [0, 0, 255, 255], [9, 9, 9, 0]]);
        let out = paletted(&[&frame], 2, &asked(3, true));
        assert_eq!(out.found, Found::Exact);
        assert_eq!(out.palette, [[0, 0, 255, 255], [0, 255, 0, 255], [0, 0, 0, 0]]);
        assert_eq!(out.transparent, Some(2));
        assert_eq!(out.frames, [vec![2, 1, 0, 2]]);
    }

    /// Frames brought to one palette share it, every frame's colors in it.
    #[test]
    fn shares_one_palette_among_frames() {
        let (first, second) = (rgba(&[[1, 1, 1, 255], [2, 2, 2, 255]]), rgba(&[[3, 3, 3, 255], [1, 1, 1, 255]]));
        let out = paletted(&[&first, &second], 2, &asked(256, true));
        assert_eq!(out.palette, [[1, 1, 1, 255], [2, 2, 2, 255], [3, 3, 3, 255]]);
        assert_eq!(out.frames, [vec![0, 1], vec![2, 0]]);
    }

    /// Median cut cuts the widest box at its median pixel and gives each
    /// box's mean; NeuQuant gives no more colors than asked.
    #[test]
    fn quantizes_more_colors_than_the_palette_holds() {
        let colors = [[0, 0, 0, 255], [10, 0, 0, 255], [200, 0, 0, 255], [210, 0, 0, 255]];
        assert_eq!(median_cut(colors.into_iter(), 2, 3), [[5, 0, 0, 255], [205, 0, 0, 255]]);
        let gradient: Vec<u8> = (0..=255u8).flat_map(|v| [v, 255 - v, v / 2, 255]).collect();
        let small = paletted(&[&gradient], 256, &Paletting { dither: DitherMethod::None, ..asked(16, false) });
        assert_eq!((small.found, small.palette.len()), (Found::MedianCut, 16));
        let large = paletted(&[&gradient], 256, &Paletting { dither: DitherMethod::None, ..asked(64, false) });
        assert_eq!(large.found, Found::NeuQuant);
        assert!(large.palette.len() <= 64, "{} colors", large.palette.len());
        assert!(large.frames[0].iter().all(|&entry| usize::from(entry) < large.palette.len()));
    }
}
