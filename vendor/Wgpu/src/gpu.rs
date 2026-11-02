//! The Rust library: a GPU device, and the buffers, textures and WGSL
//! passes made on it, for Rust code on any thread. The cmdlets run on the
//! same code. A crate takes the library with `default-features = false`,
//! which leaves the PowerShell module out, and names the `static-dxc`
//! feature to keep DXC on Windows x64.

use std::num::NonZeroU64;
use std::sync::Arc;
use std::time::Instant;

use pwrs::prelude::*;

use crate::binding::{self, Bound, Resource};
use crate::compute;
use crate::device::{self, AdapterQuery, CoreRef, InstanceOptions, OpenOptions};
use crate::element::ElementType;
use crate::names::Backend;
use crate::pipeline::{self, ComputeInner};
use crate::render::{self, DrawCall, RenderInner, RenderSpec};
pub use crate::render::{BlendFactor, BlendMode, BlendOperation, CullMode, CustomBlend, DepthTest, PrimitiveTopology, StencilFace, StencilOperation, StencilTest};
pub use crate::texture::CompareFunction;
use crate::shader::{self, ShaderLanguage, ShaderStage};
use crate::texture::{self, TextureDimension, TextureSpec};
use crate::{buffer, error};

/// How [`Gpu::open`] chooses its adapter.
#[derive(Clone, Debug, Default)]
pub struct GpuOptions {
    /// The first adapter whose name contains this text, ignoring case;
    /// the one wgpu prefers when `None`.
    pub adapter: Option<String>,
    /// When wgpu chooses, the low-power adapter rather than the
    /// high-performance one.
    pub low_power: bool,
    /// When wgpu chooses, its software adapter (WARP, lavapipe or
    /// llvmpipe).
    pub fallback: bool,
    /// The backends to look through; when `None`, Vulkan and DX12 on
    /// Windows and every native backend elsewhere.
    /// `wgpu::Backends::NOOP` adds wgpu's adapter that runs nothing. A
    /// thread that has used GL on Windows deadlocks the process on NVIDIA's
    /// driver if it exits while other threads create or destroy devices.
    pub backends: Option<wgpu::Backends>,
    /// The device's label, shown in errors and graphics debuggers.
    pub label: Option<String>,
}

/// A GPU device and its queue: opened by [`Gpu::open`] with every feature
/// and limit its adapter supports, experimental features aside, or opened
/// by another crate and taken by [`Gpu::from_parts`]. Clones share one
/// device, and every method takes `&self` from any thread.
#[derive(Clone)]
pub struct Gpu {
    core: CoreRef,
}

impl Gpu {
    /// Opens a device on the adapter `options` chooses.
    pub fn open(options: &GpuOptions) -> PsResult<Gpu> {
        let backends = options.backends.map(|set| {
            [Backend::Vulkan, Backend::Metal, Backend::Dx12, Backend::Gl, Backend::Noop].into_iter().filter(|b| set.contains(b.bits())).collect()
        });
        let open = OpenOptions {
            instance: InstanceOptions { backends, ..Default::default() },
            adapter: AdapterQuery { name: options.adapter.clone(), low_power: options.low_power, fallback: options.fallback },
            label: options.label.clone(),
            ..Default::default()
        };
        Ok(Gpu { core: device::open(&open, None)? })
    }

    /// A Gpu on a device another crate opened on `adapter`, such as the
    /// device a window's renderer draws with. It installs the callbacks a
    /// device [`Gpu::open`] opens has, in place of any the device had: an
    /// error raised outside any call is returned by the next call, and a
    /// lost device's reason by [`Gpu::lost_reason`]. Its features and
    /// limits are those the device was opened with.
    pub fn from_parts(adapter: wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue, label: String) -> PsResult<Gpu> {
        Ok(Gpu { core: device::adopt_device(adapter, device, queue, label)? })
    }

    /// The wgpu device, for anything this library does not wrap.
    pub fn device(&self) -> &wgpu::Device {
        &self.core.device
    }

    /// The device's queue.
    pub fn queue(&self) -> &wgpu::Queue {
        &self.core.queue
    }

    /// The adapter the device was opened on.
    pub fn adapter(&self) -> &wgpu::Adapter {
        &self.core.adapter
    }

    /// The adapter's name, backend, type and driver.
    pub fn adapter_info(&self) -> &wgpu::AdapterInfo {
        &self.core.info
    }

    /// The features the device was opened with.
    pub fn features(&self) -> wgpu::Features {
        self.core.features
    }

    /// The limits the device was opened with: every limit of its adapter,
    /// for a device [`Gpu::open`] opened.
    pub fn limits(&self) -> &wgpu::Limits {
        &self.core.limits
    }

    /// The device's label.
    pub fn label(&self) -> &str {
        &self.core.label
    }

    /// Why the device was lost, if it was; a lost device refuses every call.
    pub fn lost_reason(&self) -> Option<String> {
        self.core.lost_reason()
    }

    /// The errors the device raised outside any call of this library since
    /// the last call or take, oldest first, such as those of a renderer
    /// that shares the device. They are cleared, so the next call does not
    /// return them; a lost device's reason stays for [`Gpu::lost_reason`].
    pub fn take_errors(&self) -> Vec<String> {
        self.core.take_errors()
    }

    /// A buffer holding `contents`, its size rounded up to a multiple of 4
    /// bytes.
    pub fn create_buffer(&self, contents: &[u8], usage: wgpu::BufferUsages, label: &str) -> PsResult<wgpu::Buffer> {
        buffer::create_raw(&self.core, contents.len() as u64, usage, label, Some(contents))
    }

    /// A buffer of `size` zeroed bytes, rounded up to a multiple of 4.
    pub fn create_zeroed_buffer(&self, size: u64, usage: wgpu::BufferUsages, label: &str) -> PsResult<wgpu::Buffer> {
        buffer::create_raw(&self.core, size, usage, label, None)
    }

    /// Writes `bytes` at byte `offset` of `buffer`, which needs
    /// `COPY_DST`. A write that starts or ends inside a 4-byte word reads
    /// that word back first, which needs `COPY_SRC` too.
    pub fn write_buffer(&self, buffer: &wgpu::Buffer, offset: u64, bytes: &[u8]) -> PsResult<()> {
        buffer::write_bytes(&self.core, buffer, offset, bytes, None)
    }

    /// `size` bytes from byte `offset` of `buffer`, which needs `COPY_SRC`,
    /// once the GPU has finished the work submitted before.
    pub fn read_buffer(&self, buffer: &wgpu::Buffer, offset: u64, size: u64) -> PsResult<Vec<u8>> {
        buffer::read_bytes(&self.core, buffer, offset, size, None)
    }

    /// A 2D texture of `format`, with `usage`, or with every usage of
    /// copying, sampling, storage and rendering the format allows on this
    /// adapter when `None`.
    pub fn create_texture(&self, width: u32, height: u32, format: wgpu::TextureFormat, usage: Option<wgpu::TextureUsages>, label: &str) -> PsResult<wgpu::Texture> {
        let spec = TextureSpec {
            width,
            height,
            depth_or_layers: 1,
            format,
            mip_levels: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            usage: None,
            view_formats: Vec::new(),
            label: Some(label.to_string()),
        };
        let usage = match usage {
            Some(u) => u,
            None => texture::usable(&self.core, format),
        };
        Ok(texture::create_with_usage(&self.core, &spec, usage)?.texture)
    }

    /// Writes mip level 0 of `texture` from texels in rows packed with no
    /// padding.
    pub fn write_texture(&self, texture: &wgpu::Texture, bytes: &[u8]) -> PsResult<()> {
        texture::write_texels(&self.core, texture, 0, 0, wgpu::TextureAspect::All, bytes)
    }

    /// The texels of mip level 0 of `texture`, rows packed with no
    /// padding, once the GPU has finished the work submitted before.
    pub fn read_texture(&self, texture: &wgpu::Texture) -> PsResult<Vec<u8>> {
        texture::read_texels(&self.core, texture, 0, 0, wgpu::TextureAspect::All, None)
    }

    /// Compiles entry point `entry` of a WGSL compute shader (its only
    /// compute entry point when `None`) with `constants` as override
    /// values, by name or `@id`.
    pub fn compute_program(&self, wgsl: &str, entry: Option<&str>, constants: &[(&str, f64)]) -> PsResult<ComputeProgram> {
        self.compute_program_in(ShaderLanguage::Wgsl, wgsl, entry, constants)
    }

    /// Compiles a GLSL compute shader, whose entry point is `main`, with
    /// `constants` as the values of its specialization constants, keyed by
    /// `constant_id`, as "0".
    pub fn compute_program_glsl(&self, glsl: &str, constants: &[(&str, f64)]) -> PsResult<ComputeProgram> {
        self.compute_program_in(ShaderLanguage::Glsl, glsl, None, constants)
    }

    fn compute_program_in(&self, language: ShaderLanguage, text: &str, entry: Option<&str>, constants: &[(&str, f64)]) -> PsResult<ComputeProgram> {
        let parsed = shader::compile(shader::source_from_text(text.to_string(), Some(language), Some(ShaderStage::Compute), Vec::new())?, "Wgpu shader".to_string())?;
        let constants: Vec<(String, f64)> = constants.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        Ok(ComputeProgram { inner: pipeline::build(&self.core, &parsed, entry, &constants, None)? })
    }

    /// Compiles a WGSL shader's vertex and fragment entry points into a
    /// program that draws into textures of `options.format`.
    pub fn render_program(&self, wgsl: &str, options: &RenderOptions) -> PsResult<RenderProgram> {
        let parsed = shader::compile(shader::source_from_text(wgsl.to_string(), Some(ShaderLanguage::Wgsl), None, Vec::new())?, "Wgpu shader".to_string())?;
        let depth_format = match options.depth_format {
            Some(format) if !render::DEPTH_FORMATS.contains(&format) => {
                return Err(error::invalid("WgpuDepthFormat", format!("a depth_format is a depth or stencil format, and {} is not one", texture::format_name(format))));
            }
            Some(format) => {
                crate::image_io::holdable(&self.core, format)?;
                Some(format)
            }
            None => options.depth.then_some(wgpu::TextureFormat::Depth32Float),
        };
        let spec = RenderSpec {
            vertex_entry: options.vertex_entry.clone(),
            fragment_entry: options.fragment_entry.clone(),
            formats: vec![options.format],
            depth_stencil: render::depth_stencil_state(depth_format, &options.depth_test, &options.stencil, options.topology)?,
            topology: options.topology,
            cull: options.cull,
            blend: render::blend_state(options.blend, &options.custom_blend)?,
            sample_count: options.sample_count.max(1),
            constants: options.constants.clone(),
        };
        Ok(RenderProgram { inner: render::build_render(&self.core, &parsed, spec, None)?, format: options.format })
    }

    /// Waits until the GPU has finished all work submitted to the device.
    pub fn wait(&self) -> PsResult<()> {
        self.core.wait(None)
    }
}

/// A resource for one binding of a shader.
#[derive(Clone, Copy, Debug)]
pub enum Binding<'a> {
    /// A whole buffer.
    Buffer(&'a wgpu::Buffer),
    /// `size` bytes of a buffer from byte `offset`.
    BufferRange { buffer: &'a wgpu::Buffer, offset: u64, size: u64 },
    /// A whole texture, seen in its own format.
    Texture(&'a wgpu::Texture),
    /// A view of a texture.
    View(&'a wgpu::TextureView),
    /// A sampler.
    Sampler(&'a wgpu::Sampler),
}

impl Binding<'_> {
    fn resource(self) -> Resource {
        match self {
            Binding::Buffer(b) => binding::exact(b.clone(), b.size()),
            Binding::BufferRange { buffer, offset, size } => Resource::Buffer { buffer: buffer.clone(), offset, size: NonZeroU64::new(size) },
            Binding::Texture(t) => Resource::View(t.create_view(&wgpu::TextureViewDescriptor::default())),
            Binding::View(v) => Resource::View(v.clone()),
            Binding::Sampler(s) => Resource::Sampler(s.clone()),
        }
    }
}

/// Matches `given`, keyed by binding variable name or "group:binding", to
/// `bindings`.
fn bind(bindings: &[shader::Reflected], given: &[(&str, Binding<'_>)]) -> PsResult<Vec<Bound>> {
    binding::bound_by_name(bindings, given.iter().map(|(k, b)| (k.to_string(), b.resource())).collect())
}

fn names(bindings: &[shader::Reflected]) -> Vec<String> {
    bindings.iter().map(|b| b.name.clone()).collect()
}

fn infos(bindings: &[shader::Reflected]) -> Vec<BindingInfo> {
    bindings.iter().map(|b| BindingInfo { name: b.name.clone(), group: b.group, binding: b.binding, element_type: b.element_type, writable: b.writable() }).collect()
}

/// What a WGSL compute shader declares for each binding entry point
/// `entry` (its only compute entry point when `None`) uses, in group and
/// binding order, as [`ComputeProgram::binding_info`] gives it, found with
/// no device: naga parses and validates the shader as Test-WgpuShader does
/// with no device given. A shader that does not parse or validate, or an
/// entry point that is missing or not a compute one, fails with the error
/// [`Gpu::compute_program`] fails with.
pub fn compute_binding_info(wgsl: &str, entry: Option<&str>) -> PsResult<Vec<BindingInfo>> {
    binding_info_of(ShaderLanguage::Wgsl, wgsl, entry)
}

/// What a GLSL compute shader declares for each binding it uses, as
/// [`compute_binding_info`] gives it for WGSL, found with no device.
pub fn compute_binding_info_glsl(glsl: &str) -> PsResult<Vec<BindingInfo>> {
    binding_info_of(ShaderLanguage::Glsl, glsl, None)
}

fn binding_info_of(language: ShaderLanguage, text: &str, entry: Option<&str>) -> PsResult<Vec<BindingInfo>> {
    let parsed = shader::compile(shader::source_from_text(text.to_string(), Some(language), Some(ShaderStage::Compute), Vec::new())?, "Wgpu shader".to_string())?;
    let index = pipeline::compute_entry(&parsed, entry)?;
    Ok(infos(&shader::reflect(&parsed.module, &parsed.info, Some(index))))
}

/// What a program's shader declares for one binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingInfo {
    /// The binding's variable name.
    pub name: String,
    /// Its `@group`.
    pub group: u32,
    /// Its `@binding`.
    pub binding: u32,
    /// The type of each number in the buffer: the scalar of an array's
    /// elements, of a struct's last member when that is an array of no
    /// fixed length, or of the buffer's own type. `None` for a texture, a
    /// sampler, any other struct, and a scalar type no [`ElementType`]
    /// names.
    pub element_type: Option<ElementType>,
    /// Whether the shader may write it: a storage buffer declared
    /// `read_write`.
    pub writable: bool,
}

/// One `@location` input of a render program's vertex shader, fed from a
/// vertex buffer of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VertexInputInfo {
    /// The input's name: its argument's, or its struct member's.
    pub name: String,
    /// Its `@location`.
    pub location: u32,
    /// The type of each number in a vertex's value.
    pub element_type: ElementType,
    /// The numbers in each vertex's value: 1 for a scalar, 2 to 4 for a
    /// vector.
    pub components: u32,
}

fn input_infos(inputs: &[render::VertexInput]) -> Vec<VertexInputInfo> {
    inputs.iter().map(|i| VertexInputInfo { name: i.name.clone(), location: i.location, element_type: i.element, components: i.components }).collect()
}

/// What a WGSL shader's vertex and fragment entry points declare for each
/// binding they use, in group and binding order, as
/// [`RenderProgram::binding_info`] gives it, found with no device. The
/// entry points are the shader's only vertex and fragment ones when
/// `None`. A shader that does not parse or validate, an entry point it does
/// not have, and a vertex input no vertex format holds are refused with
/// the error [`Gpu::render_program`] gives.
pub fn render_binding_info(wgsl: &str, vertex_entry: Option<&str>, fragment_entry: Option<&str>) -> PsResult<Vec<BindingInfo>> {
    let parsed = render_parsed(wgsl)?;
    let vertex = shader::entry_index(&parsed.module, vertex_entry, naga::ShaderStage::Vertex)?;
    let fragment = shader::entry_index(&parsed.module, fragment_entry, naga::ShaderStage::Fragment)?;
    render::vertex_inputs(&parsed.module, vertex)?;
    Ok(infos(&render::render_bindings(&parsed, vertex, fragment)))
}

/// A WGSL shader's vertex entry point's `@location` inputs, in location
/// order, as [`RenderProgram::vertex_input_info`] gives them, found with no
/// device. The entry point is the shader's only vertex one when `None`. A
/// shader that does not parse or validate, an entry point it does not
/// have, and an input no vertex format holds are refused with the error
/// [`Gpu::render_program`] gives.
pub fn render_vertex_input_info(wgsl: &str, vertex_entry: Option<&str>) -> PsResult<Vec<VertexInputInfo>> {
    let parsed = render_parsed(wgsl)?;
    let vertex = shader::entry_index(&parsed.module, vertex_entry, naga::ShaderStage::Vertex)?;
    Ok(input_infos(&render::vertex_inputs(&parsed.module, vertex)?))
}

/// A WGSL shader parsed and validated as [`Gpu::render_program`] reads it.
fn render_parsed(wgsl: &str) -> PsResult<Arc<shader::Parsed>> {
    shader::compile(shader::source_from_text(wgsl.to_string(), Some(ShaderLanguage::Wgsl), None, Vec::new())?, "Wgpu shader".to_string())
}

/// How many workgroups a [`RunSpec`] dispatches.
#[derive(Clone, Copy, Debug, Default)]
pub enum Dispatch<'a> {
    /// Workgroups in x, y and z.
    Workgroups([u32; 3]),
    /// Invocations in x, y and z, divided by the workgroup size and rounded
    /// up.
    Invocations([u64; 3]),
    /// Workgroups in x, y and z that the GPU reads from `buffer`, which has
    /// INDIRECT usage, as three u32 from byte `offset`.
    Indirect { buffer: &'a wgpu::Buffer, offset: u64 },
    /// One invocation per element of the first buffer bound, in x, as
    /// Invoke-WgpuCompute dispatches when given no size.
    #[default]
    Auto,
}

/// One run of a [`ComputeProgram`], for [`ComputeProgram::run_with`]: what
/// Invoke-WgpuCompute's -Binding, -Workgroups, -Invocations, -Indirect,
/// -IndirectOffset, -Immediate and -PassThru give it.
#[derive(Clone, Copy, Debug, Default)]
pub struct RunSpec<'a> {
    /// The resources, by binding variable name or "group:binding".
    pub bindings: &'a [(&'a str, Binding<'a>)],
    /// How many workgroups it dispatches.
    pub dispatch: Dispatch<'a>,
    /// The immediate data the entry point reads, from its first byte and
    /// no longer than the entry point declares; zeros pad it to a multiple
    /// of 4 bytes.
    pub immediates: Option<&'a [u8]>,
    /// Whether to measure how long the GPU spent in the pass, which a
    /// device with the TIMESTAMP_QUERY feature can.
    pub timed: bool,
}

/// What [`ComputeProgram::run_with`] did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunReport {
    /// The workgroups dispatched in x, y and z; 0 for an indirect dispatch.
    pub workgroups: [u32; 3],
    /// Nanoseconds the GPU spent in the pass, when the spec asked for them
    /// and the device has the TIMESTAMP_QUERY feature.
    pub gpu_time_ns: Option<f64>,
    /// Milliseconds the whole call took, binding and waiting included.
    pub cpu_time_ms: f64,
}

/// A compiled compute entry point, run as often as needed. Clones share
/// the pipeline.
#[derive(Clone)]
pub struct ComputeProgram {
    inner: Arc<ComputeInner>,
}

impl ComputeProgram {
    /// The entry point it runs.
    pub fn entry_point(&self) -> &str {
        &self.inner.entry
    }

    /// Invocations per workgroup in x, y and z.
    pub fn workgroup_size(&self) -> [u32; 3] {
        self.inner.workgroup_size
    }

    /// The variable names of the bindings it uses, in group and binding
    /// order.
    pub fn bindings(&self) -> Vec<String> {
        names(&self.inner.bindings)
    }

    /// What the shader declares for each binding it uses, in group and
    /// binding order: the type Invoke-WgpuCompute converts an array given
    /// for a binding to, and whether it returns that array as the shader
    /// left it.
    pub fn binding_info(&self) -> Vec<BindingInfo> {
        infos(&self.inner.bindings)
    }

    /// The wgpu pipeline, for passes recorded by hand.
    pub fn pipeline(&self) -> &wgpu::ComputePipeline {
        &self.inner.pipeline
    }

    /// Binds `bindings` by name and dispatches `workgroups` in x, y and z,
    /// waiting for the GPU to finish.
    pub fn run(&self, bindings: &[(&str, Binding<'_>)], workgroups: [u32; 3]) -> PsResult<()> {
        let bound = bind(&self.inner.bindings, bindings)?;
        compute::submit_bound(&self.inner, bound, &compute::Dispatch::Workgroups(workgroups), None, false, Instant::now())?.finish(None).map(|_| ())
    }

    /// Binds `spec`'s resources by name, dispatches the workgroups it
    /// gives with its immediate data, and waits for the GPU, as
    /// Invoke-WgpuCompute does with the same parameters.
    pub fn run_with(&self, spec: &RunSpec<'_>) -> PsResult<RunReport> {
        let started = Instant::now();
        let bound = bind(&self.inner.bindings, spec.bindings)?;
        let dispatch = dispatch_of(spec.dispatch);
        let outcome = compute::submit_bound(&self.inner, bound, &dispatch, spec.immediates.map(<[u8]>::to_vec), spec.timed, started)?.finish(None)?;
        Ok(RunReport { workgroups: outcome.workgroups, gpu_time_ns: outcome.gpu_ns, cpu_time_ms: outcome.cpu_ms })
    }

    /// Records one dispatch into `pass`, a compute pass the caller began:
    /// binds `spec`'s resources by name, sets its immediate data and
    /// dispatches the workgroups it gives, refusing what
    /// [`run_with`](Self::run_with) refuses, and returns the workgroups in
    /// x, y and z, 0 for an indirect dispatch. It submits nothing, so a
    /// fault in the recorded commands is found when the caller finishes and
    /// submits its encoder, and [`Gpu::take_errors`] returns it. Timing
    /// needs queries around the whole pass, so a timed `spec` is refused.
    pub fn record(&self, pass: &mut wgpu::ComputePass<'_>, spec: &RunSpec<'_>) -> PsResult<[u32; 3]> {
        if spec.timed {
            return Err(error::invalid("WgpuRecordTimed", "record dispatches into a pass the caller began, and timing needs queries around the whole pass: give a spec that is not timed"));
        }
        let bound = bind(&self.inner.bindings, spec.bindings)?;
        let dispatch = dispatch_of(spec.dispatch);
        let prepared = compute::prepare(&self.inner, &bound, &dispatch, spec.immediates.map(<[u8]>::to_vec))?;
        compute::record_dispatch(&self.inner, pass, &prepared, &dispatch);
        Ok(prepared.counts.unwrap_or([0, 0, 0]))
    }
}

/// The dispatch a [`RunSpec`] gives, as the compute module runs it.
fn dispatch_of(dispatch: Dispatch<'_>) -> compute::Dispatch {
    match dispatch {
        Dispatch::Workgroups(counts) => compute::Dispatch::Workgroups(counts),
        Dispatch::Invocations(counts) => compute::Dispatch::Invocations(counts),
        Dispatch::Indirect { buffer, offset } => compute::Dispatch::Indirect(buffer.clone(), offset),
        Dispatch::Auto => compute::Dispatch::Auto,
    }
}

/// How [`Gpu::render_program`] builds a program.
#[derive(Clone, Debug)]
pub struct RenderOptions {
    /// The format of the textures it draws into.
    pub format: wgpu::TextureFormat,
    /// The vertex entry point; the shader's only one when `None`.
    pub vertex_entry: Option<String>,
    /// The fragment entry point; the shader's only one when `None`.
    pub fragment_entry: Option<String>,
    /// How vertices make primitives.
    pub topology: PrimitiveTopology,
    /// Which faces it discards.
    pub cull: CullMode,
    /// How fragments combine with the target.
    pub blend: BlendMode,
    /// The factors and operations of a [`BlendMode::Custom`] blend, which
    /// any other mode refuses.
    pub custom_blend: CustomBlend,
    /// Whether it tests and writes depth in a Depth32Float target.
    pub depth: bool,
    /// The format of its depth target, a depth or stencil format, which
    /// turns depth on; `None` leaves the target to `depth`.
    pub depth_format: Option<wgpu::TextureFormat>,
    /// How its depth target tests depth.
    pub depth_test: DepthTest,
    /// How its depth target tests stencil, which needs a format with a
    /// stencil aspect.
    pub stencil: StencilTest,
    /// Samples per pixel, resolved into the target.
    pub sample_count: u32,
    /// Override values, by name or `@id`.
    pub constants: Vec<(String, f64)>,
}

impl Default for RenderOptions {
    fn default() -> Self {
        RenderOptions {
            format: wgpu::TextureFormat::Rgba8Unorm,
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
}

/// One draw by a [`RenderProgram`].
#[derive(Clone, Copy, Debug)]
pub struct Draw<'a> {
    /// The vertices to draw, or the indices when `indices` is given.
    pub vertex_count: u32,
    /// The instances to draw.
    pub instance_count: u32,
    /// A buffer for each `@location` input of the vertex shader, by the
    /// input's name, each holding that input's values.
    pub vertices: &'a [(&'a str, &'a wgpu::Buffer)],
    /// An index buffer, its format and how many indices to draw.
    pub indices: Option<(&'a wgpu::Buffer, wgpu::IndexFormat, u32)>,
    /// The resources for the shader's bindings, by variable name or
    /// "group:binding".
    pub bindings: &'a [(&'a str, Binding<'a>)],
    /// The color the target is cleared to first; `None` draws over what
    /// it holds.
    pub clear: Option<wgpu::Color>,
    /// The color a blend factor of Constant reads; `None` leaves it zero.
    pub blend_constant: Option<wgpu::Color>,
    /// The depth the program's own depth target starts at, from 0 to 1;
    /// `None` starts it at 1. A draw recorded into a caller's pass takes
    /// `None`.
    pub clear_depth: Option<f32>,
    /// The value the program's own stencil starts at; `None` starts it at
    /// 0. A draw recorded into a caller's pass takes `None`.
    pub clear_stencil: Option<u32>,
    /// The value a stencil test compares with and Replace writes; `None`
    /// leaves the pass's, which a pass of the program's own starts at 0.
    /// A program with no stencil test refuses it.
    pub stencil_reference: Option<u32>,
}

impl Default for Draw<'_> {
    fn default() -> Self {
        Draw {
            vertex_count: 0,
            instance_count: 1,
            vertices: &[],
            indices: None,
            bindings: &[],
            clear: Some(wgpu::Color::BLACK),
            blend_constant: None,
            clear_depth: None,
            clear_stencil: None,
            stencil_reference: None,
        }
    }
}

/// A compiled vertex and fragment shader that draws into textures of one
/// format. Clones share the pipeline.
#[derive(Clone)]
pub struct RenderProgram {
    inner: Arc<RenderInner>,
    format: wgpu::TextureFormat,
}

impl RenderProgram {
    /// The names of the vertex shader's `@location` inputs, in location
    /// order.
    pub fn vertex_inputs(&self) -> Vec<String> {
        self.inner.inputs.iter().map(|i| i.name.clone()).collect()
    }

    /// The variable names of the bindings both stages use, in group and
    /// binding order.
    pub fn bindings(&self) -> Vec<String> {
        names(&self.inner.bindings)
    }

    /// What the shader declares for each binding both stages use, in group
    /// and binding order.
    pub fn binding_info(&self) -> Vec<BindingInfo> {
        infos(&self.inner.bindings)
    }

    /// The vertex shader's `@location` inputs, in location order, each with
    /// the type and the count of the numbers in a vertex's value.
    pub fn vertex_input_info(&self) -> Vec<VertexInputInfo> {
        input_infos(&self.inner.inputs)
    }

    /// The wgpu pipeline, for passes recorded by hand.
    pub fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.inner.pipeline
    }

    /// The format of the textures it draws into.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.format
    }

    /// Samples per pixel of the color and depth attachments it draws with.
    pub fn sample_count(&self) -> u32 {
        self.inner.spec.sample_count
    }

    /// The format of the depth attachment it tests and writes, or `None`
    /// for a program built without depth.
    pub fn depth_format(&self) -> Option<wgpu::TextureFormat> {
        self.inner.spec.depth_format()
    }

    /// Draws into `target`, waiting for the GPU to finish. A blend that
    /// reads the constant needs `draw.blend_constant`, and one that reads
    /// none refuses it. Its own depth target starts at `draw.clear_depth`
    /// and `draw.clear_stencil`, each refused for an aspect the target
    /// lacks.
    pub fn draw(&self, target: &wgpu::Texture, draw: &Draw<'_>) -> PsResult<()> {
        let parts = self.prepare(draw)?;
        let load = match draw.clear {
            Some(color) => wgpu::LoadOp::Clear(color),
            None => wgpu::LoadOp::Load,
        };
        let (depth_clear, stencil_clear) =
            render::target_clears(self.inner.spec.depth_format(), draw.clear_depth.map(f64::from), draw.clear_stencil, ["a draw's clear_depth", "a draw's clear_stencil"])?;
        let commands = render::record_draw(&self.inner, target, &call_of(&parts, draw), load, depth_clear, stencil_clear)?;
        self.inner.core.submit_and_wait(vec![commands], None)
    }

    /// Records one draw into `pass`, a render pass the caller began with
    /// attachments of this program's [`format`](Self::format),
    /// [`sample_count`](Self::sample_count) and
    /// [`depth_format`](Self::depth_format): binds `draw`'s resources and
    /// vertex buffers by name, refusing what [`draw`](Self::draw) refuses,
    /// and draws. It submits nothing, so a fault in the recorded commands
    /// is found when the caller finishes and submits its encoder, and
    /// [`Gpu::take_errors`] returns it. The pass's load decides what the
    /// target holds first, so a `draw` with a clear is refused.
    pub fn record(&self, pass: &mut wgpu::RenderPass<'_>, draw: &Draw<'_>) -> PsResult<()> {
        if draw.clear.is_some() {
            return Err(error::invalid("WgpuRecordClear", "record draws into a pass the caller began, whose load decides what the target holds first: give the draw a clear of None"));
        }
        if draw.clear_depth.is_some() || draw.clear_stencil.is_some() {
            return Err(error::invalid(
                "WgpuRecordClear",
                "record draws into a pass the caller began, whose load decides what its depth target holds first: give the draw a clear_depth and a clear_stencil of None",
            ));
        }
        let parts = self.prepare(draw)?;
        render::draw_on(pass, &self.inner.pipeline, &call_of(&parts, draw));
        Ok(())
    }

    /// The bind groups and vertex buffers of `draw`, refusing a blend
    /// constant the blend does not read or one it reads and is not given,
    /// an input with no buffer, and a buffer for no input.
    fn prepare(&self, draw: &Draw<'_>) -> PsResult<DrawParts> {
        let inner = &self.inner;
        let reads = render::reads_constant(inner.spec.blend);
        if reads && draw.blend_constant.is_none() {
            return Err(error::invalid("WgpuBlendConstant", "the blend reads a constant: give the draw a blend_constant"));
        }
        if !reads && draw.blend_constant.is_some() {
            return Err(error::invalid("WgpuBlendConstant", "a draw's blend_constant sets the constant a blend reads, and this blend reads none"));
        }
        if draw.stencil_reference.is_some() && !inner.spec.tests_stencil() {
            return Err(error::invalid("WgpuStencilReference", "a draw's stencil_reference sets the value a stencil test compares with and Replace writes, and this program has no stencil test"));
        }
        let mut vertex_buffers = Vec::with_capacity(inner.inputs.len());
        for input in &inner.inputs {
            let found = draw.vertices.iter().find(|(name, _)| name.eq_ignore_ascii_case(&input.name));
            match found {
                Some((_, buffer)) => vertex_buffers.push((*buffer).clone()),
                None => return Err(error::invalid("WgpuNoVertices", format!("no vertex buffer was given for {}", input.name))),
            }
        }
        for (name, _) in draw.vertices {
            if !inner.inputs.iter().any(|i| i.name.eq_ignore_ascii_case(name)) {
                return Err(error::invalid("WgpuNoVertexInput", format!("the vertex shader has no input named {name}")));
            }
        }
        let bound = bind(&inner.bindings, draw.bindings)?;
        let groups = binding::make_groups(&inner.core, &inner.groups, &bound, &inner.label)?;
        Ok(DrawParts { groups, vertex_buffers })
    }
}

/// The bind groups of a draw's resources, and its vertex buffers in the
/// order of the vertex inputs' locations.
struct DrawParts {
    groups: Vec<(u32, wgpu::BindGroup)>,
    vertex_buffers: Vec<wgpu::Buffer>,
}

/// What `draw` sets on a pass, over its bind groups and vertex buffers.
fn call_of<'a>(parts: &'a DrawParts, draw: &Draw<'a>) -> DrawCall<'a> {
    DrawCall {
        groups: &parts.groups,
        vertex_buffers: &parts.vertex_buffers,
        index: draw.indices,
        vertex_count: draw.vertex_count,
        instances: draw.instance_count,
        blend_constant: draw.blend_constant,
        stencil_reference: draw.stencil_reference,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOUBLE: &str = "@group(0) @binding(0) var<storage, read_write> v: array<f32>;
        @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
            if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * 2.0; }
        }";

    const FULLSCREEN: &str = "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
            return vec4<f32>(corners[i], 0.0, 1.0);
        }
        @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }";

    const POSITIONED: &str = "@vertex fn vs(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> {
            return vec4<f32>(position, 0.0, 1.0);
        }
        @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.0, 1.0, 0.0, 1.0); }";

    /// A blue triangle over the whole target, at depth 0.5.
    const HALF_DEEP: &str = "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
            var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
            return vec4<f32>(corners[i], 0.5, 1.0);
        }
        @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(0.0, 0.0, 1.0, 1.0); }";

    /// What the tests open devices with: the first adapter whose name
    /// contains WGPU_PWSH_TEST_ADAPTER when that is set, as "Basic Render"
    /// names WARP, and the adapter wgpu prefers when it is not.
    fn options() -> GpuOptions {
        match std::env::var("WGPU_PWSH_TEST_ADAPTER") {
            Ok(name) => GpuOptions { adapter: Some(name), ..Default::default() },
            Err(std::env::VarError::NotPresent) => GpuOptions::default(),
            Err(e) => panic!("WGPU_PWSH_TEST_ADAPTER cannot be read: {e}"),
        }
    }

    /// The device the tests run on: `None`, with the reason printed, only
    /// where this host has no adapter at all.
    fn gpu() -> Option<Gpu> {
        crate::faults::install();
        let adapters = device::adapters(&InstanceOptions::default()).expect("the adapters");
        if adapters.is_empty() {
            eprintln!("skipped: this host has no GPU adapter");
            return None;
        }
        Some(Gpu::open(&options()).expect("a device opens on a host with an adapter"))
    }

    fn floats(bytes: &[u8]) -> Vec<f32> {
        bytemuck::pod_collect_to_vec(bytes)
    }

    fn every_pixel(texels: &[u8], rgba: [u8; 4]) -> bool {
        texels.as_chunks::<4>().0.iter().all(|px| *px == rgba)
    }

    #[test]
    fn a_buffer_round_trips_and_takes_writes() {
        let Some(gpu) = gpu() else { return };
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST;
        let buffer = gpu.create_buffer(bytemuck::cast_slice(&[1.0f32, 2.0, 3.0, 4.0]), usage, "round trip").expect("the buffer");
        assert_eq!(floats(&gpu.read_buffer(&buffer, 0, 16).expect("read")), [1.0, 2.0, 3.0, 4.0]);
        gpu.write_buffer(&buffer, 4, bytemuck::cast_slice(&[9.0f32])).expect("write");
        assert_eq!(floats(&gpu.read_buffer(&buffer, 0, 16).expect("read")), [1.0, 9.0, 3.0, 4.0]);
        let zeroed = gpu.create_zeroed_buffer(6, usage, "zeroed").expect("the zeroed buffer");
        assert_eq!(gpu.read_buffer(&zeroed, 0, 6).expect("read"), [0u8; 6]);
    }

    #[test]
    fn a_compute_program_runs_with_bindings_by_name() {
        let Some(gpu) = gpu() else { return };
        let program = gpu.compute_program(DOUBLE, None, &[]).expect("the program");
        assert_eq!(program.entry_point(), "main");
        assert_eq!(program.workgroup_size(), [64, 1, 1]);
        assert_eq!(program.bindings(), ["v"]);
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let data = gpu.create_buffer(bytemuck::cast_slice(&[1.0f32, 2.0, 3.0]), usage, "data").expect("the buffer");
        program.run(&[("v", Binding::Buffer(&data))], [1, 1, 1]).expect("the run");
        program.run(&[("0:0", Binding::Buffer(&data))], [1, 1, 1]).expect("the run by group and binding");
        assert_eq!(floats(&gpu.read_buffer(&data, 0, 12).expect("read")), [4.0, 8.0, 12.0]);
    }

    #[test]
    fn a_compute_program_refuses_missing_unknown_and_doubled_bindings() {
        let Some(gpu) = gpu() else { return };
        let program = gpu.compute_program(DOUBLE, None, &[]).expect("the program");
        let data = gpu.create_buffer(&[0u8; 16], wgpu::BufferUsages::STORAGE, "data").expect("the buffer");
        let id = |r: PsResult<()>| r.expect_err("refused").error_id;
        assert_eq!(id(program.run(&[], [1, 1, 1])), "WgpuBindingMissing");
        assert_eq!(id(program.run(&[("w", Binding::Buffer(&data))], [1, 1, 1])), "WgpuNoBinding");
        assert_eq!(id(program.run(&[("v", Binding::Buffer(&data)), ("0:0", Binding::Buffer(&data))], [1, 1, 1])), "WgpuBindingTwice");
    }

    /// A spec dispatches by workgroups, by invocations, by the first
    /// buffer's length or by counts on the GPU, gives immediate data where
    /// the device takes it, and times the pass where the device can.
    #[test]
    fn a_compute_program_runs_from_a_spec() {
        let Some(gpu) = gpu() else { return };
        let program = gpu.compute_program(DOUBLE, None, &[]).expect("the program");
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let data = gpu.create_buffer(bytemuck::cast_slice(&[1.0f32, 2.0, 3.0]), usage, "data").expect("the buffer");
        let counts = gpu.create_buffer(bytemuck::cast_slice(&[1u32, 1, 1]), wgpu::BufferUsages::INDIRECT, "counts").expect("the counts");
        let bindings = [("v", Binding::Buffer(&data))];
        let run = |dispatch: Dispatch<'_>| program.run_with(&RunSpec { bindings: &bindings, dispatch, timed: true, ..Default::default() }).expect("the run");
        let report = run(Dispatch::Workgroups([1, 1, 1]));
        assert_eq!(report.workgroups, [1, 1, 1]);
        assert_eq!(report.gpu_time_ns.is_some(), gpu.features().contains(wgpu::Features::TIMESTAMP_QUERY));
        assert_eq!(run(Dispatch::Invocations([3, 1, 1])).workgroups, [1, 1, 1]);
        assert_eq!(run(Dispatch::Auto).workgroups, [1, 1, 1]);
        assert_eq!(run(Dispatch::Indirect { buffer: &counts, offset: 0 }).workgroups, [0, 0, 0]);
        assert_eq!(floats(&gpu.read_buffer(&data, 0, 12).expect("read")), [16.0, 32.0, 48.0]);
        if !gpu.features().contains(wgpu::Features::IMMEDIATES) {
            return;
        }
        let scaled = "struct Scale { k: f32 }
            var<immediate> scale: Scale;
            @group(0) @binding(0) var<storage, read_write> v: array<f32>;
            @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                if (id.x < arrayLength(&v)) { v[id.x] = v[id.x] * scale.k; }
            }";
        let scaled = gpu.compute_program(scaled, None, &[]).expect("the program");
        scaled.run_with(&RunSpec { bindings: &bindings, immediates: Some(bytemuck::cast_slice(&[0.25f32])), ..Default::default() }).expect("the run");
        assert_eq!(floats(&gpu.read_buffer(&data, 0, 12).expect("read")), [4.0, 8.0, 12.0]);
        let too_long = scaled.run_with(&RunSpec { bindings: &bindings, immediates: Some(&[0; 8]), ..Default::default() });
        assert_eq!(too_long.expect_err("refused").error_id, "WgpuImmediates");
    }

    /// A GLSL compute shader runs with its specialization constant's
    /// default and with a value given by its constant_id.
    #[test]
    fn a_glsl_compute_program_runs_with_its_constant() {
        let Some(gpu) = gpu() else { return };
        let glsl = "#version 450
            layout(local_size_x = 64) in;
            layout(constant_id = 0) const float factor = 2.0;
            layout(set = 0, binding = 0) buffer Values { float v[]; };
            void main() {
                uint i = gl_GlobalInvocationID.x;
                if (i < uint(v.length())) { v[i] = v[i] * factor; }
            }";
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let data = gpu.create_buffer(bytemuck::cast_slice(&[1.0f32, 2.0, 3.0]), usage, "data").expect("the buffer");
        let bindings = [("0:0", Binding::Buffer(&data))];
        let spec = RunSpec { bindings: &bindings, dispatch: Dispatch::Invocations([3, 1, 1]), ..Default::default() };
        let doubling = gpu.compute_program_glsl(glsl, &[]).expect("the program");
        assert_eq!(doubling.entry_point(), "main");
        doubling.run_with(&spec).expect("the run");
        assert_eq!(floats(&gpu.read_buffer(&data, 0, 12).expect("read")), [2.0, 4.0, 6.0]);
        gpu.compute_program_glsl(glsl, &[("0", 1.5)]).expect("the program").run_with(&spec).expect("the run");
        assert_eq!(floats(&gpu.read_buffer(&data, 0, 12).expect("read")), [3.0, 6.0, 9.0]);
    }

    /// A shader's binding info comes with no device, as a compiled
    /// program gives it, for WGSL and GLSL, and a shader that does not
    /// parse, a missing entry point or a vertex one is refused with the
    /// ids compiling it raises.
    #[test]
    fn binding_info_needs_no_device() {
        let wgsl = "@group(0) @binding(1) var<storage, read_write> output: array<u32>;
            @group(0) @binding(0) var<storage, read> input: array<f32>;
            @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                if (id.x < arrayLength(&output)) { output[id.x] = u32(input[id.x]); }
            }
            @vertex fn vs() -> @builtin(position) vec4<f32> { return vec4<f32>(0.0); }";
        let info = |name: &str, binding, element_type, writable| BindingInfo { name: name.to_string(), group: 0, binding, element_type, writable };
        let expected = [info("input", 0, Some(ElementType::Float32), false), info("output", 1, Some(ElementType::UInt32), true)];
        assert_eq!(compute_binding_info(wgsl, Some("main")).expect("the info"), expected);
        assert_eq!(compute_binding_info(wgsl, None).expect("the info"), expected);
        let glsl = "#version 450
            layout(local_size_x = 64) in;
            layout(set = 0, binding = 0) buffer Values { int v[]; };
            void main() { v[gl_GlobalInvocationID.x] += 1; }";
        let glsl_info = compute_binding_info_glsl(glsl).expect("the GLSL info");
        assert_eq!(glsl_info.len(), 1);
        assert_eq!((glsl_info[0].group, glsl_info[0].binding, glsl_info[0].element_type, glsl_info[0].writable), (0, 0, Some(ElementType::Int32), true));
        let id = |r: PsResult<Vec<BindingInfo>>| r.expect_err("refused").error_id;
        assert_eq!(id(compute_binding_info("fn broken(", None)), "WgpuShaderInvalid");
        assert_eq!(id(compute_binding_info(wgsl, Some("missing"))), "WgpuNoEntryPoint");
        assert_eq!(id(compute_binding_info(wgsl, Some("vs"))), "WgpuNotCompute");
        if let Some(gpu) = gpu() {
            assert_eq!(gpu.compute_program(wgsl, Some("main"), &[]).expect("the program").binding_info(), expected);
        }
    }

    /// A render shader's bindings, in group and binding order, and its
    /// vertex inputs, in location order, the same from a program and with
    /// no device, which refuses the same shaders.
    #[test]
    fn render_binding_and_vertex_input_info_need_no_device() {
        let wgsl = "struct Frame { time: f32, scale: f32 }
            @group(1) @binding(0) var<uniform> frame: Frame;
            @group(0) @binding(2) var<storage, read> tints: array<f32>;
            struct Out { @builtin(position) position: vec4<f32>, @location(0) color: vec3<f32> }
            @vertex fn vs(@location(1) color: vec3<f32>, @location(0) position: vec2<f32>) -> Out {
                var out: Out;
                out.position = vec4<f32>(position * frame.scale, 0.0, 1.0);
                out.color = color;
                return out;
            }
            @fragment fn fs(input: Out) -> @location(0) vec4<f32> { return vec4<f32>(input.color * tints[0], 1.0); }";
        let bindings = render_binding_info(wgsl, None, None).expect("the bindings");
        let places: Vec<(&str, u32, u32, bool)> = bindings.iter().map(|b| (b.name.as_str(), b.group, b.binding, b.writable)).collect();
        assert_eq!(places, [("tints", 0, 2, false), ("frame", 1, 0, false)]);
        assert_eq!(bindings[0].element_type, Some(ElementType::Float32));
        assert_eq!(render_binding_info(wgsl, Some("vs"), Some("fs")).expect("the bindings"), bindings);
        let input = |name: &str, location, components| VertexInputInfo { name: name.to_string(), location, element_type: ElementType::Float32, components };
        let inputs = render_vertex_input_info(wgsl, None).expect("the inputs");
        assert_eq!(inputs, [input("position", 0, 2), input("color", 1, 3)]);
        let id = |r: PsResult<Vec<BindingInfo>>| r.expect_err("refused").error_id;
        assert_eq!(id(render_binding_info("fn broken(", None, None)), "WgpuShaderInvalid");
        assert_eq!(id(render_binding_info(wgsl, Some("missing"), None)), "WgpuNoEntryPoint");
        assert_eq!(id(render_binding_info(DOUBLE, None, None)), "WgpuNoEntryPoint");
        assert_eq!(render_vertex_input_info(DOUBLE, None).expect_err("refused").error_id, "WgpuNoEntryPoint");
        if let Some(gpu) = gpu() {
            let program = gpu.render_program(wgsl, &RenderOptions::default()).expect("the program");
            assert_eq!(program.binding_info(), bindings);
            assert_eq!(program.vertex_input_info(), inputs);
        }
    }

    /// Each binding's element type and whether the shader may write it,
    /// as Invoke-WgpuCompute reads them, in group and binding order.
    #[test]
    fn a_compute_program_says_what_each_binding_holds() {
        let Some(gpu) = gpu() else { return };
        let wgsl = "struct Params { scale: f32, count: u32 }
            @group(1) @binding(0) var<uniform> params: Params;
            @group(0) @binding(1) var<storage, read_write> output: array<i32>;
            @group(0) @binding(0) var<storage, read> input: array<vec2<f32>>;
            @compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {
                if (id.x < params.count) { output[id.x] = i32(input[id.x].x * params.scale); }
            }";
        let program = gpu.compute_program(wgsl, None, &[]).expect("the program");
        let info = |name: &str, group, binding, element_type, writable| BindingInfo { name: name.to_string(), group, binding, element_type, writable };
        assert_eq!(
            program.binding_info(),
            [
                info("input", 0, 0, Some(ElementType::Float32), false),
                info("output", 0, 1, Some(ElementType::Int32), true),
                info("params", 1, 0, None, false),
            ]
        );
    }

    #[test]
    fn a_render_program_draws_into_a_texture() {
        let Some(gpu) = gpu() else { return };
        let usage = Some(wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC);
        let target = gpu.create_texture(8, 8, wgpu::TextureFormat::Rgba8Unorm, usage, "target").expect("the texture");
        let program = gpu.render_program(FULLSCREEN, &RenderOptions::default()).expect("the program");
        assert!(program.vertex_inputs().is_empty());
        program.draw(&target, &Draw { vertex_count: 3, ..Default::default() }).expect("the draw");
        assert!(every_pixel(&gpu.read_texture(&target).expect("read"), [255, 0, 0, 255]));
    }

    #[test]
    fn a_render_program_built_with_samples_and_depth_draws_with_them() {
        let Some(gpu) = gpu() else { return };
        let target = gpu.create_texture(8, 8, wgpu::TextureFormat::Rgba8Unorm, None, "target").expect("the texture");
        let options = RenderOptions { sample_count: 4, depth: true, ..Default::default() };
        let program = gpu.render_program(FULLSCREEN, &options).expect("the program");
        program.draw(&target, &Draw { vertex_count: 3, ..Default::default() }).expect("the draw");
        assert!(every_pixel(&gpu.read_texture(&target).expect("read"), [255, 0, 0, 255]));
    }

    #[test]
    fn a_draw_without_a_clear_color_keeps_what_the_target_holds() {
        let Some(gpu) = gpu() else { return };
        let target = gpu.create_texture(2, 1, wgpu::TextureFormat::Rgba8Unorm, None, "target").expect("the texture");
        gpu.write_texture(&target, &[0, 0, 255, 255, 0, 0, 255, 255]).expect("write");
        let program = gpu.render_program(POSITIONED, &RenderOptions::default()).expect("the program");
        assert_eq!(program.vertex_inputs(), ["position"]);
        let left_half: [f32; 6] = [-1.0, -1.0, 0.0, -1.0, -1.0, 3.0];
        let positions = gpu.create_buffer(bytemuck::cast_slice(&left_half), wgpu::BufferUsages::VERTEX, "positions").expect("the vertices");
        program.draw(&target, &Draw { vertex_count: 3, vertices: &[("position", &positions)], clear: None, ..Default::default() }).expect("the draw");
        assert_eq!(gpu.read_texture(&target).expect("read"), [0, 255, 0, 255, 0, 0, 255, 255]);
    }

    /// A Constant blend mixes the fragment over the target by the draw's
    /// blend constant, each channel within one step of the exact mix; a
    /// draw is refused without a constant its blend reads, or with one its
    /// blend does not read.
    #[test]
    fn a_constant_blend_mixes_by_the_draws_constant() {
        let Some(gpu) = gpu() else { return };
        let target = gpu.create_texture(4, 4, wgpu::TextureFormat::Rgba8Unorm, None, "target").expect("the texture");
        let program = gpu.render_program(FULLSCREEN, &RenderOptions { blend: BlendMode::Constant, ..Default::default() }).expect("the program");
        let blue = wgpu::Color { r: 0.0, g: 0.0, b: 1.0, a: 1.0 };
        let quarter = wgpu::Color { r: 0.25, g: 0.25, b: 0.25, a: 0.25 };
        program.draw(&target, &Draw { vertex_count: 3, clear: Some(blue), blend_constant: Some(quarter), ..Default::default() }).expect("the draw");
        let mixed = gpu.read_texture(&target).expect("read");
        assert!(mixed.as_chunks::<4>().0.iter().all(|px| px.iter().zip([64u8, 0, 191, 255]).all(|(got, want)| got.abs_diff(want) <= 1)), "{:?}", &mixed[..4]);
        let unset = program.draw(&target, &Draw { vertex_count: 3, ..Default::default() }).expect_err("refused");
        assert_eq!(unset.error_id, "WgpuBlendConstant");
        assert_eq!(unset.message, "the blend reads a constant: give the draw a blend_constant");
        let plain = gpu.render_program(FULLSCREEN, &RenderOptions::default()).expect("the program");
        let unread = plain.draw(&target, &Draw { vertex_count: 3, blend_constant: Some(quarter), ..Default::default() }).expect_err("refused");
        assert_eq!(unread.error_id, "WgpuBlendConstant");
    }

    /// A draw recorded into a pass the caller began, on multisampled and
    /// depth attachments made from the program's accessors, puts the
    /// texels draw puts once the caller submits; a clear given to record
    /// is refused, and so is what draw refuses, leaving the pass as it was.
    #[test]
    fn a_render_program_records_into_a_pass_the_caller_began() {
        let Some(gpu) = gpu() else { return };
        let program = gpu.render_program(POSITIONED, &RenderOptions { sample_count: 4, depth: true, ..Default::default() }).expect("the program");
        assert_eq!((program.format(), program.sample_count(), program.depth_format()), (wgpu::TextureFormat::Rgba8Unorm, 4, Some(wgpu::TextureFormat::Depth32Float)));
        let plain = gpu.render_program(FULLSCREEN, &RenderOptions::default()).expect("the program");
        assert_eq!((plain.sample_count(), plain.depth_format()), (1, None));
        let whole: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
        let positions = gpu.create_buffer(bytemuck::cast_slice(&whole), wgpu::BufferUsages::VERTEX, "positions").expect("the vertices");
        let target = gpu.create_texture(4, 4, program.format(), None, "target").expect("the texture");
        let attachment = |format: wgpu::TextureFormat, label: &str| {
            gpu.device().create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: 4, height: 4, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: program.sample_count(),
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        };
        let color = attachment(program.format(), "color").create_view(&wgpu::TextureViewDescriptor::default());
        let depth = attachment(program.depth_format().expect("a depth format"), "depth").create_view(&wgpu::TextureViewDescriptor::default());
        let resolved = target.create_view(&wgpu::TextureViewDescriptor::default());
        let vertices = [("position", &positions)];
        let mut encoder = gpu.device().create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("caller's encoder") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("caller's pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color,
                    depth_slice: None,
                    resolve_target: Some(&resolved),
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLUE), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            program.record(&mut pass, &Draw { vertex_count: 3, vertices: &vertices, clear: None, ..Default::default() }).expect("the record");
            let id = |r: PsResult<()>| r.expect_err("refused").error_id;
            assert_eq!(id(program.record(&mut pass, &Draw { vertex_count: 3, vertices: &vertices, ..Default::default() })), "WgpuRecordClear");
            assert_eq!(id(program.record(&mut pass, &Draw { vertex_count: 3, clear: None, ..Default::default() })), "WgpuNoVertices");
            let constant = Some(wgpu::Color::WHITE);
            assert_eq!(id(program.record(&mut pass, &Draw { vertex_count: 3, vertices: &vertices, clear: None, blend_constant: constant, ..Default::default() })), "WgpuBlendConstant");
        }
        gpu.queue().submit([encoder.finish()]);
        let recorded = gpu.read_texture(&target).expect("read");
        assert!(every_pixel(&recorded, [0, 255, 0, 255]), "{:?}", &recorded[..4]);
        assert!(gpu.take_errors().is_empty());
        let drawn = gpu.create_texture(4, 4, program.format(), None, "drawn").expect("the texture");
        program.draw(&drawn, &Draw { vertex_count: 3, vertices: &vertices, ..Default::default() }).expect("the draw");
        assert_eq!(gpu.read_texture(&drawn).expect("read"), recorded);
    }

    /// A draw's own depth target starts where clear_depth says: a triangle
    /// at depth 0.5 tested Greater is drawn against 0 and nowhere against
    /// the 1 the target starts at by default. A start for an aspect the
    /// target lacks, and a depth format that is a color format, are
    /// refused.
    #[test]
    fn a_draws_depth_target_starts_at_its_clear_depth() {
        let Some(gpu) = gpu() else { return };
        let target = gpu.create_texture(4, 4, wgpu::TextureFormat::Rgba8Unorm, None, "target").expect("the texture");
        let greater = DepthTest { compare: Some(CompareFunction::Greater), ..DepthTest::default() };
        let program = gpu.render_program(HALF_DEEP, &RenderOptions { depth: true, depth_test: greater, ..Default::default() }).expect("the program");
        program.draw(&target, &Draw { vertex_count: 3, clear_depth: Some(0.0), ..Default::default() }).expect("the draw");
        assert!(every_pixel(&gpu.read_texture(&target).expect("read"), [0, 0, 255, 255]));
        program.draw(&target, &Draw { vertex_count: 3, ..Default::default() }).expect("the draw");
        assert!(every_pixel(&gpu.read_texture(&target).expect("read"), [0, 0, 0, 255]));
        let stencil = program.draw(&target, &Draw { vertex_count: 3, clear_stencil: Some(1), ..Default::default() }).expect_err("refused");
        assert_eq!(stencil.error_id, "WgpuClearStencil");
        match gpu.render_program(HALF_DEEP, &RenderOptions { depth_format: Some(wgpu::TextureFormat::Rgba8Unorm), ..Default::default() }) {
            Ok(_) => panic!("a color format was taken as a depth format"),
            Err(e) => assert_eq!(e.error_id, "WgpuDepthFormat"),
        }
    }

    /// A stencil test masks a draw. In one pass of the caller's, on a
    /// Stencil8 target, a program writes the reference where its triangle
    /// covers the left pixel, and a full-screen program that passes only
    /// where the stencil equals the reference, the pass's after the first
    /// draw set it, colors that pixel alone. A depth start given to record
    /// and a reference given to a program with no stencil test are refused.
    #[test]
    fn a_stencil_test_masks_a_draw_to_where_a_first_one_wrote_the_reference() {
        let Some(gpu) = gpu() else { return };
        let format = wgpu::TextureFormat::Stencil8;
        let replace = StencilTest { front: StencilFace { pass: Some(StencilOperation::Replace), ..StencilFace::default() }, ..StencilTest::default() };
        let writer = gpu.render_program(POSITIONED, &RenderOptions { depth_format: Some(format), stencil: replace, ..Default::default() }).expect("the writer");
        let equal = StencilTest { front: StencilFace { compare: Some(CompareFunction::Equal), ..StencilFace::default() }, ..StencilTest::default() };
        let masked = gpu.render_program(FULLSCREEN, &RenderOptions { depth_format: Some(format), stencil: equal, ..Default::default() }).expect("the masked program");
        assert_eq!(masked.depth_format(), Some(format));
        let target = gpu.create_texture(2, 1, wgpu::TextureFormat::Rgba8Unorm, None, "target").expect("the texture");
        let stencil = gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("stencil"),
            size: wgpu::Extent3d { width: 2, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let color = target.create_view(&wgpu::TextureViewDescriptor::default());
        let stencil_view = stencil.create_view(&wgpu::TextureViewDescriptor::default());
        let left_half: [f32; 6] = [-1.0, -1.0, 0.0, -1.0, -1.0, 3.0];
        let positions = gpu.create_buffer(bytemuck::cast_slice(&left_half), wgpu::BufferUsages::VERTEX, "positions").expect("the vertices");
        let vertices = [("position", &positions)];
        let mut encoder = gpu.device().create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("caller's encoder") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("caller's pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLUE), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &stencil_view,
                    depth_ops: None,
                    stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0), store: wgpu::StoreOp::Store }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            writer.record(&mut pass, &Draw { vertex_count: 3, vertices: &vertices, clear: None, stencil_reference: Some(1), ..Default::default() }).expect("the writer's record");
            masked.record(&mut pass, &Draw { vertex_count: 3, clear: None, ..Default::default() }).expect("the masked record");
            let started = masked.record(&mut pass, &Draw { vertex_count: 3, clear: None, clear_depth: Some(0.0), ..Default::default() }).expect_err("refused");
            assert_eq!(started.error_id, "WgpuRecordClear");
        }
        gpu.queue().submit([encoder.finish()]);
        assert_eq!(gpu.read_texture(&target).expect("read"), [255, 0, 0, 255, 0, 0, 255, 255]);
        assert!(gpu.take_errors().is_empty());
        let plain = gpu.render_program(FULLSCREEN, &RenderOptions::default()).expect("the program");
        let unread = plain.draw(&target, &Draw { vertex_count: 3, stencil_reference: Some(1), ..Default::default() }).expect_err("refused");
        assert_eq!(unread.error_id, "WgpuStencilReference");
    }

    /// A dispatch recorded into a pass the caller began runs only when the
    /// caller submits, doubling a buffer as run does, and says the
    /// workgroups it dispatches; a timed spec is refused, and so is what
    /// run refuses.
    #[test]
    fn a_compute_program_records_into_a_pass_the_caller_began() {
        let Some(gpu) = gpu() else { return };
        let program = gpu.compute_program(DOUBLE, None, &[]).expect("the program");
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let data = gpu.create_buffer(bytemuck::cast_slice(&[1.0f32, 2.0, 3.0]), usage, "data").expect("the buffer");
        let bindings = [("v", Binding::Buffer(&data))];
        let mut encoder = gpu.device().create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("caller's encoder") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("caller's pass"), timestamp_writes: None });
            let by_invocations = RunSpec { bindings: &bindings, dispatch: Dispatch::Invocations([3, 1, 1]), ..Default::default() };
            assert_eq!(program.record(&mut pass, &by_invocations).expect("the record"), [1, 1, 1]);
            assert_eq!(program.record(&mut pass, &RunSpec { bindings: &bindings, ..Default::default() }).expect("the record"), [1, 1, 1]);
            let id = |r: PsResult<[u32; 3]>| r.expect_err("refused").error_id;
            assert_eq!(id(program.record(&mut pass, &RunSpec { bindings: &bindings, timed: true, ..Default::default() })), "WgpuRecordTimed");
            assert_eq!(id(program.record(&mut pass, &RunSpec::default())), "WgpuBindingMissing");
        }
        assert_eq!(floats(&gpu.read_buffer(&data, 0, 12).expect("read")), [1.0, 2.0, 3.0], "nothing runs before the caller submits");
        gpu.queue().submit([encoder.finish()]);
        assert_eq!(floats(&gpu.read_buffer(&data, 0, 12).expect("read")), [4.0, 8.0, 12.0]);
        assert!(gpu.take_errors().is_empty());
    }

    /// A device opened with wgpu itself and taken by from_parts runs
    /// compute and render programs, and a validation error raised on it
    /// outside any call is kept and returned by the next call, where
    /// wgpu's own handler would have ended the process.
    #[test]
    fn a_gpu_from_parts_runs_programs_and_keeps_an_error_raised_outside_a_call() {
        let Some(opened) = gpu() else { return };
        let adapter = opened.adapter().clone();
        let descriptor = wgpu::DeviceDescriptor { label: Some("adopted"), required_limits: adapter.limits(), ..Default::default() };
        let (device, queue) = pollster::block_on(adapter.request_device(&descriptor)).expect("a device opens");
        let gpu = Gpu::from_parts(adapter, device, queue, "adopted".to_string()).expect("the parts");
        assert_eq!(gpu.label(), "adopted");
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let data = gpu.create_buffer(bytemuck::cast_slice(&[1.0f32, 2.0, 3.0]), usage, "data").expect("the buffer");
        gpu.compute_program(DOUBLE, None, &[]).expect("the program").run(&[("v", Binding::Buffer(&data))], [1, 1, 1]).expect("the run");
        assert_eq!(floats(&gpu.read_buffer(&data, 0, 12).expect("read")), [2.0, 4.0, 6.0]);
        let target = gpu.create_texture(4, 4, wgpu::TextureFormat::Rgba8Unorm, None, "target").expect("the texture");
        let program = gpu.render_program(FULLSCREEN, &RenderOptions::default()).expect("the program");
        program.draw(&target, &Draw { vertex_count: 3, ..Default::default() }).expect("the draw");
        assert!(every_pixel(&gpu.read_texture(&target).expect("read"), [255, 0, 0, 255]));
        let mappable_storage = wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::STORAGE;
        drop(gpu.device().create_buffer(&wgpu::BufferDescriptor { label: Some("refused"), size: 4, usage: mappable_storage, mapped_at_creation: false }));
        let kept = gpu.create_buffer(&[0; 4], usage, "after").expect_err("the error raised outside a call");
        assert_eq!(kept.error_id, "WgpuUncaptured");
    }

    /// The errors raised outside any call are taken oldest first and
    /// cleared, so a second take is empty and the next call succeeds.
    #[test]
    fn takes_the_errors_raised_outside_a_call_and_clears_them() {
        let Some(gpu) = gpu() else { return };
        assert_eq!(gpu.take_errors(), Vec::<String>::new());
        // A buffer mapped at creation takes a size in whole words, whatever
        // features the device has.
        for label in ["first", "second"] {
            let _refused = gpu.device().create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: 3, usage: wgpu::BufferUsages::COPY_DST, mapped_at_creation: true });
        }
        let taken = gpu.take_errors();
        assert_eq!(taken.len(), 2, "{taken:?}");
        assert!(taken[0].contains("'first'") && taken[1].contains("'second'"), "{taken:?}");
        assert_eq!(gpu.take_errors(), Vec::<String>::new());
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        gpu.create_buffer(&[0; 4], usage, "after").expect("a call after the take");
        assert_eq!(gpu.lost_reason(), None);
    }

    #[test]
    fn a_device_is_used_from_other_threads() {
        let Some(gpu) = gpu() else { return };
        let program = gpu.compute_program(DOUBLE, None, &[]).expect("the program");
        let workers: Vec<_> = (1..=4)
            .map(|n| {
                let (gpu, program) = (gpu.clone(), program.clone());
                std::thread::spawn(move || {
                    let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
                    let data = gpu.create_buffer(bytemuck::cast_slice(&[n as f32; 4]), usage, "data").expect("the buffer");
                    program.run(&[("v", Binding::Buffer(&data))], [1, 1, 1]).expect("the run");
                    floats(&gpu.read_buffer(&data, 0, 16).expect("read"))
                })
            })
            .collect();
        for (n, worker) in (1..=4).zip(workers) {
            assert_eq!(worker.join().expect("the worker"), [2.0 * n as f32; 4]);
        }
    }

    #[test]
    fn an_instance_lists_its_adapters_once() {
        let options = InstanceOptions::default();
        let first = device::adapters(&options).expect("the adapters");
        let again = device::adapters(&options).expect("the adapters");
        assert!(Arc::ptr_eq(&first, &again));
    }

    #[test]
    fn reads_on_many_threads_at_once_all_land() {
        let Some(gpu) = gpu() else { return };
        let usage = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC;
        let readers: Vec<_> = (0..8u32)
            .map(|n| {
                let gpu = gpu.clone();
                std::thread::spawn(move || {
                    let buffer = gpu.create_buffer(bytemuck::cast_slice(&[n; 4]), usage, "reader").expect("the buffer");
                    for _ in 0..50 {
                        let bytes = gpu.read_buffer(&buffer, 0, 16).expect("read");
                        assert_eq!(bytemuck::pod_collect_to_vec::<u8, u32>(&bytes), [n; 4]);
                    }
                })
            })
            .collect();
        for reader in readers {
            reader.join().expect("a reader");
        }
    }

    /// Opens and drops devices while four other threads keep starting a
    /// thread that lists the adapters, as Get-WgpuAdapter does, and exits.
    /// A device that has not opened in thirty seconds means the process is
    /// deadlocked, so the watchdog aborts it rather than leave the run
    /// hanging.
    #[test]
    fn devices_open_while_threads_that_list_adapters_exit() {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::time::Duration;

        let Some(gpu) = gpu() else { return };
        drop(gpu);
        let stop = Arc::new(AtomicBool::new(false));
        let opened = Arc::new(AtomicU64::new(0));
        let churners: Vec<_> = (0..4)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let lister = std::thread::spawn(|| drop(device::adapters(&InstanceOptions::default()).expect("the adapters")));
                        lister.join().expect("the lister");
                    }
                })
            })
            .collect();
        let watchdog = {
            let (stop, opened) = (stop.clone(), opened.clone());
            std::thread::spawn(move || {
                let (mut seen, mut since) = (0, Instant::now());
                while !stop.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(100));
                    let now = opened.load(Ordering::Relaxed);
                    if now != seen {
                        (seen, since) = (now, Instant::now());
                    }
                    if since.elapsed() > Duration::from_secs(30) {
                        eprintln!("no device opened in 30 seconds after {seen}: the process is deadlocked");
                        std::process::abort();
                    }
                }
            })
        };
        for _ in 0..20 {
            drop(Gpu::open(&options()).expect("a device opens"));
            opened.fetch_add(1, Ordering::Relaxed);
        }
        stop.store(true, Ordering::Relaxed);
        for churner in churners {
            churner.join().expect("a churner");
        }
        watchdog.join().expect("the watchdog");
    }
}
