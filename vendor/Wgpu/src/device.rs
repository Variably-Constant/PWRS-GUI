//! Instances, adapters and devices: what every other object is made on.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use arc_swap::{ArcSwap, ArcSwapOption};
use pwrs::prelude::*;

use crate::binding::BindGroup;
use crate::buffer::{self, Buffer, BufferUsage};
use crate::element::ElementType;
use crate::encoder::{CommandBuffer, CommandEncoder, QueryKind, QuerySet};
use crate::error::{device_error, invalid, invalid_operation, not_found, panic_text};
use crate::names::{self, Backend, DeviceType, Limits};
use crate::pipeline::ComputePipeline;
use crate::shader::{Shader, ShaderLanguage, ShaderStage};
use crate::tasks::{self, Interrupt};

/// How an instance is made: which backends (every native one when `None`),
/// and which debugging aids.
#[derive(Clone, Default)]
pub(crate) struct InstanceOptions {
    pub(crate) backends: Option<Vec<Backend>>,
    pub(crate) validation: bool,
    pub(crate) gpu_based_validation: bool,
}

/// What tells one instance from another: its backends, Noop among them
/// when listed, and its flags.
#[derive(Clone, Copy, PartialEq, Eq)]
struct InstanceKey {
    backends: wgpu::Backends,
    flags: wgpu::InstanceFlags,
}

impl InstanceKey {
    fn of(o: &InstanceOptions) -> InstanceKey {
        let mut flags = wgpu::InstanceFlags::from_build_config();
        if o.validation {
            flags |= wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::DEBUG;
        }
        if o.gpu_based_validation {
            flags |= wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::GPU_BASED_VALIDATION;
        }
        InstanceKey { backends: names::backends(o.backends.as_deref()), flags }
    }
}

/// One instance the module made with the adapters it found, and the ones
/// it made before.
struct StoredInstance {
    key: InstanceKey,
    /// Kept so the instance lives as long as its entry, adapters or none.
    _instance: wgpu::Instance,
    adapters: Arc<[wgpu::Adapter]>,
    earlier: Option<Arc<StoredInstance>>,
}

/// The newest instance the module made. Instances stay until no runspace of
/// the process has the module, so closing a device never ends the threads
/// its instance runs, such as the GL backend's window thread. Each lists its adapters once, on
/// the thread that makes it and before any other thread can reach it, since
/// NVIDIA's Vulkan driver faults when threads query the physical devices of
/// one instance at once.
static INSTANCES: ArcSwapOption<StoredInstance> = ArcSwapOption::const_empty();

/// The state of the process's first instance with the GL backend: none
/// made, being made by one caller, or made. The GL backend loads libglvnd's
/// libEGL, whose initializer faults when several threads run it at once,
/// so that instance is made by one caller while the others wait; once
/// libEGL is loaded, loading it again runs no initializer.
static FIRST_GL: AtomicU8 = AtomicU8::new(NONE_MADE);
const NONE_MADE: u8 = 0;
const MAKING: u8 = 1;
const MADE: u8 = 2;

/// Marks the first GL instance made when it is dropped, so a caller that
/// unwinds while making it does not leave the others waiting.
struct FirstGl;

impl Drop for FirstGl {
    fn drop(&mut self) {
        FIRST_GL.store(MADE, Ordering::Release);
    }
}

/// Waits, in 1 ms parked steps, while another caller makes the process's
/// first instance with the GL backend; when no caller has made it yet,
/// returns the guard of this caller, which makes it.
fn first_gl_turn() -> Option<FirstGl> {
    loop {
        match FIRST_GL.compare_exchange(NONE_MADE, MAKING, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return Some(FirstGl),
            Err(MADE) => return None,
            Err(MAKING) => std::thread::park_timeout(Duration::from_millis(1)),
            Err(other) => unreachable!("the first GL instance's state is {other}, which no caller stores"),
        }
    }
}

/// The adapters of the instance for `o`'s backends and debugging aids, which
/// is made and listed on first use. The first instance with the GL backend
/// is made by one caller while the others wait for it; of two callers
/// making any other at once, both get the one stored first.
pub(crate) fn adapters(o: &InstanceOptions) -> PsResult<Arc<[wgpu::Adapter]>> {
    let key = InstanceKey::of(o);
    if let Some(stored) = find_instance(INSTANCES.load_full(), key) {
        return Ok(stored);
    }
    pin_driver_libraries()?;
    let turn = if key.backends.contains(wgpu::Backends::GL) { first_gl_turn() } else { None };
    // The caller another waited for may have made this very instance.
    if let Some(stored) = find_instance(INSTANCES.load_full(), key) {
        return Ok(stored);
    }
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = key.backends;
    desc.flags = key.flags;
    desc.backend_options.noop.enable = key.backends.contains(wgpu::Backends::NOOP);
    #[cfg(all(windows, target_arch = "aarch64", feature = "module", not(test)))]
    if key.backends.contains(wgpu::Backends::DX12) {
        desc.backend_options.dx12.shader_compiler = shipped_dxc()?;
    }
    let instance = wgpu::Instance::new(desc);
    let made: Arc<[wgpu::Adapter]> = pollster::block_on(instance.enumerate_adapters(key.backends)).into();
    let mut chosen = made.clone();
    INSTANCES.rcu(|newest| match find_instance(newest.clone(), key) {
        Some(stored) => {
            chosen = stored;
            newest.clone()
        }
        None => {
            chosen = made.clone();
            Some(Arc::new(StoredInstance { key, _instance: instance.clone(), adapters: made.clone(), earlier: newest.clone() }))
        }
    });
    drop(turn);
    Ok(chosen)
}

/// The adapters stored under `key` in the list that starts at `newest`.
fn find_instance(newest: Option<Arc<StoredInstance>>, key: InstanceKey) -> Option<Arc<[wgpu::Adapter]>> {
    let mut at = newest;
    while let Some(stored) = at {
        if stored.key == key {
            return Some(stored.adapters.clone());
        }
        at = stored.earlier.clone();
    }
    None
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryExW(name: *const u16, file: *mut std::ffi::c_void, flags: u32) -> *mut std::ffi::c_void;
    fn GetModuleHandleExW(flags: u32, name: *const u16, module: *mut *mut std::ffi::c_void) -> i32;
}

/// The libraries NVIDIA's Vulkan driver loads and frees each time it
/// creates an instance or a device, while holding a lock its DllMain takes
/// when a thread that used the driver starts or exits.
#[cfg(windows)]
const DRIVER_LIBRARIES: [&str; 2] = ["psapi.dll", "dbghelp.dll"];

/// Whether every one of DRIVER_LIBRARIES is loaded and pinned.
#[cfg(windows)]
static DRIVER_LIBRARIES_PINNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Loads each of DRIVER_LIBRARIES from System32 and pins it for the life of
/// the process, so the driver's own loads of them return at once instead
/// of waiting for the loader, which a thread starting or exiting holds.
#[cfg(windows)]
fn pin_driver_libraries() -> PsResult<()> {
    const LOAD_LIBRARY_SEARCH_SYSTEM32: u32 = 0x0000_0800;
    const GET_MODULE_HANDLE_EX_FLAG_PIN: u32 = 0x0000_0001;
    if DRIVER_LIBRARIES_PINNED.load(Ordering::Acquire) {
        return Ok(());
    }
    for library in DRIVER_LIBRARIES {
        let name: Vec<u16> = library.encode_utf16().chain([0]).collect();
        let mut module = std::ptr::null_mut();
        // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives
        // both calls, and `module` is a valid place for the handle.
        let pinned = unsafe {
            !LoadLibraryExW(name.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32).is_null()
                && GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_PIN, name.as_ptr(), &mut module) != 0
        };
        if !pinned {
            return Err(device_error("WgpuLoader", &format!("{library} cannot be loaded and pinned"), std::io::Error::last_os_error()));
        }
    }
    DRIVER_LIBRARIES_PINNED.store(true, Ordering::Release);
    Ok(())
}

#[cfg(not(windows))]
fn pin_driver_libraries() -> PsResult<()> {
    Ok(())
}

/// The compiler Direct3D 12 compiles shaders with on Arm64 Windows, where
/// no DXC is linked in: the dxcompiler.dll the module ships, loaded from
/// the copy PWRS stages for this process. A unit test, run outside any
/// PowerShell host, leaves wgpu to find a dxcompiler.dll on the PATH.
#[cfg(all(windows, target_arch = "aarch64", feature = "module", not(test)))]
fn shipped_dxc() -> PsResult<wgpu::Dx12Compiler> {
    match pwrs::native_file_path("dxcompiler.dll")?.into_os_string().into_string() {
        Ok(dxc_path) => Ok(wgpu::Dx12Compiler::DynamicDxc { dxc_path }),
        Err(path) => Err(invalid("WgpuLoader", format!("dxcompiler.dll was staged at {}, which is not text, so wgpu cannot load it", path.to_string_lossy()))),
    }
}

/// A GPU adapter wgpu can open, with everything it supports.
#[psclass(name = "Wgpu.Adapter", columns = ["Name", "Backend", "DeviceType", "Driver"])]
#[derive(Default, Clone)]
pub struct Adapter {
    /// The adapter's name, as the driver reports it.
    pub name: String,
    /// The graphics API it is reached through.
    pub backend: Backend,
    /// DiscreteGpu, IntegratedGpu, VirtualGpu, Cpu or Other.
    pub device_type: DeviceType,
    /// The vendor id, a PCI vendor id on most backends.
    pub vendor: u32,
    /// The device id, a PCI device id on most backends.
    pub device: u32,
    /// The PCI bus id, where the backend reports one.
    pub pci_bus_id: String,
    /// The driver's name.
    pub driver: String,
    /// The driver's version and details.
    pub driver_info: String,
    /// The smallest subgroup size the adapter runs.
    pub subgroup_min_size: u32,
    /// The largest subgroup size the adapter runs.
    pub subgroup_max_size: u32,
    /// Every optional feature the adapter supports, which Open-WgpuDevice requests.
    pub features: Vec<String>,
    /// Every limit of the adapter, which Open-WgpuDevice requests.
    pub limits: Limits,
    /// The downlevel capabilities the adapter has, by wgpu's names for them.
    pub downlevel_flags: Vec<String>,
    /// The shader model wgpu reports for the adapter.
    pub shader_model: String,
}

pub(crate) fn describe(adapter: &wgpu::Adapter) -> Adapter {
    let info = adapter.get_info();
    let downlevel = adapter.get_downlevel_capabilities();
    Adapter {
        name: info.name.clone(),
        backend: Backend::from_wgpu(info.backend),
        device_type: DeviceType::from_wgpu(info.device_type),
        vendor: info.vendor,
        device: info.device,
        pci_bus_id: info.device_pci_bus_id.clone(),
        driver: info.driver.clone(),
        driver_info: info.driver_info.clone(),
        subgroup_min_size: info.subgroup_min_size,
        subgroup_max_size: info.subgroup_max_size,
        features: names::feature_names(adapter.features()),
        limits: Limits::from(&adapter.limits()),
        downlevel_flags: names::downlevel_names(downlevel.flags),
        shader_model: format!("{:?}", downlevel.shader_model),
    }
}

/// Which adapter to open.
#[derive(Clone, Default)]
pub(crate) struct AdapterQuery {
    pub(crate) name: Option<String>,
    pub(crate) low_power: bool,
    pub(crate) fallback: bool,
}

/// The first adapter whose name contains `q.name`, or the one wgpu's
/// request_adapter would choose: among software adapters only for a
/// fallback, a discrete GPU before an integrated one (the other way round
/// for low power), then other, virtual and CPU adapters, in listing order
/// within each kind.
fn find_adapter(adapters: &[wgpu::Adapter], q: &AdapterQuery) -> PsResult<wgpu::Adapter> {
    if let Some(wanted) = &q.name {
        let needle = wanted.to_lowercase();
        return adapters.iter().find(|a| a.get_info().name.to_lowercase().contains(&needle)).cloned().ok_or_else(|| {
            let listed = adapters.iter().map(|a| format!("{} ({:?})", a.get_info().name, a.get_info().backend)).collect::<Vec<_>>();
            not_found("WgpuNoAdapter", format!("no adapter's name contains {wanted}; the adapters are: {}", listed.join("; ")))
        });
    }
    let rank = |kind: wgpu::DeviceType| match kind {
        wgpu::DeviceType::DiscreteGpu => if q.low_power { 2 } else { 1 },
        wgpu::DeviceType::IntegratedGpu => if q.low_power { 1 } else { 2 },
        wgpu::DeviceType::Other => 3,
        wgpu::DeviceType::VirtualGpu => 4,
        wgpu::DeviceType::Cpu => 5,
    };
    adapters
        .iter()
        .filter(|a| !q.fallback || a.get_info().device_type == wgpu::DeviceType::Cpu)
        .min_by_key(|a| rank(a.get_info().device_type))
        .cloned()
        .ok_or_else(|| {
            let reason = if q.fallback { "none of the adapters is a software adapter" } else { "no adapter was found" };
            device_error("WgpuNoAdapter", "no adapter is available", reason)
        })
}

/// The memory strategy a device asks its backend for.
#[psenum(name = "Wgpu.MemoryHint")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MemoryHint {
    /// Favor speed over memory use, the backend's own default.
    #[default]
    Performance,
    /// Favor memory use over speed.
    MemoryUsage,
}

/// Everything Open-WgpuDevice and `[Wgpu.Device]::new` take.
#[derive(Clone, Default)]
pub(crate) struct OpenOptions {
    pub(crate) instance: InstanceOptions,
    pub(crate) adapter: AdapterQuery,
    /// The features to request; every one the adapter has when `None`.
    pub(crate) features: Option<Vec<String>>,
    pub(crate) experimental: bool,
    pub(crate) label: Option<String>,
    pub(crate) trace: Option<PathBuf>,
    pub(crate) memory: Option<MemoryHint>,
}

/// A device and what it was opened with, shared by every object made on
/// it.
pub(crate) struct Core {
    pub(crate) id: u64,
    pub(crate) adapter: wgpu::Adapter,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) info: wgpu::AdapterInfo,
    pub(crate) features: wgpu::Features,
    pub(crate) limits: wgpu::Limits,
    pub(crate) label: String,
    reports: Arc<Reports>,
    /// Each BC encoder's pipeline on this device, by `convert::Encoder`,
    /// built the first time it encodes and kept, as building one can take
    /// seconds.
    pub(crate) bc_encoders: [ArcSwapOption<crate::convert::EncoderPipeline>; crate::convert::ENCODERS],
    /// The index of the newest submission the submission thread made with
    /// no error, which destroying the device waits for. Every wait is for
    /// an index a submit has returned, as wgpu tracks that submission
    /// before its submit returns; wgpu's own newest index can name a
    /// submission another thread has not finished making, which
    /// wgpu-core 30's poll asserts on.
    newest: Arc<ArcSwapOption<wgpu::SubmissionIndex>>,
    /// Where submissions go to the device's submission thread, which makes
    /// every queue.submit, in the order they were handed to it, so no
    /// thread that waits is held in a submit, which some drivers block
    /// until earlier work ends. `None` on an OpenGL device, which submits
    /// on the calling thread: NVIDIA's driver deadlocks the process when a
    /// thread that used OpenGL ends while another opens or closes a device,
    /// and a submission thread ends with its device.
    jobs: Option<mpsc::Sender<Submission>>,
}

/// What a submission's reply carries: the index wgpu returned and what the
/// errors the submit raised say, or why the submission failed.
type Reply = Result<(wgpu::SubmissionIndex, Option<String>), Failed>;

/// Why a submission failed.
enum Failed {
    /// The submit panicked, with this message.
    Panicked(String),
    /// The newest submission on another WARP device, which a submission on
    /// a WARP device waits for, could not be waited for, for this reason,
    /// and the submission was not made.
    WarpWait(String),
}

/// Command buffers the device's submission thread submits, and where it
/// sends the [`Reply`].
struct Submission {
    commands: Vec<wgpu::CommandBuffer>,
    reply: mpsc::SyncSender<Reply>,
}

/// How long either side of a submission keeps looking for the other's
/// message before it sleeps. A sleeping thread is woken by the operating
/// system, which takes tens of microseconds, and a submission is usually
/// made, and the next one handed over, sooner than that.
const LOOK: Duration = Duration::from_micros(50);

/// What looking at a channel for [`LOOK`] found.
enum Looked<T> {
    /// A message.
    Came(T),
    /// Nothing came, and the other side is still there.
    Quiet,
    /// The other side has gone.
    Gone,
}

/// Looks at `rx` for [`LOOK`] without sleeping: spinning, and every
/// sixteenth turn yielding, which runs the other side where both share a
/// processor.
fn look_for<T>(rx: &mpsc::Receiver<T>) -> Looked<T> {
    let start = Instant::now();
    let mut turns = 0u32;
    loop {
        match rx.try_recv() {
            Ok(message) => return Looked::Came(message),
            Err(mpsc::TryRecvError::Disconnected) => return Looked::Gone,
            Err(mpsc::TryRecvError::Empty) => {}
        }
        if start.elapsed() >= LOOK {
            return Looked::Quiet;
        }
        turns = turns.wrapping_add(1);
        if turns.is_multiple_of(16) {
            std::thread::yield_now();
        } else {
            std::hint::spin_loop();
        }
    }
}

/// A submission handed to the device's submission thread, whose error is
/// reported as `id` with `what`.
pub(crate) struct Pending {
    id: &'static str,
    what: &'static str,
    reply: mpsc::Receiver<Reply>,
}

impl Pending {
    /// Waits until the submission thread has made the submission, looking
    /// for its reply for [`LOOK`] and then sleeping in 100 ms steps so an
    /// interrupt ends the wait, and returns its index. A submission an
    /// interrupt stopped waiting for is made all the same.
    pub(crate) fn wait(self, core: &Core, stop: Option<&dyn Interrupt>) -> PsResult<wgpu::SubmissionIndex> {
        let reply = match look_for(&self.reply) {
            Looked::Came(reply) => reply,
            Looked::Gone => return Err(device_error("WgpuSubmitThread", "the device's submission thread has ended", mpsc::RecvError)),
            Looked::Quiet => loop {
                match self.reply.recv_timeout(Duration::from_millis(100)) {
                    Ok(reply) => break reply,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if stop.is_some_and(|s| s.interrupted()) {
                            return Err(tasks::stopped());
                        }
                    }
                    Err(e @ mpsc::RecvTimeoutError::Disconnected) => return Err(device_error("WgpuSubmitThread", "the device's submission thread has ended", e)),
                }
            },
        };
        match reply {
            Ok((index, None)) => {
                core.check()?;
                Ok(index)
            }
            Ok((_, Some(e))) => Err(device_error(self.id, self.what, e)),
            Err(Failed::Panicked(message)) => Err(device_error("WgpuPanic", "the submission panicked", message)),
            Err(Failed::WarpWait(reason)) => Err(device_error("WgpuPoll", "waiting for the newest submission on another WARP device failed", reason)),
        }
    }
}

/// Runs `f` with every validation, out-of-memory and internal error it
/// raises on this thread captured, and returns its value with what the
/// [`reported`] errors say.
fn captured<T>(device: &wgpu::Device, f: impl FnOnce() -> T) -> (T, Option<String>) {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let value = f();
    let internal = pollster::block_on(internal.pop());
    let memory = pollster::block_on(memory.pop());
    let validation = pollster::block_on(validation.pop());
    (value, reported([internal, memory, validation]))
}

/// The errors three scopes caught, as one message: the internal error, the
/// out-of-memory one, then the validation one. A validation error after
/// either of the others is often what it left behind, such as a draw
/// through a pipeline that failed to build.
fn reported(errors: [Option<wgpu::Error>; 3]) -> Option<String> {
    let messages: Vec<String> = errors.into_iter().flatten().map(|e| e.to_string()).collect();
    if messages.is_empty() { None } else { Some(messages.join("; ")) }
}

/// Whether `info` is WARP, Direct3D 12's software adapter.
fn is_warp(info: &wgpu::AdapterInfo) -> bool {
    info.backend == wgpu::Backend::Dx12 && info.device_type == wgpu::DeviceType::Cpu
}

/// What keeps the work of two WARP devices from running at once.
/// Direct3D 12 makes one device per adapter in a process, so every wgpu
/// device on WARP is a queue of that one device, and WARP faults when two
/// of its queues run work at the same time. A submission on a WARP device
/// is made holding the turn, after the GPU has run the newest submission
/// when another device made it, so one device's submissions still follow
/// each other without waiting.
struct Warp {
    /// Whether a submission holds the turn.
    turn: AtomicBool,
    /// The newest submission made holding the turn. It keeps its device
    /// open until a submission on another device takes its place.
    newest: ArcSwapOption<WarpSubmission>,
}

/// The process's WARP devices.
static WARP: Warp = Warp { turn: AtomicBool::new(false), newest: ArcSwapOption::const_empty() };

/// A submission made on a WARP device: the device's id, the device, and
/// the index its submit returned.
struct WarpSubmission {
    device_id: u64,
    device: wgpu::Device,
    index: wgpu::SubmissionIndex,
}

impl Warp {
    /// Waits for the turn, looking again each millisecond.
    fn take(&self) -> WarpTurn<'_> {
        while self.turn.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            std::thread::park_timeout(Duration::from_millis(1));
        }
        WarpTurn { warp: self }
    }
}

/// The turn of a [`Warp`], given back when dropped.
struct WarpTurn<'a> {
    warp: &'a Warp,
}

impl WarpTurn<'_> {
    /// Waits until the GPU has run the newest submission, when a device
    /// other than `device_id`'s made it. A wait that fails forgets that
    /// submission, so no later submission waits for it again.
    fn after_other_devices(&self, device_id: u64) -> Result<(), Failed> {
        let newest = match self.warp.newest.load_full() {
            Some(newest) if newest.device_id != device_id => newest,
            Some(_) | None => return Ok(()),
        };
        let waited = catch_unwind(AssertUnwindSafe(|| newest.device.poll(wgpu::PollType::Wait { submission_index: Some(newest.index.clone()), timeout: None })));
        let reason = match waited {
            Ok(Ok(_)) => return Ok(()),
            Ok(Err(e)) => e.to_string(),
            Err(payload) => panic_text(payload),
        };
        self.warp.newest.store(None);
        Err(Failed::WarpWait(reason))
    }

    /// Records submission `index`, made on `device_id`'s `device`, as the
    /// newest.
    fn made(&self, device_id: u64, device: &wgpu::Device, index: &wgpu::SubmissionIndex) {
        self.warp.newest.store(Some(Arc::new(WarpSubmission { device_id, device: device.clone(), index: index.clone() })));
    }
}

impl Drop for WarpTurn<'_> {
    fn drop(&mut self) {
        self.warp.turn.store(false, Ordering::Release);
    }
}

/// Submits `job`'s command buffers under error scopes of their own, keeps
/// the index in `newest` when wgpu raised no error, and sends the index
/// and the error back, or why the submission failed. On a WARP device,
/// which `warp` names with the device's id, the submission is made holding
/// the turn, after the newest submission another device made there.
fn submit_one(device: &wgpu::Device, queue: &wgpu::Queue, newest: &ArcSwapOption<wgpu::SubmissionIndex>, warp: Option<(&Warp, u64)>, job: Submission) {
    let Submission { commands, reply } = job;
    let made = match warp {
        Some((warp, device_id)) => {
            let turn = warp.take();
            match turn.after_other_devices(device_id) {
                Ok(()) => {
                    let made = submit_now(device, queue, newest, commands);
                    if let Ok((index, None)) = &made {
                        turn.made(device_id, device, index);
                    }
                    made
                }
                Err(failed) => Err(failed),
            }
        }
        None => submit_now(device, queue, newest, commands),
    };
    // A caller an interrupt stopped no longer listens.
    if let Err(unheard) = reply.send(made) {
        drop(unheard);
    }
}

/// Submits `commands` under error scopes of their own, and keeps the index
/// in `newest` when wgpu raised no error.
fn submit_now(device: &wgpu::Device, queue: &wgpu::Queue, newest: &ArcSwapOption<wgpu::SubmissionIndex>, commands: Vec<wgpu::CommandBuffer>) -> Reply {
    match catch_unwind(AssertUnwindSafe(|| captured(device, || queue.submit(commands)))) {
        Ok((index, error)) => {
            if error.is_none() {
                newest.store(Some(Arc::new(index.clone())));
            }
            Ok((index, error))
        }
        Err(payload) => Err(Failed::Panicked(panic_text(payload))),
    }
}

/// Submits what `jobs` brings, in order, looking for the next job for
/// [`LOOK`] after each before it sleeps; on WARP, given the device's id
/// as `warp`, through the process's [`WARP`] turn. It ends when the
/// device, and with it the sender, is dropped.
fn submit_jobs(device: &wgpu::Device, queue: &wgpu::Queue, newest: &ArcSwapOption<wgpu::SubmissionIndex>, warp: Option<u64>, jobs: mpsc::Receiver<Submission>) {
    loop {
        let job = match look_for(&jobs) {
            Looked::Came(job) => job,
            Looked::Gone => return,
            Looked::Quiet => match jobs.recv() {
                Ok(job) => job,
                Err(mpsc::RecvError) => return,
            },
        };
        submit_one(device, queue, newest, warp.map(|id| (&WARP, id)), job);
    }
}

pub(crate) type CoreRef = Arc<Core>;

/// What a device's callbacks report from whichever thread wgpu runs them
/// on: the errors raised outside any scope, and why the device was lost.
#[derive(Default)]
struct Reports {
    errors: ArcSwap<Vec<String>>,
    lost: ArcSwapOption<String>,
}

impl Reports {
    fn push_error(&self, text: String) {
        self.errors.rcu(|errors| {
            let mut next = Vec::clone(errors);
            next.push(text.clone());
            next
        });
    }
}

static NEXT_DEVICE: AtomicU64 = AtomicU64::new(1);

// A PowerShell class only with the module, the one build that raises the
// events.
/// What Wgpu.DeviceLost and Wgpu.UncapturedError carry as their
/// MessageData.
#[cfg_attr(feature = "module", psclass(name = "Wgpu.DeviceEvent", columns = ["Device", "Kind", "Message"]))]
#[derive(Default, Clone)]
pub struct DeviceEvent {
    /// The label of the device the event came from.
    pub device: String,
    /// Destroyed or Unknown for a lost device; Validation, OutOfMemory or
    /// Internal for an error.
    pub kind: String,
    /// What wgpu reported; empty for a destroyed device.
    pub message: String,
}

/// Raises an engine event, and records a raise that failed among the
/// device's errors so the next call reports it.
#[cfg(feature = "module")]
fn raise(events: &Option<PsEvents>, reports: &Reports, source: &str, event: DeviceEvent) {
    if let Some(events) = events
        && let Err(e) = events.raise(source, event)
    {
        reports.push_error(format!("the {source} event could not be raised: {}", e.message));
    }
}

/// Without the module a device has no PowerShell events: `events` is
/// always `None`, so there is nothing to raise.
#[cfg(not(feature = "module"))]
fn raise(_events: &Option<PsEvents>, _reports: &Reports, _source: &str, _event: DeviceEvent) {}

/// Opens a device with every feature and limit its adapter supports, or
/// the features `o` names. `events` receives Wgpu.DeviceLost and
/// Wgpu.UncapturedError.
pub(crate) fn open(o: &OpenOptions, events: Option<PsEvents>) -> PsResult<CoreRef> {
    let adapter = find_adapter(&adapters(&o.instance)?, &o.adapter)?;
    let supported = adapter.features();
    let experimental_mask = wgpu::Features::all_experimental_mask();
    let required = match &o.features {
        None if o.experimental => supported,
        None => supported - experimental_mask,
        Some(list) => {
            let mut wanted = wgpu::Features::empty();
            for name in list {
                let feature = names::parse_feature(name)?;
                if !supported.contains(feature) {
                    return Err(invalid("WgpuFeatureUnsupported", format!("{} does not support the feature {name}", adapter.get_info().name)));
                }
                if experimental_mask.contains(feature) && !o.experimental {
                    return Err(invalid("WgpuFeatureExperimental", format!("{name} is experimental; add -Experimental to request it")));
                }
                wanted |= feature;
            }
            wanted
        }
    };
    let experimental_features = if o.experimental {
        // The caller asked for wgpu's experimental features, whose hazards
        // wgpu documents and -Experimental names.
        unsafe { wgpu::ExperimentalFeatures::enabled() }
    } else {
        wgpu::ExperimentalFeatures::disabled()
    };
    let id = NEXT_DEVICE.fetch_add(1, Ordering::Relaxed);
    let label = match &o.label {
        Some(label) => label.clone(),
        None => format!("Wgpu device {id}"),
    };
    let trace = match &o.trace {
        Some(path) => wgpu::Trace::Directory(path.clone()),
        None => wgpu::Trace::Off,
    };
    let memory_hints = match o.memory {
        Some(MemoryHint::MemoryUsage) => wgpu::MemoryHints::MemoryUsage,
        Some(MemoryHint::Performance) | None => wgpu::MemoryHints::Performance,
    };
    let limits = adapter.limits();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some(&label),
        required_features: required,
        required_limits: limits.clone(),
        experimental_features,
        memory_hints,
        trace,
    }))
    .map_err(|e| device_error("WgpuDevice", "the device cannot be opened", e))?;
    adopt(id, adapter, device, queue, label, limits, events)
}

/// A device another crate opened, as the core a Gpu holds: with the
/// callbacks and the submission thread a device this module opens has,
/// the limits it was opened with, and no PowerShell events.
pub(crate) fn adopt_device(adapter: wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue, label: String) -> PsResult<CoreRef> {
    let id = NEXT_DEVICE.fetch_add(1, Ordering::Relaxed);
    let limits = device.limits();
    adopt(id, adapter, device, queue, label, limits, None)
}

/// The core of an open device: its uncaptured-error and device-lost
/// callbacks, which keep what they report for the next call and raise
/// `events`; its submission thread, none on OpenGL; and the features it
/// has and the `limits` it was opened with.
fn adopt(id: u64, adapter: wgpu::Adapter, device: wgpu::Device, queue: wgpu::Queue, label: String, limits: wgpu::Limits, events: Option<PsEvents>) -> PsResult<CoreRef> {
    let reports = Arc::new(Reports::default());
    {
        let reports = reports.clone();
        let events = events.clone();
        let device_label = label.clone();
        device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
            let kind = match &e {
                wgpu::Error::OutOfMemory { .. } => "OutOfMemory",
                wgpu::Error::Validation { .. } => "Validation",
                wgpu::Error::Internal { .. } => "Internal",
            };
            let text = e.to_string();
            raise(&events, &reports, "Wgpu.UncapturedError", DeviceEvent { device: device_label.clone(), kind: kind.to_string(), message: text.clone() });
            reports.push_error(text);
        }));
    }
    {
        let reports = reports.clone();
        let device_label = label.clone();
        device.set_device_lost_callback(move |reason, message| {
            let kind = format!("{reason:?}");
            let text = if message.is_empty() { kind.clone() } else { format!("{kind}: {message}") };
            raise(&events, &reports, "Wgpu.DeviceLost", DeviceEvent { device: device_label.clone(), kind, message });
            reports.lost.store(Some(Arc::new(text)));
        });
    }
    let info = adapter.get_info();
    let newest = Arc::new(ArcSwapOption::const_empty());
    let jobs = if info.backend == wgpu::Backend::Gl {
        None
    } else {
        let (jobs, queued) = mpsc::channel();
        let warp = is_warp(&info).then_some(id);
        let started = {
            let (device, queue, newest) = (device.clone(), queue.clone(), newest.clone());
            std::thread::Builder::new().name("Wgpu submit".to_string()).spawn(move || submit_jobs(&device, &queue, &newest, warp, queued))
        };
        match started {
            Ok(_detached) => Some(jobs),
            Err(e) => return Err(device_error("WgpuDevice", "the device's submission thread cannot start", e)),
        }
    };
    Ok(Arc::new(Core {
        id,
        features: device.features(),
        limits,
        adapter,
        device,
        queue,
        info,
        label,
        reports,
        bc_encoders: std::array::from_fn(|_| ArcSwapOption::const_empty()),
        newest,
        jobs,
    }))
}

impl Core {
    /// Fails when the device was lost or raised errors outside any scope
    /// since the last check, which this reports and clears.
    pub(crate) fn check(&self) -> PsResult<()> {
        if let Some(reason) = self.lost_reason() {
            return Err(device_error("WgpuDeviceLost", "the device was lost", reason));
        }
        if self.reports.errors.load().is_empty() {
            return Ok(());
        }
        let taken = self.take_errors();
        if taken.is_empty() {
            return Ok(());
        }
        Err(device_error("WgpuUncaptured", "the device reported", taken.join("; ")))
    }

    /// The errors the device raised outside any scope, not yet reported,
    /// oldest first; they are cleared, so no later check reports them.
    pub(crate) fn take_errors(&self) -> Vec<String> {
        self.reports.errors.swap(Arc::new(Vec::new())).to_vec()
    }

    /// The reason the device was lost, if it was.
    pub(crate) fn lost_reason(&self) -> Option<String> {
        self.reports.lost.load_full().map(|reason| reason.to_string())
    }

    /// The errors the device raised outside any scope, not yet reported.
    pub(crate) fn pending_errors(&self) -> Vec<String> {
        self.reports.errors.load_full().to_vec()
    }

    /// Runs `f` with every validation, out-of-memory and internal error it
    /// raises captured, and fails with the first, under `id`.
    pub(crate) fn scoped<T>(&self, id: &'static str, what: &str, f: impl FnOnce() -> T) -> PsResult<T> {
        self.check()?;
        let (value, error) = captured(&self.device, f);
        if let Some(e) = error {
            return Err(device_error(id, what, e));
        }
        self.check()?;
        Ok(value)
    }

    /// Polls until the GPU has finished submission `index` and every one
    /// before it, in 100 ms steps so an interrupt or a lost device ends the
    /// wait, or polls once without waiting when `index` is `None`. The
    /// callbacks of finished work run inside the polls, a lost device's
    /// included.
    fn drain(&self, index: Option<wgpu::SubmissionIndex>, stop: Option<&dyn Interrupt>) -> PsResult<()> {
        let index = match index {
            Some(index) => index,
            None => {
                return match self.device.poll(wgpu::PollType::Poll) {
                    Ok(_) => Ok(()),
                    Err(e) => Err(device_error("WgpuPoll", "waiting for the device failed", e)),
                };
            }
        };
        loop {
            match self.device.poll(wgpu::PollType::Wait { submission_index: Some(index.clone()), timeout: Some(Duration::from_millis(100)) }) {
                Ok(_) => return Ok(()),
                Err(wgpu::PollError::Timeout) => {
                    if self.lost_reason().is_some() {
                        return Ok(());
                    }
                    if stop.is_some_and(|s| s.interrupted()) {
                        return Err(tasks::stopped());
                    }
                }
                Err(e) => return Err(device_error("WgpuPoll", "waiting for the device failed", e)),
            }
        }
    }

    /// The index the module's newest submission on this device returned.
    fn newest(&self) -> Option<wgpu::SubmissionIndex> {
        self.newest.load_full().map(|index| wgpu::SubmissionIndex::clone(&index))
    }

    /// Waits until the GPU has finished every submission, and reports any
    /// error the device raised. An empty submission, which the submission
    /// thread makes after every submission handed to it before, marks the
    /// point to wait for.
    pub(crate) fn wait(&self, stop: Option<&dyn Interrupt>) -> PsResult<()> {
        let index = self.submit(Vec::new(), stop)?;
        self.drain(Some(index), stop)?;
        self.check()
    }

    /// Waits until the GPU has finished submission `index`, and reports
    /// any error the device raised.
    pub(crate) fn wait_for(&self, index: &wgpu::SubmissionIndex, stop: Option<&dyn Interrupt>) -> PsResult<()> {
        self.drain(Some(index.clone()), stop)?;
        self.check()
    }

    /// Polls until `settled` holds, waiting for submission `index`, then
    /// reports any error the device raised. What `settled` reads is set by
    /// a callback, which runs inside whichever thread's poll collects it,
    /// so a finished submission does not mean it has run. The callback
    /// unparks the thread waiting here, which otherwise looks again every
    /// 100 ms. An interrupt or a lost device ends the wait.
    pub(crate) fn wait_until(&self, index: &wgpu::SubmissionIndex, stop: Option<&dyn Interrupt>, settled: impl Fn() -> bool) -> PsResult<()> {
        loop {
            if settled() || self.lost_reason().is_some() {
                return self.check();
            }
            if stop.is_some_and(|s| s.interrupted()) {
                return Err(tasks::stopped());
            }
            match self.device.poll(wgpu::PollType::Wait { submission_index: Some(index.clone()), timeout: Some(Duration::from_millis(100)) }) {
                Ok(_) => {
                    if !settled() {
                        std::thread::park_timeout(Duration::from_millis(100));
                    }
                }
                Err(wgpu::PollError::Timeout) => {}
                Err(e) => return Err(device_error("WgpuPoll", "waiting for the device failed", e)),
            }
        }
    }

    /// Hands command buffers to the device's submission thread, which
    /// submits them after every submission handed to it before, and
    /// returns at once; an OpenGL device submits them here. An invalid
    /// submission is reported as `id` with `what` when the submission is
    /// waited for.
    pub(crate) fn begin_submit(&self, id: &'static str, what: &'static str, commands: Vec<wgpu::CommandBuffer>) -> PsResult<Pending> {
        self.check()?;
        let (reply, answer) = mpsc::sync_channel(1);
        let job = Submission { commands, reply };
        match &self.jobs {
            Some(jobs) => {
                if let Err(e) = jobs.send(job) {
                    return Err(device_error("WgpuSubmitThread", "the device's submission thread has ended", e));
                }
            }
            None => submit_one(&self.device, &self.queue, &self.newest, None, job),
        }
        Ok(Pending { id, what, reply: answer })
    }

    /// Submits command buffers through the device's submission thread,
    /// reporting an invalid submission as `id` with `what`, and returns
    /// once they are submitted, not once the GPU has run them. An
    /// interrupt ends the wait.
    pub(crate) fn submit_as(&self, id: &'static str, what: &'static str, commands: Vec<wgpu::CommandBuffer>, stop: Option<&dyn Interrupt>) -> PsResult<wgpu::SubmissionIndex> {
        self.begin_submit(id, what, commands)?.wait(self, stop)
    }

    /// Submits command buffers and reports an invalid submission, without
    /// waiting for the GPU to run them. Returns the submission's index.
    pub(crate) fn submit(&self, commands: Vec<wgpu::CommandBuffer>, stop: Option<&dyn Interrupt>) -> PsResult<wgpu::SubmissionIndex> {
        self.submit_as("WgpuSubmit", "the submission is invalid", commands, stop)
    }

    /// Submits command buffers, waits for them, and reports any error.
    pub(crate) fn submit_and_wait(&self, commands: Vec<wgpu::CommandBuffer>, stop: Option<&dyn Interrupt>) -> PsResult<()> {
        let index = self.submit(commands, stop)?;
        self.wait_for(&index, stop)
    }

    /// Destroys the device and waits for its queue to empty, when wgpu
    /// reports it lost and raises Wgpu.DeviceLost.
    pub(crate) fn destroy(&self, stop: Option<&dyn Interrupt>) -> PsResult<()> {
        self.device.destroy();
        self.drain(self.newest(), stop)
    }
}

static DEFAULT: ArcSwapOption<Core> = ArcSwapOption::const_empty();

/// The module's default device, opened with every feature on the adapter
/// wgpu prefers when no device is open yet or the last one was lost. Of
/// two callers opening one at once, both get the device stored first.
pub(crate) fn default_core(ps: Option<&Pipeline<'_>>) -> PsResult<CoreRef> {
    if let Some(core) = DEFAULT.load_full()
        && core.lost_reason().is_none()
    {
        return Ok(core);
    }
    let events = match ps {
        Some(ps) => Some(ps.events()?),
        None => None,
    };
    let opened = open(&OpenOptions::default(), events)?;
    let mut chosen = opened.clone();
    DEFAULT.rcu(|current| {
        chosen = match current {
            Some(core) if core.lost_reason().is_none() => core.clone(),
            _ => opened.clone(),
        };
        Some(chosen.clone())
    });
    Ok(chosen)
}

pub(crate) fn set_default(core: Option<CoreRef>) {
    DEFAULT.store(core);
}

/// Stops `core` being the default device, if it is.
fn release_if_default(core: &Core) {
    DEFAULT.rcu(|current| match current {
        Some(d) if d.id == core.id => None,
        other => other.clone(),
    });
}

fn is_default(core: &Core) -> bool {
    DEFAULT.load_full().is_some_and(|d| d.id == core.id)
}

/// The device a cmdlet's `-Device` names, or the default device.
pub(crate) fn core_for(device: &Option<PsProxy<Device>>, ps: &Pipeline<'_>) -> PsResult<CoreRef> {
    match device {
        Some(d) => d.with(|d| d.core.clone()),
        None => default_core(Some(ps)),
    }
}

/// Completes -Feature with every feature name wgpu knows.
#[completer(cmdlet = "Open-WgpuDevice", parameter = "Feature")]
pub(crate) fn complete_feature(ctx: &CompletionContext) -> PsResult<Vec<Completion>> {
    let prefix = ctx.word.to_lowercase();
    Ok(names::all_feature_names().into_iter().filter(|n| n.to_lowercase().starts_with(&prefix)).map(Completion::value).collect())
}

/// Releases the default device and the module's instances once no runspace
/// of the process has the module, and in the old copy of a reloaded library.
/// A runspace that removes the module, or closes with it imported, leaves
/// both to the runspaces that still have it.
#[on_unload]
pub(crate) fn release_default_device() -> PsResult<()> {
    set_default(None);
    INSTANCES.store(None);
    Ok(())
}

/// A device's state now: whether it was lost, and the errors it raised
/// outside any call that have not been reported yet.
#[psclass(name = "Wgpu.DeviceStatus", columns = ["IsLost", "LostReason", "PendingErrors", "IsDefault"])]
#[derive(Default, Clone)]
pub struct DeviceStatus {
    /// Whether the device was lost; a lost device refuses every call.
    pub is_lost: bool,
    /// Why it was lost; `$null` while it is not.
    pub lost_reason: Option<String>,
    /// Errors raised outside any call, which the device's next call reports.
    pub pending_errors: Vec<String>,
    /// Whether this is the device cmdlets use when given no -Device.
    pub is_default: bool,
}

/// How many objects of each kind a device holds and the memory they take,
/// as the backend counts them.
#[psclass(name = "Wgpu.Counters")]
#[derive(Default, Clone)]
pub struct Counters {
    /// Buffers the device holds now, staging buffers among them.
    pub buffers: i64,
    /// Textures the device holds now.
    pub textures: i64,
    /// Texture views the device holds now.
    pub texture_views: i64,
    /// Bind groups the device holds now.
    pub bind_groups: i64,
    /// Bind group layouts the device holds now.
    pub bind_group_layouts: i64,
    /// Render pipelines the device holds now.
    pub render_pipelines: i64,
    /// Compute pipelines the device holds now.
    pub compute_pipelines: i64,
    /// Ray tracing pipelines the device holds now.
    pub ray_tracing_pipelines: i64,
    /// Pipeline layouts the device holds now.
    pub pipeline_layouts: i64,
    /// Samplers the device holds now.
    pub samplers: i64,
    /// Command encoders the device holds now, finished or not.
    pub command_encoders: i64,
    /// Shader modules the device holds now, one for each pipeline's shader.
    pub shader_modules: i64,
    /// Query sets the device holds now.
    pub query_sets: i64,
    /// Fences the device holds now, which track submissions.
    pub fences: i64,
    /// Bytes of GPU memory attributed to buffers.
    pub buffer_memory: i64,
    /// Bytes of GPU memory attributed to textures.
    pub texture_memory: i64,
    /// Bytes of GPU memory attributed to acceleration structures.
    pub acceleration_structure_memory: i64,
    /// Number of GPU memory allocations.
    pub memory_allocations: i64,
}

/// The backend's report of a device's GPU memory.
#[psclass(name = "Wgpu.AllocatorReport", columns = ["TotalAllocatedBytes", "TotalReservedBytes", "Allocations", "Blocks"])]
#[derive(Default, Clone)]
pub struct AllocatorReport {
    /// Bytes allocated to resources.
    pub total_allocated_bytes: u64,
    /// Bytes reserved from the driver, allocated or not.
    pub total_reserved_bytes: u64,
    /// Number of allocations.
    pub allocations: u64,
    /// Number of memory blocks the allocations live in.
    pub blocks: u64,
}

/// A GPU device held in Rust. Buffers, shaders, pipelines and everything
/// else are made on it, and keep it alive while any of them lives.
#[psclass(name = "Wgpu.Device", mode = proxy, columns = ["Adapter", "Backend", "DeviceType", "Label"])]
pub struct Device {
    /// The adapter's name.
    pub adapter: String,
    /// The graphics API the device is reached through.
    pub backend: Backend,
    /// DiscreteGpu, IntegratedGpu, VirtualGpu, Cpu or Other.
    pub device_type: DeviceType,
    /// The label the device was opened with.
    pub label: String,
    /// Every optional feature the device has.
    pub features: Vec<String>,
    /// Every limit of the device.
    pub limits: Limits,
    /// The largest buffer the device creates, in bytes.
    pub max_buffer_size: u64,
    /// The largest storage buffer one binding can see, in bytes.
    pub max_storage_buffer_binding_size: u64,
    #[psfield(skip)]
    pub(crate) core: CoreRef,
}

impl Device {
    pub(crate) fn from_core(core: CoreRef) -> Device {
        Device {
            adapter: core.info.name.clone(),
            backend: Backend::from_wgpu(core.info.backend),
            device_type: DeviceType::from_wgpu(core.info.device_type),
            label: core.label.clone(),
            features: names::feature_names(core.features),
            limits: Limits::from(&core.limits),
            max_buffer_size: core.limits.max_buffer_size,
            max_storage_buffer_binding_size: core.limits.max_storage_buffer_binding_size,
            core,
        }
    }

    /// The recordings of `commands`, taken for this device's queue.
    fn take(&self, commands: &[PsProxy<CommandBuffer>]) -> PsResult<Vec<wgpu::CommandBuffer>> {
        let mut taken = Vec::with_capacity(commands.len());
        for c in commands {
            taken.push(c.with_mut(|c| c.take_for(&self.core))??);
        }
        Ok(taken)
    }
}

#[psmethods]
impl Device {
    /// Opens a device on the adapter whose name contains `adapter`, or the
    /// one wgpu prefers, through `backend` when given.
    pub fn new(adapter: Option<String>, backend: Option<Backend>) -> PsResult<Self> {
        let options = OpenOptions {
            instance: InstanceOptions { backends: backend.map(|b| vec![b]), ..Default::default() },
            adapter: AdapterQuery { name: adapter, ..Default::default() },
            ..Default::default()
        };
        Ok(Device::from_core(open(&options, None)?))
    }

    /// The device cmdlets use when given no -Device, opened now if none is.
    pub fn get_default() -> PsResult<Device> {
        Ok(Device::from_core(default_core(None)?))
    }

    /// Makes this the device cmdlets use when given no -Device.
    pub fn set_default(&self) -> PsResult<()> {
        set_default(Some(self.core.clone()));
        Ok(())
    }

    /// Copies `data`, an array of any numeric type, to a new buffer, as
    /// `elementType` when given and as its own type otherwise.
    pub fn upload(&self, data: PsObject, element_type: Option<ElementType>) -> PsResult<Buffer> {
        buffer::upload(&self.core, &data, element_type, None, None)
    }

    /// A new buffer of `length` zeroed elements of `elementType`, Float32
    /// when omitted.
    pub fn zeros(&self, length: u64, element_type: Option<ElementType>) -> PsResult<Buffer> {
        buffer::zeroed(&self.core, length, element_type, None, None)
    }

    /// A new zeroed buffer of `length` elements with the usages given, or
    /// every usage but mapping when none are.
    pub fn create_buffer(&self, length: u64, element_type: Option<ElementType>, usage: Option<Vec<BufferUsage>>, label: Option<String>) -> PsResult<Buffer> {
        buffer::zeroed(&self.core, length, element_type, usage, label)
    }

    /// Parses and validates a shader; the same as `[Wgpu.Shader]::new`.
    pub fn create_shader(&self, source: String, language: Option<ShaderLanguage>, stage: Option<ShaderStage>) -> PsResult<Shader> {
        Shader::new(source, language, stage)
    }

    /// A compute pipeline for `shader` (WGSL text or a Wgpu.Shader), with
    /// `entry` as its entry point and `constants` as its override values.
    pub fn create_compute_pipeline(&self, shader: PsObject, entry: Option<String>, constants: Option<PsHashtable>, label: Option<String>) -> PsResult<ComputePipeline> {
        crate::pipeline::create(&self.core, &shader, entry.as_deref(), constants.as_ref(), label)
    }

    /// The bind group `group` of `pipeline`, binding `resources`: an array
    /// in binding order, or a hashtable by binding name or "group:binding".
    pub fn create_bind_group(&self, pipeline: PsProxy<ComputePipeline>, group: u32, resources: PsObject) -> PsResult<BindGroup> {
        crate::binding::bind_group(&pipeline, group, &resources)
    }

    /// A command encoder to record copies and passes into.
    pub fn create_command_encoder(&self, label: Option<String>) -> PsResult<CommandEncoder> {
        CommandEncoder::open(&self.core, label)
    }

    /// A set of `count` queries of `kind`.
    pub fn create_query_set(&self, kind: QueryKind, count: u32, label: Option<String>) -> PsResult<QuerySet> {
        QuerySet::create(&self.core, kind, count, label)
    }

    /// Submits finished command buffers and waits for the GPU to run them.
    pub fn submit(&self, commands: Vec<PsProxy<CommandBuffer>>) -> PsResult<()> {
        self.core.submit_and_wait(self.take(&commands)?, None)
    }

    /// Submits finished command buffers and returns at once with a task
    /// that ends when the GPU has run them.
    pub fn submit_async(&self, commands: Vec<PsProxy<CommandBuffer>>, task: PsTask<()>) -> PsResult<()> {
        let pending = self.core.begin_submit("WgpuSubmit", "the submission is invalid", self.take(&commands)?)?;
        let core = self.core.clone();
        tasks::spawn(
            task,
            move |stop| {
                let index = pending.wait(&core, Some(stop))?;
                core.wait_for(&index, Some(stop))
            },
            Ok,
        )
    }

    /// Waits until the GPU has finished all submitted work.
    pub fn wait(&self) -> PsResult<()> {
        self.core.wait(None)
    }

    /// Returns at once with a task that ends when the GPU has finished all
    /// work submitted so far.
    pub fn wait_async(&self, task: PsTask<()>) -> PsResult<()> {
        let core = self.core.clone();
        tasks::spawn(task, move |stop| core.wait(Some(stop)), Ok)
    }

    /// Runs a compute shader once. `shader` is WGSL text, a Wgpu.Shader or a
    /// Wgpu.ComputePipeline; `resources` bind as Invoke-WgpuCompute binds
    /// them; `x`, `y` and `z` count workgroups.
    pub fn run(&self, shader: PsObject, resources: PsObject, x: u32, y: Option<u32>, z: Option<u32>, entry: Option<String>) -> PsResult<()> {
        let request = crate::compute::Request {
            shader,
            resources: Some(resources),
            entry,
            dispatch: crate::compute::Dispatch::Workgroups([x, y.unwrap_or(1), z.unwrap_or(1)]),
            ..Default::default()
        };
        crate::compute::run(&self.core, &request, None).map(|_| ())
    }

    /// Runs a compute shader once, as Run does, and returns at once with a
    /// task that ends when the GPU has finished it.
    // clippy::too_many_arguments: the arguments are Run's and the task the
    // method settles, one PowerShell method argument each.
    #[allow(clippy::too_many_arguments)]
    pub fn run_async(&self, shader: PsObject, resources: PsObject, x: u32, y: Option<u32>, z: Option<u32>, entry: Option<String>, task: PsTask<()>) -> PsResult<()> {
        let request = crate::compute::Request {
            shader,
            resources: Some(resources),
            entry,
            dispatch: crate::compute::Dispatch::Workgroups([x, y.unwrap_or(1), z.unwrap_or(1)]),
            ..Default::default()
        };
        let submitted = crate::compute::submit(&self.core, &request)?;
        tasks::spawn(task, move |stop| submitted.finish(Some(stop)).map(|_| ()), Ok)
    }

    /// Copies a buffer back from the device and returns its values.
    pub fn download(&self, buffer: PsProxy<Buffer>) -> PsResult<PsObject> {
        buffer.with(|b| b.read_all(None))?
    }

    /// Copies a buffer back on a thread of its own and returns at once
    /// with a task whose result is its values.
    pub fn download_async(&self, buffer: PsProxy<Buffer>, task: PsTask<PsObject>) -> PsResult<()> {
        buffer.with(|b| b.read_async(None, task))?
    }

    /// Whether the device was lost and what it raised outside any call.
    pub fn status(&self) -> PsResult<DeviceStatus> {
        let lost_reason = self.core.lost_reason();
        Ok(DeviceStatus { is_lost: lost_reason.is_some(), lost_reason, pending_errors: self.core.pending_errors(), is_default: is_default(&self.core) })
    }

    /// The backend's object and memory counts for this device.
    pub fn counters(&self) -> PsResult<Counters> {
        let c = self.core.device.get_internal_counters().hal;
        Ok(Counters {
            buffers: c.buffers.read() as i64,
            textures: c.textures.read() as i64,
            texture_views: c.texture_views.read() as i64,
            bind_groups: c.bind_groups.read() as i64,
            bind_group_layouts: c.bind_group_layouts.read() as i64,
            render_pipelines: c.render_pipelines.read() as i64,
            compute_pipelines: c.compute_pipelines.read() as i64,
            ray_tracing_pipelines: c.ray_tracing_pipelines.read() as i64,
            pipeline_layouts: c.pipeline_layouts.read() as i64,
            samplers: c.samplers.read() as i64,
            command_encoders: c.command_encoders.read() as i64,
            shader_modules: c.shader_modules.read() as i64,
            query_sets: c.query_sets.read() as i64,
            fences: c.fences.read() as i64,
            buffer_memory: c.buffer_memory.read() as i64,
            texture_memory: c.texture_memory.read() as i64,
            acceleration_structure_memory: c.acceleration_structure_memory.read() as i64,
            memory_allocations: c.memory_allocations.read() as i64,
        })
    }

    /// The backend's memory report, or `$null` where the backend keeps none.
    pub fn allocator_report(&self) -> PsResult<Option<AllocatorReport>> {
        Ok(self.core.device.generate_allocator_report().map(|r| AllocatorReport {
            total_allocated_bytes: r.total_allocated_bytes,
            total_reserved_bytes: r.total_reserved_bytes,
            allocations: r.allocations.len() as u64,
            blocks: r.blocks.len() as u64,
        }))
    }

    /// Nanoseconds per timestamp tick on this device's queue.
    pub fn timestamp_period(&self) -> PsResult<f32> {
        Ok(self.core.queue.get_timestamp_period())
    }

    /// Starts a capture in an attached RenderDoc or Xcode; does nothing
    /// without one. One capture at a time, which StopCapture ends.
    pub fn start_capture(&self) -> PsResult<()> {
        // wgpu's capture contract: one capture at a time, ended by
        // stop_capture.
        unsafe { self.core.device.start_graphics_debugger_capture() };
        Ok(())
    }

    /// Ends the capture StartCapture began.
    pub fn stop_capture(&self) -> PsResult<()> {
        // Ends the capture start_capture began.
        unsafe { self.core.device.stop_graphics_debugger_capture() };
        Ok(())
    }

    /// Destroys the device: every object made on it stops working, and the
    /// device reports itself lost once the work it was given has finished.
    pub fn destroy(&self) -> PsResult<()> {
        release_if_default(&self.core);
        self.core.destroy(None)
    }
}

/// Lists the GPU adapters wgpu finds, with every feature and limit each
/// supports.
///
/// # Examples
/// Get-WgpuAdapter
/// Get-WgpuAdapter -Name nvidia -Backend Vulkan
/// (Get-WgpuAdapter)[0].Features
#[cmdlet(verb = "Get", noun = "WgpuAdapter", alias = ["Get-GpuAdapter"], output = ["Wgpu.Adapter"])]
#[derive(Default)]
pub struct GetWgpuAdapter {
    /// Only adapters whose name contains this text.
    #[param(position = 0)]
    pub name: Option<String>,
    /// Only adapters reached through these backends; without it, Vulkan and DX12 on Windows and every backend elsewhere. Noop adds wgpu's adapter that runs nothing.
    #[param]
    pub backend: Option<Vec<Backend>>,
}

impl Cmdlet for GetWgpuAdapter {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let options = InstanceOptions { backends: self.backend.clone(), ..Default::default() };
        let needle = self.name.as_deref().map(str::to_lowercase);
        for adapter in adapters(&options)?.iter() {
            if needle.as_deref().is_some_and(|n| !adapter.get_info().name.to_lowercase().contains(n)) {
                continue;
            }
            ps.write(describe(adapter))?;
        }
        Ok(())
    }
}

/// Opens a GPU device with every feature and limit its adapter supports.
///
/// The device is held in Rust; buffers, shaders and pipelines are made on
/// it. -Default makes it the device cmdlets use when given no -Device.
///
/// # Examples
/// $gpu = Open-WgpuDevice
/// Open-WgpuDevice -Adapter nvidia -Backend Vulkan -Default
/// Open-WgpuDevice -Feature ShaderF16, TimestampQuery
/// Open-WgpuDevice -Fallback
#[cmdlet(verb = "Open", noun = "WgpuDevice", alias = ["Open-GpuDevice"], output = ["Wgpu.Device"])]
#[derive(Default)]
pub struct OpenWgpuDevice {
    /// Open the first adapter whose name contains this text; without it, wgpu picks one.
    #[param(position = 0)]
    pub adapter: Option<String>,
    /// Look for adapters through these backends only; without it, Vulkan and DX12 on Windows and every backend elsewhere. On Windows with NVIDIA's driver, a thread that used Gl deadlocks the process if it exits while other threads open or close devices.
    #[param]
    pub backend: Option<Vec<Backend>>,
    /// When wgpu picks, prefer the low-power adapter over the high-performance one.
    #[param]
    pub low_power: bool,
    /// When wgpu picks, take its software adapter (WARP, lavapipe or llvmpipe).
    #[param]
    pub fallback: bool,
    /// Request these features only, by name; without it every feature the adapter has is requested.
    #[param]
    pub feature: Option<Vec<String>>,
    /// Allow wgpu's experimental features (ray queries, mesh shaders, cooperative matrices), which wgpu documents as unfinished.
    #[param]
    pub experimental: bool,
    /// The device's label, shown in errors and in graphics debuggers.
    #[param]
    pub label: Option<String>,
    /// Write an API trace of everything done on the device into this folder, which must exist.
    #[param]
    pub trace_path: Option<String>,
    /// Favor speed (Performance) or memory use (MemoryUsage) in the backend's allocator.
    #[param]
    pub memory_hint: Option<MemoryHint>,
    /// Turn on the backend's validation layers and debug labels.
    #[param]
    pub validation: bool,
    /// Turn on the backend's GPU-based validation as well, which is slower.
    #[param]
    pub gpu_based_validation: bool,
    /// Make the device the one cmdlets use when given no -Device.
    #[param]
    pub default: bool,
}

impl Cmdlet for OpenWgpuDevice {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let trace = match &self.trace_path {
            Some(path) => {
                let resolved = ps.resolve_path(path, true)?;
                let folder = resolved.into_iter().next().filter(|p| Path::new(p).is_dir()).ok_or_else(|| invalid("WgpuTracePath", format!("{path} names no folder")))?;
                Some(PathBuf::from(folder))
            }
            None => None,
        };
        let options = OpenOptions {
            instance: InstanceOptions { backends: self.backend.clone(), validation: self.validation, gpu_based_validation: self.gpu_based_validation },
            adapter: AdapterQuery { name: self.adapter.clone(), low_power: self.low_power, fallback: self.fallback },
            features: self.feature.clone(),
            experimental: self.experimental,
            label: self.label.clone(),
            trace,
            memory: self.memory_hint,
        };
        let core = open(&options, Some(ps.events()?))?;
        if self.default {
            set_default(Some(core.clone()));
        }
        ps.write(Device::from_core(core))
    }
}

/// Gets the device cmdlets use when given no -Device, opening one on the
/// adapter wgpu prefers if none is open.
///
/// # Examples
/// Get-WgpuDevice
/// (Get-WgpuDevice).Features
#[cmdlet(verb = "Get", noun = "WgpuDevice", alias = ["Get-GpuDevice"], output = ["Wgpu.Device"])]
#[derive(Default)]
pub struct GetWgpuDevice {}

impl Cmdlet for GetWgpuDevice {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        ps.write(Device::from_core(default_core(Some(ps))?))
    }
}

/// Destroys a device: every object made on it stops working.
///
/// It returns once the work already given to the device has finished,
/// when the device reports itself lost and raises Wgpu.DeviceLost. Ctrl+C
/// ends that wait.
///
/// # Examples
/// $gpu | Close-WgpuDevice
/// Close-WgpuDevice
#[cmdlet(verb = "Close", noun = "WgpuDevice", alias = ["Close-GpuDevice"], supports_should_process)]
#[derive(Default)]
pub struct CloseWgpuDevice {
    /// The device to destroy; the default device when omitted.
    #[param(position = 0, value_from_pipeline)]
    pub device: Option<PsProxy<Device>>,
}

impl Cmdlet for CloseWgpuDevice {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let core = match &self.device {
            Some(d) => d.with(|d| d.core.clone())?,
            None => match DEFAULT.load_full() {
                Some(core) => core,
                None => return Err(invalid_operation("WgpuNoDevice", "no device is open")),
            },
        };
        if !ps.should_process(&core.label, "Destroy the device")? {
            return Ok(());
        }
        release_if_default(&core);
        core.destroy(Some(ps))
    }
}

/// Waits until a device has finished all the work submitted to it.
///
/// Ctrl+C ends the wait; the GPU finishes the work on its own.
///
/// # Examples
/// Wait-WgpuDevice
/// $gpu | Wait-WgpuDevice
#[cmdlet(verb = "Wait", noun = "WgpuDevice", alias = ["Wait-GpuDevice"])]
#[derive(Default)]
pub struct WaitWgpuDevice {
    /// The device to wait on; the default device when omitted.
    #[param(position = 0, value_from_pipeline)]
    pub device: Option<PsProxy<Device>>,
}

impl Cmdlet for WaitWgpuDevice {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        core_for(&self.device, ps)?.wait(Some(ps))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    /// An interrupt a test sets.
    struct Flag(AtomicBool);

    impl Interrupt for Flag {
        fn interrupted(&self) -> bool {
            self.0.load(Ordering::Acquire)
        }
    }

    /// The device the tests run on: `None`, with the reason printed, only
    /// where this host has no adapter.
    fn core() -> Option<CoreRef> {
        crate::faults::install();
        if adapters(&InstanceOptions::default()).expect("the adapters").is_empty() {
            eprintln!("skipped: this host has no GPU adapter");
            return None;
        }
        let name = match std::env::var("WGPU_PWSH_TEST_ADAPTER") {
            Ok(name) => Some(name),
            Err(std::env::VarError::NotPresent) => None,
            Err(e) => panic!("WGPU_PWSH_TEST_ADAPTER cannot be read: {e}"),
        };
        Some(open(&OpenOptions { adapter: AdapterQuery { name, ..Default::default() }, ..Default::default() }, None).expect("a device opens"))
    }

    /// A wait for a submission the submission thread has not made ends at
    /// its interrupt; a submission thread that has gone is reported as
    /// that; a panic in the submit is raised with its message, and a failed
    /// wait for another WARP device's submission with its reason; and a
    /// submission made through the thread is one a wait can name.
    #[test]
    fn a_wait_for_a_submission_ends_at_an_interrupt() {
        let Some(core) = core() else { return };
        let (held, reply) = mpsc::sync_channel(1);
        let stop = Flag(AtomicBool::new(true));
        let started = Instant::now();
        let e = match (Pending { id: "WgpuSubmit", what: "the submission is invalid", reply }).wait(&core, Some(&stop)) {
            Ok(_) => panic!("a wait for a submission never made returned an index"),
            Err(e) => e,
        };
        assert_eq!(e.error_id, tasks::STOPPED);
        assert!(started.elapsed() < Duration::from_secs(10), "the wait ended {:?} after an interrupt set before it began", started.elapsed());
        drop(held);
        let (gone, reply) = mpsc::sync_channel::<Reply>(1);
        drop(gone);
        let e = match (Pending { id: "WgpuSubmit", what: "the submission is invalid", reply }).wait(&core, None) {
            Ok(_) => panic!("a wait whose submission thread has gone returned an index"),
            Err(e) => e,
        };
        assert_eq!(e.error_id, "WgpuSubmitThread");
        let (panicked, reply) = mpsc::sync_channel::<Reply>(1);
        panicked.send(Err(Failed::Panicked("an assertion in the submit".to_string()))).expect("the reply is sent");
        let e = match (Pending { id: "WgpuSubmit", what: "the submission is invalid", reply }).wait(&core, None) {
            Ok(_) => panic!("a wait whose submission panicked returned an index"),
            Err(e) => e,
        };
        assert_eq!(e.error_id, "WgpuPanic");
        assert_eq!(e.message, "the submission panicked: an assertion in the submit");
        let (unwaited, reply) = mpsc::sync_channel::<Reply>(1);
        unwaited.send(Err(Failed::WarpWait("the device was lost".to_string()))).expect("the reply is sent");
        let e = match (Pending { id: "WgpuSubmit", what: "the submission is invalid", reply }).wait(&core, None) {
            Ok(_) => panic!("a wait whose submission was never made returned an index"),
            Err(e) => e,
        };
        assert_eq!(e.error_id, "WgpuPoll");
        assert_eq!(e.message, "waiting for the newest submission on another WARP device failed: the device was lost");
        let commands = core.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None }).finish();
        let index = core.submit(vec![commands], None).expect("submitted");
        core.wait_for(&index, None).expect("waited");
    }

    /// A submission on a WARP device waits for the newest submission when
    /// another device made it, not when its own device did, and a wait that
    /// fails is reported and forgets that submission. Any adapter shows
    /// it: the newest here names a device that never made it, which fails
    /// the wait before the GPU is asked.
    #[test]
    fn a_warp_submission_waits_only_for_another_devices_newest() {
        let Some(made) = core() else { return };
        let Some(never) = core() else { return };
        let index = made.submit(Vec::new(), None).expect("submitted");
        let warp = Warp { turn: AtomicBool::new(false), newest: ArcSwapOption::const_empty() };
        let turn = warp.take();
        turn.made(1, &never.device, &index);
        assert!(turn.after_other_devices(1).is_ok(), "a submission waited for its own device's newest");
        match turn.after_other_devices(2) {
            Ok(()) => panic!("a submission on another device went ahead of the newest"),
            Err(Failed::WarpWait(reason)) => assert!(reason.contains("has not been returned by a successful submission"), "{reason}"),
            Err(Failed::Panicked(message)) => panic!("the wait panicked: {message}"),
        }
        assert!(warp.newest.load().is_none(), "a submission no wait can reach was kept");
        assert!(turn.after_other_devices(2).is_ok(), "a submission waited for one already forgotten");
    }

    /// While one submission holds the WARP turn no other takes it, and the
    /// next takes it once the first gives it back.
    #[test]
    fn one_warp_submission_holds_the_turn_at_a_time() {
        let warp = Warp { turn: AtomicBool::new(false), newest: ArcSwapOption::const_empty() };
        let taken = AtomicBool::new(false);
        let first = warp.take();
        std::thread::scope(|s| {
            let second = s.spawn(|| {
                let turn = warp.take();
                taken.store(true, Ordering::Release);
                drop(turn);
            });
            std::thread::sleep(Duration::from_millis(50));
            assert!(!taken.load(Ordering::Acquire), "a second submission took the turn while the first held it");
            drop(first);
            second.join().expect("the second submission takes the turn");
        });
        assert!(taken.load(Ordering::Acquire));
    }

    /// Every error the scopes caught is reported, the internal or
    /// out-of-memory one before the validation one, and no error as none.
    #[test]
    fn reports_every_caught_error_with_the_likely_cause_first() {
        let internal = || wgpu::Error::Internal { source: Box::new(std::io::Error::other("FXC")), description: "the pipeline failed to build".to_string() };
        let memory = || wgpu::Error::OutOfMemory { source: Box::new(std::io::Error::other("no memory")) };
        let validation = || wgpu::Error::Validation { source: Box::new(std::io::Error::other("invalid")), description: "a draw through an invalid pipeline".to_string() };
        assert_eq!(reported([None, None, None]), None);
        assert_eq!(reported([None, None, Some(validation())]).as_deref(), Some("a draw through an invalid pipeline"));
        assert_eq!(reported([Some(internal()), None, Some(validation())]).as_deref(), Some("the pipeline failed to build; a draw through an invalid pipeline"));
        assert_eq!(reported([None, Some(memory()), Some(validation())]).as_deref(), Some("Out of Memory; a draw through an invalid pipeline"));
    }
}
