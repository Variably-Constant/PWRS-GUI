//! Reference values for image_decode::tests, written by
//! tools/hdr_reference.py from colour-science: 16-bit PQ and HLG signals
//! in BT.2020 primaries as linear light in sRGB's, BT.2408's HDR
//! reference white of 203 nits at 1.

/// The signals, red, green and blue, of each pixel.
pub(crate) const HDR_PIXELS: [u16; 27] = [
    0, 0, 0, 65535, 65535, 65535, 38050, 38050, 38050,
    49151, 49151, 49151, 32768, 32768, 32768, 16384, 16384, 16384,
    52000, 20000, 9000, 10000, 50000, 12000, 30000, 30000, 60000,
];

/// The pixels as PQ signals, as linear light in sRGB's primaries.
pub(crate) const HDR_PQ: [f32; 27] = [
    0.0_f32, 0.0_f32, 0.0_f32, 49.261086_f32, 49.261086_f32, 49.261086_f32, 0.9992053_f32, 0.9992053_f32, 0.9992053_f32,
    4.844056_f32, 4.844056_f32, 4.844056_f32, 0.4544471_f32, 0.4544471_f32, 0.4544471_f32, 0.025391392_f32, 0.025391392_f32, 0.025391392_f32,
    11.950527_f32, 0.0_f32, 0.0_f32, 0.0_f32, 6.1794457_f32, 0.0_f32, 0.0_f32, 0.1124924_f32, 24.833153_f32,
];

/// The pixels as HLG signals, as linear light in sRGB's primaries.
pub(crate) const HDR_HLG: [f32; 27] = [
    0.0_f32, 0.0_f32, 0.0_f32, 4.9261084_f32, 4.9261084_f32, 4.9261084_f32, 0.37633133_f32, 0.37633133_f32, 0.37633133_f32,
    1.0007262_f32, 1.0007262_f32, 1.0007262_f32, 0.2497482_f32, 0.2497482_f32, 0.2497482_f32, 0.047318436_f32, 0.047318436_f32, 0.047318436_f32,
    1.6790218_f32, 0.0_f32, 0.0_f32, 0.0_f32, 1.1350404_f32, 0.0_f32, 0.09027683_f32, 0.2037941_f32, 2.187392_f32,
];
