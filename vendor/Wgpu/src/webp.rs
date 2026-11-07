//! WebP files through libwebp, which libwebp-sys compiles: a file's frames
//! as libwebp's animation decoder composites them, a still file as its one
//! frame, and frames written as a still or animated WebP, lossless or lossy
//! at a quality. An animation is assembled by libwebp's mux from each frame
//! encoded whole, so every frame stays its own, where libwebp's animation
//! encoder merges a frame like the one before it into that one.

use std::ptr;
use std::slice;

use libwebp_sys as w;

/// A WebP file's frames as libwebp composites them, RGBA on its canvas,
/// with each frame's delay in milliseconds and how many times it plays, 0
/// for endlessly.
pub(crate) struct Frames {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) frames: Vec<Vec<u8>>,
    pub(crate) delays: Vec<u32>,
    pub(crate) plays: u32,
}

/// A WebP animation decoder libwebp holds open, deleted on drop.
struct Decoder(*mut w::WebPAnimDecoder);

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY: the decoder is open and nothing uses it after this.
        unsafe { w::WebPAnimDecoderDelete(self.0) }
    }
}

/// The frames of the WebP file `bytes`, or why libwebp cannot decode it.
pub(crate) fn decode(bytes: &[u8]) -> Result<Frames, String> {
    let data = w::WebPData { bytes: bytes.as_ptr(), size: bytes.len() };
    // SAFETY: the options are plain data that WebPAnimDecoderOptionsInit
    // fills.
    let mut options: w::WebPAnimDecoderOptions = unsafe { std::mem::zeroed() };
    // SAFETY: `options` is valid for writes.
    if unsafe { w::WebPAnimDecoderOptionsInit(&mut options) } == 0 {
        return Err("libwebp's decoder options do not initialize".to_string());
    }
    options.color_mode = w::WEBP_CSP_MODE::MODE_RGBA;
    // SAFETY: `data` points at `bytes`, which outlive the decoder, and
    // `options` is initialized.
    let decoder = Decoder(unsafe { w::WebPAnimDecoderNew(&data, &options) });
    if decoder.0.is_null() {
        return Err("libwebp cannot decode it".to_string());
    }
    let mut info = w::WebPAnimInfo::default();
    // SAFETY: the decoder is open and `info` is valid for writes.
    if unsafe { w::WebPAnimDecoderGetInfo(decoder.0, &mut info) } == 0 {
        return Err("libwebp cannot read its canvas".to_string());
    }
    let size = (info.canvas_width as usize) * (info.canvas_height as usize) * 4;
    let mut frames = Vec::new();
    let mut delays = Vec::new();
    let mut ended = 0;
    // SAFETY: the decoder is open.
    while unsafe { w::WebPAnimDecoderHasMoreFrames(decoder.0) } != 0 {
        let mut pixels: *mut u8 = ptr::null_mut();
        let mut end = 0;
        // SAFETY: the decoder is open, and both outputs are valid for writes.
        if unsafe { w::WebPAnimDecoderGetNext(decoder.0, &mut pixels, &mut end) } == 0 || pixels.is_null() {
            return Err(format!("its frame {} does not decode", frames.len()));
        }
        // SAFETY: libwebp hands over the canvas, `size` bytes of RGBA that
        // stay valid until the next call on the decoder.
        frames.push(unsafe { slice::from_raw_parts(pixels, size) }.to_vec());
        delays.push(u32::try_from(end - ended).map_err(|e| format!("its frame {} ends before the one before it: {e}", frames.len() - 1))?);
        ended = end;
    }
    Ok(Frames { width: info.canvas_width, height: info.canvas_height, frames, delays, plays: info.loop_count })
}

/// How a WebP file is written: lossless, keeping the colors of transparent
/// pixels, or lossy at `quality`, 0 to 100; with libwebp's `method`, 0 the
/// fastest to 6 the smallest.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Encoding {
    pub(crate) quality: Option<f32>,
    pub(crate) method: i32,
}

/// The longest a WebP frame shows, in milliseconds: its duration takes 24
/// bits.
const LONGEST_FRAME: u32 = (1 << 24) - 1;

/// A picture libwebp's encoder reads a frame from, freed on drop.
struct Picture(w::WebPPicture);

impl Drop for Picture {
    fn drop(&mut self) {
        // SAFETY: the picture was initialized, and freeing one that holds
        // nothing is allowed.
        unsafe { w::WebPPictureFree(&mut self.0) }
    }
}

/// The memory libwebp's encoder writes a file into, cleared on drop.
struct Written(w::WebPMemoryWriter);

impl Drop for Written {
    fn drop(&mut self) {
        // SAFETY: the writer was initialized, and clearing one that holds
        // nothing is allowed.
        unsafe { w::WebPMemoryWriterClear(&mut self.0) }
    }
}

/// A WebP mux libwebp holds open, deleted on drop.
struct Mux(*mut w::WebPMux);

impl Drop for Mux {
    fn drop(&mut self) {
        // SAFETY: the mux is open and nothing uses it after this.
        unsafe { w::WebPMuxDelete(self.0) }
    }
}

/// Frame `index`, `width` by `height` RGBA, as the still WebP file `config`
/// encodes, or why libwebp cannot encode it.
fn still(width: u32, height: u32, rgba: &[u8], config: &w::WebPConfig, index: usize) -> Result<Vec<u8>, String> {
    let stride = width as usize * 4;
    if rgba.len() != stride * height as usize {
        return Err(format!("frame {index} holds {} bytes, not the {} of {width} by {height} RGBA", rgba.len(), stride * height as usize));
    }
    // SAFETY: the writer is plain data that WebPMemoryWriterInit fills.
    let mut written = Written(unsafe { std::mem::zeroed() });
    // SAFETY: `written` is valid for writes.
    unsafe { w::WebPMemoryWriterInit(&mut written.0) };
    // SAFETY: the picture is plain data that the init call fills.
    let mut picture = Picture(unsafe { std::mem::zeroed() });
    // SAFETY: `picture` is valid for writes, at the ABI the bindings carry.
    if unsafe { w::WebPPictureInitInternal(&mut picture.0, w::WEBP_ENCODER_ABI_VERSION as i32) } == 0 {
        return Err(format!("frame {index}'s picture does not initialize"));
    }
    picture.0.use_argb = 1;
    picture.0.width = width as i32;
    picture.0.height = height as i32;
    // SAFETY: `rgba` holds `height` rows of `stride` bytes, which libwebp
    // copies into the picture.
    if unsafe { w::WebPPictureImportRGBA(&mut picture.0, rgba.as_ptr(), stride as i32) } == 0 {
        return Err(format!("frame {index} does not import into libwebp's picture"));
    }
    picture.0.writer = Some(w::WebPMemoryWrite);
    picture.0.custom_ptr = (&mut written.0 as *mut w::WebPMemoryWriter).cast();
    // SAFETY: `config` is validated and `picture` initialized, and the
    // writer it names outlives the call.
    if unsafe { w::WebPEncode(config, &mut picture.0) } == 0 {
        return Err(format!("libwebp cannot encode frame {index}: {:?}", picture.0.error_code));
    }
    if written.0.mem.is_null() {
        return Err(format!("libwebp wrote nothing for frame {index}"));
    }
    // SAFETY: the writer holds `size` bytes at `mem`, which are copied
    // before it is cleared.
    Ok(unsafe { slice::from_raw_parts(written.0.mem, written.0.size) }.to_vec())
}

/// `frames`, each `width` by `height` RGBA and shown for its `delays` in
/// milliseconds, as a WebP file `encoding` writes: a still file for one
/// frame, and for more an animation that plays `plays` times, 0 for
/// endlessly, of every frame encoded whole and replacing the one before
/// it. Or why libwebp cannot write them.
pub(crate) fn encode(width: u32, height: u32, frames: &[&[u8]], delays: &[u32], plays: u32, encoding: Encoding) -> Result<Vec<u8>, String> {
    if width > w::WEBP_MAX_DIMENSION || height > w::WEBP_MAX_DIMENSION {
        return Err(format!("a WebP file holds at most {0} by {0} pixels, and the frames are {width} by {height}", w::WEBP_MAX_DIMENSION));
    }
    if plays > u32::from(u16::MAX) {
        return Err(format!("a WebP file plays at most {} times, or endlessly", u16::MAX));
    }
    if let Some(&delay) = delays.iter().find(|&&delay| delay > LONGEST_FRAME) {
        return Err(format!("a WebP frame shows for at most {LONGEST_FRAME} milliseconds, and one is to show for {delay}"));
    }
    // SAFETY: the config is plain data that the init call fills.
    let mut config: w::WebPConfig = unsafe { std::mem::zeroed() };
    // A lossless file takes libwebp's own default, 75, as its effort.
    let quality = encoding.quality.unwrap_or(75.0);
    // SAFETY: `config` is valid for writes, at the ABI the bindings carry.
    if unsafe { w::WebPConfigInitInternal(&mut config, w::WebPPreset::WEBP_PRESET_DEFAULT, quality, w::WEBP_ENCODER_ABI_VERSION as i32) } == 0 {
        return Err("libwebp's encoder settings do not initialize".to_string());
    }
    config.lossless = i32::from(encoding.quality.is_none());
    config.exact = i32::from(encoding.quality.is_none());
    config.method = encoding.method;
    // SAFETY: `config` is initialized.
    if unsafe { w::WebPValidateConfig(&config) } == 0 {
        return Err(format!("libwebp refuses the settings {encoding:?}"));
    }
    if let [only] = frames {
        return still(width, height, only, &config, 0);
    }
    let mux = Mux(w::WebPMuxNew());
    if mux.0.is_null() {
        return Err("libwebp cannot start a mux".to_string());
    }
    for (index, (rgba, &delay)) in frames.iter().zip(delays).enumerate() {
        let bytes = still(width, height, rgba, &config, index)?;
        let frame = w::WebPMuxFrameInfo {
            bitstream: w::WebPData { bytes: bytes.as_ptr(), size: bytes.len() },
            x_offset: 0,
            y_offset: 0,
            duration: delay as i32,
            id: w::WebPChunkId::WEBP_CHUNK_ANMF,
            dispose_method: w::WebPMuxAnimDispose::WEBP_MUX_DISPOSE_NONE,
            blend_method: w::WebPMuxAnimBlend::WEBP_MUX_NO_BLEND,
            pad: [0],
        };
        // SAFETY: the mux is open, and it copies the frame's bytes.
        let pushed = unsafe { w::WebPMuxPushFrame(mux.0, &frame, 1) };
        if pushed != w::WebPMuxError::WEBP_MUX_OK {
            return Err(format!("libwebp's mux cannot take frame {index}: {pushed:?}"));
        }
    }
    // A transparent background, which viewers show where a frame is clear.
    let params = w::WebPMuxAnimParams { bgcolor: 0, loop_count: plays as i32 };
    // SAFETY: the mux is open.
    let set = unsafe { w::WebPMuxSetAnimationParams(mux.0, &params) };
    if set != w::WebPMuxError::WEBP_MUX_OK {
        return Err(format!("libwebp's mux cannot take the animation's settings: {set:?}"));
    }
    let mut data = w::WebPData { bytes: ptr::null(), size: 0 };
    // SAFETY: the mux is open and `data` is valid for writes.
    let assembled = unsafe { w::WebPMuxAssemble(mux.0, &mut data) };
    if assembled != w::WebPMuxError::WEBP_MUX_OK || data.bytes.is_null() {
        return Err(format!("libwebp's mux cannot assemble the file: {assembled:?}"));
    }
    // SAFETY: the mux hands over `data.size` bytes at `data.bytes`, which
    // are copied before WebPDataClear frees them.
    let bytes = unsafe { slice::from_raw_parts(data.bytes, data.size) }.to_vec();
    // SAFETY: `data` holds what WebPMuxAssemble allocated.
    unsafe { w::WebPDataClear(&mut data) };
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three 8 by 6 frames: red with a blue square, a green block over
    /// part of it, and a white bar on a clear canvas, whose transparent
    /// pixels hold a color.
    fn frames() -> Vec<Vec<u8>> {
        let canvas = |fill: [u8; 4]| -> Vec<u8> { (0..48).flat_map(|_| fill).collect() };
        let paint = |frame: &mut Vec<u8>, (x0, y0, x1, y1): (usize, usize, usize, usize), color: [u8; 4]| {
            for y in y0..y1 {
                for x in x0..x1 {
                    frame[(y * 8 + x) * 4..(y * 8 + x) * 4 + 4].copy_from_slice(&color);
                }
            }
        };
        let mut first = canvas([204, 51, 0, 255]);
        paint(&mut first, (1, 1, 4, 4), [51, 102, 204, 255]);
        let mut second = first.clone();
        paint(&mut second, (4, 2, 7, 5), [0, 153, 51, 255]);
        let mut third = canvas([10, 20, 30, 0]);
        paint(&mut third, (0, 5, 8, 6), [255, 255, 255, 255]);
        vec![first, second, third]
    }

    /// A lossless animation decodes into the frames it was made of, the
    /// colors of transparent pixels included, with their delays and plays;
    /// a frame like the one before it stays a frame of its own.
    #[test]
    fn writes_and_reads_a_lossless_animation_frame_for_frame() {
        let mut given = frames();
        given.insert(1, given[0].clone());
        let delays = [100, 150, 200, 300];
        let slices: Vec<&[u8]> = given.iter().map(Vec::as_slice).collect();
        let bytes = encode(8, 6, &slices, &delays, 3, Encoding { quality: None, method: 4 }).expect("encoded");
        let read = decode(&bytes).expect("decoded");
        assert_eq!((read.width, read.height, read.plays), (8, 6, 3));
        assert_eq!(read.delays, delays);
        assert_eq!(read.frames, given);
    }

    /// One frame makes a still file, which decodes as one frame.
    #[test]
    fn writes_one_frame_as_a_still_file() {
        let given = frames();
        let bytes = encode(8, 6, &[&given[0]], &[100], 0, Encoding { quality: None, method: 4 }).expect("encoded");
        assert!(!bytes.windows(4).any(|chunk| chunk == b"ANIM"), "the file holds an animation chunk");
        let read = decode(&bytes).expect("decoded");
        assert_eq!(read.frames, [given[0].clone()]);
    }

    /// A lossy file keeps every frame.
    #[test]
    fn writes_a_lossy_animation_at_its_quality() {
        let given = frames();
        let slices: Vec<&[u8]> = given.iter().map(Vec::as_slice).collect();
        let bytes = encode(8, 6, &slices, &[100, 200, 300], 0, Encoding { quality: Some(90.0), method: 6 }).expect("encoded");
        let read = decode(&bytes).expect("decoded");
        assert_eq!((read.frames.len(), read.plays), (3, 0));
    }

    #[test]
    fn refuses_what_a_webp_file_cannot_hold() {
        let given = frames();
        let refused = |result: Result<Vec<u8>, String>| match result {
            Ok(_) => panic!("written"),
            Err(why) => why,
        };
        let big = w::WEBP_MAX_DIMENSION + 1;
        assert_eq!(refused(encode(big, 1, &[&[0; 4]], &[100], 0, Encoding { quality: None, method: 4 })), format!("a WebP file holds at most 16383 by 16383 pixels, and the frames are {big} by 1"));
        assert_eq!(refused(encode(8, 6, &[&given[0]], &[100], 70_000, Encoding { quality: None, method: 4 })), "a WebP file plays at most 65535 times, or endlessly");
        assert_eq!(
            refused(encode(8, 6, &[&given[0], &given[1]], &[100, 1 << 24], 0, Encoding { quality: None, method: 4 })),
            "a WebP frame shows for at most 16777215 milliseconds, and one is to show for 16777216"
        );
    }
}
