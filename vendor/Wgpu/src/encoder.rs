//! Command encoders, compute passes, command buffers and query sets: the
//! low-level way to record GPU work and submit it.

use pwrs::prelude::*;

use crate::binding::BindGroup;
use crate::buffer::{self, Buffer};
use crate::device::{core_for, CoreRef, Device};
use crate::element;
use crate::error::{invalid, invalid_operation};
use crate::pipeline::ComputePipeline;
use crate::tasks::{self, Interrupt};

/// What a query set records.
#[psenum(name = "Wgpu.QueryKind")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum QueryKind {
    /// GPU timestamps, in ticks of the queue's timestamp period (feature TimestampQuery).
    #[default]
    Timestamp,
    /// Samples that passed the depth and stencil tests during a render pass.
    Occlusion,
    /// Five pipeline statistics per query (feature PipelineStatisticsQuery): vertex shader invocations, clipper invocations, clipper primitives out, fragment shader invocations, compute shader invocations.
    PipelineStatistics,
}

impl QueryKind {
    fn wgpu(self) -> wgpu::QueryType {
        match self {
            QueryKind::Timestamp => wgpu::QueryType::Timestamp,
            QueryKind::Occlusion => wgpu::QueryType::Occlusion,
            QueryKind::PipelineStatistics => wgpu::QueryType::PipelineStatistics(wgpu::PipelineStatisticsTypes::all()),
        }
    }

    /// 64-bit values each query resolves to.
    fn values(self) -> u32 {
        match self {
            QueryKind::PipelineStatistics => wgpu::PipelineStatisticsTypes::all().bits().count_ones(),
            QueryKind::Timestamp | QueryKind::Occlusion => 1,
        }
    }
}

/// A set of queries the GPU writes results into.
#[psclass(name = "Wgpu.QuerySet", mode = proxy, columns = ["Label", "Kind", "Count"])]
pub struct QuerySet {
    /// The query set's label.
    pub label: String,
    /// What its queries record.
    pub kind: QueryKind,
    /// How many queries it holds.
    pub count: u32,
    #[psfield(skip)]
    pub(crate) set: wgpu::QuerySet,
    #[psfield(skip)]
    pub(crate) core: CoreRef,
}

impl QuerySet {
    pub(crate) fn create(core: &CoreRef, kind: QueryKind, count: u32, label: Option<String>) -> PsResult<QuerySet> {
        let label = match label {
            Some(l) => l,
            None => format!("{count} x {kind:?}"),
        };
        let set = core.scoped("WgpuQuerySet", "the query set cannot be created", || {
            core.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some(&label), ty: kind.wgpu(), count })
        })?;
        Ok(QuerySet { label, kind, count, set, core: core.clone() })
    }

    /// Query `first` and how many from it, `count` running to the end of
    /// the set when omitted, checked against the set.
    fn range(&self, first: Option<u32>, count: Option<u32>) -> PsResult<(u32, u32)> {
        let first = first.unwrap_or(0);
        let count = match count {
            Some(c) => c,
            None => self.count.saturating_sub(first),
        };
        if first + count > self.count {
            return Err(invalid("WgpuRange", format!("queries {first} to {} run past the {} the set holds", first + count, self.count)));
        }
        Ok((first, count))
    }

    /// Nanoseconds per timestamp tick, for a timestamp query set.
    fn period(&self) -> PsResult<f64> {
        if self.kind != QueryKind::Timestamp {
            return Err(invalid_operation("WgpuNotTimestamps", "only a timestamp query set reads as nanoseconds"));
        }
        Ok(self.core.queue.get_timestamp_period() as f64)
    }

    /// A copy of what a thread of its own needs to resolve `count` queries
    /// from query `first`.
    fn resolver(&self, first: u32, count: u32) -> Resolver {
        Resolver { core: self.core.clone(), set: self.set.clone(), kind: self.kind, first, count }
    }
}

/// `count` queries of a set from query `first`, to resolve and read back.
struct Resolver {
    core: CoreRef,
    set: wgpu::QuerySet,
    kind: QueryKind,
    first: u32,
    count: u32,
}

impl Resolver {
    /// Resolves the queries into a buffer and reads their values back.
    fn read(&self, stop: Option<&dyn Interrupt>) -> PsResult<Vec<u64>> {
        let (first, count) = (self.first, self.count);
        let size = (count * self.kind.values()) as u64 * 8;
        let core = &self.core;
        let target = buffer::create_raw(core, size, wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, "Wgpu query results", None)?;
        let commands = core.scoped("WgpuQueryResolve", "the queries cannot be resolved", || {
            let mut encoder = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Wgpu resolve") });
            encoder.resolve_query_set(&self.set, first..first + count, &target, 0);
            encoder.finish()
        })?;
        core.submit_and_wait(vec![commands], stop)?;
        Ok(bytemuck::pod_collect_to_vec(&buffer::read_bytes(core, &target, 0, size, stop)?))
    }
}

#[psmethods]
impl QuerySet {
    /// The raw results of `count` queries from query `first` (all when
    /// omitted): one value per timestamp or occlusion query, five per
    /// pipeline-statistics query.
    pub fn read(&self, first: Option<u32>, count: Option<u32>) -> PsResult<Vec<u64>> {
        let (first, count) = self.range(first, count)?;
        self.resolver(first, count).read(None)
    }

    /// Read on a thread of its own: returns at once with a task whose
    /// result is the values.
    pub fn read_async(&self, first: Option<u32>, count: Option<u32>, task: PsTask<Vec<u64>>) -> PsResult<()> {
        let (first, count) = self.range(first, count)?;
        let resolver = self.resolver(first, count);
        tasks::spawn(task, move |stop| resolver.read(Some(stop)), Ok)
    }

    /// Timestamp results in nanoseconds.
    pub fn read_nanoseconds(&self, first: Option<u32>, count: Option<u32>) -> PsResult<Vec<f64>> {
        let period = self.period()?;
        let (first, count) = self.range(first, count)?;
        Ok(self.resolver(first, count).read(None)?.into_iter().map(|t| t as f64 * period).collect())
    }

    /// ReadNanoseconds on a thread of its own: returns at once with a task
    /// whose result is the nanoseconds.
    pub fn read_nanoseconds_async(&self, first: Option<u32>, count: Option<u32>, task: PsTask<Vec<f64>>) -> PsResult<()> {
        let period = self.period()?;
        let (first, count) = self.range(first, count)?;
        let resolver = self.resolver(first, count);
        tasks::spawn(task, move |stop| Ok(resolver.read(Some(stop))?.into_iter().map(|t| t as f64 * period).collect()), Ok)
    }

    /// Frees the query set now.
    pub fn destroy(&self) -> PsResult<()> {
        self.set.destroy();
        Ok(())
    }
}

/// A finished recording of GPU work, submitted once.
#[psclass(name = "Wgpu.CommandBuffer", mode = proxy, columns = ["Label", "IsSubmitted"])]
pub struct CommandBuffer {
    /// The label of the encoder that recorded it.
    pub label: String,
    /// Whether it was submitted; a command buffer runs once.
    pub is_submitted: bool,
    #[psfield(skip)]
    pub(crate) buffer: Option<wgpu::CommandBuffer>,
    #[psfield(skip)]
    pub(crate) core: CoreRef,
}

impl CommandBuffer {
    /// Takes the recording for submission to `core`'s queue.
    pub(crate) fn take_for(&mut self, core: &CoreRef) -> PsResult<wgpu::CommandBuffer> {
        if self.core.id != core.id {
            return Err(invalid("WgpuOtherDevice", format!("the command buffer of encoder '{}' was recorded on another device ({})", self.label, self.core.label)));
        }
        let taken = self.buffer.take().ok_or_else(|| invalid_operation("WgpuSubmitted", format!("the command buffer of encoder '{}' was already submitted; record the work again", self.label)))?;
        self.is_submitted = true;
        Ok(taken)
    }
}

/// A compute pass being recorded into an encoder.
#[psclass(name = "Wgpu.ComputePass", mode = proxy, columns = ["Label", "IsEnded"])]
pub struct ComputePass {
    /// The pass's label.
    pub label: String,
    /// Whether End was called; the encoder takes no other command until it is.
    pub is_ended: bool,
    #[psfield(skip)]
    pub(crate) pass: Option<wgpu::ComputePass<'static>>,
}

impl ComputePass {
    fn pass(&mut self) -> PsResult<&mut wgpu::ComputePass<'static>> {
        self.pass.as_mut().ok_or_else(|| invalid_operation("WgpuPassEnded", "the pass has ended"))
    }
}

#[psmethods]
impl ComputePass {
    /// Sets the pipeline the next dispatches run.
    pub fn set_pipeline(&mut self, pipeline: PsProxy<ComputePipeline>) -> PsResult<()> {
        let inner = pipeline.with(|p| p.inner.clone())?;
        self.pass()?.set_pipeline(&inner.pipeline);
        Ok(())
    }

    /// Sets bind group `index`, with dynamic offsets when its layout has them.
    pub fn set_bind_group(&mut self, index: u32, bind_group: PsProxy<BindGroup>, offsets: Option<Vec<u32>>) -> PsResult<()> {
        let group = bind_group.with(|g| g.group_object.clone())?;
        let offsets: &[u32] = match &offsets {
            Some(list) => list,
            None => &[],
        };
        self.pass()?.set_bind_group(index, &group, offsets);
        Ok(())
    }

    /// Sets immediate data from byte `offset`, from an array of numbers.
    pub fn set_immediates(&mut self, offset: u32, data: PsObject) -> PsResult<()> {
        let mut bytes = element::with_host_bytes(&data, None, |_, b| Ok(b.to_vec()))?;
        bytes.resize(bytes.len().next_multiple_of(4), 0);
        self.pass()?.set_immediates(offset, &bytes);
        Ok(())
    }

    /// Dispatches `x` by `y` by `z` workgroups (`y` and `z` 1 when omitted).
    pub fn dispatch(&mut self, x: u32, y: Option<u32>, z: Option<u32>) -> PsResult<()> {
        self.pass()?.dispatch_workgroups(x, y.unwrap_or(1), z.unwrap_or(1));
        Ok(())
    }

    /// Dispatches the workgroup counts three uint at byte `offset` of `buffer` hold.
    pub fn dispatch_indirect(&mut self, buffer: PsProxy<Buffer>, offset: Option<u64>) -> PsResult<()> {
        let raw = buffer.with(|b| b.buffer.clone())?;
        self.pass()?.dispatch_workgroups_indirect(&raw, offset.unwrap_or(0));
        Ok(())
    }

    /// Writes a timestamp into query `index` (feature TimestampQueryInsidePasses).
    pub fn write_timestamp(&mut self, query_set: PsProxy<QuerySet>, index: u32) -> PsResult<()> {
        let set = query_set.with(|q| q.set.clone())?;
        self.pass()?.write_timestamp(&set, index);
        Ok(())
    }

    /// Starts gathering pipeline statistics into query `index`.
    pub fn begin_pipeline_statistics_query(&mut self, query_set: PsProxy<QuerySet>, index: u32) -> PsResult<()> {
        let set = query_set.with(|q| q.set.clone())?;
        self.pass()?.begin_pipeline_statistics_query(&set, index);
        Ok(())
    }

    /// Stops gathering pipeline statistics.
    pub fn end_pipeline_statistics_query(&mut self) -> PsResult<()> {
        self.pass()?.end_pipeline_statistics_query();
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

/// Records GPU work: passes, copies and queries, finished into a command
/// buffer and submitted to the device.
#[psclass(name = "Wgpu.CommandEncoder", mode = proxy, columns = ["Label", "IsFinished"])]
pub struct CommandEncoder {
    /// The encoder's label.
    pub label: String,
    /// Whether Finish was called; a finished encoder records nothing more.
    pub is_finished: bool,
    #[psfield(skip)]
    pub(crate) encoder: Option<wgpu::CommandEncoder>,
    #[psfield(skip)]
    pub(crate) core: CoreRef,
}

impl CommandEncoder {
    pub(crate) fn open(core: &CoreRef, label: Option<String>) -> PsResult<CommandEncoder> {
        let label = match label {
            Some(l) => l,
            None => "Wgpu encoder".to_string(),
        };
        let encoder = core.scoped("WgpuEncoder", "the encoder cannot be created", || {
            core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(&label) })
        })?;
        Ok(CommandEncoder { label, is_finished: false, encoder: Some(encoder), core: core.clone() })
    }

    pub(crate) fn encoder(&mut self) -> PsResult<&mut wgpu::CommandEncoder> {
        self.encoder.as_mut().ok_or_else(|| invalid_operation("WgpuEncoderFinished", "the encoder was finished; open another"))
    }

    pub(crate) fn buffer_of(&self, buffer: &PsProxy<Buffer>) -> PsResult<wgpu::Buffer> {
        let (raw, owner) = buffer.with(|b| (b.buffer.clone(), b.core.clone()))?;
        if owner.id != self.core.id {
            return Err(invalid("WgpuOtherDevice", "the buffer lives on another device than the encoder"));
        }
        Ok(raw)
    }
}

#[psmethods]
impl CommandEncoder {
    /// Begins a compute pass. With `timestamps`, the GPU writes the time
    /// the pass begins into query `beginningIndex` and the time it ends
    /// into `endIndex` (0 and 1 when omitted).
    pub fn begin_compute_pass(&mut self, label: Option<String>, timestamps: Option<PsProxy<QuerySet>>, beginning_index: Option<u32>, end_index: Option<u32>) -> PsResult<ComputePass> {
        let set = match &timestamps {
            Some(q) => Some(q.with(|q| q.set.clone())?),
            None => None,
        };
        let label = match label {
            Some(l) => l,
            None => format!("{} pass", self.label),
        };
        let timestamp_writes = set.as_ref().map(|set| wgpu::ComputePassTimestampWrites {
            query_set: set,
            beginning_of_pass_write_index: Some(beginning_index.unwrap_or(0)),
            end_of_pass_write_index: Some(end_index.unwrap_or(1)),
        });
        let pass = self.encoder()?.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(&label), timestamp_writes }).forget_lifetime();
        Ok(ComputePass { label, is_ended: false, pass: Some(pass) })
    }

    /// Begins a render pass drawing into `color`, a Wgpu.Texture or
    /// Wgpu.TextureView cleared to `clearColor` (red, green, blue and
    /// alpha; opaque black when omitted). `depth` is a depth texture to
    /// test against, its depth cleared to 1 and its stencil to 0,
    /// `resolveTarget` takes a multisampled `color` resolved, and
    /// `occlusionQuerySet` receives occlusion query counts. A hashtable in
    /// place of all of them names them by key: Color, ClearColor, Depth,
    /// ClearDepth, ClearStencil, ResolveTarget, OcclusionQuerySet and
    /// Label, where a clear key given 'Load' keeps what its texture holds.
    pub fn begin_render_pass(
        &mut self,
        color: PsObject,
        clear_color: Option<Vec<f64>>,
        depth: Option<PsObject>,
        resolve_target: Option<PsObject>,
        occlusion_query_set: Option<PsProxy<QuerySet>>,
        label: Option<String>,
    ) -> PsResult<crate::render::RenderPass> {
        if crate::render::is_pass_options(&color)? {
            if clear_color.is_some() || depth.is_some() || resolve_target.is_some() || occlusion_query_set.is_some() || label.is_some() {
                return Err(invalid("WgpuRenderPassOption", "a hashtable names every part of the pass by key, so BeginRenderPass takes it alone"));
            }
            return crate::render::begin_render_pass_with(self, &color);
        }
        crate::render::begin_render_pass(self, &color, clear_color, depth.as_ref(), resolve_target.as_ref(), occlusion_query_set.as_ref(), label)
    }

    /// Copies mip level `mipLevel` (0 when omitted) of one texture into the
    /// same mip level of another of the same size and format.
    pub fn copy_texture_to_texture(&mut self, source: PsProxy<crate::texture::Texture>, destination: PsProxy<crate::texture::Texture>, mip_level: Option<u32>) -> PsResult<()> {
        let mip = mip_level.unwrap_or(0);
        let from = source.with(|t| t.texture.clone())?;
        let to = destination.with(|t| t.texture.clone())?;
        let extent = from.size().mip_level_size(mip, from.dimension());
        let mut src = from.as_image_copy();
        src.mip_level = mip;
        let mut dst = to.as_image_copy();
        dst.mip_level = mip;
        self.encoder()?.copy_texture_to_texture(src, dst, extent);
        Ok(())
    }

    /// Copies `size` bytes (to the end of the source when omitted) between
    /// buffers; offsets and size are whole 4-byte words.
    pub fn copy_buffer_to_buffer(&mut self, source: PsProxy<Buffer>, source_offset: u64, destination: PsProxy<Buffer>, destination_offset: u64, size: Option<u64>) -> PsResult<()> {
        let from = self.buffer_of(&source)?;
        let to = self.buffer_of(&destination)?;
        let size = match size {
            Some(s) => s,
            None => from.size().saturating_sub(source_offset),
        };
        self.encoder()?.copy_buffer_to_buffer(&from, source_offset, &to, destination_offset, size);
        Ok(())
    }

    /// Zeroes `size` bytes from byte `offset` of a buffer (all of it when omitted).
    pub fn clear_buffer(&mut self, buffer: PsProxy<Buffer>, offset: Option<u64>, size: Option<u64>) -> PsResult<()> {
        let raw = self.buffer_of(&buffer)?;
        self.encoder()?.clear_buffer(&raw, offset.unwrap_or(0), size);
        Ok(())
    }

    /// Resolves `count` queries from `first` into `destination` at byte `destinationOffset`.
    pub fn resolve_query_set(&mut self, query_set: PsProxy<QuerySet>, first: u32, count: u32, destination: PsProxy<Buffer>, destination_offset: Option<u64>) -> PsResult<()> {
        let set = query_set.with(|q| q.set.clone())?;
        let to = self.buffer_of(&destination)?;
        self.encoder()?.resolve_query_set(&set, first..first + count, &to, destination_offset.unwrap_or(0));
        Ok(())
    }

    /// Writes a timestamp into query `index` between passes (feature TimestampQueryInsideEncoders).
    pub fn write_timestamp(&mut self, query_set: PsProxy<QuerySet>, index: u32) -> PsResult<()> {
        let set = query_set.with(|q| q.set.clone())?;
        self.encoder()?.write_timestamp(&set, index);
        Ok(())
    }

    /// Opens a named group of commands for graphics debuggers.
    pub fn push_debug_group(&mut self, label: String) -> PsResult<()> {
        self.encoder()?.push_debug_group(&label);
        Ok(())
    }

    /// Closes the innermost debug group.
    pub fn pop_debug_group(&mut self) -> PsResult<()> {
        self.encoder()?.pop_debug_group();
        Ok(())
    }

    /// Marks a point in the recording for graphics debuggers.
    pub fn insert_debug_marker(&mut self, label: String) -> PsResult<()> {
        self.encoder()?.insert_debug_marker(&label);
        Ok(())
    }

    /// Finishes the recording into a command buffer to submit.
    pub fn finish(&mut self) -> PsResult<CommandBuffer> {
        let encoder = self.encoder.take().ok_or_else(|| invalid_operation("WgpuEncoderFinished", "the encoder was finished already"))?;
        self.is_finished = true;
        let core = self.core.clone();
        let buffer = core.scoped("WgpuFinish", "the recorded commands are invalid", || encoder.finish())?;
        Ok(CommandBuffer { label: self.label.clone(), is_submitted: false, buffer: Some(buffer), core })
    }
}

/// Opens a command encoder to record passes, copies and queries into.
///
/// # Examples
/// $encoder = New-WgpuCommandEncoder
/// $pass = $encoder.BeginComputePass(); $pass.SetPipeline($pipeline); $pass.SetBindGroup(0, $group); $pass.Dispatch(64); $pass.End()
/// $encoder.Finish() | Submit-WgpuCommand
#[cmdlet(verb = "New", noun = "WgpuCommandEncoder", alias = ["New-GpuCommandEncoder"], output = ["Wgpu.CommandEncoder"])]
#[derive(Default)]
pub struct NewWgpuCommandEncoder {
    /// The encoder's label.
    #[param(position = 0)]
    pub label: Option<String>,
    /// The device to record for; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
}

impl Cmdlet for NewWgpuCommandEncoder {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let core = core_for(&self.device, ps)?;
        ps.write(CommandEncoder::open(&core, self.label.clone())?)
    }
}

/// Submits finished command buffers to a device's queue and waits for
/// the GPU to run them.
///
/// # Examples
/// $encoder.Finish() | Submit-WgpuCommand
/// Submit-WgpuCommand $first, $second -Device $gpu
#[cmdlet(verb = "Submit", noun = "WgpuCommand", alias = ["Submit-GpuCommand"])]
#[derive(Default)]
pub struct SubmitWgpuCommand {
    /// The command buffers, in the order they run.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub command_buffer: Option<Vec<PsProxy<CommandBuffer>>>,
    /// The device whose queue runs them; the device they were recorded on when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
    core: Option<CoreRef>,
    taken: Vec<wgpu::CommandBuffer>,
}

impl Cmdlet for SubmitWgpuCommand {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(list) = &self.command_buffer else {
            return Ok(());
        };
        for c in list {
            let core = match &self.core {
                Some(core) => core.clone(),
                None => {
                    let core = match &self.device {
                        Some(d) => d.with(|d| d.core.clone())?,
                        None => c.with(|c| c.core.clone())?,
                    };
                    self.core = Some(core.clone());
                    core
                }
            };
            self.taken.push(c.with_mut(|c| c.take_for(&core))??);
        }
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        match &self.core {
            Some(core) => core.submit_and_wait(std::mem::take(&mut self.taken), Some(ps)),
            None => Ok(()),
        }
    }
}

/// Creates a set of queries the GPU writes results into: timestamps,
/// occlusion counts or pipeline statistics.
///
/// # Examples
/// $times = New-WgpuQuerySet -Kind Timestamp -Count 2
/// $times.ReadNanoseconds()
#[cmdlet(verb = "New", noun = "WgpuQuerySet", alias = ["New-GpuQuerySet"], output = ["Wgpu.QuerySet"])]
#[derive(Default)]
pub struct NewWgpuQuerySet {
    /// What the queries record.
    #[param(mandatory, position = 0)]
    pub kind: QueryKind,
    /// How many queries the set holds.
    #[param(mandatory, position = 1)]
    pub count: u32,
    /// The query set's label.
    #[param]
    pub label: Option<String>,
    /// The device to create it on; the default device when omitted.
    #[param]
    pub device: Option<PsProxy<Device>>,
}

impl Cmdlet for NewWgpuQuerySet {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let core = core_for(&self.device, ps)?;
        ps.write(QuerySet::create(&core, self.kind, self.count, self.label.clone())?)
    }
}
