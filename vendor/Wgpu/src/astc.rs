//! ASTC blocks decoded on the CPU, for a device without the ASTC feature a
//! texture needs: every 2D block size, LDR and HDR, as the ASTC chapter of
//! the Khronos Data Format Specification decodes them. An LDR texel comes
//! out as 8 bits a channel, as the specification's decode_unorm8 mode gives
//! it (Vulkan's VK_EXT_astc_decode_mode at RGBA8); an HDR texel comes out
//! as half floats. A block the specification calls an error decodes to
//! magenta, as a GPU shows it.

/// The most texels a block holds, 12 by 12.
const MAX_TEXELS: usize = 144;

/// Magenta, the color of an error block: as 8-bit values and as half floats.
const ERROR_LDR: [u16; 4] = [0xFF, 0, 0xFF, 0xFF];
const ERROR_HDR: [u16; 4] = [0x3C00, 0, 0x3C00, 0x3C00];

/// The alpha of the HDR endpoint modes that have none, a 12-bit
/// logarithmic value that decodes to 1.0.
const HDR_ALPHA: i32 = 0x780;

/// Decodes an LDR block of `width` by `height` texels into `out`, four
/// bytes a texel, rows packed. False when the block is an error, which
/// decodes to magenta.
pub(crate) fn decode_ldr(block: &[u8; 16], width: usize, height: usize, out: &mut [u8]) -> bool {
    let mut texels = [[0u16; 4]; MAX_TEXELS];
    let valid = decode(block, width, height, false, unorm8, &mut texels);
    for (texel, rgba) in out.as_chunks_mut::<4>().0.iter_mut().zip(&texels[..width * height]) {
        for (byte, value) in texel.iter_mut().zip(rgba) {
            *byte = *value as u8;
        }
    }
    valid
}

/// Decodes an HDR block of `width` by `height` texels into `out`, four
/// half floats a texel as their bits, rows packed. False when the block is
/// an error, which decodes to magenta.
pub(crate) fn decode_hdr(block: &[u8; 16], width: usize, height: usize, out: &mut [u16]) -> bool {
    let mut texels = [[0u16; 4]; MAX_TEXELS];
    let valid = decode(block, width, height, true, half, &mut texels);
    for (texel, rgba) in out.as_chunks_mut::<4>().0.iter_mut().zip(&texels[..width * height]) {
        *texel = *rgba;
    }
    valid
}

/// One channel of a texel as its block gives it: the two endpoint values
/// of its partition, LDR (0 to 255) or HDR (12-bit logarithmic), and the
/// weight between them (0 to 64).
#[derive(Clone, Copy)]
struct Channel {
    low: u32,
    high: u32,
    weight: u32,
    hdr: bool,
}

/// An LDR channel as the specification's decode_unorm8 mode gives it.
fn unorm8(c: Channel) -> u16 {
    (interpolate((c.low << 8) | 0x80, (c.high << 8) | 0x80, c.weight) >> 8) as u16
}

/// A channel as a half float: an HDR one by the specification's
/// logarithmic conversion, an LDR one widened to 16 bits first.
fn half(c: Channel) -> u16 {
    if c.hdr {
        lns_to_half(interpolate(c.low << 4, c.high << 4, c.weight))
    } else {
        unorm16_to_half(interpolate(c.low * 257, c.high * 257, c.weight))
    }
}

/// How an integer sequence stores each value: plain bits, or a trit or a
/// quint with that many plain bits below it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Bits,
    Trits,
    Quints,
}

#[derive(Clone, Copy, Debug)]
struct Range {
    kind: Kind,
    bits: u32,
}

impl Range {
    const fn new(kind: Kind, bits: u32) -> Self {
        Self { kind, bits }
    }

    /// The bits `count` values of this range take.
    fn cost(self, count: u32) -> u32 {
        count * self.bits
            + match self.kind {
                Kind::Bits => 0,
                Kind::Trits => (8 * count).div_ceil(5),
                Kind::Quints => (7 * count).div_ceil(3),
            }
    }
}

/// The ranges color endpoint values can take, the widest first, down to
/// the narrowest a valid block uses (0 to 5).
const COLOR_RANGES: [Range; 17] = [
    Range::new(Kind::Bits, 8),
    Range::new(Kind::Trits, 6),
    Range::new(Kind::Quints, 5),
    Range::new(Kind::Bits, 7),
    Range::new(Kind::Trits, 5),
    Range::new(Kind::Quints, 4),
    Range::new(Kind::Bits, 6),
    Range::new(Kind::Trits, 4),
    Range::new(Kind::Quints, 3),
    Range::new(Kind::Bits, 5),
    Range::new(Kind::Trits, 3),
    Range::new(Kind::Quints, 2),
    Range::new(Kind::Bits, 4),
    Range::new(Kind::Trits, 2),
    Range::new(Kind::Quints, 1),
    Range::new(Kind::Bits, 3),
    Range::new(Kind::Trits, 1),
];

/// One value of an integer sequence: its trit or quint, and its plain bits.
#[derive(Clone, Copy, Default)]
struct Value {
    digit: u32,
    low: u32,
}

/// Reads bits in order from one region of a block; bits past the region
/// read as zero.
struct Reader {
    bits: u128,
    at: u32,
    end: u32,
}

impl Reader {
    fn take(&mut self, count: u32) -> u32 {
        let have = self.end.saturating_sub(self.at).min(count);
        let value = if have == 0 { 0 } else { ((self.bits >> self.at) & ((1u128 << have) - 1)) as u32 };
        self.at += count;
        value
    }
}

/// The five trits an 8-bit trit block packs.
fn trits(t: u32) -> [u32; 5] {
    let bit = |value: u32, index: u32| (value >> index) & 1;
    let (c, t3, t4) = if (t >> 2) & 7 == 7 {
        ((t >> 5) << 2 | (t & 3), 2, 2)
    } else if (t >> 5) & 3 == 3 {
        (t & 0x1F, bit(t, 7), 2)
    } else {
        (t & 0x1F, (t >> 5) & 3, bit(t, 7))
    };
    let (t0, t1, t2) = if c & 3 == 3 {
        (bit(c, 3) << 1 | (bit(c, 2) & !bit(c, 3) & 1), bit(c, 4), 2)
    } else if (c >> 2) & 3 == 3 {
        (c & 3, 2, 2)
    } else {
        (bit(c, 1) << 1 | (bit(c, 0) & !bit(c, 1) & 1), (c >> 2) & 3, bit(c, 4))
    };
    [t0, t1, t2, t3, t4]
}

/// The three quints a 7-bit quint block packs.
fn quints(q: u32) -> [u32; 3] {
    let bit = |value: u32, index: u32| (value >> index) & 1;
    if (q >> 1) & 3 == 3 && (q >> 5) & 3 == 0 {
        let q2 = bit(q, 0) << 2 | (bit(q, 4) & !bit(q, 0) & 1) << 1 | (bit(q, 3) & !bit(q, 0) & 1);
        return [4, 4, q2];
    }
    let (c, q2) = if (q >> 1) & 3 == 3 { (((q >> 3) & 3) << 3 | (!(q >> 5) & 3) << 1 | (q & 1), 4) } else { (q & 0x1F, (q >> 5) & 3) };
    if c & 7 == 5 { [(c >> 3) & 3, 4, q2] } else { [c & 7, (c >> 3) & 3, q2] }
}

/// Reads `out.len()` values of `range` as an integer sequence.
fn read_sequence(reader: &mut Reader, range: Range, out: &mut [Value]) {
    let b = range.bits;
    match range.kind {
        Kind::Bits => {
            for value in out {
                *value = Value { digit: 0, low: reader.take(b) };
            }
        }
        Kind::Trits => {
            for group in out.chunks_mut(5) {
                let mut low = [0; 5];
                low[0] = reader.take(b);
                let mut t = reader.take(2);
                low[1] = reader.take(b);
                t |= reader.take(2) << 2;
                low[2] = reader.take(b);
                t |= reader.take(1) << 4;
                low[3] = reader.take(b);
                t |= reader.take(2) << 5;
                low[4] = reader.take(b);
                t |= reader.take(1) << 7;
                // A short last group holds the packed bits of its own values only.
                t &= [0x03, 0x0F, 0x1F, 0x7F, 0xFF][group.len() - 1];
                let digits = trits(t);
                for (i, value) in group.iter_mut().enumerate() {
                    *value = Value { digit: digits[i], low: low[i] };
                }
            }
        }
        Kind::Quints => {
            for group in out.chunks_mut(3) {
                let mut low = [0; 3];
                low[0] = reader.take(b);
                let mut q = reader.take(3);
                low[1] = reader.take(b);
                q |= reader.take(2) << 3;
                low[2] = reader.take(b);
                q |= reader.take(2) << 5;
                q &= [0x07, 0x1F, 0x7F][group.len() - 1];
                let digits = quints(q);
                for (i, value) in group.iter_mut().enumerate() {
                    *value = Value { digit: digits[i], low: low[i] };
                }
            }
        }
    }
}

/// `value`, `from` bits wide, widened to `to` bits by repeating its bits.
fn replicate(value: u32, from: u32, to: u32) -> u32 {
    let (from, to) = (from as i32, to as i32);
    let mut out = 0;
    let mut shift = to - from;
    while shift > -from {
        out |= if shift >= 0 { value << shift } else { value >> -shift };
        shift -= from;
    }
    out
}

/// A color endpoint value scaled to 0 to 255.
fn unquantize_color(value: Value, range: Range) -> i32 {
    if range.kind == Kind::Bits {
        return replicate(value.low, range.bits, 8) as i32;
    }
    const C: [u32; 11] = [204, 113, 93, 54, 44, 26, 22, 13, 11, 6, 5];
    let case = (range.bits * 2 - if range.kind == Kind::Trits { 2 } else { 1 }) as usize;
    let bit = |index: u32| (value.low >> index) & 1;
    let (a, b, c, d, e, f) = (bit(0), bit(1), bit(2), bit(3), bit(4), bit(5));
    let big_a = if a == 1 { 0x1FF } else { 0 };
    let big_b = match case {
        0 | 1 => 0,
        2 => (b << 8) | (b << 4) | (b << 2) | (b << 1),
        3 => (b << 8) | (b << 3) | (b << 2),
        4 => (c << 8) | (b << 7) | (c << 3) | (b << 2) | (c << 1) | b,
        5 => (c << 8) | (b << 7) | (c << 2) | (b << 1) | c,
        6 => (d << 8) | (c << 7) | (b << 6) | (d << 2) | (c << 1) | b,
        7 => (d << 8) | (c << 7) | (b << 6) | (d << 1) | c,
        8 => (e << 8) | (d << 7) | (c << 6) | (b << 5) | (e << 1) | d,
        9 => (e << 8) | (d << 7) | (c << 6) | (b << 5) | e,
        _ => (f << 8) | (e << 7) | (d << 6) | (c << 5) | (b << 4) | f,
    };
    ((((value.digit * C[case] + big_b) ^ big_a) >> 2) | (big_a & 0x80)) as i32
}

/// A weight scaled to 0 to 64.
fn unquantize_weight(value: Value, range: Range) -> u32 {
    let weight = match range.kind {
        Kind::Bits => replicate(value.low, range.bits, 6),
        _ => {
            let case = range.bits * 2 + u32::from(range.kind == Kind::Quints);
            match case {
                0 => [0, 32, 63][value.digit as usize],
                1 => [0, 16, 32, 47, 63][value.digit as usize],
                _ => {
                    const C: [u32; 5] = [50, 28, 23, 13, 11];
                    let bit = |index: u32| (value.low >> index) & 1;
                    let (a, b, c) = (bit(0), bit(1), bit(2));
                    let big_a = if a == 1 { 0x7F } else { 0 };
                    let big_b = match case {
                        2 | 3 => 0,
                        4 => (b << 6) | (b << 2) | b,
                        5 => (b << 6) | (b << 1),
                        _ => (c << 6) | (b << 5) | (c << 1) | b,
                    };
                    (((value.digit * C[case as usize - 2] + big_b) ^ big_a) >> 2) | (big_a & 0x20)
                }
            }
        }
    };
    if weight > 32 { weight + 1 } else { weight }
}

/// A block's weight grid, its planes and the range its weights take.
struct Mode {
    width: usize,
    height: usize,
    dual: bool,
    range: Range,
}

/// The weight grid an 11-bit block mode names, or None for a reserved one.
fn block_mode(m: u32) -> Option<Mode> {
    let bit = |index: u32| (m >> index) & 1;
    let bits = |low: u32, count: u32| (m >> low) & ((1 << count) - 1);
    if (bits(0, 2) == 0 && bits(6, 3) == 7) || bits(0, 4) == 0 {
        return None;
    }
    let a = bits(5, 2);
    let (r, width, height) = if bits(0, 2) == 0 {
        let r = bit(4) | bit(2) << 1 | bit(3) << 2;
        let (w, h) = match bits(7, 2) {
            0 => (12, a + 2),
            1 => (a + 2, 12),
            2 => (a + 6, bits(9, 2) + 6),
            _ if bit(5) == 1 => (10, 6),
            _ => (6, 10),
        };
        (r, w, h)
    } else {
        let r = bit(4) | bit(0) << 1 | bit(1) << 2;
        let (w, h) = match bits(2, 2) {
            0 => (bits(7, 2) + 4, a + 2),
            1 => (bits(7, 2) + 8, a + 2),
            2 => (a + 2, bits(7, 2) + 8),
            _ if bit(8) == 1 => (bit(7) + 2, a + 2),
            _ => (a + 2, bit(7) + 6),
        };
        (r, w, h)
    };
    // A grid whose height takes bits 9 and 10 has a single plane at the low precisions.
    let wide = bits(0, 2) == 0 && bits(7, 2) == 2;
    let high = !wide && bit(9) == 1;
    let dual = !wide && bit(10) == 1;
    let range = match (high, r) {
        (false, 2) => Range::new(Kind::Bits, 1),
        (false, 3) => Range::new(Kind::Trits, 0),
        (false, 4) => Range::new(Kind::Bits, 2),
        (false, 5) => Range::new(Kind::Quints, 0),
        (false, 6) => Range::new(Kind::Trits, 1),
        (false, 7) => Range::new(Kind::Bits, 3),
        (true, 2) => Range::new(Kind::Quints, 1),
        (true, 3) => Range::new(Kind::Trits, 2),
        (true, 4) => Range::new(Kind::Bits, 4),
        (true, 5) => Range::new(Kind::Quints, 2),
        (true, 6) => Range::new(Kind::Trits, 3),
        (true, 7) => Range::new(Kind::Bits, 5),
        _ => return None,
    };
    Some(Mode { width: width as usize, height: height as usize, dual, range })
}

/// A partition's endpoints, and whether its color and its alpha are HDR
/// (12-bit logarithmic values) or LDR (0 to 255).
#[derive(Clone, Copy, Default)]
struct Endpoints {
    low: [i32; 4],
    high: [i32; 4],
    hdr_color: bool,
    hdr_alpha: bool,
}

fn ldr(low: [i32; 4], high: [i32; 4]) -> Endpoints {
    Endpoints { low: low.map(|c| c.clamp(0, 0xFF)), high: high.map(|c| c.clamp(0, 0xFF)), hdr_color: false, hdr_alpha: false }
}

fn hdr(low: [i32; 4], high: [i32; 4]) -> Endpoints {
    Endpoints { low, high, hdr_color: true, hdr_alpha: true }
}

/// Moves the top bit of `a` into `b` and leaves `a` a signed 6-bit offset:
/// (offset, base).
fn transfer(a: i32, b: i32) -> (i32, i32) {
    let base = (b >> 1) | (a & 0x80);
    let offset = (a >> 1) & 0x3F;
    (if offset & 0x20 != 0 { offset - 0x40 } else { offset }, base)
}

fn blue_contract(r: i32, g: i32, b: i32, a: i32) -> [i32; 4] {
    [(r + b) >> 1, (g + b) >> 1, b, a]
}

/// The endpoints of color endpoint mode `mode` from its values `v`.
fn endpoints(mode: u32, v: &[i32]) -> Endpoints {
    match mode {
        0 => ldr([v[0], v[0], v[0], 0xFF], [v[1], v[1], v[1], 0xFF]),
        1 => {
            let l0 = (v[0] >> 2) | (v[1] & 0xC0);
            let l1 = (l0 + (v[1] & 0x3F)).min(0xFF);
            ldr([l0, l0, l0, 0xFF], [l1, l1, l1, 0xFF])
        }
        2 => {
            let (y0, y1) = if v[1] >= v[0] { (v[0] << 4, v[1] << 4) } else { ((v[1] << 4) + 8, (v[0] << 4) - 8) };
            hdr([y0, y0, y0, HDR_ALPHA], [y1, y1, y1, HDR_ALPHA])
        }
        3 => {
            let (y0, d) = if v[0] & 0x80 != 0 {
                (((v[1] >> 5) & 7) << 9 | (v[0] & 0x7F) << 2, (v[1] & 0x1F) << 2)
            } else {
                (((v[1] >> 4) & 0xF) << 8 | (v[0] & 0x7F) << 1, (v[1] & 0xF) << 1)
            };
            let y1 = (y0 + d).min(0xFFF);
            hdr([y0, y0, y0, HDR_ALPHA], [y1, y1, y1, HDR_ALPHA])
        }
        4 => ldr([v[0], v[0], v[0], v[2]], [v[1], v[1], v[1], v[3]]),
        5 => {
            let (d0, l0) = transfer(v[1], v[0]);
            let (d1, a0) = transfer(v[3], v[2]);
            ldr([l0, l0, l0, a0], [l0 + d0, l0 + d0, l0 + d0, a0 + d1])
        }
        6 => ldr([(v[0] * v[3]) >> 8, (v[1] * v[3]) >> 8, (v[2] * v[3]) >> 8, 0xFF], [v[0], v[1], v[2], 0xFF]),
        7 => hdr_rgb_scale(v),
        8 => {
            if v[1] + v[3] + v[5] >= v[0] + v[2] + v[4] {
                ldr([v[0], v[2], v[4], 0xFF], [v[1], v[3], v[5], 0xFF])
            } else {
                ldr(blue_contract(v[1], v[3], v[5], 0xFF), blue_contract(v[0], v[2], v[4], 0xFF))
            }
        }
        9 => {
            let (d0, r) = transfer(v[1], v[0]);
            let (d1, g) = transfer(v[3], v[2]);
            let (d2, b) = transfer(v[5], v[4]);
            if d0 + d1 + d2 >= 0 {
                ldr([r, g, b, 0xFF], [r + d0, g + d1, b + d2, 0xFF])
            } else {
                ldr(blue_contract(r + d0, g + d1, b + d2, 0xFF), blue_contract(r, g, b, 0xFF))
            }
        }
        10 => ldr([(v[0] * v[3]) >> 8, (v[1] * v[3]) >> 8, (v[2] * v[3]) >> 8, v[4]], [v[0], v[1], v[2], v[5]]),
        11 => hdr_rgb(v),
        12 => {
            if v[1] + v[3] + v[5] >= v[0] + v[2] + v[4] {
                ldr([v[0], v[2], v[4], v[6]], [v[1], v[3], v[5], v[7]])
            } else {
                ldr(blue_contract(v[1], v[3], v[5], v[7]), blue_contract(v[0], v[2], v[4], v[6]))
            }
        }
        13 => {
            let (d0, r) = transfer(v[1], v[0]);
            let (d1, g) = transfer(v[3], v[2]);
            let (d2, b) = transfer(v[5], v[4]);
            let (d3, a) = transfer(v[7], v[6]);
            if d0 + d1 + d2 >= 0 {
                ldr([r, g, b, a], [r + d0, g + d1, b + d2, a + d3])
            } else {
                ldr(blue_contract(r + d0, g + d1, b + d2, a + d3), blue_contract(r, g, b, a))
            }
        }
        14 => {
            let mut ends = hdr_rgb(v);
            ends.low[3] = v[6];
            ends.high[3] = v[7];
            ends.hdr_alpha = false;
            ends
        }
        _ => {
            let mut ends = hdr_rgb(v);
            let (a0, a1) = hdr_alpha(v[6], v[7]);
            ends.low[3] = a0;
            ends.high[3] = a1;
            ends
        }
    }
}

fn bit(value: i32, index: u32) -> i32 {
    (value >> index) & 1
}

/// Where each of mode 7's seven spare bits goes, for each of its six
/// submodes: the component (red, green, blue, scale) and the bit.
const RGB_SCALE_BITS: [[(usize, u32); 7]; 6] = [
    [(0, 9), (0, 8), (0, 7), (0, 10), (0, 6), (3, 6), (3, 5)],
    [(0, 8), (1, 5), (0, 7), (2, 5), (0, 6), (0, 10), (0, 9)],
    [(0, 9), (0, 8), (0, 7), (0, 6), (3, 7), (3, 6), (3, 5)],
    [(0, 8), (1, 5), (0, 7), (2, 5), (0, 6), (3, 6), (3, 5)],
    [(1, 6), (1, 5), (2, 6), (2, 5), (0, 6), (0, 7), (3, 5)],
    [(1, 6), (1, 5), (2, 6), (2, 5), (0, 6), (3, 6), (3, 5)],
];

/// HDR RGB, base and scale: color endpoint mode 7.
fn hdr_rgb_scale(v: &[i32]) -> Endpoints {
    let m10 = bit(v[1], 7) | bit(v[2], 7) << 1;
    let m23 = (v[0] >> 6) & 3;
    let (major, submode) = if m10 != 3 {
        (m10, m23)
    } else if m23 != 3 {
        (m23, 4)
    } else {
        (0, 5)
    };
    let submode = submode as usize;
    let mut c = [v[0] & 0x3F, v[1] & 0x1F, v[2] & 0x1F, v[3] & 0x1F];
    let x = [bit(v[1], 6), bit(v[1], 5), bit(v[2], 6), bit(v[2], 5), bit(v[3], 7), bit(v[3], 6), bit(v[3], 5)];
    for (spare, (component, shift)) in x.iter().zip(RGB_SCALE_BITS[submode]) {
        c[component] |= spare << shift;
    }
    let shift = [1, 1, 2, 3, 4, 5][submode];
    let [mut red, mut green, mut blue, scale] = c.map(|value| value << shift);
    if submode != 5 {
        green = red - green;
        blue = red - blue;
    }
    match major {
        1 => std::mem::swap(&mut red, &mut green),
        2 => std::mem::swap(&mut red, &mut blue),
        _ => {}
    }
    let clamp = |value: i32| value.clamp(0, 0xFFF);
    hdr([clamp(red - scale), clamp(green - scale), clamp(blue - scale), HDR_ALPHA], [clamp(red), clamp(green), clamp(blue), HDR_ALPHA])
}

/// Where each of mode 11's six spare bits goes, for each of its eight
/// submodes: the value (a, c, b0, b1, d0, d1) and the bit.
const RGB_BITS: [[(usize, u32); 6]; 8] = [
    [(2, 6), (3, 6), (4, 6), (5, 6), (4, 5), (5, 5)],
    [(2, 6), (3, 6), (2, 7), (3, 7), (4, 5), (5, 5)],
    [(0, 9), (1, 6), (4, 6), (5, 6), (4, 5), (5, 5)],
    [(2, 6), (3, 6), (0, 9), (1, 6), (4, 5), (5, 5)],
    [(2, 6), (3, 6), (2, 7), (3, 7), (0, 9), (0, 10)],
    [(0, 9), (0, 10), (1, 7), (1, 6), (4, 5), (5, 5)],
    [(2, 6), (3, 6), (0, 11), (1, 6), (0, 9), (0, 10)],
    [(0, 9), (0, 10), (0, 11), (1, 6), (4, 5), (5, 5)],
];

/// HDR RGB, direct: color endpoint mode 11, and the color of modes 14
/// and 15.
fn hdr_rgb(v: &[i32]) -> Endpoints {
    let major = bit(v[5], 7) << 1 | bit(v[4], 7);
    if major == 3 {
        return hdr([v[0] << 4, v[2] << 4, (v[4] & 0x7F) << 5, HDR_ALPHA], [v[1] << 4, v[3] << 4, (v[5] & 0x7F) << 5, HDR_ALPHA]);
    }
    let submode = (bit(v[3], 7) << 2 | bit(v[2], 7) << 1 | bit(v[1], 7)) as usize;
    let mut n = [bit(v[1], 6) << 8 | v[0], v[1] & 0x3F, v[2] & 0x3F, v[3] & 0x3F, v[4] & 0x1F, v[5] & 0x1F];
    let x = [bit(v[2], 6), bit(v[3], 6), bit(v[4], 6), bit(v[5], 6), bit(v[4], 5), bit(v[5], 5)];
    for (spare, (value, shift)) in x.iter().zip(RGB_BITS[submode]) {
        n[value] |= spare << shift;
    }
    let d_bits = [7, 6, 7, 6, 5, 6, 5, 6][submode];
    for d in &mut n[4..] {
        *d = (*d << (32 - d_bits)) >> (32 - d_bits);
    }
    let shift = (submode as u32 >> 1) ^ 3;
    let [a, c, b0, b1, d0, d1] = n.map(|value| value << shift);
    let clamp = |value: i32| value.clamp(0, 0xFFF);
    let mut low = [clamp(a - c), clamp(a - b0 - c - d0), clamp(a - b1 - c - d1), HDR_ALPHA];
    let mut high = [clamp(a), clamp(a - b0), clamp(a - b1), HDR_ALPHA];
    match major {
        1 => {
            low.swap(0, 1);
            high.swap(0, 1);
        }
        2 => {
            low.swap(0, 2);
            high.swap(0, 2);
        }
        _ => {}
    }
    hdr(low, high)
}

/// The HDR alpha endpoints of color endpoint mode 15.
fn hdr_alpha(v6: i32, v7: i32) -> (i32, i32) {
    let submode = bit(v7, 7) << 1 | bit(v6, 7);
    let (mut a0, mut a1) = (v6 & 0x7F, v7 & 0x7F);
    if submode == 3 {
        return (a0 << 5, a1 << 5);
    }
    a0 |= (a1 << (submode + 1)) & 0x780;
    a1 &= 0x3F >> submode;
    a1 ^= 0x20 >> submode;
    a1 -= 0x20 >> submode;
    a0 <<= 4 - submode;
    a1 <<= 4 - submode;
    a1 += a0;
    (a0, a1.clamp(0, 0xFFF))
}

fn hash52(seed: u32) -> u32 {
    let mut p = seed;
    p ^= p >> 15;
    p = p.wrapping_sub(p << 17);
    p = p.wrapping_add(p << 7);
    p = p.wrapping_add(p << 4);
    p ^= p >> 5;
    p = p.wrapping_add(p << 16);
    p ^= p >> 7;
    p ^= p >> 3;
    p ^= p << 6;
    p ^= p >> 17;
    p
}

/// The partition of texel (`x`, `y`) in a block of `partitions`
/// partitions with partition index `seed`.
fn partition_of(seed: u32, x: u32, y: u32, partitions: u32, small: bool) -> usize {
    let (x, y) = if small { (x << 1, y << 1) } else { (x, y) };
    let seed = seed + 1024 * (partitions - 1);
    let rnum = hash52(seed);
    let mut s = [
        rnum & 0xF,
        (rnum >> 4) & 0xF,
        (rnum >> 8) & 0xF,
        (rnum >> 12) & 0xF,
        (rnum >> 16) & 0xF,
        (rnum >> 20) & 0xF,
        (rnum >> 24) & 0xF,
        (rnum >> 28) & 0xF,
    ];
    for value in &mut s {
        *value *= *value;
    }
    let sh_a = if seed & 2 != 0 { 4 } else { 5 };
    let sh_b = if partitions == 3 { 6 } else { 5 };
    let (sh1, sh2) = if seed & 1 != 0 { (sh_a, sh_b) } else { (sh_b, sh_a) };
    for (i, value) in s.iter_mut().enumerate() {
        *value >>= if i % 2 == 0 { sh1 } else { sh2 };
    }
    let a = (s[0] * x + s[1] * y + (rnum >> 14)) & 0x3F;
    let b = (s[2] * x + s[3] * y + (rnum >> 10)) & 0x3F;
    let c = if partitions >= 3 { (s[4] * x + s[5] * y + (rnum >> 6)) & 0x3F } else { 0 };
    let d = if partitions >= 4 { (s[6] * x + s[7] * y + (rnum >> 2)) & 0x3F } else { 0 };
    if a >= b && a >= c && a >= d {
        0
    } else if b >= c && b >= d {
        1
    } else if c >= d {
        2
    } else {
        3
    }
}

/// Each texel's weights for each plane, from the weight grid by bilinear
/// interpolation.
fn texel_weights(weights: &[u32; 64], mode: &Mode, width: usize, height: usize) -> [[u32; 2]; MAX_TEXELS] {
    let planes = if mode.dual { 2 } else { 1 };
    let scale_x = (1024 + width / 2) / (width - 1);
    let scale_y = (1024 + height / 2) / (height - 1);
    let mut out = [[0; 2]; MAX_TEXELS];
    for y in 0..height {
        for x in 0..width {
            let gx = (scale_x * x * (mode.width - 1) + 32) >> 6;
            let gy = (scale_y * y * (mode.height - 1) + 32) >> 6;
            let (jx, fx) = (gx >> 4, (gx & 0xF) as u32);
            let (jy, fy) = (gy >> 4, (gy & 0xF) as u32);
            let w11 = (fx * fy + 8) >> 4;
            let (w10, w01) = (fy - w11, fx - w11);
            let w00 = 16 + w11 - fx - fy;
            let i00 = jy * mode.width + jx;
            for (plane, texel) in out[y * width + x][..planes].iter_mut().enumerate() {
                // A grid point past the edge comes with a zero factor, so any in-range index serves.
                let at = |i: usize| weights[(i * planes + plane) & 0x3F];
                *texel = (at(i00) * w00 + at(i00 + 1) * w01 + at(i00 + mode.width) * w10 + at(i00 + mode.width + 1) * w11 + 8) >> 4;
            }
        }
    }
    out
}

/// A 16-bit UNORM value as a half float, rounded toward zero, with 0xFFFF
/// exactly 1.0.
fn unorm16_to_half(value: u32) -> u16 {
    if value == 0xFFFF {
        return 0x3C00;
    }
    if value < 4 {
        return (value << 8) as u16;
    }
    let top = 31 - value.leading_zeros();
    let mantissa = ((value << (16 - top)) & 0xFFFF) >> 6;
    (((top - 1) << 10) | mantissa) as u16
}

/// A 16-bit logarithmic HDR value as a half float, the largest finite one
/// for a value past it.
fn lns_to_half(value: u32) -> u16 {
    let exponent = value >> 11;
    let m = value & 0x7FF;
    let mantissa = if m < 512 {
        3 * m
    } else if m >= 1536 {
        5 * m - 2048
    } else {
        4 * m - 512
    };
    let half = (exponent << 10) + (mantissa >> 3);
    if (half >> 10) & 0x1F == 0x1F { 0x7BFF } else { half as u16 }
}

fn interpolate(low: u32, high: u32, weight: u32) -> u32 {
    (low * (64 - weight) + high * weight + 32) / 64
}

/// A void-extent block: one color for every texel.
fn void_extent(bits: u128, is_hdr: bool, texels: &mut [[u16; 4]]) -> bool {
    let get = |start: u32, count: u32| ((bits >> start) & ((1u128 << count) - 1)) as u32;
    let error = if is_hdr { ERROR_HDR } else { ERROR_LDR };
    let (min_s, max_s, min_t, max_t) = (get(12, 13), get(25, 13), get(38, 13), get(51, 13));
    let all_ones = [min_s, max_s, min_t, max_t].iter().all(|&e| e == 0x1FFF);
    let hdr_block = get(9, 1) == 1;
    let color = [get(64, 16), get(80, 16), get(96, 16), get(112, 16)];
    // An infinite or NaN HDR color is undefined; the reference decoder treats it as an error.
    let finite = !hdr_block || color.iter().all(|&c| (c >> 10) & 0x1F != 0x1F);
    let valid = get(10, 2) == 3 && (all_ones || (min_s < max_s && min_t < max_t)) && (is_hdr || !hdr_block) && finite;
    let rgba = if !valid {
        error
    } else if !is_hdr {
        color.map(|c| (c >> 8) as u16)
    } else if hdr_block {
        color.map(|c| c as u16)
    } else {
        color.map(unorm16_to_half)
    };
    texels.fill(rgba);
    valid
}

/// The color endpoint mode of each of a block's `partitions`. The bits of
/// the modes that do not fit below bit 29 start at bit `extra`, just below
/// the weights.
fn endpoint_modes(bits: u128, partitions: u32, extra: u32) -> [u32; 4] {
    let get = |start: u32, count: u32| ((bits >> start) & ((1u128 << count) - 1)) as u32;
    if partitions == 1 {
        return [get(13, 4), 0, 0, 0];
    }
    let class = get(23, 2);
    if class == 0 {
        return [get(25, 4); 4];
    }
    let at = |index: u32| if index < 4 { 25 + index } else { extra + index - 4 };
    let mut modes = [0u32; 4];
    for (p, mode) in modes[..partitions as usize].iter_mut().enumerate() {
        let p = p as u32;
        let base = class - (1 - get(25 + p, 1));
        *mode = base << 2 | get(at(partitions + 2 * p + 1), 1) << 1 | get(at(partitions + 2 * p), 1);
    }
    modes
}

/// Decodes one block into `out`, each channel of each texel written by
/// `channel`: 8-bit values for an LDR texture, half float bits for an HDR
/// one. False for an error block.
fn decode(block: &[u8; 16], width: usize, height: usize, is_hdr: bool, channel: impl Fn(Channel) -> u16, out: &mut [[u16; 4]; MAX_TEXELS]) -> bool {
    let error = if is_hdr { ERROR_HDR } else { ERROR_LDR };
    let texels = &mut out[..width * height];
    let bits = u128::from_le_bytes(*block);
    let get = |start: u32, count: u32| ((bits >> start) & ((1u128 << count) - 1)) as u32;
    if get(0, 9) == 0x1FC {
        return void_extent(bits, is_hdr, texels);
    }
    let Some(mode) = block_mode(get(0, 11)) else {
        texels.fill(error);
        return false;
    };
    let partitions = get(11, 2) + 1;
    let weight_count = (mode.width * mode.height) as u32 * if mode.dual { 2 } else { 1 };
    let weight_bits = mode.range.cost(weight_count);
    if weight_count > 64 || !(24..=96).contains(&weight_bits) || mode.width > width || mode.height > height || (partitions == 4 && mode.dual) {
        texels.fill(error);
        return false;
    }
    let single_mode = partitions == 1 || get(23, 2) == 0;
    let config_bits = if partitions == 1 {
        17
    } else if single_mode {
        29
    } else {
        25 + 3 * partitions
    } + if mode.dual { 2 } else { 0 };
    let color_bits = 128 - weight_bits as i32 - config_bits as i32;
    // The endpoint mode bits that do not fit below bit 29 are just below the weights.
    let extra = (127 - weight_bits as i32 - if single_mode { -1 } else { 3 * partitions as i32 - 5 }) as u32;

    let modes = endpoint_modes(bits, partitions, extra);
    let value_count: u32 = modes[..partitions as usize].iter().map(|m| ((m >> 2) + 1) * 2).sum();
    if value_count > 18 || color_bits < (13 * value_count).div_ceil(5) as i32 {
        texels.fill(error);
        return false;
    }
    let Some(&color_range) = COLOR_RANGES.iter().find(|r| r.cost(value_count) as i32 <= color_bits) else {
        texels.fill(error);
        return false;
    };
    let start = if partitions == 1 { 17 } else { 29 };
    let mut values = [Value::default(); 18];
    read_sequence(&mut Reader { bits, at: start, end: start + color_bits as u32 }, color_range, &mut values[..value_count as usize]);
    let colors: [i32; 18] = values.map(|value| unquantize_color(value, color_range));
    let mut ends = [Endpoints::default(); 4];
    let mut next = 0;
    for (p, end) in ends[..partitions as usize].iter_mut().enumerate() {
        let count = (((modes[p] >> 2) + 1) * 2) as usize;
        *end = endpoints(modes[p], &colors[next..next + count]);
        next += count;
    }

    let mut grid = [Value::default(); 64];
    read_sequence(&mut Reader { bits: bits.reverse_bits(), at: 0, end: weight_bits }, mode.range, &mut grid[..weight_count as usize]);
    let mut weights = [0u32; 64];
    for (weight, value) in weights.iter_mut().zip(&grid[..weight_count as usize]) {
        *weight = unquantize_weight(*value, mode.range);
    }
    let per_texel = texel_weights(&weights, &mode, width, height);
    let second_plane = if mode.dual { Some(get(extra - 2, 2) as usize) } else { None };
    let seed = get(13, 10);
    let small = width * height < 31;

    let mut valid = true;
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            let p = if partitions == 1 { 0 } else { partition_of(seed, x as u32, y as u32, partitions, small) };
            let end = ends[p];
            if !is_hdr && end.hdr_color {
                texels[i] = error;
                valid = false;
                continue;
            }
            for c in 0..4 {
                texels[i][c] = channel(Channel {
                    low: end.low[c] as u32,
                    high: end.high[c] as u32,
                    weight: per_texel[i][usize::from(second_plane == Some(c))],
                    hdr: if c == 3 { end.hdr_alpha } else { end.hdr_color },
                });
            }
        }
    }
    valid
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows of the trit and quint tables of the Khronos conformance
    /// suite's reference decoder, the specification's special cases among them.
    #[test]
    fn unpacks_trits_and_quints_as_the_reference_tables() {
        let trit_rows: [(u32, [u32; 5]); 12] = [
            (0, [0, 0, 0, 0, 0]),
            (2, [2, 0, 0, 0, 0]),
            (3, [0, 0, 2, 0, 0]),
            (5, [1, 1, 0, 0, 0]),
            (11, [2, 0, 2, 0, 0]),
            (28, [0, 0, 0, 2, 2]),
            (31, [0, 0, 2, 2, 2]),
            (60, [0, 1, 0, 2, 2]),
            (96, [0, 0, 0, 0, 2]),
            (124, [0, 2, 2, 2, 2]),
            (127, [2, 0, 2, 2, 2]),
            (255, [2, 1, 2, 2, 2]),
        ];
        for (t, expected) in trit_rows {
            assert_eq!(trits(t), expected, "T = {t}");
        }
        let quint_rows: [(u32, [u32; 3]); 9] = [
            (5, [0, 4, 0]),
            (6, [4, 4, 0]),
            (7, [4, 4, 4]),
            (13, [1, 4, 0]),
            (14, [4, 4, 1]),
            (38, [4, 0, 4]),
            (39, [0, 4, 4]),
            (102, [0, 0, 4]),
            (127, [1, 3, 4]),
        ];
        for (q, expected) in quint_rows {
            assert_eq!(quints(q), expected, "Q = {q}");
        }
    }

    /// The unquantized weights of several weight ranges, in encoded order,
    /// as the specification lists them.
    #[test]
    fn unquantizes_weights_as_the_specification_lists_them() {
        let ranges: [(Range, &[u32]); 6] = [
            (Range::new(Kind::Trits, 1), &[0, 64, 12, 52, 25, 39]),
            (Range::new(Kind::Quints, 1), &[0, 64, 7, 57, 14, 50, 21, 43, 28, 36]),
            (Range::new(Kind::Trits, 2), &[0, 64, 17, 47, 5, 59, 23, 41, 11, 53, 28, 36]),
            (Range::new(Kind::Quints, 2), &[0, 64, 16, 48, 3, 61, 19, 45, 6, 58, 23, 41, 9, 55, 26, 38, 13, 51, 29, 35]),
            (Range::new(Kind::Bits, 2), &[0, 21, 43, 64]),
            (Range::new(Kind::Bits, 3), &[0, 9, 18, 27, 37, 46, 55, 64]),
        ];
        for (range, expected) in ranges {
            let decoded: Vec<u32> = (0..expected.len() as u32)
                .map(|encoded| {
                    let value = if range.kind == Kind::Bits {
                        Value { digit: 0, low: encoded }
                    } else {
                        Value { digit: encoded >> range.bits, low: encoded & ((1 << range.bits) - 1) }
                    };
                    unquantize_weight(value, range)
                })
                .collect();
            assert_eq!(decoded, expected, "{range:?}");
        }
    }

    #[test]
    fn decodes_a_void_extent_block_to_its_color() {
        // Bits 0-8 0x1FC, bits 10-11 set, every extent all ones, then the RGBA16 color.
        let mut bits: u128 = 0x1FC | 0b11 << 10 | ((1u128 << 52) - 1) << 12;
        bits |= 0x1234u128 << 64 | 0xFFFFu128 << 80 | 0x8000u128 << 96 | 0x00FFu128 << 112;
        let block = bits.to_le_bytes();
        let mut ldr = [0u8; 4 * 16];
        assert!(decode_ldr(&block, 4, 4, &mut ldr));
        assert_eq!(&ldr[..4], &[0x12, 0xFF, 0x80, 0x00]);
        assert_eq!(&ldr[60..], &[0x12, 0xFF, 0x80, 0x00]);
        let mut hdr = [0u16; 4 * 16];
        assert!(decode_hdr(&block, 4, 4, &mut hdr));
        assert_eq!(&hdr[..4], &[unorm16_to_half(0x1234), 0x3C00, 0x3800, unorm16_to_half(0x00FF)]);
    }

    #[test]
    fn decodes_a_reserved_block_mode_to_magenta() {
        let block = [0u8; 16];
        let mut ldr = [0u8; 4 * 36];
        assert!(!decode_ldr(&block, 6, 6, &mut ldr));
        assert!(ldr.as_chunks::<4>().0.iter().all(|texel| *texel == [0xFF, 0, 0xFF, 0xFF]));
        let mut hdr = [0u16; 4 * 36];
        assert!(!decode_hdr(&block, 6, 6, &mut hdr));
        assert!(hdr.as_chunks::<4>().0.iter().all(|texel| *texel == [0x3C00, 0, 0x3C00, 0x3C00]));
    }

    /// What a set of blocks exercised: the blocks of each partition count,
    /// partitioned blocks whose partitions have endpoint modes of their
    /// own, dual-plane blocks, void-extent blocks, and how many blocks of a
    /// single endpoint mode used each mode.
    #[derive(Default, Debug)]
    struct Coverage {
        partitions: [usize; 5],
        mixed: usize,
        dual: usize,
        void: usize,
        modes: [usize; 16],
    }

    impl Coverage {
        fn add(&mut self, block: &[u8; 16]) {
            let bits = u128::from_le_bytes(*block);
            let get = |start: u32, count: u32| ((bits >> start) & ((1u128 << count) - 1)) as u32;
            if get(0, 9) == 0x1FC {
                self.void += 1;
                return;
            }
            let Some(mode) = block_mode(get(0, 11)) else {
                return;
            };
            let partitions = get(11, 2) + 1;
            self.partitions[partitions as usize] += 1;
            self.dual += usize::from(mode.dual);
            if partitions == 1 {
                self.modes[get(13, 4) as usize] += 1;
            } else if get(23, 2) == 0 {
                self.modes[get(25, 4) as usize] += 1;
            } else {
                self.mixed += 1;
            }
        }

        fn partitioned(&self) -> usize {
            self.partitions[2..].iter().sum()
        }

        fn mode_count(&self) -> usize {
            self.modes.iter().filter(|&&n| n > 0).count()
        }
    }

    /// The next value of a splitmix64 sequence.
    fn next(state: &mut u64) -> u64 {
        *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = *state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// An image that drives an encoder through many kinds of block, in
    /// three bands: diagonal bands of color whose edges cross blocks of
    /// every size, with noise and an alpha that changes apart from the
    /// color; a gray gradient; and one hue whose brightness alone changes.
    fn ldr_image(width: usize, height: usize) -> Vec<u8> {
        let mut state = 0x2545_F491_4F6C_DD1D;
        let mut pixels = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let (across, down) = (x * 255 / width, y * 255 / height);
                let rgba = match y * 3 / height {
                    0 => {
                        let noise = (next(&mut state) & 0x1F) as usize;
                        let base = [[230, 40, 30], [20, 200, 60], [40, 60, 220]][(x + 2 * y) / 7 % 3];
                        let alpha = if (x / 5 + y / 3) % 2 == 0 { 255 } else { down };
                        [((base[0] + across) / 2 + noise).min(255), ((base[1] + down) / 2 + noise / 2).min(255), base[2] / 2 + across / 3, alpha]
                    }
                    1 => [across, across, across, 255],
                    _ => [across, across * 3 / 4, across / 3, 255],
                };
                pixels.extend(rgba.map(|c| c as u8));
            }
        }
        pixels
    }

    /// An HDR image in three bands: one hue whose brightness alone runs
    /// from 1/16 to 256 across it; two hues in checks of three texels, so
    /// that blocks hold both; and colorful noise.
    fn hdr_image(width: usize, height: usize) -> Vec<f32> {
        let mut state = 0x6A09_E667_F3BC_C908;
        let mut pixels = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let rgb = match y * 3 / height {
                    0 => [0.9, 0.6, 0.3].map(|c| c * (x as f32 / width as f32 * 12.0 - 4.0).exp2()),
                    1 if (x / 3 + y / 3) % 2 == 0 => [4.0, 0.2, 0.1].map(|c| c * (1.0 + y as f32 / 8.0)),
                    1 => [0.1, 0.3, 6.0].map(|c| c * (1.0 + y as f32 / 8.0)),
                    _ => [0u8; 3].map(|_| (next(&mut state) % 1024) as f32 / 64.0),
                };
                pixels.extend([rgb[0], rgb[1], rgb[2], 1.0]);
            }
        }
        pixels
    }

    /// texture2ddecoder's last step for an LDR channel: the endpoints'
    /// bits repeated to 16, the result scaled to 8 bits.
    fn replicated(c: Channel) -> u16 {
        ((interpolate(c.low * 257, c.high * 257, c.weight) * 255 + 32768) / 65536) as u16
    }

    /// basisu's transcoder turns one Basis file into ASTC blocks and into
    /// the pixels its own decoder decodes them to. Every LDR block size, the
    /// file marked sRGB, so both sides decode as the specification's
    /// decode_unorm8 mode does, and the transcoder told not to smooth the
    /// edges of blocks of 80 texels and more, which by default it does.
    #[test]
    fn decodes_ldr_blocks_as_basisu_decodes_them() {
        use crate::basis::{self, Image, Ktx2, Pixels, decode, flags, target};
        // basisu's order: BTF_ASTC_LDR_4X4 is 19, and its targets are 10, then 28 on.
        let sizes = [(4, 4), (5, 4), (5, 5), (6, 5), (6, 6), (8, 5), (8, 6), (10, 5), (10, 6), (8, 8), (10, 8), (10, 10), (12, 10), (12, 12)];
        let (width, height) = (60, 48);
        let pixels = ldr_image(width, height);
        let mut coverage = Coverage::default();
        for (i, &(bw, bh)) in sizes.iter().enumerate() {
            let image = Image { width: width as u32, height: height as u32, pixels: Pixels::Rgba8(&pixels) };
            let mut warnings = Vec::new();
            let file = basis::encode_ktx2(&[image], 19 + i as u32, -1, -1, flags::SRGB, &mut warnings).expect("encoded");
            let ktx2 = Ktx2::open(&file, &mut warnings).expect("read").expect("a Basis file");
            let blocks = ktx2.transcode(0, 0, 0, if i == 0 { 10 } else { 27 + i as u32 }, decode::NO_DEBLOCK_FILTERING, &mut warnings).expect("blocks");
            let reference = ktx2.transcode(0, 0, 0, target::RGBA32, decode::NO_DEBLOCK_FILTERING, &mut warnings).expect("pixels");
            assert!(warnings.is_empty(), "{bw}x{bh}: {warnings:?}");
            let across = width.div_ceil(bw);
            let (blocks, rest) = blocks.as_chunks::<16>();
            assert!(rest.is_empty() && blocks.len() == across * height.div_ceil(bh), "{bw}x{bh}: {} blocks", blocks.len());
            let mut texels = [0u8; 4 * MAX_TEXELS];
            for (index, block) in blocks.iter().enumerate() {
                coverage.add(block);
                assert!(decode_ldr(block, bw, bh, &mut texels), "{bw}x{bh} block {index} is an error");
                let (bx, by) = (index % across * bw, index / across * bh);
                for y in 0..bh.min(height - by) {
                    for x in 0..bw.min(width - bx) {
                        let at = ((by + y) * width + bx + x) * 4;
                        assert_eq!(&texels[(y * bw + x) * 4..][..4], &reference[at..at + 4], "{bw}x{bh} block {index} {block:02X?}, texel ({x}, {y})");
                    }
                }
            }
        }
        assert!(coverage.partitioned() > 0 && coverage.dual > 0 && coverage.mode_count() >= 3, "{coverage:?}");
    }

    /// What texture2ddecoder writes for a channel: an LDR one as
    /// `replicated`, an HDR one as its half float times 255, rounded down
    /// and clamped to a byte.
    fn texture2ddecoder_channel(c: Channel) -> u16 {
        if !c.hdr {
            return replicated(c);
        }
        let value = half::f16::from_bits(lns_to_half(interpolate(c.low << 4, c.high << 4, c.weight))).to_f32();
        (value * 255.0).floor().clamp(0.0, 255.0) as u16
    }

    /// The endpoint mode of each partition of a block this decoder takes as
    /// valid.
    fn modes_of(block: &[u8; 16]) -> Vec<u32> {
        let bits = u128::from_le_bytes(*block);
        let get = |start: u32, count: u32| ((bits >> start) & ((1u128 << count) - 1)) as u32;
        let mode = block_mode(get(0, 11)).expect("a valid block's mode");
        let partitions = get(11, 2) + 1;
        let weight_bits = mode.range.cost((mode.width * mode.height) as u32 * if mode.dual { 2 } else { 1 });
        let single = partitions == 1 || get(23, 2) == 0;
        let extra = (127 - weight_bits as i32 - if single { -1 } else { 3 * partitions as i32 - 5 }) as u32;
        endpoint_modes(bits, partitions, extra)[..partitions as usize].to_vec()
    }

    /// Random blocks of every size that this decoder takes as valid and
    /// that are not void-extent, decoded here and by texture2ddecoder:
    /// every block mode, partition count and range a block can take, and
    /// every endpoint mode but 11, 14 and 15. texture2ddecoder widens the
    /// signed `d` values of mode 11's RGB, which 14 and 15 share, with
    /// `vd0 |= 0xffc0` on an i32, a large positive value where the
    /// specification has a negative one; basisu's HDR blocks check mode 11,
    /// and the tests below its negative `d` and mode 15's alpha.
    #[test]
    fn decodes_random_blocks_as_texture2ddecoder_decodes_them() {
        let sizes = [(4, 4), (5, 4), (5, 5), (6, 5), (6, 6), (8, 5), (8, 6), (8, 8), (10, 5), (10, 6), (10, 8), (10, 10), (12, 10), (12, 12)];
        let mut state = 0x243F_6A88_85A3_08D3;
        let mut coverage = Coverage::default();
        let mut ours = [[0u16; 4]; MAX_TEXELS];
        let mut theirs = [0u32; MAX_TEXELS];
        for (bw, bh) in sizes {
            for _ in 0..20_000 {
                let block = (u128::from(next(&mut state)) << 64 | u128::from(next(&mut state))).to_le_bytes();
                if block[0] == 0xFC && block[1] & 1 == 1 || !decode(&block, bw, bh, true, texture2ddecoder_channel, &mut ours) {
                    continue;
                }
                if modes_of(&block).iter().any(|m| matches!(m, 11 | 14 | 15)) {
                    continue;
                }
                coverage.add(&block);
                texture2ddecoder::decode_astc_block(&block, bw, bh, &mut theirs);
                for (texel, (rgba, bgra)) in ours.iter().zip(theirs).take(bw * bh).enumerate() {
                    let [b, g, r, a] = bgra.to_le_bytes();
                    assert_eq!(rgba.map(|c| c as u8), [r, g, b, a], "{bw}x{bh} {block:02X?}, texel {texel}");
                }
            }
        }
        let checked = (0..16).filter(|m| !matches!(m, 11 | 14 | 15)).all(|m| coverage.modes[m] > 0);
        assert!(checked && coverage.partitions[2..].iter().all(|&n| n > 0) && coverage.mixed > 0 && coverage.dual > 0, "{coverage:?}");
    }

    /// Mode 11 with a negative `d`: its values traced by hand through the
    /// Khronos conformance suite's reference decoder (submode 5, d0 sign
    /// extended from 6 bits to -30, then shifted to -60).
    #[test]
    fn decodes_mode_11_with_a_negative_offset() {
        let ends = endpoints(11, &[51, 204, 119, 204, 34, 17]);
        assert_eq!((ends.low, ends.high), ([3662, 3612, 3604, HDR_ALPHA], [3686, 3576, 3662, HDR_ALPHA]));
        assert!(ends.hdr_color && ends.hdr_alpha);
    }

    /// Mode 15's alpha in each of its four submodes, traced by hand through
    /// the reference decoder.
    #[test]
    fn decodes_mode_15_alpha_in_each_submode() {
        assert_eq!(hdr_alpha(0x12, 0x10), (288, 544));
        assert_eq!(hdr_alpha(0x85, 0x10), (40, 0));
        assert_eq!(hdr_alpha(0x03, 0xBF), (1548, 1544));
        assert_eq!(hdr_alpha(0xFF, 0xC1), (4064, 2080));
    }

    /// The same for HDR: UASTC HDR 4x4, ASTC HDR 6x6 and UASTC HDR 6x6,
    /// whose blocks and half floats basisu's transcoder gives.
    #[test]
    fn decodes_hdr_blocks_as_basisu_decodes_them() {
        use crate::basis::{self, Image, Ktx2, Pixels, decode, target};
        let (width, height) = (36, 24);
        let floats = hdr_image(width, height);
        let mut coverage = Coverage::default();
        for (format, astc_target, b) in [(2, target::ASTC_HDR_4X4_RGBA, 4), (3, 27, 6), (4, 27, 6)] {
            let image = Image { width: width as u32, height: height as u32, pixels: Pixels::RgbaFloat(&floats) };
            let mut warnings = Vec::new();
            let file = basis::encode_ktx2(&[image], format, 100, 10, 0, &mut warnings).expect("encoded");
            let ktx2 = Ktx2::open(&file, &mut warnings).expect("read").expect("a Basis file");
            let blocks = ktx2.transcode(0, 0, 0, astc_target, decode::DEFAULT, &mut warnings).expect("blocks");
            let reference: Vec<u16> =
                ktx2.transcode(0, 0, 0, target::RGBA_HALF, decode::DEFAULT, &mut warnings).expect("texels").as_chunks::<2>().0.iter().map(|h| u16::from_le_bytes(*h)).collect();
            assert!(warnings.is_empty(), "BTF {format}: {warnings:?}");
            let across = width.div_ceil(b);
            let (blocks, rest) = blocks.as_chunks::<16>();
            assert!(rest.is_empty() && blocks.len() == across * height.div_ceil(b), "BTF {format}: {} blocks", blocks.len());
            let mut texels = [0u16; 4 * MAX_TEXELS];
            for (index, block) in blocks.iter().enumerate() {
                coverage.add(block);
                assert!(decode_hdr(block, b, b, &mut texels), "BTF {format} block {index} is an error");
                let (bx, by) = (index % across * b, index / across * b);
                for y in 0..b.min(height - by) {
                    for x in 0..b.min(width - bx) {
                        let at = ((by + y) * width + bx + x) * 4;
                        assert_eq!(&texels[(y * b + x) * 4..][..4], &reference[at..at + 4], "BTF {format} block {index} {block:02X?}, texel ({x}, {y})");
                    }
                }
            }
        }
        assert!(coverage.partitioned() > 0 && coverage.mode_count() >= 2, "{coverage:?}");
    }

    #[test]
    fn converts_unorm16_and_logarithmic_values_to_half_floats() {
        assert_eq!(unorm16_to_half(0), 0);
        assert_eq!(unorm16_to_half(3), 3 << 8);
        assert_eq!(unorm16_to_half(4), 0x0400);
        assert_eq!(unorm16_to_half(0x8000), 0x3800);
        assert_eq!(unorm16_to_half(0xFFFE), 0x3BFF);
        assert_eq!(unorm16_to_half(0xFFFF), 0x3C00);
        assert_eq!(lns_to_half((HDR_ALPHA as u32) << 4), 0x3C00);
        assert_eq!(lns_to_half(0xFFF0), 0x7BFF);
    }
}
