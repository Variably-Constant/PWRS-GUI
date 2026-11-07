//! The renderer's GPU thread: it owns the GPU layer, runs the session's
//! GPU calls on it one after another, and gives each reply back for the UI
//! thread to send, so a long compute or a large read never delays a frame.

use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvError, Sender};
use std::thread::JoinHandle;

use crate::proto;
use crossbeam_queue::SegQueue;
use proto::gpu::{
    AddressMode as WireAddress, Blend, BlendFactor as WireFactor, BlendOperation as WireOperation, BufferUsage, CompareFunction as WireCompare, ComputeReport, Cull, DepthFormat, DepthTest as WireDepthTest,
    DrawPipeline as WirePipeline, GpuDispatch, GpuFailure, GpuOp, GpuResource, GpuSlot, GpuValue, IndexFormat as WireIndex, PassEvery, PipelineSpec, SamplerFilter, ShaderLanguage,
    StencilFace as WireFace, StencilOperation as WireStencilOperation, StencilTest as WireStencilTest, SurfaceDraw as WireDraw, SurfacePass as WirePass, TextureFormat as WireFormat, Topology,
    WrittenArray,
};
use proto::renderer::WakeLoop;
use proto::{FromHelper, NodeId, RequestId, RingName, WindowId};
use pwrs_gui_gpu::{
    AddressMode, BlendFactor, BlendMode, BlendOperation, CompareFunction, CullMode, CustomBlend, DepthTest, Dispatch, DrawPipeline, ElementType, Every, Filter, Gpu, IndexFormat, Language, Layer, Pipeline,
    PrimitiveTopology, Resource, Run, SamplerSpec, Slot, StencilFace, StencilOperation, StencilTest, SurfaceDraw, SurfacePass, SurfacePlan, SurfaceSet, TextureFormat, wgpu,
};

/// Work for the GPU thread: a call the session waits on, replied to on its
/// reply ring; a buffer or a render pipeline to free that it does not wait
/// on; the device to destroy for a reset, which the UI thread replies to
/// once the renderer is on a new device, and the GPU thread only when the
/// destroy fails; a GpuSurface's draws to plan, which the UI thread
/// installs and replies to, `reply` being none when the UI thread plans a
/// surface again on a new device; or a surface's texture to read back.
enum Job {
    Call { request: RequestId, reply: RingName, op: GpuOp },
    Free { buffer: u64 },
    FreePipeline { pipeline: u64 },
    FreeTexture { texture: u64 },
    FreeSampler { sampler: u64 },
    Destroy { request: RequestId, reply: RingName },
    Plan { request: RequestId, reply: Option<RingName>, window: WindowId, node: NodeId, given: Given },
    Read { request: RequestId, reply: RingName, texture: wgpu::Texture },
}

/// What the GPU thread made for the UI thread to send: a reply for a
/// reply ring, a line for the log, an error the device raised outside any
/// call, which the UI thread reports as it reports those it takes, or a
/// GpuSurface's plan, or why its draws were refused, for the UI thread to
/// install and reply to.
pub(crate) enum Answer {
    Reply { reply: RingName, msg: FromHelper },
    Log(FromHelper),
    Outside(String),
    Planned { request: RequestId, reply: Option<RingName>, window: WindowId, node: NodeId, given: Given, plan: Result<SurfacePlan, GpuFailure> },
}

/// What a GpuSurface was given, which the renderer stores to plan it again
/// on a new device: its clear color, the depth and stencil it clears to,
/// its draws, its compute passes, and its frame uniform's own fields.
#[derive(Clone, Debug)]
pub(crate) struct Given {
    pub clear: [f64; 4],
    pub clear_depth: Option<f64>,
    pub clear_stencil: Option<u32>,
    pub draws: Vec<WireDraw>,
    pub passes: Vec<WirePass>,
    pub uniform: Vec<u8>,
}

/// The UI thread's end of the GPU thread.
pub(crate) struct GpuThread {
    jobs: Sender<Job>,
    answers: Arc<SegQueue<Answer>>,
    /// The layer's device, whose errors raised outside any call the UI
    /// thread takes once a frame, as the GPU thread does before each job,
    /// and whose loss the UI thread checks for.
    gpu: Gpu,
    /// The thread, which gives its layer back as it ends.
    thread: JoinHandle<Layer>,
}

impl GpuThread {
    /// Starts the thread on `layer`; `wake` wakes the UI thread after each
    /// [`Answer`].
    pub(crate) fn start(layer: Layer, wake: WakeLoop) -> Result<GpuThread, String> {
        let gpu = layer.gpu().clone();
        let (jobs, taken) = mpsc::channel();
        let answers = Arc::new(SegQueue::new());
        let out = Arc::clone(&answers);
        let thread = std::thread::Builder::new()
            .name(String::from("pwrs-gui-gpu"))
            .spawn(move || run(layer, taken, out, wake))
            .map_err(|e| format!("the GPU thread cannot start: {e}"))?;
        Ok(GpuThread { jobs, answers, gpu, thread })
    }

    /// Ends the thread once the work given to it is done, and returns its
    /// layer, or why it has none, and what it left for the UI thread to send.
    pub(crate) fn end(self) -> (Result<Layer, String>, Vec<Answer>) {
        let GpuThread { jobs, answers, gpu, thread } = self;
        drop(jobs);
        drop(gpu);
        let layer = thread.join().map_err(|panic| {
            let why = match (panic.downcast_ref::<&str>(), panic.downcast_ref::<String>()) {
                (Some(text), _) => text.to_string(),
                (None, Some(text)) => text.clone(),
                (None, None) => String::from("a panic with no message"),
            };
            format!("the GPU thread panicked: {why}")
        });
        (layer, std::iter::from_fn(|| answers.pop()).collect())
    }

    /// Destroys the device for a reset; only a destroy that fails is
    /// replied to from here.
    pub(crate) fn destroy(&self, request: RequestId, reply: RingName) {
        self.hand_over(Job::Destroy { request, reply });
    }

    /// Why the device was lost, once it is.
    pub(crate) fn lost_reason(&self) -> Option<String> {
        self.gpu.lost_reason()
    }

    /// Passes `op` to the GPU thread; when the thread has ended, the reply
    /// saying so is ready at once.
    pub(crate) fn send(&self, request: RequestId, reply: RingName, op: GpuOp) {
        self.hand_over(Job::Call { request, reply, op });
    }

    /// Frees `buffer` with no reply; a failure goes to the log.
    pub(crate) fn free(&self, buffer: u64) {
        self.hand_over(Job::Free { buffer });
    }

    /// Frees render pipeline `pipeline` with no reply; a failure goes to
    /// the log.
    pub(crate) fn free_pipeline(&self, pipeline: u64) {
        self.hand_over(Job::FreePipeline { pipeline });
    }

    /// Frees texture `texture` with no reply; a failure goes to the log.
    pub(crate) fn free_texture(&self, texture: u64) {
        self.hand_over(Job::FreeTexture { texture });
    }

    /// Frees sampler `sampler` with no reply; a failure goes to the log.
    pub(crate) fn free_sampler(&self, sampler: u64) {
        self.hand_over(Job::FreeSampler { sampler });
    }

    /// Plans what the GpuSurface `node` of window `window` was `given`; the
    /// plan comes back as [`Answer::Planned`] for the UI thread to install,
    /// and with `reply`, the call to reply to once it has.
    pub(crate) fn plan(&self, request: RequestId, reply: Option<RingName>, window: WindowId, node: NodeId, given: Given) {
        self.hand_over(Job::Plan { request, reply, window, node, given });
    }

    /// Reads a GpuSurface's texture back, replying on `reply` with its
    /// texels.
    pub(crate) fn read(&self, request: RequestId, reply: RingName, texture: wgpu::Texture) {
        self.hand_over(Job::Read { request, reply, texture });
    }

    fn hand_over(&self, job: Job) {
        if let Err(mpsc::SendError(job)) = self.jobs.send(job) {
            let failure = GpuFailure { id: String::from("GuiGpuEnded"), text: String::from("the renderer's GPU thread has ended") };
            self.answers.push(match job {
                Job::Call { request, reply, .. } | Job::Destroy { request, reply } | Job::Read { request, reply, .. } | Job::Plan { request, reply: Some(reply), .. } => {
                    Answer::Reply { reply, msg: FromHelper::GpuResult { request, outcome: Err(failure) } }
                }
                Job::Plan { window, node, reply: None, .. } => Answer::Log(FromHelper::Error {
                    window: Some(window),
                    context: String::from("GPU"),
                    text: format!("the draws of GpuSurface {node} could not be planned again: {} ({})", failure.text, failure.id),
                }),
                Job::Free { buffer } => free_failed(buffer, &failure),
                Job::FreePipeline { pipeline } => pipeline_free_failed(pipeline, &failure),
                Job::FreeTexture { texture } => freeing_failed(&format!("texture {texture}"), &failure),
                Job::FreeSampler { sampler } => freeing_failed(&format!("sampler {sampler}"), &failure),
            });
        }
    }

    /// What is ready to send, oldest first.
    pub(crate) fn answers(&self) -> Vec<Answer> {
        std::iter::from_fn(|| self.answers.pop()).collect()
    }

    /// The errors the device raised outside any call since the last take,
    /// the renderer's own drawing among them, oldest first.
    pub(crate) fn take_errors(&self) -> Vec<String> {
        self.gpu.take_errors()
    }
}

/// Runs each call as it arrives; ends when the UI thread drops its end, as
/// the renderer does as it ends or moves the layer to a new device, and
/// returns the layer.
fn run(mut layer: Layer, jobs: Receiver<Job>, answers: Arc<SegQueue<Answer>>, wake: WakeLoop) -> Layer {
    loop {
        let job = match jobs.recv() {
            Ok(job) => job,
            Err(RecvError) => return layer,
        };
        if hand_over_errors(&layer, &answers) {
            wake();
        }
        match job {
            Job::Call { request, reply, op } => {
                let outcome = call(&mut layer, op).map_err(|f| GpuFailure { id: f.id, text: f.text });
                answers.push(Answer::Reply { reply, msg: FromHelper::GpuResult { request, outcome } });
                wake();
            }
            Job::Free { buffer } => {
                if let Err(f) = layer.free(buffer) {
                    answers.push(free_failed(buffer, &GpuFailure { id: f.id, text: f.text }));
                    wake();
                }
            }
            Job::FreePipeline { pipeline } => {
                if let Err(f) = layer.free_pipeline(pipeline) {
                    answers.push(pipeline_free_failed(pipeline, &GpuFailure { id: f.id, text: f.text }));
                    wake();
                }
            }
            Job::FreeTexture { texture } => {
                if let Err(f) = layer.free_texture(texture) {
                    answers.push(freeing_failed(&format!("texture {texture}"), &GpuFailure { id: f.id, text: f.text }));
                    wake();
                }
            }
            Job::FreeSampler { sampler } => {
                if let Err(f) = layer.free_sampler(sampler) {
                    answers.push(freeing_failed(&format!("sampler {sampler}"), &GpuFailure { id: f.id, text: f.text }));
                    wake();
                }
            }
            Job::Plan { request, reply, window, node, given } => {
                let set = SurfaceSet {
                    clear: given.clear,
                    clear_depth: given.clear_depth,
                    clear_stencil: given.clear_stencil,
                    draws: given.draws.iter().map(draw_of).collect(),
                    passes: given.passes.iter().map(pass_of).collect(),
                    uniform: given.uniform.clone(),
                };
                let plan = layer.plan(set).map_err(|f| GpuFailure { id: f.id, text: f.text });
                answers.push(Answer::Planned { request, reply, window, node, given, plan });
                wake();
            }
            Job::Read { request, reply, texture } => {
                let (width, height) = (texture.width(), texture.height());
                let outcome = layer.gpu().read_texture(&texture).map(|rgba| GpuValue::Texels { width, height, rgba }).map_err(|e| GpuFailure { id: e.error_id, text: e.message });
                answers.push(Answer::Reply { reply, msg: FromHelper::GpuResult { request, outcome } });
                wake();
            }
            Job::Destroy { request, reply } => {
                match layer.destroy_device() {
                    Ok(reason) => proto::trace::line(move || format!("egui: GPU device destroyed for a reset, lost: {reason}")),
                    Err(f) => answers.push(Answer::Reply { reply, msg: FromHelper::GpuResult { request, outcome: Err(GpuFailure { id: f.id, text: f.text }) } }),
                }
                wake();
            }
        }
    }
}

/// Takes the errors the device raised outside any call, the renderer's own
/// drawing among them, so the job that runs next fails only for its own:
/// each goes to the UI thread to report, or to the log alone once the
/// device is lost, as the UI thread sends those it takes. Returns whether
/// there were any.
fn hand_over_errors(layer: &Layer, answers: &SegQueue<Answer>) -> bool {
    let taken = layer.gpu().take_errors();
    let lost = layer.gpu().lost_reason().is_some();
    let any = !taken.is_empty();
    for text in taken {
        answers.push(if lost { Answer::Log(FromHelper::Error { window: None, context: String::from("GPU"), text }) } else { Answer::Outside(text) });
    }
    any
}

/// The log's line for a free of `buffer` that failed.
fn free_failed(buffer: u64, failure: &GpuFailure) -> Answer {
    Answer::Log(FromHelper::Error { window: None, context: String::from("GPU"), text: format!("GPU buffer {buffer} could not be freed: {} ({})", failure.text, failure.id) })
}

/// The log's line for a free of render pipeline `pipeline` that failed.
fn pipeline_free_failed(pipeline: u64, failure: &GpuFailure) -> Answer {
    Answer::Log(FromHelper::Error { window: None, context: String::from("GPU"), text: format!("render pipeline {pipeline} could not be freed: {} ({})", failure.text, failure.id) })
}

/// The log's line for a free of `what`, a texture or a sampler, that
/// failed.
fn freeing_failed(what: &str, failure: &GpuFailure) -> Answer {
    Answer::Log(FromHelper::Error { window: None, context: String::from("GPU"), text: format!("{what} could not be freed: {} ({})", failure.text, failure.id) })
}

/// The texture of `format` an image file's `bytes` make, decoded to eight
/// bits a channel of red, green, blue and alpha; `format` is
/// `Rgba8UnormSrgb`, as image files store their colors, or `Rgba8Unorm`.
fn import_image(layer: &mut Layer, bytes: &[u8], format: TextureFormat, label: Option<&str>) -> Result<u64, pwrs_gui_gpu::Failure> {
    if !matches!(format, TextureFormat::Rgba8UnormSrgb | TextureFormat::Rgba8Unorm) {
        return Err(pwrs_gui_gpu::Failure { id: String::from("GuiGpuImageFormat"), text: format!("an image is imported as Rgba8UnormSrgb or Rgba8Unorm, not {format:?}") });
    }
    let decoded = image::load_from_memory(bytes).map_err(|e| pwrs_gui_gpu::Failure { id: String::from("GuiGpuImage"), text: format!("the image could not be read: {e}") })?.to_rgba8();
    let (width, height) = decoded.dimensions();
    layer.new_texture(width, height, format, Some(decoded.as_raw()), label)
}

fn call(layer: &mut Layer, op: GpuOp) -> Result<GpuValue, pwrs_gui_gpu::Failure> {
    match op {
        GpuOp::NewBuffer { bytes, label, usage } => {
            let buffer = layer.upload(&bytes, label.as_deref(), usage.as_deref().map(usage_of))?;
            Ok(GpuValue::Buffer { buffer, length: bytes.len() as u64 })
        }
        GpuOp::NewZeroed { length, label, usage } => {
            let buffer = layer.zeros(length, label.as_deref(), usage.as_deref().map(usage_of))?;
            Ok(GpuValue::Buffer { buffer, length })
        }
        GpuOp::Read { buffer, offset, length } => layer.download(buffer, offset, length).map(GpuValue::Bytes),
        GpuOp::Write { buffer, offset, bytes } => layer.write(buffer, offset, &bytes).map(|()| GpuValue::Done),
        GpuOp::Remove { buffer } => layer.free(buffer).map(|()| GpuValue::Done),
        GpuOp::Compute { source, language, entry, constants, bindings, dispatch, immediates, timed } => {
            let language = match language {
                ShaderLanguage::Wgsl => Language::Wgsl,
                ShaderLanguage::Glsl => Language::Glsl,
            };
            let bindings = bindings.into_iter().map(|(slot, resource)| (slot_of(slot), resource_of(resource))).collect();
            let ran = layer.compute(Run { source, language, entry, constants, bindings, dispatch: dispatch_of(dispatch), immediates, timed })?;
            let written = ran.written.into_iter().map(|w| WrittenArray { binding: w.binding, element: element_to_proto(w.element), bytes: w.bytes }).collect();
            Ok(GpuValue::Computed(ComputeReport { written, workgroups: ran.workgroups, gpu_time_ns: ran.gpu_time_ns, cpu_time_ms: ran.cpu_time_ms }))
        }
        GpuOp::NewRenderPipeline { spec } => {
            let (pipeline, info) = layer.new_pipeline(pipeline_of(spec))?;
            Ok(GpuValue::RenderPipeline { pipeline, vertex_inputs: info.vertex_inputs, bindings: info.bindings })
        }
        GpuOp::NewTexture { width, height, format, texels, label } => {
            let texture = layer.new_texture(width, height, texture_format_of(format), texels.as_deref(), label.as_deref())?;
            Ok(GpuValue::Texture { texture, width, height, format })
        }
        GpuOp::ImportImage { bytes, format, label } => {
            let texture = import_image(layer, &bytes, texture_format_of(format), label.as_deref())?;
            let info = layer.texture_info(texture)?;
            Ok(GpuValue::Texture { texture, width: info.width, height: info.height, format })
        }
        GpuOp::ReadTexture { texture } => layer.read_texture(texture).map(GpuValue::Bytes),
        GpuOp::WriteTexture { texture, texels } => layer.write_texture(texture, &texels).map(|()| GpuValue::Done),
        GpuOp::RemoveTexture { texture } => layer.free_texture(texture).map(|()| GpuValue::Done),
        GpuOp::NewSampler { filter, address, label } => {
            let spec = SamplerSpec {
                filter: match filter {
                    SamplerFilter::Nearest => Filter::Nearest,
                    SamplerFilter::Linear => Filter::Linear,
                },
                address: match address {
                    WireAddress::ClampToEdge => AddressMode::ClampToEdge,
                    WireAddress::Repeat => AddressMode::Repeat,
                    WireAddress::MirrorRepeat => AddressMode::MirrorRepeat,
                },
            };
            Ok(GpuValue::Sampler { sampler: layer.new_sampler(spec, label.as_deref())? })
        }
        GpuOp::SetSurface { .. }
        | GpuOp::SaveSurface { .. }
        | GpuOp::SetCamera { .. }
        | GpuOp::GetCamera { .. }
        | GpuOp::RemoveCamera { .. }
        | GpuOp::SetCanvasView { .. }
        | GpuOp::GetCanvasView { .. }
        | GpuOp::RemoveCanvasView { .. }
        | GpuOp::ResetDevice
        | GpuOp::WatchLoss => {
            let text = String::from("a GpuSurface's draws, texels and camera, a Canvas's view, a reset and a call waiting for a lost device are replied to by the renderer's UI thread, and this one reached the GPU thread");
            Err(pwrs_gui_gpu::Failure { id: String::from("GuiGpuHeld"), text })
        }
        GpuOp::SetPageSurface { .. } | GpuOp::MirrorBuffer { .. } | GpuOp::MirrorTexture { .. } | GpuOp::MirrorSampler { .. } | GpuOp::ForgetMirror { .. } => {
            let text = String::from("a page's surface and the copies of the table's objects are the HTML renderer's, and this call reached the egui renderer");
            Err(pwrs_gui_gpu::Failure { id: String::from("GuiGpuHeld"), text })
        }
    }
}

/// A render pipeline's spec as the GPU layer builds it.
fn pipeline_of(spec: PipelineSpec) -> Pipeline {
    let custom = spec.custom_blend;
    Pipeline {
        source: spec.source,
        vertex_entry: spec.vertex_entry,
        fragment_entry: spec.fragment_entry,
        topology: match spec.topology {
            Topology::PointList => PrimitiveTopology::PointList,
            Topology::LineList => PrimitiveTopology::LineList,
            Topology::LineStrip => PrimitiveTopology::LineStrip,
            Topology::TriangleList => PrimitiveTopology::TriangleList,
            Topology::TriangleStrip => PrimitiveTopology::TriangleStrip,
        },
        cull: match spec.cull {
            Cull::None => CullMode::None,
            Cull::Front => CullMode::Front,
            Cull::Back => CullMode::Back,
        },
        blend: match spec.blend {
            Blend::Replace => BlendMode::Replace,
            Blend::Alpha => BlendMode::Alpha,
            Blend::Additive => BlendMode::Additive,
            Blend::Premultiplied => BlendMode::Premultiplied,
            Blend::Constant => BlendMode::Constant,
            Blend::Custom => BlendMode::Custom,
        },
        custom_blend: CustomBlend {
            source: custom.source.map(factor_of),
            destination: custom.destination.map(factor_of),
            operation: custom.operation.map(operation_of),
            alpha_source: custom.alpha_source.map(factor_of),
            alpha_destination: custom.alpha_destination.map(factor_of),
            alpha_operation: custom.alpha_operation.map(operation_of),
        },
        depth: spec.depth,
        depth_format: spec.depth_format.map(depth_format_of),
        depth_test: depth_test_of(spec.depth_test),
        stencil: stencil_of(spec.stencil),
        sample_count: spec.sample_count,
        constants: spec.constants,
    }
}

/// A depth attachment's format as wgpu names it.
fn depth_format_of(format: DepthFormat) -> wgpu::TextureFormat {
    match format {
        DepthFormat::Stencil8 => wgpu::TextureFormat::Stencil8,
        DepthFormat::Depth16Unorm => wgpu::TextureFormat::Depth16Unorm,
        DepthFormat::Depth24Plus => wgpu::TextureFormat::Depth24Plus,
        DepthFormat::Depth24PlusStencil8 => wgpu::TextureFormat::Depth24PlusStencil8,
        DepthFormat::Depth32Float => wgpu::TextureFormat::Depth32Float,
        DepthFormat::Depth32FloatStencil8 => wgpu::TextureFormat::Depth32FloatStencil8,
    }
}

fn compare_of(compare: WireCompare) -> CompareFunction {
    match compare {
        WireCompare::Never => CompareFunction::Never,
        WireCompare::Less => CompareFunction::Less,
        WireCompare::Equal => CompareFunction::Equal,
        WireCompare::LessEqual => CompareFunction::LessEqual,
        WireCompare::Greater => CompareFunction::Greater,
        WireCompare::NotEqual => CompareFunction::NotEqual,
        WireCompare::GreaterEqual => CompareFunction::GreaterEqual,
        WireCompare::Always => CompareFunction::Always,
    }
}

fn stencil_operation_of(operation: WireStencilOperation) -> StencilOperation {
    match operation {
        WireStencilOperation::Keep => StencilOperation::Keep,
        WireStencilOperation::Zero => StencilOperation::Zero,
        WireStencilOperation::Replace => StencilOperation::Replace,
        WireStencilOperation::Invert => StencilOperation::Invert,
        WireStencilOperation::IncrementClamp => StencilOperation::IncrementClamp,
        WireStencilOperation::DecrementClamp => StencilOperation::DecrementClamp,
        WireStencilOperation::IncrementWrap => StencilOperation::IncrementWrap,
        WireStencilOperation::DecrementWrap => StencilOperation::DecrementWrap,
    }
}

/// A depth test as the GPU layer builds it, each part left absent as the
/// session left it, so the Wgpu module fills it in.
fn depth_test_of(test: WireDepthTest) -> DepthTest {
    DepthTest { compare: test.compare.map(compare_of), write: test.write, bias: test.bias, bias_slope_scale: test.bias_slope_scale, bias_clamp: test.bias_clamp }
}

/// A stencil test as the GPU layer builds it, each part left absent as
/// the session left it, so the Wgpu module fills it in.
fn stencil_of(test: WireStencilTest) -> StencilTest {
    let face = |f: WireFace| StencilFace {
        compare: f.compare.map(compare_of),
        fail: f.fail.map(stencil_operation_of),
        depth_fail: f.depth_fail.map(stencil_operation_of),
        pass: f.pass.map(stencil_operation_of),
    };
    StencilTest { front: face(test.front), back: face(test.back), read_mask: test.read_mask, write_mask: test.write_mask }
}

fn factor_of(factor: WireFactor) -> BlendFactor {
    match factor {
        WireFactor::Zero => BlendFactor::Zero,
        WireFactor::One => BlendFactor::One,
        WireFactor::Src => BlendFactor::Src,
        WireFactor::OneMinusSrc => BlendFactor::OneMinusSrc,
        WireFactor::SrcAlpha => BlendFactor::SrcAlpha,
        WireFactor::OneMinusSrcAlpha => BlendFactor::OneMinusSrcAlpha,
        WireFactor::Dst => BlendFactor::Dst,
        WireFactor::OneMinusDst => BlendFactor::OneMinusDst,
        WireFactor::DstAlpha => BlendFactor::DstAlpha,
        WireFactor::OneMinusDstAlpha => BlendFactor::OneMinusDstAlpha,
        WireFactor::SrcAlphaSaturated => BlendFactor::SrcAlphaSaturated,
        WireFactor::Constant => BlendFactor::Constant,
        WireFactor::OneMinusConstant => BlendFactor::OneMinusConstant,
        WireFactor::Src1 => BlendFactor::Src1,
        WireFactor::OneMinusSrc1 => BlendFactor::OneMinusSrc1,
        WireFactor::Src1Alpha => BlendFactor::Src1Alpha,
        WireFactor::OneMinusSrc1Alpha => BlendFactor::OneMinusSrc1Alpha,
    }
}

fn operation_of(operation: WireOperation) -> BlendOperation {
    match operation {
        WireOperation::Add => BlendOperation::Add,
        WireOperation::Subtract => BlendOperation::Subtract,
        WireOperation::ReverseSubtract => BlendOperation::ReverseSubtract,
        WireOperation::Min => BlendOperation::Min,
        WireOperation::Max => BlendOperation::Max,
    }
}

/// A GpuSurface's draw as the GPU layer plans it.
fn draw_of(draw: &WireDraw) -> SurfaceDraw {
    SurfaceDraw {
        pipeline: match &draw.pipeline {
            WirePipeline::Table(id) => DrawPipeline::Table(*id),
            WirePipeline::Inline(spec) => DrawPipeline::Inline(pipeline_of(spec.clone())),
        },
        vertices: draw.vertices.iter().map(|(slot, given)| (slot_of(slot.clone()), resource_of(given.clone()))).collect(),
        index: draw.index.map(|(id, format, count)| {
            let format = match format {
                WireIndex::Uint16 => IndexFormat::Uint16,
                WireIndex::Uint32 => IndexFormat::Uint32,
            };
            (id, format, count)
        }),
        vertex_count: draw.vertex_count,
        instance_count: draw.instance_count,
        bindings: draw.bindings.iter().map(|(slot, given)| (slot_of(slot.clone()), resource_of(given.clone()))).collect(),
        blend_constant: draw.blend_constant,
        stencil_reference: draw.stencil_reference,
    }
}

/// A GpuSurface's compute pass as the GPU layer plans it.
fn pass_of(pass: &WirePass) -> SurfacePass {
    SurfacePass {
        source: pass.source.clone(),
        language: match pass.language {
            ShaderLanguage::Wgsl => Language::Wgsl,
            ShaderLanguage::Glsl => Language::Glsl,
        },
        entry: pass.entry.clone(),
        constants: pass.constants.clone(),
        bindings: pass.bindings.iter().map(|(slot, given)| (slot_of(slot.clone()), resource_of(given.clone()))).collect(),
        dispatch: dispatch_of(pass.dispatch),
        every: match pass.every {
            PassEvery::Frame => Every::Frame,
            PassEvery::Once => Every::Once,
        },
    }
}

fn usage_of(usage: &[BufferUsage]) -> wgpu::BufferUsages {
    usage.iter().fold(wgpu::BufferUsages::empty(), |all, u| {
        all | match u {
            BufferUsage::MapRead => wgpu::BufferUsages::MAP_READ,
            BufferUsage::MapWrite => wgpu::BufferUsages::MAP_WRITE,
            BufferUsage::CopySrc => wgpu::BufferUsages::COPY_SRC,
            BufferUsage::CopyDst => wgpu::BufferUsages::COPY_DST,
            BufferUsage::Index => wgpu::BufferUsages::INDEX,
            BufferUsage::Vertex => wgpu::BufferUsages::VERTEX,
            BufferUsage::Uniform => wgpu::BufferUsages::UNIFORM,
            BufferUsage::Storage => wgpu::BufferUsages::STORAGE,
            BufferUsage::Indirect => wgpu::BufferUsages::INDIRECT,
            BufferUsage::QueryResolve => wgpu::BufferUsages::QUERY_RESOLVE,
            BufferUsage::BlasInput => wgpu::BufferUsages::BLAS_INPUT,
            BufferUsage::TlasInput => wgpu::BufferUsages::TLAS_INPUT,
        }
    })
}

fn slot_of(slot: GpuSlot) -> Slot {
    match slot {
        GpuSlot::Order(i) => Slot::Order(i as usize),
        GpuSlot::Name(name) => Slot::Name(name),
    }
}

fn resource_of(resource: GpuResource) -> Resource {
    match resource {
        GpuResource::Buffer(id) => Resource::Buffer(id),
        GpuResource::Array { element, bytes } => Resource::Array { element: element_of(element), bytes },
        GpuResource::Texture(id) => Resource::Texture(id),
        GpuResource::Sampler(id) => Resource::Sampler(id),
    }
}

fn texture_format_of(format: WireFormat) -> TextureFormat {
    match format {
        WireFormat::R8Unorm => TextureFormat::R8Unorm,
        WireFormat::Rg8Unorm => TextureFormat::Rg8Unorm,
        WireFormat::Rgba8Unorm => TextureFormat::Rgba8Unorm,
        WireFormat::Rgba8UnormSrgb => TextureFormat::Rgba8UnormSrgb,
        WireFormat::Bgra8Unorm => TextureFormat::Bgra8Unorm,
        WireFormat::Bgra8UnormSrgb => TextureFormat::Bgra8UnormSrgb,
        WireFormat::R16Float => TextureFormat::R16Float,
        WireFormat::Rg16Float => TextureFormat::Rg16Float,
        WireFormat::Rgba16Float => TextureFormat::Rgba16Float,
        WireFormat::R32Float => TextureFormat::R32Float,
        WireFormat::Rg32Float => TextureFormat::Rg32Float,
        WireFormat::Rgba32Float => TextureFormat::Rgba32Float,
        WireFormat::R32Uint => TextureFormat::R32Uint,
        WireFormat::R32Sint => TextureFormat::R32Sint,
    }
}

fn dispatch_of(dispatch: GpuDispatch) -> Dispatch {
    match dispatch {
        GpuDispatch::Workgroups(counts) => Dispatch::Workgroups(counts),
        GpuDispatch::Invocations(counts) => Dispatch::Invocations(counts),
        GpuDispatch::Indirect { buffer, offset } => Dispatch::Indirect { buffer, offset },
        GpuDispatch::Auto => Dispatch::Auto,
    }
}

fn element_of(element: proto::gpu::ElementType) -> ElementType {
    use proto::gpu::ElementType as P;
    match element {
        P::Float32 => ElementType::Float32,
        P::Float64 => ElementType::Float64,
        P::Float16 => ElementType::Float16,
        P::Int8 => ElementType::Int8,
        P::UInt8 => ElementType::UInt8,
        P::Int16 => ElementType::Int16,
        P::UInt16 => ElementType::UInt16,
        P::Int32 => ElementType::Int32,
        P::UInt32 => ElementType::UInt32,
        P::Int64 => ElementType::Int64,
        P::UInt64 => ElementType::UInt64,
    }
}

fn element_to_proto(element: ElementType) -> proto::gpu::ElementType {
    use proto::gpu::ElementType as P;
    match element {
        ElementType::Float32 => P::Float32,
        ElementType::Float64 => P::Float64,
        ElementType::Float16 => P::Float16,
        ElementType::Int8 => P::Int8,
        ElementType::UInt8 => P::UInt8,
        ElementType::Int16 => P::Int16,
        ElementType::UInt16 => P::UInt16,
        ElementType::Int32 => P::Int32,
        ElementType::UInt32 => P::UInt32,
        ElementType::Int64 => P::Int64,
        ElementType::UInt64 => P::UInt64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::gpu::CustomBlend as WireCustom;

    /// A spec of a full-screen red triangle drawing as
    /// New-GuiGpuRenderPipeline draws given nothing else.
    fn spec() -> PipelineSpec {
        PipelineSpec {
            source: String::from(
                "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
                    return vec4<f32>(corners[i], 0.0, 1.0);
                }
                @fragment fn fs() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.0, 0.0, 1.0); }",
            ),
            vertex_entry: None,
            fragment_entry: None,
            topology: Topology::TriangleList,
            cull: Cull::None,
            blend: Blend::Replace,
            custom_blend: WireCustom::default(),
            depth: false,
            depth_format: None,
            depth_test: WireDepthTest::default(),
            stencil: WireStencilTest::default(),
            sample_count: 1,
            constants: Vec::new(),
        }
    }

    /// The protocol refuses a spec's depth and stencil tests and its blend
    /// as the Wgpu module's library refuses them, under the same id, in the
    /// same words and in the same order, and accepts what it accepts, so an
    /// HTML window, whose page the module checks a spec for, refuses what an
    /// egui window does.
    #[test]
    fn the_protocol_refuses_a_spec_s_depth_and_stencil_tests_and_blend_as_the_wgpu_library_does() {
        let gpu = Gpu::open(&pwrs_gui_gpu::GpuOptions::default()).expect("a device opens");
        let mut layer = Layer::adopt(gpu.adapter().clone(), gpu.device().clone(), gpu.queue().clone(), "PWRS-GUI refusal test").expect("the layer takes the device");
        let greater = WireDepthTest { compare: Some(WireCompare::Greater), ..WireDepthTest::default() };
        let replace = WireStencilTest { front: WireFace { pass: Some(WireStencilOperation::Replace), ..WireFace::default() }, ..WireStencilTest::default() };
        let back_only = WireStencilTest { back: WireFace { compare: Some(WireCompare::Never), ..WireFace::default() }, ..WireStencilTest::default() };
        let source = WireCustom { source: Some(WireFactor::One), ..WireCustom::default() };
        let alpha_max = WireCustom { alpha_operation: Some(WireOperation::Max), ..WireCustom::default() };
        let specs = [
            PipelineSpec { blend: Blend::Alpha, custom_blend: source, ..spec() },
            PipelineSpec { custom_blend: alpha_max, ..spec() },
            PipelineSpec { blend: Blend::Constant, custom_blend: WireCustom { destination: Some(WireFactor::Zero), ..WireCustom::default() }, ..spec() },
            PipelineSpec { blend: Blend::Alpha, custom_blend: source, depth_test: greater, ..spec() },
            PipelineSpec { blend: Blend::Custom, custom_blend: source, ..spec() },
            PipelineSpec { depth_test: greater, ..spec() },
            PipelineSpec { depth_format: Some(DepthFormat::Stencil8), depth_test: WireDepthTest { write: Some(false), ..WireDepthTest::default() }, ..spec() },
            PipelineSpec { depth: true, stencil: replace, ..spec() },
            PipelineSpec { stencil: back_only, ..spec() },
            PipelineSpec { depth: true, depth_test: WireDepthTest { bias: Some(1), ..WireDepthTest::default() }, topology: Topology::LineList, ..spec() },
            PipelineSpec { depth: true, depth_test: WireDepthTest { bias_clamp: Some(0.5), ..WireDepthTest::default() }, topology: Topology::PointList, ..spec() },
            PipelineSpec { depth_format: Some(DepthFormat::Depth24PlusStencil8), depth_test: greater, stencil: replace, ..spec() },
            PipelineSpec { depth_format: Some(DepthFormat::Stencil8), stencil: back_only, ..spec() },
            PipelineSpec { depth: true, depth_test: WireDepthTest { bias: Some(-4), bias_slope_scale: Some(1.0), bias_clamp: Some(-0.25), ..WireDepthTest::default() }, ..spec() },
        ];
        let mut refused = 0;
        for given in specs {
            let library = layer.new_pipeline(pipeline_of(given.clone())).err().map(|f| (f.id, f.text));
            let protocol = given.build_refusal().map(|f| (f.id, f.text));
            assert_eq!(protocol, library, "{given:?}");
            refused += usize::from(library.is_some());
        }
        assert_eq!(refused, 10, "ten of the specs are refused and four are built");
        let unread = PipelineSpec { depth: true, ..spec() };
        let draw = WireDraw {
            pipeline: WirePipeline::Inline(unread.clone()),
            vertices: Vec::new(),
            index: None,
            vertex_count: Some(3),
            instance_count: 1,
            bindings: Vec::new(),
            blend_constant: None,
            stencil_reference: Some(1),
        };
        let planned = layer.plan(SurfaceSet { clear: [0.0; 4], clear_depth: None, clear_stencil: None, draws: vec![draw_of(&draw)], passes: Vec::new(), uniform: Vec::new() });
        assert_eq!(unread.stencil_reference_refusal(Some(1)).map(|f| (f.id, f.text)), planned.err().map(|f| (f.id, f.text)), "a reference no stencil test reads");
        assert_eq!(PipelineSpec { depth_format: Some(DepthFormat::Stencil8), stencil: replace, ..spec() }.stencil_reference_refusal(Some(1)), None);
    }
}
