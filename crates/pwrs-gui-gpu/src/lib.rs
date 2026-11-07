//! PWRS-GUI's GPU layer: the device a renderer's windows and GPU widgets
//! draw on, taken from the renderer as a `wgpu-pwsh` [`Gpu`], and the
//! buffers and render pipelines a session makes on it, by number, with the
//! plans GPU surfaces draw from them ([`surface`]). A layer belongs to the
//! thread that runs its renderer.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub mod surface;
pub mod texture;

pub use surface::{DrawPipeline, Every, FRAME_HEADER, FRAME_SLOT, FrameHeader, IndexFormat, Pipeline, PipelineInfo, SURFACE_FORMAT, SurfaceDraw, SurfacePass, SurfacePlan, SurfaceSet};
pub use texture::{AddressMode, Filter, SamplerSpec, TextureFormat, TextureInfo};
pub use wgpu_pwsh::wgpu;
pub use wgpu_pwsh::{
    Binding, BindingInfo, BlendFactor, BlendMode, BlendOperation, CompareFunction, ComputeProgram, CullMode, CustomBlend, DepthTest, ElementType, Gpu, GpuOptions, PrimitiveTopology, PsError,
    RenderOptions, RenderProgram, StencilFace, StencilOperation, StencilTest,
};

/// What a buffer can be used for when its maker names nothing, as the Wgpu
/// module's New-WgpuBuffer gives: every usage but mapping and
/// acceleration-structure input.
pub const DEFAULT_USAGE: wgpu::BufferUsages = wgpu::BufferUsages::STORAGE
    .union(wgpu::BufferUsages::UNIFORM)
    .union(wgpu::BufferUsages::VERTEX)
    .union(wgpu::BufferUsages::INDEX)
    .union(wgpu::BufferUsages::INDIRECT)
    .union(wgpu::BufferUsages::QUERY_RESOLVE)
    .union(wgpu::BufferUsages::COPY_SRC)
    .union(wgpu::BufferUsages::COPY_DST);

/// How long [`Layer::destroy_device`] waits for a destroyed device to
/// report itself lost, which it does once the GPU has finished the work
/// submitted before the destroy.
pub const DESTROY_WAIT: Duration = Duration::from_secs(10);

/// Why a GPU-layer call failed: the error id `wgpu-pwsh` gives the failure,
/// or the layer's own, and its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub id: String,
    pub text: String,
}

impl Failure {
    fn new(id: &str, text: String) -> Failure {
        Failure { id: id.to_string(), text }
    }
}

impl From<PsError> for Failure {
    fn from(e: PsError) -> Failure {
        Failure { id: e.error_id, text: e.message }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.text, self.id)
    }
}

/// Which of a compute shader's bindings a resource is bound to, or which of
/// a surface draw's vertex inputs or bindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slot {
    /// The binding at this place in the order the shader declares them,
    /// by group and then binding.
    Order(usize),
    /// The binding whose variable has this name, ignoring case, or
    /// `group:binding`.
    Name(String),
}

/// How many workgroups a compute run dispatches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dispatch {
    /// Workgroups in x, y and z.
    Workgroups([u32; 3]),
    /// Invocations in x, y and z, divided by the shader's workgroup size
    /// and rounded up.
    Invocations([u64; 3]),
    /// Workgroups the GPU reads as three u32 from byte `offset` of table
    /// buffer `buffer`.
    Indirect { buffer: u64, offset: u64 },
    /// One invocation per element of the first buffer bound, in x.
    Auto,
}

/// The language of a compute shader's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Wgsl,
    /// GLSL, whose entry point is `main`.
    Glsl,
}

/// One compute run: the shader and how it runs.
#[derive(Clone, Debug, PartialEq)]
pub struct Run {
    pub source: String,
    pub language: Language,
    /// The entry point; the shader's only compute entry point when `None`.
    pub entry: Option<String>,
    /// Override constants by name or `@id`, or a GLSL shader's
    /// specialization constants by `constant_id`.
    pub constants: Vec<(String, f64)>,
    pub bindings: Vec<(Slot, Resource)>,
    pub dispatch: Dispatch,
    /// The immediate data the entry point reads.
    pub immediates: Option<Vec<u8>>,
    /// Whether to measure the GPU's time in the pass, where the device can.
    pub timed: bool,
}

/// What a compute run did: the arrays its shader may have written, by
/// group and then binding, the workgroups dispatched (0 for an indirect
/// dispatch), the GPU's time in the pass when it was measured, and the
/// call's whole time.
#[derive(Clone, Debug, PartialEq)]
pub struct Ran {
    pub written: Vec<Written>,
    pub workgroups: [u32; 3],
    pub gpu_time_ns: Option<f64>,
    pub cpu_time_ms: f64,
}

/// What a compute shader's binding is given, or a surface draw's vertex
/// input or binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource {
    /// A buffer in the layer's table, which stays on the GPU.
    Buffer(u64),
    /// Elements of `element`, copied to the GPU as the type the shader
    /// declares for the binding or vertex input, or as they are where it
    /// declares none.
    Array { element: ElementType, bytes: Vec<u8> },
    /// A texture in the layer's table, for a binding alone.
    Texture(u64),
    /// A sampler in the layer's table, for a binding alone.
    Sampler(u64),
}

/// An array a compute run's shader may have written, as it left it: the
/// binding's name, the element type it was copied as, and its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Written {
    pub binding: String,
    pub element: ElementType,
    pub bytes: Vec<u8>,
}

/// What a buffer contains as far as the session wrote it, which fills it
/// again on a new device. Work the GPU did on it is not saved.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Kept {
    Zeros,
    Bytes(Vec<u8>),
}

struct Entry {
    buffer: wgpu::Buffer,
    label: String,
    usage: wgpu::BufferUsages,
    /// The length the session requested; the buffer itself is rounded up
    /// to a multiple of 4 bytes.
    length: u64,
    kept: Kept,
}

/// The GPU layer of one renderer: its device as a [`Gpu`], and the
/// buffers, render pipelines, textures and samplers made on it, each under
/// a number of one count that stays its own when the layer moves to a new
/// device.
pub struct Layer {
    gpu: Gpu,
    label: String,
    buffers: BTreeMap<u64, Entry>,
    pipelines: BTreeMap<u64, surface::PipelineEntry>,
    textures: BTreeMap<u64, texture::TextureEntry>,
    samplers: BTreeMap<u64, texture::SamplerEntry>,
    next: u64,
}

impl Layer {
    /// The layer on a device a renderer opened on `adapter`. It installs
    /// `wgpu-pwsh`'s uncaptured-error and device-lost callbacks on the
    /// device, in place of any it had, so an error raised outside any
    /// call waits for [`Layer::take_errors`].
    pub fn adopt(adapter: wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue, label: &str) -> Result<Layer, Failure> {
        let gpu = Gpu::from_parts(adapter, device, queue, label.to_string())?;
        Ok(Layer { gpu, label: label.to_string(), buffers: BTreeMap::new(), pipelines: BTreeMap::new(), textures: BTreeMap::new(), samplers: BTreeMap::new(), next: 1 })
    }

    /// The device, for the widgets that draw on it.
    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    /// The numbers of the buffers in the table, in ascending order.
    pub fn buffers(&self) -> Vec<u64> {
        self.buffers.keys().copied().collect()
    }

    /// A buffer containing `bytes`, labeled `label` or by its number, with
    /// `usage` or [`DEFAULT_USAGE`], and its number.
    pub fn upload(&mut self, bytes: &[u8], label: Option<&str>, usage: Option<wgpu::BufferUsages>) -> Result<u64, Failure> {
        let id = self.next;
        let label = label.map_or_else(|| buffer_label(id), str::to_string);
        let usage = usage.unwrap_or(DEFAULT_USAGE);
        let buffer = self.gpu.create_buffer(bytes, usage, &label)?;
        self.next += 1;
        self.buffers.insert(id, Entry { buffer, label, usage, length: bytes.len() as u64, kept: Kept::Bytes(bytes.to_vec()) });
        Ok(id)
    }

    /// A buffer of `length` zeroed bytes, labeled `label` or by its number,
    /// with `usage` or [`DEFAULT_USAGE`], and its number.
    pub fn zeros(&mut self, length: u64, label: Option<&str>, usage: Option<wgpu::BufferUsages>) -> Result<u64, Failure> {
        let id = self.next;
        let label = label.map_or_else(|| buffer_label(id), str::to_string);
        let usage = usage.unwrap_or(DEFAULT_USAGE);
        let buffer = self.gpu.create_zeroed_buffer(length, usage, &label)?;
        self.next += 1;
        self.buffers.insert(id, Entry { buffer, label, usage, length, kept: Kept::Zeros });
        Ok(id)
    }

    /// Writes `bytes` into buffer `id` from byte `offset`.
    pub fn write(&mut self, id: u64, offset: u64, bytes: &[u8]) -> Result<(), Failure> {
        let entry = self.buffers.get_mut(&id).ok_or_else(|| no_buffer(id))?;
        let end = within(id, entry.length, offset, bytes.len() as u64)?;
        self.gpu.write_buffer(&entry.buffer, offset, bytes)?;
        if matches!(entry.kept, Kept::Zeros) {
            entry.kept = Kept::Bytes(vec![0; to_usize(entry.length)?]);
        }
        if let Kept::Bytes(kept) = &mut entry.kept {
            kept[to_usize(offset)?..to_usize(end)?].copy_from_slice(bytes);
        }
        Ok(())
    }

    /// `length` bytes of buffer `id` from byte `offset`, or every byte
    /// from `offset` to its end, once the GPU has finished the work
    /// submitted before.
    pub fn download(&self, id: u64, offset: u64, length: Option<u64>) -> Result<Vec<u8>, Failure> {
        let entry = self.buffers.get(&id).ok_or_else(|| no_buffer(id))?;
        let length = match length {
            Some(length) => length,
            None => entry.length.checked_sub(offset).ok_or_else(|| outside(id, entry.length, offset, 0))?,
        };
        within(id, entry.length, offset, length)?;
        Ok(self.gpu.read_buffer(&entry.buffer, offset, length)?)
    }

    /// The length of buffer `id` in bytes, as the session requested it.
    pub fn length(&self, id: u64) -> Result<u64, Failure> {
        self.buffers.get(&id).map(|entry| entry.length).ok_or_else(|| no_buffer(id))
    }

    /// Removes buffer `id` from the table. Its memory goes when nothing
    /// still uses it: at once, or once the surface plans made with it are
    /// let go.
    pub fn free(&mut self, id: u64) -> Result<(), Failure> {
        self.buffers.remove(&id).map(|_| ()).ok_or_else(|| no_buffer(id))
    }

    /// Runs `run`'s compute shader with each resource bound at its slot and
    /// waits for the GPU to finish, as the Wgpu module's Invoke-WgpuCompute
    /// runs one. A table buffer is bound as it is. An array is copied to the
    /// GPU as the element type the shader declares for its binding, and
    /// comes back as the shader left it when the shader may write that
    /// binding.
    pub fn compute(&self, run: Run) -> Result<Ran, Failure> {
        let program = self.compute_program(&run.source, run.language, run.entry.as_deref(), &run.constants)?;
        let bindings = &run.bindings;
        let declared = program.binding_info();
        let mut chosen: Vec<&BindingInfo> = Vec::with_capacity(bindings.len());
        for (slot, _) in bindings {
            let info = binding_of(slot, &declared)?;
            if chosen.iter().any(|c| c.group == info.group && c.binding == info.binding) {
                return Err(twice(info));
            }
            chosen.push(info);
        }
        let mut given = Vec::with_capacity(bindings.len());
        for (info, (_, resource)) in chosen.iter().zip(bindings) {
            given.push(match resource {
                Resource::Buffer(id) => Given::Table(&self.buffers.get(id).ok_or_else(|| no_buffer(*id))?.buffer),
                Resource::Array { element, bytes } => {
                    let as_type = info.element_type.unwrap_or(*element);
                    let data = element.convert(bytes, as_type);
                    let buffer = self.gpu.create_buffer(&data, DEFAULT_USAGE, &format!("PWRS-GUI {} array", info.name))?;
                    Given::Copy { buffer, element: as_type, length: data.len() as u64 }
                }
                Resource::Texture(id) => Given::Texture(self.table_texture(*id)?),
                Resource::Sampler(id) => Given::Sampler(self.table_sampler(*id)?),
            });
        }
        let keys: Vec<String> = chosen.iter().map(|b| format!("{}:{}", b.group, b.binding)).collect();
        let bound: Vec<(&str, Binding<'_>)> = keys
            .iter()
            .zip(&given)
            .map(|(key, g)| match g {
                Given::Table(buffer) => (key.as_str(), Binding::Buffer(buffer)),
                Given::Copy { buffer, .. } => (key.as_str(), Binding::Buffer(buffer)),
                Given::Texture(texture) => (key.as_str(), Binding::Texture(texture)),
                Given::Sampler(sampler) => (key.as_str(), Binding::Sampler(sampler)),
            })
            .collect();
        let dispatch = match run.dispatch {
            Dispatch::Workgroups(counts) => wgpu_pwsh::Dispatch::Workgroups(counts),
            Dispatch::Invocations(counts) => wgpu_pwsh::Dispatch::Invocations(counts),
            Dispatch::Indirect { buffer, offset } => wgpu_pwsh::Dispatch::Indirect { buffer: &self.buffers.get(&buffer).ok_or_else(|| no_buffer(buffer))?.buffer, offset },
            Dispatch::Auto => wgpu_pwsh::Dispatch::Auto,
        };
        let report = program.run_with(&wgpu_pwsh::RunSpec { bindings: &bound, dispatch, immediates: run.immediates.as_deref(), timed: run.timed })?;
        let mut written = Vec::new();
        for (info, g) in chosen.iter().zip(&given) {
            if let Given::Copy { buffer, element, length } = g
                && info.writable
            {
                written.push(((info.group, info.binding), Written { binding: info.name.clone(), element: *element, bytes: self.gpu.read_buffer(buffer, 0, *length)? }));
            }
        }
        written.sort_by_key(|(at, _)| *at);
        let written = written.into_iter().map(|(_, w)| w).collect();
        Ok(Ran { written, workgroups: report.workgroups, gpu_time_ns: report.gpu_time_ns, cpu_time_ms: report.cpu_time_ms })
    }

    /// The compute program of `source` in `language`, at entry point `entry`
    /// (the shader's only one when `None`; a GLSL shader's is `main`), with
    /// `constants` by name, `@id` or GLSL `constant_id`.
    pub(crate) fn compute_program(&self, source: &str, language: Language, entry: Option<&str>, constants: &[(String, f64)]) -> Result<ComputeProgram, Failure> {
        let constants: Vec<(&str, f64)> = constants.iter().map(|(name, value)| (name.as_str(), *value)).collect();
        match language {
            Language::Wgsl => Ok(self.gpu.compute_program(source, entry, &constants)?),
            Language::Glsl => {
                if let Some(entry) = entry
                    && entry != "main"
                {
                    return Err(Failure::new("GuiGpuEntry", format!("a GLSL compute shader's entry point is main, and {entry} was given")));
                }
                Ok(self.gpu.compute_program_glsl(source, &constants)?)
            }
        }
    }

    /// The errors the device raised outside any call of this layer since the
    /// last take, the renderer's own drawing among them, oldest first.
    pub fn take_errors(&self) -> Vec<String> {
        self.gpu.take_errors()
    }

    /// Why the device was lost, if it was.
    pub fn lost_reason(&self) -> Option<String> {
        self.gpu.lost_reason()
    }

    /// Destroys the device, as a driver reset loses one, waits for it to
    /// report itself lost, and returns the reason it gives for the loss; the
    /// renderer drawing on it loses it too. wgpu reports a destroyed device
    /// lost at the first poll that finds its queue empty, and refuses any
    /// submission made after the destroy, so the layer polls for the last
    /// submission that succeeded rather than submitting one to wait for.
    /// Refused as GuiGpuNotLost when the device has not reported the loss
    /// within [`DESTROY_WAIT`].
    pub fn destroy_device(&self) -> Result<String, Failure> {
        let device = self.gpu.device();
        device.destroy();
        let deadline = Instant::now() + DESTROY_WAIT;
        loop {
            if let Some(reason) = self.gpu.lost_reason() {
                return Ok(reason);
            }
            if Instant::now() >= deadline {
                return Err(Failure::new("GuiGpuNotLost", format!("the device was destroyed and did not report itself lost within {} s", DESTROY_WAIT.as_secs())));
            }
            match device.poll(wgpu::PollType::Wait { submission_index: None, timeout: Some(Duration::from_millis(100)) }) {
                Ok(_) | Err(wgpu::PollError::Timeout) => {}
                Err(e) => return Err(Failure::new("GuiGpuNotLost", format!("the device was destroyed and waiting for it to report itself lost failed: {e}"))),
            }
        }
    }

    /// Moves the layer to a device the renderer opened in place of a lost
    /// one: every buffer is made again under its number and filled from
    /// what the session wrote into it, so what the GPU computed into a
    /// buffer is not there, and every render pipeline is built again under
    /// its number.
    pub fn rebuild(&mut self, adapter: wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue) -> Result<(), Failure> {
        let gpu = Gpu::from_parts(adapter, device, queue, self.label.clone())?;
        let mut made = BTreeMap::new();
        for (id, entry) in &self.buffers {
            let buffer = match &entry.kept {
                Kept::Zeros => gpu.create_zeroed_buffer(entry.length, entry.usage, &entry.label)?,
                Kept::Bytes(bytes) => gpu.create_buffer(bytes, entry.usage, &entry.label)?,
            };
            made.insert(*id, Entry { buffer, label: entry.label.clone(), usage: entry.usage, length: entry.length, kept: entry.kept.clone() });
        }
        self.rebuild_textures(&gpu)?;
        self.gpu = gpu;
        self.buffers = made;
        self.rebuild_pipelines()
    }
}

/// What a compute run binds for one resource: a buffer of the table, the
/// GPU copy of an array with the element type it was copied as and its
/// length in bytes, or a texture or a sampler of the table.
enum Given<'a> {
    Table(&'a wgpu::Buffer),
    Copy { buffer: wgpu::Buffer, element: ElementType, length: u64 },
    Texture(&'a wgpu::Texture),
    Sampler(&'a wgpu::Sampler),
}

/// The binding `slot` names of `declared`, a shader's bindings in group and
/// binding order: the one at its place, the one of its `group:binding`, or
/// the one of its name, without regard to case.
pub(crate) fn binding_of<'a>(slot: &Slot, declared: &'a [BindingInfo]) -> Result<&'a BindingInfo, Failure> {
    let found = match slot {
        Slot::Order(i) => declared.get(*i),
        Slot::Name(name) => match name.split_once(':') {
            Some((g, b)) => {
                let number = |part: &str| part.trim().parse::<u32>().map_err(|e| Failure::new("GuiGpuBinding", format!("{name} is neither a binding name nor group:binding: {e}")));
                let (g, b) = (number(g)?, number(b)?);
                declared.iter().find(|d| d.group == g && d.binding == b)
            }
            None => declared.iter().find(|d| d.name.eq_ignore_ascii_case(name)),
        },
    };
    found.ok_or_else(|| {
        let given = match slot {
            Slot::Order(i) => format!("binding {i} in declared order"),
            Slot::Name(name) => format!("binding {name}"),
        };
        let names: Vec<String> = declared.iter().map(|b| format!("{} ({}:{})", b.name, b.group, b.binding)).collect();
        Failure::new("GuiGpuBinding", format!("{given} was given, and the shader declares {}: {}", declared.len(), names.join(", ")))
    })
}

/// The refusal of binding `info` given twice.
pub(crate) fn twice(info: &BindingInfo) -> Failure {
    Failure::new("GuiGpuBinding", format!("binding {} ({}:{}) was given twice", info.name, info.group, info.binding))
}

fn buffer_label(id: u64) -> String {
    format!("PWRS-GUI buffer {id}")
}

fn no_buffer(id: u64) -> Failure {
    Failure::new("GuiGpuNoBuffer", format!("there is no GPU buffer {id}"))
}

fn outside(id: u64, length: u64, offset: u64, count: u64) -> Failure {
    Failure::new("GuiGpuRange", format!("{count} bytes from byte {offset} are outside GPU buffer {id}, which has {length} bytes"))
}

/// The end of `count` bytes from `offset`, when they lie inside a buffer
/// of `length` bytes.
fn within(id: u64, length: u64, offset: u64, count: u64) -> Result<u64, Failure> {
    match offset.checked_add(count) {
        Some(end) if end <= length => Ok(end),
        _ => Err(outside(id, length, offset, count)),
    }
}

fn to_usize(n: u64) -> Result<usize, Failure> {
    usize::try_from(n).map_err(|e| Failure::new("GuiGpuRange", format!("{n} bytes is more than this process can address: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layer on a device of wgpu's choosing, opened as a renderer opens
    /// its own and then taken.
    fn layer() -> Layer {
        let gpu = Gpu::open(&GpuOptions::default()).expect("a device opens");
        Layer::adopt(gpu.adapter().clone(), gpu.device().clone(), gpu.queue().clone(), "PWRS-GUI test").expect("the layer takes the device")
    }

    fn floats(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    fn read_floats(bytes: &[u8]) -> Vec<f32> {
        bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
    }

    /// A run of WGSL `source` with no constants, immediates or timing.
    fn wgsl(source: &str, bindings: Vec<(Slot, Resource)>, dispatch: Dispatch) -> Run {
        Run { source: source.to_string(), language: Language::Wgsl, entry: None, constants: Vec::new(), bindings, dispatch, immediates: None, timed: false }
    }

    #[test]
    fn a_buffer_reads_back_what_was_uploaded_until_it_is_freed() {
        let mut layer = layer();
        let id = layer.upload(&[1, 2, 3, 4, 5], None, None).expect("uploads");
        assert_eq!(layer.length(id).expect("reads the length"), 5);
        assert_eq!(layer.download(id, 0, None).expect("downloads it whole"), vec![1, 2, 3, 4, 5]);
        assert_eq!(layer.download(id, 3, Some(2)).expect("downloads a part"), vec![4, 5]);
        assert_eq!(layer.download(id, 4, Some(2)).expect_err("a read past the end is refused").id, "GuiGpuRange");
        layer.free(id).expect("frees");
        assert_eq!(layer.download(id, 0, None).expect_err("a freed buffer is gone").id, "GuiGpuNoBuffer");
        assert_eq!(layer.free(id).expect_err("a freed buffer is not freed twice").id, "GuiGpuNoBuffer");
    }

    #[test]
    fn a_write_inside_a_word_keeps_the_bytes_around_it() {
        let mut layer = layer();
        let id = layer.zeros(10, Some("ten zeroed bytes"), None).expect("makes a zeroed buffer");
        layer.write(id, 3, &[7, 8, 9]).expect("writes");
        assert_eq!(layer.download(id, 0, None).expect("downloads"), vec![0, 0, 0, 7, 8, 9, 0, 0, 0, 0]);
        assert_eq!(layer.write(id, 8, &[1, 2, 3]).expect_err("a write past the end is refused").id, "GuiGpuRange");
    }

    #[test]
    fn compute_binds_buffers_by_order_by_name_or_by_group_and_binding() {
        let mut layer = layer();
        let input = layer.upload(&floats(&[1.0, 2.0, 3.0, 4.0]), None, None).expect("uploads");
        let output = layer.zeros(16, None, None).expect("makes a zeroed buffer");
        let source = "@group(0) @binding(1) var<storage, read_write> b: array<f32>;\n\
                      @group(0) @binding(0) var<storage, read> a: array<f32>;\n\
                      @compute @workgroup_size(2) fn main(@builtin(global_invocation_id) i: vec3<u32>) { b[i.x] = b[i.x] + a[i.x]; }";
        let table = |slot: Slot, id: u64| (slot, Resource::Buffer(id));
        let ways = [
            (vec![table(Slot::Order(0), input), table(Slot::Order(1), output)], Dispatch::Workgroups([2, 1, 1])),
            (vec![table(Slot::Name(String::from("A")), input), table(Slot::Name(String::from("b")), output)], Dispatch::Invocations([4, 1, 1])),
            (vec![table(Slot::Name(String::from("0:1")), output), table(Slot::Name(String::from(" 0 : 0")), input)], Dispatch::Invocations([3, 1, 1])),
        ];
        for (bindings, dispatch) in ways {
            let ran = layer.compute(wgsl(source, bindings, dispatch)).expect("runs");
            assert!(ran.written.is_empty(), "table buffers stay on the GPU: {:?}", ran.written);
        }
        assert_eq!(read_floats(&layer.download(output, 0, None).expect("downloads")), vec![3.0, 6.0, 9.0, 12.0], "three runs each added a, the last over 3 invocations rounded up to 2 workgroups of 2");
        let past = layer.compute(wgsl(source, vec![table(Slot::Order(2), input)], Dispatch::Workgroups([1, 1, 1]))).expect_err("a place past the shader's bindings is refused");
        assert_eq!(past.id, "GuiGpuBinding");
        assert!(past.text.contains("a (0:0), b (0:1)"), "the refusal names the bindings in declared order: {}", past.text);
        let twice = layer.compute(wgsl(source, vec![table(Slot::Order(0), input), table(Slot::Name(String::from("a")), input)], Dispatch::Workgroups([1, 1, 1])));
        assert_eq!(twice.expect_err("one binding given twice is refused").id, "GuiGpuBinding");
        let missing = layer.compute(wgsl(source, vec![table(Slot::Order(0), input), table(Slot::Order(1), 999)], Dispatch::Workgroups([1, 1, 1])));
        assert_eq!(missing.expect_err("a missing buffer is refused").id, "GuiGpuNoBuffer");
        let unnumbered = layer.compute(wgsl(source, vec![table(Slot::Name(String::from("0:b")), output)], Dispatch::Workgroups([1, 1, 1]))).expect_err("a group:binding that is not numbers is refused");
        assert_eq!(unnumbered.id, "GuiGpuBinding");
        assert!(unnumbered.text.starts_with("0:b is neither a binding name nor group:binding"), "{}", unnumbered.text);
    }

    #[test]
    fn arrays_a_shader_may_write_come_back_by_group_and_then_binding() {
        let layer = layer();
        let source = "@group(0) @binding(1) var<storage, read_write> late: array<f32>;\n\
                      @group(0) @binding(0) var<storage, read_write> early: array<f32>;\n\
                      @compute @workgroup_size(1) fn main() { early[0] = 1.0; late[0] = 2.0; }";
        let array = |v: f32| Resource::Array { element: ElementType::Float32, bytes: floats(&[v]) };
        let given = vec![(Slot::Name(String::from("late")), array(0.0)), (Slot::Name(String::from("early")), array(0.0))];
        let ran = layer.compute(wgsl(source, given, Dispatch::Workgroups([1, 1, 1]))).expect("runs");
        let order: Vec<(&str, Vec<f32>)> = ran.written.iter().map(|w| (w.binding.as_str(), read_floats(&w.bytes))).collect();
        assert_eq!(order, vec![("early", vec![1.0]), ("late", vec![2.0])], "given late first, they come back by binding");
    }

    #[test]
    fn constants_immediate_counts_on_the_gpu_and_glsl_run_as_the_wgpu_module_runs_them() {
        let mut layer = layer();
        let data = layer.upload(&floats(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]), None, None).expect("uploads");
        let scaled = "override scale: f32 = 1.0;\n\
                      @group(0) @binding(0) var<storage, read_write> v: array<f32>;\n\
                      @compute @workgroup_size(4) fn main(@builtin(global_invocation_id) i: vec3<u32>) { v[i.x] = v[i.x] * scale; }";
        let mut run = wgsl(scaled, vec![(Slot::Order(0), Resource::Buffer(data))], Dispatch::Auto);
        run.constants = vec![(String::from("scale"), 2.0)];
        run.timed = true;
        let ran = layer.compute(run).expect("runs with one invocation per element");
        assert_eq!(ran.workgroups, [2, 1, 1], "eight elements over a workgroup of four");
        assert!(ran.cpu_time_ms > 0.0, "the call's time is reported: {}", ran.cpu_time_ms);
        if layer.gpu().features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            assert!(ran.gpu_time_ns.is_some(), "a device with timestamps times the pass");
        }
        let counts = layer.upload(&[1u32, 1, 1].iter().flat_map(|c| c.to_le_bytes()).collect::<Vec<u8>>(), Some("counts"), None).expect("uploads the counts");
        let ran = layer.compute(wgsl(scaled, vec![(Slot::Order(0), Resource::Buffer(data))], Dispatch::Indirect { buffer: counts, offset: 0 })).expect("runs from counts on the GPU");
        assert_eq!(ran.workgroups, [0, 0, 0], "an indirect dispatch reports no counts of its own");
        assert_eq!(read_floats(&layer.download(data, 0, None).expect("downloads")), vec![2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0], "doubled once by the constant, then the first workgroup again at its default of 1");
        let glsl = "#version 450\n\
                    layout(local_size_x = 4) in;\n\
                    layout(set = 0, binding = 0) buffer Data { float v[]; };\n\
                    void main() { v[gl_GlobalInvocationID.x] = v[gl_GlobalInvocationID.x] + 1.0; }";
        let mut run = wgsl(glsl, vec![(Slot::Order(0), Resource::Buffer(data))], Dispatch::Workgroups([1, 1, 1]));
        run.language = Language::Glsl;
        layer.compute(run.clone()).expect("runs GLSL");
        assert_eq!(read_floats(&layer.download(data, 0, Some(16)).expect("downloads")), vec![3.0, 5.0, 7.0, 9.0]);
        run.entry = Some(String::from("other"));
        assert_eq!(layer.compute(run).expect_err("a GLSL entry point other than main is refused").id, "GuiGpuEntry");
    }

    #[test]
    fn an_array_is_copied_as_the_declared_type_and_comes_back_when_the_shader_may_write_it() {
        let layer = layer();
        let source = "@group(0) @binding(0) var<storage, read> a: array<f32>;\n\
                      @group(0) @binding(1) var<storage, read_write> b: array<i32>;\n\
                      @compute @workgroup_size(4) fn main(@builtin(global_invocation_id) i: vec3<u32>) { b[i.x] = i32(a[i.x] * 2.0) + b[i.x]; }";
        let doubles: Vec<u8> = [1.5f64, -2.25, 3.0, 4.75].iter().flat_map(|v| v.to_le_bytes()).collect();
        let bytes: Vec<u8> = [10u8, 20, 30, 40].to_vec();
        let bindings = vec![
            (Slot::Order(0), Resource::Array { element: ElementType::Float64, bytes: doubles }),
            (Slot::Name(String::from("b")), Resource::Array { element: ElementType::UInt8, bytes }),
        ];
        let written = layer.compute(wgsl(source, bindings, Dispatch::Invocations([4, 1, 1]))).expect("runs").written;
        assert_eq!(written.len(), 1, "only the binding the shader may write comes back: {written:?}");
        assert_eq!((written[0].binding.as_str(), written[0].element), ("b", ElementType::Int32));
        let ints: Vec<i32> = written[0].bytes.as_chunks::<4>().0.iter().map(|c| i32::from_le_bytes(*c)).collect();
        assert_eq!(ints, vec![13, 16, 36, 49], "the doubles were copied as f32 and the bytes as i32, each by value; i32() truncates toward zero");
    }

    #[test]
    fn a_rebuilt_layer_keeps_each_buffers_number_and_what_the_session_wrote() {
        let mut layer = layer();
        let sent = layer.upload(&[9, 8, 7, 6], Some("sent"), None).expect("uploads");
        let written = layer.zeros(4, None, None).expect("makes a zeroed buffer");
        layer.write(written, 1, &[5]).expect("writes");
        let computed = layer.zeros(16, None, None).expect("makes a zeroed buffer");
        let source = "@group(0) @binding(0) var<storage, read_write> b: array<f32>;\n\
                      @compute @workgroup_size(4) fn main(@builtin(global_invocation_id) i: vec3<u32>) { b[i.x] = 1.0; }";
        layer.compute(wgsl(source, vec![(Slot::Order(0), Resource::Buffer(computed))], Dispatch::Workgroups([1, 1, 1]))).expect("runs");
        let other = Gpu::open(&GpuOptions::default()).expect("a second device opens");
        layer.rebuild(other.adapter().clone(), other.device().clone(), other.queue().clone()).expect("rebuilds");
        assert_eq!(layer.buffers(), vec![sent, written, computed]);
        assert_eq!(layer.download(sent, 0, None).expect("downloads what was sent"), vec![9, 8, 7, 6]);
        assert_eq!(layer.download(written, 0, None).expect("downloads what was written"), vec![0, 5, 0, 0]);
        assert_eq!(layer.download(computed, 0, None).expect("downloads what was computed"), vec![0; 16]);
        let next = layer.upload(&[1], None, None).expect("uploads after the rebuild");
        assert!(next > computed, "a new buffer's number {next} follows the last one before the rebuild, {computed}");
    }

    #[test]
    fn a_destroyed_device_reports_itself_lost_as_destroyed() {
        let mut layer = layer();
        let id = layer.upload(&[1, 2, 3, 4], None, None).expect("uploads");
        assert_eq!(layer.download(id, 0, None).expect("downloads"), vec![1, 2, 3, 4], "the device worked before the destroy");
        let reason = layer.destroy_device().expect("the destroyed device reports itself lost");
        assert!(reason.starts_with("Destroyed"), "the reason names the destroy: {reason}");
        assert_eq!(layer.lost_reason(), Some(reason), "the layer stores the reason");
    }

    #[test]
    fn an_error_raised_outside_any_call_waits_for_take_errors() {
        let layer = layer();
        // Mapped at creation with a size that is not a multiple of 4, which
        // wgpu refuses whatever features the device has.
        let unaligned = wgpu::BufferDescriptor { label: Some("refused"), size: 3, usage: wgpu::BufferUsages::COPY_DST, mapped_at_creation: true };
        let refused = layer.gpu().device().create_buffer(&unaligned);
        let taken = layer.take_errors();
        assert_eq!(taken.len(), 1, "one error, the refused buffer's: {taken:?}");
        assert!(layer.take_errors().is_empty(), "a take clears what it took");
        refused.destroy();
    }
}
