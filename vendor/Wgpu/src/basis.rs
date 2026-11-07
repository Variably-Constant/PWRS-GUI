//! Basis Universal's KTX2 transcoder and encoder, and the zstd it ships,
//! as build.rs compiles them from vendor/basis_universal, through the C
//! APIs declared in encoder/basisu_wasm_transcoder_api.h and
//! encoder/basisu_wasm_api.h there. Those APIs pass memory as an address
//! in a u64. What basisu prints comes here through native/basisu_output.c:
//! its reason goes into the error of a call that fails, and anything else
//! it prints comes back as warnings.

use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_int};
use std::sync::{Arc, Once};

use arc_swap::ArcSwapOption;
use pwrs::prelude::*;

use crate::error::invalid;

unsafe extern "C" {
    fn bt_init();
    fn bt_ktx2_open(data: u64, len: u32) -> u64;
    fn bt_ktx2_close(handle: u64);
    fn bt_ktx2_get_width(handle: u64) -> u32;
    fn bt_ktx2_get_height(handle: u64) -> u32;
    fn bt_ktx2_get_levels(handle: u64) -> u32;
    fn bt_ktx2_get_faces(handle: u64) -> u32;
    fn bt_ktx2_get_layers(handle: u64) -> u32;
    fn bt_ktx2_get_basis_tex_format(handle: u64) -> u32;
    fn bt_ktx2_is_hdr(handle: u64) -> u32;
    fn bt_ktx2_is_srgb(handle: u64) -> u32;
    fn bt_ktx2_start_transcoding(handle: u64) -> u32;
    fn bt_ktx2_get_level_orig_width(handle: u64, level: u32, layer: u32, face: u32) -> u32;
    fn bt_ktx2_get_level_orig_height(handle: u64, level: u32, layer: u32, face: u32) -> u32;
    fn bt_basis_compute_transcoded_image_size_in_bytes(target: u32, width: u32, height: u32) -> u32;
    fn bt_basis_get_bytes_per_block_or_pixel(target: u32) -> u32;
    fn bt_basis_is_format_supported(target: u32, basis_format: u32) -> u32;
    #[allow(clippy::too_many_arguments)]
    fn bt_ktx2_transcode_image_level(
        handle: u64,
        level: u32,
        layer: u32,
        face: u32,
        output: u64,
        output_blocks_or_pixels: u32,
        target: u32,
        decode_flags: u32,
        row_pitch: u32,
        rows: u32,
        channel0: i32,
        channel1: i32,
        state: u64,
    ) -> u32;

    fn bu_init();
    fn bu_new_comp_params() -> u64;
    fn bu_delete_comp_params(params: u64) -> u32;
    fn bu_comp_params_set_image_rgba32(params: u64, index: u32, data: u64, width: u32, height: u32, pitch: u32) -> u32;
    fn bu_comp_params_set_image_float_rgba(params: u64, index: u32, data: u64, width: u32, height: u32, pitch: u32) -> u32;
    fn bu_compress_texture(params: u64, format: u32, quality: i32, effort: i32, flags: u64, rdo_or_dct_quality: f32) -> u32;
    fn bu_comp_params_get_comp_data_size(params: u64) -> u64;
    fn bu_comp_params_get_comp_data_ofs(params: u64) -> u64;

    fn ZSTD_decompress(dst: *mut u8, capacity: usize, src: *const u8, size: usize) -> usize;
    fn ZSTD_compress(dst: *mut u8, capacity: usize, src: *const u8, size: usize, level: i32) -> usize;
    fn ZSTD_compressBound(size: usize) -> usize;
    fn ZSTD_isError(code: usize) -> u32;
    fn ZSTD_getErrorName(code: usize) -> *const c_char;
}

/// A line basisu printed, and whether it printed it to stderr, where its
/// errors go.
#[derive(Clone)]
struct Line {
    error: bool,
    text: String,
}

thread_local! {
    /// The lines basisu prints on this thread while a call of this module
    /// runs it.
    static CAPTURE: RefCell<Option<Vec<Line>>> = const { RefCell::new(None) };
}

/// The lines basisu prints on threads no call of this module runs on: the
/// worker threads of its encoder.
static STRAY: ArcSwapOption<Vec<Line>> = ArcSwapOption::const_empty();

/// Where native/basisu_output.c sends the text of basisu's printf, vprintf,
/// fprintf and fputs calls for stdout and stderr.
#[unsafe(no_mangle)]
extern "C" fn wgpu_basisu_text(text: *const c_char, length: usize, is_error: c_int) {
    // SAFETY: basisu_output.c passes `length` bytes of text that outlive the call.
    let bytes = unsafe { std::slice::from_raw_parts(text.cast::<u8>(), length) };
    for part in String::from_utf8_lossy(bytes).lines() {
        let text = part.trim();
        if text.is_empty() {
            continue;
        }
        let line = Line { error: is_error != 0, text: text.to_string() };
        let uncaptured = CAPTURE.with(|capture| match capture.borrow_mut().as_mut() {
            Some(lines) => {
                lines.push(line);
                None
            }
            None => Some(line),
        });
        if let Some(line) = uncaptured {
            STRAY.rcu(|current| {
                let mut next = match current {
                    Some(lines) => Vec::clone(lines),
                    None => Vec::new(),
                };
                next.push(line.clone());
                Some(Arc::new(next))
            });
        }
    }
}

/// Runs `call` with what basisu prints collected: the lines printed on this
/// thread, and those its worker threads printed meanwhile. When two calls'
/// workers print at once, both lines go to whichever call finishes first.
fn printing<T>(call: impl FnOnce() -> T) -> (T, Vec<Line>) {
    CAPTURE.with(|capture| *capture.borrow_mut() = Some(Vec::new()));
    let value = call();
    let mut lines: Vec<Line> = CAPTURE.with(|capture| capture.borrow_mut().take()).into_iter().flatten().collect();
    if let Some(stray) = STRAY.swap(None) {
        lines.extend(stray.iter().cloned());
    }
    (value, lines)
}

/// A line without the `ERROR: ` or `WARNING: ` basisu starts some with.
fn plain(line: &Line) -> &str {
    line.text.trim_start_matches("ERROR: ").trim_start_matches("WARNING: ")
}

/// What basisu printed during a call that failed, its errors first, for
/// the call's error to give as its reason.
fn printed(lines: &[Line]) -> String {
    let reasons: Vec<&str> = lines.iter().filter(|l| l.error).chain(lines.iter().filter(|l| !l.error)).map(plain).collect();
    if reasons.is_empty() { "Basis Universal gave no reason".to_string() } else { reasons.join("; ") }
}

/// What basisu printed during a call that worked, added to `warnings`.
fn warn(lines: &[Line], warnings: &mut Vec<String>) {
    warnings.extend(lines.iter().map(|line| format!("Basis Universal: {}", plain(line))));
}

/// Transcode targets, basisu's transcoder_texture_format.
pub(crate) mod target {
    /// ETC1 blocks, which are ETC2 RGB8 blocks too.
    pub(crate) const ETC1_RGB: u32 = 0;
    pub(crate) const ETC2_RGBA: u32 = 1;
    pub(crate) const BC7_RGBA: u32 = 6;
    pub(crate) const ASTC_LDR_4X4_RGBA: u32 = 10;
    pub(crate) const RGBA32: u32 = 13;
    /// Unsigned EAC R11, from the red channel.
    pub(crate) const ETC2_EAC_R11: u32 = 20;
    /// Unsigned EAC RG11, from the red channel and alpha.
    pub(crate) const ETC2_EAC_RG11: u32 = 21;
    pub(crate) const BC6H: u32 = 22;
    pub(crate) const ASTC_HDR_4X4_RGBA: u32 = 23;
    pub(crate) const RGBA_HALF: u32 = 25;
    pub(crate) const ASTC_HDR_6X6_RGBA: u32 = 27;
    /// The ASTC LDR targets after 4x4, from 5x4 on, in the order of
    /// ASTC_LDR_SIZES.
    pub(crate) const ASTC_LDR_5X4_RGBA: u32 = 28;
}

/// The block sizes of the XUASTC and ASTC LDR Basis formats, in basisu's
/// order: BTF_XUASTC_LDR_4X4 is 5 and BTF_ASTC_LDR_4X4 19, each followed
/// by the 13 larger sizes.
pub(crate) const ASTC_LDR_SIZES: [(u32, u32); 14] = [(4, 4), (5, 4), (5, 5), (6, 5), (6, 6), (8, 5), (8, 6), (10, 5), (10, 6), (8, 8), (10, 8), (10, 10), (12, 10), (12, 12)];

/// Transcoding options, basisu's DECODE_FLAGS_.
pub(crate) mod decode {
    /// basisu's defaults: among them, the block edges of an ASTC or XUASTC
    /// LDR file of 80 texels a block or more are smoothed when the file is
    /// decoded to pixels, for the encoder wrote it expecting that.
    pub(crate) const DEFAULT: u32 = 0;
    /// No smoothing of block edges: the pixels are the blocks' own.
    #[cfg(test)]
    pub(crate) const NO_DEBLOCK_FILTERING: u32 = 128;
}

/// Encoding flags, basisu's BU_COMP_FLAGS_.
pub(crate) mod flags {
    pub(crate) const THREADED: u64 = 1 << 9;
    pub(crate) const KTX2_OUTPUT: u64 = 1 << 11;
    pub(crate) const KTX2_UASTC_ZSTD: u64 = 1 << 12;
    pub(crate) const SRGB: u64 = 1 << 13;
    pub(crate) const GEN_MIPS_CLAMP: u64 = 1 << 14;
    pub(crate) const TEXTURE_TYPE_2D_ARRAY: u64 = 1 << 25;
    pub(crate) const TEXTURE_TYPE_CUBEMAP_ARRAY: u64 = 2 << 25;
}

/// Builds basisu's transcoder and encoder tables, once for the process.
fn init() {
    static INIT: Once = Once::new();
    // SAFETY: both functions fill basisu's static tables and take no input.
    INIT.call_once(|| unsafe {
        bt_init();
        bu_init();
    });
}

/// A KTX2 file in one of Basis Universal's formats, held open by basisu's
/// transcoder over bytes this borrows for as long as it is open.
pub(crate) struct Ktx2<'a> {
    handle: u64,
    _data: &'a [u8],
}

impl<'a> Ktx2<'a> {
    /// Opens `data` as a KTX2 file of a Basis format; `None` for a file
    /// basisu's transcoder does not take, such as one of a GPU format.
    /// What basisu prints along the way joins `warnings`.
    pub(crate) fn open(data: &'a [u8], warnings: &mut Vec<String>) -> PsResult<Option<Self>> {
        init();
        let len = u32::try_from(data.len()).map_err(|e| invalid("WgpuKtx2", format!("a KTX2 file of {} bytes is larger than Basis Universal reads: {e}", data.len())))?;
        // SAFETY: `data` is valid for `len` bytes and outlives the handle,
        // which `Self` borrows it for and closes on drop.
        let (handle, lines) = printing(|| unsafe { bt_ktx2_open(data.as_ptr() as u64, len) });
        warn(&lines, warnings);
        if handle == 0 {
            return Ok(None);
        }
        let file = Ktx2 { handle, _data: data };
        // SAFETY: `handle` is open.
        let (started, lines) = printing(|| unsafe { bt_ktx2_start_transcoding(handle) });
        if started == 0 {
            return Err(invalid("WgpuKtx2", format!("Basis Universal cannot read the KTX2 file's supercompressed data: {}", printed(&lines))));
        }
        warn(&lines, warnings);
        Ok(Some(file))
    }

    pub(crate) fn width(&self) -> u32 {
        // SAFETY: `self.handle` is open for the life of `self`.
        unsafe { bt_ktx2_get_width(self.handle) }
    }

    pub(crate) fn height(&self) -> u32 {
        // SAFETY: `self.handle` is open for the life of `self`.
        unsafe { bt_ktx2_get_height(self.handle) }
    }

    pub(crate) fn levels(&self) -> u32 {
        // SAFETY: `self.handle` is open for the life of `self`.
        unsafe { bt_ktx2_get_levels(self.handle) }.max(1)
    }

    pub(crate) fn faces(&self) -> u32 {
        // SAFETY: `self.handle` is open for the life of `self`.
        unsafe { bt_ktx2_get_faces(self.handle) }.max(1)
    }

    /// The array layers; 1 for a file that is not an array.
    pub(crate) fn layers(&self) -> u32 {
        // SAFETY: `self.handle` is open for the life of `self`.
        unsafe { bt_ktx2_get_layers(self.handle) }.max(1)
    }

    /// The file's basis_tex_format, a BTF_ value.
    pub(crate) fn basis_format(&self) -> u32 {
        // SAFETY: `self.handle` is open for the life of `self`.
        unsafe { bt_ktx2_get_basis_tex_format(self.handle) }
    }

    pub(crate) fn is_hdr(&self) -> bool {
        // SAFETY: `self.handle` is open for the life of `self`.
        unsafe { bt_ktx2_is_hdr(self.handle) != 0 }
    }

    pub(crate) fn is_srgb(&self) -> bool {
        // SAFETY: `self.handle` is open for the life of `self`.
        unsafe { bt_ktx2_is_srgb(self.handle) != 0 }
    }

    /// Whether the file's format transcodes to `target`.
    pub(crate) fn supports(&self, target: u32) -> bool {
        // SAFETY: a lookup in basisu's format tables.
        unsafe { bt_basis_is_format_supported(target, self.basis_format()) != 0 }
    }

    /// Mip level `level` of array layer `layer` and cube face `face`,
    /// transcoded to `target` with the options `decode_flags`, rows of
    /// blocks or pixels packed. What basisu prints joins `warnings`.
    pub(crate) fn transcode(&self, level: u32, layer: u32, face: u32, target: u32, decode_flags: u32, warnings: &mut Vec<String>) -> PsResult<Vec<u8>> {
        // SAFETY: `self.handle` is open, and the level, layer and face
        // are the transcoder's to check.
        let (width, height) = unsafe { (bt_ktx2_get_level_orig_width(self.handle, level, layer, face), bt_ktx2_get_level_orig_height(self.handle, level, layer, face)) };
        // SAFETY: lookups in basisu's format tables.
        let (size, unit) = unsafe { (bt_basis_compute_transcoded_image_size_in_bytes(target, width, height), bt_basis_get_bytes_per_block_or_pixel(target)) };
        if size == 0 || unit == 0 {
            return Err(invalid("WgpuKtx2", format!("Basis Universal cannot transcode level {level} of the KTX2 file to the format chosen")));
        }
        let mut out = vec![0u8; size as usize];
        // SAFETY: `out` holds `size` bytes, which is `size / unit` blocks or
        // pixels of `target`, and outlives the call; state 0 is the
        // handle's own.
        let (ok, lines) = printing(|| unsafe { bt_ktx2_transcode_image_level(self.handle, level, layer, face, out.as_mut_ptr() as u64, size / unit, target, decode_flags, 0, 0, -1, -1, 0) });
        if ok == 0 {
            return Err(invalid("WgpuKtx2", format!("Basis Universal could not transcode level {level}, layer {layer}, face {face} of the KTX2 file: {}", printed(&lines))));
        }
        warn(&lines, warnings);
        Ok(out)
    }
}

impl Drop for Ktx2<'_> {
    fn drop(&mut self) {
        // SAFETY: `self.handle` was opened by bt_ktx2_open and is closed once.
        unsafe { bt_ktx2_close(self.handle) }
    }
}

/// The pixels of one image an encoding takes, rows packed.
pub(crate) enum Pixels<'a> {
    Rgba8(&'a [u8]),
    RgbaFloat(&'a [f32]),
}

/// One image an encoding takes: a layer, or a cube face, of mip level 0.
pub(crate) struct Image<'a> {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: Pixels<'a>,
}

/// basisu's compression parameters, deleted on drop.
struct Params(u64);

impl Drop for Params {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from bu_new_comp_params and is deleted once.
        unsafe { bu_delete_comp_params(self.0) };
    }
}

/// The level basisu's encoder starts with for `basis_format`, which its C
/// API reads from the low byte of the flags whenever a quality or an
/// effort is -1: the quality of ETC1S (BASISU_DEFAULT_QUALITY), the pack
/// level of UASTC LDR 4x4 (cPackUASTCLevelDefault), the level of UASTC HDR
/// 4x4 (cDefaultLevel) and of the HDR 6x6 formats
/// (ASTC_HDR_6X6_DEF_USER_COMP_LEVEL), and the effort of the XUASTC and
/// ASTC LDR formats (astc_ldr::EFFORT_LEVEL_DEF) and of XUBC7
/// (xbc7::DEFAULT_EFFORT_LEVEL).
fn default_level(basis_format: u32) -> u64 {
    match basis_format {
        0 => 128,
        1 => 2,
        2 => 1,
        3 | 4 => 2,
        _ => 3,
    }
}

/// `images`, one for each layer or cube face in order, encoded in the
/// Basis format `basis_format` (a BTF_ value) into the bytes of a KTX2
/// file. `quality` (1-100) and `effort` (0-10) are basisu's own, and -1
/// takes basisu's default for the format. What basisu prints joins
/// `warnings`.
pub(crate) fn encode_ktx2(images: &[Image], basis_format: u32, quality: i32, effort: i32, flags: u64, warnings: &mut Vec<String>) -> PsResult<Vec<u8>> {
    init();
    // SAFETY: allocates a parameter set, owned by `params` from here on.
    let params = Params(unsafe { bu_new_comp_params() });
    if params.0 == 0 {
        return Err(invalid("WgpuBasisEncode", "Basis Universal could not allocate its compression parameters"));
    }
    for (index, image) in images.iter().enumerate() {
        let index = index as u32;
        let (ok, lines) = printing(|| match image.pixels {
            Pixels::Rgba8(bytes) => {
                debug_assert_eq!(bytes.len(), image.width as usize * image.height as usize * 4);
                // SAFETY: `bytes` holds `width * height` RGBA8 pixels, rows
                // packed; basisu copies them.
                unsafe { bu_comp_params_set_image_rgba32(params.0, index, bytes.as_ptr() as u64, image.width, image.height, image.width * 4) }
            }
            Pixels::RgbaFloat(floats) => {
                debug_assert_eq!(floats.len(), image.width as usize * image.height as usize * 4);
                // SAFETY: `floats` holds `width * height` RGBA32F pixels,
                // rows packed; basisu copies them.
                unsafe { bu_comp_params_set_image_float_rgba(params.0, index, floats.as_ptr() as u64, image.width, image.height, image.width * 16) }
            }
        });
        if ok == 0 {
            return Err(invalid("WgpuBasisEncode", format!("Basis Universal did not take image {index}, {}x{}: {}", image.width, image.height, printed(&lines))));
        }
        warn(&lines, warnings);
    }
    let flags = flags | self::flags::KTX2_OUTPUT | default_level(basis_format);
    // SAFETY: `params.0` holds the images just set.
    let (ok, lines) = printing(|| unsafe { bu_compress_texture(params.0, basis_format, quality, effort, flags, 0.0) });
    if ok == 0 {
        return Err(invalid("WgpuBasisEncode", format!("Basis Universal could not encode the texture in the format chosen: {}", printed(&lines))));
    }
    warn(&lines, warnings);
    // SAFETY: after a successful compression `params.0` holds the encoded
    // bytes at the address and length these return, until it is deleted.
    let encoded = unsafe {
        let (len, address) = (bu_comp_params_get_comp_data_size(params.0), bu_comp_params_get_comp_data_ofs(params.0));
        std::slice::from_raw_parts(address as usize as *const u8, len as usize).to_vec()
    };
    Ok(encoded)
}

/// The zstd error a result code carries.
fn zstd_error(code: usize) -> String {
    // SAFETY: ZSTD_getErrorName returns a static NUL-terminated string.
    unsafe { CStr::from_ptr(ZSTD_getErrorName(code)) }.to_string_lossy().into_owned()
}

/// A zstd frame decompressed into `size` bytes, the size the KTX2 level
/// index records.
pub(crate) fn zstd_decompress(frame: &[u8], size: usize) -> PsResult<Vec<u8>> {
    let mut out = vec![0u8; size];
    // SAFETY: `out` holds `size` writable bytes and `frame` is readable.
    let written = unsafe { ZSTD_decompress(out.as_mut_ptr(), size, frame.as_ptr(), frame.len()) };
    // SAFETY: ZSTD_isError reads a result code.
    if unsafe { ZSTD_isError(written) } != 0 {
        return Err(invalid("WgpuKtx2", format!("a zstd level of the KTX2 file cannot be decompressed: {}", zstd_error(written))));
    }
    if written != size {
        return Err(invalid("WgpuKtx2", format!("a zstd level of the KTX2 file holds {written} bytes where its index records {size}")));
    }
    Ok(out)
}

/// `data` compressed into one zstd frame at `level`.
pub(crate) fn zstd_compress(data: &[u8], level: i32) -> PsResult<Vec<u8>> {
    // SAFETY: a size calculation.
    let bound = unsafe { ZSTD_compressBound(data.len()) };
    let mut out = vec![0u8; bound];
    // SAFETY: `out` holds `bound` writable bytes, zstd's worst case for
    // `data`, which is readable.
    let written = unsafe { ZSTD_compress(out.as_mut_ptr(), bound, data.as_ptr(), data.len(), level) };
    // SAFETY: ZSTD_isError reads a result code.
    if unsafe { ZSTD_isError(written) } != 0 {
        return Err(invalid("WgpuKtx2", format!("a level cannot be compressed with zstd: {}", zstd_error(written))));
    }
    out.truncate(written);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zstd_round_trips() {
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        let frame = zstd_compress(&data, 3).expect("compressed");
        assert!(frame.len() < data.len());
        assert_eq!(zstd_decompress(&frame, data.len()).expect("decompressed"), data);
    }

    #[test]
    fn encodes_uastc_into_ktx2_and_transcodes_it_back() {
        let (width, height) = (8u32, 8u32);
        let pixels: Vec<u8> = (0..width * height).flat_map(|i| [(i * 4) as u8, (255 - i * 4) as u8, 128, 255]).collect();
        let image = Image { width, height, pixels: Pixels::Rgba8(&pixels) };
        let mut warnings = Vec::new();
        let file = encode_ktx2(&[image], 1, -1, -1, flags::KTX2_UASTC_ZSTD, &mut warnings).expect("encoded");
        let ktx2 = Ktx2::open(&file, &mut warnings).expect("read").expect("a Basis KTX2 file");
        assert_eq!((ktx2.width(), ktx2.height(), ktx2.levels(), ktx2.basis_format()), (8, 8, 1, 1));
        let rgba = ktx2.transcode(0, 0, 0, target::RGBA32, decode::DEFAULT, &mut warnings).expect("transcoded");
        assert_eq!(rgba.len(), pixels.len());
        let worst = rgba.iter().zip(&pixels).map(|(a, b)| a.abs_diff(*b)).max().expect("pixels");
        assert!(worst <= 16, "UASTC moved a channel by {worst}");
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn takes_bytes_that_are_no_ktx2_file_for_none() {
        assert!(Ktx2::open(b"not a KTX2 file at all", &mut Vec::new()).expect("no error").is_none());
    }

    /// basisu prints a warning when its fastest ASTC 4x4 encoder is asked
    /// to weigh channels, as sRGB content does: it arrives as a warning.
    #[test]
    fn hands_what_basisu_prints_back_as_warnings() {
        let pixels = vec![200u8; 8 * 8 * 4];
        let image = Image { width: 8, height: 8, pixels: Pixels::Rgba8(&pixels) };
        let mut warnings = Vec::new();
        encode_ktx2(&[image], 19, -1, 0, flags::SRGB, &mut warnings).expect("encoded");
        assert!(warnings.iter().any(|w| w.starts_with("Basis Universal: Fast (effort 0)")), "{warnings:?}");
    }

    /// basisu prints why it refuses five images as a cube map: the error
    /// says so.
    #[test]
    fn puts_basisu_s_reason_into_its_errors() {
        let pixels = vec![9u8; 4 * 4 * 4];
        let faces: Vec<Image> = (0..5).map(|_| Image { width: 4, height: 4, pixels: Pixels::Rgba8(&pixels) }).collect();
        let Err(error) = encode_ktx2(&faces, 1, -1, -1, flags::TEXTURE_TYPE_CUBEMAP_ARRAY, &mut Vec::new()) else {
            panic!("five faces were taken as a cube map");
        };
        assert!(error.message.contains("not a multiple of 6"), "{}", error.message);
    }
}
