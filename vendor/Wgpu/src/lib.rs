//! wgpu for PowerShell and for Rust: GPU compute, rendering and shader
//! tools through wgpu, on Direct3D 12, Vulkan, OpenGL and Metal. Devices,
//! buffers, textures, shaders and pipelines are objects held in Rust, so
//! data stays on the GPU between calls and moves only when a script reads
//! or writes it. Every cmdlet's noun starts with Wgpu and has a Gpu alias.
//!
//! The `module` feature, on by default, builds the PowerShell module.
//! Without it the crate is the Rust library in [`gpu`], whose types are
//! also at the crate root beside [`ElementType`], the element types of the
//! module's buffers and their conversions, a re-export of [`wgpu`], and
//! the error type its methods return, [`PsError`], whose `error_id` is the
//! id the module's cmdlets raise for the same failure.

// dead_code: without `module` nothing exports the cmdlets, so the code
// only they reach goes unused.
#![cfg_attr(not(feature = "module"), allow(dead_code))]

mod alpha;
mod astc;
mod basis;
mod binding;
mod buffer;
mod color;
mod compare;
mod compute;
mod convert;
mod cube;
#[cfg(test)]
mod cube_reference;
mod device;
mod element;
mod encoder;
mod error;
#[cfg(test)]
mod faults;
mod filter;
#[cfg(test)]
mod fill_reference;
#[cfg(test)]
mod filter_reference;
#[cfg(test)]
mod float_reference;
pub mod gpu;
#[cfg(all(test, feature = "avif"))]
mod hdr_reference;
mod image_decode;
mod image_io;
#[cfg(all(test, feature = "jxl"))]
mod jxl_reference;
mod names;
mod normal;
#[cfg(all(test, feature = "pdf"))]
mod pdf_reference;
mod pipeline;
#[cfg(test)]
mod profile_reference;
mod profile_table;
mod psd;
mod psd_composite8;
mod psd_descriptor;
mod psd_fill;
mod psd_gradient;
mod psd_merge;
mod psd_raster;
#[cfg(test)]
mod psd_reference;
mod psd_stroke;
mod psd_vector;
#[cfg(test)]
mod psd_vector_reference;
mod render;
mod shader;
mod tasks;
mod texel;
mod texfile;
mod texture;
mod transform;

pub use element::{host_bytes, to_clr_array, to_clr_memory, ElementType};
pub use gpu::{
    compute_binding_info, compute_binding_info_glsl, render_binding_info, render_vertex_input_info, Binding, BindingInfo, BlendFactor, BlendMode, BlendOperation, CompareFunction,
    ComputeProgram, CullMode, CustomBlend, DepthTest, Dispatch, Draw, Gpu, GpuOptions, PrimitiveTopology, RenderOptions, RenderProgram, RunReport, RunSpec, StencilFace,
    StencilOperation, StencilTest, VertexInputInfo,
};
pub use pwrs::{ErrorCategory, PsError, PsResult};
pub use wgpu;

#[cfg(feature = "module")]
use binding::{BindGroup, NewWgpuBindGroup};
#[cfg(feature = "module")]
use buffer::{Buffer, BufferSlice, BufferUsage, ClearWgpuBuffer, CopyWgpuBuffer, NewWgpuBuffer, ReadWgpuBuffer, RemoveWgpuBuffer, WriteWgpuBuffer};
#[cfg(feature = "module")]
use color::{ColorSpace, JoinWgpuTexture, ToneMapCurve};
#[cfg(feature = "module")]
use compare::{CompareWgpuTexture, MeasureWgpuTexture, TextureChannel, TextureComparison, TextureMeasurement};
#[cfg(feature = "module")]
use compute::{ComputeResult, InvokeWgpuCompute, InvokeWgpuMap, MeasureWgpuArray, Measurement};
#[cfg(feature = "module")]
use device::{Adapter, AllocatorReport, CloseWgpuDevice, Counters, Device, DeviceEvent, DeviceStatus, GetWgpuAdapter, GetWgpuDevice, MemoryHint, OpenWgpuDevice, WaitWgpuDevice};
#[cfg(feature = "module")]
use convert::{MipEdge, MipFilter};
#[cfg(feature = "module")]
use cube::{ConvertFromWgpuCubeMap, ConvertToWgpuCubeMap, ConvertToWgpuIrradianceMap, ConvertToWgpuSpecularMap, CubeLayout, Harmonics, NewWgpuBrdfTable, RoughnessMapping};
#[cfg(feature = "module")]
use encoder::{CommandBuffer, CommandEncoder, ComputePass, NewWgpuCommandEncoder, NewWgpuQuerySet, QueryKind, QuerySet, SubmitWgpuCommand};
#[cfg(feature = "module")]
use filter::{EdgeOperator, FilterEdge};
#[cfg(feature = "module")]
use image_decode::RenderingIntent;
#[cfg(feature = "module")]
use names::{Backend, DeviceType, Limits};
#[cfg(feature = "module")]
use normal::{ConvertToWgpuNormalMap, HeightChannel, HeightEdge, NormalAlpha, NormalKernel, NormalMips};
#[cfg(feature = "module")]
use pipeline::{ComputePipeline, NewWgpuComputePipeline};
#[cfg(feature = "module")]
use render::{InvokeWgpuRender, NewWgpuRenderPipeline, RenderPass, RenderPipeline};
#[cfg(feature = "module")]
use shader::{
    BindingKind, ConvertToWgpuShader, GetWgpuShaderInfo, NewWgpuShader, Shader, ShaderBinding, ShaderDiagnostic, ShaderEntryPoint, ShaderInfo, ShaderLanguage, ShaderOverride, ShaderStage,
    ShaderTarget, TestWgpuShader,
};
#[cfg(feature = "module")]
use texture::{
    AddressMode, BorderColor, Decoder, ExportWgpuImage, FilterMode, GetWgpuTextureFormat, ImportWgpuImage, NewWgpuSampler, NewWgpuTexture, ReadWgpuTexture, Sampler,
    Texture, TextureAspect, TextureDimension, TextureFormatInfo, TextureUsage, TextureView, WriteWgpuTexture,
};
#[cfg(feature = "module")]
use transform::{Anchor, ConvertWgpuTexture, FlipDirection, PowerOfTwo, ResizeFilter, ResizeWgpuTexture};

#[cfg(feature = "module")]
pwrs::export_module! {
    name: "Wgpu",
    cmdlets: [
        GetWgpuAdapter,
        OpenWgpuDevice,
        GetWgpuDevice,
        CloseWgpuDevice,
        WaitWgpuDevice,
        NewWgpuBuffer,
        ReadWgpuBuffer,
        WriteWgpuBuffer,
        CopyWgpuBuffer,
        ClearWgpuBuffer,
        RemoveWgpuBuffer,
        NewWgpuShader,
        TestWgpuShader,
        GetWgpuShaderInfo,
        ConvertToWgpuShader,
        NewWgpuComputePipeline,
        NewWgpuBindGroup,
        InvokeWgpuCompute,
        InvokeWgpuMap,
        MeasureWgpuArray,
        NewWgpuCommandEncoder,
        SubmitWgpuCommand,
        NewWgpuQuerySet,
        NewWgpuTexture,
        ReadWgpuTexture,
        WriteWgpuTexture,
        ImportWgpuImage,
        ExportWgpuImage,
        ResizeWgpuTexture,
        ConvertWgpuTexture,
        ConvertToWgpuNormalMap,
        ConvertToWgpuCubeMap,
        ConvertFromWgpuCubeMap,
        ConvertToWgpuIrradianceMap,
        ConvertToWgpuSpecularMap,
        NewWgpuBrdfTable,
        JoinWgpuTexture,
        CompareWgpuTexture,
        MeasureWgpuTexture,
        NewWgpuSampler,
        GetWgpuTextureFormat,
        NewWgpuRenderPipeline,
        InvokeWgpuRender,
    ],
    classes: [
        Adapter,
        Limits,
        Device,
        DeviceStatus,
        DeviceEvent,
        Counters,
        AllocatorReport,
        Buffer,
        BufferSlice,
        Shader,
        ShaderInfo,
        ShaderEntryPoint,
        ShaderBinding,
        ShaderOverride,
        ShaderDiagnostic,
        ComputePipeline,
        BindGroup,
        CommandEncoder,
        ComputePass,
        CommandBuffer,
        QuerySet,
        ComputeResult,
        Measurement,
        Texture,
        TextureView,
        Sampler,
        TextureFormatInfo,
        TextureComparison,
        TextureMeasurement,
        Harmonics,
        RenderPipeline,
        RenderPass,
    ],
    enums: [
        ElementType,
        Backend,
        DeviceType,
        MemoryHint,
        BufferUsage,
        ShaderLanguage,
        ShaderStage,
        ShaderTarget,
        BindingKind,
        QueryKind,
        TextureUsage,
        TextureDimension,
        TextureAspect,
        Decoder,
        RenderingIntent,
        MipFilter,
        MipEdge,
        ResizeFilter,
        PowerOfTwo,
        Anchor,
        FlipDirection,
        FilterEdge,
        EdgeOperator,
        ColorSpace,
        ToneMapCurve,
        HeightChannel,
        NormalKernel,
        HeightEdge,
        NormalAlpha,
        NormalMips,
        CubeLayout,
        RoughnessMapping,
        TextureChannel,
        FilterMode,
        AddressMode,
        CompareFunction,
        BorderColor,
        PrimitiveTopology,
        CullMode,
        BlendMode,
        BlendFactor,
        BlendOperation,
        StencilOperation,
    ],
    completers: [
        device::complete_feature,
        texture::complete_texture_format,
        texture::complete_image_format,
        texture::complete_export_format,
        texture::complete_join_format,
        texture::complete_normal_map_format,
        texture::complete_convert_weights,
        texture::complete_import_weights,
        texture::complete_export_weights,
        texture::complete_format_name,
        render::complete_pipeline_depth_format,
        render::complete_render_depth_format,
    ],
    transforms: [texture::import_mip_edge, texture::export_mip_edge],
    on_unload: device::release_default_device,
}
