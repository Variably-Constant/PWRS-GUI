//! Texture files read and written whole, DDS and KTX2: every mip level,
//! array layer and cube face, in the file's own format. A KTX2 file in a
//! Basis Universal format is transcoded to the best format the device
//! takes, and a block-compressed texture the device cannot hold as it is
//! is decoded on the CPU.

use std::io::{Cursor, Read};

use ddsfile::{AlphaMode, Caps2, D3D10ResourceDimension, D3DFormat, Dds, DxgiFormat, MiscFlag, NewDxgiParams};
use pwrs::prelude::*;
use wgpu::{AstcBlock, AstcChannel, TextureDimension, TextureFormat as F};

use crate::basis::{self, Ktx2, target};
use crate::error::invalid;
use crate::names::feature_names;
use crate::texture::format_name;

/// What reading, decoding or writing a texture file says besides its
/// result: lines for the verbose stream, and warnings, how its mip levels
/// below level 0 were made, and whether a conversion wrote its normals
/// again for a format that stores them the other way.
#[derive(Default, Debug)]
pub(crate) struct Said {
    pub(crate) verbose: Vec<String>,
    pub(crate) warnings: Vec<String>,
    pub(crate) mips: MipsMade,
    pub(crate) normals_rewritten: bool,
}

/// How a call's mip levels below level 0 were made, which says whether
/// the mip filter and edge it named were used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MipsMade {
    /// None was rendered: the file had every level the call asked for.
    #[default]
    NoneRendered,
    /// They were rendered on the GPU with the call's filter and edge.
    Rendered,
    /// Basis Universal made them from level 0 with its own filter.
    ByBasis,
}

/// A texture as a file holds it.
pub(crate) struct TextureFile {
    pub(crate) dimension: TextureDimension,
    pub(crate) width: u32,
    pub(crate) height: u32,
    /// The depth slices of a 3D texture; 1 otherwise.
    pub(crate) depth: u32,
    /// The array layers, six for each cube; 1 for a single image.
    pub(crate) layers: u32,
    pub(crate) cube: bool,
    pub(crate) format: F,
    /// Each mip level's texels: every layer in order, and in each its
    /// slices and rows of blocks, packed.
    pub(crate) levels: Vec<Vec<u8>>,
}

impl TextureFile {
    /// The bytes one layer of mip level `level` holds.
    pub(crate) fn layer_len(&self, level: usize) -> usize {
        self.levels[level].len() / self.layers as usize
    }

    /// The depth slices mip level `level` of a 3D texture has; 1 for any
    /// other texture.
    fn slices(&self, level: u32) -> u32 {
        if self.dimension == TextureDimension::D3 { (self.depth >> level).max(1) } else { 1 }
    }
}

/// The bytes mip level `level` of one layer takes, or a refusal for a
/// format whose texels have no layout outside the GPU.
pub(crate) fn level_len(format: F, width: u32, height: u32, depth: u32, level: u32) -> PsResult<usize> {
    let block = format.block_copy_size(None).ok_or_else(|| invalid("WgpuTextureFile", format!("a {} texture has no layout a file can hold", format_name(format))))?;
    let (bw, bh) = format.block_dimensions();
    let w = (width >> level).max(1);
    let h = (height >> level).max(1);
    let d = (depth >> level).max(1);
    Ok(w.div_ceil(bw) as usize * h.div_ceil(bh) as usize * d as usize * block as usize)
}

/// Each format a file can hold: wgpu's, Vulkan's VkFormat (KTX2) and,
/// where Direct3D has one, DXGI's (DDS).
const FORMATS: &[(F, u32, Option<DxgiFormat>)] = &[
    (F::R8Unorm, 9, Some(DxgiFormat::R8_UNorm)),
    (F::R8Snorm, 10, Some(DxgiFormat::R8_SNorm)),
    (F::R8Uint, 13, Some(DxgiFormat::R8_UInt)),
    (F::R8Sint, 14, Some(DxgiFormat::R8_SInt)),
    (F::Rg8Unorm, 16, Some(DxgiFormat::R8G8_UNorm)),
    (F::Rg8Snorm, 17, Some(DxgiFormat::R8G8_SNorm)),
    (F::Rg8Uint, 20, Some(DxgiFormat::R8G8_UInt)),
    (F::Rg8Sint, 21, Some(DxgiFormat::R8G8_SInt)),
    (F::Rgba8Unorm, 37, Some(DxgiFormat::R8G8B8A8_UNorm)),
    (F::Rgba8Snorm, 38, Some(DxgiFormat::R8G8B8A8_SNorm)),
    (F::Rgba8Uint, 41, Some(DxgiFormat::R8G8B8A8_UInt)),
    (F::Rgba8Sint, 42, Some(DxgiFormat::R8G8B8A8_SInt)),
    (F::Rgba8UnormSrgb, 43, Some(DxgiFormat::R8G8B8A8_UNorm_sRGB)),
    (F::Bgra8Unorm, 44, Some(DxgiFormat::B8G8R8A8_UNorm)),
    (F::Bgra8UnormSrgb, 50, Some(DxgiFormat::B8G8R8A8_UNorm_sRGB)),
    (F::Rgb10a2Unorm, 64, Some(DxgiFormat::R10G10B10A2_UNorm)),
    (F::Rgb10a2Uint, 68, Some(DxgiFormat::R10G10B10A2_UInt)),
    (F::R16Unorm, 70, Some(DxgiFormat::R16_UNorm)),
    (F::R16Snorm, 71, Some(DxgiFormat::R16_SNorm)),
    (F::R16Uint, 74, Some(DxgiFormat::R16_UInt)),
    (F::R16Sint, 75, Some(DxgiFormat::R16_SInt)),
    (F::R16Float, 76, Some(DxgiFormat::R16_Float)),
    (F::Rg16Unorm, 77, Some(DxgiFormat::R16G16_UNorm)),
    (F::Rg16Snorm, 78, Some(DxgiFormat::R16G16_SNorm)),
    (F::Rg16Uint, 81, Some(DxgiFormat::R16G16_UInt)),
    (F::Rg16Sint, 82, Some(DxgiFormat::R16G16_SInt)),
    (F::Rg16Float, 83, Some(DxgiFormat::R16G16_Float)),
    (F::Rgba16Unorm, 91, Some(DxgiFormat::R16G16B16A16_UNorm)),
    (F::Rgba16Snorm, 92, Some(DxgiFormat::R16G16B16A16_SNorm)),
    (F::Rgba16Uint, 95, Some(DxgiFormat::R16G16B16A16_UInt)),
    (F::Rgba16Sint, 96, Some(DxgiFormat::R16G16B16A16_SInt)),
    (F::Rgba16Float, 97, Some(DxgiFormat::R16G16B16A16_Float)),
    (F::R32Uint, 98, Some(DxgiFormat::R32_UInt)),
    (F::R32Sint, 99, Some(DxgiFormat::R32_SInt)),
    (F::R32Float, 100, Some(DxgiFormat::R32_Float)),
    (F::Rg32Uint, 101, Some(DxgiFormat::R32G32_UInt)),
    (F::Rg32Sint, 102, Some(DxgiFormat::R32G32_SInt)),
    (F::Rg32Float, 103, Some(DxgiFormat::R32G32_Float)),
    (F::Rgba32Uint, 107, Some(DxgiFormat::R32G32B32A32_UInt)),
    (F::Rgba32Sint, 108, Some(DxgiFormat::R32G32B32A32_SInt)),
    (F::Rgba32Float, 109, Some(DxgiFormat::R32G32B32A32_Float)),
    (F::Rg11b10Ufloat, 122, Some(DxgiFormat::R11G11B10_Float)),
    (F::Rgb9e5Ufloat, 123, Some(DxgiFormat::R9G9B9E5_SharedExp)),
    (F::Depth16Unorm, 124, Some(DxgiFormat::D16_UNorm)),
    (F::Depth32Float, 126, Some(DxgiFormat::D32_Float)),
    (F::Stencil8, 127, None),
    (F::Bc1RgbaUnorm, 133, Some(DxgiFormat::BC1_UNorm)),
    (F::Bc1RgbaUnormSrgb, 134, Some(DxgiFormat::BC1_UNorm_sRGB)),
    (F::Bc2RgbaUnorm, 135, Some(DxgiFormat::BC2_UNorm)),
    (F::Bc2RgbaUnormSrgb, 136, Some(DxgiFormat::BC2_UNorm_sRGB)),
    (F::Bc3RgbaUnorm, 137, Some(DxgiFormat::BC3_UNorm)),
    (F::Bc3RgbaUnormSrgb, 138, Some(DxgiFormat::BC3_UNorm_sRGB)),
    (F::Bc4RUnorm, 139, Some(DxgiFormat::BC4_UNorm)),
    (F::Bc4RSnorm, 140, Some(DxgiFormat::BC4_SNorm)),
    (F::Bc5RgUnorm, 141, Some(DxgiFormat::BC5_UNorm)),
    (F::Bc5RgSnorm, 142, Some(DxgiFormat::BC5_SNorm)),
    (F::Bc6hRgbUfloat, 143, Some(DxgiFormat::BC6H_UF16)),
    (F::Bc6hRgbFloat, 144, Some(DxgiFormat::BC6H_SF16)),
    (F::Bc7RgbaUnorm, 145, Some(DxgiFormat::BC7_UNorm)),
    (F::Bc7RgbaUnormSrgb, 146, Some(DxgiFormat::BC7_UNorm_sRGB)),
    (F::Etc2Rgb8Unorm, 147, None),
    (F::Etc2Rgb8UnormSrgb, 148, None),
    (F::Etc2Rgb8A1Unorm, 149, None),
    (F::Etc2Rgb8A1UnormSrgb, 150, None),
    (F::Etc2Rgba8Unorm, 151, None),
    (F::Etc2Rgba8UnormSrgb, 152, None),
    (F::EacR11Unorm, 153, None),
    (F::EacR11Snorm, 154, None),
    (F::EacRg11Unorm, 155, None),
    (F::EacRg11Snorm, 156, None),
];

/// The ASTC block sizes, in the order VkFormat numbers them.
const ASTC_BLOCKS: [AstcBlock; 14] = [
    AstcBlock::B4x4,
    AstcBlock::B5x4,
    AstcBlock::B5x5,
    AstcBlock::B6x5,
    AstcBlock::B6x6,
    AstcBlock::B8x5,
    AstcBlock::B8x6,
    AstcBlock::B8x8,
    AstcBlock::B10x5,
    AstcBlock::B10x6,
    AstcBlock::B10x8,
    AstcBlock::B10x10,
    AstcBlock::B12x10,
    AstcBlock::B12x12,
];

/// VkFormat's first ASTC LDR format, ASTC_4x4_UNORM_BLOCK; each size has
/// its UNORM code then its SRGB code.
const VK_ASTC_LDR: u32 = 157;
/// VkFormat's first ASTC HDR format, ASTC_4x4_SFLOAT_BLOCK.
const VK_ASTC_HDR: u32 = 1_000_066_000;

/// The VkFormat KTX2 records for `format`.
pub(crate) fn vk_format(format: F) -> Option<u32> {
    if let F::Astc { block, channel } = format {
        let index = ASTC_BLOCKS.iter().position(|b| *b == block)? as u32;
        return Some(match channel {
            AstcChannel::Unorm => VK_ASTC_LDR + 2 * index,
            AstcChannel::UnormSrgb => VK_ASTC_LDR + 2 * index + 1,
            AstcChannel::Hdr => VK_ASTC_HDR + index,
        });
    }
    FORMATS.iter().find(|(f, _, _)| *f == format).map(|(_, vk, _)| *vk)
}

/// The format a KTX2 file's VkFormat names. BC1's RGB codes read as its
/// RGBA formats, the only BC1 formats wgpu has.
fn from_vk(code: u32) -> Option<F> {
    if (VK_ASTC_LDR..VK_ASTC_LDR + 28).contains(&code) {
        let index = (code - VK_ASTC_LDR) as usize;
        let channel = if index.is_multiple_of(2) { AstcChannel::Unorm } else { AstcChannel::UnormSrgb };
        return Some(F::Astc { block: ASTC_BLOCKS[index / 2], channel });
    }
    if (VK_ASTC_HDR..VK_ASTC_HDR + 14).contains(&code) {
        return Some(F::Astc { block: ASTC_BLOCKS[(code - VK_ASTC_HDR) as usize], channel: AstcChannel::Hdr });
    }
    match code {
        131 => Some(F::Bc1RgbaUnorm),
        132 => Some(F::Bc1RgbaUnormSrgb),
        _ => FORMATS.iter().find(|(_, vk, _)| *vk == code).map(|(f, _, _)| *f),
    }
}

/// The DXGI format a DDS file records for `format`.
pub(crate) fn dxgi_format(format: F) -> Option<DxgiFormat> {
    FORMATS.iter().find(|(f, _, _)| *f == format).and_then(|(_, _, dxgi)| *dxgi)
}

fn from_dxgi(format: DxgiFormat) -> Option<F> {
    FORMATS.iter().find(|(_, _, dxgi)| *dxgi == Some(format)).map(|(f, _, _)| *f)
}

/// The format of a DDS file without a DX10 header, from its legacy
/// Direct3D format, read as linear: such a file records no color space.
fn from_d3d(format: D3DFormat) -> Option<F> {
    Some(match format {
        D3DFormat::DXT1 => F::Bc1RgbaUnorm,
        D3DFormat::DXT2 | D3DFormat::DXT3 => F::Bc2RgbaUnorm,
        D3DFormat::DXT4 | D3DFormat::DXT5 => F::Bc3RgbaUnorm,
        D3DFormat::A8B8G8R8 => F::Rgba8Unorm,
        D3DFormat::A8R8G8B8 => F::Bgra8Unorm,
        D3DFormat::G16R16 => F::Rg16Unorm,
        D3DFormat::A16B16G16R16 => F::Rgba16Unorm,
        D3DFormat::A2B10G10R10 => F::Rgb10a2Unorm,
        D3DFormat::L8 => F::R8Unorm,
        D3DFormat::A8L8 => F::Rg8Unorm,
        D3DFormat::L16 => F::R16Unorm,
        D3DFormat::R16F => F::R16Float,
        D3DFormat::G16R16F => F::Rg16Float,
        D3DFormat::A16B16G16R16F => F::Rgba16Float,
        D3DFormat::R32F => F::R32Float,
        D3DFormat::G32R32F => F::Rg32Float,
        D3DFormat::A32B32G32R32F => F::Rgba32Float,
        _ => return None,
    })
}

/// `format` as sRGB when `srgb` asks for it and the format has an sRGB
/// counterpart; otherwise `format` itself.
fn with_srgb(format: F, srgb: bool) -> F {
    if srgb { format.add_srgb_suffix() } else { format }
}

/// A file's `format` as -Srgb reads it: its sRGB counterpart when `srgb`
/// asks for one, with a warning when it has none or is sRGB already.
fn relabel(format: F, srgb: bool, said: &mut Said) -> F {
    if !srgb {
        return format;
    }
    let relabeled = format.add_srgb_suffix();
    if relabeled == format {
        let why = if format.is_srgb() { "is sRGB already" } else { "has no sRGB form" };
        said.warnings.push(format!("-Srgb did nothing for the file: its format, {}, {why}", format_name(format)));
    }
    relabeled
}

/// Reads a DDS file. `srgb` reads its color as sRGB where its format has
/// an sRGB counterpart, and a warning says when it has none.
pub(crate) fn read_dds(bytes: &[u8], srgb: bool, said: &mut Said) -> PsResult<TextureFile> {
    let dds = Dds::read(Cursor::new(bytes)).map_err(|e| invalid("WgpuTextureFile", format!("the file cannot be read as a DDS texture: {e}")))?;
    let found = match &dds.header10 {
        Some(h10) => from_dxgi(h10.dxgi_format),
        None => match dds.get_d3d_format() {
            Some(d3d) => from_d3d(d3d),
            None => dds.get_dxgi_format().and_then(from_dxgi).map(|f| f.remove_srgb_suffix()),
        },
    };
    let format = found.ok_or_else(|| invalid("WgpuTextureFile", "the DDS file's format is one the module does not read"))?;
    let format = relabel(format, srgb, said);
    let resource = dds.header10.as_ref().map(|h| h.resource_dimension);
    let volume = dds.header.caps2.contains(Caps2::VOLUME) || resource == Some(D3D10ResourceDimension::Texture3D);
    let dimension = match resource {
        Some(D3D10ResourceDimension::Texture1D) => TextureDimension::D1,
        _ if volume => TextureDimension::D3,
        _ => TextureDimension::D2,
    };
    let cube = dds.header.caps2.contains(Caps2::CUBEMAP) || dds.header10.as_ref().is_some_and(|h| h.misc_flag.contains(MiscFlag::TEXTURECUBE));
    let (width, height) = (dds.get_width(), dds.get_height().max(1));
    let depth = if volume { dds.get_depth().max(1) } else { 1 };
    let layers = dds.get_num_array_layers().max(1);
    let mips = dds.get_num_mipmap_levels().max(1);
    // The data holds each layer whole, its mip levels largest first.
    let mut levels = vec![Vec::new(); mips as usize];
    let mut at = 0;
    for layer in 0..layers {
        for (mip, level) in levels.iter_mut().enumerate() {
            let len = level_len(format, width, height, depth, mip as u32)?;
            let texels = dds.data.get(at..at + len).ok_or_else(|| invalid("WgpuTextureFile", format!("the DDS file ends inside mip level {mip} of layer {layer}")))?;
            level.extend_from_slice(texels);
            at += len;
        }
    }
    Ok(TextureFile { dimension, width, height, depth, layers, cube, format, levels })
}

/// Writes `file` as a DDS file with a DX10 header.
pub(crate) fn write_dds(file: &TextureFile) -> PsResult<Vec<u8>> {
    let format = dxgi_format(file.format).ok_or_else(|| invalid("WgpuTextureFile", format!("a DDS file cannot hold a {} texture", format_name(file.format))))?;
    let volume = file.dimension == TextureDimension::D3;
    let mut dds = Dds::new_dxgi(NewDxgiParams {
        height: file.height,
        width: file.width,
        depth: volume.then_some(file.depth),
        format,
        mipmap_levels: (file.levels.len() > 1).then_some(file.levels.len() as u32),
        array_layers: (file.layers > 1).then_some(file.layers),
        caps2: if file.cube {
            Some(Caps2::CUBEMAP | Caps2::CUBEMAP_ALLFACES)
        } else if volume {
            Some(Caps2::VOLUME)
        } else {
            None
        },
        is_cubemap: file.cube,
        resource_dimension: match file.dimension {
            TextureDimension::D1 => D3D10ResourceDimension::Texture1D,
            TextureDimension::D2 => D3D10ResourceDimension::Texture2D,
            TextureDimension::D3 => D3D10ResourceDimension::Texture3D,
        },
        alpha_mode: AlphaMode::Unknown,
    })
    .map_err(|e| invalid("WgpuTextureFile", format!("the DDS header cannot be made: {e}")))?;
    let mut data = Vec::with_capacity(dds.data.len());
    for layer in 0..file.layers as usize {
        for (mip, level) in file.levels.iter().enumerate() {
            let len = file.layer_len(mip);
            data.extend_from_slice(&level[layer * len..(layer + 1) * len]);
        }
    }
    if data.len() != dds.data.len() {
        return Err(invalid("WgpuTextureFile", format!("the texture's {} bytes do not fill the {} a DDS file of its shape holds", data.len(), dds.data.len())));
    }
    dds.data = data;
    let mut bytes = Vec::new();
    dds.write(&mut bytes).map_err(|e| invalid("WgpuTextureFile", format!("the DDS file cannot be written: {e}")))?;
    Ok(bytes)
}

/// Reads a KTX2 file. A file in a Basis Universal format is transcoded to
/// the best format `features` allow, which a verbose line says; `srgb`
/// reads the color of a file of a GPU format as sRGB where its format has
/// an sRGB counterpart. A warning says when `srgb` does nothing: for a
/// format with no sRGB form, and for a Basis Universal file, which records
/// whether its color is sRGB.
pub(crate) fn read_ktx2(bytes: &[u8], srgb: bool, features: wgpu::Features, said: &mut Said) -> PsResult<TextureFile> {
    if let Some(basis_file) = Ktx2::open(bytes, &mut said.warnings)? {
        if srgb {
            let recorded = if basis_file.is_srgb() { "sRGB" } else { "linear" };
            said.warnings.push(format!("-Srgb did nothing for the file: a Basis Universal file records whether its color is sRGB, and this one's is {recorded}"));
        }
        return read_basis(&basis_file, features, said);
    }
    let reader = ktx2::Reader::new(bytes).map_err(|e| invalid("WgpuTextureFile", format!("the file cannot be read as a KTX2 texture: {e}")))?;
    let header = reader.header();
    let code = header.format.ok_or_else(|| invalid("WgpuTextureFile", "the KTX2 file names no format and is not in a Basis Universal format"))?.value();
    let format = from_vk(code).ok_or_else(|| invalid("WgpuTextureFile", format!("the KTX2 file's format, VkFormat {code}, is one the module does not read")))?;
    let format = relabel(format, srgb, said);
    let dimension = if header.pixel_depth > 0 {
        TextureDimension::D3
    } else if header.pixel_height == 0 {
        TextureDimension::D1
    } else {
        TextureDimension::D2
    };
    let mut levels = Vec::with_capacity(header.level_count.max(1) as usize);
    for (mip, level) in reader.levels().enumerate() {
        let texels = match header.supercompression_scheme {
            None => level.data.to_vec(),
            Some(ktx2::SupercompressionScheme::Zstandard) => basis::zstd_decompress(level.data, level.uncompressed_byte_length as usize)?,
            Some(ktx2::SupercompressionScheme::ZLIB) => {
                let mut out = Vec::with_capacity(level.uncompressed_byte_length as usize);
                flate2::read::ZlibDecoder::new(level.data)
                    .read_to_end(&mut out)
                    .map_err(|e| invalid("WgpuTextureFile", format!("mip level {mip} of the KTX2 file cannot be inflated: {e}")))?;
                out
            }
            Some(other) => return Err(invalid("WgpuTextureFile", format!("the KTX2 file's supercompression, {other:?}, is one the module does not read for a GPU format"))),
        };
        levels.push(texels);
    }
    let file = TextureFile {
        dimension,
        width: header.pixel_width,
        height: header.pixel_height.max(1),
        depth: header.pixel_depth.max(1),
        layers: header.layer_count.max(1) * header.face_count,
        cube: header.face_count == 6,
        format,
        levels,
    };
    for (mip, level) in file.levels.iter().enumerate() {
        let expected = level_len(file.format, file.width, file.height, file.depth, mip as u32)? * file.layers as usize;
        if level.len() != expected {
            return Err(invalid("WgpuTextureFile", format!("mip level {mip} of the KTX2 file holds {} bytes where its shape takes {expected}", level.len())));
        }
    }
    Ok(file)
}

/// The ASTC block of `width` by `height` texels.
fn astc_block(width: u32, height: u32) -> PsResult<AstcBlock> {
    ASTC_BLOCKS
        .into_iter()
        .find(|&block| F::Astc { block, channel: AstcChannel::Unorm }.block_dimensions() == (width, height))
        .ok_or_else(|| invalid("WgpuTextureFile", format!("ASTC has no {width}x{height} block")))
}

/// A KTX2 file in a Basis Universal format, transcoded level by level to
/// the best format `features` allow: first the format the file's own
/// blocks become without being encoded again (ASTC at the file's block
/// size, ETC2 for ETC1S), then BC, ASTC and ETC2, then uncompressed.
fn read_basis(file: &Ktx2, features: wgpu::Features, said: &mut Said) -> PsResult<TextureFile> {
    use wgpu::Features as Need;
    let srgb = file.is_srgb();
    let (width, height) = (file.width(), file.height().max(1));
    let astc = |block, channel| F::Astc { block, channel };
    let mut choices: Vec<(Need, u32, F)> = Vec::new();
    match file.basis_format() {
        code @ 5..=32 => {
            let index = ((code - 5) % 14) as usize;
            let (w, h) = basis::ASTC_LDR_SIZES[index];
            let target = if index == 0 { target::ASTC_LDR_4X4_RGBA } else { target::ASTC_LDR_5X4_RGBA + index as u32 - 1 };
            choices.push((Need::TEXTURE_COMPRESSION_ASTC, target, astc(astc_block(w, h)?, AstcChannel::Unorm)));
        }
        0 => choices.push((Need::TEXTURE_COMPRESSION_ETC2, target::ETC2_RGBA, F::Etc2Rgba8Unorm)),
        1 => choices.push((Need::TEXTURE_COMPRESSION_ASTC, target::ASTC_LDR_4X4_RGBA, astc(AstcBlock::B4x4, AstcChannel::Unorm))),
        2 => choices.push((Need::TEXTURE_COMPRESSION_ASTC_HDR, target::ASTC_HDR_4X4_RGBA, astc(AstcBlock::B4x4, AstcChannel::Hdr))),
        3 | 4 => choices.push((Need::TEXTURE_COMPRESSION_ASTC_HDR, target::ASTC_HDR_6X6_RGBA, astc(AstcBlock::B6x6, AstcChannel::Hdr))),
        _ => {}
    }
    if file.is_hdr() {
        choices.extend([
            (Need::TEXTURE_COMPRESSION_BC, target::BC6H, F::Bc6hRgbUfloat),
            (Need::TEXTURE_COMPRESSION_ASTC_HDR, target::ASTC_HDR_4X4_RGBA, astc(AstcBlock::B4x4, AstcChannel::Hdr)),
            (Need::empty(), target::RGBA_HALF, F::Rgba16Float),
        ]);
    } else {
        choices.extend([
            (Need::TEXTURE_COMPRESSION_BC, target::BC7_RGBA, F::Bc7RgbaUnorm),
            (Need::TEXTURE_COMPRESSION_ASTC, target::ASTC_LDR_4X4_RGBA, astc(AstcBlock::B4x4, AstcChannel::Unorm)),
            (Need::TEXTURE_COMPRESSION_ETC2, target::ETC2_RGBA, F::Etc2Rgba8Unorm),
            (Need::empty(), target::RGBA32, F::Rgba8Unorm),
        ]);
    }
    // A block-compressed texture is a whole number of its blocks.
    let fits = |format: F| {
        let (bw, bh) = format.block_dimensions();
        width % bw == 0 && height % bh == 0
    };
    let takes = |need: Need, target: u32| features.contains(need) && file.supports(target);
    let (target, format) = choices
        .iter()
        .find(|&&(need, target, format)| takes(need, target) && fits(format))
        .map(|&(_, target, format)| (target, with_srgb(format, srgb)))
        .ok_or_else(|| invalid("WgpuTextureFile", "Basis Universal can transcode the KTX2 file to no format this device takes"))?;
    let unfit = !format.is_compressed() && choices.iter().any(|&(need, target, format)| format.is_compressed() && takes(need, target) && !fits(format));
    let (layers, faces) = (file.layers(), file.faces());
    let mut levels = Vec::with_capacity(file.levels() as usize);
    for level in 0..file.levels() {
        let mut texels = Vec::new();
        for layer in 0..layers {
            for face in 0..faces {
                texels.extend_from_slice(&file.transcode(level, layer, face, target, basis::decode::DEFAULT, &mut said.warnings)?);
            }
        }
        levels.push(texels);
    }
    let mut note = format!("the KTX2 file's Basis Universal texels were transcoded into {}", format_name(format));
    if unfit {
        note.push_str(&format!(", because its {width}x{height} size is not a whole number of the blocks of the compressed formats it could take"));
    }
    said.verbose.push(note);
    Ok(TextureFile { dimension: TextureDimension::D2, width, height, depth: 1, layers: layers * faces, cube: faces == 6, format, levels })
}

/// A KTX2 file in a Basis Universal format transcoded through `target`
/// into `format`, which the caller chooses rather than the device: the
/// first `levels` of its mip levels, with every layer and face.
pub(crate) fn transcode_ktx2(bytes: &[u8], target: u32, format: F, levels: u32, said: &mut Said) -> PsResult<TextureFile> {
    let file = Ktx2::open(bytes, &mut said.warnings)?.ok_or_else(|| invalid("WgpuTextureFile", "the KTX2 file is not in a Basis Universal format"))?;
    if !file.supports(target) {
        return Err(invalid("WgpuTextureFile", format!("Basis Universal cannot transcode the KTX2 file into {}", format_name(format))));
    }
    let (width, height) = (file.width(), file.height().max(1));
    let (layers, faces) = (file.layers(), file.faces());
    let mut out = Vec::with_capacity(file.levels().min(levels) as usize);
    for level in 0..file.levels().min(levels) {
        let mut texels = Vec::new();
        for layer in 0..layers {
            for face in 0..faces {
                texels.extend_from_slice(&file.transcode(level, layer, face, target, basis::decode::DEFAULT, &mut said.warnings)?);
            }
        }
        out.push(texels);
    }
    Ok(TextureFile { dimension: TextureDimension::D2, width, height, depth: 1, layers: layers * faces, cube: faces == 6, format, levels: out })
}

/// The key/value data the module writes: the KTXwriter entry, which the
/// KTX2 specification asks every writer to record.
fn key_values() -> Vec<u8> {
    let entry = concat!("KTXwriter\0Wgpu ", env!("CARGO_PKG_VERSION"), "\0").as_bytes();
    let mut out = (entry.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(entry);
    out.resize(out.len().next_multiple_of(4), 0);
    out
}

/// Writes `file` as a KTX2 file, its levels smallest first: as they are,
/// each aligned as the KTX2 specification requires, or each compressed
/// with Zstandard at level `zstd`.
pub(crate) fn write_ktx2(file: &TextureFile, zstd: Option<i32>) -> PsResult<Vec<u8>> {
    let vk = vk_format(file.format)
        .and_then(ktx2::Format::new)
        .ok_or_else(|| invalid("WgpuTextureFile", format!("a KTX2 file cannot hold a {} texture", format_name(file.format))))?;
    let (basic, type_size) =
        ktx2::dfd::Basic::from_format(vk).map_err(|e| invalid("WgpuTextureFile", format!("the KTX2 data format descriptor for {} cannot be made: {e}", format_name(file.format))))?;
    let block = file.format.block_copy_size(None).ok_or_else(|| invalid("WgpuTextureFile", format!("a {} texture has no layout a file can hold", format_name(file.format))))?;
    let descriptor = ktx2::dfd::Block::Basic(basic).to_vec();
    let mut dfd = ((4 + descriptor.len()) as u32).to_le_bytes().to_vec();
    dfd.extend_from_slice(&descriptor);
    let kvd = key_values();
    let level_count = file.levels.len();
    let dfd_offset = ktx2::Header::LENGTH + level_count * ktx2::LevelIndex::LENGTH;
    let kvd_offset = dfd_offset + dfd.len();
    let body_start = kvd_offset + kvd.len();
    let align = if zstd.is_some() { 1 } else { lcm(block as usize, 4) };
    let mut index = vec![ktx2::LevelIndex { byte_offset: 0, byte_length: 0, uncompressed_byte_length: 0 }; level_count];
    let mut body = Vec::new();
    for mip in (0..level_count).rev() {
        let start = (body_start + body.len()).next_multiple_of(align);
        body.resize(start - body_start, 0);
        let texels = &file.levels[mip];
        let compressed;
        let stored: &[u8] = match zstd {
            Some(level) => {
                compressed = basis::zstd_compress(texels, level)?;
                &compressed
            }
            None => texels,
        };
        index[mip] = ktx2::LevelIndex { byte_offset: start as u64, byte_length: stored.len() as u64, uncompressed_byte_length: texels.len() as u64 };
        body.extend_from_slice(stored);
    }
    let faces = if file.cube { 6 } else { 1 };
    let header = ktx2::Header {
        format: Some(vk),
        type_size,
        pixel_width: file.width,
        pixel_height: if file.dimension == TextureDimension::D1 { 0 } else { file.height },
        pixel_depth: if file.dimension == TextureDimension::D3 { file.depth } else { 0 },
        layer_count: if file.layers / faces > 1 { file.layers / faces } else { 0 },
        face_count: faces,
        level_count: level_count as u32,
        supercompression_scheme: zstd.map(|_| ktx2::SupercompressionScheme::Zstandard),
        index: ktx2::Index {
            dfd_byte_offset: dfd_offset as u32,
            dfd_byte_length: dfd.len() as u32,
            kvd_byte_offset: kvd_offset as u32,
            kvd_byte_length: kvd.len() as u32,
            sgd_byte_offset: 0,
            sgd_byte_length: 0,
        },
    };
    let mut out = Vec::with_capacity(body_start + body.len());
    out.extend_from_slice(&header.as_bytes());
    for entry in &index {
        out.extend_from_slice(&entry.as_bytes());
    }
    out.extend_from_slice(&dfd);
    out.extend_from_slice(&kvd);
    out.extend_from_slice(&body);
    Ok(out)
}

fn lcm(a: usize, b: usize) -> usize {
    let (mut x, mut y) = (a, b);
    while y != 0 {
        (x, y) = (y, x % y);
    }
    a / x * b
}

/// Why a texture of `file`'s format and shape cannot be made on a device
/// with `features`, or None when it can.
fn unusable(file: &TextureFile, features: wgpu::Features) -> Option<String> {
    let format = file.format;
    let missing = format.required_features().difference(features);
    if !missing.is_empty() {
        return Some(format!("the device has no {}", feature_names(missing).join(" or ")));
    }
    if !format.is_compressed() {
        return None;
    }
    match file.dimension {
        TextureDimension::D1 => return Some("a 1D texture cannot be block-compressed".to_string()),
        TextureDimension::D3 => {
            let sliced = match format {
                F::Astc { .. } => Some(wgpu::Features::TEXTURE_COMPRESSION_ASTC_SLICED_3D),
                _ if format.required_features() == wgpu::Features::TEXTURE_COMPRESSION_BC => Some(wgpu::Features::TEXTURE_COMPRESSION_BC_SLICED_3D),
                _ => None,
            };
            match sliced {
                Some(feature) if features.contains(feature) => {}
                Some(feature) => return Some(format!("the device has no {} for a 3D texture", feature_names(feature).join(" or "))),
                None => return Some(format!("a 3D texture cannot be {}", format_name(format))),
            }
        }
        TextureDimension::D2 => {}
    }
    let (bw, bh) = format.block_dimensions();
    if !file.width.is_multiple_of(bw) || !file.height.is_multiple_of(bh) {
        return Some(format!("its {}x{} size is not a whole number of the format's {bw}x{bh} blocks", file.width, file.height));
    }
    None
}

/// The uncompressed format a block-compressed one decodes into: 8 bits a
/// channel with the same channels, signed and sRGB formats kept so, and
/// half floats for HDR.
pub(crate) fn decoded_format(format: F) -> Option<F> {
    Some(match format {
        F::Bc1RgbaUnorm | F::Bc2RgbaUnorm | F::Bc3RgbaUnorm | F::Bc7RgbaUnorm | F::Etc2Rgb8Unorm | F::Etc2Rgb8A1Unorm | F::Etc2Rgba8Unorm => F::Rgba8Unorm,
        F::Bc1RgbaUnormSrgb | F::Bc2RgbaUnormSrgb | F::Bc3RgbaUnormSrgb | F::Bc7RgbaUnormSrgb | F::Etc2Rgb8UnormSrgb | F::Etc2Rgb8A1UnormSrgb | F::Etc2Rgba8UnormSrgb => {
            F::Rgba8UnormSrgb
        }
        F::Bc4RUnorm | F::EacR11Unorm => F::R8Unorm,
        F::Bc4RSnorm | F::EacR11Snorm => F::R8Snorm,
        F::Bc5RgUnorm | F::EacRg11Unorm => F::Rg8Unorm,
        F::Bc5RgSnorm | F::EacRg11Snorm => F::Rg8Snorm,
        F::Bc6hRgbUfloat | F::Bc6hRgbFloat | F::Astc { channel: AstcChannel::Hdr, .. } => F::Rgba16Float,
        F::Astc { channel: AstcChannel::Unorm, .. } => F::Rgba8Unorm,
        F::Astc { channel: AstcChannel::UnormSrgb, .. } => F::Rgba8UnormSrgb,
        _ => return None,
    })
}

/// `file` as a device with `features` can hold it: unchanged when it can
/// hold the file's format, and otherwise, for a block-compressed format,
/// decoded on the CPU into an uncompressed one with every level, layer and
/// slice kept, which a warning says.
pub(crate) fn decode_for(file: TextureFile, features: wgpu::Features, said: &mut Said) -> PsResult<TextureFile> {
    let Some(reason) = unusable(&file, features) else {
        return Ok(file);
    };
    if decoded_format(file.format).is_none() {
        return Err(invalid("WgpuTextureFormat", format!("a {} texture cannot be made: {reason}", format_name(file.format))));
    }
    let from = file.format;
    let (file, errors) = decode(file)?;
    let mut note = format!("{reason}, so the {} texels were decoded on the CPU into {}", format_name(from), format_name(file.format));
    if errors > 0 {
        note.push_str(&format!("; {errors} of its blocks are errors by the ASTC specification and decode as magenta"));
    }
    said.warnings.push(note);
    Ok(file)
}

/// `file` with its texels uncompressed: a block-compressed format decoded
/// on the CPU, as `decode_for` decodes it, and any other format as it is.
/// A warning counts the blocks that are errors by the ASTC specification.
pub(crate) fn uncompressed(file: TextureFile, said: &mut Said) -> PsResult<TextureFile> {
    if !file.format.is_compressed() {
        return Ok(file);
    }
    let (file, errors) = decode(file)?;
    if errors > 0 {
        said.warnings.push(format!("{errors} blocks of the texture are errors by the ASTC specification and decode as magenta"));
    }
    Ok(file)
}

/// Every level, layer and slice of `file`, block-compressed, decoded into
/// the format `decoded_format` names, with the count of blocks that are
/// errors.
fn decode(file: TextureFile) -> PsResult<(TextureFile, usize)> {
    let into = decoded_format(file.format).ok_or_else(|| invalid("WgpuTextureDecode", format!("the module decodes no {} texels on the CPU", format_name(file.format))))?;
    let mut errors = 0;
    let mut levels = Vec::with_capacity(file.levels.len());
    for (mip, level) in file.levels.iter().enumerate() {
        let mip = mip as u32;
        let (w, h) = ((file.width >> mip).max(1), (file.height >> mip).max(1));
        let slice_len = level_len(file.format, w, h, 1, 0)?;
        let slices = file.layers as usize * file.slices(mip) as usize;
        if level.len() != slice_len * slices {
            return Err(invalid("WgpuTextureDecode", format!("mip level {mip} holds {} bytes where its shape takes {}", level.len(), slice_len * slices)));
        }
        let mut out = Vec::new();
        for slice in level.chunks_exact(slice_len) {
            errors += decode_slice(file.format, into, slice, w as usize, h as usize, &mut out)?;
        }
        levels.push(out);
    }
    Ok((TextureFile { format: into, levels, ..file }, errors))
}

/// One slice of `format` blocks, `width` by `height` texels, decoded into
/// `into` and appended to `out`, rows packed. The count of blocks that
/// decode as errors.
fn decode_slice(format: F, into: F, blocks: &[u8], width: usize, height: usize, out: &mut Vec<u8>) -> PsResult<usize> {
    let (bw, bh) = format.block_dimensions();
    let (bw, bh) = (bw as usize, bh as usize);
    let block_len = format.block_copy_size(None).ok_or_else(|| invalid("WgpuTextureDecode", format!("{} has no block layout", format_name(format))))? as usize;
    let texel = into.block_copy_size(None).ok_or_else(|| invalid("WgpuTextureDecode", format!("{} has no texel layout", format_name(into))))? as usize;
    let across = width.div_ceil(bw);
    let start = out.len();
    out.resize(start + width * height * texel, 0);
    let image = &mut out[start..];
    let mut decoded = vec![0u8; bw * bh * texel];
    let mut errors = 0;
    for (index, block) in blocks.chunks_exact(block_len).enumerate() {
        if !decode_block(format, block, &mut decoded)? {
            errors += 1;
        }
        let (bx, by) = (index % across * bw, index / across * bh);
        let row_len = bw.min(width - bx) * texel;
        for y in 0..bh.min(height - by) {
            let at = ((by + y) * width + bx) * texel;
            image[at..at + row_len].copy_from_slice(&decoded[y * bw * texel..y * bw * texel + row_len]);
        }
    }
    Ok(errors)
}

/// One block of `format` decoded into `out`, rows of the block's texels
/// packed in the format `decoded_format` names. False for a block the
/// format's specification calls an error.
fn decode_block(format: F, block: &[u8], out: &mut [u8]) -> PsResult<bool> {
    match format {
        F::Bc1RgbaUnorm | F::Bc1RgbaUnormSrgb => bcdec_rs::bc1(block, out, 16),
        F::Bc2RgbaUnorm | F::Bc2RgbaUnormSrgb => bcdec_rs::bc2(block, out, 16),
        F::Bc3RgbaUnorm | F::Bc3RgbaUnormSrgb => bcdec_rs::bc3(block, out, 16),
        F::Bc7RgbaUnorm | F::Bc7RgbaUnormSrgb => bcdec_rs::bc7(block, out, 16),
        F::Bc4RUnorm | F::Bc4RSnorm => bcdec_rs::bc4(block, out, 4, format == F::Bc4RSnorm),
        F::Bc5RgUnorm | F::Bc5RgSnorm => bcdec_rs::bc5(block, out, 8, format == F::Bc5RgSnorm),
        F::Bc6hRgbUfloat | F::Bc6hRgbFloat => {
            let mut rgb = [0u16; 48];
            bcdec_rs::bc6h_half(block, &mut rgb, 12, format == F::Bc6hRgbFloat);
            for (texel, color) in out.as_chunks_mut::<8>().0.iter_mut().zip(rgb.as_chunks::<3>().0) {
                for (bytes, half) in texel.as_chunks_mut::<2>().0.iter_mut().zip(color.iter().chain([&0x3C00])) {
                    *bytes = half.to_le_bytes();
                }
            }
        }
        F::Etc2Rgb8Unorm | F::Etc2Rgb8UnormSrgb => etc2(texture2ddecoder::decode_etc2_rgb_block, block, out),
        F::Etc2Rgb8A1Unorm | F::Etc2Rgb8A1UnormSrgb => etc2(texture2ddecoder::decode_etc2_rgba1_block, block, out),
        F::Etc2Rgba8Unorm | F::Etc2Rgba8UnormSrgb => etc2(texture2ddecoder::decode_etc2_rgba8_block, block, out),
        F::EacR11Unorm | F::EacR11Snorm | F::EacRg11Unorm | F::EacRg11Snorm => {
            let signed = matches!(format, F::EacR11Snorm | F::EacRg11Snorm);
            let channels = if matches!(format, F::EacRg11Unorm | F::EacRg11Snorm) { 2 } else { 1 };
            for (channel, half) in block.as_chunks::<8>().0.iter().take(channels).enumerate() {
                for (i, value) in eac_values(half, signed).into_iter().enumerate() {
                    out[i * channels + channel] = if signed { snorm11_to_8(value) } else { unorm11_to_8(value) };
                }
            }
        }
        F::Astc { channel, .. } => {
            let (bw, bh) = format.block_dimensions();
            let block: &[u8; 16] = block.try_into().map_err(|e| invalid("WgpuTextureDecode", format!("an ASTC block is 16 bytes: {e}")))?;
            if channel != AstcChannel::Hdr {
                return Ok(crate::astc::decode_ldr(block, bw as usize, bh as usize, out));
            }
            let mut halves = [0u16; 4 * 144];
            let valid = crate::astc::decode_hdr(block, bw as usize, bh as usize, &mut halves);
            for (bytes, half) in out.as_chunks_mut::<2>().0.iter_mut().zip(halves) {
                *bytes = half.to_le_bytes();
            }
            return Ok(valid);
        }
        other => return Err(invalid("WgpuTextureDecode", format!("the module decodes no {} texels on the CPU", format_name(other)))),
    }
    Ok(true)
}

/// An ETC2 block decoded by texture2ddecoder, which packs each texel as
/// BGRA, rewritten as RGBA.
fn etc2(decode: fn(&[u8], &mut [u32]), block: &[u8], out: &mut [u8]) {
    let mut texels = [0u32; 16];
    decode(block, &mut texels);
    for (rgba, texel) in out.as_chunks_mut::<4>().0.iter_mut().zip(texels) {
        let [b, g, r, a] = texel.to_le_bytes();
        *rgba = [r, g, b, a];
    }
}

/// The modifier table of EAC, the ETC2 alpha table of the Khronos Data
/// Format Specification.
const EAC_MODIFIERS: [[i32; 8]; 16] = [
    [-3, -6, -9, -15, 2, 5, 8, 14],
    [-3, -7, -10, -13, 2, 6, 9, 12],
    [-2, -5, -8, -13, 1, 4, 7, 12],
    [-2, -4, -6, -13, 1, 3, 5, 12],
    [-3, -6, -8, -12, 2, 5, 7, 11],
    [-3, -7, -9, -11, 2, 6, 8, 10],
    [-4, -7, -8, -11, 3, 6, 7, 10],
    [-3, -5, -8, -11, 2, 4, 7, 10],
    [-2, -6, -8, -10, 1, 5, 7, 9],
    [-2, -5, -8, -10, 1, 4, 7, 9],
    [-2, -4, -8, -10, 1, 3, 7, 9],
    [-2, -5, -7, -10, 1, 4, 6, 9],
    [-3, -4, -7, -10, 2, 3, 6, 9],
    [-1, -2, -3, -10, 0, 1, 2, 9],
    [-4, -6, -8, -9, 3, 5, 7, 8],
    [-3, -5, -7, -9, 2, 4, 6, 8],
];

/// The 16 values of one 8-byte EAC block, rows packed: 0 to 2047, or
/// -1023 to 1023 for a signed block.
fn eac_values(block: &[u8], signed: bool) -> [i32; 16] {
    let bits = block.iter().take(8).fold(0u64, |acc, &byte| acc << 8 | u64::from(byte));
    let base = (bits >> 56) as u8;
    let multiplier = ((bits >> 52) & 0xF) as i32;
    let table = &EAC_MODIFIERS[((bits >> 48) & 0xF) as usize];
    let mut out = [0; 16];
    for i in 0..16 {
        let modifier = table[((bits >> (45 - 3 * i)) & 7) as usize];
        let step = if multiplier == 0 { modifier } else { modifier * multiplier * 8 };
        let value = if signed {
            // The base -128 reads as -127.
            ((base as i8).max(-127) as i32 * 8 + step).clamp(-1023, 1023)
        } else {
            (i32::from(base) * 8 + 4 + step).clamp(0, 2047)
        };
        // The block holds its texels column by column.
        out[(i % 4) * 4 + i / 4] = value;
    }
    out
}

/// An 11-bit unsigned EAC value as the nearest 8-bit UNORM value.
fn unorm11_to_8(value: i32) -> u8 {
    ((value * 255 + 1023) / 2047) as u8
}

/// An 11-bit signed EAC value as the nearest 8-bit SNORM value.
fn snorm11_to_8(value: i32) -> u8 {
    ((2 * value * 127 + value.signum() * 1023) / 2046) as i8 as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A BC1 block whose every texel takes color 0: pure red in RGB565.
    const RED_BC1: [u8; 8] = [0x00, 0xF8, 0x1F, 0x00, 0, 0, 0, 0];

    fn bc1_file(width: u32, height: u32, mips: u32, layers: u32, cube: bool) -> TextureFile {
        let levels = (0..mips)
            .map(|mip| {
                let blocks = ((width >> mip).max(1).div_ceil(4) * (height >> mip).max(1).div_ceil(4) * layers) as usize;
                RED_BC1.repeat(blocks)
            })
            .collect();
        TextureFile { dimension: TextureDimension::D2, width, height, depth: 1, layers, cube, format: F::Bc1RgbaUnorm, levels }
    }

    fn same(a: &TextureFile, b: &TextureFile) {
        assert_eq!((a.dimension, a.width, a.height, a.depth, a.layers, a.cube, a.format), (b.dimension, b.width, b.height, b.depth, b.layers, b.cube, b.format));
        assert_eq!(a.levels, b.levels);
    }

    #[test]
    fn a_dds_file_round_trips_every_level_and_layer() {
        let mut said = Said::default();
        let file = bc1_file(16, 8, 4, 12, true);
        same(&read_dds(&write_dds(&file).expect("write"), false, &mut said).expect("read"), &file);
        let mut volume = TextureFile {
            dimension: TextureDimension::D3,
            width: 4,
            height: 4,
            depth: 4,
            layers: 1,
            cube: false,
            format: F::Rgba16Float,
            levels: vec![(0..4 * 4 * 4 * 8).map(|i| i as u8).collect(), (0..2 * 2 * 2 * 8).map(|i| i as u8).collect()],
        };
        same(&read_dds(&write_dds(&volume).expect("write"), false, &mut said).expect("read"), &volume);
        volume.format = F::Rgba8UnormSrgb;
        volume.levels = vec![vec![7; 4 * 4 * 4 * 4], vec![9; 2 * 2 * 2 * 4]];
        same(&read_dds(&write_dds(&volume).expect("write"), false, &mut said).expect("read"), &volume);
        assert!(said.verbose.is_empty() && said.warnings.is_empty(), "{said:?}");
    }

    /// -Srgb reads a format with an sRGB form as that form, and warns, with
    /// the file still read, for one with none and for one sRGB already.
    #[test]
    fn srgb_relabels_a_file_or_warns_that_it_did_nothing() {
        let mut said = Said::default();
        let file = bc1_file(8, 8, 1, 1, false);
        assert_eq!(read_dds(&write_dds(&file).expect("write"), true, &mut said).expect("read").format, F::Bc1RgbaUnormSrgb);
        assert!(said.warnings.is_empty(), "{said:?}");
        let half = TextureFile { format: F::Rgba16Float, levels: vec![vec![0; 8 * 8 * 8]], ..bc1_file(8, 8, 1, 1, false) };
        assert_eq!(read_ktx2(&write_ktx2(&half, None).expect("write"), true, wgpu::Features::all(), &mut said).expect("read").format, F::Rgba16Float);
        let srgb = TextureFile { format: F::Bc1RgbaUnormSrgb, ..bc1_file(8, 8, 1, 1, false) };
        assert_eq!(read_dds(&write_dds(&srgb).expect("write"), true, &mut said).expect("read").format, F::Bc1RgbaUnormSrgb);
        assert_eq!(
            said.warnings,
            [
                "-Srgb did nothing for the file: its format, Rgba16Float, has no sRGB form",
                "-Srgb did nothing for the file: its format, Bc1RgbaUnormSrgb, is sRGB already",
            ]
        );
    }

    #[test]
    fn a_ktx2_file_round_trips_stored_and_with_zstd() {
        for zstd in [None, Some(6)] {
            let file = bc1_file(20, 12, 5, 3, false);
            let mut said = Said::default();
            let read = read_ktx2(&write_ktx2(&file, zstd).expect("write"), false, wgpu::Features::all(), &mut said).expect("read");
            assert!(said.verbose.is_empty() && said.warnings.is_empty(), "{said:?}");
            same(&read, &file);
            let cube = bc1_file(8, 8, 1, 6, true);
            same(&read_ktx2(&write_ktx2(&cube, zstd).expect("write"), false, wgpu::Features::all(), &mut said).expect("read"), &cube);
        }
    }

    #[test]
    fn a_written_ktx2_file_aligns_its_levels_and_names_its_writer() {
        let file = bc1_file(16, 16, 3, 1, false);
        let bytes = write_ktx2(&file, None).expect("write");
        let reader = ktx2::Reader::new(&bytes[..]).expect("parse");
        assert_eq!(reader.writer(), Some(concat!("Wgpu ", env!("CARGO_PKG_VERSION"), "\0")));
        for level in reader.levels() {
            let offset = level.data.as_ptr() as usize - bytes.as_ptr() as usize;
            assert_eq!(offset % 8, 0, "a BC1 level starts on a multiple of lcm(8, 4)");
        }
    }

    #[test]
    fn decodes_bc1_on_the_cpu_for_a_device_without_bc_and_says_so() {
        let file = bc1_file(8, 4, 2, 1, false);
        let mut said = Said::default();
        let decoded = decode_for(file, wgpu::Features::empty(), &mut said).expect("decode");
        assert_eq!(decoded.format, F::Rgba8Unorm);
        assert_eq!(decoded.levels[0].len(), 8 * 4 * 4);
        assert_eq!(decoded.levels[1].len(), 4 * 2 * 4);
        for level in &decoded.levels {
            assert!(level.as_chunks::<4>().0.iter().all(|texel| *texel == [255, 0, 0, 255]));
        }
        assert!(matches!(&said.warnings[..], [note] if note.contains("TextureCompressionBc") && note.contains("Rgba8Unorm")), "{said:?}");
    }

    #[test]
    fn keeps_a_texture_the_device_can_hold() {
        let mut said = Said::default();
        let kept = decode_for(bc1_file(8, 8, 1, 1, false), wgpu::Features::TEXTURE_COMPRESSION_BC, &mut said).expect("decode");
        assert_eq!(kept.format, F::Bc1RgbaUnorm);
        assert!(said.warnings.is_empty(), "{said:?}");
        let decoded = decode_for(bc1_file(6, 8, 1, 1, false), wgpu::Features::TEXTURE_COMPRESSION_BC, &mut said).expect("decode");
        assert_eq!(decoded.format, F::Rgba8Unorm);
        assert!(matches!(&said.warnings[..], [note] if note.contains("6x8")), "{said:?}");
    }

    /// texture2ddecoder's ETC2 alpha decoder reads the block layout and the
    /// modifier table EAC shares: with a nonzero multiplier, the top 8 of
    /// an unsigned EAC value's 11 bits are the alpha it decodes.
    #[test]
    fn decodes_eac_texels_where_texture2ddecoder_decodes_etc2_alpha() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut compared = 0;
        for _ in 0..2000 {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            let block = (z ^ (z >> 31)).to_be_bytes();
            if block[1] >> 4 == 0 {
                continue;
            }
            let mut theirs = [0u32; 16];
            texture2ddecoder::decode_etc2_a8_block(&block, &mut theirs);
            for (value, texel) in eac_values(&block, false).iter().zip(theirs) {
                assert_eq!((value >> 3) as u32, texel >> 24, "{block:02X?}");
            }
            compared += 1;
        }
        assert!(compared > 1800, "only {compared} blocks had a nonzero multiplier");
    }

    #[test]
    fn decodes_eac_values_by_the_specification() {
        // Base -128, read as -127; multiplier 1; table 0; every index 0, modifier -3.
        let block = [0x80, 0x10, 0, 0, 0, 0, 0, 0];
        assert!(eac_values(&block, true).iter().all(|&v| v == -1023));
        assert!(eac_values(&block, false).iter().all(|&v| v == 128 * 8 + 4 - 3 * 8));
        // Base 16; multiplier 0, so the modifier counts unscaled; table 13; every index 7, modifier 9.
        let block = [0x10, 0x0D, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        assert!(eac_values(&block, true).iter().all(|&v| v == 16 * 8 + 9));
        assert!(eac_values(&block, false).iter().all(|&v| v == 16 * 8 + 4 + 9));
    }

    #[test]
    fn rounds_eac_values_to_8_bits() {
        assert_eq!((unorm11_to_8(0), unorm11_to_8(2047), unorm11_to_8(4), unorm11_to_8(5)), (0, 255, 0, 1));
        assert_eq!((snorm11_to_8(0), snorm11_to_8(1023), snorm11_to_8(-1023), snorm11_to_8(512), snorm11_to_8(-512)), (0, 127, 129, 64, 192));
    }
}
