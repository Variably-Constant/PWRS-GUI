//! Render pipelines and render passes: drawing offscreen into textures and
//! image files.

use std::path::Path;
use std::sync::Arc;

use pwrs::prelude::*;

use crate::binding::{self, BindGroup, Bound};
use crate::buffer::{self, Buffer};
use crate::device::{core_for, CoreRef, Device};
use crate::element::{self, ElementType};
use crate::encoder::{CommandEncoder, QuerySet};
use crate::error::{invalid, invalid_operation, not_found};
use crate::image_io;
use crate::pipeline::{self, Group};
use crate::shader::{self, Parsed, Reflected, ShaderBinding};
use crate::texfile::Said;
use crate::texture::{self, CompareFunction, Texture, TextureView};

/// How vertices make primitives.
#[psenum(name = "Wgpu.PrimitiveTopology")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PrimitiveTopology {
    /// Each vertex is a point.
    PointList,
    /// Each pair of vertices is a line.
    LineList,
    /// Each vertex after the first ends a line from the one before.
    LineStrip,
    /// Each three vertices are a triangle.
    #[default]
    TriangleList,
    /// Each vertex after the second ends a triangle with the two before.
    TriangleStrip,
}

/// Which faces a render pipeline discards. A triangle whose corners turn
/// counterclockwise on the target faces the viewer.
#[psenum(name = "Wgpu.CullMode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CullMode {
    /// Draws both faces.
    #[default]
    None,
    /// Discards faces wound counterclockwise, which face the viewer.
    Front,
    /// Discards faces wound clockwise, which face away.
    Back,
}

/// How a fragment's color combines with the color under it.
#[psenum(name = "Wgpu.BlendMode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BlendMode {
    /// The fragment replaces the color.
    #[default]
    Replace,
    /// Alpha blending: the fragment over the color by its alpha.
    Alpha,
    /// The fragment adds to the color.
    Additive,
    /// Alpha blending of a fragment whose color is multiplied by its alpha.
    Premultiplied,
    /// The fragment and the color mixed by the blend constant a draw sets,
    /// by -BlendConstant or SetBlendConstant: the fragment times the
    /// constant plus the color times one minus it.
    Constant,
    /// The factors and operations a custom blend names: -BlendSource,
    /// -BlendDestination and -BlendOperation, and their alpha forms.
    Custom,
}

/// What a blend multiplies the fragment's color (source) or the color
/// under it (destination) by.
#[psenum(name = "Wgpu.BlendFactor")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BlendFactor {
    /// 0.
    Zero,
    /// 1.
    #[default]
    One,
    /// The fragment's color.
    Src,
    /// 1 minus the fragment's color.
    OneMinusSrc,
    /// The fragment's alpha.
    SrcAlpha,
    /// 1 minus the fragment's alpha.
    OneMinusSrcAlpha,
    /// The color under the fragment.
    Dst,
    /// 1 minus the color under the fragment.
    OneMinusDst,
    /// The alpha under the fragment.
    DstAlpha,
    /// 1 minus the alpha under the fragment.
    OneMinusDstAlpha,
    /// The smaller of the fragment's alpha and 1 minus the alpha under it;
    /// 1 for alpha.
    SrcAlphaSaturated,
    /// The blend constant a draw sets, by -BlendConstant or SetBlendConstant.
    Constant,
    /// 1 minus the blend constant.
    OneMinusConstant,
    /// The fragment's second color, `@blend_src(1)`; needs the
    /// DualSourceBlending feature.
    Src1,
    /// 1 minus the fragment's second color.
    OneMinusSrc1,
    /// The fragment's second color's alpha.
    Src1Alpha,
    /// 1 minus the fragment's second color's alpha.
    OneMinusSrc1Alpha,
}

impl BlendFactor {
    fn wgpu(self) -> wgpu::BlendFactor {
        match self {
            BlendFactor::Zero => wgpu::BlendFactor::Zero,
            BlendFactor::One => wgpu::BlendFactor::One,
            BlendFactor::Src => wgpu::BlendFactor::Src,
            BlendFactor::OneMinusSrc => wgpu::BlendFactor::OneMinusSrc,
            BlendFactor::SrcAlpha => wgpu::BlendFactor::SrcAlpha,
            BlendFactor::OneMinusSrcAlpha => wgpu::BlendFactor::OneMinusSrcAlpha,
            BlendFactor::Dst => wgpu::BlendFactor::Dst,
            BlendFactor::OneMinusDst => wgpu::BlendFactor::OneMinusDst,
            BlendFactor::DstAlpha => wgpu::BlendFactor::DstAlpha,
            BlendFactor::OneMinusDstAlpha => wgpu::BlendFactor::OneMinusDstAlpha,
            BlendFactor::SrcAlphaSaturated => wgpu::BlendFactor::SrcAlphaSaturated,
            BlendFactor::Constant => wgpu::BlendFactor::Constant,
            BlendFactor::OneMinusConstant => wgpu::BlendFactor::OneMinusConstant,
            BlendFactor::Src1 => wgpu::BlendFactor::Src1,
            BlendFactor::OneMinusSrc1 => wgpu::BlendFactor::OneMinusSrc1,
            BlendFactor::Src1Alpha => wgpu::BlendFactor::Src1Alpha,
            BlendFactor::OneMinusSrc1Alpha => wgpu::BlendFactor::OneMinusSrc1Alpha,
        }
    }
}

/// How a blend combines the fragment's weighted color with the weighted
/// color under it.
#[psenum(name = "Wgpu.BlendOperation")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BlendOperation {
    /// Source plus destination.
    #[default]
    Add,
    /// Source minus destination.
    Subtract,
    /// Destination minus source.
    ReverseSubtract,
    /// The smaller of source and destination; both factors must be One.
    Min,
    /// The larger of source and destination; both factors must be One.
    Max,
}

impl BlendOperation {
    fn wgpu(self) -> wgpu::BlendOperation {
        match self {
            BlendOperation::Add => wgpu::BlendOperation::Add,
            BlendOperation::Subtract => wgpu::BlendOperation::Subtract,
            BlendOperation::ReverseSubtract => wgpu::BlendOperation::ReverseSubtract,
            BlendOperation::Min => wgpu::BlendOperation::Min,
            BlendOperation::Max => wgpu::BlendOperation::Max,
        }
    }
}

/// The factors and operations of a Custom blend. The color ones are One,
/// Zero and Add when omitted, which replaces the color, and each alpha one
/// is its color one when omitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct CustomBlend {
    /// What the fragment's color is multiplied by.
    pub source: Option<BlendFactor>,
    /// What the color under the fragment is multiplied by.
    pub destination: Option<BlendFactor>,
    /// How the two products combine.
    pub operation: Option<BlendOperation>,
    /// What the fragment's alpha is multiplied by.
    pub alpha_source: Option<BlendFactor>,
    /// What the alpha under the fragment is multiplied by.
    pub alpha_destination: Option<BlendFactor>,
    /// How the two alpha products combine.
    pub alpha_operation: Option<BlendOperation>,
}

impl CustomBlend {
    fn is_empty(&self) -> bool {
        *self == CustomBlend::default()
    }
}

/// The blend state `mode` stands for, with `custom` naming a Custom
/// blend's factors and operations. A factor or operation given with any
/// other mode is refused.
pub(crate) fn blend_state(mode: BlendMode, custom: &CustomBlend) -> PsResult<Option<wgpu::BlendState>> {
    if mode != BlendMode::Custom && !custom.is_empty() {
        return Err(invalid(
            "WgpuBlend",
            format!("-BlendSource, -BlendDestination, -BlendOperation and their alpha forms name a Custom blend, and the blend is {mode:?}: give -Blend Custom"),
        ));
    }
    let both = |source: wgpu::BlendFactor, destination: wgpu::BlendFactor| {
        let component = wgpu::BlendComponent { src_factor: source, dst_factor: destination, operation: wgpu::BlendOperation::Add };
        Some(wgpu::BlendState { color: component, alpha: component })
    };
    Ok(match mode {
        BlendMode::Replace => None,
        BlendMode::Alpha => Some(wgpu::BlendState::ALPHA_BLENDING),
        BlendMode::Premultiplied => Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
        BlendMode::Additive => both(wgpu::BlendFactor::One, wgpu::BlendFactor::One),
        BlendMode::Constant => both(wgpu::BlendFactor::Constant, wgpu::BlendFactor::OneMinusConstant),
        BlendMode::Custom => {
            let source = custom.source.unwrap_or(BlendFactor::One);
            let destination = custom.destination.unwrap_or(BlendFactor::Zero);
            let operation = custom.operation.unwrap_or(BlendOperation::Add);
            let component = |s: BlendFactor, d: BlendFactor, o: BlendOperation| wgpu::BlendComponent { src_factor: s.wgpu(), dst_factor: d.wgpu(), operation: o.wgpu() };
            Some(wgpu::BlendState {
                color: component(source, destination, operation),
                alpha: component(custom.alpha_source.unwrap_or(source), custom.alpha_destination.unwrap_or(destination), custom.alpha_operation.unwrap_or(operation)),
            })
        }
    })
}

/// Whether `blend` reads the blend constant: a Constant or
/// OneMinusConstant factor on its color or its alpha. A draw through a
/// pipeline whose blend reads it must set it.
pub(crate) fn reads_constant(blend: Option<wgpu::BlendState>) -> bool {
    let constant = |f: wgpu::BlendFactor| matches!(f, wgpu::BlendFactor::Constant | wgpu::BlendFactor::OneMinusConstant);
    blend.is_some_and(|b| [b.color, b.alpha].iter().any(|c| constant(c.src_factor) || constant(c.dst_factor)))
}

/// What a stencil test does to the stencil value under a fragment.
#[psenum(name = "Wgpu.StencilOperation")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StencilOperation {
    /// Leaves the value as it is.
    #[default]
    Keep,
    /// Sets the value to 0.
    Zero,
    /// Sets the value to the stencil reference.
    Replace,
    /// Inverts the value's bits.
    Invert,
    /// Adds 1, staying at the largest value.
    IncrementClamp,
    /// Subtracts 1, staying at 0.
    DecrementClamp,
    /// Adds 1, wrapping from the largest value to 0.
    IncrementWrap,
    /// Subtracts 1, wrapping from 0 to the largest value.
    DecrementWrap,
}

impl StencilOperation {
    fn wgpu(self) -> wgpu::StencilOperation {
        match self {
            StencilOperation::Keep => wgpu::StencilOperation::Keep,
            StencilOperation::Zero => wgpu::StencilOperation::Zero,
            StencilOperation::Replace => wgpu::StencilOperation::Replace,
            StencilOperation::Invert => wgpu::StencilOperation::Invert,
            StencilOperation::IncrementClamp => wgpu::StencilOperation::IncrementClamp,
            StencilOperation::DecrementClamp => wgpu::StencilOperation::DecrementClamp,
            StencilOperation::IncrementWrap => wgpu::StencilOperation::IncrementWrap,
            StencilOperation::DecrementWrap => wgpu::StencilOperation::DecrementWrap,
        }
    }
}

/// The stencil test of one face: how the stencil reference compares with
/// the stencil value under a fragment, and what a failed test, a passed
/// test whose depth test fails, and a passed test do to that value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct StencilFace {
    /// The comparison, the reference on its left.
    pub compare: Option<CompareFunction>,
    /// What a failed stencil test does.
    pub fail: Option<StencilOperation>,
    /// What a passed stencil test whose depth test fails does.
    pub depth_fail: Option<StencilOperation>,
    /// What passing both tests does.
    pub pass: Option<StencilOperation>,
}

/// A stencil test. Each part of the front face is Always or Keep when
/// `None`, each part of the back face is the front face's when `None`, and
/// each mask is every bit when `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct StencilTest {
    /// The test of triangles facing the viewer, and of points and lines.
    pub front: StencilFace,
    /// The test of triangles facing away.
    pub back: StencilFace,
    /// The bits of the reference and of the value the comparison reads.
    pub read_mask: Option<u32>,
    /// The bits of the value the operations change.
    pub write_mask: Option<u32>,
}

/// A depth test: how a fragment's depth compares with the depth under it,
/// Less when `None`; whether a fragment that passes writes its depth, as it
/// does when `None`; and the bias added to each fragment's depth, none when
/// `None`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct DepthTest {
    /// The comparison, the fragment's depth on its left.
    pub compare: Option<CompareFunction>,
    /// Whether a fragment that passes writes its depth.
    pub write: Option<bool>,
    /// A constant bias, in the depth format's smallest steps.
    pub bias: Option<i32>,
    /// A bias scaled by the triangle's depth slope.
    pub bias_slope_scale: Option<f32>,
    /// The largest bias the two give together, the most negative below 0,
    /// and no limit at 0.
    pub bias_clamp: Option<f32>,
}

/// The depth and stencil formats, as -DepthFormat names them.
pub(crate) const DEPTH_FORMATS: [wgpu::TextureFormat; 6] = [
    wgpu::TextureFormat::Stencil8,
    wgpu::TextureFormat::Depth16Unorm,
    wgpu::TextureFormat::Depth24Plus,
    wgpu::TextureFormat::Depth24PlusStencil8,
    wgpu::TextureFormat::Depth32Float,
    wgpu::TextureFormat::Depth32FloatStencil8,
];

/// The depth target a pipeline draws with: the format `name` gives, which
/// must be a depth or stencil format the device can hold, or Depth32Float
/// when only `depth` is set, or none.
pub(crate) fn depth_target(core: &CoreRef, depth: bool, name: Option<&str>) -> PsResult<Option<wgpu::TextureFormat>> {
    let Some(name) = name else {
        return Ok(depth.then_some(wgpu::TextureFormat::Depth32Float));
    };
    let format = texture::parse_format(name)?;
    if !DEPTH_FORMATS.contains(&format) {
        let names: Vec<String> = DEPTH_FORMATS.iter().map(|f| texture::format_name(*f)).collect();
        return Err(invalid("WgpuDepthFormat", format!("-DepthFormat names a depth or stencil format, one of {}, and {name} is not one", names.join(", "))));
    }
    image_io::holdable(core, format)?;
    Ok(Some(format))
}

/// The depth and stencil state of a pipeline drawing `topology` with a
/// depth target of `format`, `None` with none. A depth test needs a format
/// with a depth aspect and a stencil test one with a stencil aspect, and a
/// depth bias moves only triangles.
pub(crate) fn depth_stencil_state(
    format: Option<wgpu::TextureFormat>,
    depth: &DepthTest,
    stencil: &StencilTest,
    topology: PrimitiveTopology,
) -> PsResult<Option<wgpu::DepthStencilState>> {
    let has_depth = format.is_some_and(|f| f.has_depth_aspect());
    if *depth != DepthTest::default() && !has_depth {
        return Err(invalid("WgpuDepthTest", format!("-DepthCompare, -DepthWrite and the depth bias test depth, and {}: give -Depth or a -DepthFormat with depth", lacking(format, "depth"))));
    }
    if *stencil != StencilTest::default() && !format.is_some_and(|f| f.has_stencil_aspect()) {
        return Err(invalid(
            "WgpuStencil",
            format!("a stencil test needs a depth target with a stencil aspect, and {}: give -DepthFormat Stencil8, Depth24PlusStencil8 or Depth32FloatStencil8", lacking(format, "stencil")),
        ));
    }
    let bias = wgpu::DepthBiasState { constant: depth.bias.unwrap_or(0), slope_scale: depth.bias_slope_scale.unwrap_or(0.0), clamp: depth.bias_clamp.unwrap_or(0.0) };
    let triangles = matches!(topology, PrimitiveTopology::TriangleList | PrimitiveTopology::TriangleStrip);
    if (bias.is_enabled() || bias.clamp != 0.0) && !triangles {
        return Err(invalid("WgpuDepthBias", format!("a depth bias moves triangles, and the topology is {topology:?}")));
    }
    let Some(format) = format else {
        return Ok(None);
    };
    let front = &stencil.front;
    let keep = StencilOperation::Keep;
    let face = |own: &StencilFace, under: &StencilFace| wgpu::StencilFaceState {
        compare: own.compare.or(under.compare).unwrap_or(CompareFunction::Always).wgpu(),
        fail_op: own.fail.or(under.fail).unwrap_or(keep).wgpu(),
        depth_fail_op: own.depth_fail.or(under.depth_fail).unwrap_or(keep).wgpu(),
        pass_op: own.pass.or(under.pass).unwrap_or(keep).wgpu(),
    };
    Ok(Some(wgpu::DepthStencilState {
        format,
        depth_write_enabled: has_depth.then(|| depth.write.unwrap_or(true)),
        depth_compare: has_depth.then(|| depth.compare.unwrap_or(CompareFunction::Less).wgpu()),
        stencil: wgpu::StencilState {
            front: face(front, &StencilFace::default()),
            back: face(&stencil.back, front),
            read_mask: stencil.read_mask.unwrap_or(u32::MAX),
            write_mask: stencil.write_mask.unwrap_or(u32::MAX),
        },
        bias,
    }))
}

/// Why a depth target of `format`, `None` for none, lacks `aspect`.
fn lacking(format: Option<wgpu::TextureFormat>, aspect: &str) -> String {
    match format {
        Some(f) => format!("the depth target, {}, has no {aspect} aspect", texture::format_name(f)),
        None => String::from("there is no depth target"),
    }
}

/// What a draw's own depth target of `format` starts at: `depth`, 1 when
/// `None`, and `stencil`, 0 when `None`, which `names` name in an error. A
/// value for an aspect the target lacks, and a depth outside 0 to 1, are
/// refused.
pub(crate) fn target_clears(format: Option<wgpu::TextureFormat>, depth: Option<f64>, stencil: Option<u32>, names: [&str; 2]) -> PsResult<(f32, u32)> {
    let [depth_name, stencil_name] = names;
    if let Some(value) = depth {
        if !format.is_some_and(|f| f.has_depth_aspect()) {
            return Err(invalid("WgpuClearDepth", format!("{depth_name} sets the depth the depth target starts at, and {}", lacking(format, "depth"))));
        }
        if !(0.0..=1.0).contains(&value) {
            return Err(invalid("WgpuClearDepth", format!("{depth_name} is a depth from 0 to 1, and {value} is not")));
        }
    }
    if stencil.is_some() && !format.is_some_and(|f| f.has_stencil_aspect()) {
        return Err(invalid("WgpuClearStencil", format!("{stencil_name} sets the value the stencil starts at, and {}", lacking(format, "stencil"))));
    }
    Ok((depth.unwrap_or(1.0) as f32, stencil.unwrap_or(0)))
}

/// One `@location` input of a vertex shader, fed from a buffer of its own.
#[derive(Clone)]
pub(crate) struct VertexInput {
    pub(crate) location: u32,
    pub(crate) name: String,
    pub(crate) format: wgpu::VertexFormat,
    pub(crate) element: ElementType,
    pub(crate) components: u32,
}

fn vertex_format(scalar: naga::Scalar, components: u32) -> Option<(wgpu::VertexFormat, ElementType)> {
    use naga::ScalarKind as K;
    use wgpu::VertexFormat as V;
    Some(match (scalar.kind, scalar.width, components) {
        (K::Float, 4, 1) => (V::Float32, ElementType::Float32),
        (K::Float, 4, 2) => (V::Float32x2, ElementType::Float32),
        (K::Float, 4, 3) => (V::Float32x3, ElementType::Float32),
        (K::Float, 4, 4) => (V::Float32x4, ElementType::Float32),
        (K::Float, 8, 1) => (V::Float64, ElementType::Float64),
        (K::Float, 8, 2) => (V::Float64x2, ElementType::Float64),
        (K::Float, 8, 3) => (V::Float64x3, ElementType::Float64),
        (K::Float, 8, 4) => (V::Float64x4, ElementType::Float64),
        (K::Float, 2, 2) => (V::Float16x2, ElementType::Float16),
        (K::Float, 2, 4) => (V::Float16x4, ElementType::Float16),
        (K::Uint, 4, 1) => (V::Uint32, ElementType::UInt32),
        (K::Uint, 4, 2) => (V::Uint32x2, ElementType::UInt32),
        (K::Uint, 4, 3) => (V::Uint32x3, ElementType::UInt32),
        (K::Uint, 4, 4) => (V::Uint32x4, ElementType::UInt32),
        (K::Sint, 4, 1) => (V::Sint32, ElementType::Int32),
        (K::Sint, 4, 2) => (V::Sint32x2, ElementType::Int32),
        (K::Sint, 4, 3) => (V::Sint32x3, ElementType::Int32),
        (K::Sint, 4, 4) => (V::Sint32x4, ElementType::Int32),
        _ => return None,
    })
}

/// The `@location` inputs of a vertex entry point, in location order.
pub(crate) fn vertex_inputs(module: &naga::Module, entry: usize) -> PsResult<Vec<VertexInput>> {
    let function = &module.entry_points[entry].function;
    let mut inputs = Vec::new();
    let mut take = |name: String, ty: naga::Handle<naga::Type>, binding: &Option<naga::Binding>| -> PsResult<()> {
        let Some(naga::Binding::Location { location, .. }) = binding else { return Ok(()) };
        let (scalar, components) = match &module.types[ty].inner {
            naga::TypeInner::Scalar(s) => (*s, 1),
            naga::TypeInner::Vector { size, scalar } => (*scalar, *size as u32),
            other => return Err(invalid("WgpuVertexInput", format!("vertex input {name} is {other:?}, which no vertex format holds"))),
        };
        let (format, element) = vertex_format(scalar, components).ok_or_else(|| invalid("WgpuVertexInput", format!("vertex input {name} has no vertex format")))?;
        inputs.push(VertexInput { location: *location, name, format, element, components });
        Ok(())
    };
    for (i, argument) in function.arguments.iter().enumerate() {
        let arg_name = match &argument.name {
            Some(n) => n.clone(),
            None => format!("input{i}"),
        };
        match &module.types[argument.ty].inner {
            naga::TypeInner::Struct { members, .. } if argument.binding.is_none() => {
                for (j, member) in members.iter().enumerate() {
                    let member_name = match &member.name {
                        Some(n) => n.clone(),
                        None => format!("{arg_name}{j}"),
                    };
                    take(member_name, member.ty, &member.binding)?;
                }
            }
            _ => take(arg_name, argument.ty, &argument.binding)?,
        }
    }
    inputs.sort_by_key(|i| i.location);
    Ok(inputs)
}

/// What a render pipeline is built with.
#[derive(Clone)]
pub(crate) struct RenderSpec {
    pub(crate) vertex_entry: Option<String>,
    pub(crate) fragment_entry: Option<String>,
    pub(crate) formats: Vec<wgpu::TextureFormat>,
    pub(crate) depth_stencil: Option<wgpu::DepthStencilState>,
    pub(crate) topology: PrimitiveTopology,
    pub(crate) cull: CullMode,
    pub(crate) blend: Option<wgpu::BlendState>,
    pub(crate) sample_count: u32,
    pub(crate) constants: Vec<(String, f64)>,
}

impl RenderSpec {
    /// The format of the depth target, or `None` without one.
    pub(crate) fn depth_format(&self) -> Option<wgpu::TextureFormat> {
        self.depth_stencil.as_ref().map(|d| d.format)
    }

    /// Whether a stencil test reads or changes the stencil.
    pub(crate) fn tests_stencil(&self) -> bool {
        self.depth_stencil.as_ref().is_some_and(|d| d.stencil.is_enabled())
    }
}

/// A render pipeline and what it was built from.
pub(crate) struct RenderInner {
    pub(crate) core: CoreRef,
    pub(crate) pipeline: wgpu::RenderPipeline,
    pub(crate) groups: Vec<Group>,
    pub(crate) bindings: Vec<Reflected>,
    pub(crate) inputs: Vec<VertexInput>,
    pub(crate) spec: RenderSpec,
    pub(crate) vertex_entry: String,
    pub(crate) fragment_entry: String,
    pub(crate) label: String,
}

/// The bindings a vertex and a fragment entry point use between them, in
/// group and binding order, each once.
pub(crate) fn render_bindings(parsed: &Parsed, vertex: usize, fragment: usize) -> Vec<Reflected> {
    let mut bindings = shader::reflect(&parsed.module, &parsed.info, Some(vertex));
    for b in shader::reflect(&parsed.module, &parsed.info, Some(fragment)) {
        if !bindings.iter().any(|x| x.group == b.group && x.binding == b.binding) {
            bindings.push(b);
        }
    }
    bindings.sort_by_key(|b| (b.group, b.binding));
    bindings
}

pub(crate) fn build_render(core: &CoreRef, parsed: &Arc<Parsed>, spec: RenderSpec, label: Option<String>) -> PsResult<Arc<RenderInner>> {
    let module = &parsed.module;
    let vertex = shader::entry_index(module, spec.vertex_entry.as_deref(), naga::ShaderStage::Vertex)?;
    let fragment = shader::entry_index(module, spec.fragment_entry.as_deref(), naga::ShaderStage::Fragment)?;
    let vertex_entry = module.entry_points[vertex].name.clone();
    let fragment_entry = module.entry_points[fragment].name.clone();
    let label = match label {
        Some(l) => l,
        None => format!("{} {vertex_entry}/{fragment_entry}", parsed.label),
    };
    let bindings = render_bindings(parsed, vertex, fragment);
    let inputs = vertex_inputs(module, vertex)?;
    let groups = pipeline::layouts(core, &bindings, wgpu::ShaderStages::VERTEX_FRAGMENT, &label)?;
    let shader_module = shader::module_on(core, parsed)?;
    let constants: Vec<(&str, f64)> = spec.constants.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    let attributes: Vec<[wgpu::VertexAttribute; 1]> = inputs.iter().map(|i| [wgpu::VertexAttribute { format: i.format, offset: 0, shader_location: i.location }]).collect();
    let buffers: Vec<Option<wgpu::VertexBufferLayout>> = inputs
        .iter()
        .zip(&attributes)
        .map(|(i, a)| Some(wgpu::VertexBufferLayout { array_stride: i.format.size(), step_mode: wgpu::VertexStepMode::Vertex, attributes: a }))
        .collect();
    let targets: Vec<Option<wgpu::ColorTargetState>> = spec
        .formats
        .iter()
        .map(|f| Some(wgpu::ColorTargetState { format: *f, blend: spec.blend, write_mask: wgpu::ColorWrites::ALL }))
        .collect();
    let topology = match spec.topology {
        PrimitiveTopology::PointList => wgpu::PrimitiveTopology::PointList,
        PrimitiveTopology::LineList => wgpu::PrimitiveTopology::LineList,
        PrimitiveTopology::LineStrip => wgpu::PrimitiveTopology::LineStrip,
        PrimitiveTopology::TriangleList => wgpu::PrimitiveTopology::TriangleList,
        PrimitiveTopology::TriangleStrip => wgpu::PrimitiveTopology::TriangleStrip,
    };
    let cull_mode = match spec.cull {
        CullMode::None => None,
        CullMode::Front => Some(wgpu::Face::Front),
        CullMode::Back => Some(wgpu::Face::Back),
    };
    let pipeline = core.scoped("WgpuRenderPipeline", "the render pipeline cannot be built", || {
        let group_layouts: Vec<Option<&wgpu::BindGroupLayout>> = groups.iter().map(|g| Some(&g.layout)).collect();
        let layout = core.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(&label), bind_group_layouts: &group_layouts, immediate_size: 0 });
        core.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader_module,
                entry_point: Some(&vertex_entry),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, zero_initialize_workgroup_memory: true },
                buffers: &buffers,
            },
            primitive: wgpu::PrimitiveState { topology, cull_mode, ..Default::default() },
            depth_stencil: spec.depth_stencil.clone(),
            multisample: wgpu::MultisampleState { count: spec.sample_count, mask: !0, alpha_to_coverage_enabled: false },
            fragment: Some(wgpu::FragmentState {
                module: &shader_module,
                entry_point: Some(&fragment_entry),
                compilation_options: wgpu::PipelineCompilationOptions { constants: &constants, zero_initialize_workgroup_memory: true },
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        })
    })?;
    Ok(Arc::new(RenderInner { core: core.clone(), pipeline, groups, bindings, inputs, spec, vertex_entry, fragment_entry, label }))
}

/// A render pipeline, built for one device: a vertex and a fragment
/// entry point with their vertex inputs, bindings and target formats.
#[psclass(name = "Wgpu.RenderPipeline", mode = proxy, columns = ["Label", "VertexEntryPoint", "FragmentEntryPoint", "TargetFormats"])]
pub struct RenderPipeline {
    /// The pipeline's label.
    pub label: String,
    /// The vertex entry point.
    pub vertex_entry_point: String,
    /// The fragment entry point.
    pub fragment_entry_point: String,
    /// The vertex inputs, in location order, each fed from a buffer of its own.
    pub vertex_inputs: Vec<String>,
    /// The bindings both stages use, in group and binding order.
    pub bindings: Vec<ShaderBinding>,
    /// The formats of its color targets.
    pub target_formats: Vec<String>,
    /// The format of its depth target, or `$null`.
    pub depth_format: Option<String>,
    /// Samples per pixel.
    pub sample_count: u32,
    /// The adapter of the device it was built on.
    pub adapter: String,
    #[psfield(skip)]
    pub(crate) inner: Arc<RenderInner>,
}

impl RenderPipeline {
    pub(crate) fn from_inner(inner: Arc<RenderInner>) -> RenderPipeline {
        RenderPipeline {
            label: inner.label.clone(),
            vertex_entry_point: inner.vertex_entry.clone(),
            fragment_entry_point: inner.fragment_entry.clone(),
            vertex_inputs: inner.inputs.iter().map(|i| i.name.clone()).collect(),
            bindings: inner.bindings.iter().map(Reflected::describe).collect(),
            target_formats: inner.spec.formats.iter().map(|f| texture::format_name(*f)).collect(),
            depth_format: inner.spec.depth_format().map(texture::format_name),
            sample_count: inner.spec.sample_count,
            adapter: inner.core.info.name.clone(),
            inner,
        }
    }
}

#[psmethods]
impl RenderPipeline {
    /// The bind group `group` of this pipeline, binding `resources`.
    pub fn bind_group(&self, group: u32, resources: PsObject) -> PsResult<BindGroup> {
        binding::bind_group_from(&self.inner.core, &self.inner.groups, group, &resources, &self.inner.label)
    }
}

/// The vertex buffers of a draw, from arrays or buffers by input name or
/// in location order; and the vertex count the first array gives.
fn vertex_buffers(core: &CoreRef, inputs: &[VertexInput], vertices: Option<&PsObject>) -> PsResult<(Vec<wgpu::Buffer>, Option<u32>)> {
    let Some(vertices) = vertices else {
        if inputs.is_empty() {
            return Ok((Vec::new(), None));
        }
        return Err(invalid("WgpuNoVertices", format!("the vertex shader reads {}; give them with -Vertex", inputs.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", "))));
    };
    let type_name = vertices.type_name()?;
    let values: Vec<PsObject> = if type_name == "System.Collections.Hashtable" || type_name == "System.Collections.Specialized.OrderedDictionary" {
        let table = PsHashtable(vertices.clone());
        let mut out = Vec::with_capacity(inputs.len());
        for input in inputs {
            let key = table.keys()?.into_iter().find(|k| k.eq_ignore_ascii_case(&input.name)).ok_or_else(|| not_found("WgpuNoVertices", format!("-Vertex gives no values for {}", input.name)))?;
            out.push(table.get(&key)?);
        }
        out
    } else if inputs.len() == 1 {
        vec![vertices.clone()]
    } else {
        Vec::<PsObject>::from_ps(vertices)?
    };
    if values.len() != inputs.len() {
        return Err(invalid("WgpuVertexCount", format!("the vertex shader reads {} inputs and {} were given", inputs.len(), values.len())));
    }
    let mut count = None;
    let mut buffers = Vec::with_capacity(inputs.len());
    for (input, value) in inputs.iter().zip(&values) {
        if value.type_name()? == "Wgpu.Buffer" {
            let (raw, length) = PsProxy::<Buffer>::from_ps(value)?.with(|b| (b.buffer.clone(), b.length))?;
            count.get_or_insert((length / input.components as u64) as u32);
            buffers.push(raw);
            continue;
        }
        let raw = element::with_host_bytes(value, Some(input.element), |ty, bytes| {
            count.get_or_insert((bytes.len() as u64 / ty.size() / input.components as u64) as u32);
            buffer::create_raw(core, bytes.len() as u64, wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, &input.name, Some(bytes))
        })?;
        buffers.push(raw);
    }
    Ok((buffers, count))
}

/// What one draw sets on a render pass: its bind groups, one vertex buffer
/// per vertex input in location order, its indices, the vertices and
/// instances it draws, the color a blend factor of Constant reads, and the
/// value a stencil test compares with, each left as the pass has it when
/// `None`.
pub(crate) struct DrawCall<'a> {
    pub(crate) groups: &'a [(u32, wgpu::BindGroup)],
    pub(crate) vertex_buffers: &'a [wgpu::Buffer],
    pub(crate) index: Option<(&'a wgpu::Buffer, wgpu::IndexFormat, u32)>,
    pub(crate) vertex_count: u32,
    pub(crate) instances: u32,
    pub(crate) blend_constant: Option<wgpu::Color>,
    pub(crate) stencil_reference: Option<u32>,
}

/// Sets `pipeline` and what `call` reads on `pass`, and draws.
pub(crate) fn draw_on(pass: &mut wgpu::RenderPass<'_>, pipeline: &wgpu::RenderPipeline, call: &DrawCall<'_>) {
    pass.set_pipeline(pipeline);
    if let Some(constant) = call.blend_constant {
        pass.set_blend_constant(constant);
    }
    if let Some(reference) = call.stencil_reference {
        pass.set_stencil_reference(reference);
    }
    for (index, group) in call.groups {
        pass.set_bind_group(*index, group, &[]);
    }
    for (slot, buffer) in call.vertex_buffers.iter().enumerate() {
        pass.set_vertex_buffer(slot as u32, buffer.slice(..));
    }
    match call.index {
        Some((raw, format, count)) => {
            pass.set_index_buffer(raw.slice(..), format);
            pass.draw_indexed(0..count, 0, 0..call.instances);
        }
        None => pass.draw(0..call.vertex_count, 0..call.instances),
    }
}

/// A size-matched attachment for `target`, of the pipeline's sample count.
fn attachment(core: &CoreRef, target: &wgpu::Texture, format: wgpu::TextureFormat, samples: u32, what: &str) -> PsResult<wgpu::Texture> {
    core.scoped("WgpuTexture", &format!("the {what} target cannot be created"), || {
        core.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(what),
            size: wgpu::Extent3d { width: target.width(), height: target.height(), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
    })
}

/// The operations of a depth target's depth or stencil aspect: `load`, then
/// a store, for an aspect the format has, and `None` for one it lacks.
fn aspect_ops<V>(has: bool, load: wgpu::LoadOp<V>) -> Option<wgpu::Operations<V>> {
    has.then_some(wgpu::Operations { load, store: wgpu::StoreOp::Store })
}

/// Records one draw of `inner` into `target`, which holds what `load`
/// says before it. A pipeline built with more than one sample draws into a
/// multisampled target resolved into `target`, and one built with a depth
/// target tests against one of its format, its depth starting at
/// `depth_clear` and its stencil at `stencil_clear`.
pub(crate) fn record_draw(
    inner: &RenderInner,
    target: &wgpu::Texture,
    call: &DrawCall<'_>,
    load: wgpu::LoadOp<wgpu::Color>,
    depth_clear: f32,
    stencil_clear: u32,
) -> PsResult<wgpu::CommandBuffer> {
    let core = &inner.core;
    let samples = inner.spec.sample_count;
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let multisampled = if samples > 1 { Some(attachment(core, target, target.format(), samples, "multisampled")?) } else { None };
    let depth = match inner.spec.depth_format() {
        Some(format) => Some(attachment(core, target, format, samples, "depth")?),
        None => None,
    };
    core.scoped("WgpuDraw", "the draw is invalid", || {
        let ms_view = multisampled.as_ref().map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()));
        let depth_view = depth.as_ref().map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()));
        let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(&inner.label) });
        {
            let (color, resolve) = match &ms_view {
                Some(ms) => (ms, Some(&view)),
                None => (&view, None),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&inner.label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    depth_slice: None,
                    resolve_target: resolve,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: depth_view.as_ref().zip(inner.spec.depth_format()).map(|(v, format)| wgpu::RenderPassDepthStencilAttachment {
                    view: v,
                    depth_ops: aspect_ops(format.has_depth_aspect(), wgpu::LoadOp::Clear(depth_clear)),
                    stencil_ops: aspect_ops(format.has_stencil_aspect(), wgpu::LoadOp::Clear(stencil_clear)),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            draw_on(&mut pass, &inner.pipeline, call);
        }
        encoder.finish()
    })
}

fn color_of(values: &Option<Vec<f64>>) -> PsResult<wgpu::Color> {
    match values.as_deref() {
        None => Ok(wgpu::Color::BLACK),
        Some([r, g, b]) => Ok(wgpu::Color { r: *r, g: *g, b: *b, a: 1.0 }),
        Some([r, g, b, a]) => Ok(wgpu::Color { r: *r, g: *g, b: *b, a: *a }),
        Some(other) => Err(invalid("WgpuClearColor", format!("a clear color is three or four numbers, red, green, blue and alpha; {} were given", other.len()))),
    }
}

/// The color -BlendConstant names: four numbers, red, green, blue and
/// alpha.
fn constant_of(values: &Option<Vec<f64>>) -> PsResult<Option<wgpu::Color>> {
    match values.as_deref() {
        None => Ok(None),
        Some([r, g, b, a]) => Ok(Some(wgpu::Color { r: *r, g: *g, b: *b, a: *a })),
        Some(other) => Err(invalid("WgpuBlendConstant", format!("a blend constant is four numbers, red, green, blue and alpha; {} were given", other.len()))),
    }
}

/// Draws offscreen with a vertex and a fragment shader into a texture, or
/// into an image file.
///
/// The shader is WGSL text or a Wgpu.Shader holding a vertex and a
/// fragment entry point, or a pipeline from New-WgpuRenderPipeline; GLSL
/// holds one stage, so it cannot draw. Each `@location` input of the
/// vertex shader reads an array or buffer of its own from -Vertex, by
/// name or in location order; a shader with no inputs draws -VertexCount
/// vertices from vertex_index alone. -Binding gives the resources both
/// stages use.
///
/// # Examples
/// Invoke-WgpuRender $wgsl -Width 256 -Height 256 -VertexCount 3 -OutFile .\triangle.png
/// Invoke-WgpuRender $wgsl 512 512 -Vertex @{ position = $xyz; color = $rgb } -Depth -SampleCount 4
/// $tex = Invoke-WgpuRender $pipeline -Target $tex -Vertex $positions -Index ([uint32[]](0, 1, 2, 2, 1, 3))
#[cmdlet(verb = "Invoke", noun = "WgpuRender", alias = ["Invoke-GpuRender"], output = ["Wgpu.Texture"])]
#[derive(Default)]
pub struct InvokeWgpuRender {
    /// WGSL text or a Wgpu.Shader with a vertex and a fragment entry point, or a Wgpu.RenderPipeline.
    #[param(mandatory, position = 0, raw)]
    pub shader: Option<PsObject>,
    /// The width of the texture drawn into, when -Target is not given.
    #[param(position = 1)]
    pub width: Option<u32>,
    /// The height of the texture drawn into, when -Target is not given.
    #[param(position = 2)]
    pub height: Option<u32>,
    /// A texture to draw into instead of a new one.
    #[param]
    pub target: Option<PsProxy<Texture>>,
    /// The format of a new target texture; Rgba8Unorm when omitted.
    #[param]
    pub format: Option<String>,
    /// The vertex inputs: a hashtable by input name, or arrays or buffers in location order.
    #[param(raw)]
    pub vertex: Option<PsObject>,
    /// Vertices to draw; from the first vertex input when omitted.
    #[param]
    pub vertex_count: Option<u32>,
    /// Instances to draw; 1 when omitted.
    #[param]
    pub instance_count: Option<u32>,
    /// Indices to draw by: uint[] or ushort[], or a Wgpu.Buffer of either.
    #[param(raw)]
    pub index: Option<PsObject>,
    /// The resources for the shaders' bindings.
    #[param(raw)]
    pub binding: Option<PsObject>,
    /// The color the target is cleared to, as red, green, blue and alpha; opaque black when omitted.
    #[param]
    pub clear_color: Option<Vec<f64>>,
    /// How vertices make primitives; TriangleList when omitted.
    #[param]
    pub topology: Option<PrimitiveTopology>,
    /// Which faces to discard; none when omitted.
    #[param]
    pub cull_mode: Option<CullMode>,
    /// How fragments combine with the target; Replace when omitted. Constant
    /// mixes by -BlendConstant, and Custom takes -BlendSource,
    /// -BlendDestination and -BlendOperation.
    #[param]
    pub blend: Option<BlendMode>,
    /// With -Blend Custom, what the fragment's color is multiplied by; One when omitted.
    #[param]
    pub blend_source: Option<BlendFactor>,
    /// With -Blend Custom, what the color under the fragment is multiplied by; Zero when omitted.
    #[param]
    pub blend_destination: Option<BlendFactor>,
    /// With -Blend Custom, how the two products combine; Add when omitted.
    #[param]
    pub blend_operation: Option<BlendOperation>,
    /// With -Blend Custom, what the fragment's alpha is multiplied by; -BlendSource when omitted.
    #[param]
    pub alpha_blend_source: Option<BlendFactor>,
    /// With -Blend Custom, what the alpha under the fragment is multiplied by; -BlendDestination when omitted.
    #[param]
    pub alpha_blend_destination: Option<BlendFactor>,
    /// With -Blend Custom, how the two alpha products combine; -BlendOperation when omitted.
    #[param]
    pub alpha_blend_operation: Option<BlendOperation>,
    /// The color a blend factor of Constant reads, as red, green, blue and alpha; zero when omitted.
    #[param]
    pub blend_constant: Option<Vec<f64>>,
    /// Test and write depth in a Depth32Float target; a pipeline from New-WgpuRenderPipeline draws as it was built.
    #[param]
    pub depth: bool,
    /// Test depth, stencil or both in a target of this format: Stencil8, Depth16Unorm, Depth24Plus, Depth24PlusStencil8, Depth32Float or Depth32FloatStencil8.
    #[param]
    pub depth_format: Option<String>,
    /// How a fragment's depth compares with the depth under it, for the fragment to be drawn; Less when omitted.
    #[param]
    pub depth_compare: Option<CompareFunction>,
    /// Whether a fragment that passes the depth test writes its depth; it does unless -DepthWrite:$false.
    #[param]
    pub depth_write: Option<bool>,
    /// A constant added to each triangle's depth, in the depth format's smallest steps; 0 when omitted.
    #[param]
    pub depth_bias: Option<i32>,
    /// A bias this many times the triangle's depth slope; 0 when omitted.
    #[param]
    pub depth_bias_slope_scale: Option<f64>,
    /// The largest bias the two give together, or the most negative when below 0; no limit when omitted.
    #[param]
    pub depth_bias_clamp: Option<f64>,
    /// How the stencil test compares -StencilReference with the stencil value under a fragment, the reference on the left; Always when omitted.
    #[param]
    pub stencil_compare: Option<CompareFunction>,
    /// What a failed stencil test does to the stencil value; Keep when omitted.
    #[param]
    pub stencil_fail: Option<StencilOperation>,
    /// What a passed stencil test whose depth test fails does to the stencil value; Keep when omitted.
    #[param]
    pub stencil_depth_fail: Option<StencilOperation>,
    /// What passing the stencil test and the depth test does to the stencil value; Keep when omitted.
    #[param]
    pub stencil_pass: Option<StencilOperation>,
    /// The bits of the reference and of the stencil value the stencil test compares; all of them when omitted.
    #[param]
    pub stencil_read_mask: Option<u32>,
    /// The bits of the stencil value the stencil operations change; all of them when omitted.
    #[param]
    pub stencil_write_mask: Option<u32>,
    /// -StencilCompare for triangles facing away; -StencilCompare's when omitted.
    #[param]
    pub back_stencil_compare: Option<CompareFunction>,
    /// -StencilFail for triangles facing away; -StencilFail's when omitted.
    #[param]
    pub back_stencil_fail: Option<StencilOperation>,
    /// -StencilDepthFail for triangles facing away; -StencilDepthFail's when omitted.
    #[param]
    pub back_stencil_depth_fail: Option<StencilOperation>,
    /// -StencilPass for triangles facing away; -StencilPass's when omitted.
    #[param]
    pub back_stencil_pass: Option<StencilOperation>,
    /// The value the stencil test compares with and Replace writes; 0 when omitted. A pipeline with no stencil test refuses it.
    #[param]
    pub stencil_reference: Option<u32>,
    /// The depth the depth target starts at, from 0 to 1; 1 when omitted.
    #[param]
    pub clear_depth: Option<f64>,
    /// The value the stencil starts at; 0 when omitted.
    #[param]
    pub clear_stencil: Option<u32>,
    /// Samples per pixel, resolved into the target; 1 when omitted. A pipeline from New-WgpuRenderPipeline draws with its own.
    #[param]
    pub sample_count: Option<u32>,
    /// The vertex entry point; the shader's only one when omitted.
    #[param]
    pub vertex_entry: Option<String>,
    /// The fragment entry point; the shader's only one when omitted.
    #[param]
    pub fragment_entry: Option<String>,
    /// Override constant values, by name or `@id`.
    #[param]
    pub constant: Option<PsHashtable>,
    /// Save the result as an image file.
    #[param]
    pub out_file: Option<String>,
    /// Write the texture even when -OutFile is given.
    #[param]
    pub pass_thru: bool,
    /// The device to draw on; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
}

impl InvokeWgpuRender {
    /// The depth test the parameters name.
    fn depth_test(&self) -> DepthTest {
        DepthTest {
            compare: self.depth_compare,
            write: self.depth_write,
            bias: self.depth_bias,
            bias_slope_scale: self.depth_bias_slope_scale.map(|s| s as f32),
            bias_clamp: self.depth_bias_clamp.map(|c| c as f32),
        }
    }

    /// The stencil test the parameters name.
    fn stencil_test(&self) -> StencilTest {
        StencilTest {
            front: StencilFace { compare: self.stencil_compare, fail: self.stencil_fail, depth_fail: self.stencil_depth_fail, pass: self.stencil_pass },
            back: StencilFace { compare: self.back_stencil_compare, fail: self.back_stencil_fail, depth_fail: self.back_stencil_depth_fail, pass: self.back_stencil_pass },
            read_mask: self.stencil_read_mask,
            write_mask: self.stencil_write_mask,
        }
    }
}

impl Cmdlet for InvokeWgpuRender {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(shader) = &self.shader else {
            return Err(invalid("WgpuNoShader", "no shader was given"));
        };
        let core = core_for(&self.device, ps)?;
        let target = match &self.target {
            Some(t) => t.with(|t| Texture::wrap(&t.core, t.texture.clone(), t.label.clone()))?,
            None => {
                let (Some(width), Some(height)) = (self.width, self.height) else {
                    return Err(invalid("WgpuNoTarget", "give -Width and -Height, or a -Target texture"));
                };
                let format = match &self.format {
                    Some(name) => texture::parse_format(name)?,
                    None => wgpu::TextureFormat::Rgba8Unorm,
                };
                let spec = texture::TextureSpec {
                    width,
                    height,
                    depth_or_layers: 1,
                    format,
                    mip_levels: 1,
                    sample_count: 1,
                    dimension: texture::TextureDimension::D2,
                    usage: None,
                    view_formats: Vec::new(),
                    label: Some("Invoke-WgpuRender".to_string()),
                };
                texture::create(&core, &spec)?
            }
        };
        let samples = self.sample_count.unwrap_or(1);
        let inner = if shader.type_name()? == "Wgpu.RenderPipeline" {
            PsProxy::<RenderPipeline>::from_ps(shader)?.with(|p| p.inner.clone())?
        } else {
            let parsed = pipeline::parsed_of(shader, None, None)?;
            let topology = self.topology.unwrap_or(PrimitiveTopology::TriangleList);
            let depth_format = depth_target(&core, self.depth, self.depth_format.as_deref())?;
            let spec = RenderSpec {
                vertex_entry: self.vertex_entry.clone(),
                fragment_entry: self.fragment_entry.clone(),
                formats: vec![target.texture.format()],
                depth_stencil: depth_stencil_state(depth_format, &self.depth_test(), &self.stencil_test(), topology)?,
                topology,
                cull: self.cull_mode.unwrap_or(CullMode::None),
                blend: blend_state(
                    self.blend.unwrap_or(BlendMode::Replace),
                    &CustomBlend {
                        source: self.blend_source,
                        destination: self.blend_destination,
                        operation: self.blend_operation,
                        alpha_source: self.alpha_blend_source,
                        alpha_destination: self.alpha_blend_destination,
                        alpha_operation: self.alpha_blend_operation,
                    },
                )?,
                sample_count: samples,
                constants: pipeline::constants_of(self.constant.as_ref())?,
            };
            build_render(&core, &parsed, spec, None)?
        };
        let blend_constant = constant_of(&self.blend_constant)?;
        let reads = reads_constant(inner.spec.blend);
        if reads && blend_constant.is_none() {
            return Err(invalid("WgpuBlendConstant", "the blend reads a constant: give -BlendConstant"));
        }
        if !reads && blend_constant.is_some() {
            return Err(invalid("WgpuBlendConstant", "-BlendConstant sets the constant a blend reads, and this blend reads none"));
        }
        if self.stencil_reference.is_some() && !inner.spec.tests_stencil() {
            return Err(invalid("WgpuStencilReference", "-StencilReference sets the value a stencil test compares with and Replace writes, and this pipeline has no stencil test"));
        }
        let (depth_clear, stencil_clear) = target_clears(inner.spec.depth_format(), self.clear_depth, self.clear_stencil, ["-ClearDepth", "-ClearStencil"])?;
        let (vertex_buffers, counted) = vertex_buffers(&core, &inner.inputs, self.vertex.as_ref())?;
        let bound: Vec<Bound> = binding::resolve_all(&core, &inner.bindings, self.binding.as_ref())?;
        let groups = binding::make_groups(&core, &inner.groups, &bound, &inner.label)?;
        let index = match &self.index {
            Some(value) if value.type_name()? == "Wgpu.Buffer" => {
                let (raw, ty, length) = PsProxy::<Buffer>::from_ps(value)?.with(|b| (b.buffer.clone(), b.element_type, b.length))?;
                Some((raw, ty, length as u32))
            }
            Some(value) => Some(element::with_host_bytes(value, None, |ty, bytes| {
                let ty = if ty == ElementType::UInt16 { ty } else { ElementType::UInt32 };
                let converted = element::convert(ElementType::UInt32, bytes, ty);
                let source = if ty == ElementType::UInt16 { bytes.to_vec() } else { converted };
                let raw = buffer::create_raw(&core, source.len() as u64, wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST, "indices", Some(&source))?;
                Ok((raw, ty, (source.len() as u64 / ty.size()) as u32))
            })?),
            None => None,
        };
        let vertex_count = match (self.vertex_count, counted) {
            (Some(n), _) => n,
            (None, Some(n)) => n,
            (None, None) => return Err(invalid("WgpuVertexCount", "the shader reads no vertex inputs; give -VertexCount")),
        };
        let index = index.as_ref().map(|(raw, ty, count)| (raw, if *ty == ElementType::UInt16 { wgpu::IndexFormat::Uint16 } else { wgpu::IndexFormat::Uint32 }, *count));
        let call = DrawCall {
            groups: &groups,
            vertex_buffers: &vertex_buffers,
            index,
            vertex_count,
            instances: self.instance_count.unwrap_or(1),
            blend_constant,
            stencil_reference: self.stencil_reference,
        };
        let commands = record_draw(&inner, &target.texture, &call, wgpu::LoadOp::Clear(color_of(&self.clear_color)?), depth_clear, stencil_clear)?;
        core.submit_and_wait(vec![commands], Some(ps))?;
        if let Some(path) = &self.out_file {
            let resolved = ps.resolve_path(path, true)?;
            let file = resolved.into_iter().next().ok_or_else(|| invalid("WgpuImage", format!("{path} names no file")))?;
            let mut said = Said::default();
            let saved = image_io::save(&core, &target.texture, target.is_cube, Path::new(&file), None, Some(ps), &mut said);
            texture::tell(ps, &said)?;
            saved?;
            if !self.pass_thru {
                return Ok(());
            }
        }
        ps.write(target)
    }
}

/// Builds a render pipeline from a shader with a vertex and a fragment
/// entry point, for one device and one target format.
///
/// # Examples
/// $pipeline = New-WgpuRenderPipeline $wgsl -Format Rgba8Unorm
/// New-WgpuRenderPipeline $wgsl -Depth -SampleCount 4 -Blend Alpha
#[cmdlet(verb = "New", noun = "WgpuRenderPipeline", alias = ["New-GpuRenderPipeline"], output = ["Wgpu.RenderPipeline"])]
#[derive(Default)]
pub struct NewWgpuRenderPipeline {
    /// WGSL text or a Wgpu.Shader with a vertex and a fragment entry point.
    #[param(mandatory, position = 0, value_from_pipeline, raw)]
    pub shader: Option<PsObject>,
    /// The formats of the color targets; Rgba8Unorm when omitted.
    #[param]
    pub format: Option<Vec<String>>,
    /// Test and write depth in a Depth32Float target.
    #[param]
    pub depth: bool,
    /// Test depth, stencil or both in a target of this format: Stencil8, Depth16Unorm, Depth24Plus, Depth24PlusStencil8, Depth32Float or Depth32FloatStencil8.
    #[param]
    pub depth_format: Option<String>,
    /// How a fragment's depth compares with the depth under it, for the fragment to be drawn; Less when omitted.
    #[param]
    pub depth_compare: Option<CompareFunction>,
    /// Whether a fragment that passes the depth test writes its depth; it does unless -DepthWrite:$false.
    #[param]
    pub depth_write: Option<bool>,
    /// A constant added to each triangle's depth, in the depth format's smallest steps; 0 when omitted.
    #[param]
    pub depth_bias: Option<i32>,
    /// A bias this many times the triangle's depth slope; 0 when omitted.
    #[param]
    pub depth_bias_slope_scale: Option<f64>,
    /// The largest bias the two give together, or the most negative when below 0; no limit when omitted.
    #[param]
    pub depth_bias_clamp: Option<f64>,
    /// How the stencil test compares the stencil reference, which a pass sets, with the stencil value under a fragment, the reference on the left; Always when omitted.
    #[param]
    pub stencil_compare: Option<CompareFunction>,
    /// What a failed stencil test does to the stencil value; Keep when omitted.
    #[param]
    pub stencil_fail: Option<StencilOperation>,
    /// What a passed stencil test whose depth test fails does to the stencil value; Keep when omitted.
    #[param]
    pub stencil_depth_fail: Option<StencilOperation>,
    /// What passing the stencil test and the depth test does to the stencil value; Keep when omitted.
    #[param]
    pub stencil_pass: Option<StencilOperation>,
    /// The bits of the reference and of the stencil value the stencil test compares; all of them when omitted.
    #[param]
    pub stencil_read_mask: Option<u32>,
    /// The bits of the stencil value the stencil operations change; all of them when omitted.
    #[param]
    pub stencil_write_mask: Option<u32>,
    /// -StencilCompare for triangles facing away; -StencilCompare's when omitted.
    #[param]
    pub back_stencil_compare: Option<CompareFunction>,
    /// -StencilFail for triangles facing away; -StencilFail's when omitted.
    #[param]
    pub back_stencil_fail: Option<StencilOperation>,
    /// -StencilDepthFail for triangles facing away; -StencilDepthFail's when omitted.
    #[param]
    pub back_stencil_depth_fail: Option<StencilOperation>,
    /// -StencilPass for triangles facing away; -StencilPass's when omitted.
    #[param]
    pub back_stencil_pass: Option<StencilOperation>,
    /// How vertices make primitives; TriangleList when omitted.
    #[param]
    pub topology: Option<PrimitiveTopology>,
    /// Which faces to discard; none when omitted.
    #[param]
    pub cull_mode: Option<CullMode>,
    /// How fragments combine with the target; Replace when omitted. Constant
    /// mixes by the constant a draw sets, and Custom takes -BlendSource,
    /// -BlendDestination and -BlendOperation.
    #[param]
    pub blend: Option<BlendMode>,
    /// With -Blend Custom, what the fragment's color is multiplied by; One when omitted.
    #[param]
    pub blend_source: Option<BlendFactor>,
    /// With -Blend Custom, what the color under the fragment is multiplied by; Zero when omitted.
    #[param]
    pub blend_destination: Option<BlendFactor>,
    /// With -Blend Custom, how the two products combine; Add when omitted.
    #[param]
    pub blend_operation: Option<BlendOperation>,
    /// With -Blend Custom, what the fragment's alpha is multiplied by; -BlendSource when omitted.
    #[param]
    pub alpha_blend_source: Option<BlendFactor>,
    /// With -Blend Custom, what the alpha under the fragment is multiplied by; -BlendDestination when omitted.
    #[param]
    pub alpha_blend_destination: Option<BlendFactor>,
    /// With -Blend Custom, how the two alpha products combine; -BlendOperation when omitted.
    #[param]
    pub alpha_blend_operation: Option<BlendOperation>,
    /// Samples per pixel; 1 when omitted.
    #[param]
    pub sample_count: Option<u32>,
    /// The vertex entry point; the shader's only one when omitted.
    #[param]
    pub vertex_entry: Option<String>,
    /// The fragment entry point; the shader's only one when omitted.
    #[param]
    pub fragment_entry: Option<String>,
    /// Override constant values, by name or `@id`.
    #[param]
    pub constant: Option<PsHashtable>,
    /// The pipeline's label.
    #[param]
    pub label: Option<String>,
    /// The device to build it on; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
}

impl NewWgpuRenderPipeline {
    /// The depth test the parameters name.
    fn depth_test(&self) -> DepthTest {
        DepthTest {
            compare: self.depth_compare,
            write: self.depth_write,
            bias: self.depth_bias,
            bias_slope_scale: self.depth_bias_slope_scale.map(|s| s as f32),
            bias_clamp: self.depth_bias_clamp.map(|c| c as f32),
        }
    }

    /// The stencil test the parameters name.
    fn stencil_test(&self) -> StencilTest {
        StencilTest {
            front: StencilFace { compare: self.stencil_compare, fail: self.stencil_fail, depth_fail: self.stencil_depth_fail, pass: self.stencil_pass },
            back: StencilFace { compare: self.back_stencil_compare, fail: self.back_stencil_fail, depth_fail: self.back_stencil_depth_fail, pass: self.back_stencil_pass },
            read_mask: self.stencil_read_mask,
            write_mask: self.stencil_write_mask,
        }
    }
}

impl Cmdlet for NewWgpuRenderPipeline {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(shader) = &self.shader else {
            return Err(invalid("WgpuNoShader", "no shader was given"));
        };
        let core = core_for(&self.device, ps)?;
        let parsed = pipeline::parsed_of(shader, None, None)?;
        let formats = match &self.format {
            Some(list) => list.iter().map(|n| texture::parse_format(n)).collect::<PsResult<Vec<_>>>()?,
            None => vec![wgpu::TextureFormat::Rgba8Unorm],
        };
        let topology = self.topology.unwrap_or(PrimitiveTopology::TriangleList);
        let depth_format = depth_target(&core, self.depth, self.depth_format.as_deref())?;
        let spec = RenderSpec {
            vertex_entry: self.vertex_entry.clone(),
            fragment_entry: self.fragment_entry.clone(),
            formats,
            depth_stencil: depth_stencil_state(depth_format, &self.depth_test(), &self.stencil_test(), topology)?,
            topology,
            cull: self.cull_mode.unwrap_or(CullMode::None),
            blend: blend_state(
                self.blend.unwrap_or(BlendMode::Replace),
                &CustomBlend {
                    source: self.blend_source,
                    destination: self.blend_destination,
                    operation: self.blend_operation,
                    alpha_source: self.alpha_blend_source,
                    alpha_destination: self.alpha_blend_destination,
                    alpha_operation: self.alpha_blend_operation,
                },
            )?,
            sample_count: self.sample_count.unwrap_or(1),
            constants: pipeline::constants_of(self.constant.as_ref())?,
        };
        ps.write(RenderPipeline::from_inner(build_render(&core, &parsed, spec, self.label.clone())?))
    }
}

/// The depth and stencil format names that start with `word`, ignoring
/// case.
fn depth_format_completions(word: &str) -> Vec<Completion> {
    let prefix = word.to_lowercase();
    DEPTH_FORMATS.iter().map(|f| texture::format_name(*f)).filter(|n| n.to_lowercase().starts_with(&prefix)).map(Completion::value).collect()
}

/// Completes -DepthFormat with the depth and stencil formats.
#[completer(cmdlet = "New-WgpuRenderPipeline", parameter = "DepthFormat")]
pub(crate) fn complete_pipeline_depth_format(ctx: &CompletionContext) -> PsResult<Vec<Completion>> {
    Ok(depth_format_completions(&ctx.word))
}

/// Completes -DepthFormat with the depth and stencil formats.
#[completer(cmdlet = "Invoke-WgpuRender", parameter = "DepthFormat")]
pub(crate) fn complete_render_depth_format(ctx: &CompletionContext) -> PsResult<Vec<Completion>> {
    Ok(depth_format_completions(&ctx.word))
}

/// A render pass being recorded into an encoder.
#[psclass(name = "Wgpu.RenderPass", mode = proxy, columns = ["Label", "IsEnded"])]
pub struct RenderPass {
    /// The pass's label.
    pub label: String,
    /// Whether End was called; the encoder takes no other command until it is.
    pub is_ended: bool,
    #[psfield(skip)]
    pub(crate) pass: Option<wgpu::RenderPass<'static>>,
}

impl RenderPass {
    fn pass(&mut self) -> PsResult<&mut wgpu::RenderPass<'static>> {
        self.pass.as_mut().ok_or_else(|| invalid_operation("WgpuPassEnded", "the pass has ended"))
    }
}

#[psmethods]
impl RenderPass {
    /// Sets the pipeline the next draws use.
    pub fn set_pipeline(&mut self, pipeline: PsProxy<RenderPipeline>) -> PsResult<()> {
        let inner = pipeline.with(|p| p.inner.clone())?;
        self.pass()?.set_pipeline(&inner.pipeline);
        Ok(())
    }

    /// Sets bind group `index`.
    pub fn set_bind_group(&mut self, index: u32, bind_group: PsProxy<BindGroup>) -> PsResult<()> {
        let group = bind_group.with(|g| g.group_object.clone())?;
        self.pass()?.set_bind_group(index, &group, &[]);
        Ok(())
    }

    /// Feeds vertex input `slot` from a buffer.
    pub fn set_vertex_buffer(&mut self, slot: u32, buffer: PsProxy<Buffer>) -> PsResult<()> {
        let raw = buffer.with(|b| b.buffer.clone())?;
        self.pass()?.set_vertex_buffer(slot, raw.slice(..));
        Ok(())
    }

    /// Sets the index buffer: UInt16 or UInt32 elements.
    pub fn set_index_buffer(&mut self, buffer: PsProxy<Buffer>) -> PsResult<()> {
        let (raw, ty) = buffer.with(|b| (b.buffer.clone(), b.element_type))?;
        let format = match ty {
            ElementType::UInt16 => wgpu::IndexFormat::Uint16,
            ElementType::UInt32 => wgpu::IndexFormat::Uint32,
            other => return Err(invalid("WgpuIndexType", format!("an index buffer holds UInt16 or UInt32, not {other:?}"))),
        };
        self.pass()?.set_index_buffer(raw.slice(..), format);
        Ok(())
    }

    /// Draws `vertexCount` vertices of `instanceCount` instances (1 when omitted).
    pub fn draw(&mut self, vertex_count: u32, instance_count: Option<u32>, first_vertex: Option<u32>, first_instance: Option<u32>) -> PsResult<()> {
        let (fv, fi) = (first_vertex.unwrap_or(0), first_instance.unwrap_or(0));
        self.pass()?.draw(fv..fv + vertex_count, fi..fi + instance_count.unwrap_or(1));
        Ok(())
    }

    /// Draws `indexCount` indices of `instanceCount` instances (1 when omitted).
    pub fn draw_indexed(&mut self, index_count: u32, instance_count: Option<u32>, first_index: Option<u32>, base_vertex: Option<i32>, first_instance: Option<u32>) -> PsResult<()> {
        let (fx, fi) = (first_index.unwrap_or(0), first_instance.unwrap_or(0));
        self.pass()?.draw_indexed(fx..fx + index_count, base_vertex.unwrap_or(0), fi..fi + instance_count.unwrap_or(1));
        Ok(())
    }

    /// Draws with the arguments a buffer holds at byte `offset`.
    pub fn draw_indirect(&mut self, buffer: PsProxy<Buffer>, offset: Option<u64>) -> PsResult<()> {
        let raw = buffer.with(|b| b.buffer.clone())?;
        self.pass()?.draw_indirect(&raw, offset.unwrap_or(0));
        Ok(())
    }

    /// Draws indexed with the arguments a buffer holds at byte `offset`.
    pub fn draw_indexed_indirect(&mut self, buffer: PsProxy<Buffer>, offset: Option<u64>) -> PsResult<()> {
        let raw = buffer.with(|b| b.buffer.clone())?;
        self.pass()?.draw_indexed_indirect(&raw, offset.unwrap_or(0));
        Ok(())
    }

    /// Sets the viewport, with depths 0 to 1 when omitted.
    pub fn set_viewport(&mut self, x: f32, y: f32, width: f32, height: f32, min_depth: Option<f32>, max_depth: Option<f32>) -> PsResult<()> {
        self.pass()?.set_viewport(x, y, width, height, min_depth.unwrap_or(0.0), max_depth.unwrap_or(1.0));
        Ok(())
    }

    /// Limits drawing to a rectangle.
    pub fn set_scissor_rect(&mut self, x: u32, y: u32, width: u32, height: u32) -> PsResult<()> {
        self.pass()?.set_scissor_rect(x, y, width, height);
        Ok(())
    }

    /// Sets the constant color blend factors read.
    pub fn set_blend_constant(&mut self, r: f64, g: f64, b: f64, a: f64) -> PsResult<()> {
        self.pass()?.set_blend_constant(wgpu::Color { r, g, b, a });
        Ok(())
    }

    /// Sets the stencil reference value.
    pub fn set_stencil_reference(&mut self, reference: u32) -> PsResult<()> {
        self.pass()?.set_stencil_reference(reference);
        Ok(())
    }

    /// Starts counting samples into occlusion query `index` of the pass's query set.
    pub fn begin_occlusion_query(&mut self, index: u32) -> PsResult<()> {
        self.pass()?.begin_occlusion_query(index);
        Ok(())
    }

    /// Stops counting samples.
    pub fn end_occlusion_query(&mut self) -> PsResult<()> {
        self.pass()?.end_occlusion_query();
        Ok(())
    }

    /// Opens a named group of commands for graphics debuggers.
    pub fn push_debug_group(&mut self, label: String) -> PsResult<()> {
        self.pass()?.push_debug_group(&label);
        Ok(())
    }

    /// Closes the innermost debug group.
    pub fn pop_debug_group(&mut self) -> PsResult<()> {
        self.pass()?.pop_debug_group();
        Ok(())
    }

    /// Marks a point in the pass for graphics debuggers.
    pub fn insert_debug_marker(&mut self, label: String) -> PsResult<()> {
        self.pass()?.insert_debug_marker(&label);
        Ok(())
    }

    /// Ends the pass, so its encoder takes commands again.
    pub fn end(&mut self) -> PsResult<()> {
        self.pass = None;
        self.is_ended = true;
        Ok(())
    }
}

/// The view a render attachment value names, a texture or a view, and the
/// format it reads its texels as.
fn attachment_view(value: &PsObject) -> PsResult<(wgpu::TextureView, wgpu::TextureFormat)> {
    match value.type_name()?.as_str() {
        "Wgpu.Texture" => PsProxy::<Texture>::from_ps(value)?.with(|t| (t.texture.create_view(&wgpu::TextureViewDescriptor::default()), t.texture.format())),
        "Wgpu.TextureView" => {
            let (view, format) = PsProxy::<TextureView>::from_ps(value)?.with(|v| (v.view.clone(), v.format.clone()))?;
            Ok((view, texture::parse_format(&format)?))
        }
        other => Err(invalid("WgpuAttachment", format!("a render attachment is a Wgpu.Texture or a Wgpu.TextureView, not {other}"))),
    }
}

/// What a render pass begins with: the view it draws into and what that
/// holds first, a depth target with what its depth and stencil hold first,
/// the view a multisampled color resolves into, the query set its
/// occlusion queries count into, and its label.
struct PassTargets {
    color: wgpu::TextureView,
    color_load: wgpu::LoadOp<wgpu::Color>,
    depth: Option<(wgpu::TextureView, wgpu::TextureFormat)>,
    depth_load: wgpu::LoadOp<f32>,
    stencil_load: wgpu::LoadOp<u32>,
    resolve: Option<wgpu::TextureView>,
    occlusion: Option<wgpu::QuerySet>,
    label: Option<String>,
}

impl PassTargets {
    /// Begins the pass on `encoder`, loading each aspect the depth target
    /// has.
    fn begin(self, encoder: &mut CommandEncoder) -> PsResult<RenderPass> {
        let label = match self.label {
            Some(l) => l,
            None => format!("{} render pass", encoder.label),
        };
        let pass = encoder
            .encoder()?
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(&label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.color,
                    depth_slice: None,
                    resolve_target: self.resolve.as_ref(),
                    ops: wgpu::Operations { load: self.color_load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: self.depth.as_ref().map(|(view, format)| wgpu::RenderPassDepthStencilAttachment {
                    view,
                    depth_ops: aspect_ops(format.has_depth_aspect(), self.depth_load),
                    stencil_ops: aspect_ops(format.has_stencil_aspect(), self.stencil_load),
                }),
                timestamp_writes: None,
                occlusion_query_set: self.occlusion.as_ref(),
                multiview_mask: None,
            })
            .forget_lifetime();
        Ok(RenderPass { label, is_ended: false, pass: Some(pass) })
    }
}

/// Begins a render pass on an encoder: implemented here and called from
/// the encoder's BeginRenderPass method. The pass clears its color to
/// `clear_color`, and its depth target's depth to 1 and stencil to 0.
pub(crate) fn begin_render_pass(
    encoder: &mut CommandEncoder,
    color: &PsObject,
    clear_color: Option<Vec<f64>>,
    depth: Option<&PsObject>,
    resolve_target: Option<&PsObject>,
    occlusion: Option<&PsProxy<QuerySet>>,
    label: Option<String>,
) -> PsResult<RenderPass> {
    let targets = PassTargets {
        color: attachment_view(color)?.0,
        color_load: wgpu::LoadOp::Clear(color_of(&clear_color)?),
        depth: match depth {
            Some(d) => Some(attachment_view(d)?),
            None => None,
        },
        depth_load: wgpu::LoadOp::Clear(1.0),
        stencil_load: wgpu::LoadOp::Clear(0),
        resolve: match resolve_target {
            Some(r) => Some(attachment_view(r)?.0),
            None => None,
        },
        occlusion: match occlusion {
            Some(q) => Some(q.with(|q| q.set.clone())?),
            None => None,
        },
        label,
    };
    targets.begin(encoder)
}

/// The keys of the hashtable BeginRenderPass takes, after WebGPU's render
/// pass descriptor.
const PASS_KEYS: [&str; 8] = ["Color", "ClearColor", "Depth", "ClearDepth", "ClearStencil", "ResolveTarget", "OcclusionQuerySet", "Label"];

/// Whether `value` is a hashtable, which BeginRenderPass takes in place of
/// its arguments.
pub(crate) fn is_pass_options(value: &PsObject) -> PsResult<bool> {
    let name = value.type_name()?;
    Ok(name == "System.Collections.Hashtable" || name == "System.Collections.Specialized.OrderedDictionary")
}

/// Whether a clear value is 'Load', which keeps what the texture holds.
fn is_load(value: &PsObject) -> PsResult<bool> {
    Ok(value.type_name()? == "System.String" && String::from_ps(value)?.eq_ignore_ascii_case("Load"))
}

/// Begins a render pass on an encoder from a hashtable naming its
/// attachments and clears by the keys of PASS_KEYS. A clear key given
/// 'Load' keeps what its texture holds, and an omitted one clears color to
/// opaque black, depth to 1 and stencil to 0.
pub(crate) fn begin_render_pass_with(encoder: &mut CommandEncoder, options: &PsObject) -> PsResult<RenderPass> {
    let table = PsHashtable(options.clone());
    let mut values: [Option<PsObject>; 8] = Default::default();
    for key in table.keys()? {
        let Some(slot) = PASS_KEYS.iter().position(|k| k.eq_ignore_ascii_case(&key)) else {
            return Err(invalid(
                "WgpuRenderPassOption",
                format!("BeginRenderPass takes the keys Color, ClearColor, Depth, ClearDepth, ClearStencil, ResolveTarget, OcclusionQuerySet and Label, and {key} is none of them"),
            ));
        };
        values[slot] = Some(table.get(&key)?);
    }
    let [color, clear_color, depth, clear_depth, clear_stencil, resolve, occlusion, label] = values;
    let Some(color) = color else {
        return Err(invalid("WgpuRenderPassOption", "BeginRenderPass's hashtable names no Color, the texture or view the pass draws into"));
    };
    let color_load = match &clear_color {
        Some(value) if is_load(value)? => wgpu::LoadOp::Load,
        Some(value) => {
            let numbers = Vec::<f64>::from_ps(value).map_err(|e| invalid("WgpuClearColor", format!("ClearColor is red, green, blue and alpha, or 'Load': {}", e.message)))?;
            wgpu::LoadOp::Clear(color_of(&Some(numbers))?)
        }
        None => wgpu::LoadOp::Clear(wgpu::Color::BLACK),
    };
    let depth = match &depth {
        Some(d) => Some(attachment_view(d)?),
        None => None,
    };
    let format = depth.as_ref().map(|(_, f)| *f);
    if clear_depth.is_some() && !format.is_some_and(|f| f.has_depth_aspect()) {
        return Err(invalid("WgpuClearDepth", format!("ClearDepth sets what the depth target's depth starts at, and {}", lacking(format, "depth"))));
    }
    if clear_stencil.is_some() && !format.is_some_and(|f| f.has_stencil_aspect()) {
        return Err(invalid("WgpuClearStencil", format!("ClearStencil sets what the depth target's stencil starts at, and {}", lacking(format, "stencil"))));
    }
    let depth_load = match &clear_depth {
        Some(value) if is_load(value)? => wgpu::LoadOp::Load,
        Some(value) => {
            let number = f64::from_ps(value).map_err(|e| invalid("WgpuClearDepth", format!("ClearDepth is a depth from 0 to 1 or 'Load': {}", e.message)))?;
            if !(0.0..=1.0).contains(&number) {
                return Err(invalid("WgpuClearDepth", format!("ClearDepth is a depth from 0 to 1 or 'Load', and {number} is not")));
            }
            wgpu::LoadOp::Clear(number as f32)
        }
        None => wgpu::LoadOp::Clear(1.0),
    };
    let stencil_load = match &clear_stencil {
        Some(value) if is_load(value)? => wgpu::LoadOp::Load,
        Some(value) => wgpu::LoadOp::Clear(u32::from_ps(value).map_err(|e| invalid("WgpuClearStencil", format!("ClearStencil is a stencil value or 'Load': {}", e.message)))?),
        None => wgpu::LoadOp::Clear(0),
    };
    let targets = PassTargets {
        color: attachment_view(&color)?.0,
        color_load,
        depth,
        depth_load,
        stencil_load,
        resolve: match &resolve {
            Some(r) => Some(attachment_view(r)?.0),
            None => None,
        },
        occlusion: match &occlusion {
            Some(q) => {
                let set = PsProxy::<QuerySet>::from_ps(q).map_err(|e| invalid("WgpuRenderPassOption", format!("OcclusionQuerySet is a Wgpu.QuerySet: {}", e.message)))?;
                Some(set.with(|q| q.set.clone())?)
            }
            None => None,
        },
        label: match &label {
            Some(l) => Some(String::from_ps(l)?),
            None => None,
        },
    };
    targets.begin(encoder)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each mode builds wgpu's blend state for it; a Custom blend's alpha
    /// takes what its color names unless it names its own; a factor or an
    /// operation beside any mode but Custom is refused.
    #[test]
    fn builds_the_blend_each_mode_names_and_refuses_a_custom_part_without_custom() {
        let none = CustomBlend::default();
        assert_eq!(blend_state(BlendMode::Replace, &none).expect("replace"), None);
        let constant = blend_state(BlendMode::Constant, &none).expect("constant").expect("a state");
        let mixed = wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Constant, dst_factor: wgpu::BlendFactor::OneMinusConstant, operation: wgpu::BlendOperation::Add };
        assert_eq!((constant.color, constant.alpha), (mixed, mixed));
        let plain = blend_state(BlendMode::Custom, &none).expect("custom").expect("a state");
        assert_eq!(plain, wgpu::BlendState::REPLACE);
        let custom = CustomBlend { destination: Some(BlendFactor::One), operation: Some(BlendOperation::ReverseSubtract), alpha_operation: Some(BlendOperation::Max), ..CustomBlend::default() };
        let state = blend_state(BlendMode::Custom, &custom).expect("custom").expect("a state");
        assert_eq!(state.color, wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::ReverseSubtract });
        assert_eq!(state.alpha, wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Max });
        let refused = blend_state(BlendMode::Alpha, &custom).expect_err("refused");
        assert_eq!(refused.error_id, "WgpuBlend");
        assert!(refused.message.ends_with("the blend is Alpha: give -Blend Custom"), "{}", refused.message);
    }

    /// A blend reads the constant when a factor of its color or of its
    /// alpha is Constant or OneMinusConstant, and not otherwise.
    #[test]
    fn tells_a_blend_that_reads_the_constant() {
        let none = CustomBlend::default();
        let blend = |mode: BlendMode, custom: &CustomBlend| blend_state(mode, custom).expect("a blend");
        assert!(reads_constant(blend(BlendMode::Constant, &none)));
        assert!(reads_constant(blend(BlendMode::Custom, &CustomBlend { alpha_destination: Some(BlendFactor::OneMinusConstant), ..CustomBlend::default() })));
        assert!(!reads_constant(blend(BlendMode::Replace, &none)));
        assert!(!reads_constant(blend(BlendMode::Alpha, &none)));
        assert!(!reads_constant(blend(BlendMode::Custom, &CustomBlend { source: Some(BlendFactor::SrcAlpha), ..CustomBlend::default() })));
    }

    /// With no test named, a depth target tests Less and writes, and its
    /// stencil test changes nothing; each part of the back face is the
    /// front face's unless it names its own; a mask is every bit unless
    /// given; and a target with no depth aspect leaves depth off.
    #[test]
    fn builds_the_depth_and_stencil_state_the_tests_name() {
        let triangles = PrimitiveTopology::TriangleList;
        let (no_depth, no_stencil) = (DepthTest::default(), StencilTest::default());
        assert_eq!(depth_stencil_state(None, &no_depth, &no_stencil, triangles).expect("no target"), None);
        let plain = depth_stencil_state(Some(wgpu::TextureFormat::Depth32Float), &no_depth, &no_stencil, triangles).expect("plain").expect("a state");
        assert_eq!((plain.depth_compare, plain.depth_write_enabled), (Some(wgpu::CompareFunction::Less), Some(true)));
        assert!(!plain.stencil.is_enabled());
        assert_eq!(plain.bias, wgpu::DepthBiasState::default());
        let depth = DepthTest { compare: Some(CompareFunction::Greater), write: Some(false), bias: Some(2), bias_slope_scale: Some(1.5), bias_clamp: Some(0.25) };
        let tested = depth_stencil_state(Some(wgpu::TextureFormat::Depth24Plus), &depth, &no_stencil, triangles).expect("tested").expect("a state");
        assert_eq!((tested.depth_compare, tested.depth_write_enabled), (Some(wgpu::CompareFunction::Greater), Some(false)));
        assert_eq!(tested.bias, wgpu::DepthBiasState { constant: 2, slope_scale: 1.5, clamp: 0.25 });
        let stencil = StencilTest {
            front: StencilFace { compare: Some(CompareFunction::Equal), pass: Some(StencilOperation::Replace), ..StencilFace::default() },
            back: StencilFace { fail: Some(StencilOperation::Zero), ..StencilFace::default() },
            read_mask: Some(0x0f),
            write_mask: None,
        };
        let masked = depth_stencil_state(Some(wgpu::TextureFormat::Depth24PlusStencil8), &no_depth, &stencil, triangles).expect("masked").expect("a state");
        let front = wgpu::StencilFaceState {
            compare: wgpu::CompareFunction::Equal,
            fail_op: wgpu::StencilOperation::Keep,
            depth_fail_op: wgpu::StencilOperation::Keep,
            pass_op: wgpu::StencilOperation::Replace,
        };
        assert_eq!(masked.stencil.front, front);
        assert_eq!(masked.stencil.back, wgpu::StencilFaceState { fail_op: wgpu::StencilOperation::Zero, ..front });
        assert_eq!((masked.stencil.read_mask, masked.stencil.write_mask), (0x0f, u32::MAX));
        let stencil_only = depth_stencil_state(Some(wgpu::TextureFormat::Stencil8), &no_depth, &stencil, triangles).expect("stencil only").expect("a state");
        assert_eq!((stencil_only.depth_compare, stencil_only.depth_write_enabled), (None, None));
        assert!(stencil_only.stencil.is_enabled());
    }

    /// A depth test needs a depth target with a depth aspect, a stencil
    /// test one with a stencil aspect, and a depth bias triangles.
    #[test]
    fn refuses_a_test_its_depth_target_cannot_hold() {
        let triangles = PrimitiveTopology::TriangleList;
        let greater = DepthTest { compare: Some(CompareFunction::Greater), ..DepthTest::default() };
        let replace = StencilTest { front: StencilFace { pass: Some(StencilOperation::Replace), ..StencilFace::default() }, ..StencilTest::default() };
        let refused = |format, depth: &DepthTest, stencil: &StencilTest, topology| depth_stencil_state(format, depth, stencil, topology).expect_err("refused");
        let without = refused(None, &greater, &StencilTest::default(), triangles);
        assert_eq!(without.error_id, "WgpuDepthTest");
        assert!(without.message.contains("and there is no depth target"), "{}", without.message);
        let stencil8 = refused(Some(wgpu::TextureFormat::Stencil8), &greater, &StencilTest::default(), triangles);
        assert!(stencil8.message.contains("the depth target, Stencil8, has no depth aspect"), "{}", stencil8.message);
        let depth_only = refused(Some(wgpu::TextureFormat::Depth32Float), &DepthTest::default(), &replace, triangles);
        assert_eq!(depth_only.error_id, "WgpuStencil");
        assert!(depth_only.message.contains("the depth target, Depth32Float, has no stencil aspect"), "{}", depth_only.message);
        let biased = DepthTest { bias: Some(1), ..DepthTest::default() };
        let lines = refused(Some(wgpu::TextureFormat::Depth32Float), &biased, &StencilTest::default(), PrimitiveTopology::LineList);
        assert_eq!(lines.error_id, "WgpuDepthBias");
        assert_eq!(lines.message, "a depth bias moves triangles, and the topology is LineList");
    }

    /// A draw's own depth target starts at depth 1 and stencil 0 unless
    /// told otherwise; a start for an aspect the target lacks, and a depth
    /// outside 0 to 1, are refused.
    #[test]
    fn starts_a_draws_depth_target_where_it_is_told() {
        let names = ["-ClearDepth", "-ClearStencil"];
        let both = Some(wgpu::TextureFormat::Depth24PlusStencil8);
        assert_eq!(target_clears(both, None, None, names).expect("the defaults"), (1.0, 0));
        assert_eq!(target_clears(both, Some(0.0), Some(7), names).expect("the starts"), (0.0, 7));
        let depth = target_clears(Some(wgpu::TextureFormat::Stencil8), Some(0.5), None, names).expect_err("refused");
        assert_eq!(depth.error_id, "WgpuClearDepth");
        assert_eq!(depth.message, "-ClearDepth sets the depth the depth target starts at, and the depth target, Stencil8, has no depth aspect");
        let range = target_clears(both, Some(1.5), None, names).expect_err("refused");
        assert_eq!(range.message, "-ClearDepth is a depth from 0 to 1, and 1.5 is not");
        let stencil = target_clears(Some(wgpu::TextureFormat::Depth32Float), None, Some(1), names).expect_err("refused");
        assert_eq!(stencil.error_id, "WgpuClearStencil");
        assert_eq!(stencil.message, "-ClearStencil sets the value the stencil starts at, and the depth target, Depth32Float, has no stencil aspect");
        let none = target_clears(None, None, Some(1), names).expect_err("refused");
        assert!(none.message.ends_with("and there is no depth target"), "{}", none.message);
    }
}
