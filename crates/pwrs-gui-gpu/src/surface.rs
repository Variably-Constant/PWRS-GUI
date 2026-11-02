//! Render pipelines in the layer's table, and the plans GPU surfaces draw
//! from them: each compute pass's program and bindings, and each draw's
//! pipeline, vertex buffers, index buffer and bindings, taken from the
//! table or copied from arrays, and checked by recording them once into
//! passes of the surface's attachments.

use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::{DEFAULT_USAGE, Dispatch, Failure, Language, Layer, Resource, Slot, binding_of, no_buffer, twice};
use wgpu_pwsh::{Binding, BlendMode, ComputeProgram, CullMode, CustomBlend, DepthTest, Draw, ElementType, Gpu, PrimitiveTopology, RenderOptions, RenderProgram, RunSpec, StencilTest, VertexInputInfo, wgpu};

/// The format of the texture a GPU surface draws into, which every
/// pipeline the layer builds draws into.
pub const SURFACE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// How a render pipeline draws, as the Wgpu module's New-WgpuRenderPipeline
/// takes it, into textures of [`SURFACE_FORMAT`].
#[derive(Clone, Debug, PartialEq)]
pub struct Pipeline {
    /// WGSL with a vertex and a fragment entry point.
    pub source: String,
    /// The vertex entry point; the shader's only one when `None`.
    pub vertex_entry: Option<String>,
    /// The fragment entry point; the shader's only one when `None`.
    pub fragment_entry: Option<String>,
    pub topology: PrimitiveTopology,
    pub cull: CullMode,
    pub blend: BlendMode,
    /// The factors and operations of a Custom blend, which any other blend
    /// refuses.
    pub custom_blend: CustomBlend,
    /// Whether it tests and writes depth in a Depth32Float attachment,
    /// when `depth_format` names no format.
    pub depth: bool,
    /// The format of its depth attachment, a depth or stencil format,
    /// which turns depth on.
    pub depth_format: Option<wgpu::TextureFormat>,
    /// How its depth attachment tests depth, which needs a format with a
    /// depth aspect.
    pub depth_test: DepthTest,
    /// How its depth attachment tests stencil, which needs a format with a
    /// stencil aspect.
    pub stencil: StencilTest,
    /// Samples per pixel, resolved into the surface's texture.
    pub sample_count: u32,
    /// Override constants by name or `@id`.
    pub constants: Vec<(String, f64)>,
}

impl Pipeline {
    fn options(&self) -> RenderOptions {
        RenderOptions {
            format: SURFACE_FORMAT,
            vertex_entry: self.vertex_entry.clone(),
            fragment_entry: self.fragment_entry.clone(),
            topology: self.topology,
            cull: self.cull,
            blend: self.blend,
            custom_blend: self.custom_blend,
            depth: self.depth,
            depth_format: self.depth_format,
            depth_test: self.depth_test,
            stencil: self.stencil,
            sample_count: self.sample_count,
            constants: self.constants.clone(),
        }
    }
}

/// What a pipeline the layer built reads: its vertex inputs in location
/// order and the bindings its stages use in group and binding order, by
/// name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineInfo {
    pub vertex_inputs: Vec<String>,
    pub bindings: Vec<String>,
}

/// A pipeline in the table: what it was built from, which builds it again
/// on a new device, and the program.
pub(crate) struct PipelineEntry {
    spec: Pipeline,
    program: RenderProgram,
}

fn build(gpu: &Gpu, spec: &Pipeline) -> Result<RenderProgram, Failure> {
    Ok(gpu.render_program(&spec.source, &spec.options())?)
}

fn no_pipeline(id: u64) -> Failure {
    Failure::new("GuiGpuNoPipeline", format!("there is no render pipeline {id}"))
}

/// The pipeline a surface's draw draws with: one in the table, or one
/// built for the draw alone from its own shader.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawPipeline {
    Table(u64),
    Inline(Pipeline),
}

/// The type of an index buffer's indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndexFormat {
    Uint16,
    Uint32,
}

/// One draw of a surface. A vertex input or a binding is given a buffer of
/// the table, by number, or an array the plan copies to the GPU as the type
/// the shader declares for it, or as it is for a binding of a struct; a
/// slot names a vertex input or a binding by its place, in location order
/// or in group and binding order, or by its name or `group:binding`.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceDraw {
    pub pipeline: DrawPipeline,
    pub vertices: Vec<(Slot, Resource)>,
    /// An index buffer, the type of its indices, and how many to draw.
    pub index: Option<(u64, IndexFormat, u32)>,
    /// The vertices to draw; when absent, the index buffer's count, or else
    /// as many as the first vertex input's buffer or array has values for.
    pub vertex_count: Option<u32>,
    pub instance_count: u32,
    pub bindings: Vec<(Slot, Resource)>,
    /// The color a blend factor of Constant reads.
    pub blend_constant: Option<[f64; 4]>,
    /// The value its pipeline's stencil test compares with and Replace
    /// writes; 0 when absent, whatever a draw before it set. A pipeline
    /// with no stencil test refuses one.
    pub stencil_reference: Option<u32>,
}

/// When a surface's compute pass runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Every {
    /// Before the draws of every frame.
    #[default]
    Frame,
    /// Before the draws of the first frame drawn with its plan.
    Once,
}

/// One compute pass of a surface, run before its draws in the frame's own
/// commands: a compute shader as [`Layer::compute`] takes one, its
/// bindings given as a draw's are.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfacePass {
    pub source: String,
    pub language: Language,
    /// The entry point; the shader's only compute entry point when `None`.
    pub entry: Option<String>,
    pub constants: Vec<(String, f64)>,
    pub bindings: Vec<(Slot, Resource)>,
    pub dispatch: Dispatch,
    pub every: Every,
}

/// What a surface does each frame: its compute passes in order, then the
/// color its texture is cleared to, the depth and stencil its depth
/// attachment is cleared to, 1 and 0 when absent, and its draws in order,
/// and the frame uniform's own fields, from its byte [`FRAME_HEADER`].
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceSet {
    pub clear: [f64; 4],
    pub clear_depth: Option<f64>,
    pub clear_stencil: Option<u32>,
    pub draws: Vec<SurfaceDraw>,
    pub passes: Vec<SurfacePass>,
    pub uniform: Vec<u8>,
}

/// The `@group` and `@binding` of a surface's frame uniform.
pub const FRAME_SLOT: (u32, u32) = (1, 0);

/// The bytes of a surface's frame uniform the renderer writes each frame,
/// ahead of the session's own fields.
pub const FRAME_HEADER: usize = 160;

const IDENTITY: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

/// What a surface's frame uniform contains ahead of the session's own fields,
/// as the renderer writes it each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameHeader {
    /// Seconds since the surface's first frame.
    pub time: f32,
    /// Seconds since its previous frame.
    pub delta: f32,
    /// Its size in physical pixels.
    pub resolution: [f32; 2],
    /// The pointer in physical pixels from its top left; -1, -1 while the
    /// pointer is outside it.
    pub pointer: [f32; 2],
    /// Whether its window draws in its dark theme.
    pub dark: bool,
    /// The display's pixels per point.
    pub scale: f32,
    /// The view matrix, column by column.
    pub view: [[f32; 4]; 4],
    /// The projection matrix, column by column.
    pub projection: [[f32; 4]; 4],
}

impl Default for FrameHeader {
    /// No time, no size, the pointer outside, the light theme at one pixel
    /// per point, and identity matrices.
    fn default() -> FrameHeader {
        FrameHeader { time: 0.0, delta: 0.0, resolution: [0.0; 2], pointer: [-1.0, -1.0], dark: false, scale: 1.0, view: IDENTITY, projection: IDENTITY }
    }
}

impl FrameHeader {
    /// The header's bytes in the uniform, WGSL's layout of `time: f32,
    /// delta: f32, resolution: vec2<f32>, pointer: vec2<f32>, dark: f32,
    /// scale: f32, view: mat4x4<f32>, projection: mat4x4<f32>`: bytes 0, 4,
    /// 8, 16, 24, 28, 32 and 96, `dark` 1 for the dark theme and 0 for the
    /// light. `dark` and `scale` stand where a struct that declares neither
    /// has the padding WGSL puts before `view`, so such a struct reads the
    /// same bytes as before.
    pub fn bytes(&self) -> [u8; FRAME_HEADER] {
        let mut out = [0u8; FRAME_HEADER];
        let mut put = |at: usize, value: f32| out[at..at + 4].copy_from_slice(&value.to_le_bytes());
        put(0, self.time);
        put(4, self.delta);
        put(8, self.resolution[0]);
        put(12, self.resolution[1]);
        put(16, self.pointer[0]);
        put(20, self.pointer[1]);
        put(24, if self.dark { 1.0 } else { 0.0 });
        put(28, self.scale);
        for (start, matrix) in [(32, &self.view), (96, &self.projection)] {
            for (column, values) in matrix.iter().enumerate() {
                for (row, value) in values.iter().enumerate() {
                    put(start + (column * 4 + row) * 4, *value);
                }
            }
        }
        out
    }
}

/// A surface's draws with everything they read, ready to record into a
/// pass each frame on any thread: the clear color, the samples per pixel
/// and depth format its attachments take, which its draws' pipelines
/// share, the depth and stencil its depth attachment is cleared to, each
/// draw, and the frame uniform when a draw reads it.
#[derive(Clone)]
pub struct SurfacePlan {
    pub clear: wgpu::Color,
    pub samples: u32,
    pub depth: Option<wgpu::TextureFormat>,
    pub clear_depth: f32,
    pub clear_stencil: u32,
    passes: Vec<PlannedPass>,
    draws: Vec<PlannedDraw>,
    frame: Option<wgpu::Buffer>,
}

/// How a planned compute pass dispatches, its indirect buffer taken from
/// the table when the plan was made.
#[derive(Clone)]
enum PlannedDispatch {
    Workgroups([u32; 3]),
    Invocations([u64; 3]),
    Indirect(wgpu::Buffer, u64),
    Auto,
}

#[derive(Clone)]
struct PlannedPass {
    program: ComputeProgram,
    bindings: Vec<(String, Bound)>,
    dispatch: PlannedDispatch,
    every: Every,
    /// Whether a frame has run the pass, shared by every copy of the plan,
    /// for a pass that runs once.
    ran: Arc<AtomicBool>,
}

impl PlannedPass {
    fn record(&self, pass: &mut wgpu::ComputePass<'_>) -> Result<(), Failure> {
        let bindings: Vec<(&str, Binding<'_>)> = self.bindings.iter().map(|(name, bound)| (name.as_str(), bound.binding())).collect();
        let dispatch = match &self.dispatch {
            PlannedDispatch::Workgroups(counts) => wgpu_pwsh::Dispatch::Workgroups(*counts),
            PlannedDispatch::Invocations(counts) => wgpu_pwsh::Dispatch::Invocations(*counts),
            PlannedDispatch::Indirect(buffer, offset) => wgpu_pwsh::Dispatch::Indirect { buffer, offset: *offset },
            PlannedDispatch::Auto => wgpu_pwsh::Dispatch::Auto,
        };
        self.program.record(pass, &RunSpec { bindings: &bindings, dispatch, immediates: None, timed: false })?;
        Ok(())
    }
}

/// What a planned pass or draw binds at one binding.
#[derive(Clone)]
enum Bound {
    Buffer(wgpu::Buffer),
    Texture(wgpu::Texture),
    Sampler(wgpu::Sampler),
}

impl Bound {
    fn binding(&self) -> Binding<'_> {
        match self {
            Bound::Buffer(buffer) => Binding::Buffer(buffer),
            Bound::Texture(texture) => Binding::Texture(texture),
            Bound::Sampler(sampler) => Binding::Sampler(sampler),
        }
    }
}

#[derive(Clone)]
struct PlannedDraw {
    program: RenderProgram,
    vertices: Vec<(String, wgpu::Buffer)>,
    index: Option<(wgpu::Buffer, wgpu::IndexFormat, u32)>,
    vertex_count: u32,
    instances: u32,
    bindings: Vec<(String, Bound)>,
    blend_constant: Option<wgpu::Color>,
    stencil_reference: Option<u32>,
}

impl PlannedDraw {
    /// Records the draw into `pass`. A draw given no stencil reference
    /// tests against 0, as a pass of its own would, so it never reads the
    /// reference a draw before it in the same pass set.
    fn record(&self, pass: &mut wgpu::RenderPass<'_>) -> Result<(), Failure> {
        let vertices: Vec<(&str, &wgpu::Buffer)> = self.vertices.iter().map(|(name, buffer)| (name.as_str(), buffer)).collect();
        let bindings: Vec<(&str, Binding<'_>)> = self.bindings.iter().map(|(name, bound)| (name.as_str(), bound.binding())).collect();
        if self.stencil_reference.is_none() {
            pass.set_stencil_reference(0);
        }
        let draw = Draw {
            vertex_count: self.vertex_count,
            instance_count: self.instances,
            vertices: &vertices,
            indices: self.index.as_ref().map(|(buffer, format, count)| (buffer, *format, *count)),
            bindings: &bindings,
            clear: None,
            blend_constant: self.blend_constant,
            clear_depth: None,
            clear_stencil: None,
            stencil_reference: self.stencil_reference,
        };
        Ok(self.program.record(pass, &draw)?)
    }
}

impl std::fmt::Debug for SurfacePlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfacePlan")
            .field("clear", &self.clear)
            .field("samples", &self.samples)
            .field("depth", &self.depth)
            .field("clear_depth", &self.clear_depth)
            .field("clear_stencil", &self.clear_stencil)
            .field("passes", &self.passes.len())
            .field("draws", &self.draws.len())
            .finish()
    }
}

impl SurfacePlan {
    /// The plan of a surface that only clears to `clear`.
    pub fn cleared(clear: [f64; 4]) -> SurfacePlan {
        SurfacePlan { clear: color(clear), samples: 1, depth: None, clear_depth: 1.0, clear_stencil: 0, passes: Vec::new(), draws: Vec::new(), frame: None }
    }

    /// The depth attachment of a pass drawing the plan, on `view`, a
    /// texture of [`SurfacePlan::depth`] and [`SurfacePlan::samples`]:
    /// its depth cleared to [`SurfacePlan::clear_depth`] and its stencil to
    /// [`SurfacePlan::clear_stencil`], each where its format has one, and
    /// stored.
    pub fn depth_attachment<'a>(&self, view: &'a wgpu::TextureView) -> wgpu::RenderPassDepthStencilAttachment<'a> {
        let has = |aspect: fn(&wgpu::TextureFormat) -> bool| self.depth.as_ref().is_some_and(aspect);
        wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: has(wgpu::TextureFormat::has_depth_aspect).then_some(wgpu::Operations { load: wgpu::LoadOp::Clear(self.clear_depth), store: wgpu::StoreOp::Store }),
            stencil_ops: has(wgpu::TextureFormat::has_stencil_aspect).then_some(wgpu::Operations { load: wgpu::LoadOp::Clear(self.clear_stencil), store: wgpu::StoreOp::Store }),
        }
    }

    /// Whether a pass or a draw reads the frame uniform, which changes every
    /// frame.
    pub fn reads_frame(&self) -> bool {
        self.frame.is_some()
    }

    /// Whether each frame can differ from the one before: a pass or a draw
    /// reads the frame uniform, or a pass runs every frame.
    pub fn animates(&self) -> bool {
        self.reads_frame() || self.passes.iter().any(|p| p.every == Every::Frame)
    }

    /// Records the compute passes the frame about to be drawn runs into
    /// `encoder`, ahead of the draws' pass: each that runs every frame, and
    /// each that runs once if no frame has run it.
    pub fn compute(&self, encoder: &mut wgpu::CommandEncoder) -> Result<(), Failure> {
        let due: Vec<&PlannedPass> = self.passes.iter().filter(|p| p.every == Every::Frame || !p.ran.swap(true, Ordering::AcqRel)).collect();
        record_passes(encoder, &due)
    }

    /// Writes `header` at the start of the frame uniform through `queue`,
    /// for the frame about to be recorded; nothing when no draw reads it.
    pub fn write_frame(&self, queue: &wgpu::Queue, header: &FrameHeader) {
        if let Some(buffer) = &self.frame {
            queue.write_buffer(buffer, 0, &header.bytes());
        }
    }

    /// Records each draw into `pass`, a pass whose attachments are of
    /// [`SURFACE_FORMAT`], [`SurfacePlan::samples`] and
    /// [`SurfacePlan::depth`]; the pass's loads clear them, its depth
    /// attachment's as [`SurfacePlan::depth_attachment`] gives them.
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>) -> Result<(), Failure> {
        self.draws.iter().try_for_each(|d| d.record(pass))
    }
}

/// Records `passes` in order into one compute pass of `encoder`; nothing
/// when there are none.
fn record_passes(encoder: &mut wgpu::CommandEncoder, passes: &[&PlannedPass]) -> Result<(), Failure> {
    if passes.is_empty() {
        return Ok(());
    }
    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("PWRS-GUI GpuSurface passes"), timestamp_writes: None });
    passes.iter().try_for_each(|p| p.record(&mut pass))
}

fn color([r, g, b, a]: [f64; 4]) -> wgpu::Color {
    wgpu::Color { r, g, b, a }
}

/// What a surface's depth attachment of `format`, none for draws without
/// one, starts each frame at: `depth`, from 0 to 1, 1 when absent, and
/// `stencil`, 0 when absent. A start given for what the format does not
/// hold is refused, as the Wgpu module's Invoke-WgpuRender refuses one.
fn clears(format: Option<wgpu::TextureFormat>, depth: Option<f64>, stencil: Option<u32>) -> Result<(f32, u32), Failure> {
    let lacking = |aspect: &str| match format {
        Some(f) => format!("the draws' depth target, {f:?}, has no {aspect} aspect"),
        None => String::from("the draws' pipelines have no depth target"),
    };
    if let Some(value) = depth {
        if !(0.0..=1.0).contains(&value) {
            return Err(Failure::new("GuiGpuClearDepth", format!("-ClearDepth is a depth from 0 to 1, and {value} is not")));
        }
        if !format.is_some_and(|f| f.has_depth_aspect()) {
            return Err(Failure::new("GuiGpuClearDepth", format!("-ClearDepth sets the depth the depth target starts at, and {}", lacking("depth"))));
        }
    }
    if stencil.is_some() && !format.is_some_and(|f| f.has_stencil_aspect()) {
        return Err(Failure::new("GuiGpuClearStencil", format!("-ClearStencil sets the value the stencil starts at, and {}", lacking("stencil"))));
    }
    Ok((depth.unwrap_or(1.0) as f32, stencil.unwrap_or(0)))
}

/// The vertex input `slot` names of `inputs`, a shader's in location order:
/// the one at its place, or the one of its name, without regard to case.
fn vertex_input<'a>(slot: &Slot, inputs: &'a [VertexInputInfo]) -> Result<&'a VertexInputInfo, Failure> {
    let found = match slot {
        Slot::Order(at) => inputs.get(*at),
        Slot::Name(name) => inputs.iter().find(|i| i.name.eq_ignore_ascii_case(name)),
    };
    found.ok_or_else(|| {
        let given = match slot {
            Slot::Order(at) => format!("vertex input {at} in order"),
            Slot::Name(name) => format!("vertex input {name}"),
        };
        let has = match inputs.len() {
            0 => String::from("none"),
            n => format!("{n}: {}", inputs.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", ")),
        };
        Failure::new("GuiGpuVertexInput", format!("{given} was given, and the shader has {has}"))
    })
}

impl Layer {
    /// Builds a render pipeline from `spec` and puts it in the table:
    /// returns its number and what it reads.
    pub fn new_pipeline(&mut self, spec: Pipeline) -> Result<(u64, PipelineInfo), Failure> {
        let program = build(&self.gpu, &spec)?;
        let info = PipelineInfo { vertex_inputs: program.vertex_inputs(), bindings: program.bindings() };
        let id = self.next;
        self.next += 1;
        self.pipelines.insert(id, PipelineEntry { spec, program });
        Ok((id, info))
    }

    /// Removes render pipeline `id` from the table.
    pub fn free_pipeline(&mut self, id: u64) -> Result<(), Failure> {
        self.pipelines.remove(&id).map(|_| ()).ok_or_else(|| no_pipeline(id))
    }

    /// The numbers of the render pipelines in the table, in ascending order.
    pub fn pipelines(&self) -> Vec<u64> {
        self.pipelines.keys().copied().collect()
    }

    /// Builds the table's render pipelines again on the layer's device.
    pub(crate) fn rebuild_pipelines(&mut self) -> Result<(), Failure> {
        for entry in self.pipelines.values_mut() {
            entry.program = build(&self.gpu, &entry.spec)?;
        }
        Ok(())
    }

    /// The buffer `resource` gives a draw, and its length in bytes: a buffer
    /// of the table, at the length the session gave it, or an array copied
    /// to the GPU as `declared`, or as its own type when that is `None`,
    /// into a buffer labeled `label` that belongs to the plan alone.
    fn given_buffer(&self, resource: &Resource, declared: Option<ElementType>, label: &str) -> Result<(wgpu::Buffer, u64), Failure> {
        match resource {
            Resource::Buffer(id) => {
                let entry = self.buffers.get(id).ok_or_else(|| no_buffer(*id))?;
                Ok((entry.buffer.clone(), entry.length))
            }
            Resource::Array { element, bytes } => {
                let data = element.convert(bytes, declared.unwrap_or(*element));
                Ok((self.gpu.create_buffer(&data, DEFAULT_USAGE, label)?, data.len() as u64))
            }
            Resource::Texture(_) | Resource::Sampler(_) => Err(Failure::new("GuiGpuVertex", String::from("a vertex input takes a buffer or an array; a texture or a sampler goes to a binding"))),
        }
    }

    /// What `resource` binds at a binding: a buffer as [`Layer::given_buffer`]
    /// gives one, or a texture or a sampler of the table.
    fn given_bound(&self, resource: &Resource, declared: Option<ElementType>, label: &str) -> Result<Bound, Failure> {
        match resource {
            Resource::Texture(id) => Ok(Bound::Texture(self.table_texture(*id)?.clone())),
            Resource::Sampler(id) => Ok(Bound::Sampler(self.table_sampler(*id)?.clone())),
            Resource::Buffer(_) | Resource::Array { .. } => Ok(Bound::Buffer(self.given_buffer(resource, declared, label)?.0)),
        }
    }

    /// A surface's frame uniform: the header of no frame yet, then `own`,
    /// the session's fields, then zeros up to a multiple of 16 bytes.
    fn frame_buffer(&self, own: &[u8]) -> Result<wgpu::Buffer, Failure> {
        let mut bytes = FrameHeader::default().bytes().to_vec();
        bytes.extend_from_slice(own);
        bytes.resize(bytes.len().next_multiple_of(16), 0);
        Ok(self.gpu.create_buffer(&bytes, DEFAULT_USAGE, "PWRS-GUI GpuSurface frame")?)
    }

    /// The plan of the draws `set` gives a surface, each draw's pipeline
    /// taken from the table or built for it alone, its vertex inputs and
    /// bindings given buffers of the table or copies of arrays, and every
    /// draw recorded once into a pass of the surface's attachments,
    /// unsubmitted, so a draw wgpu refuses is refused here and not at each
    /// frame. The draws' pipelines must share their samples per pixel and
    /// depth format, which the surface's attachments take, and a depth or
    /// stencil clear needs a format that holds it. A pass or a draw whose
    /// shaders declare a binding at [`FRAME_SLOT`] reads the surface's
    /// frame uniform there, one buffer every such pass and draw shares.
    /// Each pass's program is built and its bindings given as a draw's
    /// are, and the passes are recorded once ahead of the draws.
    pub fn plan(&self, set: SurfaceSet) -> Result<SurfacePlan, Failure> {
        let SurfaceSet { clear, clear_depth, clear_stencil, draws: given, passes: asked, uniform } = set;
        let mut draws = Vec::with_capacity(given.len());
        let mut targets: Option<(u32, Option<wgpu::TextureFormat>, usize)> = None;
        let mut frame: Option<wgpu::Buffer> = None;
        let mut passes = Vec::with_capacity(asked.len());
        for (at, pass) in asked.into_iter().enumerate() {
            let program = self.compute_program(&pass.source, pass.language, pass.entry.as_deref(), &pass.constants)?;
            let declared = program.binding_info();
            let mut bindings: Vec<(String, Bound)> = Vec::with_capacity(pass.bindings.len() + 1);
            for (slot, resource) in &pass.bindings {
                let info = binding_of(slot, &declared)?;
                if (info.group, info.binding) == FRAME_SLOT {
                    let text = format!("pass {at}'s binding {} (1:0) is the surface's frame uniform, which the renderer fills each frame; its own fields, from byte {FRAME_HEADER}, are given with -Uniform", info.name);
                    return Err(Failure::new("GuiGpuBinding", text));
                }
                let key = format!("{}:{}", info.group, info.binding);
                if bindings.iter().any(|(given, _)| *given == key) {
                    return Err(twice(info));
                }
                bindings.push((key, self.given_bound(resource, info.element_type, &format!("PWRS-GUI pass {at} {} array", info.name))?));
            }
            if declared.iter().any(|b| (b.group, b.binding) == FRAME_SLOT) {
                if frame.is_none() {
                    frame = Some(self.frame_buffer(&uniform)?);
                }
                if let Some(shared) = &frame {
                    bindings.push((format!("{}:{}", FRAME_SLOT.0, FRAME_SLOT.1), Bound::Buffer(shared.clone())));
                }
            }
            let dispatch = match pass.dispatch {
                Dispatch::Workgroups(counts) => PlannedDispatch::Workgroups(counts),
                Dispatch::Invocations(counts) => PlannedDispatch::Invocations(counts),
                Dispatch::Indirect { buffer, offset } => PlannedDispatch::Indirect(self.buffers.get(&buffer).ok_or_else(|| no_buffer(buffer))?.buffer.clone(), offset),
                Dispatch::Auto => PlannedDispatch::Auto,
            };
            passes.push(PlannedPass { program, bindings, dispatch, every: pass.every, ran: Arc::new(AtomicBool::new(false)) });
        }
        for (at, draw) in given.into_iter().enumerate() {
            let program = match draw.pipeline {
                DrawPipeline::Table(id) => self.pipelines.get(&id).ok_or_else(|| no_pipeline(id))?.program.clone(),
                DrawPipeline::Inline(spec) => build(&self.gpu, &spec)?,
            };
            let these = (program.sample_count(), program.depth_format());
            match targets {
                None => targets = Some((these.0, these.1, at)),
                Some((samples, depth, first)) if (samples, depth) != these => {
                    let depth_of = |d: Option<wgpu::TextureFormat>| d.map_or_else(|| String::from("no depth"), |f| format!("{f:?} depth"));
                    let text = format!("draw {at}'s pipeline draws with {} samples and {}, and draw {first}'s with {samples} and {}; a surface's draws share its attachments", these.0, depth_of(these.1), depth_of(depth));
                    return Err(Failure::new("GuiGpuSurfaceTargets", text));
                }
                Some(_) => {}
            }
            let inputs = program.vertex_input_info();
            let mut vertices: Vec<(String, wgpu::Buffer)> = Vec::with_capacity(draw.vertices.len());
            let mut first = None;
            for (slot, resource) in &draw.vertices {
                let info = vertex_input(slot, &inputs)?;
                if vertices.iter().any(|(name, _)| *name == info.name) {
                    return Err(Failure::new("GuiGpuVertexInput", format!("vertex input {} was given twice", info.name)));
                }
                let (buffer, length) = self.given_buffer(resource, Some(info.element_type), &format!("PWRS-GUI {} vertices", info.name))?;
                if inputs.first().is_some_and(|f| f.location == info.location) {
                    first = Some((info, length));
                }
                vertices.push((info.name.clone(), buffer));
            }
            let declared = program.binding_info();
            let mut bindings: Vec<(String, Bound)> = Vec::with_capacity(draw.bindings.len());
            for (slot, resource) in &draw.bindings {
                let info = binding_of(slot, &declared)?;
                if (info.group, info.binding) == FRAME_SLOT {
                    let text = format!("binding {} (1:0) is the surface's frame uniform, which the renderer fills each frame; its own fields, from byte {FRAME_HEADER}, are given with -Uniform", info.name);
                    return Err(Failure::new("GuiGpuBinding", text));
                }
                let key = format!("{}:{}", info.group, info.binding);
                if bindings.iter().any(|(given, _)| *given == key) {
                    return Err(twice(info));
                }
                bindings.push((key, self.given_bound(resource, info.element_type, &format!("PWRS-GUI {} array", info.name))?));
            }
            if declared.iter().any(|b| (b.group, b.binding) == FRAME_SLOT) {
                if frame.is_none() {
                    frame = Some(self.frame_buffer(&uniform)?);
                }
                if let Some(shared) = &frame {
                    bindings.push((format!("{}:{}", FRAME_SLOT.0, FRAME_SLOT.1), Bound::Buffer(shared.clone())));
                }
            }
            let index = match draw.index {
                Some((id, format, count)) => {
                    let format = match format {
                        IndexFormat::Uint16 => wgpu::IndexFormat::Uint16,
                        IndexFormat::Uint32 => wgpu::IndexFormat::Uint32,
                    };
                    Some((self.buffers.get(&id).map(|e| e.buffer.clone()).ok_or_else(|| no_buffer(id))?, format, count))
                }
                None => None,
            };
            let vertex_count = match (draw.vertex_count, &index, first) {
                (Some(n), _, _) => n,
                (None, Some((_, _, count)), _) => *count,
                (None, None, Some((info, length))) => {
                    let count = length / (u64::from(info.components) * info.element_type.size());
                    u32::try_from(count).map_err(|e| Failure::new("GuiGpuVertexCount", format!("draw {at}'s {} has values for {count} vertices, more than a draw takes: {e}", info.name)))?
                }
                (None, None, None) => return Err(Failure::new("GuiGpuVertexCount", format!("draw {at} gives no vertex count, no index buffer and no values for a first vertex input; give -VertexCount"))),
            };
            draws.push(PlannedDraw {
                program,
                vertices,
                index,
                vertex_count,
                instances: draw.instance_count,
                bindings,
                blend_constant: draw.blend_constant.map(color),
                stencil_reference: draw.stencil_reference,
            });
        }
        if frame.is_none() && !uniform.is_empty() {
            return Err(Failure::new("GuiGpuUniform", String::from("-Uniform was given, and no pass or draw reads the frame uniform at @group(1) @binding(0)")));
        }
        let (samples, depth) = targets.map_or((1, None), |(samples, depth, _)| (samples, depth));
        let (clear_depth, clear_stencil) = clears(depth, clear_depth, clear_stencil)?;
        let plan = SurfacePlan { clear: color(clear), samples, depth, clear_depth, clear_stencil, passes, draws, frame };
        self.check(&plan)?;
        Ok(plan)
    }

    /// Records `plan`'s passes, each of them, and its draws into a pass of
    /// one-pixel attachments of its format, samples and depth, and finishes
    /// the commands without submitting them, in a validation error scope of
    /// this thread's, so what wgpu refuses in the passes or the draws is
    /// refused here with wgpu's reason.
    fn check(&self, plan: &SurfacePlan) -> Result<(), Failure> {
        let device = self.gpu.device();
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let attachment = |format: wgpu::TextureFormat, samples: u32, label: &str| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let color = attachment(SURFACE_FORMAT, plan.samples, "PWRS-GUI surface check color");
        let resolve = (plan.samples > 1).then(|| attachment(SURFACE_FORMAT, 1, "PWRS-GUI surface check resolve"));
        let depth = plan.depth.map(|format| attachment(format, plan.samples, "PWRS-GUI surface check depth"));
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("PWRS-GUI surface check") });
        let computed = record_passes(&mut encoder, &plan.passes.iter().collect::<Vec<_>>());
        let recorded = {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("PWRS-GUI surface check"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color,
                    depth_slice: None,
                    resolve_target: resolve.as_ref(),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(plan.clear), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: depth.as_ref().map(|view| plan.depth_attachment(view)),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            plan.record(&mut pass)
        };
        drop(encoder.finish());
        let mut popped = std::pin::pin!(scope.pop());
        let caught = match popped.as_mut().poll(&mut std::task::Context::from_waker(std::task::Waker::noop())) {
            std::task::Poll::Ready(caught) => caught,
            std::task::Poll::Pending => return Err(Failure::new("GuiGpuDraw", String::from("the device did not say whether the surface's draws record"))),
        };
        computed?;
        recorded?;
        match caught {
            Some(error) => Err(Failure::new("GuiGpuDraw", format!("the surface's passes and draws do not record: {error}"))),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompareFunction, DEFAULT_USAGE, GpuOptions, StencilFace, StencilOperation};

    fn layer() -> Layer {
        let gpu = Gpu::open(&GpuOptions::default()).expect("a device opens");
        Layer::adopt(gpu.adapter().clone(), gpu.device().clone(), gpu.queue().clone(), "PWRS-GUI surface test").expect("the layer takes the device")
    }

    const FULLSCREEN: &str = "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
            return vec4<f32>(corners[i], 0.0, 1.0);
        }
        @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }";

    const POSITIONED: &str = "struct Tint { color: vec4<f32> }
        @group(0) @binding(0) var<uniform> tint: Tint;
        @vertex fn vs(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(position, 0.0, 1.0); }
        @fragment fn fs() -> @location(0) vec4<f32> { return tint.color; }";

    fn spec(source: &str) -> Pipeline {
        Pipeline {
            source: source.to_string(),
            vertex_entry: None,
            fragment_entry: None,
            topology: PrimitiveTopology::TriangleList,
            cull: CullMode::None,
            blend: BlendMode::Replace,
            custom_blend: CustomBlend::default(),
            depth: false,
            depth_format: None,
            depth_test: DepthTest::default(),
            stencil: StencilTest::default(),
            sample_count: 1,
            constants: Vec::new(),
        }
    }

    fn floats(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn draw(pipeline: DrawPipeline) -> SurfaceDraw {
        SurfaceDraw { pipeline, vertices: Vec::new(), index: None, vertex_count: Some(3), instance_count: 1, bindings: Vec::new(), blend_constant: None, stencil_reference: None }
    }

    /// What a surface given `draws` and `passes` does, cleared to `clear`
    /// with no depth or stencil clear of its own and no uniform.
    fn set(clear: [f64; 4], passes: Vec<SurfacePass>, draws: Vec<SurfaceDraw>) -> SurfaceSet {
        SurfaceSet { clear, clear_depth: None, clear_stencil: None, uniform: Vec::new(), passes, draws }
    }

    /// Draws `plan` into a `size` by `size` texture as a surface does, its
    /// attachments of the plan's samples and depth, and reads it back.
    fn drawn(layer: &Layer, plan: &SurfacePlan, size: u32) -> Vec<u8> {
        let gpu = layer.gpu();
        let device = gpu.device();
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let target = gpu.create_texture(size, size, SURFACE_FORMAT, Some(usage), "drawn").expect("the target");
        let attachment = |format: wgpu::TextureFormat| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("attachment"),
                    size: wgpu::Extent3d { width: size, height: size, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: plan.samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let resolved = target.create_view(&wgpu::TextureViewDescriptor::default());
        let multisampled = (plan.samples > 1).then(|| attachment(SURFACE_FORMAT));
        let depth = plan.depth.map(attachment);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("drawn") });
        plan.compute(&mut encoder).expect("the passes record");
        {
            let (view, resolve_target) = match &multisampled {
                Some(m) => (m, Some(&resolved)),
                None => (&resolved, None),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("drawn"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(plan.clear), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: depth.as_ref().map(|view| plan.depth_attachment(view)),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            plan.record(&mut pass).expect("the plan records");
        }
        gpu.queue().submit([encoder.finish()]);
        assert!(layer.take_errors().is_empty(), "the frame raised no error");
        gpu.read_texture(&target).expect("the texels")
    }

    fn every_texel(texels: &[u8], rgba: [u8; 4]) -> bool {
        texels.as_chunks::<4>().0.iter().all(|t| *t == rgba)
    }

    const WRITES_GREEN: &str = "@group(0) @binding(0) var<storage, read_write> tint: array<f32, 4>;
        @compute @workgroup_size(1) fn main() { tint[0] = 0.0; tint[1] = 1.0; tint[2] = 0.0; tint[3] = 1.0; }";

    const READS_TINT: &str = "@group(0) @binding(0) var<storage, read> tint: array<f32, 4>;
        @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
            return vec4<f32>(corners[i], 0.0, 1.0);
        }
        @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(tint[0], tint[1], tint[2], tint[3]); }";

    const COUNTS: &str = "@group(0) @binding(0) var<storage, read_write> count: array<u32, 1>;
        @compute @workgroup_size(1) fn main() { count[0] = count[0] + 1u; }";

    const KEEPS_TIME: &str = "struct Frame { time: f32, delta: f32, resolution: vec2<f32>, pointer: vec2<f32>, view: mat4x4<f32>, projection: mat4x4<f32> }
        @group(1) @binding(0) var<uniform> frame: Frame;
        @group(0) @binding(0) var<storage, read_write> kept: array<f32, 1>;
        @compute @workgroup_size(1) fn main() { kept[0] = frame.time; }";

    const SAMPLES: &str = "@group(2) @binding(0) var picture: texture_2d<f32>;
        @group(2) @binding(1) var look: sampler;
        @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
            return vec4<f32>(corners[i], 0.0, 1.0);
        }
        @fragment fn fs() -> @location(0) vec4<f32> { return textureSample(picture, look, vec2<f32>(0.5, 0.5)); }";

    #[test]
    fn a_draw_samples_a_texture_of_the_table_through_a_sampler_bound_by_name() {
        let mut layer = layer();
        let picture = layer.new_texture(1, 1, crate::TextureFormat::Rgba8Unorm, Some(&[0, 255, 0, 255]), None).expect("a texture");
        let look = layer.new_sampler(crate::SamplerSpec::default(), None).expect("a sampler");
        let bindings = vec![(Slot::Name(String::from("picture")), Resource::Texture(picture)), (Slot::Name(String::from("LOOK")), Resource::Sampler(look))];
        let sampled = SurfaceDraw { bindings, ..draw(DrawPipeline::Inline(spec(SAMPLES))) };
        let plan = layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![sampled] }).expect("the plan");
        assert!(every_texel(&drawn(&layer, &plan, 2), [0, 255, 0, 255]));
        layer.free_texture(picture).expect("freed");
        assert!(every_texel(&drawn(&layer, &plan, 2), [0, 255, 0, 255]), "the plan still has the texture it draws with");
        let as_vertices = SurfaceDraw { vertices: vec![(Slot::Order(0), Resource::Texture(picture))], ..draw(DrawPipeline::Inline(spec(POSITIONED))) };
        let refused = layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![as_vertices] }).expect_err("a texture feeds no vertices");
        assert_eq!(refused.id, "GuiGpuVertex");
    }

    /// A pass of `source` over table buffer `buffer`, one workgroup.
    fn pass(source: &str, buffer: u64, every: Every) -> SurfacePass {
        SurfacePass { source: source.to_string(), language: Language::Wgsl, entry: None, constants: Vec::new(), bindings: vec![(Slot::Order(0), Resource::Buffer(buffer))], dispatch: Dispatch::Workgroups([1, 1, 1]), every }
    }

    #[test]
    fn a_pass_writes_what_the_draws_read_in_the_same_frame() {
        let mut layer = layer();
        let tint = layer.zeros(16, None, None).expect("a buffer");
        let reads = SurfaceDraw { bindings: vec![(Slot::Order(0), Resource::Buffer(tint))], ..draw(DrawPipeline::Inline(spec(READS_TINT))) };
        let plan = layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: vec![pass(WRITES_GREEN, tint, Every::Frame)], draws: vec![reads] }).expect("the plan");
        assert!(plan.animates(), "a pass every frame draws every frame");
        assert!(every_texel(&drawn(&layer, &plan, 2), [0, 255, 0, 255]), "the draw reads what the pass wrote before it");
    }

    #[test]
    fn a_pass_given_once_runs_in_the_first_frame_alone_and_one_given_every_frame_in_each() {
        let mut layer = layer();
        let (once, each) = (layer.zeros(4, None, None).expect("a buffer"), layer.zeros(4, None, None).expect("a buffer"));
        let plan = layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: vec![pass(COUNTS, once, Every::Once), pass(COUNTS, each, Every::Frame)], draws: Vec::new() }).expect("the plan");
        for _ in 0..3 {
            drawn(&layer, &plan, 1);
        }
        let count = |id| u32::from_le_bytes(layer.download(id, 0, None).expect("read").try_into().expect("four bytes"));
        assert_eq!((count(once), count(each)), (1, 3), "the plan's check recorded the passes without running them");
        drawn(&layer, &plan.clone(), 1);
        assert_eq!(count(once), 1, "a copy of the plan shares whether its passes ran");
        let only_once = layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: vec![pass(COUNTS, once, Every::Once)], draws: Vec::new() }).expect("the plan");
        assert!(!only_once.animates(), "a pass run once requests no frame after the first");
    }

    #[test]
    fn a_pass_reads_the_frame_uniform_it_declares_and_refuses_a_binding_in_its_place() {
        let mut layer = layer();
        let kept = layer.zeros(4, None, None).expect("a buffer");
        let plan = layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: vec![pass(KEEPS_TIME, kept, Every::Frame)], draws: Vec::new() }).expect("the plan");
        assert!(plan.reads_frame());
        plan.write_frame(layer.gpu().queue(), &FrameHeader { time: 2.5, ..FrameHeader::default() });
        drawn(&layer, &plan, 1);
        assert_eq!(f32::from_le_bytes(layer.download(kept, 0, None).expect("read").try_into().expect("four bytes")), 2.5);
        let given = SurfacePass { bindings: vec![(Slot::Order(0), Resource::Buffer(kept)), (Slot::Name(String::from("frame")), Resource::Buffer(kept))], ..pass(KEEPS_TIME, kept, Every::Frame) };
        let refused = layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: vec![given], draws: Vec::new() }).expect_err("the frame uniform's slot is the renderer's");
        assert_eq!(refused.id, "GuiGpuBinding");
        assert!(refused.text.starts_with("pass 0"), "{}", refused.text);
    }

    #[test]
    fn a_pipeline_in_the_table_says_what_it_reads_and_is_freed_once() {
        let mut layer = layer();
        let (id, info) = layer.new_pipeline(spec(POSITIONED)).expect("the pipeline builds");
        assert_eq!(info, PipelineInfo { vertex_inputs: vec![String::from("position")], bindings: vec![String::from("tint")] });
        assert_eq!(layer.pipelines(), vec![id]);
        let buffer = layer.upload(&[1, 2, 3, 4], None, None).expect("uploads");
        assert!(buffer > id, "pipelines and buffers are numbered from one count: {id} then {buffer}");
        layer.free_pipeline(id).expect("frees");
        assert_eq!(layer.free_pipeline(id).expect_err("freed twice").id, "GuiGpuNoPipeline");
        let broken = layer.new_pipeline(spec("@vertex fn vs( {")).expect_err("a shader that does not parse is refused");
        assert_eq!(broken.id, "WgpuShaderInvalid");
    }

    #[test]
    fn a_plan_draws_its_pipelines_with_their_buffers_and_bindings() {
        let mut layer = layer();
        let (table, _) = layer.new_pipeline(spec(FULLSCREEN)).expect("the pipeline builds");
        let red = layer.plan(SurfaceSet { clear: [0.0, 0.0, 1.0, 1.0], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![draw(DrawPipeline::Table(table))] }).expect("the plan");
        assert!(every_texel(&drawn(&layer, &red, 4), [255, 0, 0, 255]), "a full-screen triangle covers every texel");
        let cleared = drawn(&layer, &SurfacePlan::cleared([0.0, 0.0, 1.0, 1.0]), 4);
        assert!(every_texel(&cleared, [0, 0, 255, 255]), "a surface with no draws shows its clear color");
        let left_half = layer.upload(&floats(&[-1.0, -1.0, 0.0, -1.0, -1.0, 3.0]), Some("left half"), None).expect("the vertices");
        let green = layer.upload(&floats(&[0.0, 1.0, 0.0, 1.0]), Some("green"), None).expect("the tint");
        let tinted = SurfaceDraw {
            vertices: vec![(Slot::Order(0), Resource::Buffer(left_half))],
            bindings: vec![(Slot::Name(String::from("TINT")), Resource::Buffer(green))],
            ..draw(DrawPipeline::Inline(spec(POSITIONED)))
        };
        let plan = layer.plan(SurfaceSet { clear: [0.0, 0.0, 0.0, 1.0], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![tinted] }).expect("the plan");
        let texels = drawn(&layer, &plan, 2);
        assert_eq!(texels, [0, 255, 0, 255, 0, 0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 255], "the left column green, the right the clear color");
    }

    #[test]
    fn a_plan_keeps_drawing_with_a_pipeline_and_buffers_freed_after_it_was_made() {
        let mut layer = layer();
        let (pipeline, _) = layer.new_pipeline(spec(POSITIONED)).expect("the pipeline builds");
        let left_half = layer.upload(&floats(&[-1.0, -1.0, 0.0, -1.0, -1.0, 3.0]), None, None).expect("the vertices");
        let green = layer.upload(&floats(&[0.0, 1.0, 0.0, 1.0]), None, None).expect("the tint");
        let tinted = SurfaceDraw { vertices: vec![(Slot::Order(0), Resource::Buffer(left_half))], bindings: vec![(Slot::Order(0), Resource::Buffer(green))], ..draw(DrawPipeline::Table(pipeline)) };
        let plan = layer.plan(SurfaceSet { clear: [0.0, 0.0, 0.0, 1.0], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![tinted.clone()] }).expect("the plan");
        layer.free_pipeline(pipeline).expect("frees the pipeline");
        layer.free(left_half).expect("frees the vertices");
        layer.free(green).expect("frees the tint");
        let texels = drawn(&layer, &plan, 2);
        assert_eq!(texels, [0, 255, 0, 255, 0, 0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 255], "the plan drew with what it was made with");
        let again = layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![tinted] }).expect_err("a new plan finds none of them");
        assert_eq!(again.id, "GuiGpuNoPipeline");
    }

    #[test]
    fn a_plan_draws_with_the_samples_and_depth_its_pipelines_share() {
        let mut layer = layer();
        let deep = Pipeline { sample_count: 4, depth: true, ..spec(FULLSCREEN) };
        let (id, _) = layer.new_pipeline(deep.clone()).expect("the pipeline builds");
        let plan = layer.plan(SurfaceSet { clear: [0.0, 0.0, 0.0, 1.0], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![draw(DrawPipeline::Table(id)), draw(DrawPipeline::Inline(deep))] }).expect("the plan");
        assert_eq!((plan.samples, plan.depth), (4, Some(wgpu::TextureFormat::Depth32Float)));
        assert!(every_texel(&drawn(&layer, &plan, 4), [255, 0, 0, 255]), "drawn multisampled and resolved");
        let mixed = layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![draw(DrawPipeline::Table(id)), draw(DrawPipeline::Inline(spec(FULLSCREEN)))] });
        let mixed = mixed.expect_err("draws whose pipelines take other attachments are refused");
        assert_eq!(mixed.id, "GuiGpuSurfaceTargets");
        assert!(mixed.text.contains("draw 1's pipeline draws with 1 samples and no depth, and draw 0's with 4 and Depth32Float depth"), "{}", mixed.text);
        let formats = layer.plan(set([0.0; 4], Vec::new(), vec![draw(DrawPipeline::Inline(Pipeline { depth: true, ..spec(FULLSCREEN) })), draw(DrawPipeline::Inline(Pipeline { depth_format: Some(wgpu::TextureFormat::Depth24Plus), ..spec(FULLSCREEN) }))]));
        let formats = formats.expect_err("draws whose depth formats differ are refused");
        assert!(formats.text.contains("draw 1's pipeline draws with 1 samples and Depth24Plus depth, and draw 0's with 1 and Depth32Float depth"), "{}", formats.text);
    }

    /// A triangle over the whole surface at the depth its constant `depth`
    /// names, red at `red` 1 and blue at `red` 0.
    const DEEP: &str = "override depth: f32 = 0.5;
        override red: f32 = 1.0;
        @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
            return vec4<f32>(corners[i], depth, 1.0);
        }
        @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(red, 0.0, 1.0 - red, 1.0); }";

    /// A green triangle over the left column of a surface two texels wide.
    const LEFT_HALF: &str = "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(0.0, -1.0), vec2<f32>(-1.0, 3.0));
            return vec4<f32>(corners[i], 0.0, 1.0);
        }
        @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.0, 1.0, 0.0, 1.0); }";

    /// A draw of [`DEEP`] at `depth` in red or blue, through a pipeline of
    /// depth format `format` testing as `test` does.
    fn deep(format: wgpu::TextureFormat, test: DepthTest, depth: f64, red: f64) -> SurfaceDraw {
        let constants = vec![(String::from("depth"), depth), (String::from("red"), red)];
        draw(DrawPipeline::Inline(Pipeline { depth_format: Some(format), depth_test: test, constants, ..spec(DEEP) }))
    }

    /// The texels of a surface two texels square whose left column is
    /// `left` and whose right is `right`.
    fn columns(left: [u8; 4], right: [u8; 4]) -> Vec<u8> {
        [left, right, left, right].concat()
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];

    #[test]
    fn a_plan_tests_reversed_depth_against_the_depth_it_clears_to_in_every_depth_format() {
        let layer = layer();
        let greater = DepthTest { compare: Some(CompareFunction::Greater), ..DepthTest::default() };
        let mut formats = vec![wgpu::TextureFormat::Depth16Unorm, wgpu::TextureFormat::Depth24Plus, wgpu::TextureFormat::Depth24PlusStencil8, wgpu::TextureFormat::Depth32Float];
        if layer.gpu().device().features().contains(wgpu::Features::DEPTH32FLOAT_STENCIL8) {
            formats.push(wgpu::TextureFormat::Depth32FloatStencil8);
        }
        for format in formats {
            let near_then_far = |test: DepthTest| vec![deep(format, test, 0.2, 1.0), deep(format, test, 0.8, 0.0)];
            let reversed = layer.plan(SurfaceSet { clear_depth: Some(0.0), ..set([0.0, 1.0, 0.0, 1.0], Vec::new(), near_then_far(greater)) }).expect("the reversed plan");
            assert_eq!((reversed.depth, reversed.clear_depth), (Some(format), 0.0));
            assert!(every_texel(&drawn(&layer, &reversed, 2), BLUE), "{format:?}: Greater from 0 draws the far triangle over the near one");
            let unmet = layer.plan(set([0.0, 1.0, 0.0, 1.0], Vec::new(), near_then_far(greater))).expect("the plan");
            assert_eq!(unmet.clear_depth, 1.0, "depth starts at 1 when no clear is given");
            assert!(every_texel(&drawn(&layer, &unmet, 2), GREEN), "{format:?}: no depth is greater than 1");
            let plain = layer.plan(set([0.0, 1.0, 0.0, 1.0], Vec::new(), near_then_far(DepthTest::default()))).expect("the plan");
            assert!(every_texel(&drawn(&layer, &plain, 2), RED), "{format:?}: Less keeps the near triangle");
        }
    }

    #[test]
    fn a_fragment_that_passes_without_writing_depth_lets_a_farther_one_draw_over_it() {
        let layer = layer();
        let format = wgpu::TextureFormat::Depth32Float;
        let unwritten = DepthTest { write: Some(false), ..DepthTest::default() };
        let written = layer.plan(set([0.0; 4], Vec::new(), vec![deep(format, DepthTest::default(), 0.2, 1.0), deep(format, DepthTest::default(), 0.8, 0.0)])).expect("the plan");
        assert!(every_texel(&drawn(&layer, &written, 2), RED));
        let plan = layer.plan(set([0.0; 4], Vec::new(), vec![deep(format, unwritten, 0.2, 1.0), deep(format, DepthTest::default(), 0.8, 0.0)])).expect("the plan");
        assert!(every_texel(&drawn(&layer, &plan, 2), BLUE), "the near triangle left the depth at 1");
    }

    #[test]
    fn a_depth_bias_moves_a_triangle_toward_the_viewer_as_far_as_its_clamp_lets_it() {
        let layer = layer();
        let format = wgpu::TextureFormat::Depth32Float;
        let behind = |test: DepthTest| layer.plan(set([0.0; 4], Vec::new(), vec![deep(format, DepthTest::default(), 0.5, 0.0), deep(format, test, 0.50001, 1.0)])).expect("the plan");
        assert!(every_texel(&drawn(&layer, &behind(DepthTest::default()), 2), BLUE), "the red triangle lies behind the blue");
        let biased = DepthTest { bias: Some(-1000), ..DepthTest::default() };
        assert!(every_texel(&drawn(&layer, &behind(biased), 2), RED), "1000 of the format's steps toward the viewer bring it in front");
        let clamped = DepthTest { bias_clamp: Some(-0.000_001), ..biased };
        assert!(every_texel(&drawn(&layer, &behind(clamped), 2), BLUE), "a clamp of a millionth keeps it behind");
    }

    /// A draw of `source` through a pipeline testing `stencil` in a Stencil8
    /// attachment, with `reference`.
    fn stenciled(source: &str, stencil: StencilTest, reference: Option<u32>) -> SurfaceDraw {
        SurfaceDraw { stencil_reference: reference, ..draw(DrawPipeline::Inline(Pipeline { depth_format: Some(wgpu::TextureFormat::Stencil8), stencil, ..spec(source) })) }
    }

    #[test]
    fn a_draw_without_a_stencil_reference_tests_against_0_and_not_the_one_a_draw_before_it_set() {
        let layer = layer();
        let replace = StencilTest { front: StencilFace { pass: Some(StencilOperation::Replace), ..StencilFace::default() }, ..StencilTest::default() };
        let equal = StencilTest { front: StencilFace { compare: Some(CompareFunction::Equal), ..StencilFace::default() }, ..StencilTest::default() };
        let frame = |masked: Option<u32>, clear_stencil: Option<u32>| {
            let draws = vec![stenciled(LEFT_HALF, replace, Some(1)), stenciled(FULLSCREEN, equal, masked)];
            let plan = layer.plan(SurfaceSet { clear_stencil, ..set([0.0, 0.0, 1.0, 1.0], Vec::new(), draws) }).expect("the plan");
            drawn(&layer, &plan, 2)
        };
        assert_eq!(frame(Some(1), None), columns(RED, BLUE), "the second draw kept to where the first wrote 1");
        assert_eq!(frame(None, None), columns(GREEN, RED), "with no reference of its own it tests 0, not the first draw's 1");
        assert_eq!(frame(Some(1), Some(1)), columns(RED, RED), "the stencil starts at 1 everywhere");
        let unread = SurfaceDraw { stencil_reference: Some(1), ..draw(DrawPipeline::Inline(Pipeline { depth: true, ..spec(FULLSCREEN) })) };
        let refused = layer.plan(set([0.0; 4], Vec::new(), vec![unread])).expect_err("a reference no stencil test reads");
        assert_eq!(refused.id, "WgpuStencilReference");
    }

    #[test]
    fn a_triangle_facing_away_takes_the_back_face_s_stencil_test() {
        let mut layer = layer();
        let faces = "@vertex fn vs(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(position, 0.0, 1.0); }
            @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }";
        let triangles = layer.upload(&floats(&[-1.0, -1.0, 0.0, -1.0, -1.0, 3.0, 0.0, -1.0, 0.0, 3.0, 2.0, -1.0]), Some("facing and turned away"), None).expect("the vertices");
        let tested = |front: CompareFunction, back: CompareFunction| {
            let stencil = StencilTest { front: StencilFace { compare: Some(front), ..StencilFace::default() }, back: StencilFace { compare: Some(back), ..StencilFace::default() }, ..StencilTest::default() };
            let two = SurfaceDraw { vertices: vec![(Slot::Order(0), Resource::Buffer(triangles))], vertex_count: Some(6), ..stenciled(faces, stencil, None) };
            let plan = layer.plan(set([0.0, 0.0, 1.0, 1.0], Vec::new(), vec![two])).expect("the plan");
            drawn(&layer, &plan, 2)
        };
        assert_eq!(tested(CompareFunction::Never, CompareFunction::Always), columns(BLUE, RED), "the left triangle faces the viewer, the right faces away");
        assert_eq!(tested(CompareFunction::Always, CompareFunction::Never), columns(RED, BLUE));
    }

    #[test]
    fn a_plan_refuses_a_clear_its_depth_target_cannot_hold() {
        let layer = layer();
        let refused = |format: Option<wgpu::TextureFormat>, clear_depth: Option<f64>, clear_stencil: Option<u32>| {
            let plain = draw(DrawPipeline::Inline(Pipeline { depth_format: format, ..spec(FULLSCREEN) }));
            let f = layer.plan(SurfaceSet { clear_depth, clear_stencil, ..set([0.0; 4], Vec::new(), vec![plain]) }).expect_err("refused");
            (f.id, f.text)
        };
        assert_eq!(refused(None, Some(0.5), None), (String::from("GuiGpuClearDepth"), String::from("-ClearDepth sets the depth the depth target starts at, and the draws' pipelines have no depth target")));
        assert_eq!(refused(Some(wgpu::TextureFormat::Depth32Float), Some(1.5), None), (String::from("GuiGpuClearDepth"), String::from("-ClearDepth is a depth from 0 to 1, and 1.5 is not")));
        assert_eq!(
            refused(Some(wgpu::TextureFormat::Stencil8), Some(0.0), None),
            (String::from("GuiGpuClearDepth"), String::from("-ClearDepth sets the depth the depth target starts at, and the draws' depth target, Stencil8, has no depth aspect"))
        );
        assert_eq!(
            refused(Some(wgpu::TextureFormat::Depth32Float), None, Some(1)),
            (String::from("GuiGpuClearStencil"), String::from("-ClearStencil sets the value the stencil starts at, and the draws' depth target, Depth32Float, has no stencil aspect"))
        );
        let stencil_only = layer.plan(SurfaceSet { clear_stencil: Some(255), ..set([0.0; 4], Vec::new(), vec![draw(DrawPipeline::Inline(Pipeline { depth_format: Some(wgpu::TextureFormat::Stencil8), ..spec(FULLSCREEN) }))]) });
        assert_eq!(stencil_only.expect("a stencil clear for a format that holds stencil").clear_stencil, 255);
    }

    #[test]
    fn a_plan_refuses_what_its_draws_cannot_record_when_it_is_made() {
        let mut layer = layer();
        let (id, _) = layer.new_pipeline(spec(POSITIONED)).expect("the pipeline builds");
        let vertices = layer.upload(&floats(&[-1.0, -1.0, 3.0, -1.0, -1.0, 3.0]), None, None).expect("the vertices");
        let tint = layer.upload(&floats(&[1.0, 1.0, 1.0, 1.0]), None, None).expect("the tint");
        let unmarked = layer.upload(&floats(&[-1.0, -1.0, 3.0, -1.0, -1.0, 3.0]), None, Some(wgpu::BufferUsages::STORAGE)).expect("a buffer without vertex use");
        let table = |given: Vec<(Slot, u64)>| -> Vec<(Slot, Resource)> { given.into_iter().map(|(slot, buffer)| (slot, Resource::Buffer(buffer))).collect() };
        let given = |vertices: Vec<(Slot, u64)>, bindings: Vec<(Slot, u64)>| SurfaceDraw { vertices: table(vertices), bindings: table(bindings), ..draw(DrawPipeline::Table(id)) };
        let refused = |draw: SurfaceDraw| layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![draw] }).expect_err("refused");
        let whole = given(vec![(Slot::Order(0), vertices)], vec![(Slot::Order(0), tint)]);
        layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![whole.clone()] }).expect("a draw with all it reads records");
        assert_eq!(refused(SurfaceDraw { pipeline: DrawPipeline::Table(999), ..whole.clone() }).id, "GuiGpuNoPipeline");
        assert_eq!(refused(given(vec![(Slot::Order(0), 999)], vec![(Slot::Order(0), tint)])).id, "GuiGpuNoBuffer");
        let past = refused(given(vec![(Slot::Order(1), vertices)], vec![(Slot::Order(0), tint)]));
        assert_eq!(past.id, "GuiGpuVertexInput");
        assert!(past.text.contains("vertex input 1 in order was given, and the shader has 1: position"), "{}", past.text);
        assert_eq!(refused(given(vec![(Slot::Order(0), vertices)], Vec::new())).id, "WgpuBindingMissing");
        assert_eq!(refused(given(Vec::new(), vec![(Slot::Order(0), tint)])).id, "WgpuNoVertices");
        let named_twice = refused(given(vec![(Slot::Order(0), vertices), (Slot::Name(String::from("POSITION")), vertices)], vec![(Slot::Order(0), tint)]));
        assert_eq!((named_twice.id.as_str(), named_twice.text.as_str()), ("GuiGpuVertexInput", "vertex input position was given twice"));
        assert_eq!(refused(given(vec![(Slot::Order(0), vertices)], vec![(Slot::Order(0), tint), (Slot::Name(String::from("0:0")), tint)])).id, "GuiGpuBinding");
        let counted = layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![SurfaceDraw { vertex_count: None, ..whole.clone() }] }).expect("a draw with no count takes its first vertex input's");
        assert_eq!(counted.draws[0].vertex_count, 3, "six f32 for a vec2<f32> input are three vertices");
        let nothing = refused(SurfaceDraw { vertex_count: None, ..draw(DrawPipeline::Inline(spec(FULLSCREEN))) });
        assert_eq!(nothing.id, "GuiGpuVertexCount", "a draw with no vertex input, count or index buffer has nothing to count");
        let wrong = refused(given(vec![(Slot::Order(0), unmarked)], vec![(Slot::Order(0), tint)]));
        assert_eq!(wrong.id, "GuiGpuDraw");
        assert!(wrong.text.starts_with("the surface's passes and draws do not record: "), "{}", wrong.text);
        assert!(layer.take_errors().is_empty(), "the check's own error is caught in its scope, not left for the next call");
        assert!(DEFAULT_USAGE.contains(wgpu::BufferUsages::VERTEX), "a buffer made with no usage named can feed vertices");
    }

    #[test]
    fn a_plan_copies_arrays_as_the_types_its_shader_declares_and_counts_their_vertices() {
        let layer = layer();
        let typed = "@group(0) @binding(0) var<uniform> tint: vec4<f32>;
            @vertex fn vs(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(position, 0.0, 1.0); }
            @fragment fn fs() -> @location(0) vec4<f32> { return tint; }";
        let doubles = |values: &[f64]| -> Vec<u8> { values.iter().flat_map(|v| v.to_le_bytes()).collect() };
        let arrays = SurfaceDraw {
            vertices: vec![(Slot::Name(String::from("position")), Resource::Array { element: ElementType::Float64, bytes: doubles(&[-1.0, -1.0, 0.0, -1.0, -1.0, 3.0]) })],
            bindings: vec![(Slot::Order(0), Resource::Array { element: ElementType::Float64, bytes: doubles(&[0.0, 1.0, 0.0, 1.0]) })],
            vertex_count: None,
            ..draw(DrawPipeline::Inline(spec(typed)))
        };
        let plan = layer.plan(SurfaceSet { clear: [0.0, 0.0, 0.0, 1.0], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![arrays] }).expect("the plan");
        assert_eq!(plan.draws[0].vertex_count, 3, "six values for a vec2<f32> input");
        let green_left = [0, 255, 0, 255, 0, 0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 255];
        assert_eq!(drawn(&layer, &plan, 2), green_left, "the doubles went to the GPU as the f32 the shader declares");
        assert!(layer.buffers().is_empty(), "an array's copy belongs to the plan, not the table");
        let structured = SurfaceDraw {
            vertices: vec![(Slot::Order(0), Resource::Array { element: ElementType::Float32, bytes: floats(&[-1.0, -1.0, 0.0, -1.0, -1.0, 3.0]) })],
            bindings: vec![(Slot::Order(0), Resource::Array { element: ElementType::Float32, bytes: floats(&[0.0, 1.0, 0.0, 1.0]) })],
            vertex_count: None,
            ..draw(DrawPipeline::Inline(spec(POSITIONED)))
        };
        let plan = layer.plan(SurfaceSet { clear: [0.0, 0.0, 0.0, 1.0], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![structured] }).expect("an array for a binding of a struct goes as it is");
        assert_eq!(drawn(&layer, &plan, 2), green_left);
    }

    /// Covers the surface reading the frame uniform: red the time, green
    /// the session's tint's, blue whether the pointer is outside, alpha the
    /// projection's last element.
    const FRAMED: &str = "struct Frame { time: f32, delta: f32, resolution: vec2<f32>, pointer: vec2<f32>, view: mat4x4<f32>, projection: mat4x4<f32>, tint: vec4<f32> }
        @group(1) @binding(0) var<uniform> frame: Frame;
        @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
            return vec4<f32>(corners[i], 0.0, 1.0);
        }
        @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(frame.time, frame.tint.g, select(0.0, 1.0, frame.pointer.x < 0.0), frame.projection[3][3]); }";

    #[test]
    fn a_frame_header_lays_out_as_the_wgsl_struct_does() {
        let header = FrameHeader { time: 1.5, delta: 0.25, resolution: [640.0, 480.0], pointer: [10.0, 20.0], dark: true, scale: 1.25, ..FrameHeader::default() };
        let bytes = header.bytes();
        let at = |offset: usize| f32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("four bytes"));
        assert_eq!([at(0), at(4), at(8), at(12), at(16), at(20)], [1.5, 0.25, 640.0, 480.0, 10.0, 20.0]);
        assert_eq!([at(24), at(28)], [1.0, 1.25], "the dark theme and the pixels per point, where a struct without them has the padding before view");
        let light = FrameHeader::default().bytes();
        assert_eq!(f32::from_le_bytes(light[24..28].try_into().expect("four bytes")), 0.0, "the light theme is 0");
        let diagonal = |start: usize| [at(start), at(start + 20), at(start + 40), at(start + 60)];
        assert_eq!(diagonal(32), [1.0; 4], "view is identity without a camera");
        assert_eq!(diagonal(96), [1.0; 4], "and so is projection");
        assert_eq!(at(36), 0.0, "off the diagonal");
        assert_eq!(FrameHeader::default().pointer, [-1.0, -1.0], "the pointer starts outside");
    }

    #[test]
    fn a_draw_that_declares_the_frame_uniform_reads_the_header_and_the_sessions_fields() {
        let layer = layer();
        let set = |uniform: Vec<u8>, draws: Vec<SurfaceDraw>| SurfaceSet { clear: [0.0, 0.0, 0.0, 1.0], clear_depth: None, clear_stencil: None, draws, passes: Vec::new(), uniform };
        let plan = layer.plan(set(floats(&[0.0, 1.0, 0.0, 1.0]), vec![draw(DrawPipeline::Inline(spec(FRAMED)))])).expect("the plan");
        assert!(plan.reads_frame());
        assert!(every_texel(&drawn(&layer, &plan, 2), [0, 255, 255, 255]), "before a frame is written: no time, the tint from byte 160, the pointer outside, identity projection");
        plan.write_frame(layer.gpu().queue(), &FrameHeader { time: 1.0, ..FrameHeader::default() });
        assert!(every_texel(&drawn(&layer, &plan, 2), [255, 255, 255, 255]), "the header written for a frame");
        let plain = layer.plan(set(Vec::new(), vec![draw(DrawPipeline::Inline(spec(FULLSCREEN)))])).expect("a draw that reads no frame");
        assert!(!plain.reads_frame());
        let unread = layer.plan(set(vec![0; 16], vec![draw(DrawPipeline::Inline(spec(FULLSCREEN)))])).expect_err("a uniform no draw reads");
        assert_eq!(unread.id, "GuiGpuUniform");
        let given = SurfaceDraw { bindings: vec![(Slot::Name(String::from("1:0")), Resource::Array { element: ElementType::Float32, bytes: floats(&[0.0; 44]) })], ..draw(DrawPipeline::Inline(spec(FRAMED))) };
        let taken = layer.plan(set(Vec::new(), vec![given])).expect_err("a binding given for the frame uniform");
        assert_eq!(taken.id, "GuiGpuBinding");
        assert!(taken.text.contains("is the surface's frame uniform"), "{}", taken.text);
    }

    #[test]
    fn a_rebuilt_layer_keeps_its_pipelines_under_their_numbers() {
        let mut layer = layer();
        let (id, _) = layer.new_pipeline(spec(FULLSCREEN)).expect("the pipeline builds");
        let other = Gpu::open(&GpuOptions::default()).expect("a second device opens");
        layer.rebuild(other.adapter().clone(), other.device().clone(), other.queue().clone()).expect("rebuilds");
        assert_eq!(layer.pipelines(), vec![id]);
        let plan = layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, uniform: Vec::new(),passes: Vec::new(), draws: vec![draw(DrawPipeline::Table(id))] }).expect("the plan on the new device");
        assert!(every_texel(&drawn(&layer, &plan, 2), [255, 0, 0, 255]));
    }
}
