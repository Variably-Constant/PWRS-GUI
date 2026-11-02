//! The GPU layer's calls between a session and the egui renderer, in the
//! Wgpu module's terms: buffers of typed elements and render pipelines in
//! the renderer's table, compute shaders run over buffers or over arrays
//! sent with the call, and the draws a GpuSurface widget makes each frame;
//! and the calls that give the HTML renderer a page's surface and copies of
//! the table's objects its draws use.

use crate::WindowId;
use crate::camera::{CameraChange, CameraSpec};
use crate::view::{ViewChange, ViewShown};
use serde::{Deserialize, Serialize};

/// The scalar type of a GPU buffer's elements, named as the Wgpu module
/// names them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ElementType {
    /// WGSL `f32`, .NET `float`.
    #[default]
    Float32,
    /// WGSL `f64`, .NET `double`.
    Float64,
    /// WGSL `f16`, read and written as .NET `float`.
    Float16,
    Int8,
    UInt8,
    Int16,
    UInt16,
    /// WGSL `i32`, .NET `int`.
    Int32,
    /// WGSL `u32`, .NET `uint`.
    UInt32,
    /// WGSL `i64`, .NET `long`.
    Int64,
    /// WGSL `u64`, .NET `ulong`.
    UInt64,
}

impl ElementType {
    /// Every element type, in the order the Wgpu module lists them.
    pub const ALL: [ElementType; 11] = [
        ElementType::Float32,
        ElementType::Float64,
        ElementType::Float16,
        ElementType::Int8,
        ElementType::UInt8,
        ElementType::Int16,
        ElementType::UInt16,
        ElementType::Int32,
        ElementType::UInt32,
        ElementType::Int64,
        ElementType::UInt64,
    ];

    /// The type's name, as the Wgpu module writes it.
    pub fn name(self) -> &'static str {
        match self {
            ElementType::Float32 => "Float32",
            ElementType::Float64 => "Float64",
            ElementType::Float16 => "Float16",
            ElementType::Int8 => "Int8",
            ElementType::UInt8 => "UInt8",
            ElementType::Int16 => "Int16",
            ElementType::UInt16 => "UInt16",
            ElementType::Int32 => "Int32",
            ElementType::UInt32 => "UInt32",
            ElementType::Int64 => "Int64",
            ElementType::UInt64 => "UInt64",
        }
    }

    /// The type named `name`, compared without regard to case.
    pub fn named(name: &str) -> Option<ElementType> {
        ElementType::ALL.into_iter().find(|t| t.name().eq_ignore_ascii_case(name))
    }

    /// The size of one element in bytes.
    pub fn size(self) -> u64 {
        match self {
            ElementType::Int8 | ElementType::UInt8 => 1,
            ElementType::Float16 | ElementType::Int16 | ElementType::UInt16 => 2,
            ElementType::Float32 | ElementType::Int32 | ElementType::UInt32 => 4,
            ElementType::Float64 | ElementType::Int64 | ElementType::UInt64 => 8,
        }
    }
}

/// Which of a compute shader's bindings a resource is for, or which of a
/// GpuSurface draw's vertex inputs or bindings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuSlot {
    /// The binding at this place in the order the shader declares them, by
    /// group and then binding.
    Order(u32),
    /// The binding whose variable has this name, ignoring case, or
    /// `group:binding`.
    Name(String),
}

/// What a compute shader's binding is given, or a GpuSurface draw's vertex
/// input or binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GpuResource {
    /// A buffer in the renderer's table, which stays on the GPU.
    Buffer(u64),
    /// Elements of `element`, copied to the GPU as the type the shader
    /// declares for the binding or vertex input, or as they are for a
    /// binding of a struct; an array a compute shader may write comes back
    /// in [`ComputeReport::written`].
    Array { element: ElementType, bytes: Vec<u8> },
    /// A texture in the renderer's table, for a binding.
    Texture(u64),
    /// A sampler in the renderer's table, for a binding.
    Sampler(u64),
}

/// A texture's format, named as the Wgpu module names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TextureFormat {
    R8Unorm,
    Rg8Unorm,
    Rgba8Unorm,
    Rgba8UnormSrgb,
    Bgra8Unorm,
    Bgra8UnormSrgb,
    R16Float,
    Rg16Float,
    Rgba16Float,
    R32Float,
    Rg32Float,
    Rgba32Float,
    R32Uint,
    R32Sint,
}

/// How a sampler picks between texels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SamplerFilter {
    Nearest,
    #[default]
    Linear,
}

/// What a sampler reads outside a texture's edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AddressMode {
    #[default]
    ClampToEdge,
    Repeat,
    MirrorRepeat,
}

/// How many workgroups a compute run dispatches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuDispatch {
    /// Workgroups in x, y and z.
    Workgroups([u32; 3]),
    /// Invocations in x, y and z, divided by the shader's workgroup size and
    /// rounded up.
    Invocations([u64; 3]),
    /// Workgroups the GPU reads as three u32 from byte `offset` of `buffer`.
    Indirect { buffer: u64, offset: u64 },
    /// One invocation per element of the first buffer bound, in x.
    Auto,
}

/// What a buffer may be used for, named as the Wgpu module names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BufferUsage {
    MapRead,
    MapWrite,
    CopySrc,
    CopyDst,
    Index,
    Vertex,
    Uniform,
    Storage,
    Indirect,
    QueryResolve,
    BlasInput,
    TlasInput,
}

/// The language of a compute shader's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShaderLanguage {
    Wgsl,
    /// GLSL, whose entry point is `main`.
    Glsl,
}

/// How vertices make primitives, named as the Wgpu module names it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Topology {
    PointList,
    LineList,
    LineStrip,
    #[default]
    TriangleList,
    TriangleStrip,
}

/// Which faces a render pipeline discards, named as the Wgpu module names
/// it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Cull {
    #[default]
    None,
    Front,
    Back,
}

/// How a fragment's color combines with the color under it, named as the
/// Wgpu module names it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Blend {
    #[default]
    Replace,
    Alpha,
    Additive,
    Premultiplied,
    Constant,
    Custom,
}

/// What a Custom blend multiplies a color by, named as the Wgpu module
/// names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlendFactor {
    Zero,
    One,
    Src,
    OneMinusSrc,
    SrcAlpha,
    OneMinusSrcAlpha,
    Dst,
    OneMinusDst,
    DstAlpha,
    OneMinusDstAlpha,
    SrcAlphaSaturated,
    Constant,
    OneMinusConstant,
    Src1,
    OneMinusSrc1,
    Src1Alpha,
    OneMinusSrc1Alpha,
}

/// How a Custom blend combines its two products, named as the Wgpu module
/// names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlendOperation {
    Add,
    Subtract,
    ReverseSubtract,
    Min,
    Max,
}

/// The factors and operations of a Custom blend: the color ones One, Zero
/// and Add when absent, each alpha one its color one when absent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CustomBlend {
    pub source: Option<BlendFactor>,
    pub destination: Option<BlendFactor>,
    pub operation: Option<BlendOperation>,
    pub alpha_source: Option<BlendFactor>,
    pub alpha_destination: Option<BlendFactor>,
    pub alpha_operation: Option<BlendOperation>,
}

/// The format of a render pipeline's depth target, which holds depth,
/// stencil or both, named as the Wgpu module names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DepthFormat {
    Stencil8,
    Depth16Unorm,
    Depth24Plus,
    Depth24PlusStencil8,
    Depth32Float,
    /// Needs the adapter's Depth32FloatStencil8 feature.
    Depth32FloatStencil8,
}

impl DepthFormat {
    /// Every depth format, in the order the Wgpu module lists them.
    pub const ALL: [DepthFormat; 6] = [
        DepthFormat::Stencil8,
        DepthFormat::Depth16Unorm,
        DepthFormat::Depth24Plus,
        DepthFormat::Depth24PlusStencil8,
        DepthFormat::Depth32Float,
        DepthFormat::Depth32FloatStencil8,
    ];

    /// The format's name, as the Wgpu module writes it.
    pub fn name(self) -> &'static str {
        match self {
            DepthFormat::Stencil8 => "Stencil8",
            DepthFormat::Depth16Unorm => "Depth16Unorm",
            DepthFormat::Depth24Plus => "Depth24Plus",
            DepthFormat::Depth24PlusStencil8 => "Depth24PlusStencil8",
            DepthFormat::Depth32Float => "Depth32Float",
            DepthFormat::Depth32FloatStencil8 => "Depth32FloatStencil8",
        }
    }

    /// Whether the format holds depth.
    pub fn has_depth(self) -> bool {
        self != DepthFormat::Stencil8
    }

    /// Whether the format holds stencil.
    pub fn has_stencil(self) -> bool {
        matches!(self, DepthFormat::Stencil8 | DepthFormat::Depth24PlusStencil8 | DepthFormat::Depth32FloatStencil8)
    }
}

/// How a depth or a stencil test compares two values, named as the Wgpu
/// module names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompareFunction {
    Never,
    Less,
    Equal,
    LessEqual,
    Greater,
    NotEqual,
    GreaterEqual,
    Always,
}

/// What a stencil test does to the stencil value under a fragment, named
/// as the Wgpu module names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StencilOperation {
    Keep,
    Zero,
    Replace,
    Invert,
    IncrementClamp,
    DecrementClamp,
    IncrementWrap,
    DecrementWrap,
}

/// A render pipeline's depth test, as New-WgpuRenderPipeline's
/// -DepthCompare, -DepthWrite and depth bias give it, each part the Wgpu
/// module's own when absent: Less, a passing fragment writing its depth,
/// and no bias.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DepthTest {
    /// How a fragment's depth compares with the depth under it, the
    /// fragment's on the left.
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

/// The stencil test of one face, as New-WgpuRenderPipeline's
/// -StencilCompare, -StencilFail, -StencilDepthFail and -StencilPass give
/// it: how the stencil reference compares with the stencil value under a
/// fragment, the reference on the left, and what a failed test, a passed
/// test whose depth test fails, and a passed test do to that value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StencilFace {
    pub compare: Option<CompareFunction>,
    pub fail: Option<StencilOperation>,
    pub depth_fail: Option<StencilOperation>,
    pub pass: Option<StencilOperation>,
}

/// A render pipeline's stencil test, as the Wgpu module takes it: each
/// part of the front face Always or Keep when absent, each part of the
/// back face the front face's when absent, and each mask every bit when
/// absent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

impl StencilTest {
    /// The front face with every part given, as the Wgpu module fills it.
    pub fn front_face(&self) -> (CompareFunction, StencilOperation, StencilOperation, StencilOperation) {
        let keep = StencilOperation::Keep;
        let f = &self.front;
        (f.compare.unwrap_or(CompareFunction::Always), f.fail.unwrap_or(keep), f.depth_fail.unwrap_or(keep), f.pass.unwrap_or(keep))
    }

    /// The back face with every part given: its own, or else the front
    /// face's.
    pub fn back_face(&self) -> (CompareFunction, StencilOperation, StencilOperation, StencilOperation) {
        let (compare, fail, depth_fail, pass) = self.front_face();
        let b = &self.back;
        (b.compare.unwrap_or(compare), b.fail.unwrap_or(fail), b.depth_fail.unwrap_or(depth_fail), b.pass.unwrap_or(pass))
    }

    /// The masks, the comparison's and the operations', every bit when
    /// absent.
    pub fn masks(&self) -> (u32, u32) {
        (self.read_mask.unwrap_or(u32::MAX), self.write_mask.unwrap_or(u32::MAX))
    }

    /// Whether the test reads or changes the stencil, as wgpu decides: a
    /// face compares other than Always or does other than Keep, and a mask
    /// is not 0.
    pub fn is_enabled(&self) -> bool {
        let ignores = |face: (CompareFunction, StencilOperation, StencilOperation, StencilOperation)| {
            let keep = StencilOperation::Keep;
            face == (CompareFunction::Always, keep, keep, keep)
        };
        let (read, write) = self.masks();
        !(ignores(self.front_face()) && ignores(self.back_face())) && (read != 0 || write != 0)
    }
}

/// A render pipeline's shader and how it draws, as the Wgpu module's
/// New-WgpuRenderPipeline takes them, into a GpuSurface's texture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PipelineSpec {
    /// WGSL with a vertex and a fragment entry point.
    pub source: String,
    /// The vertex entry point; the shader's only one when `None`.
    pub vertex_entry: Option<String>,
    /// The fragment entry point; the shader's only one when `None`.
    pub fragment_entry: Option<String>,
    pub topology: Topology,
    pub cull: Cull,
    pub blend: Blend,
    /// A Custom blend's factors and operations, which any other blend
    /// refuses.
    pub custom_blend: CustomBlend,
    /// Whether it tests and writes depth in a Depth32Float attachment,
    /// when `depth_format` names no format.
    pub depth: bool,
    /// The format of its depth attachment, which turns depth on.
    pub depth_format: Option<DepthFormat>,
    /// How its depth attachment tests depth, which needs a format with
    /// depth.
    pub depth_test: DepthTest,
    /// How its depth attachment tests stencil, which needs a format with
    /// stencil.
    pub stencil: StencilTest,
    /// Samples per pixel, resolved into the surface's texture.
    pub sample_count: u32,
    /// Override constants by name or `@id`.
    pub constants: Vec<(String, f64)>,
}

impl PipelineSpec {
    /// The format of its depth attachment: `depth_format`, or Depth32Float
    /// with `depth` alone; none with neither.
    pub fn depth_target(&self) -> Option<DepthFormat> {
        self.depth_format.or(self.depth.then_some(DepthFormat::Depth32Float))
    }

    /// Whether its stencil test reads or changes the stencil, which a
    /// draw's stencil reference is for.
    pub fn tests_stencil(&self) -> bool {
        self.depth_target().is_some_and(DepthFormat::has_stencil) && self.stencil.is_enabled()
    }

    /// Why the Wgpu module refuses its depth and stencil tests, under that
    /// module's id and in its words, in the order it checks them: a depth
    /// test without a format that holds depth, a stencil test without one
    /// that holds stencil, and a depth bias on points or lines.
    pub fn depth_stencil_refusal(&self) -> Option<GpuFailure> {
        let target = self.depth_target();
        let lacking = |aspect: &str| match target {
            Some(f) => format!("the depth target, {}, has no {aspect} aspect", f.name()),
            None => String::from("there is no depth target"),
        };
        let refused = |id: &str, text: String| Some(GpuFailure { id: id.to_string(), text });
        if self.depth_test != DepthTest::default() && !target.is_some_and(DepthFormat::has_depth) {
            return refused("WgpuDepthTest", format!("-DepthCompare, -DepthWrite and the depth bias test depth, and {}: give -Depth or a -DepthFormat with depth", lacking("depth")));
        }
        if self.stencil != StencilTest::default() && !target.is_some_and(DepthFormat::has_stencil) {
            let text = format!("a stencil test needs a depth target with a stencil aspect, and {}: give -DepthFormat Stencil8, Depth24PlusStencil8 or Depth32FloatStencil8", lacking("stencil"));
            return refused("WgpuStencil", text);
        }
        let t = &self.depth_test;
        let biased = t.bias.unwrap_or(0) != 0 || t.bias_slope_scale.unwrap_or(0.0) != 0.0 || t.bias_clamp.unwrap_or(0.0) != 0.0;
        if biased && !matches!(self.topology, Topology::TriangleList | Topology::TriangleStrip) {
            return refused("WgpuDepthBias", format!("a depth bias moves triangles, and the topology is {:?}", self.topology));
        }
        None
    }

    /// Why the Wgpu module refuses its blend, under that module's id and in
    /// its words: a Custom blend's factor or operation beside another blend.
    pub fn blend_refusal(&self) -> Option<GpuFailure> {
        (self.blend != Blend::Custom && self.custom_blend != CustomBlend::default()).then(|| GpuFailure {
            id: String::from("WgpuBlend"),
            text: format!("-BlendSource, -BlendDestination, -BlendOperation and their alpha forms name a Custom blend, and the blend is {:?}: give -Blend Custom", self.blend),
        })
    }

    /// Why the Wgpu module refuses to build a pipeline of this spec, in the
    /// order it checks the spec: its depth and stencil tests, then its
    /// blend.
    pub fn build_refusal(&self) -> Option<GpuFailure> {
        self.depth_stencil_refusal().or_else(|| self.blend_refusal())
    }

    /// Why the Wgpu module refuses `reference`, a draw's stencil reference,
    /// through this pipeline, under that module's id and in its words: a
    /// reference given where no stencil test reads it.
    pub fn stencil_reference_refusal(&self, reference: Option<u32>) -> Option<GpuFailure> {
        (reference.is_some() && !self.tests_stencil()).then(|| GpuFailure {
            id: String::from("WgpuStencilReference"),
            text: String::from("a draw's stencil_reference sets the value a stencil test compares with and Replace writes, and this program has no stencil test"),
        })
    }
}

/// The pipeline a GpuSurface's draw draws with: one in the renderer's
/// table, or one built for the draw alone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum DrawPipeline {
    Table(u64),
    Inline(PipelineSpec),
}

/// The type of an index buffer's indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexFormat {
    Uint16,
    Uint32,
}

/// One draw of a GpuSurface, in the Wgpu module's Invoke-WgpuRender terms.
/// A vertex input or a binding is given a buffer of the renderer's table,
/// by number, or an array, which the renderer copies to the GPU as the type
/// the shader declares for it; a slot names a vertex input in location
/// order or by name, or a binding in group and binding order, by name or
/// as `group:binding`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfaceDraw {
    pub pipeline: DrawPipeline,
    pub vertices: Vec<(GpuSlot, GpuResource)>,
    /// An index buffer, the type of its indices, and how many to draw.
    pub index: Option<(u64, IndexFormat, u32)>,
    /// The vertices to draw; when absent, the index buffer's count, or else
    /// as many as the first vertex input's buffer or array has values for.
    pub vertex_count: Option<u32>,
    pub instance_count: u32,
    pub bindings: Vec<(GpuSlot, GpuResource)>,
    /// The color a blend factor of Constant reads, as red, green, blue and
    /// alpha.
    pub blend_constant: Option<[f64; 4]>,
    /// The value its pipeline's stencil test compares with and Replace
    /// writes; 0 when absent, and refused beside a pipeline with no stencil
    /// test.
    pub stencil_reference: Option<u32>,
}

/// When a GpuSurface's compute pass runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PassEvery {
    /// Before the draws of every frame.
    #[default]
    Frame,
    /// Before the draws of the first frame drawn with the surface's plan.
    Once,
}

/// One compute pass of a GpuSurface, in the Wgpu module's
/// Invoke-WgpuCompute terms, run before its draws in the frame's own
/// commands, so what it writes the draws read without leaving the GPU. A
/// binding is given as a draw's is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SurfacePass {
    pub source: String,
    pub language: ShaderLanguage,
    /// The entry point; the shader's only compute entry point when `None`.
    pub entry: Option<String>,
    /// Override constants by name, `@id` or GLSL `constant_id`.
    pub constants: Vec<(String, f64)>,
    pub bindings: Vec<(GpuSlot, GpuResource)>,
    pub dispatch: GpuDispatch,
    pub every: PassEvery,
}

/// A buffer, texture or sampler of the egui renderer's table, by number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TableObject {
    Buffer(u64),
    Texture(u64),
    Sampler(u64),
}

/// A resource of a draw or pass an HTML page makes: an object of the egui
/// renderer's table, of which the page has a copy under the table's
/// number, or an array's bytes sent with the surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PageResource {
    Table(TableObject),
    Bytes(Vec<u8>),
}

/// One vertex input of a draw an HTML page makes: its `@location`, the
/// WebGPU vertex format of its values, the bytes from one vertex's value to
/// the next, and its values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageVertex {
    pub location: u32,
    pub format: String,
    pub stride: u64,
    pub resource: PageResource,
}

/// One binding of a draw or pass an HTML page makes, at its group and
/// binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageBinding {
    pub group: u32,
    pub binding: u32,
    pub resource: PageResource,
}

/// One draw of a GpuSurface an HTML page makes, resolved by the module so
/// the page reads no shader: its pipeline's WGSL and how it draws, its
/// vertex inputs and bindings, whether its stages read the frame uniform
/// at `@group(1) @binding(0)`, which the page binds itself, and how many
/// vertices and instances it draws.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageDraw {
    pub spec: PipelineSpec,
    pub vertices: Vec<PageVertex>,
    pub bindings: Vec<PageBinding>,
    pub frame: bool,
    /// The index buffer and the type of its indices, of which
    /// `vertex_count` are drawn.
    pub index: Option<(PageResource, IndexFormat)>,
    pub vertex_count: u32,
    pub instance_count: u32,
    pub blend_constant: Option<[f64; 4]>,
    pub stencil_reference: Option<u32>,
}

/// One compute pass of a GpuSurface an HTML page runs on WebGPU: its WGSL,
/// its entry point and constants, its bindings, whether it reads the frame
/// uniform at `@group(1) @binding(0)`, and the workgroups it dispatches or
/// the buffer and byte an indirect dispatch reads them from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PagePass {
    pub source: String,
    pub entry: String,
    pub constants: Vec<(String, f64)>,
    pub bindings: Vec<PageBinding>,
    pub frame: bool,
    pub workgroups: [u32; 3],
    pub indirect: Option<(u64, u64)>,
    pub every: PassEvery,
}

/// What an HTML page does for a GpuSurface each frame, as
/// [`GpuOp::SetSurface`] gives the egui renderer: its passes, its clear
/// color, depth and stencil, its draws and the frame uniform's own fields
/// from byte 160.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageSurface {
    pub clear: [f64; 4],
    pub clear_depth: Option<f64>,
    pub clear_stencil: Option<u32>,
    pub draws: Vec<PageDraw>,
    pub passes: Vec<PagePass>,
    pub uniform: Vec<u8>,
}

/// What a compute run did, as the Wgpu module's ComputeResult reports it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComputeReport {
    /// The arrays the shader may have written, by group and then binding,
    /// each with its binding's name and the element type the shader
    /// declares for it.
    pub written: Vec<WrittenArray>,
    /// The workgroups dispatched in x, y and z; 0 for an indirect dispatch.
    pub workgroups: [u32; 3],
    /// Nanoseconds the GPU spent in the pass, when the run is timed and the
    /// device can measure them.
    pub gpu_time_ns: Option<f64>,
    /// Milliseconds the whole run took in the renderer.
    pub cpu_time_ms: f64,
}

/// A call on the renderer's GPU layer, replied to with
/// [`crate::FromHelper::GpuResult`]. Offsets and lengths are in bytes; the
/// session records each buffer's element type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GpuOp {
    /// A buffer with `bytes` in it, with `usage` or every usage but mapping
    /// and acceleration-structure input; the reply is
    /// [`GpuValue::Buffer`].
    NewBuffer { bytes: Vec<u8>, label: Option<String>, usage: Option<Vec<BufferUsage>> },
    /// A buffer of `length` zeroed bytes, with `usage` as for `NewBuffer`;
    /// the reply is [`GpuValue::Buffer`].
    NewZeroed { length: u64, label: Option<String>, usage: Option<Vec<BufferUsage>> },
    /// `length` bytes of `buffer` from byte `offset`, or to its end; the
    /// reply is [`GpuValue::Bytes`].
    Read { buffer: u64, offset: u64, length: Option<u64> },
    /// Writes `bytes` into `buffer` from byte `offset`.
    Write { buffer: u64, offset: u64, bytes: Vec<u8> },
    /// Takes `buffer` out of the renderer's table; its memory goes once no
    /// GpuSurface draws with it.
    Remove { buffer: u64 },
    /// Runs entry point `entry` (the shader's only compute entry point when
    /// `None`) of the compute shader `source`, with `constants` by name,
    /// `@id` or GLSL `constant_id`, each resource bound at its slot, over
    /// `dispatch`, with `immediates`, timing the pass when `timed`, as the
    /// Wgpu module's Invoke-WgpuCompute runs one; the reply is
    /// [`GpuValue::Computed`].
    Compute {
        source: String,
        language: ShaderLanguage,
        entry: Option<String>,
        constants: Vec<(String, f64)>,
        bindings: Vec<(GpuSlot, GpuResource)>,
        dispatch: GpuDispatch,
        immediates: Option<Vec<u8>>,
        timed: bool,
    },
    /// A render pipeline built from `spec` in the renderer's table, which a
    /// GpuSurface's draws name; the reply is [`GpuValue::RenderPipeline`].
    NewRenderPipeline { spec: PipelineSpec },
    /// Gives the GpuSurface named `name` in window `window` what each frame
    /// does: `passes` in order, then its texture cleared to `clear`, red,
    /// green, blue and alpha from 0 to 1, its depth attachment's depth to
    /// `clear_depth`, 1 when absent, and its stencil to `clear_stencil`, 0
    /// when absent, and `draws` in order, in place of what it did before;
    /// `uniform` is the frame uniform's own fields, from its byte 160. A
    /// clear for what the draws' depth attachment does not hold is refused.
    /// The reply, [`GpuValue::Done`], comes once the surface has them,
    /// every pass and draw having recorded once in the renderer.
    SetSurface {
        window: WindowId,
        name: String,
        clear: [f64; 4],
        clear_depth: Option<f64>,
        clear_stencil: Option<u32>,
        draws: Vec<SurfaceDraw>,
        passes: Vec<SurfacePass>,
        uniform: Vec<u8>,
    },
    /// The texels of the GpuSurface named `name` in window `window` once a
    /// frame of its window drawn after the call has painted them, so the
    /// draws given before the call are in them; the reply is
    /// [`GpuValue::Texels`], or a refusal when no frame draws the surface
    /// within 30 s.
    SaveSurface { window: WindowId, name: String },
    /// Changes the camera of the GpuSurface named `name` in window
    /// `window`, which fills its frame uniform's view and projection,
    /// starting from [`CameraSpec::default`] when it has none; the reply is
    /// [`GpuValue::Camera`] with the camera it now has, or a GuiGpuCamera
    /// refusal when the change cannot make one.
    SetCamera { window: WindowId, name: String, change: CameraChange },
    /// The camera of the GpuSurface named `name` in window `window`, where
    /// the person has moved it; the reply is [`GpuValue::Camera`].
    GetCamera { window: WindowId, name: String },
    /// Takes the camera from the GpuSurface named `name` in window
    /// `window`, so its frame uniform's view and projection are identity
    /// matrices again; the reply is [`GpuValue::Done`].
    RemoveCamera { window: WindowId, name: String },
    /// Changes the view of the Canvas named `name` in window `window`,
    /// starting from [`crate::view::ViewSpec::default`] when it has none, a fit made
    /// after the other parts and, before the canvas is first drawn, once it
    /// is; with an ease, the view moves to the change from the view drawn
    /// when it is made; the reply is [`GpuValue::CanvasView`] with the view
    /// as it is then drawn, or a GuiCanvasView refusal when the change
    /// cannot make one.
    SetCanvasView { window: WindowId, name: String, change: ViewChange },
    /// The view of the Canvas named `name` in window `window`, where the
    /// person has moved it and as a move under way draws it; the reply is
    /// [`GpuValue::CanvasView`].
    GetCanvasView { window: WindowId, name: String },
    /// Takes the view from the Canvas named `name` in window `window`, so
    /// its items' coordinates are the canvas's own and no gesture moves
    /// them; the reply is [`GpuValue::Done`].
    RemoveCanvasView { window: WindowId, name: String },
    /// A `width` by `height` texture of `format` in the renderer's table,
    /// filled with `texels`, rows packed, or zeroed; the reply is
    /// [`GpuValue::Texture`].
    NewTexture { width: u32, height: u32, format: TextureFormat, texels: Option<Vec<u8>>, label: Option<String> },
    /// A texture of `format`, `Rgba8UnormSrgb` or `Rgba8Unorm`, made from an
    /// image file's `bytes`, which the renderer decodes; the reply is
    /// [`GpuValue::Texture`].
    ImportImage { bytes: Vec<u8>, format: TextureFormat, label: Option<String> },
    /// Every texel of `texture`, rows packed; the reply is
    /// [`GpuValue::Bytes`].
    ReadTexture { texture: u64 },
    /// Writes every texel of `texture`, rows packed.
    WriteTexture { texture: u64, texels: Vec<u8> },
    /// Takes `texture` out of the renderer's table; its memory goes once no
    /// GpuSurface's plan uses it.
    RemoveTexture { texture: u64 },
    /// A sampler of `filter` and `address` in the renderer's table; the
    /// reply is [`GpuValue::Sampler`].
    NewSampler { filter: SamplerFilter, address: AddressMode, label: Option<String> },
    /// Gives the GpuSurface named `name` in window `window` of the HTML
    /// renderer what its page does each frame, as `SetSurface` gives the
    /// egui renderer's. The reply, [`GpuValue::Done`], comes once the page
    /// has it.
    SetPageSurface { window: WindowId, name: String, surface: PageSurface },
    /// Gives the HTML renderer `bytes` of buffer `buffer` of the egui
    /// renderer's table, a buffer of `length` bytes, from byte `offset`;
    /// the pages with a copy of the buffer take them. The reply is
    /// [`GpuValue::Done`].
    MirrorBuffer { buffer: u64, length: u64, offset: u64, bytes: Vec<u8> },
    /// Gives the HTML renderer every texel of texture `texture`, rows
    /// packed, with its size and format, as `MirrorBuffer` gives a buffer.
    MirrorTexture { texture: u64, width: u32, height: u32, format: TextureFormat, texels: Vec<u8> },
    /// Gives the HTML renderer sampler `sampler`'s filter and address mode,
    /// as `MirrorBuffer` gives a buffer.
    MirrorSampler { sampler: u64, filter: SamplerFilter, address: AddressMode },
    /// The HTML renderer lets go of its copy of `object`; the reply is
    /// [`GpuValue::Done`].
    ForgetMirror { object: TableObject },
    /// Destroys the renderer's device, as a driver reset loses one. The
    /// reply, [`GpuValue::Lost`], comes once the renderer goes on with a new
    /// device; otherwise it says why the device could not be destroyed.
    ResetDevice,
    /// Waits for the renderer's device to be lost. The reply,
    /// [`GpuValue::Lost`], comes once the renderer is on a new one. The
    /// renderer's UI thread sets the call aside and replies to it; it never
    /// runs on the GPU thread.
    WatchLoss,
}

/// The reply to a GPU-layer call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum GpuValue {
    /// A new buffer's number, and its length in bytes.
    Buffer { buffer: u64, length: u64 },
    Bytes(Vec<u8>),
    /// What a compute run did.
    Computed(ComputeReport),
    /// A new render pipeline's number, its vertex inputs in location order,
    /// and the bindings its stages use in group and binding order, by name.
    RenderPipeline { pipeline: u64, vertex_inputs: Vec<String>, bindings: Vec<String> },
    /// A GpuSurface's texels, red, green, blue and alpha bytes, rows top to
    /// bottom with no padding.
    Texels { width: u32, height: u32, rgba: Vec<u8> },
    /// The device was lost for `reason`, and the renderer is on a new one,
    /// its buffers filled again from what the session wrote into them.
    Lost { reason: String },
    /// A GpuSurface's camera; `None` for a surface without one.
    Camera(Option<CameraSpec>),
    /// A Canvas's view as drawn, and whether it is moving; `None` for a
    /// canvas without one.
    CanvasView(Option<ViewShown>),
    /// A new texture's number, its size and its format.
    Texture { texture: u64, width: u32, height: u32, format: TextureFormat },
    /// A new sampler's number.
    Sampler { sampler: u64 },
    Done,
}

/// An array a compute shader may have written.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WrittenArray {
    pub binding: String,
    pub element: ElementType,
    pub bytes: Vec<u8>,
}

/// Why a GPU-layer call failed: the error's id, as the Wgpu module or
/// PWRS-GUI's GPU layer names it, and its text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuFailure {
    pub id: String,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_element_type_has_its_size() {
        let sizes: Vec<u64> = [
            ElementType::Float32,
            ElementType::Float64,
            ElementType::Float16,
            ElementType::Int8,
            ElementType::UInt8,
            ElementType::Int16,
            ElementType::UInt16,
            ElementType::Int32,
            ElementType::UInt32,
            ElementType::Int64,
            ElementType::UInt64,
        ]
        .into_iter()
        .map(ElementType::size)
        .collect();
        assert_eq!(sizes, vec![4, 8, 2, 1, 1, 2, 2, 4, 4, 8, 8]);
    }

    #[test]
    fn every_element_type_is_found_by_its_name_in_any_case() {
        for t in ElementType::ALL {
            assert_eq!(ElementType::named(&t.name().to_ascii_lowercase()), Some(t));
        }
        assert_eq!(ElementType::named("Float128"), None);
    }

    #[test]
    fn a_compute_call_survives_the_wire() {
        let op = GpuOp::Compute {
            source: String::from("@compute @workgroup_size(1) fn main() {}"),
            language: ShaderLanguage::Wgsl,
            entry: None,
            constants: vec![(String::from("scale"), 2.0)],
            bindings: vec![(GpuSlot::Order(0), GpuResource::Buffer(7)), (GpuSlot::Name(String::from("0:1")), GpuResource::Array { element: ElementType::Int32, bytes: vec![1, 0, 0, 0] })],
            dispatch: GpuDispatch::Indirect { buffer: 7, offset: 16 },
            immediates: Some(vec![3, 0, 0, 0]),
            timed: true,
        };
        let bytes = postcard::to_stdvec(&op).expect("encodes");
        let back: GpuOp = postcard::from_bytes(&bytes).expect("decodes");
        assert_eq!(back, op);
    }

    #[test]
    fn a_surface_s_draws_survive_the_wire() {
        let spec = PipelineSpec {
            source: String::from("@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0); }"),
            vertex_entry: Some(String::from("vs")),
            fragment_entry: None,
            topology: Topology::LineStrip,
            cull: Cull::Back,
            blend: Blend::Custom,
            custom_blend: CustomBlend { source: Some(BlendFactor::SrcAlpha), alpha_operation: Some(BlendOperation::Max), ..CustomBlend::default() },
            depth: true,
            depth_format: Some(DepthFormat::Depth24PlusStencil8),
            depth_test: DepthTest { compare: Some(CompareFunction::Greater), write: Some(false), bias: Some(-2), bias_slope_scale: Some(1.5), bias_clamp: Some(0.25) },
            stencil: StencilTest {
                front: StencilFace { compare: Some(CompareFunction::Equal), pass: Some(StencilOperation::IncrementWrap), ..StencilFace::default() },
                back: StencilFace { fail: Some(StencilOperation::Invert), ..StencilFace::default() },
                read_mask: Some(0x0f),
                write_mask: None,
            },
            sample_count: 4,
            constants: vec![(String::from("scale"), 0.5)],
        };
        let draw = SurfaceDraw {
            pipeline: DrawPipeline::Inline(spec),
            vertices: vec![
                (GpuSlot::Name(String::from("position")), GpuResource::Buffer(3)),
                (GpuSlot::Order(1), GpuResource::Array { element: ElementType::Float64, bytes: 0.5f64.to_le_bytes().to_vec() }),
            ],
            index: Some((4, IndexFormat::Uint16, 6)),
            vertex_count: None,
            instance_count: 2,
            bindings: vec![(GpuSlot::Order(0), GpuResource::Buffer(5)), (GpuSlot::Name(String::from("1:0")), GpuResource::Array { element: ElementType::Int32, bytes: vec![2, 0, 0, 0] })],
            blend_constant: Some([0.25, 0.5, 0.75, 1.0]),
            stencil_reference: Some(7),
        };
        let plain = SurfaceDraw { pipeline: DrawPipeline::Table(2), vertices: Vec::new(), index: None, vertex_count: Some(3), instance_count: 1, bindings: Vec::new(), blend_constant: None, stencil_reference: None };
        let pass = SurfacePass {
            source: String::from("@compute @workgroup_size(64) fn main() {}"),
            language: ShaderLanguage::Wgsl,
            entry: None,
            constants: vec![(String::from("rate"), 0.5)],
            bindings: vec![(GpuSlot::Name(String::from("particles")), GpuResource::Buffer(6))],
            dispatch: GpuDispatch::Invocations([1000, 1, 1]),
            every: PassEvery::Once,
        };
        let op = GpuOp::SetSurface {
            window: 9,
            name: String::from("View"),
            clear: [0.0, 0.0, 0.0, 1.0],
            clear_depth: Some(0.0),
            clear_stencil: Some(3),
            draws: vec![draw, plain],
            passes: vec![pass],
            uniform: vec![0, 0, 128, 63],
        };
        let bytes = postcard::to_stdvec(&op).expect("encodes");
        let back: GpuOp = postcard::from_bytes(&bytes).expect("decodes");
        assert_eq!(back, op);
    }

    #[test]
    fn texture_calls_and_their_answers_survive_the_wire() {
        for op in [
            GpuOp::NewTexture { width: 2, height: 1, format: TextureFormat::Rgba16Float, texels: Some(vec![0; 16]), label: Some(String::from("albedo")) },
            GpuOp::ImportImage { bytes: vec![137, 80, 78, 71], format: TextureFormat::Rgba8UnormSrgb, label: None },
            GpuOp::ReadTexture { texture: 4 },
            GpuOp::WriteTexture { texture: 4, texels: vec![1, 2, 3, 4] },
            GpuOp::RemoveTexture { texture: 4 },
            GpuOp::NewSampler { filter: SamplerFilter::Nearest, address: AddressMode::MirrorRepeat, label: None },
            GpuOp::Compute {
                source: String::new(),
                language: ShaderLanguage::Wgsl,
                entry: None,
                constants: Vec::new(),
                bindings: vec![(GpuSlot::Order(0), GpuResource::Texture(4)), (GpuSlot::Order(1), GpuResource::Sampler(5))],
                dispatch: GpuDispatch::Auto,
                immediates: None,
                timed: false,
            },
        ] {
            let bytes = postcard::to_stdvec(&op).expect("encodes");
            let back: GpuOp = postcard::from_bytes(&bytes).expect("decodes");
            assert_eq!(back, op);
        }
        for value in [GpuValue::Texture { texture: 4, width: 2, height: 1, format: TextureFormat::R32Sint }, GpuValue::Sampler { sampler: 5 }] {
            let bytes = postcard::to_stdvec(&value).expect("encodes");
            let back: GpuValue = postcard::from_bytes(&bytes).expect("decodes");
            assert_eq!(back, value);
        }
    }

    #[test]
    fn a_page_s_surface_and_the_copies_of_table_objects_survive_the_wire() {
        let spec = PipelineSpec {
            source: String::from("@vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0); }"),
            vertex_entry: None,
            fragment_entry: None,
            topology: Topology::TriangleList,
            cull: Cull::None,
            blend: Blend::Alpha,
            custom_blend: CustomBlend::default(),
            depth: false,
            depth_format: Some(DepthFormat::Stencil8),
            depth_test: DepthTest::default(),
            stencil: StencilTest { front: StencilFace { pass: Some(StencilOperation::Replace), ..StencilFace::default() }, ..StencilTest::default() },
            sample_count: 1,
            constants: Vec::new(),
        };
        let draw = PageDraw {
            spec,
            vertices: vec![PageVertex { location: 0, format: String::from("float32x2"), stride: 8, resource: PageResource::Table(TableObject::Buffer(3)) }],
            bindings: vec![
                PageBinding { group: 0, binding: 0, resource: PageResource::Table(TableObject::Texture(4)) },
                PageBinding { group: 0, binding: 1, resource: PageResource::Table(TableObject::Sampler(5)) },
                PageBinding { group: 0, binding: 2, resource: PageResource::Bytes(vec![0, 0, 128, 63]) },
            ],
            frame: true,
            index: Some((PageResource::Bytes(vec![0, 0, 1, 0, 2, 0]), IndexFormat::Uint16)),
            vertex_count: 3,
            instance_count: 1,
            blend_constant: None,
            stencil_reference: Some(1),
        };
        let pass = PagePass {
            source: String::from("@compute @workgroup_size(64) fn main() {}"),
            entry: String::from("main"),
            constants: Vec::new(),
            bindings: vec![PageBinding { group: 0, binding: 0, resource: PageResource::Table(TableObject::Buffer(3)) }],
            frame: false,
            workgroups: [16, 1, 1],
            indirect: Some((7, 0)),
            every: PassEvery::Frame,
        };
        for op in [
            GpuOp::SetPageSurface {
                window: 1 << 62,
                name: String::from("View"),
                surface: PageSurface { clear: [0.0, 0.0, 0.0, 1.0], clear_depth: None, clear_stencil: Some(1), draws: vec![draw], passes: vec![pass], uniform: Vec::new() },
            },
            GpuOp::MirrorBuffer { buffer: 3, length: 64, offset: 16, bytes: vec![1; 8] },
            GpuOp::MirrorTexture { texture: 4, width: 1, height: 1, format: TextureFormat::Rgba8Unorm, texels: vec![255, 0, 0, 255] },
            GpuOp::MirrorSampler { sampler: 5, filter: SamplerFilter::Linear, address: AddressMode::Repeat },
            GpuOp::ForgetMirror { object: TableObject::Texture(4) },
        ] {
            let bytes = postcard::to_stdvec(&op).expect("encodes");
            let back: GpuOp = postcard::from_bytes(&bytes).expect("decodes");
            assert_eq!(back, op);
        }
    }

    #[test]
    fn camera_calls_and_their_answer_survive_the_wire() {
        let change = CameraChange { distance: Some(12.5), controls: Some(crate::camera::CameraControls::Blender), ..CameraChange::default() };
        for op in [
            GpuOp::SetCamera { window: 3, name: String::from("View"), change },
            GpuOp::GetCamera { window: 3, name: String::from("View") },
            GpuOp::RemoveCamera { window: 3, name: String::from("View") },
        ] {
            let bytes = postcard::to_stdvec(&op).expect("encodes");
            let back: GpuOp = postcard::from_bytes(&bytes).expect("decodes");
            assert_eq!(back, op);
        }
        for value in [GpuValue::Camera(Some(CameraSpec { yaw: 45.0, ..CameraSpec::default() })), GpuValue::Camera(None)] {
            let bytes = postcard::to_stdvec(&value).expect("encodes");
            let back: GpuValue = postcard::from_bytes(&bytes).expect("decodes");
            assert_eq!(back, value);
        }
    }

    #[test]
    fn canvas_view_calls_and_their_answer_survive_the_wire() {
        use crate::camera::CameraModifiers;
        use crate::canvas::Easing;
        use crate::view::{ViewControls, ViewEase, ViewFit, ViewGesture, ViewKey, ViewMove, ViewSpec};
        let keys = ViewControls::Custom(vec![(ViewGesture::Key { key: ViewKey::Letter(b'F'), modifiers: CameraModifiers::NONE }, ViewMove::Fit)]);
        let ease = ViewEase { seconds: 0.5, easing: Easing::EaseInOutCubic };
        let change = ViewChange { zoom: Some(2.0), controls: Some(keys), fit: Some(ViewFit::Items(vec![String::from("Box")])), ease: Some(ease), ..ViewChange::default() };
        for op in [
            GpuOp::SetCanvasView { window: 3, name: String::from("Board"), change },
            GpuOp::GetCanvasView { window: 3, name: String::from("Board") },
            GpuOp::RemoveCanvasView { window: 3, name: String::from("Board") },
        ] {
            let bytes = postcard::to_stdvec(&op).expect("encodes");
            let back: GpuOp = postcard::from_bytes(&bytes).expect("decodes");
            assert_eq!(back, op);
        }
        let shown = ViewShown { view: ViewSpec { center: Some([4.0, 5.0]), max_zoom: 8.0, ..ViewSpec::default() }, moving: true };
        for value in [GpuValue::CanvasView(Some(shown)), GpuValue::CanvasView(None)] {
            let bytes = postcard::to_stdvec(&value).expect("encodes");
            let back: GpuValue = postcard::from_bytes(&bytes).expect("decodes");
            assert_eq!(back, value);
        }
    }

    /// A spec of a full-screen shader drawing as New-WgpuRenderPipeline
    /// draws given nothing else.
    fn plain_spec() -> PipelineSpec {
        PipelineSpec {
            source: String::new(),
            vertex_entry: None,
            fragment_entry: None,
            topology: Topology::TriangleList,
            cull: Cull::None,
            blend: Blend::Replace,
            custom_blend: CustomBlend::default(),
            depth: false,
            depth_format: None,
            depth_test: DepthTest::default(),
            stencil: StencilTest::default(),
            sample_count: 1,
            constants: Vec::new(),
        }
    }

    #[test]
    fn each_depth_format_says_what_it_holds_under_the_wgpu_module_s_name() {
        let held: Vec<(&str, bool, bool)> = DepthFormat::ALL.into_iter().map(|f| (f.name(), f.has_depth(), f.has_stencil())).collect();
        assert_eq!(
            held,
            vec![
                ("Stencil8", false, true),
                ("Depth16Unorm", true, false),
                ("Depth24Plus", true, false),
                ("Depth24PlusStencil8", true, true),
                ("Depth32Float", true, false),
                ("Depth32FloatStencil8", true, true),
            ]
        );
    }

    #[test]
    fn a_stencil_test_fills_its_faces_as_the_wgpu_module_does_and_is_on_once_it_reads_or_changes_the_stencil() {
        let keep = StencilOperation::Keep;
        let none = StencilTest::default();
        assert_eq!(none.front_face(), (CompareFunction::Always, keep, keep, keep));
        assert_eq!(none.masks(), (u32::MAX, u32::MAX));
        assert!(!none.is_enabled(), "Always and Keep on both faces leave the stencil alone");
        let equal = StencilTest { front: StencilFace { compare: Some(CompareFunction::Equal), ..StencilFace::default() }, back: StencilFace { pass: Some(StencilOperation::Zero), ..StencilFace::default() }, ..none };
        assert_eq!(equal.back_face(), (CompareFunction::Equal, keep, keep, StencilOperation::Zero), "the back takes the front's compare and its own pass");
        assert!(equal.is_enabled());
        assert!(!StencilTest { read_mask: Some(0), write_mask: Some(0), ..equal }.is_enabled(), "no bit to read or change");
        assert!(StencilTest { back: StencilFace { compare: Some(CompareFunction::Never), ..StencilFace::default() }, ..none }.is_enabled(), "a back face of its own alone");
    }

    #[test]
    fn a_spec_s_depth_target_and_stencil_test_are_refused_as_the_wgpu_module_refuses_them() {
        let plain = plain_spec();
        assert_eq!(plain.depth_target(), None);
        assert_eq!(PipelineSpec { depth: true, ..plain_spec() }.depth_target(), Some(DepthFormat::Depth32Float));
        assert_eq!(PipelineSpec { depth: true, depth_format: Some(DepthFormat::Stencil8), ..plain_spec() }.depth_target(), Some(DepthFormat::Stencil8), "a format given wins over -Depth");
        let greater = DepthTest { compare: Some(CompareFunction::Greater), ..DepthTest::default() };
        let replace = StencilTest { front: StencilFace { pass: Some(StencilOperation::Replace), ..StencilFace::default() }, ..StencilTest::default() };
        let refusal = |spec: PipelineSpec| spec.depth_stencil_refusal().map(|f| (f.id, f.text));
        assert_eq!(refusal(plain_spec()), None);
        assert_eq!(
            refusal(PipelineSpec { depth_test: greater, ..plain_spec() }),
            Some((String::from("WgpuDepthTest"), String::from("-DepthCompare, -DepthWrite and the depth bias test depth, and there is no depth target: give -Depth or a -DepthFormat with depth")))
        );
        assert_eq!(
            refusal(PipelineSpec { depth_format: Some(DepthFormat::Stencil8), depth_test: DepthTest { write: Some(false), ..DepthTest::default() }, ..plain_spec() }).map(|r| r.1),
            Some(String::from("-DepthCompare, -DepthWrite and the depth bias test depth, and the depth target, Stencil8, has no depth aspect: give -Depth or a -DepthFormat with depth"))
        );
        assert_eq!(
            refusal(PipelineSpec { depth: true, stencil: replace, ..plain_spec() }),
            Some((
                String::from("WgpuStencil"),
                String::from("a stencil test needs a depth target with a stencil aspect, and the depth target, Depth32Float, has no stencil aspect: give -DepthFormat Stencil8, Depth24PlusStencil8 or Depth32FloatStencil8")
            ))
        );
        let biased = DepthTest { bias: Some(1), ..DepthTest::default() };
        assert_eq!(
            refusal(PipelineSpec { depth: true, depth_test: biased, topology: Topology::LineList, ..plain_spec() }),
            Some((String::from("WgpuDepthBias"), String::from("a depth bias moves triangles, and the topology is LineList")))
        );
        assert_eq!(refusal(PipelineSpec { depth: true, depth_test: DepthTest { bias: Some(0), ..DepthTest::default() }, topology: Topology::PointList, ..plain_spec() }), None, "a bias of 0 moves nothing");
        let stenciled = PipelineSpec { depth_format: Some(DepthFormat::Depth24PlusStencil8), stencil: replace, ..plain_spec() };
        assert_eq!(refusal(stenciled.clone()), None);
        assert!(stenciled.tests_stencil());
        assert!(!PipelineSpec { depth_format: Some(DepthFormat::Depth24PlusStencil8), ..plain_spec() }.tests_stencil(), "a format with stencil and no test");
    }

    #[test]
    fn a_custom_blend_s_parts_beside_another_blend_are_refused_as_the_wgpu_module_refuses_them_after_the_depth_tests() {
        let source = CustomBlend { source: Some(BlendFactor::One), ..CustomBlend::default() };
        let refusal = |spec: PipelineSpec| spec.build_refusal().map(|f| (f.id, f.text));
        assert_eq!(
            refusal(PipelineSpec { blend: Blend::Alpha, custom_blend: source, ..plain_spec() }),
            Some((
                String::from("WgpuBlend"),
                String::from("-BlendSource, -BlendDestination, -BlendOperation and their alpha forms name a Custom blend, and the blend is Alpha: give -Blend Custom")
            ))
        );
        let max = CustomBlend { alpha_operation: Some(BlendOperation::Max), ..CustomBlend::default() };
        assert!(refusal(PipelineSpec { custom_blend: max, ..plain_spec() }).is_some_and(|(_, text)| text.ends_with("the blend is Replace: give -Blend Custom")), "the default blend is Replace");
        assert_eq!(refusal(PipelineSpec { blend: Blend::Custom, custom_blend: source, ..plain_spec() }), None, "a Custom blend takes its parts");
        assert_eq!(refusal(PipelineSpec { blend: Blend::Alpha, ..plain_spec() }), None, "another blend without parts");
        let both = PipelineSpec { blend: Blend::Alpha, custom_blend: source, depth_test: DepthTest { compare: Some(CompareFunction::Greater), ..DepthTest::default() }, ..plain_spec() };
        assert_eq!(refusal(both).map(|(id, _)| id), Some(String::from("WgpuDepthTest")), "the depth tests are checked first, as the Wgpu module checks them");
    }
}
