//! Shaders: parsed and validated by naga with no device, reflected,
//! translated between languages, and compiled on devices.

use std::borrow::Cow;
use std::num::NonZeroU32;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Arc;

use naga::common::wgsl::ToWgsl;
use pwrs::prelude::*;

use crate::device::CoreRef;
use crate::element::ElementType;
use crate::error::{for_each_file, input_files, invalid, not_found, panic_text};

/// The language a shader is written in.
#[psenum(name = "Wgpu.ShaderLanguage")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShaderLanguage {
    /// WebGPU Shading Language.
    #[default]
    Wgsl,
    /// OpenGL Shading Language, one stage per shader.
    Glsl,
    /// SPIR-V words.
    SpirV,
}

/// A pipeline stage.
#[psenum(name = "Wgpu.ShaderStage")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShaderStage {
    /// A compute shader.
    #[default]
    Compute,
    /// A vertex shader.
    Vertex,
    /// A fragment shader.
    Fragment,
    /// A task shader, which launches mesh shaders.
    Task,
    /// A mesh shader.
    Mesh,
    /// A ray-tracing stage.
    RayTracing,
}

impl ShaderStage {
    pub(crate) fn naga(self) -> PsResult<naga::ShaderStage> {
        Ok(match self {
            ShaderStage::Compute => naga::ShaderStage::Compute,
            ShaderStage::Vertex => naga::ShaderStage::Vertex,
            ShaderStage::Fragment => naga::ShaderStage::Fragment,
            ShaderStage::Task => naga::ShaderStage::Task,
            ShaderStage::Mesh => naga::ShaderStage::Mesh,
            ShaderStage::RayTracing => return Err(invalid("WgpuStage", "a ray-tracing stage cannot be named here; give the entry point instead")),
        })
    }

    pub(crate) fn from_naga(stage: naga::ShaderStage) -> ShaderStage {
        match stage {
            naga::ShaderStage::Compute => ShaderStage::Compute,
            naga::ShaderStage::Vertex => ShaderStage::Vertex,
            naga::ShaderStage::Fragment => ShaderStage::Fragment,
            naga::ShaderStage::Task => ShaderStage::Task,
            naga::ShaderStage::Mesh => ShaderStage::Mesh,
            _ => ShaderStage::RayTracing,
        }
    }
}

/// A language ConvertTo-WgpuShader writes.
#[psenum(name = "Wgpu.ShaderTarget")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShaderTarget {
    /// WGSL text.
    #[default]
    Wgsl,
    /// SPIR-V words, as `uint[]`.
    SpirV,
    /// GLSL text for one entry point (OpenGL 4.50, or OpenGL ES 3.10 and up).
    Glsl,
    /// HLSL text for Direct3D, Shader Model 5.0 to 6.9.
    Hlsl,
    /// Metal Shading Language text.
    Msl,
    /// The module's intermediate form as a Graphviz graph.
    Dot,
}

/// What a shader binding holds.
#[psenum(name = "Wgpu.BindingKind")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BindingKind {
    /// `var<uniform>`.
    UniformBuffer,
    /// `var<storage, read_write>`.
    #[default]
    StorageBuffer,
    /// `var<storage, read>`.
    ReadOnlyStorageBuffer,
    /// `sampler`.
    Sampler,
    /// `sampler_comparison`.
    ComparisonSampler,
    /// A sampled or depth texture.
    Texture,
    /// A storage texture.
    StorageTexture,
    /// A ray-tracing acceleration structure.
    AccelerationStructure,
    /// An external texture.
    ExternalTexture,
}

/// A problem naga or a device found in a shader.
#[psclass(name = "Wgpu.ShaderDiagnostic", columns = ["Label", "Severity", "Line", "Column", "Message"])]
#[derive(Default, Clone)]
pub struct ShaderDiagnostic {
    /// The shader it is in: the path of the file it was read from, or Wgpu
    /// shader for text and SPIR-V.
    pub label: String,
    /// Error.
    pub severity: String,
    /// What is wrong.
    pub message: String,
    /// The line it is on, from 1; 0 where the source has no lines, as with SPIR-V.
    pub line: u32,
    /// The column it starts at, from 1.
    pub column: u32,
    /// How many characters it covers.
    pub length: u32,
    /// The text of that line; `$null` where the source has no lines.
    pub source_line: Option<String>,
    /// The full report, with every labeled span and note.
    pub detail: String,
}

/// An entry point of a shader.
#[psclass(name = "Wgpu.ShaderEntryPoint", columns = ["Name", "Stage", "WorkgroupSize"], show = "{Name}")]
#[derive(Default, Clone)]
pub struct ShaderEntryPoint {
    /// Its name.
    pub name: String,
    /// The stage it runs in.
    pub stage: ShaderStage,
    /// Invocations per workgroup in x, y and z, for compute, task and mesh stages.
    pub workgroup_size: Vec<u32>,
    /// The names of the bindings it uses, in group and binding order.
    pub bindings: Vec<String>,
}

/// A resource binding a shader declares.
#[psclass(name = "Wgpu.ShaderBinding", columns = ["Group", "Binding", "Name", "Kind", "WgslType"], show = "{Name}")]
#[derive(Default, Clone)]
pub struct ShaderBinding {
    /// The `@group`.
    pub group: u32,
    /// The `@binding`.
    pub binding: u32,
    /// The variable's name.
    pub name: String,
    /// What it holds.
    pub kind: BindingKind,
    /// Its type, written as WGSL.
    pub wgsl_type: String,
    /// For a buffer of numbers, the type of each number.
    pub element_type: Option<ElementType>,
    /// Bytes of the fixed part of a buffer binding; a runtime-sized array adds `Stride` per element.
    pub size: u64,
    /// Bytes per element of a runtime-sized or fixed array.
    pub stride: u64,
    /// The number of resources in a binding array; 0 for a single resource.
    pub count: u32,
}

/// A pipeline-overridable constant a shader declares.
#[psclass(name = "Wgpu.ShaderOverride", columns = ["Name", "Id", "WgslType", "Default"])]
#[derive(Default, Clone)]
pub struct ShaderOverride {
    /// Its name; `$null` for one known only by its `@id`.
    pub name: Option<String>,
    /// Its `@id`, where it has one.
    pub id: Option<u32>,
    /// Its type, written as WGSL.
    pub wgsl_type: String,
    /// Its default value, where it has one that is a literal.
    pub default: Option<f64>,
}

/// What a shader declares: entry points, bindings and override constants.
#[psclass(name = "Wgpu.ShaderInfo", columns = ["Label", "Language", "EntryPoints", "Bindings", "Overrides"])]
#[derive(Default, Clone)]
pub struct ShaderInfo {
    /// The shader's label: the path of the file it was read from, the label
    /// it was made with, or Wgpu shader for text and SPIR-V.
    pub label: String,
    /// The language it was written in.
    pub language: ShaderLanguage,
    /// Its entry points.
    pub entry_points: Vec<ShaderEntryPoint>,
    /// Its resource bindings, in group and binding order.
    pub bindings: Vec<ShaderBinding>,
    /// Its pipeline-overridable constants.
    pub overrides: Vec<ShaderOverride>,
}

/// A shader's source, as it is handed to a device.
pub(crate) enum Source {
    Wgsl(String),
    Glsl { text: String, stage: naga::ShaderStage, defines: Vec<(String, String)> },
    SpirV(Vec<u32>),
}

impl Source {
    fn language(&self) -> ShaderLanguage {
        match self {
            Source::Wgsl(_) => ShaderLanguage::Wgsl,
            Source::Glsl { .. } => ShaderLanguage::Glsl,
            Source::SpirV(_) => ShaderLanguage::SpirV,
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Source::Wgsl(text) | Source::Glsl { text, .. } => Some(text),
            Source::SpirV(_) => None,
        }
    }
}

/// A shader naga parsed and validated.
pub(crate) struct Parsed {
    pub(crate) source: Source,
    pub(crate) module: naga::Module,
    pub(crate) info: naga::valid::ModuleInfo,
    pub(crate) label: String,
}

fn diagnostic(message: String, text: Option<&str>, location: Option<naga::SourceLocation>, detail: String) -> ShaderDiagnostic {
    match (location, text) {
        (Some(at), Some(text)) => ShaderDiagnostic {
            label: String::new(),
            severity: "Error".to_string(),
            message,
            line: at.line_number,
            column: at.line_position,
            length: at.length,
            source_line: text.lines().nth(at.line_number.saturating_sub(1) as usize).map(str::to_string),
            detail,
        },
        _ => ShaderDiagnostic { severity: "Error".to_string(), message, detail, ..Default::default() },
    }
}

/// An error and the chain of errors that caused it, as one sentence.
fn chain(e: &dyn std::error::Error) -> String {
    let mut text = e.to_string();
    let mut next = e.source();
    while let Some(cause) = next {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        next = cause.source();
    }
    text
}

/// Parses a shader, or describes every problem the parser found.
pub(crate) fn parse(source: &Source) -> Result<naga::Module, Vec<ShaderDiagnostic>> {
    match source {
        Source::Wgsl(text) => naga::front::wgsl::parse_str(text).map_err(|e| {
            let detail = e.emit_to_string(text);
            vec![diagnostic(e.message().to_string(), Some(text), e.location(text), detail)]
        }),
        Source::Glsl { text, stage, defines } => {
            let mut options = naga::front::glsl::Options::from(*stage);
            options.defines.extend(defines.iter().cloned());
            naga::front::glsl::Frontend::default().parse(&options, text).map_err(|errors| {
                errors.errors.iter().map(|e| diagnostic(e.kind.to_string(), Some(text), e.location(text), e.kind.to_string())).collect()
            })
        }
        Source::SpirV(words) => {
            // SPIR-V is read in Vulkan's clip space, whose y points down, as
            // the module writes it: a vertex shader's position is flipped
            // into WebGPU's, so SPIR-V from glslc or dxc, and SPIR-V
            // written here, draws as it does in Vulkan.
            let options = naga::front::spv::Options { adjust_coordinate_space: true, strict_capabilities: true, block_ctx_dump_prefix: None };
            // naga's SPIR-V reader panics on some malformed words, such as
            // a type width that is not a power of two; that panic is the
            // shader's one problem, reported as any other.
            match catch_unwind(AssertUnwindSafe(|| naga::front::spv::parse_u8_slice(bytemuck::cast_slice(words), &options))) {
                Ok(parsed) => parsed.map_err(|e| vec![diagnostic(chain(&e), None, None, chain(&e))]),
                Err(payload) => {
                    let message = format!("naga panicked while reading the SPIR-V: {}", panic_text(payload));
                    Err(vec![diagnostic(message.clone(), None, None, message)])
                }
            }
        }
    }
}

/// Validates a module with every capability naga has, as a device with
/// every feature would.
pub(crate) fn validate(module: &naga::Module, text: Option<&str>) -> Result<naga::valid::ModuleInfo, Vec<ShaderDiagnostic>> {
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(module).map_err(|e| {
        let detail = match text {
            Some(text) => e.emit_to_string(text),
            None => chain(e.as_inner()),
        };
        let location = text.and_then(|text| e.location(text));
        vec![diagnostic(chain(e.as_inner()), text, location, detail)]
    })
}

/// Parses and validates `source`, failing with every diagnostic.
pub(crate) fn compile(source: Source, label: String) -> PsResult<Arc<Parsed>> {
    let checked = parse(&source).and_then(|module| validate(&module, source.text()).map(|info| (module, info)));
    match checked {
        Ok((module, info)) => Ok(Arc::new(Parsed { source, module, info, label })),
        Err(diagnostics) => Err(diagnostics_error(&diagnostics)),
    }
}

pub(crate) fn diagnostics_error(diagnostics: &[ShaderDiagnostic]) -> PsError {
    let message = match diagnostics.first() {
        Some(d) if d.line > 0 => format!("the shader is invalid: line {}, column {}: {}", d.line, d.column, d.message),
        Some(d) => format!("the shader is invalid: {}", d.message),
        None => "the shader is invalid".to_string(),
    };
    let details = diagnostics.iter().map(|d| d.detail.clone()).collect::<Vec<_>>().join("\n");
    PsError::new(ErrorCategory::ParserError, "WgpuShaderInvalid", message).with_details(details)
}

fn scalar_name(s: naga::Scalar) -> String {
    use naga::ScalarKind as K;
    match (s.kind, s.width) {
        (K::Bool, _) => "bool".to_string(),
        (K::AbstractInt, _) => "abstract-int".to_string(),
        (K::AbstractFloat, _) => "abstract-float".to_string(),
        (K::Sint, w) => format!("i{}", w * 8),
        (K::Uint, w) => format!("u{}", w * 8),
        (K::Float, w) => format!("f{}", w * 8),
    }
}

/// A type written as WGSL.
pub(crate) fn type_name(module: &naga::Module, ty: naga::Handle<naga::Type>) -> String {
    use naga::TypeInner as T;
    let t = &module.types[ty];
    if let (Some(name), T::Struct { .. }) = (&t.name, &t.inner) {
        return name.clone();
    }
    match &t.inner {
        T::Scalar(s) => scalar_name(*s),
        T::Vector { size, scalar } => format!("vec{}<{}>", *size as u8, scalar_name(*scalar)),
        T::Matrix { columns, rows, scalar } => format!("mat{}x{}<{}>", *columns as u8, *rows as u8, scalar_name(*scalar)),
        T::Atomic(s) => format!("atomic<{}>", scalar_name(*s)),
        T::Array { base, size, .. } => match size {
            naga::ArraySize::Constant(n) => format!("array<{}, {n}>", type_name(module, *base)),
            naga::ArraySize::Pending(_) => format!("array<{}, override>", type_name(module, *base)),
            naga::ArraySize::Dynamic => format!("array<{}>", type_name(module, *base)),
        },
        T::BindingArray { base, size } => match size {
            naga::ArraySize::Constant(n) => format!("binding_array<{}, {n}>", type_name(module, *base)),
            naga::ArraySize::Pending(_) | naga::ArraySize::Dynamic => format!("binding_array<{}>", type_name(module, *base)),
        },
        T::Struct { .. } => "struct".to_string(),
        T::Sampler { comparison: true } => "sampler_comparison".to_string(),
        T::Sampler { comparison: false } => "sampler".to_string(),
        T::Image { dim, arrayed, class } => {
            let dim = match dim {
                naga::ImageDimension::D1 => "1d",
                naga::ImageDimension::D2 => "2d",
                naga::ImageDimension::D3 => "3d",
                naga::ImageDimension::Cube => "cube",
            };
            let arrayed = if *arrayed { "_array" } else { "" };
            match class {
                naga::ImageClass::Sampled { kind, multi } => {
                    let multi = if *multi { "multisampled_" } else { "" };
                    let kind = match kind {
                        naga::ScalarKind::Sint => "i32",
                        naga::ScalarKind::Uint => "u32",
                        _ => "f32",
                    };
                    format!("texture_{multi}{dim}{arrayed}<{kind}>")
                }
                naga::ImageClass::Depth { multi } => {
                    let multi = if *multi { "multisampled_" } else { "" };
                    format!("texture_depth_{multi}{dim}{arrayed}")
                }
                naga::ImageClass::Storage { format, access } => {
                    let access = if access.contains(naga::StorageAccess::ATOMIC) {
                        "atomic"
                    } else if access.contains(naga::StorageAccess::LOAD | naga::StorageAccess::STORE) {
                        "read_write"
                    } else if access.contains(naga::StorageAccess::LOAD) {
                        "read"
                    } else {
                        "write"
                    };
                    format!("texture_storage_{dim}{arrayed}<{}, {access}>", format.to_wgsl())
                }
                naga::ImageClass::External => "texture_external".to_string(),
            }
        }
        T::AccelerationStructure { .. } => "acceleration_structure".to_string(),
        T::RayQuery { .. } => "ray_query".to_string(),
        other => format!("{other:?}"),
    }
}

/// The type of each number in a buffer of `ty`: the scalar of an array's
/// element, of a struct's last runtime-sized array, or of the type itself.
fn element_of(module: &naga::Module, ty: naga::Handle<naga::Type>) -> Option<ElementType> {
    use naga::TypeInner as T;
    match &module.types[ty].inner {
        T::Scalar(s) | T::Atomic(s) | T::Vector { scalar: s, .. } | T::Matrix { scalar: s, .. } => ElementType::from_scalar(*s),
        T::Array { base, .. } => element_of(module, *base),
        T::Struct { members, .. } => {
            let last = members.last()?;
            match &module.types[last.ty].inner {
                T::Array { size: naga::ArraySize::Dynamic, base, .. } => element_of(module, *base),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The type a buffer of `ty` binds and reflects as: the member of a struct
/// that is unnamed and holds one unnamed member at offset 0, the wrapper
/// naga's SPIR-V writer puts around every buffer it writes, so a shader
/// read back from that SPIR-V binds by the names its source had; `ty`
/// itself otherwise.
fn unwrapped(module: &naga::Module, ty: naga::Handle<naga::Type>) -> naga::Handle<naga::Type> {
    let t = &module.types[ty];
    match &t.inner {
        naga::TypeInner::Struct { members, .. } if t.name.is_none() && members.len() == 1 && members[0].name.is_none() && members[0].offset == 0 => members[0].ty,
        _ => ty,
    }
}

/// (fixed bytes, bytes per array element) of a buffer of `ty`.
fn sizes(module: &naga::Module, ty: naga::Handle<naga::Type>) -> (u64, u64) {
    use naga::TypeInner as T;
    let inner = &module.types[ty].inner;
    match inner {
        T::Array { size: naga::ArraySize::Dynamic, stride, .. } => (0, *stride as u64),
        T::Array { size: naga::ArraySize::Constant(n), stride, .. } => (n.get() as u64 * *stride as u64, *stride as u64),
        T::Struct { members, span } => match members.last().map(|m| (&module.types[m.ty].inner, m.offset)) {
            Some((T::Array { size: naga::ArraySize::Dynamic, stride, .. }, offset)) => (offset as u64, *stride as u64),
            _ => (*span as u64, 0),
        },
        other => (other.size(module.to_ctx()) as u64, 0),
    }
}

fn storage_format(format: naga::StorageFormat) -> wgpu::TextureFormat {
    use naga::StorageFormat as S;
    use wgpu::TextureFormat as F;
    match format {
        S::R8Unorm => F::R8Unorm,
        S::R8Snorm => F::R8Snorm,
        S::R8Uint => F::R8Uint,
        S::R8Sint => F::R8Sint,
        S::R16Uint => F::R16Uint,
        S::R16Sint => F::R16Sint,
        S::R16Float => F::R16Float,
        S::Rg8Unorm => F::Rg8Unorm,
        S::Rg8Snorm => F::Rg8Snorm,
        S::Rg8Uint => F::Rg8Uint,
        S::Rg8Sint => F::Rg8Sint,
        S::R32Uint => F::R32Uint,
        S::R32Sint => F::R32Sint,
        S::R32Float => F::R32Float,
        S::Rg16Uint => F::Rg16Uint,
        S::Rg16Sint => F::Rg16Sint,
        S::Rg16Float => F::Rg16Float,
        S::Rgba8Unorm => F::Rgba8Unorm,
        S::Rgba8Snorm => F::Rgba8Snorm,
        S::Rgba8Uint => F::Rgba8Uint,
        S::Rgba8Sint => F::Rgba8Sint,
        S::Bgra8Unorm => F::Bgra8Unorm,
        S::Rgb10a2Uint => F::Rgb10a2Uint,
        S::Rgb10a2Unorm => F::Rgb10a2Unorm,
        S::Rg11b10Ufloat => F::Rg11b10Ufloat,
        S::R64Uint => F::R64Uint,
        S::Rg32Uint => F::Rg32Uint,
        S::Rg32Sint => F::Rg32Sint,
        S::Rg32Float => F::Rg32Float,
        S::Rgba16Uint => F::Rgba16Uint,
        S::Rgba16Sint => F::Rgba16Sint,
        S::Rgba16Float => F::Rgba16Float,
        S::Rgba32Uint => F::Rgba32Uint,
        S::Rgba32Sint => F::Rgba32Sint,
        S::Rgba32Float => F::Rgba32Float,
        S::R16Unorm => F::R16Unorm,
        S::R16Snorm => F::R16Snorm,
        S::Rg16Unorm => F::Rg16Unorm,
        S::Rg16Snorm => F::Rg16Snorm,
        S::Rgba16Unorm => F::Rgba16Unorm,
        S::Rgba16Snorm => F::Rgba16Snorm,
    }
}

fn view_dimension(dim: naga::ImageDimension, arrayed: bool) -> wgpu::TextureViewDimension {
    match (dim, arrayed) {
        (naga::ImageDimension::D1, _) => wgpu::TextureViewDimension::D1,
        (naga::ImageDimension::D2, false) => wgpu::TextureViewDimension::D2,
        (naga::ImageDimension::D2, true) => wgpu::TextureViewDimension::D2Array,
        (naga::ImageDimension::D3, _) => wgpu::TextureViewDimension::D3,
        (naga::ImageDimension::Cube, false) => wgpu::TextureViewDimension::Cube,
        (naga::ImageDimension::Cube, true) => wgpu::TextureViewDimension::CubeArray,
    }
}

/// The place of one member in a struct buffer, for packing it from a
/// hashtable.
#[derive(Clone)]
pub(crate) struct Member {
    pub(crate) name: String,
    pub(crate) offset: u32,
    /// The scalar type of each number; `None` for a member that is not
    /// numbers (a nested struct).
    pub(crate) element: Option<ElementType>,
    /// Numbers per column: 1 for a scalar, the size of a vector, the rows
    /// of a matrix.
    pub(crate) rows: u32,
    /// Columns of a matrix; 1 otherwise.
    pub(crate) columns: u32,
    /// Bytes from one column of a matrix to the next.
    pub(crate) column_stride: u32,
    /// Elements of a fixed-size array member; 1 otherwise.
    pub(crate) count: u32,
    /// Bytes from one array element to the next.
    pub(crate) stride: u32,
}

impl Member {
    /// How many numbers the member holds.
    pub(crate) fn numbers(&self) -> u32 {
        self.rows * self.columns * self.count
    }
}

/// The members of a struct buffer type, empty for any other type.
fn members(module: &naga::Module, ty: naga::Handle<naga::Type>) -> Vec<Member> {
    use naga::TypeInner as T;
    let T::Struct { members, .. } = &module.types[ty].inner else {
        return Vec::new();
    };
    members
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let (base, count, stride) = match &module.types[m.ty].inner {
                T::Array { base, size: naga::ArraySize::Constant(n), stride } => (*base, n.get(), *stride),
                _ => (m.ty, 1, 0),
            };
            let (element, rows, columns, column_stride) = match &module.types[base].inner {
                T::Scalar(s) | T::Atomic(s) => (ElementType::from_scalar(*s), 1, 1, 0),
                T::Vector { size, scalar } => (ElementType::from_scalar(*scalar), *size as u32, 1, 0),
                T::Matrix { columns, rows, scalar } => {
                    let rows = *rows as u32;
                    let column_stride = if rows == 2 { 2 * scalar.width as u32 } else { 4 * scalar.width as u32 };
                    (ElementType::from_scalar(*scalar), rows, *columns as u32, column_stride)
                }
                _ => (None, 1, 1, 0),
            };
            let name = match &m.name {
                Some(name) => name.clone(),
                None => format!("member{i}"),
            };
            Member { name, offset: m.offset, element, rows, columns, column_stride, count, stride }
        })
        .collect()
}

/// A resource binding, with everything a layout and a bind group need.
#[derive(Clone)]
pub(crate) struct Reflected {
    pub(crate) group: u32,
    pub(crate) binding: u32,
    pub(crate) name: String,
    pub(crate) kind: BindingKind,
    pub(crate) ty: wgpu::BindingType,
    pub(crate) count: Option<NonZeroU32>,
    pub(crate) element_type: Option<ElementType>,
    pub(crate) type_name: String,
    pub(crate) size: u64,
    pub(crate) stride: u64,
    /// For a buffer of a struct type, the place of each member.
    pub(crate) members: Vec<Member>,
}

impl Reflected {
    pub(crate) fn describe(&self) -> ShaderBinding {
        ShaderBinding {
            group: self.group,
            binding: self.binding,
            name: self.name.clone(),
            kind: self.kind,
            wgsl_type: self.type_name.clone(),
            element_type: self.element_type,
            size: self.size,
            stride: self.stride,
            count: self.count.map_or(0, NonZeroU32::get),
        }
    }

    /// Whether the shader may write the buffer.
    pub(crate) fn writable(&self) -> bool {
        matches!(self.kind, BindingKind::StorageBuffer)
    }
}

/// The bindings `module` declares, in group and binding order; with an
/// entry point, only the ones it uses. A sampled float texture is
/// filterable when the entry point samples it.
pub(crate) fn reflect(module: &naga::Module, info: &naga::valid::ModuleInfo, entry: Option<usize>) -> Vec<Reflected> {
    let function = entry.map(|i| info.get_entry_point(i));
    let mut out = Vec::new();
    for (handle, var) in module.global_variables.iter() {
        let Some(binding) = &var.binding else { continue };
        if let Some(f) = function
            && f[handle].is_empty()
        {
            continue;
        }
        let sampled = function.is_some_and(|f| f.sampling_set.iter().any(|key| key.image == handle));
        let (inner_ty, count) = match &module.types[var.ty].inner {
            naga::TypeInner::BindingArray { base, size: naga::ArraySize::Constant(n) } => (*base, Some(*n)),
            naga::TypeInner::BindingArray { base, .. } => (*base, None),
            _ => (var.ty, None),
        };
        let (kind, ty) = match var.space {
            naga::AddressSpace::Uniform => (BindingKind::UniformBuffer, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
            naga::AddressSpace::Storage { access } => {
                let read_only = !access.contains(naga::StorageAccess::STORE);
                let kind = if read_only { BindingKind::ReadOnlyStorageBuffer } else { BindingKind::StorageBuffer };
                (kind, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None })
            }
            naga::AddressSpace::Handle => match &module.types[inner_ty].inner {
                naga::TypeInner::Sampler { comparison: true } => (BindingKind::ComparisonSampler, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)),
                naga::TypeInner::Sampler { comparison: false } => (BindingKind::Sampler, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                naga::TypeInner::Image { dim, arrayed, class } => match class {
                    naga::ImageClass::Sampled { kind, multi } => {
                        let sample_type = match kind {
                            naga::ScalarKind::Sint => wgpu::TextureSampleType::Sint,
                            naga::ScalarKind::Uint => wgpu::TextureSampleType::Uint,
                            _ => wgpu::TextureSampleType::Float { filterable: sampled && !*multi },
                        };
                        (BindingKind::Texture, wgpu::BindingType::Texture { sample_type, view_dimension: view_dimension(*dim, *arrayed), multisampled: *multi })
                    }
                    naga::ImageClass::Depth { multi } => (
                        BindingKind::Texture,
                        wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: view_dimension(*dim, *arrayed), multisampled: *multi },
                    ),
                    naga::ImageClass::Storage { format, access } => {
                        let load = access.contains(naga::StorageAccess::LOAD);
                        let store = access.contains(naga::StorageAccess::STORE);
                        let access = if access.contains(naga::StorageAccess::ATOMIC) {
                            wgpu::StorageTextureAccess::Atomic
                        } else if load && store {
                            wgpu::StorageTextureAccess::ReadWrite
                        } else if load {
                            wgpu::StorageTextureAccess::ReadOnly
                        } else {
                            wgpu::StorageTextureAccess::WriteOnly
                        };
                        (BindingKind::StorageTexture, wgpu::BindingType::StorageTexture { access, format: storage_format(*format), view_dimension: view_dimension(*dim, *arrayed) })
                    }
                    naga::ImageClass::External => (BindingKind::ExternalTexture, wgpu::BindingType::ExternalTexture),
                },
                naga::TypeInner::AccelerationStructure { vertex_return } => {
                    (BindingKind::AccelerationStructure, wgpu::BindingType::AccelerationStructure { vertex_return: *vertex_return })
                }
                _ => continue,
            },
            _ => continue,
        };
        let is_buffer = matches!(kind, BindingKind::UniformBuffer | BindingKind::StorageBuffer | BindingKind::ReadOnlyStorageBuffer);
        let shown = if is_buffer { unwrapped(module, inner_ty) } else { inner_ty };
        // The declared type's fixed size, when larger, keeps a buffer as
        // large as the binding wgpu validates.
        let (size, stride) = if is_buffer {
            let (size, stride) = sizes(module, shown);
            (size.max(sizes(module, inner_ty).0), stride)
        } else {
            (0, 0)
        };
        let element_type = if is_buffer { element_of(module, shown) } else { None };
        let members = if is_buffer { members(module, shown) } else { Vec::new() };
        let name = match &var.name {
            Some(name) => name.clone(),
            None => format!("binding{}_{}", binding.group, binding.binding),
        };
        out.push(Reflected {
            group: binding.group,
            binding: binding.binding,
            name,
            kind,
            ty,
            count,
            element_type,
            type_name: type_name(module, if shown == inner_ty { var.ty } else { shown }),
            size,
            stride,
            members,
        });
    }
    out.sort_by_key(|r| (r.group, r.binding));
    out
}

/// The bytes of immediate data the entry point reads; 0 when it reads none.
pub(crate) fn immediate_size(module: &naga::Module, info: &naga::valid::ModuleInfo, entry: usize) -> u32 {
    let function = info.get_entry_point(entry);
    module
        .global_variables
        .iter()
        .filter(|(h, v)| v.space == naga::AddressSpace::Immediate && !function[*h].is_empty())
        .map(|(_, v)| module.types[v.ty].inner.size(module.to_ctx()))
        .max()
        .unwrap_or(0)
}

fn entry_points(module: &naga::Module, info: &naga::valid::ModuleInfo) -> Vec<ShaderEntryPoint> {
    module
        .entry_points
        .iter()
        .enumerate()
        .map(|(i, ep)| ShaderEntryPoint {
            name: ep.name.clone(),
            stage: ShaderStage::from_naga(ep.stage),
            workgroup_size: ep.workgroup_size.to_vec(),
            bindings: reflect(module, info, Some(i)).into_iter().map(|r| r.name).collect(),
        })
        .collect()
}

fn overrides(module: &naga::Module) -> Vec<ShaderOverride> {
    module
        .overrides
        .iter()
        .map(|(_, o)| ShaderOverride {
            name: o.name.clone(),
            id: o.id.map(u32::from),
            wgsl_type: type_name(module, o.ty),
            default: o.init.and_then(|init| match module.global_expressions[init] {
                naga::Expression::Literal(literal) => literal_value(literal),
                _ => None,
            }),
        })
        .collect()
}

fn literal_value(literal: naga::Literal) -> Option<f64> {
    Some(match literal {
        naga::Literal::F64(v) => v,
        naga::Literal::F32(v) => v as f64,
        naga::Literal::F16(v) => v.to_f64(),
        naga::Literal::U16(v) => v as f64,
        naga::Literal::I16(v) => v as f64,
        naga::Literal::U32(v) => v as f64,
        naga::Literal::I32(v) => v as f64,
        naga::Literal::U64(v) => v as f64,
        naga::Literal::I64(v) => v as f64,
        naga::Literal::Bool(v) => f64::from(u8::from(v)),
        naga::Literal::AbstractInt(v) => v as f64,
        naga::Literal::AbstractFloat(v) => v,
    })
}

/// Everything `parsed` declares.
pub(crate) fn info_of(parsed: &Parsed) -> ShaderInfo {
    ShaderInfo {
        label: parsed.label.clone(),
        language: parsed.source.language(),
        entry_points: entry_points(&parsed.module, &parsed.info),
        bindings: reflect(&parsed.module, &parsed.info, None).iter().map(Reflected::describe).collect(),
        overrides: overrides(&parsed.module),
    }
}

/// The index of entry point `name`, or of the only entry point of
/// `stage` when no name is given.
pub(crate) fn entry_index(module: &naga::Module, name: Option<&str>, stage: naga::ShaderStage) -> PsResult<usize> {
    match name {
        Some(name) => module
            .entry_points
            .iter()
            .position(|ep| ep.name == name)
            .ok_or_else(|| not_found("WgpuNoEntryPoint", format!("the shader has no entry point named {name}"))),
        None => {
            let mut matching = module.entry_points.iter().enumerate().filter(|(_, ep)| ep.stage == stage);
            match (matching.next(), matching.next()) {
                (Some((i, _)), None) => Ok(i),
                (None, _) => Err(not_found("WgpuNoEntryPoint", format!("the shader has no {stage:?} entry point"))),
                (Some(_), Some(_)) => Err(invalid("WgpuEntryPointAmbiguous", format!("the shader has several {stage:?} entry points; name one with -Entry"))),
            }
        }
    }
}

/// The module with every override replaced by its default, for the back
/// ends that cannot write overrides.
fn resolved<'a>(parsed: &'a Parsed, entry: Option<(naga::ShaderStage, &str)>) -> PsResult<(Cow<'a, naga::Module>, Cow<'a, naga::valid::ModuleInfo>)> {
    naga::back::pipeline_constants::process_overrides(&parsed.module, &parsed.info, entry, &naga::back::PipelineConstants::default())
        .map_err(|e| invalid("WgpuOverrides", format!("the shader's override constants cannot be resolved: {}", chain(&e))))
}

/// Translation options ConvertTo-WgpuShader and `Shader.ConvertTo` take.
#[derive(Clone, Default)]
pub(crate) struct Translation {
    pub(crate) entry: Option<String>,
    pub(crate) stage: Option<ShaderStage>,
    pub(crate) glsl_version: Option<u16>,
    pub(crate) glsl_es: bool,
    pub(crate) hlsl_shader_model: Option<String>,
    pub(crate) msl_version: Option<String>,
    pub(crate) spirv_version: Option<String>,
}

fn version_pair(text: &str, what: &str) -> PsResult<(u8, u8)> {
    let refuse = |why: String| invalid("WgpuVersion", format!("{text} is not a {what} version such as 1.5: {why}"));
    let mut parts = text.split('.');
    let (Some(major), Some(minor), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(refuse("it is not two numbers joined by a dot".to_string()));
    };
    let major = major.trim().parse::<u8>().map_err(|e| refuse(e.to_string()))?;
    let minor = minor.trim().parse::<u8>().map_err(|e| refuse(e.to_string()))?;
    Ok((major, minor))
}

/// `parsed` in `target`: text for WGSL, GLSL, HLSL, MSL and Dot, `uint[]`
/// words for SPIR-V.
pub(crate) fn translate(parsed: &Parsed, target: ShaderTarget, t: &Translation) -> PsResult<PsObject> {
    let fail = |e: &dyn std::error::Error| invalid("WgpuTranslate", format!("the shader cannot be written as {target:?}: {}", chain(e)));
    let pick = |i: usize| (parsed.module.entry_points[i].stage, parsed.module.entry_points[i].name.clone());
    let entry = match (&t.entry, t.stage) {
        (Some(name), _) => Some(pick(entry_index(&parsed.module, Some(name), naga::ShaderStage::Compute)?)),
        (None, Some(stage)) => Some(pick(entry_index(&parsed.module, None, stage.naga()?)?)),
        (None, None) => None,
    };
    match target {
        ShaderTarget::Wgsl => {
            let text = naga::back::wgsl::write_string(&parsed.module, &parsed.info, naga::back::wgsl::WriterFlags::empty()).map_err(|e| fail(&e))?;
            text.into_ps()
        }
        ShaderTarget::Dot => {
            let text = naga::back::dot::write(&parsed.module, Some(&parsed.info), naga::back::dot::Options::default()).map_err(|e| fail(&e))?;
            text.into_ps()
        }
        ShaderTarget::SpirV => {
            let mut options = naga::back::spv::Options::default();
            // Every build writes the shader's names, OpName and OpMemberName,
            // as glslc does, so a shader read back keeps the names of its
            // bindings and members; naga's default writes them in a debug
            // build only. With no debug_info it writes no source or lines.
            options.flags.insert(naga::back::spv::WriterFlags::DEBUG);
            if let Some(v) = &t.spirv_version {
                options.lang_version = version_pair(v, "SPIR-V")?;
            }
            let pipeline = entry.as_ref().map(|(stage, name)| naga::back::spv::PipelineOptions { shader_stage: *stage, entry_point: name.clone() });
            let (module, info) = resolved(parsed, entry.as_ref().map(|(s, n)| (*s, n.as_str())))?;
            let words = naga::back::spv::write_vec(&module, &info, &options, pipeline.as_ref()).map_err(|e| fail(&e))?;
            PsObject::from_slice::<u32>(&words)
        }
        ShaderTarget::Glsl => {
            let (stage, name) = match &entry {
                Some(e) => e.clone(),
                None => pick(entry_index(&parsed.module, None, naga::ShaderStage::Compute)?),
            };
            let version = if t.glsl_es {
                naga::back::glsl::Version::Embedded { version: t.glsl_version.unwrap_or(310), is_webgl: false }
            } else {
                naga::back::glsl::Version::Desktop(t.glsl_version.unwrap_or(450))
            };
            let options = naga::back::glsl::Options { version, ..Default::default() };
            let pipeline = naga::back::glsl::PipelineOptions { shader_stage: stage, entry_point: name.clone(), multiview: None };
            let (module, info) = resolved(parsed, Some((stage, &name)))?;
            let mut text = String::new();
            let mut writer = naga::back::glsl::Writer::new(&mut text, &module, &info, &options, &pipeline, naga::proc::BoundsCheckPolicies::default()).map_err(|e| fail(&e))?;
            writer.write().map_err(|e| fail(&e))?;
            text.into_ps()
        }
        ShaderTarget::Hlsl => {
            let shader_model = match t.hlsl_shader_model.as_deref() {
                None => naga::back::hlsl::ShaderModel::V6_0,
                Some(v) => match version_pair(v, "shader model")? {
                    (5, 0) => naga::back::hlsl::ShaderModel::V5_0,
                    (5, 1) => naga::back::hlsl::ShaderModel::V5_1,
                    (6, 0) => naga::back::hlsl::ShaderModel::V6_0,
                    (6, 1) => naga::back::hlsl::ShaderModel::V6_1,
                    (6, 2) => naga::back::hlsl::ShaderModel::V6_2,
                    (6, 3) => naga::back::hlsl::ShaderModel::V6_3,
                    (6, 4) => naga::back::hlsl::ShaderModel::V6_4,
                    (6, 5) => naga::back::hlsl::ShaderModel::V6_5,
                    (6, 6) => naga::back::hlsl::ShaderModel::V6_6,
                    (6, 7) => naga::back::hlsl::ShaderModel::V6_7,
                    (6, 8) => naga::back::hlsl::ShaderModel::V6_8,
                    (6, 9) => naga::back::hlsl::ShaderModel::V6_9,
                    (major, minor) => return Err(invalid("WgpuVersion", format!("HLSL has no shader model {major}.{minor} naga writes; it writes 5.0 to 6.9"))),
                },
            };
            let options = naga::back::hlsl::Options { shader_model, ..Default::default() };
            let pipeline = naga::back::hlsl::PipelineOptions { entry_point: entry.clone() };
            let (module, info) = resolved(parsed, entry.as_ref().map(|(s, n)| (*s, n.as_str())))?;
            let mut text = String::new();
            let mut writer = naga::back::hlsl::Writer::new(&mut text, &options, &pipeline);
            writer.write(&module, &info, None).map_err(|e| fail(&e))?;
            text.into_ps()
        }
        ShaderTarget::Msl => {
            let lang_version = match &t.msl_version {
                Some(v) => version_pair(v, "Metal Shading Language")?,
                None => (2, 4),
            };
            let options = naga::back::msl::Options { lang_version, ..Default::default() };
            let pipeline = naga::back::msl::PipelineOptions { entry_point: entry.clone(), ..Default::default() };
            let (module, info) = resolved(parsed, entry.as_ref().map(|(s, n)| (*s, n.as_str())))?;
            let (text, _) = naga::back::msl::write_string(&module, &info, &options, &pipeline).map_err(|e| fail(&e))?;
            text.into_ps()
        }
    }
}

/// Compiles `parsed` on a device.
pub(crate) fn module_on(core: &CoreRef, parsed: &Parsed) -> PsResult<wgpu::ShaderModule> {
    let defines: Vec<(&str, &str)> = match &parsed.source {
        Source::Glsl { defines, .. } => defines.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect(),
        Source::Wgsl(_) | Source::SpirV(_) => Vec::new(),
    };
    // SPIR-V goes to the device as the module naga read it, flipped into
    // WebGPU's clip space as parse reads it, since wgpu reads SPIR-V words
    // in WebGPU's clip space itself.
    let source = match &parsed.source {
        Source::Wgsl(text) => wgpu::ShaderSource::Wgsl(Cow::Borrowed(text)),
        Source::Glsl { text, stage, .. } => wgpu::ShaderSource::Glsl { shader: Cow::Borrowed(text), stage: *stage, defines: &defines },
        Source::SpirV(_) => wgpu::ShaderSource::Naga(Cow::Owned(parsed.module.clone())),
    };
    core.scoped("WgpuShader", "the shader cannot be compiled on this device", || {
        core.device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some(&parsed.label), source })
    })
}

/// Reads SPIR-V from a `byte[]`, a `uint[]` or any collection of words.
pub(crate) fn spirv_words(obj: &PsObject) -> PsResult<Vec<u32>> {
    if obj.type_name()? == "System.Byte[]" {
        let bytes = obj.pin::<u8>()?;
        if bytes.len() % 4 != 0 {
            return Err(invalid("WgpuSpirV", format!("SPIR-V is a whole number of 4-byte words; {} bytes is not", bytes.len())));
        }
        return Ok(bytemuck::pod_collect_to_vec(&bytes[..]));
    }
    Vec::<u32>::from_ps(obj)
}

/// The source a file holds, its language taken from `language` or from
/// the extension: .wgsl, .spv, or a GLSL extension (.comp, .vert, .frag,
/// .glsl, .mesh, .task).
pub(crate) fn source_from_file(path: &Path, language: Option<ShaderLanguage>, stage: Option<ShaderStage>, defines: Vec<(String, String)>) -> PsResult<Source> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let language = match (language, ext.as_deref()) {
        (Some(l), _) => l,
        (None, Some("spv")) => ShaderLanguage::SpirV,
        (None, Some("comp" | "vert" | "frag" | "glsl" | "mesh" | "task")) => ShaderLanguage::Glsl,
        (None, _) => ShaderLanguage::Wgsl,
    };
    let stage = match (stage, ext.as_deref()) {
        (Some(s), _) => s,
        (None, Some("vert")) => ShaderStage::Vertex,
        (None, Some("frag")) => ShaderStage::Fragment,
        (None, Some("mesh")) => ShaderStage::Mesh,
        (None, Some("task")) => ShaderStage::Task,
        (None, _) => ShaderStage::Compute,
    };
    let read_error = |e: std::io::Error| not_found("WgpuShaderFile", format!("{} cannot be read: {e}", path.display()));
    Ok(match language {
        ShaderLanguage::SpirV => {
            let bytes = std::fs::read(path).map_err(read_error)?;
            if bytes.len() % 4 != 0 {
                return Err(invalid("WgpuSpirV", format!("{} is not a whole number of 4-byte SPIR-V words", path.display())));
            }
            Source::SpirV(bytemuck::pod_collect_to_vec(&bytes))
        }
        ShaderLanguage::Glsl => Source::Glsl { text: std::fs::read_to_string(path).map_err(read_error)?, stage: stage.naga()?, defines },
        ShaderLanguage::Wgsl => Source::Wgsl(std::fs::read_to_string(path).map_err(read_error)?),
    })
}

/// The source text holds: WGSL, or GLSL when `language` says so or the
/// text starts with `#version`, of `stage` (Compute when omitted).
pub(crate) fn source_from_text(text: String, language: Option<ShaderLanguage>, stage: Option<ShaderStage>, defines: Vec<(String, String)>) -> PsResult<Source> {
    let language = match language {
        Some(l) => l,
        None if text.trim_start().starts_with("#version") => ShaderLanguage::Glsl,
        None => ShaderLanguage::Wgsl,
    };
    Ok(match language {
        ShaderLanguage::Wgsl => Source::Wgsl(text),
        ShaderLanguage::Glsl => Source::Glsl { text, stage: stage.unwrap_or(ShaderStage::Compute).naga()?, defines },
        ShaderLanguage::SpirV => return Err(invalid("WgpuSpirV", "SPIR-V is binary; give it as words or bytes with -SpirV, or as a .spv file")),
    })
}

/// A shader parsed and validated by naga, independent of any device: it
/// compiles on whichever device a pipeline is made on.
#[psclass(name = "Wgpu.Shader", mode = proxy, columns = ["Label", "Language", "EntryPoints"])]
pub struct Shader {
    /// The shader's label.
    pub label: String,
    /// The language it was written in.
    pub language: ShaderLanguage,
    /// Its entry points.
    pub entry_points: Vec<ShaderEntryPoint>,
    /// Its resource bindings, in group and binding order.
    pub bindings: Vec<ShaderBinding>,
    /// Its pipeline-overridable constants.
    pub overrides: Vec<ShaderOverride>,
    /// Its source text; `$null` for SPIR-V.
    pub source: Option<String>,
    #[psfield(skip)]
    pub(crate) parsed: Arc<Parsed>,
}

impl Shader {
    pub(crate) fn from_parsed(parsed: Arc<Parsed>) -> Shader {
        let info = info_of(&parsed);
        Shader {
            label: parsed.label.clone(),
            language: info.language,
            entry_points: info.entry_points,
            bindings: info.bindings,
            overrides: info.overrides,
            source: parsed.source.text().map(str::to_string),
            parsed,
        }
    }

    pub(crate) fn compile(source: Source, label: Option<String>) -> PsResult<Shader> {
        let label = label.unwrap_or_else(|| "Wgpu shader".to_string());
        Ok(Shader::from_parsed(compile(source, label)?))
    }
}

#[psmethods]
impl Shader {
    /// Parses and validates `source`: WGSL, or GLSL of `stage` (Compute when
    /// omitted) when `language` says so or the text starts with `#version`.
    pub fn new(source: String, language: Option<ShaderLanguage>, stage: Option<ShaderStage>) -> PsResult<Self> {
        Shader::compile(source_from_text(source, language, stage, Vec::new())?, None)
    }

    /// Parses and validates a shader file: .wgsl, .spv, or GLSL (.comp,
    /// .vert, .frag, .glsl, .mesh, .task).
    pub fn from_file(path: String, language: Option<ShaderLanguage>, stage: Option<ShaderStage>) -> PsResult<Shader> {
        Shader::compile(source_from_file(Path::new(&path), language, stage, Vec::new())?, Some(path))
    }

    /// Parses and validates SPIR-V given as `byte[]` or `uint[]`.
    pub fn from_spir_v(words: PsObject) -> PsResult<Shader> {
        Shader::compile(Source::SpirV(spirv_words(&words)?), None)
    }

    /// The shader written in `target`, for `entry` when given: text, or
    /// `uint[]` for SPIR-V.
    pub fn convert_to(&self, target: ShaderTarget, entry: Option<String>) -> PsResult<PsObject> {
        translate(&self.parsed, target, &Translation { entry, ..Default::default() })
    }

    /// Everything the shader declares.
    pub fn info(&self) -> PsResult<ShaderInfo> {
        Ok(info_of(&self.parsed))
    }
}

fn defines_of(table: &Option<PsHashtable>) -> PsResult<Vec<(String, String)>> {
    let mut out = Vec::new();
    if let Some(table) = table {
        for key in table.keys()? {
            let value = String::from_ps(&table.get(&key)?)?;
            out.push((key, value));
        }
    }
    Ok(out)
}

/// What a shader cmdlet's -Source, -Path, -LiteralPath or -SpirV gives,
/// with the parameters that say how to read it.
struct ShaderInput<'a> {
    source: &'a Option<String>,
    path: &'a Option<String>,
    literal_path: &'a Option<String>,
    spirv: &'a Option<PsObject>,
    language: Option<ShaderLanguage>,
    stage: Option<ShaderStage>,
    defines: &'a Option<PsHashtable>,
}

/// Runs `each` on every shader `input` names, with its label: the text or
/// SPIR-V words given, or each file a -Path matches or a -LiteralPath
/// names, labeled with its path. A file that cannot be read, or that
/// `each` fails on, is written to the error stream and the files after it
/// still run. `one_file` refuses a -Path that matches more than one file,
/// for an -OutFile that holds one result.
fn for_each_shader(ps: &Pipeline<'_>, input: &ShaderInput<'_>, one_file: bool, mut each: impl FnMut(Source, String) -> PsResult<()>) -> PsResult<()> {
    let defines = defines_of(input.defines)?;
    if let Some(words) = input.spirv {
        return each(Source::SpirV(spirv_words(words)?), "Wgpu shader".to_string());
    }
    let (path, literal) = match (input.path.as_deref(), input.literal_path.as_deref()) {
        (Some(path), _) => (path, false),
        (None, Some(path)) => (path, true),
        (None, None) => {
            return match input.source {
                Some(text) => each(source_from_text(text.clone(), input.language, input.stage, defines)?, "Wgpu shader".to_string()),
                None => Err(invalid("WgpuNoShader", "give the shader as -Source, -Path, -LiteralPath or -SpirV")),
            };
        }
    };
    let files = input_files(ps, path, literal, "WgpuShaderFile")?;
    if one_file && files.len() > 1 {
        return Err(invalid("WgpuOutFile", format!("-OutFile holds one shader, and {path} matches {} files: {}", files.len(), files.join(", "))));
    }
    for_each_file(ps, files, |file| each(source_from_file(Path::new(&file), input.language, input.stage, defines.clone())?, file))
}

/// Parses and validates a shader, independent of any device.
///
/// The shader is WGSL text, GLSL text (with -Language Glsl or a leading
/// #version), a file, or SPIR-V words. It compiles on whichever device a
/// pipeline is made on.
///
/// # Examples
/// $shader = New-WgpuShader -Source $wgsl
/// New-WgpuShader -Path .\blur.comp
/// Get-ChildItem .\shaders -Filter *.wgsl | New-WgpuShader
/// New-WgpuShader -SpirV ([IO.File]::ReadAllBytes("$PWD\kernel.spv"))
#[cmdlet(verb = "New", noun = "WgpuShader", alias = ["New-GpuShader"], default_parameter_set = "Source", output = ["Wgpu.Shader"])]
#[derive(Default)]
pub struct NewWgpuShader {
    /// The shader text.
    #[param(mandatory, position = 0, value_from_pipeline, set = "Source")]
    pub source: Option<String>,
    /// A shader file: .wgsl, .spv, or GLSL (.comp, .vert, .frag, .glsl); a
    /// wildcard makes a shader of every file it matches.
    #[param(mandatory, set = "Path", supports_wildcards)]
    pub path: Option<String>,
    /// A shader file named exactly as written, wildcard characters
    /// included; the files Get-ChildItem pipes bind here.
    #[param(mandatory, set = "LiteralPath", value_from_pipeline_by_property_name, alias = ["PSPath", "LP", "FullName"])]
    pub literal_path: Option<String>,
    /// SPIR-V as byte[] or uint[].
    #[param(mandatory, set = "SpirV", raw)]
    pub spir_v: Option<PsObject>,
    /// The language; inferred from the text or the file extension when omitted.
    #[param]
    pub language: Option<ShaderLanguage>,
    /// The stage of a GLSL shader; Compute when omitted.
    #[param]
    pub stage: Option<ShaderStage>,
    /// Preprocessor definitions for GLSL, as name = value.
    #[param]
    pub define: Option<PsHashtable>,
    /// The shader's label.
    #[param]
    pub label: Option<String>,
}

impl Cmdlet for NewWgpuShader {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let input = ShaderInput {
            source: &self.source,
            path: &self.path,
            literal_path: &self.literal_path,
            spirv: &self.spir_v,
            language: self.language,
            stage: self.stage,
            defines: &self.define,
        };
        for_each_shader(ps, &input, false, |source, label| {
            let label = match &self.label {
                Some(given) => given.clone(),
                None => label,
            };
            ps.write(Shader::compile(source, Some(label))?)
        })
    }
}

/// Validates a shader and writes whether it is valid, or with
/// -Diagnostics every problem found, with its line and column.
///
/// Without -Device the shader is checked by naga as a device with every
/// feature would check it; with -Device it is compiled on that device,
/// against the features it has. A shader made with New-WgpuShader was
/// validated as it was made, so it tests valid, and with -Device is
/// compiled on that device.
///
/// # Examples
/// Test-WgpuShader -Source $wgsl
/// Test-WgpuShader -Path .\kernel.wgsl -Diagnostics
/// Get-ChildItem .\shaders -Filter *.wgsl | Test-WgpuShader
/// Test-WgpuShader -Source $wgsl -Device $gpu
/// New-WgpuShader -Path .\kernel.wgsl | Test-WgpuShader -Device $gpu
#[cmdlet(verb = "Test", noun = "WgpuShader", alias = ["Test-GpuShader"], default_parameter_set = "Source", output = ["System.Boolean", "Wgpu.ShaderDiagnostic"])]
#[derive(Default)]
pub struct TestWgpuShader {
    /// The shader text.
    #[param(mandatory, position = 0, value_from_pipeline, set = "Source")]
    pub source: Option<String>,
    /// A shader file: .wgsl, .spv, or GLSL (.comp, .vert, .frag, .glsl); a
    /// wildcard checks every file it matches.
    #[param(mandatory, set = "Path", supports_wildcards)]
    pub path: Option<String>,
    /// A shader file named exactly as written, wildcard characters
    /// included; the files Get-ChildItem pipes bind here.
    #[param(mandatory, set = "LiteralPath", value_from_pipeline_by_property_name, alias = ["PSPath", "LP", "FullName"])]
    pub literal_path: Option<String>,
    /// SPIR-V as byte[] or uint[].
    #[param(mandatory, set = "SpirV", raw)]
    pub spir_v: Option<PsObject>,
    /// A shader made with New-WgpuShader, given in place of text, by
    /// position or from the pipeline.
    #[param(mandatory, position = 0, value_from_pipeline, set = "Shader")]
    pub shader: Option<PsProxy<Shader>>,
    /// The language; inferred from the text or the file extension when omitted.
    #[param]
    pub language: Option<ShaderLanguage>,
    /// The stage of a GLSL shader; Compute when omitted.
    #[param]
    pub stage: Option<ShaderStage>,
    /// Preprocessor definitions for GLSL, as name = value.
    #[param]
    pub define: Option<PsHashtable>,
    /// Compile the shader on this device as well.
    #[param]
    pub device: Option<PsProxy<crate::device::Device>>,
    /// Write each problem found instead of $true or $false.
    #[param]
    pub diagnostics: bool,
}

impl TestWgpuShader {
    /// The problem compiling a valid shader on -Device finds, or none
    /// without -Device.
    fn on_device(&self, parsed: &Parsed) -> PsResult<Vec<ShaderDiagnostic>> {
        let Some(device) = &self.device else {
            return Ok(Vec::new());
        };
        let core = device.with(|d| d.core.clone())?;
        Ok(match module_on(&core, parsed) {
            Ok(_) => Vec::new(),
            Err(e) => vec![ShaderDiagnostic { severity: "Error".to_string(), message: e.message.clone(), detail: e.message, ..Default::default() }],
        })
    }

    /// Writes each problem found, with -Diagnostics, or whether none was.
    fn report(&self, ps: &Pipeline<'_>, found: Vec<ShaderDiagnostic>, label: &str) -> PsResult<()> {
        if self.diagnostics {
            for d in found {
                ps.write(ShaderDiagnostic { label: label.to_string(), ..d })?;
            }
            Ok(())
        } else {
            ps.write(found.is_empty())
        }
    }
}

impl Cmdlet for TestWgpuShader {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if let Some(shader) = &self.shader {
            let parsed = shader.with(|s| s.parsed.clone())?;
            let found = self.on_device(&parsed)?;
            return self.report(ps, found, &parsed.label);
        }
        let input = ShaderInput {
            source: &self.source,
            path: &self.path,
            literal_path: &self.literal_path,
            spirv: &self.spir_v,
            language: self.language,
            stage: self.stage,
            defines: &self.define,
        };
        for_each_shader(ps, &input, false, |source, label| {
            let checked = parse(&source).and_then(|module| validate(&module, source.text()).map(|info| (module, info)));
            let found = match checked {
                Err(diagnostics) => diagnostics,
                Ok((module, info)) => self.on_device(&Parsed { source, module, info, label: label.clone() })?,
            };
            self.report(ps, found, &label)
        })
    }
}

/// Writes what a shader declares: its entry points with their stages and
/// workgroup sizes, its resource bindings with their kinds and types, and
/// its override constants.
///
/// # Examples
/// Get-WgpuShaderInfo -Source $wgsl
/// (Get-WgpuShaderInfo -Path .\kernel.wgsl).Bindings
/// Get-ChildItem .\shaders -Filter *.wgsl | Get-WgpuShaderInfo
#[cmdlet(verb = "Get", noun = "WgpuShaderInfo", alias = ["Get-GpuShaderInfo"], default_parameter_set = "Source", output = ["Wgpu.ShaderInfo"])]
#[derive(Default)]
pub struct GetWgpuShaderInfo {
    /// The shader text.
    #[param(mandatory, position = 0, value_from_pipeline, set = "Source")]
    pub source: Option<String>,
    /// A shader file: .wgsl, .spv, or GLSL (.comp, .vert, .frag, .glsl); a
    /// wildcard describes every file it matches.
    #[param(mandatory, set = "Path", supports_wildcards)]
    pub path: Option<String>,
    /// A shader file named exactly as written, wildcard characters
    /// included; the files Get-ChildItem pipes bind here.
    #[param(mandatory, set = "LiteralPath", value_from_pipeline_by_property_name, alias = ["PSPath", "LP", "FullName"])]
    pub literal_path: Option<String>,
    /// SPIR-V as byte[] or uint[].
    #[param(mandatory, set = "SpirV", raw)]
    pub spir_v: Option<PsObject>,
    /// A shader made with New-WgpuShader, given in place of text, by
    /// position or from the pipeline.
    #[param(mandatory, position = 0, value_from_pipeline, set = "Shader")]
    pub shader: Option<PsProxy<Shader>>,
    /// The language; inferred from the text or the file extension when omitted.
    #[param]
    pub language: Option<ShaderLanguage>,
    /// The stage of a GLSL shader; Compute when omitted.
    #[param]
    pub stage: Option<ShaderStage>,
    /// Preprocessor definitions for GLSL, as name = value.
    #[param]
    pub define: Option<PsHashtable>,
}

impl Cmdlet for GetWgpuShaderInfo {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        if let Some(shader) = &self.shader {
            let info = shader.with(|s| info_of(&s.parsed))?;
            return ps.write(info);
        }
        let input = ShaderInput {
            source: &self.source,
            path: &self.path,
            literal_path: &self.literal_path,
            spirv: &self.spir_v,
            language: self.language,
            stage: self.stage,
            defines: &self.define,
        };
        for_each_shader(ps, &input, false, |source, label| {
            let parsed = compile(source, label)?;
            ps.write(info_of(&parsed))
        })
    }
}

/// Translates a WGSL, GLSL or SPIR-V shader into WGSL, SPIR-V, GLSL, HLSL,
/// MSL or a Graphviz graph of its intermediate form, with no GPU.
///
/// GLSL output is for one entry point; give -Entry or -Stage when the
/// shader has several. SPIR-V comes out as uint[] words, or as a binary
/// file with -OutFile.
///
/// # Examples
/// ConvertTo-WgpuShader -Source $wgsl -To Hlsl
/// ConvertTo-WgpuShader -Path .\blur.comp -To Wgsl
/// ConvertTo-WgpuShader -Source $wgsl -To SpirV -OutFile kernel.spv
/// ConvertTo-WgpuShader -Source $wgsl -To Glsl -GlslVersion 310 -GlslEs
/// Get-ChildItem .\shaders -Filter *.wgsl | ConvertTo-WgpuShader -To Msl
#[cmdlet(verb = "ConvertTo", noun = "WgpuShader", alias = ["ConvertTo-GpuShader"], default_parameter_set = "Source", output = ["System.String", "System.UInt32[]"])]
#[derive(Default)]
pub struct ConvertToWgpuShader {
    /// The shader text.
    #[param(mandatory, position = 0, value_from_pipeline, set = "Source")]
    pub source: Option<String>,
    /// A shader file: .wgsl, .spv, or GLSL (.comp, .vert, .frag, .glsl); a
    /// wildcard translates every file it matches.
    #[param(mandatory, set = "Path", supports_wildcards)]
    pub path: Option<String>,
    /// A shader file named exactly as written, wildcard characters
    /// included; the files Get-ChildItem pipes bind here.
    #[param(mandatory, set = "LiteralPath", value_from_pipeline_by_property_name, alias = ["PSPath", "LP", "FullName"])]
    pub literal_path: Option<String>,
    /// SPIR-V as byte[] or uint[].
    #[param(mandatory, set = "SpirV", raw)]
    pub spir_v: Option<PsObject>,
    /// A shader made with New-WgpuShader, given in place of text, by
    /// position or from the pipeline.
    #[param(mandatory, position = 0, value_from_pipeline, set = "Shader")]
    pub shader: Option<PsProxy<Shader>>,
    /// The language to write.
    #[param(mandatory, position = 1)]
    pub to: ShaderTarget,
    /// The language of the input; inferred from the text or the file extension when omitted.
    #[param]
    pub language: Option<ShaderLanguage>,
    /// The stage of a GLSL input, and the stage of the entry point to write.
    #[param]
    pub stage: Option<ShaderStage>,
    /// The entry point to write; every entry point when omitted, where the target allows it.
    #[param]
    pub entry: Option<String>,
    /// Preprocessor definitions for GLSL input, as name = value.
    #[param]
    pub define: Option<PsHashtable>,
    /// The GLSL version to write: 450 for desktop when omitted, 310 with -GlslEs.
    #[param]
    pub glsl_version: Option<u16>,
    /// Write OpenGL ES GLSL.
    #[param]
    pub glsl_es: bool,
    /// The HLSL shader model, 5.0 to 6.9; 6.0 when omitted.
    #[param]
    pub shader_model: Option<String>,
    /// The Metal Shading Language version; 2.4 when omitted.
    #[param]
    pub msl_version: Option<String>,
    /// The SPIR-V version, such as 1.5; naga's default when omitted.
    #[param]
    pub spir_v_version: Option<String>,
    /// Write the result to this file instead of the pipeline; SPIR-V as binary.
    #[param]
    pub out_file: Option<String>,
    /// Whether this pipeline has written -OutFile, so a second shader is
    /// refused rather than written over the first.
    wrote_out_file: bool,
}

impl Cmdlet for ConvertToWgpuShader {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let translation = Translation {
            entry: self.entry.clone(),
            stage: self.stage,
            glsl_version: self.glsl_version,
            glsl_es: self.glsl_es,
            hlsl_shader_model: self.shader_model.clone(),
            msl_version: self.msl_version.clone(),
            spirv_version: self.spir_v_version.clone(),
        };
        let mut write = |parsed: Arc<Parsed>| -> PsResult<()> {
            let output = translate(&parsed, self.to, &translation)?;
            let Some(path) = &self.out_file else {
                // A note added to a string or an array goes on a wrapper,
                // so the wrapper is what is written.
                let noted = PsType::from_name("System.Management.Automation.PSObject").call_static("AsPSObject", &[output])?;
                pwrs::object::add_note(&noted, "Label", parsed.label.clone().into_ps()?)?;
                return ps.write_object(&noted);
            };
            let resolved = ps.resolve_path(path, true)?;
            let target = resolved.into_iter().next().ok_or_else(|| invalid("WgpuOutFile", format!("{path} names no file")))?;
            if self.wrote_out_file {
                return Err(invalid("WgpuOutFile", format!("-OutFile holds one shader, and {target} holds one from earlier in this pipeline")));
            }
            let bytes = if self.to == ShaderTarget::SpirV {
                let words = Vec::<u32>::from_ps(&output)?;
                bytemuck::cast_slice::<u32, u8>(&words).to_vec()
            } else {
                String::from_ps(&output)?.into_bytes()
            };
            std::fs::write(&target, bytes).map_err(|e| PsError::new(ErrorCategory::WriteError, "WgpuOutFile", format!("{target} cannot be written: {e}")))?;
            self.wrote_out_file = true;
            Ok(())
        };
        match &self.shader {
            Some(shader) => write(shader.with(|s| s.parsed.clone())?),
            None => {
                let input = ShaderInput {
                    source: &self.source,
                    path: &self.path,
                    literal_path: &self.literal_path,
                    spirv: &self.spir_v,
                    language: self.language,
                    stage: self.stage,
                    defines: &self.define,
                };
                for_each_shader(ps, &input, self.out_file.is_some(), |source, label| write(compile(source, label)?))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPIR-V words for a compute shader over floats, from naga's writer.
    fn float_words() -> Vec<u32> {
        let text = "@group(0) @binding(0) var<storage, read_write> v: array<f32>;\n@compute @workgroup_size(1) fn main() { v[0] = v[0] * 2.0; }";
        let module = naga::front::wgsl::parse_str(text).expect("the WGSL parses");
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module).expect("the WGSL is valid");
        naga::back::spv::write_vec(&module, &info, &naga::back::spv::Options::default(), None).expect("naga writes SPIR-V")
    }

    #[test]
    fn a_panic_in_the_spir_v_reader_is_the_shaders_one_problem() {
        let mut words = float_words();
        let mut at = 5;
        while words[at] & 0xffff != 22 {
            at += (words[at] >> 16) as usize;
        }
        words[at + 2] = 7;
        match parse(&Source::SpirV(words)) {
            Ok(_) => panic!("SPIR-V with a float width of 7 was read"),
            Err(problems) => {
                assert_eq!(problems.len(), 1);
                assert_eq!(problems[0].line, 0);
                assert!(problems[0].message.starts_with("naga panicked while reading the SPIR-V: "), "{}", problems[0].message);
            }
        }
    }

    #[test]
    fn spir_v_with_every_width_a_power_of_two_still_reads() {
        assert!(parse(&Source::SpirV(float_words())).is_ok());
    }
}
