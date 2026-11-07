//! The GPU layer's commands, in the Wgpu module's terms: buffers of typed
//! elements and textures that live in the egui renderer, made from
//! PowerShell arrays or image files and read back as arrays with the Wgpu
//! module's own conversions, and the samplers that read textures; compute
//! runs; and render pipelines with the draws a GpuSurface makes with them.

use crate::canvas::TICKS_PER_SECOND;
use crate::session::{self, arm, gui_error};
use crate::widgets::full_path;
use crate::window;
use arc_swap::ArcSwapOption;
use pwrs::prelude::*;
use pwrs::sys::PS_TYPE_TIMESPAN;
use pwrs_gui_client::proto::camera::{CameraButton, CameraChange, CameraControls, CameraGesture, CameraModifiers, CameraMove, CameraSpec};
use pwrs_gui_client::proto::canvas::Easing;
use pwrs_gui_client::proto::view::{ViewChange, ViewControls, ViewEase, ViewFit, ViewGesture, ViewMove, ViewShown};
use pwrs_gui_client::proto::gpu::{
    AddressMode as WireAddress, Blend, BlendFactor as WireFactor, BlendOperation as WireOperation, BufferUsage, CompareFunction as WireCompare, Cull, CustomBlend, DepthFormat as WireDepthFormat,
    DepthTest as WireDepthTest, DrawPipeline, ElementType as WireType, GpuDispatch, GpuFailure, GpuOp, GpuResource, GpuSlot, GpuValue, IndexFormat, PageBinding, PageDraw, PagePass, PageResource,
    PageSurface, PageVertex, PassEvery, PipelineSpec, SamplerFilter, ShaderLanguage, StencilFace as WireFace, StencilOperation as WireStencil, StencilTest as WireStencilTest, SurfaceDraw, SurfacePass,
    TableObject, TextureFormat as WireFormat, Topology,
};
use pwrs_gui_client::{Canceler, GuiError, Renderer, Session};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use wgpu_pwsh::ElementType;

/// The type of a GPU buffer's elements.
#[psenum(name = "PwrsGui.ElementType")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiElementType {
    /// 32-bit float: WGSL `f32`, .NET `float`.
    #[default]
    Float32,
    /// 64-bit float: WGSL `f64`, .NET `double`.
    Float64,
    /// 16-bit float: WGSL `f16`, read and written as .NET `float`.
    Float16,
    /// 8-bit signed integer, .NET `sbyte`.
    Int8,
    /// 8-bit unsigned integer, .NET `byte`.
    UInt8,
    /// 16-bit signed integer, .NET `short`.
    Int16,
    /// 16-bit unsigned integer, .NET `ushort`.
    UInt16,
    /// 32-bit signed integer: WGSL `i32`, .NET `int`.
    Int32,
    /// 32-bit unsigned integer: WGSL `u32`, .NET `uint`.
    UInt32,
    /// 64-bit signed integer: WGSL `i64`, .NET `long`.
    Int64,
    /// 64-bit unsigned integer: WGSL `u64`, .NET `ulong`.
    UInt64,
}

/// What a GPU buffer may be used for.
#[psenum(name = "PwrsGui.BufferUsage")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiBufferUsage {
    /// Mapped for reading on the CPU.
    MapRead,
    /// Mapped for writing on the CPU.
    MapWrite,
    /// The source of a copy.
    CopySrc,
    /// The destination of a copy or a write.
    CopyDst,
    /// An index buffer of a draw.
    Index,
    /// A vertex buffer of a draw.
    Vertex,
    /// A uniform buffer binding.
    Uniform,
    /// A storage buffer binding.
    #[default]
    Storage,
    /// The arguments of an indirect dispatch or draw.
    Indirect,
    /// The destination of resolved query results.
    QueryResolve,
    /// Input to a bottom-level acceleration structure build.
    BlasInput,
    /// Input to a top-level acceleration structure build.
    TlasInput,
}

/// Every usage, in the order a buffer lists them.
const USAGES: [GuiBufferUsage; 12] = [
    GuiBufferUsage::MapRead,
    GuiBufferUsage::MapWrite,
    GuiBufferUsage::CopySrc,
    GuiBufferUsage::CopyDst,
    GuiBufferUsage::Index,
    GuiBufferUsage::Vertex,
    GuiBufferUsage::Uniform,
    GuiBufferUsage::Storage,
    GuiBufferUsage::Indirect,
    GuiBufferUsage::QueryResolve,
    GuiBufferUsage::BlasInput,
    GuiBufferUsage::TlasInput,
];

/// The usages a buffer gets when its maker names none: every usage but
/// mapping and acceleration-structure input.
const GENERAL: [GuiBufferUsage; 8] = [
    GuiBufferUsage::CopySrc,
    GuiBufferUsage::CopyDst,
    GuiBufferUsage::Index,
    GuiBufferUsage::Vertex,
    GuiBufferUsage::Uniform,
    GuiBufferUsage::Storage,
    GuiBufferUsage::Indirect,
    GuiBufferUsage::QueryResolve,
];

impl GuiBufferUsage {
    fn wire(self) -> BufferUsage {
        match self {
            GuiBufferUsage::MapRead => BufferUsage::MapRead,
            GuiBufferUsage::MapWrite => BufferUsage::MapWrite,
            GuiBufferUsage::CopySrc => BufferUsage::CopySrc,
            GuiBufferUsage::CopyDst => BufferUsage::CopyDst,
            GuiBufferUsage::Index => BufferUsage::Index,
            GuiBufferUsage::Vertex => BufferUsage::Vertex,
            GuiBufferUsage::Uniform => BufferUsage::Uniform,
            GuiBufferUsage::Storage => BufferUsage::Storage,
            GuiBufferUsage::Indirect => BufferUsage::Indirect,
            GuiBufferUsage::QueryResolve => BufferUsage::QueryResolve,
            GuiBufferUsage::BlasInput => BufferUsage::BlasInput,
            GuiBufferUsage::TlasInput => BufferUsage::TlasInput,
        }
    }
}

/// The usages `given` names, once each and in the order a buffer lists
/// them, or [`GENERAL`]'s when it names none.
fn usages(given: Option<&[GuiBufferUsage]>) -> Vec<GuiBufferUsage> {
    match given {
        Some(list) => USAGES.iter().copied().filter(|u| list.contains(u)).collect(),
        None => GENERAL.to_vec(),
    }
}

impl GuiElementType {
    fn element(self) -> ElementType {
        match self {
            GuiElementType::Float32 => ElementType::Float32,
            GuiElementType::Float64 => ElementType::Float64,
            GuiElementType::Float16 => ElementType::Float16,
            GuiElementType::Int8 => ElementType::Int8,
            GuiElementType::UInt8 => ElementType::UInt8,
            GuiElementType::Int16 => ElementType::Int16,
            GuiElementType::UInt16 => ElementType::UInt16,
            GuiElementType::Int32 => ElementType::Int32,
            GuiElementType::UInt32 => ElementType::UInt32,
            GuiElementType::Int64 => ElementType::Int64,
            GuiElementType::UInt64 => ElementType::UInt64,
        }
    }

    fn of(element: ElementType) -> GuiElementType {
        match element {
            ElementType::Float32 => GuiElementType::Float32,
            ElementType::Float64 => GuiElementType::Float64,
            ElementType::Float16 => GuiElementType::Float16,
            ElementType::Int8 => GuiElementType::Int8,
            ElementType::UInt8 => GuiElementType::UInt8,
            ElementType::Int16 => GuiElementType::Int16,
            ElementType::UInt16 => GuiElementType::UInt16,
            ElementType::Int32 => GuiElementType::Int32,
            ElementType::UInt32 => GuiElementType::UInt32,
            ElementType::Int64 => GuiElementType::Int64,
            ElementType::UInt64 => GuiElementType::UInt64,
        }
    }
}

/// The language of a compute shader's text.
#[psenum(name = "PwrsGui.ShaderLanguage")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiShaderLanguage {
    /// WebGPU Shading Language.
    #[default]
    Wgsl,
    /// OpenGL Shading Language, whose compute entry point is main.
    Glsl,
}

/// An element type as the protocol writes it.
fn wire(element: ElementType) -> WireType {
    match element {
        ElementType::Float32 => WireType::Float32,
        ElementType::Float64 => WireType::Float64,
        ElementType::Float16 => WireType::Float16,
        ElementType::Int8 => WireType::Int8,
        ElementType::UInt8 => WireType::UInt8,
        ElementType::Int16 => WireType::Int16,
        ElementType::UInt16 => WireType::UInt16,
        ElementType::Int32 => WireType::Int32,
        ElementType::UInt32 => WireType::UInt32,
        ElementType::Int64 => WireType::Int64,
        ElementType::UInt64 => WireType::UInt64,
    }
}

/// An element type from the protocol.
fn unwire(element: WireType) -> ElementType {
    match element {
        WireType::Float32 => ElementType::Float32,
        WireType::Float64 => ElementType::Float64,
        WireType::Float16 => ElementType::Float16,
        WireType::Int8 => ElementType::Int8,
        WireType::UInt8 => ElementType::UInt8,
        WireType::Int16 => ElementType::Int16,
        WireType::UInt16 => ElementType::UInt16,
        WireType::Int32 => ElementType::Int32,
        WireType::UInt32 => ElementType::UInt32,
        WireType::Int64 => ElementType::Int64,
        WireType::UInt64 => ElementType::UInt64,
    }
}

/// A GPU buffer of typed elements in the egui renderer. Its data stays on
/// the GPU until something reads it, and the renderer frees it when the
/// object is collected or Remove-GuiGpuBuffer frees it; a GpuSurface
/// drawing with it uses it until its draws change.
#[psclass(name = "PwrsGui.GpuBuffer", mode = proxy, columns = ["Label", "ElementType", "Length", "Size", "Adapter"])]
pub struct GpuBuffer {
    /// The buffer's label.
    pub label: String,
    /// The type of its elements.
    pub element_type: GuiElementType,
    /// The number of elements in it.
    pub length: u64,
    /// Its size in bytes, the elements' bytes rounded up to a multiple of 4.
    pub size: u64,
    /// What it may be used for.
    pub usage: Vec<GuiBufferUsage>,
    /// The adapter of the renderer's device; null for a renderer another
    /// module started, which this session did not start.
    pub adapter: Option<String>,
    #[psfield(skip)]
    pub(crate) id: u64,
    #[psfield(skip)]
    pub(crate) session: Session,
    #[psfield(skip)]
    removed: AtomicBool,
}

impl GpuBuffer {
    fn element(&self) -> ElementType {
        self.element_type.element()
    }

    /// The renderer's number for the buffer, refused once it is removed.
    fn live(&self) -> PsResult<u64> {
        if self.removed.load(Ordering::Acquire) {
            return Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiGpuRemoved", format!("the GPU buffer {} was removed", self.label)));
        }
        Ok(self.id)
    }

    /// The element range `start..start + count` in bytes, checked against
    /// the buffer's length.
    fn byte_range(&self, start: u64, count: Option<u64>) -> PsResult<(u64, u64)> {
        if start > self.length {
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuRange", format!("element {start} is past the end of a buffer of {} elements", self.length)));
        }
        let count = count.unwrap_or(self.length - start);
        if start + count > self.length {
            let message = format!("elements {start} to {} run past the end of a buffer of {} elements", start + count, self.length);
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuRange", message));
        }
        let size = self.element().size();
        Ok((start * size, count * size))
    }
}

impl Drop for GpuBuffer {
    fn drop(&mut self) {
        if self.removed.load(Ordering::Acquire) || !self.session.is_alive() {
            return;
        }
        if let Err(e) = self.session.gpu_free(self.id) {
            eprintln!("PWRS-GUI: GPU buffer {} ({}) could not be freed as its object was collected: {e}", self.id, self.label);
        }
    }
}

/// What PwrsGui.DeviceLost gives as its MessageData.
#[psclass(name = "PwrsGui.GpuDeviceEvent", columns = ["Reason"])]
#[derive(Default, Clone)]
pub struct GpuDeviceEvent {
    /// Why the egui renderer's GPU device was lost, as wgpu reported it.
    pub reason: String,
}

/// A runspace that has used the GPU, by its InstanceId, and the events
/// PwrsGui.DeviceLost is raised through in it.
#[derive(Clone)]
struct Listener {
    runspace: String,
    events: PsEvents,
}

/// Every runspace that has used the GPU.
static LISTENERS: ArcSwapOption<Vec<Listener>> = ArcSwapOption::const_empty();

/// The sessions whose GPU device a thread waits on for its loss.
static WATCHED: ArcSwapOption<Vec<Session>> = ArcSwapOption::const_empty();

fn listeners_now(now: &Option<Arc<Vec<Listener>>>) -> Vec<Listener> {
    match now {
        Some(list) => list.as_ref().clone(),
        None => Vec::new(),
    }
}

fn watched_now(now: &Option<Arc<Vec<Session>>>) -> Vec<Session> {
    match now {
        Some(list) => list.as_ref().clone(),
        None => Vec::new(),
    }
}

/// Registers the runspace `ps` runs in for PwrsGui.DeviceLost, and waits on
/// `session`'s device for its loss unless a thread waits on it already. A
/// runspace that cannot be registered is warned that it will not be told of
/// a loss; its GPU
/// call goes ahead.
fn listen(ps: &Pipeline<'_>, session: &Session) -> PsResult<()> {
    if let Err(e) = register(ps) {
        pwrs::warning!(ps, "this runspace cannot be told when the GPU device is lost, so PwrsGui.DeviceLost will not reach it: {}", e.message)?;
    }
    watch(session);
    Ok(())
}

fn register(ps: &Pipeline<'_>) -> PsResult<()> {
    let id = PsType::from_name("runspace").call_static("get_DefaultRunspace", &[])?.get("InstanceId")?.call("ToString", &[])?;
    let runspace = String::from_ps(&id)?;
    if LISTENERS.load().as_ref().is_some_and(|list| list.iter().any(|l| l.runspace == runspace)) {
        return Ok(());
    }
    let events = ps.events()?;
    LISTENERS.rcu(|now| {
        let mut list = listeners_now(now);
        if !list.iter().any(|l| l.runspace == runspace) {
            list.push(Listener { runspace: runspace.clone(), events: events.clone() });
        }
        Some(Arc::new(list))
    });
    Ok(())
}

/// Starts a thread that waits for `session`'s GPU device to be lost and
/// raises PwrsGui.DeviceLost in every listening runspace each time, unless
/// one waits on it already; the thread ends with the session.
fn watch(session: &Session) {
    if WATCHED.load().as_ref().is_some_and(|list| list.iter().any(|s| s.same(session))) {
        return;
    }
    let mut added = false;
    WATCHED.rcu(|now| {
        let mut list = watched_now(now);
        added = !list.iter().any(|s| s.same(session));
        if added {
            list.push(session.clone());
        }
        Some(Arc::new(list))
    });
    if !added {
        return;
    }
    let watched = session.clone();
    if let Err(e) = std::thread::Builder::new().name(String::from("pwrs-gui-device-loss")).spawn(move || watch_loss(watched)) {
        eprintln!("PWRS-GUI: the wait for a lost GPU device could not start, so PwrsGui.DeviceLost will not be raised: {e}");
        WATCHED.rcu(|now| Some(Arc::new(watched_now(now).into_iter().filter(|s| !s.same(session)).collect())));
    }
}

fn watch_loss(session: Session) {
    loop {
        let answer = match session.gpu(GpuOp::WatchLoss, |_: Canceler| Ok::<(), GuiError>(())) {
            Ok(answer) => answer,
            // The session ended, and the wait with it.
            Err(GuiError::Gone(_)) => break,
            Err(e) => {
                eprintln!("PWRS-GUI: the wait for a lost GPU device ended: {e}");
                break;
            }
        };
        match answer {
            Ok(GpuValue::Lost { reason }) => {
                raise_lost(&reason);
                resend_copies(&session);
            }
            Ok(other) => {
                eprintln!("PWRS-GUI: the renderer replied to a wait for a lost GPU device with {other:?}");
                break;
            }
            // The renderer ended, or has no GPU layer, which each GPU
            // command reports itself.
            Err(f) if f.id == "GuiGpuEnded" || f.id == "GuiGpuNoLayer" => break,
            Err(f) => {
                eprintln!("PWRS-GUI: the wait for a lost GPU device was refused: {} ({})", f.text, f.id);
                break;
            }
        }
    }
    WATCHED.rcu(|now| Some(Arc::new(watched_now(now).into_iter().filter(|s| !s.same(&session)).collect())));
}

/// Raises PwrsGui.DeviceLost in every listening runspace; one that cannot
/// take it, as a closed runspace cannot, is told no more.
fn raise_lost(reason: &str) {
    let Some(listeners) = LISTENERS.load_full() else {
        return;
    };
    let mut gone = Vec::new();
    for l in listeners.iter() {
        if let Err(e) = l.events.raise("PwrsGui.DeviceLost", GpuDeviceEvent { reason: reason.to_string() }) {
            eprintln!("PWRS-GUI: runspace {} could not be told its GPU device was lost, and is told no more: {}", l.runspace, e.message);
            gone.push(l.runspace.clone());
        }
    }
    if !gone.is_empty() {
        LISTENERS.rcu(|now| Some(Arc::new(listeners_now(now).into_iter().filter(|l| !gone.contains(&l.runspace)).collect())));
    }
}

/// A GPU-layer failure as PowerShell reports it, under the id the layer or
/// the Wgpu module gave it.
fn failed(f: GpuFailure) -> PsError {
    let category = match f.id.as_str() {
        "GuiGpuNoBuffer" | "GuiGpuNoPipeline" | "GuiGpuNoSurface" | "GuiGpuNoTexture" | "GuiGpuNoSampler" | "GuiGpuNoCopy" | "GuiNoCanvas" => ErrorCategory::ObjectNotFound,
        "GuiGpuNoLayer" | "GuiGpuEnded" | "GuiGpuNoWebGpu" => ErrorCategory::ResourceUnavailable,
        "GuiGpuTextureSize" | "GuiGpuImageFormat" | "GuiCanvasView" => ErrorCategory::InvalidArgument,
        "GuiGpuImage" => ErrorCategory::InvalidData,
        _ => ErrorCategory::InvalidOperation,
    };
    PsError::new(category, &f.id, f.text)
}

/// The renderer's reply to `op`, or its failure.
pub(crate) fn call(ps: &Pipeline<'_>, session: &Session, op: GpuOp) -> PsResult<GpuValue> {
    session.gpu(op, arm(ps)).map_err(gui_error)?.map_err(failed)
}

/// The error for a reply of a kind `what` does not take.
fn unexpected(what: &str, value: &GpuValue) -> PsError {
    PsError::new(ErrorCategory::ProtocolError, "GuiGpuAnswer", format!("the renderer replied to {what} with {value:?}")).terminating()
}

/// A buffer of `bytes` with `length` elements of `element`, made in the
/// egui renderer with `usage` and labeled `label`, or zeroed when `bytes` is
/// `None`.
fn make(ps: &Pipeline<'_>, element: ElementType, length: u64, bytes: Option<Vec<u8>>, usage: Option<&[GuiBufferUsage]>, label: Option<String>) -> PsResult<GpuBuffer> {
    make_in(ps, session::egui_session(ps)?, element, length, bytes, usage, label)
}

/// [`make`] in `session`'s renderer.
fn make_in(ps: &Pipeline<'_>, session: Session, element: ElementType, length: u64, bytes: Option<Vec<u8>>, usage: Option<&[GuiBufferUsage]>, label: Option<String>) -> PsResult<GpuBuffer> {
    if length == 0 {
        return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuEmpty", "a buffer needs at least one element"));
    }
    listen(ps, &session)?;
    let label = label.unwrap_or_else(|| format!("{length} x {element:?}"));
    let wire_usage = usage.map(|list| list.iter().map(|u| u.wire()).collect());
    let op = match bytes {
        Some(bytes) => GpuOp::NewBuffer { bytes, label: Some(label.clone()), usage: wire_usage },
        None => {
            let size = length
                .checked_mul(element.size())
                .ok_or_else(|| PsError::new(ErrorCategory::InvalidArgument, "GuiGpuBufferTooLarge", format!("{length} elements of {element:?} is more bytes than a buffer can address")))?;
            GpuOp::NewZeroed { length: size, label: Some(label.clone()), usage: wire_usage }
        }
    };
    let (id, bytes) = match call(ps, &session, op)? {
        GpuValue::Buffer { buffer, length } => (buffer, length),
        other => return Err(unexpected("a new buffer", &other)),
    };
    let adapter = session.hello().map(|h| h.capabilities.adapter.clone());
    Ok(GpuBuffer {
        label,
        element_type: GuiElementType::of(element),
        length,
        size: bytes.max(4).next_multiple_of(4),
        usage: usages(usage),
        adapter,
        id,
        session,
        removed: AtomicBool::new(false),
    })
}

/// Creates a GPU buffer in the egui renderer from an array of any numeric
/// type, or of a length.
///
/// The buffer stays on the GPU until something reads it. Without -Type it
/// takes the array's own element type (Float32 for an untyped array); with
/// -Type the values are converted, as the Wgpu module converts them.
///
/// # Examples
/// $buf = New-GuiGpuBuffer ([float[]](1..1024))
/// $buf = New-GuiGpuBuffer -Length 4096 -Type UInt32
/// 1..100 | New-GuiGpuBuffer -Type Int32
#[cmdlet(verb = "New", noun = "GuiGpuBuffer", default_parameter_set = "Data", output = ["PwrsGui.GpuBuffer"])]
#[derive(Default)]
pub struct NewGuiGpuBuffer {
    /// The values: an array of any numeric type, a number, or numbers piped in.
    #[param(position = 0, value_from_pipeline, raw, set = "Data")]
    pub data: Option<PsObject>,
    /// The number of zeroed elements.
    #[param(mandatory, set = "Length")]
    pub length: u64,
    /// The element type; the array's own type, or Float32, when omitted.
    #[param(alias = ["Type"])]
    pub element_type: Option<GuiElementType>,
    /// What the buffer may be used for; every usage but mapping and acceleration-structure input when omitted.
    #[param]
    pub usage: Option<Vec<GuiBufferUsage>>,
    /// The buffer's label.
    #[param]
    pub label: Option<String>,
    collected: Vec<PsObject>,
}

impl Cmdlet for NewGuiGpuBuffer {
    fn process(&mut self, _ps: &Pipeline<'_>) -> PsResult<()> {
        if let Some(data) = &self.data {
            self.collected.push(data.clone());
        }
        Ok(())
    }

    fn end(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let usage = self.usage.as_deref();
        if self.collected.is_empty() {
            let element = self.element_type.map_or(ElementType::Float32, GuiElementType::element);
            return ps.write(make(ps, element, self.length, None, usage, self.label.clone())?);
        }
        let mut element = self.element_type.map(GuiElementType::element);
        let mut bytes = Vec::new();
        for item in &self.collected {
            let (own, more) = wgpu_pwsh::host_bytes(item, element)?;
            element = Some(own);
            bytes.extend_from_slice(&more);
        }
        let element = element.unwrap_or(ElementType::Float32);
        let length = bytes.len() as u64 / element.size();
        ps.write(make(ps, element, length, Some(bytes), usage, self.label.clone())?)
    }
}

/// Copies a GPU buffer back from the renderer and writes its elements as
/// one array.
///
/// # Examples
/// Read-GuiGpuBuffer $buf
/// $buf | Read-GuiGpuBuffer -Offset 10 -Count 5
/// Read-GuiGpuBuffer $buf -Type UInt32
/// Read-GuiGpuBuffer $buf -Enumerate | Measure-Object -Sum
#[cmdlet(verb = "Read", noun = "GuiGpuBuffer", output = ["System.Array"])]
#[derive(Default)]
pub struct ReadGuiGpuBuffer {
    /// The buffer to read.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub buffer: Option<PsProxy<GpuBuffer>>,
    /// The first element to read; 0 when omitted.
    #[param]
    pub offset: Option<u64>,
    /// The number of elements to read; to the end when omitted.
    #[param]
    pub count: Option<u64>,
    /// Read the bytes as this element type instead of the buffer's own.
    #[param(alias = ["Type"])]
    pub element_type: Option<GuiElementType>,
    /// Write a Memory<T> over the values instead of an array (an array on Windows PowerShell).
    #[param]
    pub as_memory: bool,
    /// Write each element to the pipeline rather than one array.
    #[param]
    pub enumerate: bool,
}

impl Cmdlet for ReadGuiGpuBuffer {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(proxy) = &self.buffer else {
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuNoBuffer", "no buffer was given"));
        };
        let (session, id, element, (offset, length)) = proxy.with(|b| -> PsResult<_> { Ok((b.session.clone(), b.live()?, b.element(), b.byte_range(self.offset.unwrap_or(0), self.count)?)) })??;
        listen(ps, &session)?;
        let bytes = match call(ps, &session, GpuOp::Read { buffer: id, offset, length: Some(length) })? {
            GpuValue::Bytes(bytes) => bytes,
            other => return Err(unexpected("a read", &other)),
        };
        let element = self.element_type.map_or(element, GuiElementType::element);
        let values = if self.as_memory { wgpu_pwsh::to_clr_memory(element, &bytes)? } else { wgpu_pwsh::to_clr_array(element, &bytes)? };
        if self.enumerate { ps.write_enumerated(&values) } else { ps.write_object(&values) }
    }
}

/// Overwrites elements of a GPU buffer with new values.
///
/// # Examples
/// Write-GuiGpuBuffer $buf ([float[]](1, 2, 3))
/// Write-GuiGpuBuffer $buf 7 -Offset 100
#[cmdlet(verb = "Write", noun = "GuiGpuBuffer")]
#[derive(Default)]
pub struct WriteGuiGpuBuffer {
    /// The buffer to write.
    #[param(mandatory, position = 0)]
    pub buffer: Option<PsProxy<GpuBuffer>>,
    /// The values, converted to the buffer's element type.
    #[param(mandatory, position = 1, raw)]
    pub data: Option<PsObject>,
    /// The first element to overwrite; 0 when omitted.
    #[param]
    pub offset: Option<u64>,
}

impl Cmdlet for WriteGuiGpuBuffer {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let (Some(proxy), Some(data)) = (&self.buffer, &self.data) else {
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuNoData", "a buffer and data are both needed"));
        };
        let (session, id, offset, bytes) = proxy.with(|b| -> PsResult<_> {
            let element = b.element();
            let (_, bytes) = wgpu_pwsh::host_bytes(data, Some(element))?;
            let (offset, _) = b.byte_range(self.offset.unwrap_or(0), Some(bytes.len() as u64 / element.size()))?;
            Ok((b.session.clone(), b.live()?, offset, bytes))
        })??;
        listen(ps, &session)?;
        let mirrored = mirroring(TableObject::Buffer(id));
        let sent = mirrored.as_ref().map(|_| bytes.clone());
        match call(ps, &session, GpuOp::Write { buffer: id, offset, bytes })? {
            GpuValue::Done => {}
            other => return Err(unexpected("a write", &other)),
        }
        // A page drawing from the buffer takes the bytes written too.
        if let (Some((page, Mirrored::Buffer { length, .. })), Some(bytes)) = (mirrored, sent) {
            mirror_through(&page, GpuOp::MirrorBuffer { buffer: id, length, offset, bytes }, &|s, op| call(ps, s, op))?;
        }
        Ok(())
    }
}

/// Takes a GPU buffer out of the renderer now, rather than when its object
/// is collected, and frees its memory; a GpuSurface still drawing with it
/// uses the memory until its draws change or its window closes.
///
/// # Examples
/// $buf | Remove-GuiGpuBuffer
#[cmdlet(verb = "Remove", noun = "GuiGpuBuffer", supports_should_process)]
#[derive(Default)]
pub struct RemoveGuiGpuBuffer {
    /// The buffer to free.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub buffer: Option<PsProxy<GpuBuffer>>,
}

impl Cmdlet for RemoveGuiGpuBuffer {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(proxy) = &self.buffer else {
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuNoBuffer", "no buffer was given"));
        };
        let (session, id, label, removed) = proxy.with(|b| (b.session.clone(), b.id, b.label.clone(), b.removed.load(Ordering::Acquire)))?;
        if removed || !ps.should_process(&label, "Free the buffer's GPU memory")? {
            return Ok(());
        }
        listen(ps, &session)?;
        match call(ps, &session, GpuOp::Remove { buffer: id })? {
            GpuValue::Done => proxy.with(|b| b.removed.store(true, Ordering::Release)),
            other => Err(unexpected("a removal", &other)),
        }
    }
}

/// The format of a texture's texels.
#[psenum(name = "PwrsGui.TextureFormat")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiTextureFormat {
    /// One 8-bit channel, sampled as 0 to 1.
    R8Unorm,
    /// Two 8-bit channels, sampled as 0 to 1.
    Rg8Unorm,
    /// Red, green, blue and alpha bytes, sampled as 0 to 1.
    #[default]
    Rgba8Unorm,
    /// Red, green, blue and alpha bytes whose color is sRGB-encoded, sampled as linear values from 0 to 1.
    Rgba8UnormSrgb,
    /// Blue, green, red and alpha bytes, sampled as red, green, blue and alpha from 0 to 1.
    Bgra8Unorm,
    /// Blue, green, red and alpha bytes whose color is sRGB-encoded, sampled as red, green, blue and alpha in linear values.
    Bgra8UnormSrgb,
    /// One 16-bit float channel, written and read as .NET float.
    R16Float,
    /// Two 16-bit float channels, written and read as .NET float.
    Rg16Float,
    /// Four 16-bit float channels, written and read as .NET float.
    Rgba16Float,
    /// One 32-bit float channel.
    R32Float,
    /// Two 32-bit float channels.
    Rg32Float,
    /// Four 32-bit float channels.
    Rgba32Float,
    /// One 32-bit unsigned integer channel, which a shader loads as u32 rather than samples.
    R32Uint,
    /// One 32-bit signed integer channel, which a shader loads as i32 rather than samples.
    R32Sint,
}

impl GuiTextureFormat {
    fn wire(self) -> WireFormat {
        match self {
            GuiTextureFormat::R8Unorm => WireFormat::R8Unorm,
            GuiTextureFormat::Rg8Unorm => WireFormat::Rg8Unorm,
            GuiTextureFormat::Rgba8Unorm => WireFormat::Rgba8Unorm,
            GuiTextureFormat::Rgba8UnormSrgb => WireFormat::Rgba8UnormSrgb,
            GuiTextureFormat::Bgra8Unorm => WireFormat::Bgra8Unorm,
            GuiTextureFormat::Bgra8UnormSrgb => WireFormat::Bgra8UnormSrgb,
            GuiTextureFormat::R16Float => WireFormat::R16Float,
            GuiTextureFormat::Rg16Float => WireFormat::Rg16Float,
            GuiTextureFormat::Rgba16Float => WireFormat::Rgba16Float,
            GuiTextureFormat::R32Float => WireFormat::R32Float,
            GuiTextureFormat::Rg32Float => WireFormat::Rg32Float,
            GuiTextureFormat::Rgba32Float => WireFormat::Rgba32Float,
            GuiTextureFormat::R32Uint => WireFormat::R32Uint,
            GuiTextureFormat::R32Sint => WireFormat::R32Sint,
        }
    }

    fn of(format: WireFormat) -> GuiTextureFormat {
        match format {
            WireFormat::R8Unorm => GuiTextureFormat::R8Unorm,
            WireFormat::Rg8Unorm => GuiTextureFormat::Rg8Unorm,
            WireFormat::Rgba8Unorm => GuiTextureFormat::Rgba8Unorm,
            WireFormat::Rgba8UnormSrgb => GuiTextureFormat::Rgba8UnormSrgb,
            WireFormat::Bgra8Unorm => GuiTextureFormat::Bgra8Unorm,
            WireFormat::Bgra8UnormSrgb => GuiTextureFormat::Bgra8UnormSrgb,
            WireFormat::R16Float => GuiTextureFormat::R16Float,
            WireFormat::Rg16Float => GuiTextureFormat::Rg16Float,
            WireFormat::Rgba16Float => GuiTextureFormat::Rgba16Float,
            WireFormat::R32Float => GuiTextureFormat::R32Float,
            WireFormat::Rg32Float => GuiTextureFormat::Rg32Float,
            WireFormat::Rgba32Float => GuiTextureFormat::Rgba32Float,
            WireFormat::R32Uint => GuiTextureFormat::R32Uint,
            WireFormat::R32Sint => GuiTextureFormat::R32Sint,
        }
    }

    /// The number type a texel's channels are written and read as.
    fn element(self) -> ElementType {
        match self {
            GuiTextureFormat::R8Unorm | GuiTextureFormat::Rg8Unorm | GuiTextureFormat::Rgba8Unorm | GuiTextureFormat::Rgba8UnormSrgb | GuiTextureFormat::Bgra8Unorm | GuiTextureFormat::Bgra8UnormSrgb => {
                ElementType::UInt8
            }
            GuiTextureFormat::R16Float | GuiTextureFormat::Rg16Float | GuiTextureFormat::Rgba16Float => ElementType::Float16,
            GuiTextureFormat::R32Float | GuiTextureFormat::Rg32Float | GuiTextureFormat::Rgba32Float => ElementType::Float32,
            GuiTextureFormat::R32Uint => ElementType::UInt32,
            GuiTextureFormat::R32Sint => ElementType::Int32,
        }
    }
}

/// How a sampler picks between texels.
#[psenum(name = "PwrsGui.FilterMode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiFilterMode {
    /// The nearest texel.
    #[default]
    Nearest,
    /// A blend of the nearest texels.
    Linear,
}

impl GuiFilterMode {
    fn wire(self) -> SamplerFilter {
        match self {
            GuiFilterMode::Nearest => SamplerFilter::Nearest,
            GuiFilterMode::Linear => SamplerFilter::Linear,
        }
    }
}

/// What a sampler reads past a texture's edges.
#[psenum(name = "PwrsGui.AddressMode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiAddressMode {
    /// The edge texel.
    #[default]
    ClampToEdge,
    /// The texture again.
    Repeat,
    /// The texture mirrored.
    MirrorRepeat,
}

impl GuiAddressMode {
    fn wire(self) -> WireAddress {
        match self {
            GuiAddressMode::ClampToEdge => WireAddress::ClampToEdge,
            GuiAddressMode::Repeat => WireAddress::Repeat,
            GuiAddressMode::MirrorRepeat => WireAddress::MirrorRepeat,
        }
    }
}

/// A texture in the egui renderer: a grid of texels of one format, which a
/// GpuSurface's draws and passes and Invoke-GuiGpuCompute bind by name. Its
/// texels stay on the GPU until something reads them, and the renderer
/// frees it when the object is collected or Remove-GuiGpuTexture frees it;
/// a GpuSurface drawing with it uses it until its draws change.
#[psclass(name = "PwrsGui.GpuTexture", mode = proxy, columns = ["Label", "Width", "Height", "Format", "Adapter"])]
pub struct GpuTexture {
    /// The texture's label.
    pub label: String,
    /// Its width in texels.
    pub width: u32,
    /// Its height in texels.
    pub height: u32,
    /// The format of its texels.
    pub format: GuiTextureFormat,
    /// The adapter of the renderer's device; null for a renderer another
    /// module started, which this session did not start.
    pub adapter: Option<String>,
    #[psfield(skip)]
    pub(crate) id: u64,
    #[psfield(skip)]
    pub(crate) session: Session,
    #[psfield(skip)]
    removed: AtomicBool,
}

impl GpuTexture {
    /// The renderer's number for the texture, refused once it is removed.
    fn live(&self) -> PsResult<u64> {
        if self.removed.load(Ordering::Acquire) {
            return Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiGpuRemoved", format!("the GPU texture {} was removed", self.label)));
        }
        Ok(self.id)
    }
}

impl Drop for GpuTexture {
    fn drop(&mut self) {
        if self.removed.load(Ordering::Acquire) || !self.session.is_alive() {
            return;
        }
        if let Err(e) = self.session.gpu_free_texture(self.id) {
            eprintln!("PWRS-GUI: GPU texture {} ({}) could not be freed as its object was collected: {e}", self.id, self.label);
        }
    }
}

/// A sampler in the egui renderer: how a shader filters and wraps when it
/// samples a texture, the same in every direction and when magnifying and
/// minifying alike. A draw, a pass or a compute run binds it by name, as it
/// binds a texture. The renderer frees it when the object is collected; a
/// GpuSurface drawing with it uses it until its draws change.
#[psclass(name = "PwrsGui.GpuSampler", mode = proxy, columns = ["Label", "Filter", "AddressMode", "Adapter"])]
pub struct GpuSampler {
    /// The sampler's label.
    pub label: String,
    /// How it picks between texels.
    pub filter: GuiFilterMode,
    /// What it reads past a texture's edges.
    pub address_mode: GuiAddressMode,
    /// The adapter of the renderer's device; null for a renderer another
    /// module started, which this session did not start.
    pub adapter: Option<String>,
    #[psfield(skip)]
    pub(crate) id: u64,
    #[psfield(skip)]
    pub(crate) session: Session,
}

impl Drop for GpuSampler {
    fn drop(&mut self) {
        if !self.session.is_alive() {
            return;
        }
        if let Err(e) = self.session.gpu_free_sampler(self.id) {
            eprintln!("PWRS-GUI: GPU sampler {} ({}) could not be freed as its object was collected: {e}", self.id, self.label);
        }
    }
}

/// The texture the renderer returned as `value`, in `session`'s renderer
/// and labeled `label`.
fn texture_of(session: Session, value: GpuValue, label: String) -> PsResult<GpuTexture> {
    let GpuValue::Texture { texture, width, height, format } = value else {
        return Err(unexpected("a new texture", &value));
    };
    let adapter = session.hello().map(|h| h.capabilities.adapter.clone());
    Ok(GpuTexture { label, width, height, format: GuiTextureFormat::of(format), adapter, id: texture, session, removed: AtomicBool::new(false) })
}

/// Creates a texture in the egui renderer: a grid of texels of one format,
/// zeroed or filled from an array, which a GpuSurface's draws and passes
/// and Invoke-GuiGpuCompute bind by name.
///
/// Without -Format the texture is Rgba8Unorm. -Data gives every texel, rows
/// packed from the top, as the format's number type: byte for the 8-bit
/// formats, float for the float formats, uint for R32Uint and int for
/// R32Sint, converted as the Wgpu module converts them.
///
/// # Examples
/// $tex = New-GuiGpuTexture -Width 256 -Height 256
/// New-GuiGpuTexture 2 2 -Format R32Float -Data ([float[]](0, 0.25, 0.5, 1))
/// New-GuiGpuTexture 1 1 -Data ([byte[]](255, 0, 0, 255)) -Label red
#[cmdlet(verb = "New", noun = "GuiGpuTexture", output = ["PwrsGui.GpuTexture"])]
#[derive(Default)]
pub struct NewGuiGpuTexture {
    /// Width in texels.
    #[param(mandatory, position = 0)]
    pub width: u32,
    /// Height in texels; 1 when omitted.
    #[param(position = 1)]
    pub height: Option<u32>,
    /// The format of its texels; Rgba8Unorm when omitted.
    #[param]
    pub format: Option<GuiTextureFormat>,
    /// Every texel, rows packed from the top, as the format's number type; zeroed when omitted.
    #[param(raw)]
    pub data: Option<PsObject>,
    /// The texture's label.
    #[param]
    pub label: Option<String>,
}

impl Cmdlet for NewGuiGpuTexture {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let format = self.format.unwrap_or(GuiTextureFormat::Rgba8Unorm);
        let height = self.height.unwrap_or(1);
        let texels = match self.data.as_ref().filter(|d| !d.is_null()) {
            Some(data) => Some(wgpu_pwsh::host_bytes(data, Some(format.element()))?.1),
            None => None,
        };
        let label = match &self.label {
            Some(l) => l.clone(),
            None => format!("{} x {height} {format:?}", self.width),
        };
        let session = session::egui_session(ps)?;
        listen(ps, &session)?;
        let value = call(ps, &session, GpuOp::NewTexture { width: self.width, height, format: format.wire(), texels, label: Some(label.clone()) })?;
        ps.write(texture_of(session, value, label)?)
    }
}

/// The files -Path or -LiteralPath names: every file a -Path wildcard
/// matches, or the one a -LiteralPath names as written, which may be a path
/// a provider qualifies, as the PSPath Get-ChildItem pipes is. A path that
/// names no file is refused with PowerShell's reason.
fn image_files(ps: &Pipeline<'_>, path: &str, literal: bool) -> PsResult<Vec<String>> {
    let files = ps.resolve_path(path, literal).map_err(|e| {
        if e.category == ErrorCategory::OperationStopped {
            return e;
        }
        let reason = e.message.split_once(": ").filter(|(kind, _)| kind.ends_with("Exception")).map_or(e.message.as_str(), |(_, rest)| rest);
        PsError::new(ErrorCategory::ObjectNotFound, "GuiGpuImage", reason)
    })?;
    if files.is_empty() {
        return Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiGpuImage", format!("{path} names no file")));
    }
    Ok(files)
}

/// Loads an image file into a texture in the egui renderer: PNG, JPEG,
/// GIF, WebP, BMP or ICO, decoded to eight bits a channel.
///
/// The texture is Rgba8Unorm, which a GpuSurface shows byte for byte, so a
/// draw that writes the image's texels as it samples them shows the image
/// as the file has it. -Srgb makes it Rgba8UnormSrgb, whose samples are
/// linear values for a shader to light, encoded as sRGB again before the
/// draw writes them. A file that cannot be loaded is reported, and the
/// files after it load.
///
/// # Examples
/// $photo = Import-GuiGpuImage .\photo.jpg
/// Import-GuiGpuImage .\albedo.png -Srgb -Label albedo
/// Get-ChildItem .\icons -Filter *.png | Import-GuiGpuImage
#[cmdlet(verb = "Import", noun = "GuiGpuImage", default_parameter_set = "Path", output = ["PwrsGui.GpuTexture"])]
#[derive(Default)]
pub struct ImportGuiGpuImage {
    /// The image file; a wildcard loads every file it matches.
    #[param(mandatory, position = 0, set = "Path", value_from_pipeline_by_property_name)]
    pub path: Option<String>,
    /// The image file, named exactly as written, wildcard characters included; the files Get-ChildItem pipes bind here.
    #[param(mandatory, set = "LiteralPath", value_from_pipeline_by_property_name, alias = ["PSPath", "LP", "FullName"])]
    pub literal_path: Option<String>,
    /// The texture's format, Rgba8Unorm or Rgba8UnormSrgb; Rgba8Unorm, or Rgba8UnormSrgb with -Srgb, when omitted.
    #[param]
    pub format: Option<GuiTextureFormat>,
    /// Read the image's color as sRGB, into a texture of Rgba8UnormSrgb.
    #[param]
    pub srgb: bool,
    /// The texture's label; the file's path when omitted.
    #[param]
    pub label: Option<String>,
}

impl ImportGuiGpuImage {
    /// The format -Format and -Srgb give the texture.
    fn format(&self) -> PsResult<GuiTextureFormat> {
        match (self.format, self.srgb) {
            (None, false) => Ok(GuiTextureFormat::Rgba8Unorm),
            (None, true) | (Some(GuiTextureFormat::Rgba8UnormSrgb), true) => Ok(GuiTextureFormat::Rgba8UnormSrgb),
            (Some(named), false) => Ok(named),
            (Some(named), true) => Err(binding_error("GuiGpuImageFormat", format!("-Srgb reads the image into Rgba8UnormSrgb, and -Format names {named:?}"))),
        }
    }

    /// The texture of the image file `file`, made in `session`'s renderer
    /// as `format`.
    fn import(&self, ps: &Pipeline<'_>, session: &Session, file: &str, format: GuiTextureFormat) -> PsResult<GpuTexture> {
        let in_file = |mut e: PsError| {
            e.message = format!("{file}: {}", e.message);
            e
        };
        let bytes = std::fs::read(file).map_err(|e| PsError::new(ErrorCategory::ReadError, "GuiGpuImage", format!("{file} could not be read: {e}")))?;
        let label = match &self.label {
            Some(l) => l.clone(),
            None => file.to_string(),
        };
        let value = call(ps, session, GpuOp::ImportImage { bytes, format: format.wire(), label: Some(label.clone()) }).map_err(in_file)?;
        texture_of(session.clone(), value, label)
    }
}

impl Cmdlet for ImportGuiGpuImage {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let format = self.format()?;
        let files = match (self.path.as_deref(), self.literal_path.as_deref()) {
            (Some(path), _) => image_files(ps, path, false)?,
            (None, Some(path)) => image_files(ps, path, true)?,
            (None, None) => return Err(binding_error("GuiGpuImage", String::from("give the file as -Path or -LiteralPath"))),
        };
        let session = session::egui_session(ps)?;
        listen(ps, &session)?;
        for file in files {
            if ps.stopping() {
                break;
            }
            match self.import(ps, &session, &file, format) {
                Ok(texture) => ps.write(texture)?,
                Err(e) if e.terminating || e.category == ErrorCategory::OperationStopped => return Err(e),
                Err(e) => ps.write_error(&e)?,
            }
        }
        Ok(())
    }
}

/// Copies a texture's texels back from the renderer and writes them as one
/// array, rows packed from the top, as the format's number type.
///
/// # Examples
/// Read-GuiGpuTexture $tex
/// $tex | Read-GuiGpuTexture -AsBytes
#[cmdlet(verb = "Read", noun = "GuiGpuTexture", output = ["System.Array"])]
#[derive(Default)]
pub struct ReadGuiGpuTexture {
    /// The texture to read.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub texture: Option<PsProxy<GpuTexture>>,
    /// Write the texels as byte[] instead of the format's number type.
    #[param]
    pub as_bytes: bool,
}

impl Cmdlet for ReadGuiGpuTexture {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(proxy) = &self.texture else {
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuNoTexture", "no texture was given"));
        };
        let (session, id, format) = proxy.with(|t| -> PsResult<_> { Ok((t.session.clone(), t.live()?, t.format)) })??;
        listen(ps, &session)?;
        let bytes = match call(ps, &session, GpuOp::ReadTexture { texture: id })? {
            GpuValue::Bytes(bytes) => bytes,
            other => return Err(unexpected("a texture's read", &other)),
        };
        let element = if self.as_bytes { ElementType::UInt8 } else { format.element() };
        ps.write_object(&wgpu_pwsh::to_clr_array(element, &bytes)?)
    }
}

/// Writes every texel of a texture, rows packed from the top, converted to
/// the format's number type.
///
/// # Examples
/// Write-GuiGpuTexture $tex $pixels
/// Write-GuiGpuTexture $tex ([byte[]](255, 0, 0, 255))
#[cmdlet(verb = "Write", noun = "GuiGpuTexture")]
#[derive(Default)]
pub struct WriteGuiGpuTexture {
    /// The texture to write.
    #[param(mandatory, position = 0)]
    pub texture: Option<PsProxy<GpuTexture>>,
    /// Every texel, rows packed from the top, as the format's number type.
    #[param(mandatory, position = 1, raw)]
    pub data: Option<PsObject>,
}

impl Cmdlet for WriteGuiGpuTexture {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let (Some(proxy), Some(data)) = (&self.texture, &self.data) else {
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuNoData", "a texture and data are both needed"));
        };
        let (session, id, format) = proxy.with(|t| -> PsResult<_> { Ok((t.session.clone(), t.live()?, t.format)) })??;
        let (_, texels) = wgpu_pwsh::host_bytes(data, Some(format.element()))?;
        listen(ps, &session)?;
        let mirrored = mirroring(TableObject::Texture(id));
        let sent = mirrored.as_ref().map(|_| texels.clone());
        match call(ps, &session, GpuOp::WriteTexture { texture: id, texels })? {
            GpuValue::Done => {}
            other => return Err(unexpected("a texture's write", &other)),
        }
        // A page drawing from the texture takes its new texels too.
        if let (Some((page, Mirrored::Texture { width, height, format, .. })), Some(texels)) = (mirrored, sent) {
            mirror_through(&page, GpuOp::MirrorTexture { texture: id, width, height, format, texels }, &|s, op| call(ps, s, op))?;
        }
        Ok(())
    }
}

/// Takes a texture out of the renderer now, rather than when its object is
/// collected, and frees its memory; a GpuSurface still drawing with it
/// uses the memory until its draws change or its window closes.
///
/// # Examples
/// $tex | Remove-GuiGpuTexture
#[cmdlet(verb = "Remove", noun = "GuiGpuTexture", supports_should_process)]
#[derive(Default)]
pub struct RemoveGuiGpuTexture {
    /// The texture to free.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub texture: Option<PsProxy<GpuTexture>>,
}

impl Cmdlet for RemoveGuiGpuTexture {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(proxy) = &self.texture else {
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuNoTexture", "no texture was given"));
        };
        let (session, id, label, removed) = proxy.with(|t| (t.session.clone(), t.id, t.label.clone(), t.removed.load(Ordering::Acquire)))?;
        if removed || !ps.should_process(&label, "Free the texture's GPU memory")? {
            return Ok(());
        }
        listen(ps, &session)?;
        match call(ps, &session, GpuOp::RemoveTexture { texture: id })? {
            GpuValue::Done => proxy.with(|t| t.removed.store(true, Ordering::Release)),
            other => Err(unexpected("a texture's removal", &other)),
        }
    }
}

/// Creates a sampler in the egui renderer: how a shader filters and wraps
/// when it samples a texture, the same in every direction and when
/// magnifying and minifying alike.
///
/// # Examples
/// $linear = New-GuiGpuSampler -Filter Linear
/// New-GuiGpuSampler -AddressMode Repeat -Label tiles
#[cmdlet(verb = "New", noun = "GuiGpuSampler", output = ["PwrsGui.GpuSampler"])]
#[derive(Default)]
pub struct NewGuiGpuSampler {
    /// How it picks between texels; Nearest when omitted.
    #[param]
    pub filter: Option<GuiFilterMode>,
    /// What it reads past a texture's edges; ClampToEdge when omitted.
    #[param]
    pub address_mode: Option<GuiAddressMode>,
    /// The sampler's label.
    #[param]
    pub label: Option<String>,
}

impl Cmdlet for NewGuiGpuSampler {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let filter = self.filter.unwrap_or(GuiFilterMode::Nearest);
        let address = self.address_mode.unwrap_or(GuiAddressMode::ClampToEdge);
        let session = session::egui_session(ps)?;
        listen(ps, &session)?;
        let sampler = match call(ps, &session, GpuOp::NewSampler { filter: filter.wire(), address: address.wire(), label: self.label.clone() })? {
            GpuValue::Sampler { sampler } => sampler,
            other => return Err(unexpected("a new sampler", &other)),
        };
        let label = match &self.label {
            Some(l) => l.clone(),
            None => format!("sampler {sampler}"),
        };
        let adapter = session.hello().map(|h| h.capabilities.adapter.clone());
        ps.write(GpuSampler { label, filter, address_mode: address, adapter, id: sampler, session })
    }
}

/// Destroys the egui renderer's GPU device, as a driver reset loses one, and
/// returns once the renderer goes on with a new device.
///
/// The renderer's windows open again where they were, and its GPU buffers
/// and textures have again what the session wrote into them; what a shader
/// wrote into one is gone. Every runspace that has used the GPU gets the
/// PwrsGui.DeviceLost engine event, whose MessageData's Reason says why the
/// device was lost, as it does when a driver loses the device.
///
/// # Examples
/// Reset-GuiGpuDevice
/// Register-EngineEvent PwrsGui.DeviceLost -Action { Write-Host "GPU device lost: $($Event.MessageData.Reason)" }
#[cmdlet(verb = "Reset", noun = "GuiGpuDevice", supports_should_process)]
#[derive(Default)]
pub struct ResetGuiGpuDevice {}

impl Cmdlet for ResetGuiGpuDevice {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let session = session::egui_session(ps)?;
        listen(ps, &session)?;
        if !ps.should_process("the egui renderer's GPU device", "Destroy the device")? {
            return Ok(());
        }
        match call(ps, &session, GpuOp::ResetDevice)? {
            GpuValue::Lost { .. } => Ok(()),
            other => Err(unexpected("a reset", &other)),
        }
    }
}

/// What Invoke-GuiGpuCompute -PassThru writes: every array the shader may
/// have written, by binding name, and how long the run took.
#[psclass(name = "PwrsGui.ComputeResult", columns = ["Outputs", "Workgroups", "GpuTimeNs", "CpuTimeMs"])]
#[derive(Default, Clone)]
pub struct GpuComputeResult {
    /// A hashtable of each array the shader may have written, by binding name.
    pub outputs: PsObject,
    /// The workgroups dispatched in x, y and z; 0 for an indirect dispatch.
    pub workgroups: Vec<u32>,
    /// Nanoseconds the GPU spent in the pass, where the device has timestamp queries and -PassThru is given.
    pub gpu_time_ns: Option<f64>,
    /// Milliseconds the run took in the renderer, binding and copying included.
    pub cpu_time_ms: f64,
}

fn is_dictionary(type_name: &str) -> bool {
    matches!(type_name, "System.Collections.Hashtable" | "System.Collections.Specialized.OrderedDictionary")
}

/// Whether `type_name` is the type of an object of the renderer's table: a
/// buffer, a texture or a sampler.
fn is_table(type_name: &str) -> bool {
    matches!(type_name, "PwrsGui.GpuBuffer" | "PwrsGui.GpuTexture" | "PwrsGui.GpuSampler")
}

/// Whether `type_name` is an array whose items are resources, one each.
fn is_list(type_name: &str) -> bool {
    matches!(type_name, "System.Object[]" | "PwrsGui.GpuBuffer[]" | "PwrsGui.GpuTexture[]" | "PwrsGui.GpuSampler[]")
}

fn is_number(type_name: &str) -> bool {
    matches!(
        type_name,
        "System.Single" | "System.Double" | "System.Decimal" | "System.SByte" | "System.Byte" | "System.Int16" | "System.UInt16" | "System.Int32" | "System.UInt32" | "System.Int64" | "System.UInt64"
    )
}

fn binding_list(declared: &[wgpu_pwsh::BindingInfo]) -> String {
    declared.iter().map(|b| format!("{} ({}:{})", b.name, b.group, b.binding)).collect::<Vec<_>>().join(", ")
}

fn binding_error(id: &str, message: String) -> PsError {
    PsError::new(ErrorCategory::InvalidArgument, id, message)
}

/// Override constant values, by name or `@id`, from a hashtable of numbers.
fn constants_of(table: Option<&PsHashtable>) -> PsResult<Vec<(String, f64)>> {
    let mut constants = Vec::new();
    if let Some(table) = table {
        for key in table.keys()? {
            let value = f64::from_ps(&table.get(&key)?)?;
            constants.push((key, value));
        }
    }
    Ok(constants)
}

/// The binding a hashtable key names: `group:binding`, or its variable's
/// name, ignoring case.
fn find<'a>(declared: &'a [wgpu_pwsh::BindingInfo], key: &str) -> PsResult<&'a wgpu_pwsh::BindingInfo> {
    if let Some((g, b)) = key.split_once(':') {
        let number = |part: &str| part.trim().parse::<u32>().map_err(|e| binding_error("GuiGpuBinding", format!("{key} is neither a binding name nor group:binding: {e}")));
        let (g, b) = (number(g)?, number(b)?);
        return declared.iter().find(|d| d.group == g && d.binding == b).ok_or_else(|| binding_error("GuiGpuBinding", format!("the shader uses no binding {g}:{b}; it uses {}", binding_list(declared))));
    }
    declared
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case(key))
        .ok_or_else(|| binding_error("GuiGpuBinding", format!("the shader uses no binding named {key}; it uses {}", binding_list(declared))))
}

/// Matches `resources` to the shader's bindings as Invoke-WgpuCompute
/// does: a hashtable keyed by binding name or `group:binding`; an array of
/// resources in the order the shader declares its bindings; or, for a
/// shader with one binding, its resource itself. Returns each binding and
/// its resource, by group and then binding.
fn match_resources<'a>(declared: &'a [wgpu_pwsh::BindingInfo], resources: Option<&PsObject>) -> PsResult<Vec<(&'a wgpu_pwsh::BindingInfo, PsObject)>> {
    let resources = match resources {
        Some(r) if !r.is_null() => r,
        _ => return Ok(Vec::new()),
    };
    let type_name = resources.type_name()?;
    if is_dictionary(&type_name) {
        let table = PsHashtable(resources.clone());
        let mut out = Vec::new();
        for key in table.keys()? {
            out.push((find(declared, &key)?, table.get(&key)?));
        }
        out.sort_by_key(|(b, _)| (b.group, b.binding));
        return Ok(out);
    }
    let items = if is_list(&type_name) {
        let items = Vec::<PsObject>::from_ps(resources)?;
        let numbers = match items.first() {
            Some(first) => is_number(&first.type_name()?),
            None => false,
        };
        if numbers { vec![resources.clone()] } else { items }
    } else {
        vec![resources.clone()]
    };
    if items.len() != declared.len() {
        let uses = match declared.len() {
            1 => String::from("1 binding"),
            n => format!("{n} bindings"),
        };
        let given = match items.len() {
            1 => String::from("1 resource was given"),
            n => format!("{n} resources were given"),
        };
        let message = format!("the shader uses {uses} ({}) and {given}; give one per binding in that order, or a hashtable by name", binding_list(declared));
        return Err(binding_error("GuiGpuBindingCount", message));
    }
    Ok(declared.iter().zip(items).collect())
}

/// The table buffer in `value`, which must live in `session`.
fn table_buffer(value: &PsObject, session: &Session) -> PsResult<u64> {
    buffer_in(&PsProxy::<GpuBuffer>::from_ps(value)?, session)
}

/// The renderer's number for `proxy`'s buffer, which must live in
/// `session`.
fn buffer_in(proxy: &PsProxy<GpuBuffer>, session: &Session) -> PsResult<u64> {
    proxy.with(|b| {
        if !b.session.same(session) {
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuOtherSession", format!("the GPU buffer '{}' lives in a renderer that has since ended", b.label)));
        }
        b.live()
    })?
}

/// The renderer's number for `proxy`'s texture, which must live in
/// `session`.
fn texture_in(proxy: &PsProxy<GpuTexture>, session: &Session) -> PsResult<u64> {
    proxy.with(|t| {
        if !t.session.same(session) {
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuOtherSession", format!("the GPU texture '{}' lives in a renderer that has since ended", t.label)));
        }
        t.live()
    })?
}

/// The renderer's number for `proxy`'s sampler, which must live in
/// `session`.
fn sampler_in(proxy: &PsProxy<GpuSampler>, session: &Session) -> PsResult<u64> {
    proxy.with(|s| {
        if !s.session.same(session) {
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuOtherSession", format!("the GPU sampler '{}' lives in a renderer that has since ended", s.label)));
        }
        Ok(s.id)
    })?
}

/// The resource `value`, of type `type_name`, gives as an object of the
/// renderer's table, which must live in `session`: a buffer, a texture or
/// a sampler; `None` for a value of any other type.
fn table_resource(value: &PsObject, type_name: &str, session: &Session) -> PsResult<Option<GpuResource>> {
    Ok(match type_name {
        "PwrsGui.GpuBuffer" => Some(GpuResource::Buffer(table_buffer(value, session)?)),
        "PwrsGui.GpuTexture" => Some(GpuResource::Texture(texture_in(&PsProxy::<GpuTexture>::from_ps(value)?, session)?)),
        "PwrsGui.GpuSampler" => Some(GpuResource::Sampler(sampler_in(&PsProxy::<GpuSampler>::from_ps(value)?, session)?)),
        _ => None,
    })
}

/// The resource `value` gives `binding`: a table buffer, texture or
/// sampler, or an array copied to the GPU as the binding's declared type,
/// converted as the Wgpu module converts one.
fn resource(binding: &wgpu_pwsh::BindingInfo, value: &PsObject, session: &Session) -> PsResult<GpuResource> {
    let type_name = value.type_name()?;
    if let Some(table) = table_resource(value, &type_name, session)? {
        return Ok(table);
    }
    if is_dictionary(&type_name) {
        return Err(binding_error("GuiGpuStruct", format!("{} takes an array, a GpuBuffer, a GpuTexture or a GpuSampler; pack a struct's members into a buffer to bind them", binding.name)));
    }
    let (element, bytes) = wgpu_pwsh::host_bytes(value, binding.element_type)?;
    if bytes.is_empty() {
        return Err(binding_error("GuiGpuEmpty", format!("the array for {} is empty", binding.name)));
    }
    Ok(GpuResource::Array { element: wire(element), bytes })
}

/// Runs a compute shader once in the egui renderer over arrays or GPU
/// buffers and writes the arrays it may have written, as the Wgpu module's
/// Invoke-WgpuCompute does.
///
/// The shader is WGSL or GLSL text. -Binding gives its resources: an array
/// in the order the shader declares its bindings, a hashtable by binding
/// name or "group:binding", or for one binding the resource itself. An
/// array of numbers is copied to the GPU as the element type the shader
/// declares, and each array bound read_write comes back as an array of
/// that type; a PwrsGui.GpuBuffer stays on the GPU, and a texture or
/// sampler binding takes a PwrsGui.GpuTexture or GpuSampler. Without
/// -Workgroups, -Invocations or -Indirect the run has one invocation per
/// element of the first buffer.
///
/// # Examples
/// Invoke-GuiGpuCompute $double ([float[]](1..1024))
/// Invoke-GuiGpuCompute $add @{ a = $a; b = [float[]](1..10) } -Invocations 10
/// Invoke-GuiGpuCompute $wgsl -Binding $buf -Workgroups 64, 64
/// Invoke-GuiGpuCompute $wgsl $data -PassThru | Select-Object GpuTimeNs
#[cmdlet(verb = "Invoke", noun = "GuiGpuCompute", output = ["System.Array", "PwrsGui.ComputeResult"])]
#[derive(Default)]
pub struct InvokeGuiGpuCompute {
    /// WGSL or GLSL text.
    #[param(mandatory, position = 0)]
    pub shader: Option<String>,
    /// The resources for the shader's bindings.
    #[param(position = 1, raw, alias = ["Data", "Buffer", "Resource"])]
    pub binding: Option<PsObject>,
    /// Workgroups to dispatch in x, y and z (1 for a dimension left out).
    #[param]
    pub workgroups: Option<Vec<u32>>,
    /// Invocations in x, y and z, divided by the shader's workgroup size and rounded up.
    #[param]
    pub invocations: Option<Vec<u64>>,
    /// Read the workgroup counts from this GPU buffer (three uint).
    #[param]
    pub indirect: Option<PsProxy<GpuBuffer>>,
    /// The byte offset of the counts in -Indirect.
    #[param]
    pub indirect_offset: Option<u64>,
    /// The entry point; the shader's only compute entry point when omitted.
    #[param]
    pub entry: Option<String>,
    /// Override constant values, by name or `@id`; a GLSL shader's specialization constants by constant_id.
    #[param]
    pub constant: Option<PsHashtable>,
    /// Immediate data, as an array of numbers.
    #[param(raw)]
    pub immediate: Option<PsObject>,
    /// The language of the shader text; GLSL when it starts with #version, WGSL otherwise, when omitted.
    #[param]
    pub language: Option<GuiShaderLanguage>,
    /// Write a PwrsGui.ComputeResult with every output and the time taken, measured on the GPU where it can be.
    #[param]
    pub pass_thru: bool,
}

impl InvokeGuiGpuCompute {
    fn dispatch(&self, session: &Session) -> PsResult<GpuDispatch> {
        let three = |v: &[u64], what: &str| -> PsResult<[u64; 3]> {
            match v {
                [x] => Ok([*x, 1, 1]),
                [x, y] => Ok([*x, *y, 1]),
                [x, y, z] => Ok([*x, *y, *z]),
                _ => Err(binding_error("GuiGpuDispatchSize", format!("{what} takes one to three counts, x, y and z"))),
            }
        };
        Ok(match (&self.workgroups, &self.invocations, &self.indirect) {
            (Some(w), None, None) => {
                let w: Vec<u64> = w.iter().map(|v| u64::from(*v)).collect();
                let [x, y, z] = three(&w, "-Workgroups")?;
                GpuDispatch::Workgroups([x as u32, y as u32, z as u32])
            }
            (None, Some(n), None) => GpuDispatch::Invocations(three(n, "-Invocations")?),
            (None, None, Some(buffer)) => GpuDispatch::Indirect { buffer: buffer_in(buffer, session)?, offset: self.indirect_offset.unwrap_or(0) },
            (None, None, None) => GpuDispatch::Auto,
            _ => return Err(binding_error("GuiGpuDispatchSize", String::from("give one of -Workgroups, -Invocations and -Indirect"))),
        })
    }
}

/// A compute shader as the renderer runs it: its text, language, entry
/// point and constants, its bindings and its dispatch, and the objects of
/// the renderer's table it may write.
struct ComputeParts {
    source: String,
    language: ShaderLanguage,
    entry: Option<String>,
    constants: Vec<(String, f64)>,
    bindings: Vec<(GpuSlot, GpuResource)>,
    dispatch: GpuDispatch,
    written: Vec<TableObject>,
}

/// The bindings, by group and binding, at which the WGSL compute shader
/// `source` declares a storage texture it may store into, or why naga's
/// WGSL front end cannot read it.
fn storage_writes(source: &str) -> PsResult<Vec<(u32, u32)>> {
    let module = naga::front::wgsl::parse_str(source)
        .map_err(|e| binding_error("WgpuShaderInvalid", format!("the shader could not be read for the storage textures it writes, which a page drawing from them takes again: {e}")))?;
    let stores = |ty: naga::Handle<naga::Type>| matches!(module.types[ty].inner, naga::TypeInner::Image { class: naga::ImageClass::Storage { access, .. }, .. } if access.contains(naga::StorageAccess::STORE));
    Ok(module.global_variables.iter().filter(|(_, v)| stores(v.ty)).filter_map(|(_, v)| v.binding.as_ref().map(|b| (b.group, b.binding))).collect())
}

impl InvokeGuiGpuCompute {
    /// The shader's text, its language, given or read from the text, and
    /// the bindings it declares, which need no renderer.
    fn declared(&self) -> PsResult<(&str, GuiShaderLanguage, Vec<wgpu_pwsh::BindingInfo>)> {
        let Some(source) = &self.shader else {
            return Err(binding_error("GuiGpuNoShader", String::from("no shader was given")));
        };
        let language = match self.language {
            Some(l) => l,
            None if source.trim_start().starts_with("#version") => GuiShaderLanguage::Glsl,
            None => GuiShaderLanguage::Wgsl,
        };
        let declared = match language {
            GuiShaderLanguage::Wgsl => wgpu_pwsh::compute_binding_info(source, self.entry.as_deref())?,
            GuiShaderLanguage::Glsl => {
                if let Some(entry) = &self.entry
                    && entry != "main"
                {
                    return Err(binding_error("GuiGpuEntry", format!("a GLSL compute shader's entry point is main, and {entry} was given")));
                }
                wgpu_pwsh::compute_binding_info_glsl(source)?
            }
        };
        Ok((source, language, declared))
    }

    /// The run these parameters give, as the renderer takes it, for the
    /// shader `source` in `language` declaring `declared`: its buffers,
    /// textures and samplers must live in `session`, and each it binds is
    /// put in `held`. It may write the buffers it binds read_write, and of
    /// the textures an HTML page draws from, those a WGSL shader declares
    /// storage it may store into, or any a GLSL shader binds, which the
    /// module does not read.
    fn parts(&self, source: &str, language: GuiShaderLanguage, declared: &[wgpu_pwsh::BindingInfo], session: &Session, held: &mut Vec<PsObject>) -> PsResult<ComputeParts> {
        let mut stores: Option<Vec<(u32, u32)>> = None;
        let mut bindings = Vec::new();
        let mut written = Vec::new();
        for (info, value) in match_resources(declared, self.binding.as_ref())? {
            if is_table(&value.type_name()?) {
                held.push(value.clone());
            }
            let bound = resource(info, &value, session)?;
            match &bound {
                GpuResource::Buffer(id) if info.writable => written.push(TableObject::Buffer(*id)),
                GpuResource::Texture(id) if mirroring(TableObject::Texture(*id)).is_some() => {
                    let writes = match language {
                        GuiShaderLanguage::Glsl => true,
                        GuiShaderLanguage::Wgsl => {
                            if stores.is_none() {
                                stores = Some(storage_writes(source)?);
                            }
                            stores.as_ref().is_some_and(|s| s.contains(&(info.group, info.binding)))
                        }
                    };
                    if writes {
                        written.push(TableObject::Texture(*id));
                    }
                }
                _ => {}
            }
            bindings.push((GpuSlot::Name(format!("{}:{}", info.group, info.binding)), bound));
        }
        Ok(ComputeParts {
            source: source.to_string(),
            language: match language {
                GuiShaderLanguage::Wgsl => ShaderLanguage::Wgsl,
                GuiShaderLanguage::Glsl => ShaderLanguage::Glsl,
            },
            entry: self.entry.clone(),
            constants: constants_of(self.constant.as_ref())?,
            bindings,
            dispatch: self.dispatch(session)?,
            written,
        })
    }
}

impl Cmdlet for InvokeGuiGpuCompute {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let (source, language, declared) = self.declared()?;
        let session = session::egui_session(ps)?;
        listen(ps, &session)?;
        let parts = self.parts(source, language, &declared, &session, &mut Vec::new())?;
        let immediates = match &self.immediate {
            Some(data) => Some(wgpu_pwsh::host_bytes(data, None)?.1),
            None => None,
        };
        let op = GpuOp::Compute {
            source: parts.source,
            language: parts.language,
            entry: parts.entry,
            constants: parts.constants,
            bindings: parts.bindings,
            dispatch: parts.dispatch,
            immediates,
            timed: self.pass_thru,
        };
        let report = match call(ps, &session, op)? {
            GpuValue::Computed(report) => report,
            other => return Err(unexpected("a compute run", &other)),
        };
        // A page drawing from what the run may have written takes it again.
        for object in parts.written {
            send_again(ps, &session, object)?;
        }
        if self.pass_thru {
            let outputs = PsHashtable::new()?;
            for w in &report.written {
                outputs.set(&w.binding, wgpu_pwsh::to_clr_array(unwire(w.element), &w.bytes)?)?;
            }
            return ps.write(GpuComputeResult { outputs: outputs.0, workgroups: report.workgroups.to_vec(), gpu_time_ns: report.gpu_time_ns, cpu_time_ms: report.cpu_time_ms });
        }
        for w in &report.written {
            ps.write_object(&wgpu_pwsh::to_clr_array(unwire(w.element), &w.bytes)?)?;
        }
        Ok(())
    }
}

/// How vertices make primitives.
#[psenum(name = "PwrsGui.PrimitiveTopology")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiPrimitiveTopology {
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

const TOPOLOGIES: [(GuiPrimitiveTopology, &str); 5] = [
    (GuiPrimitiveTopology::PointList, "PointList"),
    (GuiPrimitiveTopology::LineList, "LineList"),
    (GuiPrimitiveTopology::LineStrip, "LineStrip"),
    (GuiPrimitiveTopology::TriangleList, "TriangleList"),
    (GuiPrimitiveTopology::TriangleStrip, "TriangleStrip"),
];

impl GuiPrimitiveTopology {
    fn wire(self) -> Topology {
        match self {
            GuiPrimitiveTopology::PointList => Topology::PointList,
            GuiPrimitiveTopology::LineList => Topology::LineList,
            GuiPrimitiveTopology::LineStrip => Topology::LineStrip,
            GuiPrimitiveTopology::TriangleList => Topology::TriangleList,
            GuiPrimitiveTopology::TriangleStrip => Topology::TriangleStrip,
        }
    }
}

/// Which faces a render pipeline discards. A triangle whose corners turn
/// counterclockwise on the surface faces the viewer.
#[psenum(name = "PwrsGui.CullMode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiCullMode {
    /// Draws both faces.
    #[default]
    None,
    /// Discards faces wound counterclockwise, which face the viewer.
    Front,
    /// Discards faces wound clockwise, which face away.
    Back,
}

const CULL_MODES: [(GuiCullMode, &str); 3] = [(GuiCullMode::None, "None"), (GuiCullMode::Front, "Front"), (GuiCullMode::Back, "Back")];

impl GuiCullMode {
    fn wire(self) -> Cull {
        match self {
            GuiCullMode::None => Cull::None,
            GuiCullMode::Front => Cull::Front,
            GuiCullMode::Back => Cull::Back,
        }
    }
}

/// How a fragment's color combines with the color under it.
#[psenum(name = "PwrsGui.BlendMode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiBlendMode {
    /// The fragment replaces the color.
    #[default]
    Replace,
    /// Alpha blending: the fragment over the color by its alpha.
    Alpha,
    /// The fragment adds to the color.
    Additive,
    /// Alpha blending of a fragment whose color is multiplied by its alpha.
    Premultiplied,
    /// The fragment and the color mixed by the blend constant a draw's
    /// -BlendConstant sets: the fragment times the constant plus the color
    /// times one minus it.
    Constant,
    /// The factors and operations a custom blend names: -BlendSource,
    /// -BlendDestination and -BlendOperation, and their alpha forms.
    Custom,
}

const BLEND_MODES: [(GuiBlendMode, &str); 6] = [
    (GuiBlendMode::Replace, "Replace"),
    (GuiBlendMode::Alpha, "Alpha"),
    (GuiBlendMode::Additive, "Additive"),
    (GuiBlendMode::Premultiplied, "Premultiplied"),
    (GuiBlendMode::Constant, "Constant"),
    (GuiBlendMode::Custom, "Custom"),
];

impl GuiBlendMode {
    fn wire(self) -> Blend {
        match self {
            GuiBlendMode::Replace => Blend::Replace,
            GuiBlendMode::Alpha => Blend::Alpha,
            GuiBlendMode::Additive => Blend::Additive,
            GuiBlendMode::Premultiplied => Blend::Premultiplied,
            GuiBlendMode::Constant => Blend::Constant,
            GuiBlendMode::Custom => Blend::Custom,
        }
    }
}

/// What a blend multiplies the fragment's color (source) or the color
/// under it (destination) by.
#[psenum(name = "PwrsGui.BlendFactor")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiBlendFactor {
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
    /// The blend constant a draw's -BlendConstant sets.
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

const BLEND_FACTORS: [(GuiBlendFactor, &str); 17] = [
    (GuiBlendFactor::Zero, "Zero"),
    (GuiBlendFactor::One, "One"),
    (GuiBlendFactor::Src, "Src"),
    (GuiBlendFactor::OneMinusSrc, "OneMinusSrc"),
    (GuiBlendFactor::SrcAlpha, "SrcAlpha"),
    (GuiBlendFactor::OneMinusSrcAlpha, "OneMinusSrcAlpha"),
    (GuiBlendFactor::Dst, "Dst"),
    (GuiBlendFactor::OneMinusDst, "OneMinusDst"),
    (GuiBlendFactor::DstAlpha, "DstAlpha"),
    (GuiBlendFactor::OneMinusDstAlpha, "OneMinusDstAlpha"),
    (GuiBlendFactor::SrcAlphaSaturated, "SrcAlphaSaturated"),
    (GuiBlendFactor::Constant, "Constant"),
    (GuiBlendFactor::OneMinusConstant, "OneMinusConstant"),
    (GuiBlendFactor::Src1, "Src1"),
    (GuiBlendFactor::OneMinusSrc1, "OneMinusSrc1"),
    (GuiBlendFactor::Src1Alpha, "Src1Alpha"),
    (GuiBlendFactor::OneMinusSrc1Alpha, "OneMinusSrc1Alpha"),
];

impl GuiBlendFactor {
    fn wire(self) -> WireFactor {
        match self {
            GuiBlendFactor::Zero => WireFactor::Zero,
            GuiBlendFactor::One => WireFactor::One,
            GuiBlendFactor::Src => WireFactor::Src,
            GuiBlendFactor::OneMinusSrc => WireFactor::OneMinusSrc,
            GuiBlendFactor::SrcAlpha => WireFactor::SrcAlpha,
            GuiBlendFactor::OneMinusSrcAlpha => WireFactor::OneMinusSrcAlpha,
            GuiBlendFactor::Dst => WireFactor::Dst,
            GuiBlendFactor::OneMinusDst => WireFactor::OneMinusDst,
            GuiBlendFactor::DstAlpha => WireFactor::DstAlpha,
            GuiBlendFactor::OneMinusDstAlpha => WireFactor::OneMinusDstAlpha,
            GuiBlendFactor::SrcAlphaSaturated => WireFactor::SrcAlphaSaturated,
            GuiBlendFactor::Constant => WireFactor::Constant,
            GuiBlendFactor::OneMinusConstant => WireFactor::OneMinusConstant,
            GuiBlendFactor::Src1 => WireFactor::Src1,
            GuiBlendFactor::OneMinusSrc1 => WireFactor::OneMinusSrc1,
            GuiBlendFactor::Src1Alpha => WireFactor::Src1Alpha,
            GuiBlendFactor::OneMinusSrc1Alpha => WireFactor::OneMinusSrc1Alpha,
        }
    }
}

/// How a blend combines the fragment's weighted color with the weighted
/// color under it.
#[psenum(name = "PwrsGui.BlendOperation")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiBlendOperation {
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

const BLEND_OPERATIONS: [(GuiBlendOperation, &str); 5] = [
    (GuiBlendOperation::Add, "Add"),
    (GuiBlendOperation::Subtract, "Subtract"),
    (GuiBlendOperation::ReverseSubtract, "ReverseSubtract"),
    (GuiBlendOperation::Min, "Min"),
    (GuiBlendOperation::Max, "Max"),
];

impl GuiBlendOperation {
    fn wire(self) -> WireOperation {
        match self {
            GuiBlendOperation::Add => WireOperation::Add,
            GuiBlendOperation::Subtract => WireOperation::Subtract,
            GuiBlendOperation::ReverseSubtract => WireOperation::ReverseSubtract,
            GuiBlendOperation::Min => WireOperation::Min,
            GuiBlendOperation::Max => WireOperation::Max,
        }
    }
}

/// The format of a render pipeline's depth attachment, which holds depth,
/// stencil or both.
#[psenum(name = "PwrsGui.DepthFormat")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiDepthFormat {
    /// 8-bit stencil values, and no depth.
    Stencil8,
    /// 16-bit depth, and no stencil.
    Depth16Unorm,
    /// Depth of 24 bits or more, and no stencil.
    Depth24Plus,
    /// Depth of 24 bits or more, and 8-bit stencil values.
    Depth24PlusStencil8,
    /// 32-bit float depth, and no stencil.
    #[default]
    Depth32Float,
    /// 32-bit float depth and 8-bit stencil values; needs the adapter's Depth32FloatStencil8 feature.
    Depth32FloatStencil8,
}

const DEPTH_FORMATS: [(GuiDepthFormat, &str); 6] = [
    (GuiDepthFormat::Stencil8, "Stencil8"),
    (GuiDepthFormat::Depth16Unorm, "Depth16Unorm"),
    (GuiDepthFormat::Depth24Plus, "Depth24Plus"),
    (GuiDepthFormat::Depth24PlusStencil8, "Depth24PlusStencil8"),
    (GuiDepthFormat::Depth32Float, "Depth32Float"),
    (GuiDepthFormat::Depth32FloatStencil8, "Depth32FloatStencil8"),
];

impl GuiDepthFormat {
    fn wire(self) -> WireDepthFormat {
        match self {
            GuiDepthFormat::Stencil8 => WireDepthFormat::Stencil8,
            GuiDepthFormat::Depth16Unorm => WireDepthFormat::Depth16Unorm,
            GuiDepthFormat::Depth24Plus => WireDepthFormat::Depth24Plus,
            GuiDepthFormat::Depth24PlusStencil8 => WireDepthFormat::Depth24PlusStencil8,
            GuiDepthFormat::Depth32Float => WireDepthFormat::Depth32Float,
            GuiDepthFormat::Depth32FloatStencil8 => WireDepthFormat::Depth32FloatStencil8,
        }
    }
}

/// How a depth or a stencil test compares two values: a fragment's depth
/// with the depth under it, or the stencil reference with the stencil value
/// under a fragment, the new value on the left.
#[psenum(name = "PwrsGui.CompareFunction")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiCompareFunction {
    /// Never passes.
    Never,
    /// Passes when the new value is less.
    #[default]
    Less,
    /// Passes when the values are equal.
    Equal,
    /// Passes when the new value is less or equal.
    LessEqual,
    /// Passes when the new value is greater.
    Greater,
    /// Passes when the values differ.
    NotEqual,
    /// Passes when the new value is greater or equal.
    GreaterEqual,
    /// Always passes.
    Always,
}

const COMPARE_FUNCTIONS: [(GuiCompareFunction, &str); 8] = [
    (GuiCompareFunction::Never, "Never"),
    (GuiCompareFunction::Less, "Less"),
    (GuiCompareFunction::Equal, "Equal"),
    (GuiCompareFunction::LessEqual, "LessEqual"),
    (GuiCompareFunction::Greater, "Greater"),
    (GuiCompareFunction::NotEqual, "NotEqual"),
    (GuiCompareFunction::GreaterEqual, "GreaterEqual"),
    (GuiCompareFunction::Always, "Always"),
];

impl GuiCompareFunction {
    fn wire(self) -> WireCompare {
        match self {
            GuiCompareFunction::Never => WireCompare::Never,
            GuiCompareFunction::Less => WireCompare::Less,
            GuiCompareFunction::Equal => WireCompare::Equal,
            GuiCompareFunction::LessEqual => WireCompare::LessEqual,
            GuiCompareFunction::Greater => WireCompare::Greater,
            GuiCompareFunction::NotEqual => WireCompare::NotEqual,
            GuiCompareFunction::GreaterEqual => WireCompare::GreaterEqual,
            GuiCompareFunction::Always => WireCompare::Always,
        }
    }
}

/// What a stencil test does to the stencil value under a fragment.
#[psenum(name = "PwrsGui.StencilOperation")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiStencilOperation {
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

const STENCIL_OPERATIONS: [(GuiStencilOperation, &str); 8] = [
    (GuiStencilOperation::Keep, "Keep"),
    (GuiStencilOperation::Zero, "Zero"),
    (GuiStencilOperation::Replace, "Replace"),
    (GuiStencilOperation::Invert, "Invert"),
    (GuiStencilOperation::IncrementClamp, "IncrementClamp"),
    (GuiStencilOperation::DecrementClamp, "DecrementClamp"),
    (GuiStencilOperation::IncrementWrap, "IncrementWrap"),
    (GuiStencilOperation::DecrementWrap, "DecrementWrap"),
];

impl GuiStencilOperation {
    fn wire(self) -> WireStencil {
        match self {
            GuiStencilOperation::Keep => WireStencil::Keep,
            GuiStencilOperation::Zero => WireStencil::Zero,
            GuiStencilOperation::Replace => WireStencil::Replace,
            GuiStencilOperation::Invert => WireStencil::Invert,
            GuiStencilOperation::IncrementClamp => WireStencil::IncrementClamp,
            GuiStencilOperation::DecrementClamp => WireStencil::DecrementClamp,
            GuiStencilOperation::IncrementWrap => WireStencil::IncrementWrap,
            GuiStencilOperation::DecrementWrap => WireStencil::DecrementWrap,
        }
    }
}

/// The one of `names` that `value`, an enum value or its name as text,
/// names, compared without regard to case.
fn named<T: Copy>(value: &PsObject, names: &[(T, &str)]) -> Result<T, String> {
    let given = String::from_ps(value).map_err(|e| e.message)?;
    match names.iter().find(|(_, name)| name.eq_ignore_ascii_case(given.trim())) {
        Some((found, _)) => Ok(*found),
        None => {
            let all: Vec<&str> = names.iter().map(|(_, name)| *name).collect();
            Err(format!("{given} is none of {}", all.join(", ")))
        }
    }
}

/// How a render pipeline draws, as New-WgpuRenderPipeline's parameters say
/// it: New-GuiGpuRenderPipeline's, and those of a draw whose pipeline
/// Set-GuiGpuSurface builds from WGSL text.
#[derive(Clone, Debug, Default, PartialEq)]
struct PipelineOptions {
    depth: bool,
    depth_format: Option<GuiDepthFormat>,
    depth_compare: Option<GuiCompareFunction>,
    depth_write: Option<bool>,
    depth_bias: Option<i32>,
    depth_bias_slope_scale: Option<f64>,
    depth_bias_clamp: Option<f64>,
    stencil_compare: Option<GuiCompareFunction>,
    stencil_fail: Option<GuiStencilOperation>,
    stencil_depth_fail: Option<GuiStencilOperation>,
    stencil_pass: Option<GuiStencilOperation>,
    stencil_read_mask: Option<u32>,
    stencil_write_mask: Option<u32>,
    back_stencil_compare: Option<GuiCompareFunction>,
    back_stencil_fail: Option<GuiStencilOperation>,
    back_stencil_depth_fail: Option<GuiStencilOperation>,
    back_stencil_pass: Option<GuiStencilOperation>,
    topology: Option<GuiPrimitiveTopology>,
    cull_mode: Option<GuiCullMode>,
    blend: Option<GuiBlendMode>,
    blend_source: Option<GuiBlendFactor>,
    blend_destination: Option<GuiBlendFactor>,
    blend_operation: Option<GuiBlendOperation>,
    alpha_blend_source: Option<GuiBlendFactor>,
    alpha_blend_destination: Option<GuiBlendFactor>,
    alpha_blend_operation: Option<GuiBlendOperation>,
    sample_count: Option<u32>,
    vertex_entry: Option<String>,
    fragment_entry: Option<String>,
    constants: Vec<(String, f64)>,
}

impl PipelineOptions {
    /// Whether any was given, which a draw through a pipeline object
    /// refuses, since the object draws as it was built.
    fn any(&self) -> bool {
        *self != PipelineOptions::default()
    }

    /// The pipeline of WGSL `source` drawn as these say.
    fn spec(&self, source: String) -> PipelineSpec {
        PipelineSpec {
            source,
            vertex_entry: self.vertex_entry.clone(),
            fragment_entry: self.fragment_entry.clone(),
            topology: self.topology.unwrap_or_default().wire(),
            cull: self.cull_mode.unwrap_or_default().wire(),
            blend: self.blend.unwrap_or_default().wire(),
            custom_blend: CustomBlend {
                source: self.blend_source.map(GuiBlendFactor::wire),
                destination: self.blend_destination.map(GuiBlendFactor::wire),
                operation: self.blend_operation.map(GuiBlendOperation::wire),
                alpha_source: self.alpha_blend_source.map(GuiBlendFactor::wire),
                alpha_destination: self.alpha_blend_destination.map(GuiBlendFactor::wire),
                alpha_operation: self.alpha_blend_operation.map(GuiBlendOperation::wire),
            },
            depth: self.depth,
            depth_format: self.depth_format.map(GuiDepthFormat::wire),
            depth_test: WireDepthTest {
                compare: self.depth_compare.map(GuiCompareFunction::wire),
                write: self.depth_write,
                bias: self.depth_bias,
                bias_slope_scale: self.depth_bias_slope_scale.map(|s| s as f32),
                bias_clamp: self.depth_bias_clamp.map(|c| c as f32),
            },
            stencil: WireStencilTest {
                front: WireFace {
                    compare: self.stencil_compare.map(GuiCompareFunction::wire),
                    fail: self.stencil_fail.map(GuiStencilOperation::wire),
                    depth_fail: self.stencil_depth_fail.map(GuiStencilOperation::wire),
                    pass: self.stencil_pass.map(GuiStencilOperation::wire),
                },
                back: WireFace {
                    compare: self.back_stencil_compare.map(GuiCompareFunction::wire),
                    fail: self.back_stencil_fail.map(GuiStencilOperation::wire),
                    depth_fail: self.back_stencil_depth_fail.map(GuiStencilOperation::wire),
                    pass: self.back_stencil_pass.map(GuiStencilOperation::wire),
                },
                read_mask: self.stencil_read_mask,
                write_mask: self.stencil_write_mask,
            },
            sample_count: self.sample_count.unwrap_or(1),
            constants: self.constants.clone(),
        }
    }
}

/// `source` as a render pipeline's shader, which is WGSL: GLSL has one
/// stage, so it cannot draw.
fn wgsl_of(source: &str) -> PsResult<String> {
    if source.trim_start().starts_with("#version") {
        let text = "a render pipeline's shader is WGSL with a vertex and a fragment entry point; GLSL has one stage, so it cannot draw";
        return Err(binding_error("GuiGpuShaderLanguage", String::from(text)));
    }
    Ok(source.to_string())
}

/// A render pipeline in the egui renderer, drawing into a GpuSurface's
/// texture: a vertex and a fragment entry point with the vertex inputs and
/// bindings they read. The renderer frees it when the object is collected;
/// a GpuSurface drawing with it uses it until its draws change.
#[psclass(name = "PwrsGui.GpuRenderPipeline", mode = proxy, columns = ["Label", "VertexInputs", "Bindings", "SampleCount"])]
pub struct GpuRenderPipeline {
    /// The pipeline's label.
    pub label: String,
    /// The vertex entry point it was given; null when the shader's only one
    /// draws.
    pub vertex_entry_point: Option<String>,
    /// The fragment entry point it was given; null when the shader's only
    /// one draws.
    pub fragment_entry_point: Option<String>,
    /// The vertex inputs, in location order, each fed from a buffer of its
    /// own.
    pub vertex_inputs: Vec<String>,
    /// The bindings both stages use, in group and binding order.
    pub bindings: Vec<String>,
    /// The format of its depth attachment, or null.
    pub depth_format: Option<String>,
    /// Samples per pixel.
    pub sample_count: u32,
    /// The adapter of the renderer's device; null for a renderer another
    /// module started, which this session did not start.
    pub adapter: Option<String>,
    #[psfield(skip)]
    pub(crate) id: u64,
    #[psfield(skip)]
    pub(crate) session: Session,
    /// What it was built from, which an HTML page builds its own from.
    #[psfield(skip)]
    pub(crate) spec: PipelineSpec,
}

impl Drop for GpuRenderPipeline {
    fn drop(&mut self) {
        if !self.session.is_alive() {
            return;
        }
        if let Err(e) = self.session.gpu_free_pipeline(self.id) {
            eprintln!("PWRS-GUI: render pipeline {} ({}) could not be freed as its object was collected: {e}", self.id, self.label);
        }
    }
}

/// The renderer's number for `proxy`'s pipeline, which must live in
/// `session`.
fn pipeline_in(proxy: &PsProxy<GpuRenderPipeline>, session: &Session) -> PsResult<u64> {
    proxy.with(|p| {
        if !p.session.same(session) {
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuOtherSession", format!("the render pipeline '{}' lives in a renderer that has since ended", p.label)));
        }
        Ok(p.id)
    })?
}

/// Builds a render pipeline in the egui renderer from WGSL with a vertex and
/// a fragment entry point, as the Wgpu module's New-WgpuRenderPipeline
/// builds one, drawing into a GpuSurface's texture. Its vertex layout comes
/// from the shader's @location inputs, each fed from a buffer of its own.
/// With -Depth or -DepthFormat it tests depth, stencil or both in a depth
/// attachment the surface keeps, which Set-GuiGpuSurface clears each frame.
/// Set-GuiGpuSurface draws with it.
///
/// # Examples
/// $pipeline = New-GuiGpuRenderPipeline $wgsl
/// New-GuiGpuRenderPipeline $wgsl -Depth -SampleCount 4 -Blend Alpha
/// New-GuiGpuRenderPipeline $wgsl -DepthFormat Stencil8 -StencilPass Replace
#[cmdlet(verb = "New", noun = "GuiGpuRenderPipeline", output = ["PwrsGui.GpuRenderPipeline"])]
#[derive(Default)]
pub struct NewGuiGpuRenderPipeline {
    /// WGSL text with a vertex and a fragment entry point.
    #[param(mandatory, position = 0, value_from_pipeline)]
    pub shader: Option<String>,
    /// Test and write depth in a Depth32Float attachment.
    #[param]
    pub depth: bool,
    /// Test depth, stencil or both in an attachment of this format: Stencil8, Depth16Unorm, Depth24Plus, Depth24PlusStencil8, Depth32Float or Depth32FloatStencil8.
    #[param]
    pub depth_format: Option<GuiDepthFormat>,
    /// How a fragment's depth compares with the depth under it, for the fragment to be drawn; Less when omitted.
    #[param]
    pub depth_compare: Option<GuiCompareFunction>,
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
    /// How the stencil test compares the stencil reference, which a draw sets, with the stencil value under a fragment, the reference on the left; Always when omitted.
    #[param]
    pub stencil_compare: Option<GuiCompareFunction>,
    /// What a failed stencil test does to the stencil value; Keep when omitted.
    #[param]
    pub stencil_fail: Option<GuiStencilOperation>,
    /// What a passed stencil test whose depth test fails does to the stencil value; Keep when omitted.
    #[param]
    pub stencil_depth_fail: Option<GuiStencilOperation>,
    /// What passing the stencil test and the depth test does to the stencil value; Keep when omitted.
    #[param]
    pub stencil_pass: Option<GuiStencilOperation>,
    /// The bits of the reference and of the stencil value the stencil test compares; all of them when omitted.
    #[param]
    pub stencil_read_mask: Option<u32>,
    /// The bits of the stencil value the stencil operations change; all of them when omitted.
    #[param]
    pub stencil_write_mask: Option<u32>,
    /// -StencilCompare for triangles facing away; -StencilCompare's when omitted.
    #[param]
    pub back_stencil_compare: Option<GuiCompareFunction>,
    /// -StencilFail for triangles facing away; -StencilFail's when omitted.
    #[param]
    pub back_stencil_fail: Option<GuiStencilOperation>,
    /// -StencilDepthFail for triangles facing away; -StencilDepthFail's when omitted.
    #[param]
    pub back_stencil_depth_fail: Option<GuiStencilOperation>,
    /// -StencilPass for triangles facing away; -StencilPass's when omitted.
    #[param]
    pub back_stencil_pass: Option<GuiStencilOperation>,
    /// How vertices make primitives; TriangleList when omitted.
    #[param]
    pub topology: Option<GuiPrimitiveTopology>,
    /// Which faces to discard; none when omitted.
    #[param]
    pub cull_mode: Option<GuiCullMode>,
    /// How fragments combine with the surface; Replace when omitted.
    /// Constant mixes by the constant a draw sets, and Custom takes
    /// -BlendSource, -BlendDestination and -BlendOperation.
    #[param]
    pub blend: Option<GuiBlendMode>,
    /// With -Blend Custom, what the fragment's color is multiplied by; One when omitted.
    #[param]
    pub blend_source: Option<GuiBlendFactor>,
    /// With -Blend Custom, what the color under the fragment is multiplied by; Zero when omitted.
    #[param]
    pub blend_destination: Option<GuiBlendFactor>,
    /// With -Blend Custom, how the two products combine; Add when omitted.
    #[param]
    pub blend_operation: Option<GuiBlendOperation>,
    /// With -Blend Custom, what the fragment's alpha is multiplied by; -BlendSource when omitted.
    #[param]
    pub alpha_blend_source: Option<GuiBlendFactor>,
    /// With -Blend Custom, what the alpha under the fragment is multiplied by; -BlendDestination when omitted.
    #[param]
    pub alpha_blend_destination: Option<GuiBlendFactor>,
    /// With -Blend Custom, how the two alpha products combine; -BlendOperation when omitted.
    #[param]
    pub alpha_blend_operation: Option<GuiBlendOperation>,
    /// Samples per pixel, resolved into the surface; 1 when omitted.
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
}

impl NewGuiGpuRenderPipeline {
    fn options(&self) -> PsResult<PipelineOptions> {
        Ok(PipelineOptions {
            depth: self.depth,
            depth_format: self.depth_format,
            depth_compare: self.depth_compare,
            depth_write: self.depth_write,
            depth_bias: self.depth_bias,
            depth_bias_slope_scale: self.depth_bias_slope_scale,
            depth_bias_clamp: self.depth_bias_clamp,
            stencil_compare: self.stencil_compare,
            stencil_fail: self.stencil_fail,
            stencil_depth_fail: self.stencil_depth_fail,
            stencil_pass: self.stencil_pass,
            stencil_read_mask: self.stencil_read_mask,
            stencil_write_mask: self.stencil_write_mask,
            back_stencil_compare: self.back_stencil_compare,
            back_stencil_fail: self.back_stencil_fail,
            back_stencil_depth_fail: self.back_stencil_depth_fail,
            back_stencil_pass: self.back_stencil_pass,
            topology: self.topology,
            cull_mode: self.cull_mode,
            blend: self.blend,
            blend_source: self.blend_source,
            blend_destination: self.blend_destination,
            blend_operation: self.blend_operation,
            alpha_blend_source: self.alpha_blend_source,
            alpha_blend_destination: self.alpha_blend_destination,
            alpha_blend_operation: self.alpha_blend_operation,
            sample_count: self.sample_count,
            vertex_entry: self.vertex_entry.clone(),
            fragment_entry: self.fragment_entry.clone(),
            constants: constants_of(self.constant.as_ref())?,
        })
    }
}

impl Cmdlet for NewGuiGpuRenderPipeline {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let Some(source) = &self.shader else {
            return Err(binding_error("GuiGpuNoShader", String::from("no shader was given")));
        };
        let options = self.options()?;
        let spec = options.spec(wgsl_of(source)?);
        let session = session::egui_session(ps)?;
        listen(ps, &session)?;
        let (pipeline, vertex_inputs, bindings) = match call(ps, &session, GpuOp::NewRenderPipeline { spec: spec.clone() })? {
            GpuValue::RenderPipeline { pipeline, vertex_inputs, bindings } => (pipeline, vertex_inputs, bindings),
            other => return Err(unexpected("a new render pipeline", &other)),
        };
        let label = match &self.label {
            Some(l) => l.clone(),
            None => format!("render pipeline {pipeline}"),
        };
        let adapter = session.hello().map(|h| h.capabilities.adapter.clone());
        ps.write(GpuRenderPipeline {
            label,
            vertex_entry_point: options.vertex_entry,
            fragment_entry_point: options.fragment_entry,
            vertex_inputs,
            bindings,
            depth_format: spec.depth_target().map(|f| f.name().to_string()),
            sample_count: options.sample_count.unwrap_or(1),
            adapter,
            id: pipeline,
            session,
            spec,
        })
    }
}

/// The keys a -Draw hashtable takes, which are Set-GuiGpuSurface's own
/// parameters for one draw.
const DRAW_KEYS: [&str; 38] = [
    "Shader",
    "Vertex",
    "VertexCount",
    "InstanceCount",
    "Index",
    "Binding",
    "BlendConstant",
    "StencilReference",
    "Topology",
    "CullMode",
    "Blend",
    "BlendSource",
    "BlendDestination",
    "BlendOperation",
    "AlphaBlendSource",
    "AlphaBlendDestination",
    "AlphaBlendOperation",
    "Depth",
    "DepthFormat",
    "DepthCompare",
    "DepthWrite",
    "DepthBias",
    "DepthBiasSlopeScale",
    "DepthBiasClamp",
    "StencilCompare",
    "StencilFail",
    "StencilDepthFail",
    "StencilPass",
    "StencilReadMask",
    "StencilWriteMask",
    "BackStencilCompare",
    "BackStencilFail",
    "BackStencilDepthFail",
    "BackStencilPass",
    "SampleCount",
    "VertexEntry",
    "FragmentEntry",
    "Constant",
];

/// One draw as Set-GuiGpuSurface's own parameters or a -Draw hashtable give
/// it.
#[derive(Default)]
struct GivenDraw {
    shader: Option<PsObject>,
    vertex: Option<PsObject>,
    vertex_count: Option<u32>,
    instance_count: Option<u32>,
    index: Option<PsObject>,
    binding: Option<PsObject>,
    blend_constant: Option<Vec<f64>>,
    stencil_reference: Option<u32>,
    options: PipelineOptions,
}

impl GivenDraw {
    /// Whether anything but the shader was given, which no draw takes
    /// without one.
    fn any_but_shader(&self) -> bool {
        self.vertex.is_some()
            || self.vertex_count.is_some()
            || self.instance_count.is_some()
            || self.index.is_some()
            || self.binding.is_some()
            || self.blend_constant.is_some()
            || self.stencil_reference.is_some()
            || self.options.any()
    }

    /// The draw a -Draw hashtable gives, its keys [`DRAW_KEYS`] compared
    /// without regard to case; a key given null is not given.
    fn of_table(given: &PsObject) -> PsResult<GivenDraw> {
        let type_name = given.type_name()?;
        if !is_dictionary(&type_name) {
            return Err(binding_error("GuiGpuDrawKey", format!("a draw is a hashtable of {}, not a {type_name}", DRAW_KEYS.join(", "))));
        }
        let table = PsHashtable(given.clone());
        let mut d = GivenDraw::default();
        for key in table.keys()? {
            let v = table.get(&key)?;
            let known = DRAW_KEYS.iter().any(|k| k.eq_ignore_ascii_case(&key));
            if !known {
                return Err(binding_error("GuiGpuDrawKey", format!("a draw takes {}; it was given {key}", DRAW_KEYS.join(", "))));
            }
            if v.is_null() {
                continue;
            }
            let value = |e: String| binding_error("GuiGpuDrawValue", format!("{key}: {e}"));
            let read = |e: PsError| value(e.message);
            let o = &mut d.options;
            match key.to_ascii_lowercase().as_str() {
                "shader" => d.shader = Some(v),
                "vertex" => d.vertex = Some(v),
                "vertexcount" => d.vertex_count = Some(u32::from_ps(&v).map_err(read)?),
                "instancecount" => d.instance_count = Some(u32::from_ps(&v).map_err(read)?),
                "index" => d.index = Some(v),
                "binding" => d.binding = Some(v),
                "blendconstant" => d.blend_constant = Some(Vec::<f64>::from_ps(&v).map_err(read)?),
                "stencilreference" => d.stencil_reference = Some(u32::from_ps(&v).map_err(read)?),
                "depthformat" => o.depth_format = Some(named(&v, &DEPTH_FORMATS).map_err(value)?),
                "depthcompare" => o.depth_compare = Some(named(&v, &COMPARE_FUNCTIONS).map_err(value)?),
                "depthwrite" => o.depth_write = Some(bool::from_ps(&v).map_err(read)?),
                "depthbias" => o.depth_bias = Some(i32::from_ps(&v).map_err(read)?),
                "depthbiasslopescale" => o.depth_bias_slope_scale = Some(f64::from_ps(&v).map_err(read)?),
                "depthbiasclamp" => o.depth_bias_clamp = Some(f64::from_ps(&v).map_err(read)?),
                "stencilcompare" => o.stencil_compare = Some(named(&v, &COMPARE_FUNCTIONS).map_err(value)?),
                "stencilfail" => o.stencil_fail = Some(named(&v, &STENCIL_OPERATIONS).map_err(value)?),
                "stencildepthfail" => o.stencil_depth_fail = Some(named(&v, &STENCIL_OPERATIONS).map_err(value)?),
                "stencilpass" => o.stencil_pass = Some(named(&v, &STENCIL_OPERATIONS).map_err(value)?),
                "stencilreadmask" => o.stencil_read_mask = Some(u32::from_ps(&v).map_err(read)?),
                "stencilwritemask" => o.stencil_write_mask = Some(u32::from_ps(&v).map_err(read)?),
                "backstencilcompare" => o.back_stencil_compare = Some(named(&v, &COMPARE_FUNCTIONS).map_err(value)?),
                "backstencilfail" => o.back_stencil_fail = Some(named(&v, &STENCIL_OPERATIONS).map_err(value)?),
                "backstencildepthfail" => o.back_stencil_depth_fail = Some(named(&v, &STENCIL_OPERATIONS).map_err(value)?),
                "backstencilpass" => o.back_stencil_pass = Some(named(&v, &STENCIL_OPERATIONS).map_err(value)?),
                "topology" => o.topology = Some(named(&v, &TOPOLOGIES).map_err(value)?),
                "cullmode" => o.cull_mode = Some(named(&v, &CULL_MODES).map_err(value)?),
                "blend" => o.blend = Some(named(&v, &BLEND_MODES).map_err(value)?),
                "blendsource" => o.blend_source = Some(named(&v, &BLEND_FACTORS).map_err(value)?),
                "blenddestination" => o.blend_destination = Some(named(&v, &BLEND_FACTORS).map_err(value)?),
                "blendoperation" => o.blend_operation = Some(named(&v, &BLEND_OPERATIONS).map_err(value)?),
                "alphablendsource" => o.alpha_blend_source = Some(named(&v, &BLEND_FACTORS).map_err(value)?),
                "alphablenddestination" => o.alpha_blend_destination = Some(named(&v, &BLEND_FACTORS).map_err(value)?),
                "alphablendoperation" => o.alpha_blend_operation = Some(named(&v, &BLEND_OPERATIONS).map_err(value)?),
                "depth" => o.depth = bool::from_ps(&v).map_err(read)?,
                "samplecount" => o.sample_count = Some(u32::from_ps(&v).map_err(read)?),
                "vertexentry" => o.vertex_entry = Some(String::from_ps(&v).map_err(read)?),
                "fragmententry" => o.fragment_entry = Some(String::from_ps(&v).map_err(read)?),
                _constant => {
                    if !is_dictionary(&v.type_name()?) {
                        return Err(value(String::from("override constants are a hashtable of numbers by name or @id")));
                    }
                    o.constants = constants_of(Some(&PsHashtable(v))).map_err(read)?;
                }
            }
        }
        Ok(d)
    }
}

/// `e` as the error of draw `at` of -Draw.
fn in_draw(at: usize, mut e: PsError) -> PsError {
    e.message = format!("draw {at}: {}", e.message);
    e
}

/// The keys a -Compute hashtable takes: Invoke-GuiGpuCompute's parameters,
/// and Every.
const PASS_KEYS: [&str; 10] = ["Shader", "Binding", "Workgroups", "Invocations", "Indirect", "IndirectOffset", "Entry", "Constant", "Language", "Every"];

/// One compute pass as a -Compute hashtable gives it: the run
/// Invoke-GuiGpuCompute's parameters would give, the indirect buffer it
/// names, and when it runs.
struct GivenPass {
    compute: InvokeGuiGpuCompute,
    indirect: Option<PsObject>,
    every: PassEvery,
}

impl GivenPass {
    /// The pass a -Compute hashtable gives, its keys [`PASS_KEYS`] compared
    /// without regard to case; a key given null is not given.
    fn of_table(given: &PsObject) -> PsResult<GivenPass> {
        let type_name = given.type_name()?;
        if !is_dictionary(&type_name) {
            return Err(binding_error("GuiGpuPassKey", format!("a pass is a hashtable of {}, not a {type_name}", PASS_KEYS.join(", "))));
        }
        let table = PsHashtable(given.clone());
        let mut p = GivenPass { compute: InvokeGuiGpuCompute::default(), indirect: None, every: PassEvery::Frame };
        for key in table.keys()? {
            let v = table.get(&key)?;
            if !PASS_KEYS.iter().any(|k| k.eq_ignore_ascii_case(&key)) {
                return Err(binding_error("GuiGpuPassKey", format!("a pass takes {}; it was given {key}", PASS_KEYS.join(", "))));
            }
            if v.is_null() {
                continue;
            }
            let value = |e: String| binding_error("GuiGpuPassValue", format!("{key}: {e}"));
            let read = |e: PsError| value(e.message);
            let c = &mut p.compute;
            match key.to_ascii_lowercase().as_str() {
                "shader" => c.shader = Some(String::from_ps(&v).map_err(read)?),
                "binding" => c.binding = Some(v),
                "workgroups" => c.workgroups = Some(Vec::<u32>::from_ps(&crate::widgets::as_list(&v).map_err(value)?).map_err(read)?),
                "invocations" => c.invocations = Some(Vec::<u64>::from_ps(&crate::widgets::as_list(&v).map_err(value)?).map_err(read)?),
                "indirect" => {
                    c.indirect = Some(PsProxy::<GpuBuffer>::from_ps(&v).map_err(read)?);
                    p.indirect = Some(v);
                }
                "indirectoffset" => c.indirect_offset = Some(u64::from_ps(&v).map_err(read)?),
                "entry" => c.entry = Some(String::from_ps(&v).map_err(read)?),
                "constant" => {
                    if !is_dictionary(&v.type_name()?) {
                        return Err(value(String::from("override constants are a hashtable of numbers by name or @id")));
                    }
                    c.constant = Some(PsHashtable(v));
                }
                "language" => {
                    let named = String::from_ps(&v).map_err(read)?;
                    c.language = Some(match named.to_ascii_lowercase().as_str() {
                        "wgsl" => GuiShaderLanguage::Wgsl,
                        "glsl" => GuiShaderLanguage::Glsl,
                        _ => return Err(value(format!("{named} is not a shader language: Wgsl or Glsl"))),
                    });
                }
                _every => {
                    let named = String::from_ps(&v).map_err(read)?;
                    p.every = match named.to_ascii_lowercase().as_str() {
                        "frame" => PassEvery::Frame,
                        "once" => PassEvery::Once,
                        _ => return Err(value(format!("{named} is not when a pass runs: Frame or Once"))),
                    };
                }
            }
        }
        if p.compute.shader.is_none() {
            return Err(binding_error("GuiGpuNoShader", String::from("a pass takes a Shader: WGSL or GLSL compute text")));
        }
        Ok(p)
    }

    /// The pass as the renderer takes it: its buffers, textures and
    /// samplers must live in `session`, and each it names is put in `held`.
    fn pass(&self, session: &Session, held: &mut Vec<PsObject>) -> PsResult<SurfacePass> {
        let (source, language, declared) = self.compute.declared()?;
        let parts = self.compute.parts(source, language, &declared, session, held)?;
        held.extend(self.indirect.clone());
        Ok(SurfacePass { source: parts.source, language: parts.language, entry: parts.entry, constants: parts.constants, bindings: parts.bindings, dispatch: parts.dispatch, every: self.every })
    }
}

/// `e` as the error of pass `at` of -Compute.
fn in_pass(at: usize, mut e: PsError) -> PsError {
    e.message = format!("pass {at}: {}", e.message);
    e
}

/// The draw `given` describes, as the renderer takes it, with the objects
/// it names put in `held`: its pipeline, a pipeline object or one built
/// from WGSL text for this draw alone, and its buffers, which must live in
/// `session`. An array for a vertex input or a binding goes with the draw,
/// and an array of indices becomes a buffer of its own.
fn surface_draw(ps: &Pipeline<'_>, session: &Session, given: GivenDraw, held: &mut Vec<PsObject>) -> PsResult<SurfaceDraw> {
    let Some(shader) = given.shader.filter(|s| !s.is_null()) else {
        return Err(binding_error("GuiGpuNoShader", String::from("a draw takes a shader: WGSL text or a PwrsGui.GpuRenderPipeline")));
    };
    let pipeline = match shader.type_name()?.as_str() {
        "PwrsGui.GpuRenderPipeline" => {
            if given.options.any() {
                let text = "a PwrsGui.GpuRenderPipeline draws as New-GuiGpuRenderPipeline built it, so the pipeline parameters go to that command; a draw takes them to build a pipeline of its own from WGSL text";
                return Err(binding_error("GuiGpuPipelineOptions", String::from(text)));
            }
            let id = pipeline_in(&PsProxy::<GpuRenderPipeline>::from_ps(&shader)?, session)?;
            held.push(shader);
            DrawPipeline::Table(id)
        }
        "System.String" => DrawPipeline::Inline(given.options.spec(wgsl_of(&String::from_ps(&shader)?)?)),
        other => return Err(binding_error("GuiGpuNoShader", format!("a draw's shader is WGSL text or a PwrsGui.GpuRenderPipeline, not a {other}"))),
    };
    let vertices = resources_for(given.vertex.as_ref(), session, "Vertex", "GuiGpuVertex", held)?;
    let bindings = resources_for(given.binding.as_ref(), session, "Binding", "GuiGpuBinding", held)?;
    let index = match given.index.filter(|i| !i.is_null()) {
        Some(value) => Some(index_of(ps, session, &value, held)?),
        None => None,
    };
    let blend_constant = match given.blend_constant.as_deref() {
        None => None,
        Some([r, g, b, a]) => Some([*r, *g, *b, *a]),
        Some(other) => return Err(binding_error("GuiGpuBlendConstant", format!("a blend constant is four numbers, red, green, blue and alpha; {} were given", other.len()))),
    };
    Ok(SurfaceDraw {
        pipeline,
        vertices,
        index,
        vertex_count: given.vertex_count,
        instance_count: given.instance_count.unwrap_or(1),
        bindings,
        blend_constant,
        stencil_reference: given.stencil_reference,
    })
}

/// What `value` gives a draw's vertex inputs or its bindings, `what` naming
/// which, each with its slot: a hashtable by name, or for a binding
/// `group:binding`; values in order, location order for vertex inputs and
/// group and binding order for bindings; or one value, for the first. An
/// array of numbers is one value. Each is a resource as [`draw_resource`]
/// reads it.
fn resources_for(value: Option<&PsObject>, session: &Session, what: &str, id: &str, held: &mut Vec<PsObject>) -> PsResult<Vec<(GpuSlot, GpuResource)>> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let type_name = value.type_name()?;
    let mut out = Vec::new();
    if is_dictionary(&type_name) {
        let table = PsHashtable(value.clone());
        for key in table.keys()? {
            let given = table.get(&key)?;
            out.push((GpuSlot::Name(key.clone()), draw_resource(&given, session, &format!("{what} {key}"), id, held)?));
        }
        return Ok(out);
    }
    let items = if is_list(&type_name) {
        let items = Vec::<PsObject>::from_ps(value)?;
        let numbers = match items.first() {
            Some(first) => is_number(&first.type_name()?),
            None => false,
        };
        if numbers { vec![value.clone()] } else { items }
    } else {
        vec![value.clone()]
    };
    for (at, given) in (0u32..).zip(items) {
        out.push((GpuSlot::Order(at), draw_resource(&given, session, what, id, held)?));
    }
    Ok(out)
}

/// The resource `value` gives one vertex input or binding, `what` naming
/// it and `id` the error refusing it: a PwrsGui.GpuBuffer of `session`, or
/// for a binding a PwrsGui.GpuTexture or GpuSampler of it, which goes into
/// `held`; or an array of numbers in its own element type, Float32 for an
/// untyped array, which the renderer copies to the GPU as the type the
/// shader declares.
fn draw_resource(value: &PsObject, session: &Session, what: &str, id: &str, held: &mut Vec<PsObject>) -> PsResult<GpuResource> {
    let type_name = value.type_name()?;
    if let Some(table) = table_resource(value, &type_name, session)? {
        held.push(value.clone());
        return Ok(table);
    }
    let takes = if id == "GuiGpuVertex" { "a PwrsGui.GpuBuffer or an array of numbers" } else { "a PwrsGui.GpuBuffer, GpuTexture or GpuSampler, or an array of numbers" };
    if is_dictionary(&type_name) {
        return Err(binding_error(id, format!("{what} takes {takes}, not a hashtable")));
    }
    let (element, bytes) = wgpu_pwsh::host_bytes(value, None).map_err(|e| binding_error(id, format!("{what} takes {takes}, and was given a {type_name}: {}", e.message)))?;
    if bytes.is_empty() {
        return Err(binding_error("GuiGpuEmpty", format!("the array for {what} is empty")));
    }
    Ok(GpuResource::Array { element: wire(element), bytes })
}

/// The index buffer `value` gives a draw, the type of its indices, and how
/// many it draws: a PwrsGui.GpuBuffer of UInt16 or UInt32 elements of
/// `session`, or an array of indices, put in a buffer of its own as UInt16
/// for a ushort[] and as UInt32 for any other. The buffer goes into `held`.
fn index_of(ps: &Pipeline<'_>, session: &Session, value: &PsObject, held: &mut Vec<PsObject>) -> PsResult<(u64, IndexFormat, u32)> {
    let count = |length: u64| u32::try_from(length).map_err(|e| binding_error("GuiGpuIndexCount", format!("{length} indices are more than a draw takes: {e}")));
    if value.type_name()? == "PwrsGui.GpuBuffer" {
        let proxy = PsProxy::<GpuBuffer>::from_ps(value)?;
        let (element, length) = proxy.with(|b| (b.element_type, b.length))?;
        let format = match element {
            GuiElementType::UInt16 => IndexFormat::Uint16,
            GuiElementType::UInt32 => IndexFormat::Uint32,
            other => return Err(binding_error("GuiGpuIndexType", format!("an index buffer has UInt16 or UInt32 elements, not {other:?}"))),
        };
        let id = buffer_in(&proxy, session)?;
        held.push(value.clone());
        return Ok((id, format, count(length)?));
    }
    let (own, bytes) = wgpu_pwsh::host_bytes(value, None)?;
    let (element, bytes, format) = match own {
        ElementType::UInt16 => (own, bytes, IndexFormat::Uint16),
        _ => (ElementType::UInt32, wgpu_pwsh::host_bytes(value, Some(ElementType::UInt32))?.1, IndexFormat::Uint32),
    };
    let length = bytes.len() as u64 / element.size();
    let buffer = make_in(ps, session.clone(), element, length, Some(bytes), Some(&[GuiBufferUsage::Index, GuiBufferUsage::CopyDst]), Some(String::from("indices")))?;
    let id = buffer.id;
    held.push(buffer.into_ps()?);
    Ok((id, format, count(length)?))
}

/// The clear color `given` names: red, green, blue and alpha, the alpha 1
/// when three are given; opaque black when none is.
fn clear_of(given: Option<&[f64]>) -> PsResult<[f64; 4]> {
    match given {
        None => Ok([0.0, 0.0, 0.0, 1.0]),
        Some([r, g, b]) => Ok([*r, *g, *b, 1.0]),
        Some([r, g, b, a]) => Ok([*r, *g, *b, *a]),
        Some(other) => Err(binding_error("GuiGpuClearColor", format!("a clear color is three or four numbers, red, green, blue and alpha; {} were given", other.len()))),
    }
}

/// Gives a GpuSurface what it draws each frame, in the Wgpu module's
/// Invoke-WgpuRender terms: its texture cleared to -ClearColor, then one
/// draw from this command's own parameters, or the draws -Draw gives in
/// order, each a hashtable whose keys are those parameters' names. Each
/// call replaces all the surface's draws at once, so no frame shows part of
/// them; with no draw, the surface shows its clear color.
///
/// A draw's -Shader is a PwrsGui.GpuRenderPipeline, or WGSL text built with
/// the pipeline parameters for this draw alone. -Vertex gives its vertex
/// inputs' values by input name or in location order, and -Binding its
/// bindings' by name, group:binding or in the order the shaders declare
/// them: each a PwrsGui.GpuBuffer, or an array of numbers copied to the GPU
/// as the type the shader declares, or as it is for a binding of a struct.
/// A texture or sampler binding takes a PwrsGui.GpuTexture or GpuSampler.
/// -Index takes a buffer of UInt16 or UInt32 indices, or an array of
/// indices. Without -VertexCount a draw draws as many vertices as it has
/// indices, or else as many as its first vertex input has values for.
///
/// A pipeline built with -Depth or -DepthFormat tests depth, stencil or
/// both in the surface's depth attachment, of the format its draws share,
/// which each frame clears to -ClearDepth and -ClearStencil, 1 and 0 when
/// omitted, so -DepthCompare Greater with -ClearDepth 0 draws reversed
/// depth. Each draw whose pipeline tests stencil compares its own
/// -StencilReference, 0 when omitted, whatever a draw before it gave.
///
/// A draw whose shaders declare a binding at @group(1) @binding(0) reads the
/// surface's frame uniform there, a struct whose first 160 bytes the
/// renderer writes before each frame: time and delta in seconds as f32, at
/// bytes 0 and 4; the surface's resolution and the pointer as vec2<f32> in
/// physical pixels from its top left, at 8 and 16, the pointer -1, -1 while
/// outside it; and the view and projection of the camera Set-GuiGpuCamera
/// gives, as mat4x4<f32> at 32 and 96, identity matrices without one.
/// -Uniform gives the struct's own fields from byte 160.
/// A surface whose draws read it draws every frame.
///
/// -Compute gives compute passes the frame runs before the draws, in
/// order, in the frame's own commands, so what a pass writes into a
/// buffer the draws read without it leaving the GPU. Each is a hashtable
/// of Invoke-GuiGpuCompute's parameters and Every: Frame runs it before
/// every frame's draws, Once before the first frame's alone. A pass may
/// read the frame uniform too, and the passes are replaced with the draws.
///
/// In an HTML window the page draws the surface from its own copies of the
/// objects the draws and passes name, which the module sends again after
/// each change. It runs the passes over its copies, so what they write
/// stays on the page. A page whose engine offers no WebGPU draws with
/// WebGL 2 and refuses a surface with a compute pass, a storage buffer or
/// texture, or a blend with a second source, with GuiGpuNoWebGpu.
///
/// The surface uses the buffers, textures, samplers and pipelines its
/// draws and passes name until they change or its window closes. Inside a handler it acts on the
/// handler's window; elsewhere -Window names the window.
///
/// # Examples
/// Set-GuiGpuSurface View $pipeline -VertexCount 3
/// Set-GuiGpuSurface View $wgsl -Vertex ([float[]](-1, -1, 3, -1, -1, 3)) -Binding @{ tint = 0, 1, 0, 1 }
/// Set-GuiGpuSurface View $wgsl -Vertex @{ position = $positions } -Index ([uint32[]](0, 1, 2, 2, 1, 3)) -Binding @{ tint = $tint }
/// Set-GuiGpuSurface View -Draw @{ Shader = $sky; VertexCount = 3 }, @{ Shader = $mesh; Vertex = $corners; VertexCount = 36; Depth = $true } -ClearColor 0, 0, 0
/// Set-GuiGpuSurface View $framed -VertexCount 3 -Uniform 0, 1, 0, 1
/// Set-GuiGpuSurface View $wgsl -VertexCount 3 -Depth -DepthCompare Greater -ClearDepth 0
#[cmdlet(verb = "Set", noun = "GuiGpuSurface", default_parameter_set = "One")]
#[derive(Default)]
pub struct SetGuiGpuSurface {
    /// The surface's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The draw's pipeline: a PwrsGui.GpuRenderPipeline, or WGSL text with a vertex and a fragment entry point, built with the pipeline parameters.
    #[param(position = 1, raw, set = "One")]
    pub shader: Option<PsObject>,
    /// The vertex inputs' values: a hashtable by input name, or values in location order, each a PwrsGui.GpuBuffer or an array of numbers copied as the input's type.
    #[param(raw, set = "One")]
    pub vertex: Option<PsObject>,
    /// Vertices to draw; when omitted, the indices' count with -Index, or else as many as the first vertex input has values for.
    #[param(set = "One")]
    pub vertex_count: Option<u32>,
    /// Instances to draw; 1 when omitted.
    #[param(set = "One")]
    pub instance_count: Option<u32>,
    /// Indices to draw by: a PwrsGui.GpuBuffer of UInt16 or UInt32 elements, or an array of indices, stored as UInt16 for a ushort[] and as UInt32 for any other.
    #[param(raw, set = "One")]
    pub index: Option<PsObject>,
    /// The bindings' values: a hashtable by binding name or group:binding, or values in the order the shaders declare their bindings, each a PwrsGui.GpuBuffer, GpuTexture or GpuSampler, or an array of numbers copied as the binding's type.
    #[param(raw, set = "One")]
    pub binding: Option<PsObject>,
    /// The color a blend factor of Constant reads, as red, green, blue and alpha; given only when the blend reads it.
    #[param(set = "One")]
    pub blend_constant: Option<Vec<f64>>,
    /// The value the pipeline's stencil test compares with and Replace writes; 0 when omitted. A pipeline with no stencil test refuses it.
    #[param(set = "One")]
    pub stencil_reference: Option<u32>,
    /// For a shader of WGSL text, test and write depth in a Depth32Float attachment.
    #[param(set = "One")]
    pub depth: bool,
    /// For a shader of WGSL text, test depth, stencil or both in an attachment of this format: Stencil8, Depth16Unorm, Depth24Plus, Depth24PlusStencil8, Depth32Float or Depth32FloatStencil8.
    #[param(set = "One")]
    pub depth_format: Option<GuiDepthFormat>,
    /// For a shader of WGSL text, how a fragment's depth compares with the depth under it, for the fragment to be drawn; Less when omitted.
    #[param(set = "One")]
    pub depth_compare: Option<GuiCompareFunction>,
    /// For a shader of WGSL text, whether a fragment that passes the depth test writes its depth; it does unless -DepthWrite:$false.
    #[param(set = "One")]
    pub depth_write: Option<bool>,
    /// For a shader of WGSL text, a constant added to each triangle's depth, in the depth format's smallest steps; 0 when omitted.
    #[param(set = "One")]
    pub depth_bias: Option<i32>,
    /// For a shader of WGSL text, a bias this many times the triangle's depth slope; 0 when omitted.
    #[param(set = "One")]
    pub depth_bias_slope_scale: Option<f64>,
    /// For a shader of WGSL text, the largest bias the two give together, or the most negative when below 0; no limit when omitted.
    #[param(set = "One")]
    pub depth_bias_clamp: Option<f64>,
    /// For a shader of WGSL text, how the stencil test compares -StencilReference with the stencil value under a fragment, the reference on the left; Always when omitted.
    #[param(set = "One")]
    pub stencil_compare: Option<GuiCompareFunction>,
    /// For a shader of WGSL text, what a failed stencil test does to the stencil value; Keep when omitted.
    #[param(set = "One")]
    pub stencil_fail: Option<GuiStencilOperation>,
    /// For a shader of WGSL text, what a passed stencil test whose depth test fails does to the stencil value; Keep when omitted.
    #[param(set = "One")]
    pub stencil_depth_fail: Option<GuiStencilOperation>,
    /// For a shader of WGSL text, what passing the stencil test and the depth test does to the stencil value; Keep when omitted.
    #[param(set = "One")]
    pub stencil_pass: Option<GuiStencilOperation>,
    /// For a shader of WGSL text, the bits of the reference and of the stencil value the stencil test compares; all of them when omitted.
    #[param(set = "One")]
    pub stencil_read_mask: Option<u32>,
    /// For a shader of WGSL text, the bits of the stencil value the stencil operations change; all of them when omitted.
    #[param(set = "One")]
    pub stencil_write_mask: Option<u32>,
    /// -StencilCompare for triangles facing away; -StencilCompare's when omitted.
    #[param(set = "One")]
    pub back_stencil_compare: Option<GuiCompareFunction>,
    /// -StencilFail for triangles facing away; -StencilFail's when omitted.
    #[param(set = "One")]
    pub back_stencil_fail: Option<GuiStencilOperation>,
    /// -StencilDepthFail for triangles facing away; -StencilDepthFail's when omitted.
    #[param(set = "One")]
    pub back_stencil_depth_fail: Option<GuiStencilOperation>,
    /// -StencilPass for triangles facing away; -StencilPass's when omitted.
    #[param(set = "One")]
    pub back_stencil_pass: Option<GuiStencilOperation>,
    /// For a shader of WGSL text, how vertices make primitives; TriangleList when omitted.
    #[param(set = "One")]
    pub topology: Option<GuiPrimitiveTopology>,
    /// For a shader of WGSL text, which faces to discard; none when omitted.
    #[param(set = "One")]
    pub cull_mode: Option<GuiCullMode>,
    /// For a shader of WGSL text, how fragments combine with the surface; Replace when omitted.
    #[param(set = "One")]
    pub blend: Option<GuiBlendMode>,
    /// With -Blend Custom, what the fragment's color is multiplied by; One when omitted.
    #[param(set = "One")]
    pub blend_source: Option<GuiBlendFactor>,
    /// With -Blend Custom, what the color under the fragment is multiplied by; Zero when omitted.
    #[param(set = "One")]
    pub blend_destination: Option<GuiBlendFactor>,
    /// With -Blend Custom, how the two products combine; Add when omitted.
    #[param(set = "One")]
    pub blend_operation: Option<GuiBlendOperation>,
    /// With -Blend Custom, what the fragment's alpha is multiplied by; -BlendSource when omitted.
    #[param(set = "One")]
    pub alpha_blend_source: Option<GuiBlendFactor>,
    /// With -Blend Custom, what the alpha under the fragment is multiplied by; -BlendDestination when omitted.
    #[param(set = "One")]
    pub alpha_blend_destination: Option<GuiBlendFactor>,
    /// With -Blend Custom, how the two alpha products combine; -BlendOperation when omitted.
    #[param(set = "One")]
    pub alpha_blend_operation: Option<GuiBlendOperation>,
    /// For a shader of WGSL text, samples per pixel, resolved into the surface; 1 when omitted.
    #[param(set = "One")]
    pub sample_count: Option<u32>,
    /// For a shader of WGSL text, the vertex entry point; the shader's only one when omitted.
    #[param(set = "One")]
    pub vertex_entry: Option<String>,
    /// For a shader of WGSL text, the fragment entry point; the shader's only one when omitted.
    #[param(set = "One")]
    pub fragment_entry: Option<String>,
    /// For a shader of WGSL text, override constant values, by name or `@id`.
    #[param(set = "One")]
    pub constant: Option<PsHashtable>,
    /// The draws, in order, each a hashtable whose keys are this command's draw parameters: Shader, Vertex, VertexCount, InstanceCount, Index, Binding, BlendConstant, StencilReference and the pipeline parameters.
    #[param(mandatory, set = "Draw")]
    pub draw: Vec<PsObject>,
    /// The color the surface is cleared to each frame, as red, green, blue and alpha, the alpha 1 when three are given; opaque black when omitted.
    #[param]
    pub clear_color: Option<Vec<f64>>,
    /// The depth the surface's depth attachment is cleared to each frame, from 0 to 1; 1 when omitted. Refused when the draws' depth format holds no depth.
    #[param]
    pub clear_depth: Option<f64>,
    /// The value the surface's stencil is cleared to each frame; 0 when omitted. Refused when the draws' depth format holds no stencil.
    #[param]
    pub clear_stencil: Option<u32>,
    /// The frame uniform's own fields, from its byte 160, for draws whose shaders read it at @group(1) @binding(0): an array of numbers in its own element type, Float32 for an untyped array.
    #[param(raw)]
    pub uniform: Option<PsObject>,
    /// The compute passes run before the draws, in order, each a hashtable of Invoke-GuiGpuCompute's parameters (Shader, Binding, Workgroups, Invocations, Indirect, IndirectOffset, Entry, Constant, Language) and Every, Frame or Once.
    #[param]
    pub compute: Vec<PsObject>,
    /// The window's number; inside a handler, the handler's window when not given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl SetGuiGpuSurface {
    /// The draw this command's own parameters give.
    fn given(&self) -> PsResult<GivenDraw> {
        let present = |o: &Option<PsObject>| o.clone().filter(|v| !v.is_null());
        Ok(GivenDraw {
            shader: present(&self.shader),
            vertex: present(&self.vertex),
            vertex_count: self.vertex_count,
            instance_count: self.instance_count,
            index: present(&self.index),
            binding: present(&self.binding),
            blend_constant: self.blend_constant.clone(),
            stencil_reference: self.stencil_reference,
            options: PipelineOptions {
                depth: self.depth,
                depth_format: self.depth_format,
                depth_compare: self.depth_compare,
                depth_write: self.depth_write,
                depth_bias: self.depth_bias,
                depth_bias_slope_scale: self.depth_bias_slope_scale,
                depth_bias_clamp: self.depth_bias_clamp,
                stencil_compare: self.stencil_compare,
                stencil_fail: self.stencil_fail,
                stencil_depth_fail: self.stencil_depth_fail,
                stencil_pass: self.stencil_pass,
                stencil_read_mask: self.stencil_read_mask,
                stencil_write_mask: self.stencil_write_mask,
                back_stencil_compare: self.back_stencil_compare,
                back_stencil_fail: self.back_stencil_fail,
                back_stencil_depth_fail: self.back_stencil_depth_fail,
                back_stencil_pass: self.back_stencil_pass,
                topology: self.topology,
                cull_mode: self.cull_mode,
                blend: self.blend,
                blend_source: self.blend_source,
                blend_destination: self.blend_destination,
                blend_operation: self.blend_operation,
                alpha_blend_source: self.alpha_blend_source,
                alpha_blend_destination: self.alpha_blend_destination,
                alpha_blend_operation: self.alpha_blend_operation,
                sample_count: self.sample_count,
                vertex_entry: self.vertex_entry.clone(),
                fragment_entry: self.fragment_entry.clone(),
                constants: constants_of(self.constant.as_ref())?,
            },
        })
    }
}

impl Cmdlet for SetGuiGpuSurface {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let clear = clear_of(self.clear_color.as_deref())?;
        if let Some(depth) = self.clear_depth.filter(|d| !(0.0..=1.0).contains(d)) {
            return Err(binding_error("GuiGpuClearDepth", format!("-ClearDepth is a depth from 0 to 1, and {depth} is not")));
        }
        let listed = !self.draw.is_empty();
        let given = if listed {
            self.draw.iter().enumerate().map(|(at, table)| GivenDraw::of_table(table).map_err(|e| in_draw(at, e))).collect::<PsResult<Vec<GivenDraw>>>()?
        } else {
            let one = self.given()?;
            if one.shader.is_none() && one.any_but_shader() {
                return Err(binding_error("GuiGpuNoShader", String::from("the draw parameters describe a draw, which takes -Shader: WGSL text or a PwrsGui.GpuRenderPipeline")));
            }
            if one.shader.is_some() { vec![one] } else { Vec::new() }
        };
        let uniform = match self.uniform.as_ref().filter(|v| !v.is_null()) {
            Some(value) => wgpu_pwsh::host_bytes(value, None).map_err(|e| binding_error("GuiGpuUniform", format!("-Uniform takes an array of numbers: {}", e.message)))?.1,
            None => Vec::new(),
        };
        let asked = self.compute.iter().enumerate().map(|(at, table)| GivenPass::of_table(table).map_err(|e| in_pass(at, e))).collect::<PsResult<Vec<GivenPass>>>()?;
        let id = window::window_of(ps, self.window)?;
        let (open, node) = window::surface_named(id, &self.name)?;
        let session = session::of_window(id)?;
        listen(ps, &session)?;
        if Renderer::of_number(id) == Renderer::Html {
            let table = session::egui_session(ps)?;
            let mut held = Vec::new();
            let mut objects = Vec::new();
            let mut draws = Vec::with_capacity(given.len());
            for (at, one) in given.into_iter().enumerate() {
                let draw = page_draw(&table, one, &mut held, &mut objects);
                draws.push(if listed { draw.map_err(|e| in_draw(at, e))? } else { draw? });
            }
            let mut passes = Vec::with_capacity(asked.len());
            for (at, one) in asked.iter().enumerate() {
                passes.push(page_pass(&table, one, &mut held, &mut objects).map_err(|e| in_pass(at, e))?);
            }
            if !uniform.is_empty() && !draws.iter().any(|d| d.frame) && !passes.iter().any(|p| p.frame) {
                return Err(binding_error("GuiGpuUniform", String::from("-Uniform was given, and no pass or draw reads the frame uniform at @group(1) @binding(0)")));
            }
            for object in unmirrored(&session, &objects) {
                send_copy(ps, &table, &session, object)?;
            }
            let surface = PageSurface { clear, clear_depth: self.clear_depth, clear_stencil: self.clear_stencil, draws, passes, uniform };
            return match call(ps, &session, GpuOp::SetPageSurface { window: id, name: self.name.clone(), surface })? {
                GpuValue::Done => {
                    open.hold_surface(node, held);
                    for object in hold_mirrors(&session, (id, node), &objects) {
                        forget_copy(ps, &session, object)?;
                    }
                    Ok(())
                }
                other => Err(unexpected("a page's surface", &other)),
            };
        }
        let mut held = Vec::new();
        let mut draws = Vec::with_capacity(given.len());
        for (at, one) in given.into_iter().enumerate() {
            let draw = surface_draw(ps, &session, one, &mut held);
            draws.push(if listed { draw.map_err(|e| in_draw(at, e))? } else { draw? });
        }
        let mut passes = Vec::with_capacity(asked.len());
        for (at, one) in asked.iter().enumerate() {
            passes.push(one.pass(&session, &mut held).map_err(|e| in_pass(at, e))?);
        }
        let op = GpuOp::SetSurface { window: id, name: self.name.clone(), clear, clear_depth: self.clear_depth, clear_stencil: self.clear_stencil, draws, passes, uniform };
        match call(ps, &session, op)? {
            GpuValue::Done => {
                open.hold_surface(node, held);
                Ok(())
            }
            other => Err(unexpected("a surface's draws", &other)),
        }
    }
}

/// A table object a page's draw or pass uses, with what its copy on the
/// page is made from: a buffer's size in bytes, a texture's size and
/// format, a sampler's settings.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mirrored {
    Buffer { id: u64, length: u64 },
    Texture { id: u64, width: u32, height: u32, format: WireFormat },
    Sampler { id: u64, filter: SamplerFilter, address: WireAddress },
}

impl Mirrored {
    fn object(self) -> TableObject {
        match self {
            Mirrored::Buffer { id, .. } => TableObject::Buffer(id),
            Mirrored::Texture { id, .. } => TableObject::Texture(id),
            Mirrored::Sampler { id, .. } => TableObject::Sampler(id),
        }
    }
}

/// The table objects the HTML renderer of `session` has copies of: what
/// each copy is made from and how many of its surfaces use it, and which
/// each surface uses, by window and node.
#[derive(Clone)]
struct Mirrors {
    session: Session,
    holders: BTreeMap<TableObject, (Mirrored, u32)>,
    held: BTreeMap<(u64, u32), Vec<TableObject>>,
}

/// The copies the running HTML renderer has; another session's are
/// forgotten, as its renderer's end took them.
static MIRRORS: ArcSwapOption<Mirrors> = ArcSwapOption::const_empty();

/// The copies `now` records for the HTML renderer of `page`, or none when
/// it records another renderer's.
fn mirrors_now(now: &Option<Arc<Mirrors>>, page: &Session) -> Mirrors {
    match now {
        Some(m) if m.session.same(page) => Mirrors::clone(m),
        _ => Mirrors { session: page.clone(), holders: BTreeMap::new(), held: BTreeMap::new() },
    }
}

/// The objects of `objects`, once each, of which the HTML renderer of
/// `page` has no copy yet.
fn unmirrored(page: &Session, objects: &[Mirrored]) -> Vec<Mirrored> {
    let now = MIRRORS.load_full();
    let kept = now.as_ref().filter(|m| m.session.same(page));
    let mut out: Vec<Mirrored> = Vec::new();
    for o in objects {
        let have = kept.is_some_and(|m| m.holders.contains_key(&o.object()));
        if !have && !out.iter().any(|x| x.object() == o.object()) {
            out.push(*o);
        }
    }
    out
}

/// Records that the surface at `place`, a window and a node, uses
/// `objects` in place of what it used, and returns the objects no surface
/// of `page`'s renderer uses any more.
fn hold_mirrors(page: &Session, place: (u64, u32), objects: &[Mirrored]) -> Vec<TableObject> {
    let mut wanted: Vec<Mirrored> = Vec::new();
    for o in objects {
        if !wanted.iter().any(|w| w.object() == o.object()) {
            wanted.push(*o);
        }
    }
    let mut gone = Vec::new();
    MIRRORS.rcu(|now| {
        let mut m = mirrors_now(now, page);
        gone.clear();
        let named: Vec<TableObject> = wanted.iter().map(|w| w.object()).collect();
        let old = if named.is_empty() { m.held.remove(&place) } else { m.held.insert(place, named) };
        for w in &wanted {
            m.holders.entry(w.object()).or_insert((*w, 0)).1 += 1;
        }
        for o in old.into_iter().flatten() {
            let left = m.holders.get(&o).map_or(0, |(_, n)| n.saturating_sub(1));
            if left == 0 {
                m.holders.remove(&o);
                gone.push(o);
            } else if let Some(held) = m.holders.get_mut(&o) {
                held.1 = left;
            }
        }
        Some(Arc::new(m))
    });
    gone
}

/// The HTML renderer with a copy of `object`, and what the copy is made
/// from, when one has it.
fn mirroring(object: TableObject) -> Option<(Session, Mirrored)> {
    let now = MIRRORS.load_full()?;
    let (what, _) = now.holders.get(&object)?;
    Some((now.session.clone(), *what))
}

/// `op` run by `session`'s renderer with no stop to wait for, as a
/// thread of the module's own calls it.
fn quiet_call(session: &Session, op: GpuOp) -> PsResult<GpuValue> {
    session.gpu(op, |_: Canceler| Ok::<(), GuiError>(())).map_err(gui_error)?.map_err(failed)
}

/// Gives the HTML renderer of `page` `op`, a copy or a part of one, each
/// call made through `call`.
fn mirror_through(page: &Session, op: GpuOp, call: &dyn Fn(&Session, GpuOp) -> PsResult<GpuValue>) -> PsResult<()> {
    match call(page, op)? {
        GpuValue::Done => Ok(()),
        other => Err(unexpected("a copy for a page", &other)),
    }
}

/// Gives the HTML renderer of `page` a copy of `what`, read from the egui
/// renderer of `table`, each call made through `call`.
fn copy_through(table: &Session, page: &Session, what: Mirrored, call: &dyn Fn(&Session, GpuOp) -> PsResult<GpuValue>) -> PsResult<()> {
    let op = match what {
        Mirrored::Buffer { id, length } => match call(table, GpuOp::Read { buffer: id, offset: 0, length: None })? {
            GpuValue::Bytes(bytes) => GpuOp::MirrorBuffer { buffer: id, length, offset: 0, bytes },
            other => return Err(unexpected("a buffer's read for a page", &other)),
        },
        Mirrored::Texture { id, width, height, format } => match call(table, GpuOp::ReadTexture { texture: id })? {
            GpuValue::Bytes(texels) => GpuOp::MirrorTexture { texture: id, width, height, format, texels },
            other => return Err(unexpected("a texture's read for a page", &other)),
        },
        Mirrored::Sampler { id, filter, address } => GpuOp::MirrorSampler { sampler: id, filter, address },
    };
    mirror_through(page, op, call)
}

/// Gives the HTML renderer of `page` a copy of `what`, read from the egui
/// renderer of `table`.
fn send_copy(ps: &Pipeline<'_>, table: &Session, page: &Session, what: Mirrored) -> PsResult<()> {
    copy_through(table, page, what, &|s, op| call(ps, s, op))
}

/// Sends the HTML renderer with a copy of `object` the copy again, read
/// from the egui renderer of `table`, as it is after a write or a compute
/// run changed it; nothing when no renderer has one.
fn send_again(ps: &Pipeline<'_>, table: &Session, object: TableObject) -> PsResult<()> {
    match mirroring(object) {
        Some((page, what)) => send_copy(ps, table, &page, what),
        None => Ok(()),
    }
}

/// Tells the HTML renderer of `page` to let its copy of `object` go.
fn forget_copy(ps: &Pipeline<'_>, page: &Session, object: TableObject) -> PsResult<()> {
    match call(ps, page, GpuOp::ForgetMirror { object })? {
        GpuValue::Done => Ok(()),
        other => Err(unexpected("a copy let go", &other)),
    }
}

/// Lets go of what window `window`'s GpuSurfaces used in the HTML renderer
/// once the window has closed: the copies no other surface uses are
/// forgotten. A renderer that has ended took its copies with it.
pub(crate) fn release_window(window: u64) {
    let Some(now) = MIRRORS.load_full() else {
        return;
    };
    let places: Vec<(u64, u32)> = now.held.keys().filter(|(w, _)| *w == window).copied().collect();
    let mut gone = Vec::new();
    for place in places {
        gone.extend(hold_mirrors(&now.session, place, &[]));
    }
    for object in gone {
        match now.session.gpu(GpuOp::ForgetMirror { object }, |_: Canceler| Ok::<(), GuiError>(())) {
            Ok(Ok(GpuValue::Done)) | Err(GuiError::Gone(_)) => {}
            Ok(Ok(other)) => eprintln!("PWRS-GUI: the HTML renderer replied to the release of its copy of {object:?} with {other:?}"),
            Ok(Err(f)) => eprintln!("PWRS-GUI: the HTML renderer still had its copy of {object:?} after window {window} closed: {} ({})", f.text, f.id),
            Err(e) => eprintln!("PWRS-GUI: the HTML renderer was not told to let go of its copy of {object:?} after window {window} closed: {e}"),
        }
    }
}

/// Sends the HTML renderer every copy it has again, read from the egui
/// renderer of `table` on the device it went on with after losing one,
/// whose buffers and textures have what the session wrote into them rather
/// than what a shader wrote.
fn resend_copies(table: &Session) {
    let Some(now) = MIRRORS.load_full() else {
        return;
    };
    for (what, _) in now.holders.values() {
        if let Err(e) = copy_through(table, &now.session, *what, &quiet_call) {
            eprintln!("PWRS-GUI: the HTML renderer's copy of {:?} could not be sent again after the GPU device was lost: {} ({})", what.object(), e.message, e.error_id);
        }
    }
}

/// The values `value` gives a draw's vertex inputs or bindings, each with
/// the key it was given under: a hashtable's by key, or values in order,
/// an array of numbers being one value, or the one value given.
fn given_values(value: Option<&PsObject>) -> PsResult<Vec<(Option<String>, PsObject)>> {
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let type_name = value.type_name()?;
    if is_dictionary(&type_name) {
        let table = PsHashtable(value.clone());
        let mut out = Vec::new();
        for key in table.keys()? {
            let given = table.get(&key)?;
            out.push((Some(key), given));
        }
        return Ok(out);
    }
    let items = if is_list(&type_name) {
        let items = Vec::<PsObject>::from_ps(value)?;
        let numbers = match items.first() {
            Some(first) => is_number(&first.type_name()?),
            None => false,
        };
        if numbers { vec![value.clone()] } else { items }
    } else {
        vec![value.clone()]
    };
    Ok(items.into_iter().map(|v| (None, v)).collect())
}

/// The page resource `value` gives the slot `what` names, an array
/// converted to `element` where the shader declares one, with its size in
/// bytes and elements where it has one: a buffer, texture or sampler of
/// `table`, which goes into `held` and `objects`, or an array's bytes. `id`
/// is the error a value of another kind is refused with.
fn page_resource(value: &PsObject, table: &Session, element: Option<ElementType>, what: &str, id: &str, held: &mut Vec<PsObject>, objects: &mut Vec<Mirrored>) -> PsResult<(PageResource, Option<(u64, u64)>)> {
    let type_name = value.type_name()?;
    let (object, size) = match type_name.as_str() {
        "PwrsGui.GpuBuffer" => {
            let proxy = PsProxy::<GpuBuffer>::from_ps(value)?;
            let number = buffer_in(&proxy, table)?;
            let (length, size, own) = proxy.with(|b| (b.length, b.size, b.element()))?;
            (Mirrored::Buffer { id: number, length: size }, Some((length * own.size(), length)))
        }
        "PwrsGui.GpuTexture" => {
            let proxy = PsProxy::<GpuTexture>::from_ps(value)?;
            let number = texture_in(&proxy, table)?;
            let (width, height, format) = proxy.with(|t| (t.width, t.height, t.format))?;
            (Mirrored::Texture { id: number, width, height, format: format.wire() }, None)
        }
        "PwrsGui.GpuSampler" => {
            let proxy = PsProxy::<GpuSampler>::from_ps(value)?;
            let number = sampler_in(&proxy, table)?;
            let (filter, address) = proxy.with(|s| (s.filter, s.address_mode))?;
            (Mirrored::Sampler { id: number, filter: filter.wire(), address: address.wire() }, None)
        }
        _ => {
            let takes = "a PwrsGui.GpuBuffer, GpuTexture or GpuSampler, or an array of numbers";
            if is_dictionary(&type_name) {
                return Err(binding_error(id, format!("{what} takes {takes}, not a hashtable")));
            }
            let (own, bytes) = wgpu_pwsh::host_bytes(value, element).map_err(|e| binding_error(id, format!("{what} takes {takes}, and was given a {type_name}: {}", e.message)))?;
            if bytes.is_empty() {
                return Err(binding_error("GuiGpuEmpty", format!("the array for {what} is empty")));
            }
            let length = bytes.len() as u64;
            return Ok((PageResource::Bytes(bytes), Some((length, length / own.size()))));
        }
    };
    held.push(value.clone());
    objects.push(object);
    Ok((PageResource::Table(object.object()), size))
}

/// The WebGPU vertex format of an input of `components` values of
/// `element`.
fn vertex_format(element: ElementType, components: u32) -> PsResult<String> {
    let base = match element {
        ElementType::Float32 => "float32",
        ElementType::Float16 => "float16",
        ElementType::Int32 => "sint32",
        ElementType::UInt32 => "uint32",
        other => return Err(binding_error("GuiGpuVertexInput", format!("a vertex input of {other:?} has no vertex format a page reads"))),
    };
    Ok(if components == 1 { base.to_string() } else { format!("{base}x{components}") })
}

/// A draw's vertex inputs as a page feeds them, from `value` as -Vertex
/// gives it, each input's values converted to the type the shader declares,
/// in location order; and the vertices the first input has values for.
fn page_vertices(value: Option<&PsObject>, inputs: &[wgpu_pwsh::VertexInputInfo], table: &Session, held: &mut Vec<PsObject>, objects: &mut Vec<Mirrored>) -> PsResult<(Vec<PageVertex>, Option<u64>)> {
    let mut sorted: Vec<&wgpu_pwsh::VertexInputInfo> = inputs.iter().collect();
    sorted.sort_by_key(|i| i.location);
    let names = || sorted.iter().map(|i| format!("{} ({})", i.name, i.location)).collect::<Vec<_>>().join(", ");
    let given = given_values(value)?;
    let count = given.len();
    let mut out: Vec<PageVertex> = Vec::new();
    let mut first = None;
    for (at, (key, v)) in given.into_iter().enumerate() {
        let input = match &key {
            Some(k) => *sorted.iter().find(|i| i.name.eq_ignore_ascii_case(k)).ok_or_else(|| binding_error("GuiGpuVertexInput", format!("the shader has no vertex input named {k}; it has {}", names())))?,
            None => *sorted.get(at).ok_or_else(|| binding_error("GuiGpuVertexInput", format!("{count} values were given, and the shader has {} vertex inputs: {}", sorted.len(), names())))?,
        };
        if out.iter().any(|o| o.location == input.location) {
            return Err(binding_error("GuiGpuVertexInput", format!("vertex input {} was given twice", input.name)));
        }
        let stride = u64::from(input.components) * input.element_type.size();
        let (resource, size) = page_resource(&v, table, Some(input.element_type), &format!("Vertex {}", input.name), "GuiGpuVertex", held, objects)?;
        if matches!(resource, PageResource::Table(TableObject::Texture(_) | TableObject::Sampler(_))) {
            return Err(binding_error("GuiGpuVertex", String::from("a vertex input takes a buffer or an array; a texture or a sampler goes to a binding")));
        }
        if sorted.first().is_some_and(|f| f.location == input.location) {
            first = size.map(|(bytes, _)| bytes / stride.max(1));
        }
        out.push(PageVertex { location: input.location, format: vertex_format(input.element_type, input.components)?, stride, resource });
    }
    if out.len() != sorted.len() {
        return Err(binding_error("GuiGpuVertexInput", format!("{} vertex inputs were given, and the shader has {}: {}", out.len(), sorted.len(), names())));
    }
    out.sort_by_key(|v| v.location);
    Ok((out, first))
}

/// A draw's or pass's bindings as a page binds them, from `value` as
/// -Binding gives it, by name, `group:binding` or in the order the shaders
/// declare them, an array converted to the type the shader declares; every
/// binding but the frame uniform's at `@group(1) @binding(0)` takes one.
fn page_bindings(value: Option<&PsObject>, declared: &[wgpu_pwsh::BindingInfo], table: &Session, held: &mut Vec<PsObject>, objects: &mut Vec<Mirrored>) -> PsResult<Vec<PageBinding>> {
    let mut sorted: Vec<&wgpu_pwsh::BindingInfo> = declared.iter().collect();
    sorted.sort_by_key(|b| (b.group, b.binding));
    let given = given_values(value)?;
    let count = given.len();
    let mut out: Vec<PageBinding> = Vec::new();
    for (at, (key, v)) in given.into_iter().enumerate() {
        let info = match &key {
            Some(k) => find(declared, k)?,
            None => *sorted.get(at).ok_or_else(|| binding_error("GuiGpuBinding", format!("{count} values were given, and the shaders declare {} bindings: {}", sorted.len(), binding_list(declared))))?,
        };
        if (info.group, info.binding) == (1, 0) {
            return Err(binding_error("GuiGpuBinding", format!("{} at @group(1) @binding(0) is the frame uniform, which the surface fills; it takes no value", info.name)));
        }
        if out.iter().any(|b| (b.group, b.binding) == (info.group, info.binding)) {
            return Err(binding_error("GuiGpuBinding", format!("binding {} ({}:{}) was given twice", info.name, info.group, info.binding)));
        }
        let (resource, _) = page_resource(&v, table, info.element_type, &format!("Binding {}", info.name), "GuiGpuBinding", held, objects)?;
        out.push(PageBinding { group: info.group, binding: info.binding, resource });
    }
    let missing: Vec<String> = sorted
        .iter()
        .filter(|b| (b.group, b.binding) != (1, 0) && !out.iter().any(|o| (o.group, o.binding) == (b.group, b.binding)))
        .map(|b| format!("{} ({}:{})", b.name, b.group, b.binding))
        .collect();
    if !missing.is_empty() {
        return Err(binding_error("GuiGpuBinding", format!("the shaders read {}, and no value was given for it", missing.join(", "))));
    }
    out.sort_by_key(|b| (b.group, b.binding));
    Ok(out)
}

/// A draw's index buffer for a page, the type of its indices and how many
/// it draws, as [`index_of`] reads one: a buffer of `table`, or an array
/// sent with the draw.
fn page_index(value: &PsObject, table: &Session, held: &mut Vec<PsObject>, objects: &mut Vec<Mirrored>) -> PsResult<(PageResource, IndexFormat, u32)> {
    let count = |length: u64| u32::try_from(length).map_err(|e| binding_error("GuiGpuIndexCount", format!("{length} indices are more than a draw takes: {e}")));
    if value.type_name()? == "PwrsGui.GpuBuffer" {
        let proxy = PsProxy::<GpuBuffer>::from_ps(value)?;
        let (element, length, size) = proxy.with(|b| (b.element_type, b.length, b.size))?;
        let format = match element {
            GuiElementType::UInt16 => IndexFormat::Uint16,
            GuiElementType::UInt32 => IndexFormat::Uint32,
            other => return Err(binding_error("GuiGpuIndexType", format!("an index buffer has UInt16 or UInt32 elements, not {other:?}"))),
        };
        let number = buffer_in(&proxy, table)?;
        held.push(value.clone());
        objects.push(Mirrored::Buffer { id: number, length: size });
        return Ok((PageResource::Table(TableObject::Buffer(number)), format, count(length)?));
    }
    let (own, bytes) = wgpu_pwsh::host_bytes(value, None)?;
    let (bytes, format, size) = match own {
        ElementType::UInt16 => (bytes, IndexFormat::Uint16, 2),
        _ => (wgpu_pwsh::host_bytes(value, Some(ElementType::UInt32))?.1, IndexFormat::Uint32, 4),
    };
    let length = bytes.len() as u64 / size;
    Ok((PageResource::Bytes(bytes), format, count(length)?))
}

/// A draw as an HTML page makes it: its pipeline's spec, from a pipeline
/// object or built from WGSL text with the draw's own parameters, and its
/// vertex inputs, bindings, use of the frame uniform, index and counts
/// resolved against the shader, which the page does not read. Table
/// objects of `table` go into `held` and `objects`. A spec's depth and
/// stencil tests and its blend, and a stencil reference no test reads, are
/// refused as the Wgpu module refuses them in an egui window.
fn page_draw(table: &Session, given: GivenDraw, held: &mut Vec<PsObject>, objects: &mut Vec<Mirrored>) -> PsResult<PageDraw> {
    let Some(shader) = given.shader.clone().filter(|s| !s.is_null()) else {
        return Err(binding_error("GuiGpuNoShader", String::from("a draw takes a shader: WGSL text or a PwrsGui.GpuRenderPipeline")));
    };
    let spec = match shader.type_name()?.as_str() {
        "PwrsGui.GpuRenderPipeline" => {
            if given.options.any() {
                let text = "a PwrsGui.GpuRenderPipeline draws as New-GuiGpuRenderPipeline built it, so the pipeline parameters go to that command; a draw takes them to build a pipeline of its own from WGSL text";
                return Err(binding_error("GuiGpuPipelineOptions", String::from(text)));
            }
            let proxy = PsProxy::<GpuRenderPipeline>::from_ps(&shader)?;
            pipeline_in(&proxy, table)?;
            let spec = proxy.with(|p| p.spec.clone())?;
            held.push(shader);
            spec
        }
        "System.String" => given.options.spec(wgsl_of(&String::from_ps(&shader)?)?),
        other => return Err(binding_error("GuiGpuNoShader", format!("a draw's shader is WGSL text or a PwrsGui.GpuRenderPipeline, not a {other}"))),
    };
    if let Some(refusal) = spec.build_refusal().or_else(|| spec.stencil_reference_refusal(given.stencil_reference)) {
        return Err(failed(refusal));
    }
    let inputs = wgpu_pwsh::render_vertex_input_info(&spec.source, spec.vertex_entry.as_deref())?;
    let declared = wgpu_pwsh::render_binding_info(&spec.source, spec.vertex_entry.as_deref(), spec.fragment_entry.as_deref())?;
    let frame = declared.iter().any(|b| (b.group, b.binding) == (1, 0));
    let (vertices, first) = page_vertices(given.vertex.as_ref(), &inputs, table, held, objects)?;
    let bindings = page_bindings(given.binding.as_ref(), &declared, table, held, objects)?;
    let index = match given.index.as_ref().filter(|i| !i.is_null()) {
        Some(value) => Some(page_index(value, table, held, objects)?),
        None => None,
    };
    let vertex_count = match (given.vertex_count, &index, first) {
        (Some(n), _, _) => n,
        (None, Some((_, _, n)), _) => *n,
        (None, None, Some(n)) => u32::try_from(n).map_err(|e| binding_error("GuiGpuVertexCount", format!("the first vertex input has values for {n} vertices, more than a draw takes: {e}")))?,
        (None, None, None) => return Err(binding_error("GuiGpuVertexCount", String::from("the draw gives no vertex count, no index buffer and no values for a first vertex input; give -VertexCount"))),
    };
    let blend_constant = match given.blend_constant.as_deref() {
        None => None,
        Some([r, g, b, a]) => Some([*r, *g, *b, *a]),
        Some(other) => return Err(binding_error("GuiGpuBlendConstant", format!("a blend constant is four numbers, red, green, blue and alpha; {} were given", other.len()))),
    };
    Ok(PageDraw {
        spec,
        vertices,
        bindings,
        frame,
        index: index.map(|(r, f, _)| (r, f)),
        vertex_count,
        instance_count: given.instance_count.unwrap_or(1),
        blend_constant,
        stencil_reference: given.stencil_reference,
    })
}

/// The name and workgroup size of compute entry point `entry` of the WGSL
/// `source`, or of its only one when `None`.
fn compute_entry(source: &str, entry: Option<&str>) -> PsResult<(String, [u32; 3])> {
    let module = naga::front::wgsl::parse_str(source).map_err(|e| binding_error("GuiGpuEntry", format!("the pass's shader could not be read for its entry point: {e}")))?;
    let points: Vec<&naga::EntryPoint> = module.entry_points.iter().filter(|p| p.stage == naga::ShaderStage::Compute).collect();
    let found = match entry {
        Some(name) => points.iter().find(|p| p.name == name).copied(),
        None if points.len() == 1 => points.first().copied(),
        None => None,
    };
    let Some(point) = found else {
        let names = points.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", ");
        let text = match entry {
            Some(name) => format!("the shader has no compute entry point {name}; it has {names}"),
            None => format!("the shader has {} compute entry points, {names}; name one with Entry", points.len()),
        };
        return Err(binding_error("GuiGpuEntry", text));
    };
    Ok((point.name.clone(), point.workgroup_size))
}

/// The workgroups that run `invocations` in x, y and z, rounded up, for a
/// workgroup of `size`.
fn workgroups_for(invocations: [u64; 3], size: [u32; 3]) -> PsResult<[u32; 3]> {
    let mut out = [1u32; 3];
    for (at, axis) in ["x", "y", "z"].iter().enumerate() {
        let n = invocations[at].div_ceil(u64::from(size[at].max(1)));
        out[at] = u32::try_from(n).map_err(|e| binding_error("GuiGpuDispatchSize", format!("{} invocations in {axis} are more workgroups than a dispatch takes: {e}", invocations[at])))?;
    }
    Ok(out)
}

/// A compute pass as an HTML page runs it on WebGPU: its WGSL, entry point,
/// bindings and use of the frame uniform, and its workgroups worked out
/// from -Invocations or the first binding's elements, which the page
/// cannot read from the shader.
fn page_pass(table: &Session, given: &GivenPass, held: &mut Vec<PsObject>, objects: &mut Vec<Mirrored>) -> PsResult<PagePass> {
    let (source, language, declared) = given.compute.declared()?;
    if language == GuiShaderLanguage::Glsl {
        return Err(binding_error("GuiGpuShaderLanguage", String::from("an HTML window's page runs compute passes of WGSL; give this pass's shader as WGSL")));
    }
    let (entry, size) = compute_entry(source, given.compute.entry.as_deref())?;
    let frame = declared.iter().any(|b| (b.group, b.binding) == (1, 0));
    let mut bindings = Vec::new();
    let mut first = None;
    for (info, value) in match_resources(&declared, given.compute.binding.as_ref())? {
        if (info.group, info.binding) == (1, 0) {
            return Err(binding_error("GuiGpuBinding", format!("{} at @group(1) @binding(0) is the frame uniform, which the surface fills; it takes no value", info.name)));
        }
        let (resource, elements) = page_resource(&value, table, info.element_type, &format!("Binding {}", info.name), "GuiGpuBinding", held, objects)?;
        if first.is_none() {
            first = elements.map(|(_, n)| n);
        }
        bindings.push(PageBinding { group: info.group, binding: info.binding, resource });
    }
    let (workgroups, indirect) = match given.compute.dispatch(table)? {
        GpuDispatch::Workgroups(w) => (w, None),
        GpuDispatch::Invocations(n) => (workgroups_for(n, size)?, None),
        GpuDispatch::Indirect { buffer, offset } => {
            if let Some(object) = &given.indirect {
                let length = PsProxy::<GpuBuffer>::from_ps(object)?.with(|b| b.size)?;
                objects.push(Mirrored::Buffer { id: buffer, length });
                held.push(object.clone());
            }
            ([0, 0, 0], Some((buffer, offset)))
        }
        GpuDispatch::Auto => match first {
            Some(n) => (workgroups_for([n, 1, 1], size)?, None),
            None => return Err(binding_error("GuiGpuDispatchSize", String::from("the pass binds no buffer to count invocations from; give Workgroups or Invocations"))),
        },
    };
    let constants = constants_of(given.compute.constant.as_ref())?;
    Ok(PagePass { source: source.to_string(), entry, constants, bindings, frame, workgroups, indirect, every: given.every })
}

/// `rgba`, `width` by `height` texels of red, green, blue and alpha bytes in
/// rows from the top, as a PNG file's bytes.
fn png_of(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// Writes a GpuSurface to a PNG file as a frame of its window drawn after
/// the command was given paints it, so the draws Set-GuiGpuSurface gave
/// before are in it, at the surface's size in pixels; a file already at the
/// path is replaced. Inside a handler it acts on the handler's window;
/// elsewhere -Window names the window.
///
/// # Examples
/// Save-GuiGpuSurface View .\frame.png
/// Save-GuiGpuSurface -Window $w.Id -Name View -Path "$env:TEMP\view.png"
#[cmdlet(verb = "Save", noun = "GuiGpuSurface")]
#[derive(Default)]
pub struct SaveGuiGpuSurface {
    /// The surface's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The PNG file to write, from the current location when relative.
    #[param(mandatory, position = 1)]
    pub path: String,
    /// The window's number; inside a handler, the handler's window when not
    /// given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl Cmdlet for SaveGuiGpuSurface {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let id = window::window_of(ps, self.window)?;
        window::surface_named(id, &self.name)?;
        let path = full_path(ps, &self.path)?;
        if !std::path::Path::new(&path).extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) {
            return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiGpuSaveFormat", format!("Save-GuiGpuSurface writes a PNG, and {path} does not end in .png")).terminating());
        }
        let session = session::of_window(id)?;
        listen(ps, &session)?;
        let (width, height, rgba) = match call(ps, &session, GpuOp::SaveSurface { window: id, name: self.name.clone() })? {
            GpuValue::Texels { width, height, rgba } => (width, height, rgba),
            other => return Err(unexpected("a surface's texels", &other)),
        };
        let png = png_of(width, height, &rgba).map_err(|e| PsError::new(ErrorCategory::InvalidData, "GuiGpuPng", format!("the surface's {width} by {height} texels could not be written as a PNG: {e}")).terminating())?;
        std::fs::write(&path, png).map_err(|e| PsError::new(ErrorCategory::WriteError, "GuiGpuSave", format!("{path} could not be written: {e}")).terminating())
    }
}

/// The controls a person moves a GpuSurface's camera with.
#[psenum(name = "PwrsGui.CameraControls")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiCameraControls {
    /// three.js OrbitControls': the left button orbits, or pans with Shift or Ctrl; the right button pans, or orbits with Shift or Ctrl; the middle button and the wheel zoom.
    #[default]
    Default,
    /// Blender's: the middle button orbits, pans with Shift and zooms with Ctrl; the wheel zooms.
    Blender,
    /// The gestures -Bindings names.
    Custom,
    /// No gesture: the camera moves only when a script sets it.
    None,
}

/// What Get-GuiGpuCamera and Set-GuiGpuCamera -PassThru write: a
/// GpuSurface's camera where the person has moved it.
#[psclass(name = "PwrsGui.GpuCamera", columns = ["Name", "Target", "Distance", "Yaw", "Pitch", "Controls"])]
#[derive(Default, Clone)]
pub struct GpuCamera {
    /// The surface's name.
    pub name: String,
    /// The window's number.
    pub window: u64,
    /// The point the camera looks at and orbits, as x, y and z.
    pub target: Vec<f64>,
    /// How far the camera is from the target.
    pub distance: f64,
    /// Degrees around +Y from +Z toward +X; at 0 the camera is on the target's +Z side.
    pub yaw: f64,
    /// Degrees the camera is raised above the target's level, from -90, under it, to 90, over it.
    pub pitch: f64,
    /// The vertical field of view in degrees.
    pub field_of_view: f64,
    /// The distance from the camera at which the scene starts to be drawn.
    pub near: f64,
    /// The distance from the camera beyond which the scene is not drawn.
    pub far: f64,
    /// Where the camera is, as x, y and z.
    pub eye: Vec<f64>,
    /// The controls that move it.
    pub controls: GuiCameraControls,
    /// Each gesture the controls take and the move it makes, in the order they are matched, as 'Shift+Left' = 'Pan'.
    pub bindings: PsObject,
    /// The view matrix in the frame uniform at byte 32, column by column.
    pub view: Vec<f32>,
}

impl GpuCamera {
    /// `spec` as the camera of the GpuSurface named `name` in window
    /// `window`, its bindings in an ordered dictionary whose keys are
    /// compared without regard to case, as `[ordered]` makes one.
    fn of(name: &str, window: u64, spec: &CameraSpec) -> PsResult<GpuCamera> {
        let comparer = PsType::from_name("System.StringComparer").call_static("get_OrdinalIgnoreCase", &[])?;
        let bindings = PsHashtable(PsType::from_name("System.Collections.Specialized.OrderedDictionary").new(&[comparer])?);
        for (gesture, made) in spec.controls.bindings() {
            bindings.set(&gesture_text(gesture), move_text(made).into_ps()?)?;
        }
        let controls = match spec.controls {
            CameraControls::Default => GuiCameraControls::Default,
            CameraControls::Blender => GuiCameraControls::Blender,
            CameraControls::Custom(_) => GuiCameraControls::Custom,
            CameraControls::Fixed => GuiCameraControls::None,
        };
        Ok(GpuCamera {
            name: name.to_string(),
            window,
            target: spec.target.to_vec(),
            distance: spec.distance,
            yaw: spec.yaw,
            pitch: spec.pitch,
            field_of_view: spec.field_of_view,
            near: spec.near,
            far: spec.far,
            eye: spec.eye().to_vec(),
            controls,
            bindings: bindings.0,
            view: spec.view().iter().flatten().copied().collect(),
        })
    }
}

/// The gesture `text` names: Left, Right, Middle or Wheel after any of
/// Shift, Ctrl and Alt, joined by + and compared without regard to case;
/// Control, Command and Cmd are Ctrl, and Option is Alt.
fn gesture_of(text: &str) -> Result<CameraGesture, String> {
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let Some((last, keys)) = parts.split_last() else {
        return Err(format!("'{text}' names no button or wheel"));
    };
    let mut modifiers = CameraModifiers::NONE;
    for key in keys {
        match key.to_ascii_lowercase().as_str() {
            "shift" => modifiers.shift = true,
            "ctrl" | "control" | "command" | "cmd" => modifiers.ctrl = true,
            "alt" | "option" => modifiers.alt = true,
            _ => return Err(format!("'{text}': {key} is not a modifier key; a gesture's modifiers are Shift, Ctrl and Alt")),
        }
    }
    let drag = |button| Ok(CameraGesture::Drag { button, modifiers });
    match last.to_ascii_lowercase().as_str() {
        "left" => drag(CameraButton::Left),
        "right" => drag(CameraButton::Right),
        "middle" => drag(CameraButton::Middle),
        "wheel" => Ok(CameraGesture::Wheel { modifiers }),
        _ => Err(format!("'{text}' ends in {last}; a gesture ends in Left, Right, Middle or Wheel")),
    }
}

/// `gesture` as [`gesture_of`] reads it, its modifier keys first.
fn gesture_text(gesture: CameraGesture) -> String {
    let (modifiers, last) = match gesture {
        CameraGesture::Drag { button, modifiers } => (modifiers, match button {
            CameraButton::Left => "Left",
            CameraButton::Right => "Right",
            CameraButton::Middle => "Middle",
        }),
        CameraGesture::Wheel { modifiers } => (modifiers, "Wheel"),
    };
    let held = [(modifiers.shift, "Shift"), (modifiers.ctrl, "Ctrl"), (modifiers.alt, "Alt")];
    let mut parts: Vec<&str> = held.iter().filter(|(on, _)| *on).map(|(_, name)| *name).collect();
    parts.push(last);
    parts.join("+")
}

/// The move `text` names, without regard to case.
fn move_of(text: &str) -> Result<CameraMove, String> {
    match text.trim().to_ascii_lowercase().as_str() {
        "orbit" => Ok(CameraMove::Orbit),
        "pan" => Ok(CameraMove::Pan),
        "zoom" => Ok(CameraMove::Zoom),
        _ => Err(format!("'{text}' is not a camera move; a gesture makes Orbit, Pan or Zoom")),
    }
}

fn move_text(made: CameraMove) -> &'static str {
    match made {
        CameraMove::Orbit => "Orbit",
        CameraMove::Pan => "Pan",
        CameraMove::Zoom => "Zoom",
    }
}

/// The custom controls `table` gives, each key a gesture and each value the
/// move it makes, in order of the gestures' names; two keys naming one
/// gesture are refused, so no match depends on the order.
fn bindings_of(table: &PsHashtable) -> PsResult<Vec<(CameraGesture, CameraMove)>> {
    let mut named: Vec<(CameraGesture, CameraMove, String)> = Vec::new();
    for key in table.keys()? {
        let value = String::from_ps(&table.get(&key)?)?;
        let gesture = gesture_of(&key).map_err(|e| binding_error("GuiGpuCamera", e))?;
        let made = move_of(&value).map_err(|e| binding_error("GuiGpuCamera", format!("{key}: {e}")))?;
        if let Some((_, _, first)) = named.iter().find(|(g, _, _)| *g == gesture) {
            return Err(binding_error("GuiGpuCamera", format!("{first} and {key} name one gesture")));
        }
        named.push((gesture, made, key));
    }
    if named.is_empty() {
        return Err(binding_error("GuiGpuCamera", String::from("-Bindings names no gesture; -Controls None leaves the camera to the script")));
    }
    named.sort_by_key(|(gesture, _, _)| gesture_text(*gesture));
    Ok(named.into_iter().map(|(gesture, made, _)| (gesture, made)).collect())
}

/// Sets the camera of a GpuSurface: where it looks from, its projection,
/// and the controls the person moves it with. The camera's view and
/// projection fill the surface's frame uniform at bytes 32 and 96, which
/// draws read at @group(1) @binding(0). A surface without a camera starts
/// from the default one: 5 units out along +Z from the origin, a 50 degree
/// field of view, near 0.1 and far 2000, and the Default controls. A
/// parameter left out leaves what the camera has, so -Controls switches the
/// controls without moving the camera.
///
/// The Default controls are three.js OrbitControls': the left button
/// orbits, or pans with Shift or Ctrl; the right button pans, or orbits
/// with Shift or Ctrl; the middle button and the wheel zoom. Blender's: the
/// middle button orbits, pans with Shift and zooms with Ctrl; the wheel
/// zooms. -Bindings gives custom controls, a hashtable of gestures such as
/// 'Left', 'Shift+Middle' or 'Ctrl+Wheel' to the moves they make: Orbit,
/// Pan or Zoom. None leaves the camera to the script.
///
/// Inside a handler it acts on the handler's window; elsewhere -Window
/// names the window.
///
/// # Examples
/// Set-GuiGpuCamera View -Distance 8 -Pitch 30
/// Set-GuiGpuCamera View -Controls Blender
/// Set-GuiGpuCamera View -Bindings @{ Left = 'Orbit'; 'Shift+Left' = 'Pan'; Wheel = 'Zoom' }
#[cmdlet(verb = "Set", noun = "GuiGpuCamera")]
#[derive(Default)]
pub struct SetGuiGpuCamera {
    /// The surface's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The point the camera looks at and orbits, as x, y and z.
    #[param]
    pub target: Option<Vec<f64>>,
    /// How far the camera is from the target, greater than 0.
    #[param]
    pub distance: Option<f64>,
    /// Degrees around +Y from +Z toward +X; at 0 the camera is on the target's +Z side.
    #[param]
    pub yaw: Option<f64>,
    /// Degrees the camera is raised above the target's level, from -90, under it, to 90, over it.
    #[param]
    pub pitch: Option<f64>,
    /// The vertical field of view in degrees, between 0 and 180.
    #[param]
    pub field_of_view: Option<f64>,
    /// The distance from the camera at which the scene starts to be drawn, greater than 0.
    #[param]
    pub near: Option<f64>,
    /// The distance from the camera beyond which the scene is not drawn, greater than the near distance.
    #[param]
    pub far: Option<f64>,
    /// The controls the person moves the camera with: Default, Blender, Custom with -Bindings, or None.
    #[param]
    pub controls: Option<GuiCameraControls>,
    /// Custom controls: a hashtable of gestures, such as 'Shift+Left', to the moves they make, Orbit, Pan or Zoom.
    #[param]
    pub bindings: Option<PsHashtable>,
    /// Writes the camera as it now is.
    #[param]
    pub pass_thru: bool,
    /// The window's number; inside a handler, the handler's window when not given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl SetGuiGpuCamera {
    /// The change this command's parameters make to a camera.
    fn change(&self) -> PsResult<CameraChange> {
        let target = match self.target.as_deref() {
            None => None,
            Some([x, y, z]) => Some([*x, *y, *z]),
            Some(other) => return Err(binding_error("GuiGpuCamera", format!("-Target takes three numbers, x, y and z, and was given {}", other.len()))),
        };
        let controls = match (self.controls, &self.bindings) {
            (None | Some(GuiCameraControls::Custom), Some(table)) => Some(CameraControls::Custom(bindings_of(table)?)),
            (Some(GuiCameraControls::Custom), None) => return Err(binding_error("GuiGpuCamera", String::from("custom controls are the gestures -Bindings gives"))),
            (Some(other), Some(_)) => return Err(binding_error("GuiGpuCamera", format!("-Bindings gives custom controls, and -Controls {other:?} names others"))),
            (Some(GuiCameraControls::Default), None) => Some(CameraControls::Default),
            (Some(GuiCameraControls::Blender), None) => Some(CameraControls::Blender),
            (Some(GuiCameraControls::None), None) => Some(CameraControls::Fixed),
            (None, None) => None,
        };
        Ok(CameraChange { target, distance: self.distance, yaw: self.yaw, pitch: self.pitch, field_of_view: self.field_of_view, near: self.near, far: self.far, controls })
    }
}

impl Cmdlet for SetGuiGpuCamera {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let change = self.change()?;
        let id = window::window_of(ps, self.window)?;
        window::surface_named(id, &self.name)?;
        let session = session::of_window(id)?;
        listen(ps, &session)?;
        match call(ps, &session, GpuOp::SetCamera { window: id, name: self.name.clone(), change })? {
            GpuValue::Camera(Some(spec)) if self.pass_thru => ps.write(GpuCamera::of(&self.name, id, &spec)?),
            GpuValue::Camera(Some(_)) => Ok(()),
            other => Err(unexpected("a surface's camera", &other)),
        }
    }
}

/// Writes the camera of a GpuSurface where the person has moved it: its
/// target, distance and angles, its projection, its controls with the
/// gestures they take, where the camera is, and its view matrix. A surface
/// without a camera writes nothing. Inside a handler it acts on the
/// handler's window; elsewhere -Window names the window.
///
/// # Examples
/// Get-GuiGpuCamera View
/// (Get-GuiGpuCamera View -Window $w.Id).Bindings
#[cmdlet(verb = "Get", noun = "GuiGpuCamera")]
#[derive(Default)]
pub struct GetGuiGpuCamera {
    /// The surface's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The window's number; inside a handler, the handler's window when not given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl Cmdlet for GetGuiGpuCamera {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let id = window::window_of(ps, self.window)?;
        window::surface_named(id, &self.name)?;
        let session = session::of_window(id)?;
        listen(ps, &session)?;
        match call(ps, &session, GpuOp::GetCamera { window: id, name: self.name.clone() })? {
            GpuValue::Camera(Some(spec)) => ps.write(GpuCamera::of(&self.name, id, &spec)?),
            GpuValue::Camera(None) => Ok(()),
            other => Err(unexpected("a surface's camera", &other)),
        }
    }
}

/// Takes the camera from a GpuSurface, so its frame uniform's view and
/// projection are identity matrices again and the person's pointer moves
/// nothing. A surface without a camera is left as it is. Inside a handler
/// it acts on the handler's window; elsewhere -Window names the window.
///
/// # Examples
/// Remove-GuiGpuCamera View
#[cmdlet(verb = "Remove", noun = "GuiGpuCamera", supports_should_process)]
#[derive(Default)]
pub struct RemoveGuiGpuCamera {
    /// The surface's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The window's number; inside a handler, the handler's window when not given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl Cmdlet for RemoveGuiGpuCamera {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let id = window::window_of(ps, self.window)?;
        window::surface_named(id, &self.name)?;
        if !ps.should_process(&format!("GpuSurface {} in window {id}", self.name), "Remove the camera")? {
            return Ok(());
        }
        let session = session::of_window(id)?;
        listen(ps, &session)?;
        match call(ps, &session, GpuOp::RemoveCamera { window: id, name: self.name.clone() })? {
            GpuValue::Done => Ok(()),
            other => Err(unexpected("a surface's camera", &other)),
        }
    }
}

/// The controls a person moves a Canvas's view with.
#[psenum(name = "PwrsGui.CanvasViewControls")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiCanvasViewControls {
    /// Leaflet's and Google Maps': a left drag where no named item takes it pans and glides on, Shift and a left drag zooms to the box it draws, the wheel and a pinch zoom, a double-click zooms in a step and with Shift out, the arrows pan, and + and = zoom in a step and - out.
    #[default]
    Map,
    /// Figma's and Inkscape's: the wheel scrolls, Shift and the wheel scroll sideways, Ctrl and the wheel or a pinch zoom, a middle drag or a left drag with the space bar down pans, + and = zoom in a step and - out, Shift+1 fits every item, Shift+0 returns to zoom 1, and Ctrl and the arrows pan.
    Design,
    /// The gestures and keys -Bindings names.
    Custom,
    /// No gesture: the view moves only when a script sets it.
    None,
}

/// What Get-GuiCanvasView and Set-GuiCanvasView -PassThru write: a
/// Canvas's view as it is drawn.
#[psclass(name = "PwrsGui.CanvasView", columns = ["Name", "Center", "Zoom", "Controls", "Moving"])]
#[derive(Default, Clone)]
pub struct CanvasView {
    /// The canvas's name.
    pub name: String,
    /// The window's number.
    pub window: u64,
    /// The point of the items' coordinates at the canvas's middle, as x and y; null while the canvas has not been drawn and its view stays at the canvas's own middle.
    pub center: Option<Vec<f64>>,
    /// How many points on the canvas one unit of the items' coordinates takes.
    pub zoom: f64,
    /// The smallest zoom the person may go to.
    pub min_zoom: f64,
    /// The largest zoom the person may go to.
    pub max_zoom: f64,
    /// The controls that move it.
    pub controls: GuiCanvasViewControls,
    /// Each gesture and key the controls take and the move it makes, in the order they are matched, as 'Shift+Left' = 'BoxZoom'.
    pub bindings: PsObject,
    /// Whether a step, a glide or an eased change is moving the view, so it goes on changing.
    pub moving: bool,
}

impl CanvasView {
    /// `shown` as the view of the Canvas named `name` in window `window`,
    /// its bindings in an ordered dictionary whose keys are compared
    /// without regard to case, as `[ordered]` makes one.
    fn of(name: &str, window: u64, shown: &ViewShown) -> PsResult<CanvasView> {
        let comparer = PsType::from_name("System.StringComparer").call_static("get_OrdinalIgnoreCase", &[])?;
        let bindings = PsHashtable(PsType::from_name("System.Collections.Specialized.OrderedDictionary").new(&[comparer])?);
        let view = &shown.view;
        for (gesture, made) in view.controls.bindings() {
            bindings.set(&gesture.written(), made.name().into_ps()?)?;
        }
        let controls = match view.controls {
            ViewControls::Map => GuiCanvasViewControls::Map,
            ViewControls::Design => GuiCanvasViewControls::Design,
            ViewControls::Custom(_) => GuiCanvasViewControls::Custom,
            ViewControls::None => GuiCanvasViewControls::None,
        };
        Ok(CanvasView {
            name: name.to_string(),
            window,
            center: view.center.map(|c| c.to_vec()),
            zoom: view.zoom,
            min_zoom: view.min_zoom,
            max_zoom: view.max_zoom,
            controls,
            bindings: bindings.0,
            moving: shown.moving,
        })
    }
}

/// The custom controls `table` gives a view, each key a gesture or a key
/// and each value the move it makes, in order of the gestures' names; two
/// keys naming one gesture are refused, so no match depends on the order, and
/// so is a binding the view cannot take.
fn view_bindings_of(table: &PsHashtable) -> PsResult<Vec<(ViewGesture, ViewMove)>> {
    let refuse = |text: String| binding_error("GuiCanvasView", text);
    let mut named: Vec<(ViewGesture, ViewMove, String)> = Vec::new();
    for key in table.keys()? {
        let value = String::from_ps(&table.get(&key)?)?;
        let gesture = ViewGesture::named(&key).map_err(refuse)?;
        let made = ViewMove::named(&value).map_err(|e| refuse(format!("{key}: {e}")))?;
        if let Some((_, _, first)) = named.iter().find(|(g, _, _)| *g == gesture) {
            return Err(refuse(format!("{first} and {key} name one gesture")));
        }
        named.push((gesture, made, key));
    }
    if named.is_empty() {
        return Err(refuse(String::from("-Bindings names no gesture; -Controls None leaves the view to the script")));
    }
    named.sort_by_key(|(gesture, _, _)| gesture.written());
    let bindings: Vec<(ViewGesture, ViewMove)> = named.into_iter().map(|(gesture, made, _)| (gesture, made)).collect();
    match ViewControls::Custom(bindings.clone()).problem() {
        Some(problem) => Err(refuse(problem)),
        None => Ok(bindings),
    }
}

/// The seconds `given` says: a number, or a [TimeSpan]'s length.
fn seconds_of(given: &PsObject) -> PsResult<f64> {
    if given.type_tag()? == PS_TYPE_TIMESPAN {
        return Ok(PsTimeSpan::from_ps(given)?.ticks as f64 / TICKS_PER_SECOND);
    }
    f64::from_ps(given).map_err(|e| binding_error("GuiCanvasView", format!("-Duration takes seconds or a [TimeSpan]: {}", e.message)))
}

/// Sets the view of a Canvas: the point of its items' coordinates at the
/// canvas's middle and how near, the smallest and largest zoom the person
/// may go to, a fit to every item or to named ones, and the controls the
/// person moves the view with. A canvas without a view starts from one
/// that stays at the canvas's own middle at zoom 1 under the Map controls.
/// A parameter left out leaves what the view has, so -Controls switches the
/// controls without moving the view; a fit is made after the other
/// parameters. With -Duration the view moves to the change over that many
/// seconds along -Easing, Ease when not given, from the view drawn when
/// the command runs, ending any move under way where it is; a canvas not
/// yet drawn takes the change at once.
///
/// The Map controls are Leaflet's and Google Maps': a left drag where no
/// named item takes the drag pans, and glides on when let go moving; Shift
/// and a left drag draws a box and zooms to it; the wheel and a pinch zoom
/// about the pointer; a double-click zooms in a step, and with Shift out;
/// the arrows pan, and + and = zoom in a step and - out. The Design
/// controls are Figma's and Inkscape's: the wheel scrolls, Shift and the
/// wheel scroll sideways, Ctrl and the wheel or a pinch zoom about the
/// pointer, a middle drag or a left drag with the space bar down pans, +
/// and = zoom in a step and - out, Shift+1 fits every item, Shift+0 returns
/// to zoom 1, and Ctrl and the arrows pan. -Bindings gives custom controls,
/// a hashtable of gestures such as 'Left', 'Space+Left', 'Shift+Wheel',
/// 'Pinch' or 'DoubleClick', and keys such as 'ArrowLeft', 'Plus' or
/// 'Shift+1', to the moves they make: Pan, Glide, Zoom, Scroll,
/// ScrollAcross, ZoomIn, ZoomOut, BoxZoom, Fit or ActualSize. None leaves
/// the view to the script. Keys move the view while the canvas has the
/// focus.
///
/// Inside a handler it acts on the handler's window; elsewhere -Window
/// names the window.
///
/// # Examples
/// Set-GuiCanvasView Board -Center 100,50 -Zoom 2
/// Set-GuiCanvasView Board -Controls Design
/// Set-GuiCanvasView Board -FitItem Box -Duration 0.5 -Easing EaseInOutCubic
/// Set-GuiCanvasView Board -Bindings @{ Middle = 'Pan'; Wheel = 'Zoom'; F = 'Fit' }
#[cmdlet(verb = "Set", noun = "GuiCanvasView")]
#[derive(Default)]
pub struct SetGuiCanvasView {
    /// The canvas's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The point of the items' coordinates at the canvas's middle, as x and y.
    #[param]
    pub center: Option<Vec<f64>>,
    /// How many points on the canvas one unit of the items' coordinates takes, from MinZoom to MaxZoom.
    #[param]
    pub zoom: Option<f64>,
    /// The smallest zoom the person may go to, greater than 0.
    #[param]
    pub min_zoom: Option<f64>,
    /// The largest zoom the person may go to, no smaller than MinZoom.
    #[param]
    pub max_zoom: Option<f64>,
    /// The controls the person moves the view with: Map, Design, Custom with -Bindings, or None.
    #[param]
    pub controls: Option<GuiCanvasViewControls>,
    /// Custom controls: a hashtable of gestures and keys, such as 'Shift+Left' or 'ArrowLeft', to the moves they make.
    #[param]
    pub bindings: Option<PsHashtable>,
    /// Fits the view to every item, as the canvas draws them.
    #[param]
    pub fit: bool,
    /// Fits the view to the items these names name, compared without regard to case.
    #[param]
    pub fit_item: Option<Vec<String>>,
    /// How long the view takes to move to the change, as seconds or a [TimeSpan], greater than 0; at once when not given.
    #[param]
    pub duration: Option<PsObject>,
    /// The curve the move follows with -Duration: Linear, Ease, EaseIn, EaseOut or EaseInOut, or one of Penner's, such as EaseInOutCubic; Ease when not given.
    #[param]
    pub easing: Option<String>,
    /// Writes the view as it is then drawn.
    #[param]
    pub pass_thru: bool,
    /// The window's number; inside a handler, the handler's window when not given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl SetGuiCanvasView {
    /// The change this command's parameters make to a view.
    fn change(&self) -> PsResult<ViewChange> {
        let refuse = |text: String| binding_error("GuiCanvasView", text);
        let center = match self.center.as_deref() {
            None => None,
            Some([x, y]) => Some([*x, *y]),
            Some(other) => return Err(refuse(format!("-Center takes two numbers, x and y, and was given {}", other.len()))),
        };
        let controls = match (self.controls, &self.bindings) {
            (None | Some(GuiCanvasViewControls::Custom), Some(table)) => Some(ViewControls::Custom(view_bindings_of(table)?)),
            (Some(GuiCanvasViewControls::Custom), None) => return Err(refuse(String::from("custom controls are the gestures -Bindings gives"))),
            (Some(other), Some(_)) => return Err(refuse(format!("-Bindings gives custom controls, and -Controls {other:?} names others"))),
            (Some(GuiCanvasViewControls::Map), None) => Some(ViewControls::Map),
            (Some(GuiCanvasViewControls::Design), None) => Some(ViewControls::Design),
            (Some(GuiCanvasViewControls::None), None) => Some(ViewControls::None),
            (None, None) => None,
        };
        let fit = match (self.fit, self.fit_item.as_deref()) {
            (true, Some(_)) => return Err(refuse(String::from("-Fit fits every item, and -FitItem names some; give one"))),
            (true, None) => Some(ViewFit::All),
            (false, Some([])) => return Err(refuse(String::from("-FitItem names no item"))),
            (false, Some(names)) => Some(ViewFit::Items(names.to_vec())),
            (false, None) => None,
        };
        let ease = match (&self.duration, &self.easing) {
            (Some(given), easing) => {
                let easing = match easing {
                    Some(named) => Easing::named(named).map_err(refuse)?,
                    None => Easing::Ease,
                };
                Some(ViewEase { seconds: seconds_of(given)?, easing })
            }
            (None, Some(_)) => return Err(refuse(String::from("-Easing shapes the move -Duration gives; give -Duration too"))),
            (None, None) => None,
        };
        Ok(ViewChange { center, zoom: self.zoom, min_zoom: self.min_zoom, max_zoom: self.max_zoom, controls, fit, ease })
    }
}

impl Cmdlet for SetGuiCanvasView {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let change = self.change()?;
        let id = window::window_of(ps, self.window)?;
        window::view_canvas(id, &self.name)?;
        let session = session::of_window(id)?;
        match call(ps, &session, GpuOp::SetCanvasView { window: id, name: self.name.clone(), change })? {
            GpuValue::CanvasView(Some(shown)) if self.pass_thru => ps.write(CanvasView::of(&self.name, id, &shown)?),
            GpuValue::CanvasView(Some(_)) => Ok(()),
            other => Err(unexpected("a canvas's view", &other)),
        }
    }
}

/// Writes the view of a Canvas as it is drawn: where the person has moved
/// it, or a move under way has it, how near it is, the smallest and
/// largest zoom the person may go to, its controls with the gestures and
/// keys they take, and whether it is moving. A canvas without a view writes
/// nothing. Inside a handler it acts on the handler's window; elsewhere
/// -Window names the window.
///
/// # Examples
/// Get-GuiCanvasView Board
/// (Get-GuiCanvasView Board -Window $w.Id).Bindings
#[cmdlet(verb = "Get", noun = "GuiCanvasView")]
#[derive(Default)]
pub struct GetGuiCanvasView {
    /// The canvas's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The window's number; inside a handler, the handler's window when not given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl Cmdlet for GetGuiCanvasView {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let id = window::window_of(ps, self.window)?;
        window::view_canvas(id, &self.name)?;
        let session = session::of_window(id)?;
        match call(ps, &session, GpuOp::GetCanvasView { window: id, name: self.name.clone() })? {
            GpuValue::CanvasView(Some(shown)) => ps.write(CanvasView::of(&self.name, id, &shown)?),
            GpuValue::CanvasView(None) => Ok(()),
            other => Err(unexpected("a canvas's view", &other)),
        }
    }
}

/// Takes the view from a Canvas, so its items' coordinates are the
/// canvas's own again and no gesture or key moves them. A canvas without a
/// view is left as it is. Inside a handler it acts on the handler's
/// window; elsewhere -Window names the window.
///
/// # Examples
/// Remove-GuiCanvasView Board
#[cmdlet(verb = "Remove", noun = "GuiCanvasView", supports_should_process)]
#[derive(Default)]
pub struct RemoveGuiCanvasView {
    /// The canvas's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The window's number; inside a handler, the handler's window when not given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl Cmdlet for RemoveGuiCanvasView {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let id = window::window_of(ps, self.window)?;
        window::view_canvas(id, &self.name)?;
        if !ps.should_process(&format!("Canvas {} in window {id}", self.name), "Remove the view")? {
            return Ok(());
        }
        let session = session::of_window(id)?;
        match call(ps, &session, GpuOp::RemoveCanvasView { window: id, name: self.name.clone() })? {
            GpuValue::Done => Ok(()),
            other => Err(unexpected("a canvas's view", &other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gesture_reads_its_keys_in_any_case_and_order_and_is_written_back_in_one() {
        assert_eq!(gesture_of("Left"), Ok(CameraGesture::Drag { button: CameraButton::Left, modifiers: CameraModifiers::NONE }));
        let both = CameraModifiers { shift: true, ctrl: true, alt: false };
        assert_eq!(gesture_of("ctrl + SHIFT+middle"), Ok(CameraGesture::Drag { button: CameraButton::Middle, modifiers: both }));
        assert_eq!(gesture_of("Command+Wheel"), Ok(CameraGesture::Wheel { modifiers: CameraModifiers::CTRL }));
        assert_eq!(gesture_text(CameraGesture::Drag { button: CameraButton::Middle, modifiers: both }), "Shift+Ctrl+Middle");
        assert!(gesture_of("Meta+Left").is_err_and(|e| e.contains("Meta")));
        assert!(gesture_of("Shift").is_err_and(|e| e.contains("ends in Shift")));
        assert_eq!(move_of(" pan "), Ok(CameraMove::Pan));
        assert!(move_of("Spin").is_err());
        for controls in [CameraControls::Default, CameraControls::Blender] {
            for (gesture, _) in controls.bindings() {
                assert_eq!(gesture_of(&gesture_text(gesture)), Ok(gesture), "{controls:?}: each gesture reads back as written");
            }
        }
    }

    #[test]
    fn set_gui_gpu_camera_takes_three_numbers_for_a_target_and_bindings_for_custom_controls() {
        let given = |f: fn(&mut SetGuiGpuCamera)| {
            let mut command = SetGuiGpuCamera::default();
            f(&mut command);
            command.change()
        };
        assert_eq!(given(|_| ()).expect("nothing given is a change"), CameraChange::default(), "nothing given changes nothing");
        assert!(given(|c| c.target = Some(vec![1.0, 2.0])).is_err());
        assert_eq!(given(|c| c.target = Some(vec![1.0, 2.0, 3.0])).expect("three numbers make a target").target, Some([1.0, 2.0, 3.0]));
        assert!(given(|c| c.controls = Some(GuiCameraControls::Custom)).is_err(), "custom controls without -Bindings");
        assert_eq!(given(|c| c.controls = Some(GuiCameraControls::None)).expect("None names controls").controls, Some(CameraControls::Fixed));
    }

    #[test]
    fn a_pipeline_given_nothing_draws_as_new_wgpu_render_pipeline_does_by_default() {
        let none = PipelineOptions::default();
        assert!(!none.any());
        let spec = none.spec(String::from("wgsl"));
        assert_eq!((spec.topology, spec.cull, spec.blend, spec.depth, spec.sample_count), (Topology::TriangleList, Cull::None, Blend::Replace, false, 1));
        assert_eq!(spec.custom_blend, CustomBlend::default());
        let custom = PipelineOptions { blend: Some(GuiBlendMode::Custom), alpha_blend_operation: Some(GuiBlendOperation::Max), ..PipelineOptions::default() };
        assert!(custom.any());
        let spec = custom.spec(String::from("wgsl"));
        assert_eq!((spec.blend, spec.custom_blend.alpha_operation, spec.custom_blend.source), (Blend::Custom, Some(WireOperation::Max), None));
        assert!(PipelineOptions { constants: vec![(String::from("scale"), 2.0)], ..PipelineOptions::default() }.any(), "a constant is an option a pipeline object refuses");
        let plain = PipelineOptions::default().spec(String::from("wgsl"));
        assert_eq!((plain.depth_format, plain.depth_test, plain.stencil, plain.depth_target()), (None, WireDepthTest::default(), WireStencilTest::default(), None), "no depth or stencil test");
    }

    #[test]
    fn the_depth_and_stencil_parameters_go_to_the_spec_as_given_each_back_face_part_left_to_the_front_s() {
        let given = PipelineOptions {
            depth_format: Some(GuiDepthFormat::Depth24PlusStencil8),
            depth_compare: Some(GuiCompareFunction::Greater),
            depth_write: Some(false),
            depth_bias: Some(-3),
            depth_bias_slope_scale: Some(1.5),
            depth_bias_clamp: Some(0.25),
            stencil_compare: Some(GuiCompareFunction::Equal),
            stencil_pass: Some(GuiStencilOperation::IncrementWrap),
            stencil_read_mask: Some(0x0f),
            back_stencil_fail: Some(GuiStencilOperation::Invert),
            ..PipelineOptions::default()
        };
        assert!(given.any(), "the depth and stencil parameters are options a pipeline object refuses");
        let spec = given.spec(String::from("wgsl"));
        assert_eq!(spec.depth_target(), Some(WireDepthFormat::Depth24PlusStencil8));
        assert_eq!(spec.depth_test, WireDepthTest { compare: Some(WireCompare::Greater), write: Some(false), bias: Some(-3), bias_slope_scale: Some(1.5), bias_clamp: Some(0.25) });
        assert_eq!(spec.stencil.front, WireFace { compare: Some(WireCompare::Equal), fail: None, depth_fail: None, pass: Some(WireStencil::IncrementWrap) });
        assert_eq!(spec.stencil.back, WireFace { fail: Some(WireStencil::Invert), ..WireFace::default() }, "the back face's parts stay absent, for the front's to fill");
        assert_eq!((spec.stencil.read_mask, spec.stencil.write_mask), (Some(0x0f), None));
        assert!(spec.tests_stencil());
        assert_eq!(spec.depth_stencil_refusal(), None);
    }

    #[test]
    fn every_name_table_names_each_value_once_in_declared_order() {
        let topologies: Vec<GuiPrimitiveTopology> = TOPOLOGIES.iter().map(|(t, _)| *t).collect();
        assert_eq!(topologies.iter().map(|t| t.wire()).collect::<Vec<_>>(), [Topology::PointList, Topology::LineList, Topology::LineStrip, Topology::TriangleList, Topology::TriangleStrip]);
        let factors: Vec<WireFactor> = BLEND_FACTORS.iter().map(|(f, _)| f.wire()).collect();
        for (at, (_, name)) in BLEND_FACTORS.iter().enumerate() {
            assert_eq!(format!("{:?}", factors[at]), *name, "the factor named {name} is wgpu's of that name");
        }
        for (mode, name) in BLEND_MODES {
            assert_eq!(format!("{:?}", mode.wire()), name);
        }
        for (operation, name) in BLEND_OPERATIONS {
            assert_eq!(format!("{:?}", operation.wire()), name);
        }
        for (cull, name) in CULL_MODES {
            assert_eq!(format!("{:?}", cull.wire()), name);
        }
        for (format, name) in DEPTH_FORMATS {
            assert_eq!(format.wire().name(), name, "the format named {name} is the Wgpu module's of that name");
        }
        assert_eq!(DEPTH_FORMATS.map(|(f, _)| f.wire()), WireDepthFormat::ALL, "every format, in the Wgpu module's order");
        for (compare, name) in COMPARE_FUNCTIONS {
            assert_eq!(format!("{:?}", compare.wire()), name);
        }
        for (operation, name) in STENCIL_OPERATIONS {
            assert_eq!(format!("{:?}", operation.wire()), name);
        }
        for key in ["DepthFormat", "DepthCompare", "BackStencilPass", "StencilReference"] {
            assert_eq!(DRAW_KEYS.iter().filter(|k| **k == key).count(), 1, "a draw takes {key} once");
        }
    }

    #[test]
    fn a_clear_color_takes_three_or_four_numbers() {
        assert_eq!(clear_of(None).expect("none given"), [0.0, 0.0, 0.0, 1.0], "opaque black");
        assert_eq!(clear_of(Some(&[0.25, 0.5, 0.75])).expect("three"), [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(clear_of(Some(&[0.1, 0.2, 0.3, 0.4])).expect("four"), [0.1, 0.2, 0.3, 0.4]);
        let refused = clear_of(Some(&[1.0, 2.0])).expect_err("two are refused");
        assert_eq!(refused.error_id, "GuiGpuClearColor");
        assert!(refused.message.ends_with("2 were given"), "{}", refused.message);
    }

    #[test]
    fn a_surface_s_texels_write_a_png_that_reads_back_the_same() {
        let rgba = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 10, 20, 30, 40];
        let bytes = png_of(2, 2, &rgba).expect("a PNG");
        let mut reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().expect("a PNG header");
        let mut back = vec![0; reader.output_buffer_size().expect("a buffer size")];
        let info = reader.next_frame(&mut back).expect("the PNG's texels");
        assert_eq!((info.width, info.height, info.color_type, info.bit_depth), (2, 2, png::ColorType::Rgba, png::BitDepth::Eight));
        assert_eq!(&back[..info.buffer_size()], &rgba);
        assert!(png_of(2, 2, &rgba[..12]).is_err(), "texels short of the size are refused");
    }

    const TEXTURE_FORMATS: [GuiTextureFormat; 14] = [
        GuiTextureFormat::R8Unorm,
        GuiTextureFormat::Rg8Unorm,
        GuiTextureFormat::Rgba8Unorm,
        GuiTextureFormat::Rgba8UnormSrgb,
        GuiTextureFormat::Bgra8Unorm,
        GuiTextureFormat::Bgra8UnormSrgb,
        GuiTextureFormat::R16Float,
        GuiTextureFormat::Rg16Float,
        GuiTextureFormat::Rgba16Float,
        GuiTextureFormat::R32Float,
        GuiTextureFormat::Rg32Float,
        GuiTextureFormat::Rgba32Float,
        GuiTextureFormat::R32Uint,
        GuiTextureFormat::R32Sint,
    ];

    #[test]
    fn every_texture_format_crosses_the_wire_as_the_format_of_its_name() {
        for format in TEXTURE_FORMATS {
            assert_eq!(format!("{:?}", format.wire()), format!("{format:?}"));
            assert_eq!(GuiTextureFormat::of(format.wire()), format, "{format:?} reads back as itself");
        }
        assert_eq!(TEXTURE_FORMATS.iter().map(|f| f.element()).filter(|e| *e == ElementType::UInt8).count(), 6, "the six 8-bit formats are written as bytes");
        assert_eq!((GuiTextureFormat::Rg16Float.element(), GuiTextureFormat::R32Uint.element(), GuiTextureFormat::R32Sint.element()), (ElementType::Float16, ElementType::UInt32, ElementType::Int32));
        assert_eq!(format!("{:?}{:?}", GuiFilterMode::default().wire(), GuiAddressMode::default().wire()), "NearestClampToEdge", "a sampler given nothing is New-WgpuSampler's");
    }

    #[test]
    fn import_gui_gpu_image_reads_into_rgba8unorm_and_into_its_srgb_form_with_srgb() {
        let given = |format: Option<GuiTextureFormat>, srgb: bool| ImportGuiGpuImage { format, srgb, ..ImportGuiGpuImage::default() }.format();
        assert_eq!(given(None, false).expect("nothing given"), GuiTextureFormat::Rgba8Unorm);
        assert_eq!(given(None, true).expect("-Srgb"), GuiTextureFormat::Rgba8UnormSrgb);
        assert_eq!(given(Some(GuiTextureFormat::Rgba8UnormSrgb), true).expect("both name sRGB"), GuiTextureFormat::Rgba8UnormSrgb);
        assert_eq!(given(Some(GuiTextureFormat::R32Float), false).expect("a format the renderer judges"), GuiTextureFormat::R32Float);
        let refused = given(Some(GuiTextureFormat::Rgba8Unorm), true).expect_err("-Srgb against another format");
        assert_eq!(refused.error_id, "GuiGpuImageFormat");
    }

    #[test]
    fn a_page_s_vertex_inputs_take_the_webgpu_format_of_their_type() {
        assert_eq!(vertex_format(ElementType::Float32, 2).expect("vec2<f32>"), "float32x2");
        assert_eq!(vertex_format(ElementType::Float32, 1).expect("f32"), "float32");
        assert_eq!(vertex_format(ElementType::Int32, 4).expect("vec4<i32>"), "sint32x4");
        assert_eq!(vertex_format(ElementType::UInt32, 3).expect("vec3<u32>"), "uint32x3");
        assert_eq!(vertex_format(ElementType::Float16, 2).expect("vec2<f16>"), "float16x2");
        assert_eq!(vertex_format(ElementType::Float64, 2).expect_err("no f64 vertex format").error_id, "GuiGpuVertexInput");
    }

    #[test]
    fn a_page_s_pass_takes_its_entry_point_and_workgroups_from_the_shader() {
        let two = "@compute @workgroup_size(64, 2) fn first() {}\n@compute @workgroup_size(8) fn second() {}";
        assert_eq!(compute_entry(two, Some("first")).expect("named"), (String::from("first"), [64, 2, 1]));
        assert_eq!(compute_entry(two, None).expect_err("two entry points").error_id, "GuiGpuEntry");
        assert_eq!(compute_entry(two, Some("third")).expect_err("no such entry point").error_id, "GuiGpuEntry");
        let one = "@compute @workgroup_size(16, 16) fn main() {}";
        assert_eq!(compute_entry(one, None).expect("the only one"), (String::from("main"), [16, 16, 1]));
        assert_eq!(workgroups_for([1000, 1, 1], [64, 1, 1]).expect("rounded up"), [16, 1, 1]);
        assert_eq!(workgroups_for([33, 33, 1], [16, 16, 1]).expect("in two dimensions"), [3, 3, 1]);
        assert_eq!(workgroups_for([u64::MAX, 1, 1], [1, 1, 1]).expect_err("more than a dispatch takes").error_id, "GuiGpuDispatchSize");
    }

    #[test]
    fn a_page_s_refusals_keep_their_ids_under_the_categories_of_their_kind() {
        let reported = |id: &str| failed(GpuFailure { id: id.to_string(), text: String::from("why") });
        let lacking = reported("GuiGpuNoWebGpu");
        assert_eq!((lacking.error_id.as_str(), lacking.category), ("GuiGpuNoWebGpu", ErrorCategory::ResourceUnavailable), "an engine without WebGPU is a renderer without a GPU layer");
        assert_eq!(reported("GuiGpuNoCopy").category, ErrorCategory::ObjectNotFound, "a copy the page lacks is an object the renderer lacks");
        assert_eq!(reported("GuiGpuNotDrawn").category, ErrorCategory::InvalidOperation);
    }

    #[test]
    fn a_compute_shader_s_storage_textures_it_may_store_into_are_found_by_binding() {
        let source = "@group(0) @binding(0) var written: texture_storage_2d<rgba8unorm, write>;
            @group(0) @binding(1) var sampled: texture_2d<f32>;
            @group(1) @binding(2) var both: texture_storage_2d<r32float, read_write>;
            @group(1) @binding(3) var read_only: texture_storage_2d<r32float, read>;
            @compute @workgroup_size(1) fn main() { textureStore(written, vec2<i32>(0), vec4<f32>(1.0)); }";
        assert_eq!(storage_writes(source).expect("read"), vec![(0, 0), (1, 2)], "write and read_write storage, not a sampled or a read-only one");
        assert_eq!(storage_writes("@compute fn main( {").expect_err("unreadable").error_id, "WgpuShaderInvalid");
    }

    #[test]
    fn a_texture_and_a_sampler_are_objects_of_the_table_and_their_arrays_list_resources() {
        for name in ["PwrsGui.GpuBuffer", "PwrsGui.GpuTexture", "PwrsGui.GpuSampler"] {
            assert!(is_table(name), "{name}");
            assert!(is_list(&format!("{name}[]")), "{name}[]");
        }
        assert!(is_list("System.Object[]"));
        assert!(!is_table("PwrsGui.GpuRenderPipeline"), "a pipeline draws; it is no binding's resource");
        assert!(!is_list("System.Single[]"), "an array of numbers is one resource");
    }
}
