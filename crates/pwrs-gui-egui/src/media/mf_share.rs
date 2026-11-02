//! Media Foundation's frames kept on the GPU. Where the renderer draws on a
//! hardware adapter with wgpu's Direct3D 12 device, or with its Vulkan
//! device where that imports Direct3D 11 textures, a media engine with a
//! picture decodes on a Direct3D 11 device of the same adapter and writes
//! each frame into one of two textures shared with wgpu's device, and the
//! paint callback copies it on the GPU into the player's own texture. With
//! OpenGL or WARP, or where a step here fails, the frames come through the
//! CPU.

use arc_swap::ArcSwapOption;
use pwrs_gui_gpu::wgpu;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{CloseHandle, HANDLE, LUID, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_QUERY_DESC, D3D11_QUERY_EVENT, D3D11_RESOURCE_MISC_SHARED,
    D3D11_RESOURCE_MISC_SHARED_NTHANDLE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Query, ID3D11Texture2D,
};
use windows::Win32::Graphics::Direct3D12::ID3D12Resource;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND, DXGI_SHARED_RESOURCE_READ, DXGI_SHARED_RESOURCE_WRITE, IDXGIAdapter, IDXGIFactory1, IDXGIFactory4, IDXGIResource1};
use windows::Win32::Media::MediaFoundation::{IMFDXGIDeviceManager, IMFMediaEngine, MFARGB};
use windows::core::{BOOL, GUID, HRESULT, Interface, PCWSTR};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};

/// The adapter wgpu's device draws with, where the path can share frames
/// with it: its locally unique identifier, the high part above the low; 0
/// while the renderer draws otherwise.
static ADAPTER: AtomicU64 = AtomicU64::new(0);

/// Which of wgpu's backends the device that frames are shared with is.
static BACKEND: AtomicU8 = AtomicU8::new(NO_BACKEND);
const NO_BACKEND: u8 = 0;
const DIRECT3D_12: u8 = 1;
const VULKAN: u8 = 2;

/// How many devices the renderer has drawn with, so a texture imported
/// into an earlier one is imported again.
static DEVICES: AtomicU64 = AtomicU64::new(0);

/// How long a frame's write may take to run on the GPU before the frame is
/// skipped.
const WRITE_WAIT: Duration = Duration::from_millis(250);

/// How many frames in a row may stall before a player gives the shared
/// path up and takes its frames through the CPU.
const STALLS: u8 = 3;

/// How many frames in a row have stalled on the GPU.
#[derive(Debug, Default)]
struct Stalls(u8);

impl Stalls {
    /// Counts a frame whose write stalled; true once [`STALLS`] have
    /// stalled in a row.
    fn stalled(&mut self) -> bool {
        self.0 = self.0.saturating_add(1);
        self.0 >= STALLS
    }

    /// Starts the count again, as a frame was written in time.
    fn written(&mut self) {
        self.0 = 0;
    }
}

/// Records the device the renderer draws with from now on: the engines
/// opened after it share their frames with it where it is wgpu's Direct3D
/// 12 device on hardware, or its Vulkan device there with the Direct3D 11
/// import, its adapter found among DXGI's by its vendor and device.
pub(crate) fn link(adapter: &wgpu::Adapter, device: &wgpu::Device) {
    DEVICES.fetch_add(1, Ordering::AcqRel);
    let info = adapter.get_info();
    let (backend, luid) = match info.backend {
        _ if info.device_type == wgpu::DeviceType::Cpu => (NO_BACKEND, 0),
        // SAFETY: wgpu's own device, whose Direct3D 12 device is only read.
        wgpu::Backend::Dx12 => match unsafe { device.as_hal::<wgpu::hal::api::Dx12>() } {
            // SAFETY: as above.
            Some(hal) => (DIRECT3D_12, packed(unsafe { hal.raw_device().GetAdapterLuid() })),
            None => (NO_BACKEND, 0),
        },
        wgpu::Backend::Vulkan if device.features().contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_WIN32) => match adapter_of(info.vendor, info.device) {
            Ok(luid) => (VULKAN, luid),
            Err(why) => {
                eprintln!("pwrs-gui-egui: players' frames come through the CPU: {why}");
                (NO_BACKEND, 0)
            }
        },
        _ => (NO_BACKEND, 0),
    };
    BACKEND.store(backend, Ordering::Release);
    ADAPTER.store(luid, Ordering::Release);
    pwrs_gui_proto::trace::line(|| match backend {
        DIRECT3D_12 => format!("media: players' frames are shared with wgpu's Direct3D 12 device on {}", info.name),
        VULKAN => format!("media: players' frames are shared with wgpu's Vulkan device on {}", info.name),
        _ => format!("media: players' frames come through the CPU, as the renderer draws with {:?} on {} ({:?})", info.backend, info.name, info.device_type),
    });
}

/// The identifier, packed, of the hardware adapter DXGI lists first with
/// PCI vendor `vendor` and device `device`.
fn adapter_of(vendor: u32, device: u32) -> Result<u64, String> {
    let exports = exports()?;
    // SAFETY: the out pointer lives through the call, and the factory is
    // owned from it.
    let factory = unsafe {
        let mut raw = std::ptr::null_mut();
        checked((exports.create_factory)(&IDXGIFactory1::IID, &mut raw), "the DXGI factory could not be made")?;
        IDXGIFactory1::from_raw(raw)
    };
    for index in 0.. {
        // SAFETY: the factory lists its adapters by index until none is left.
        let listed = match unsafe { factory.EnumAdapters1(index) } {
            Ok(a) => a,
            Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(e) => return Err(format!("DXGI could not list its adapters: {}", e.message())),
        };
        // SAFETY: as above.
        let desc = unsafe { listed.GetDesc1() }.map_err(|e| format!("an adapter could not be described: {}", e.message()))?;
        if desc.VendorId == vendor && desc.DeviceId == device && (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) == 0 {
            return Ok(packed(desc.AdapterLuid));
        }
    }
    Err(format!("DXGI lists no adapter of vendor {vendor:#06x} and device {device:#06x}, which wgpu's Vulkan device draws with"))
}

fn packed(luid: LUID) -> u64 {
    (u64::from(luid.HighPart as u32) << 32) | u64::from(luid.LowPart)
}

fn unpacked(v: u64) -> LUID {
    LUID { LowPart: v as u32, HighPart: (v >> 32) as i32 }
}

/// Success, or `what` failed with the system's words for `hr`.
fn checked(hr: HRESULT, what: &str) -> Result<(), String> {
    if hr.is_ok() { Ok(()) } else { Err(format!("{what}: {}", windows::core::Error::from(hr).message())) }
}

/// `D3D11CreateDevice`, as `d3d11.h` declares it.
type CreateDevice = unsafe extern "system" fn(*mut c_void, i32, *mut c_void, u32, *const i32, u32, u32, *mut *mut c_void, *mut i32, *mut *mut c_void) -> HRESULT;

/// `CreateDXGIFactory1`, as `dxgi.h` declares it.
type CreateFactory = unsafe extern "system" fn(*const GUID, *mut *mut c_void) -> HRESULT;

/// `MFCreateDXGIDeviceManager`, as `mfapi.h` declares it, which
/// `mfplat.dll` exports from Windows 8.
pub(super) type CreateManager = unsafe extern "system" fn(*mut u32, *mut *mut c_void) -> HRESULT;

/// The exports of the system's Direct3D 11 and DXGI libraries the path
/// calls, loaded at run time so the renderer starts without them.
#[derive(Clone, Copy)]
struct Exports {
    create_device: CreateDevice,
    create_factory: CreateFactory,
}

static EXPORTS: OnceLock<Result<Exports, String>> = OnceLock::new();

fn exports() -> Result<Exports, String> {
    EXPORTS.get_or_init(load).clone()
}

fn load() -> Result<Exports, String> {
    let open = |name: &str| {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a nul-terminated wide name; the library stays loaded for
        // the life of the process.
        let module = unsafe { LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
        if module.is_null() { Err(format!("{name} could not be loaded: {}", std::io::Error::last_os_error())) } else { Ok(module) }
    };
    let (d3d11, dxgi) = (open("d3d11.dll")?, open("dxgi.dll")?);
    // SAFETY: nul-terminated names of exports both libraries have had since
    // Windows 7.
    let found = unsafe { (GetProcAddress(d3d11, c"D3D11CreateDevice".as_ptr().cast()), GetProcAddress(dxgi, c"CreateDXGIFactory1".as_ptr().cast())) };
    let (Some(device), Some(factory)) = found else {
        return Err(String::from("d3d11.dll or dxgi.dll lacks D3D11CreateDevice or CreateDXGIFactory1"));
    };
    // SAFETY: the exports' own signatures, as the headers declare them.
    let create_device = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, CreateDevice>(device) };
    // SAFETY: as above.
    let create_factory = unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, CreateFactory>(factory) };
    Ok(Exports { create_device, create_factory })
}

/// An NT handle of a shared texture, closed with it.
struct Handle(HANDLE);

// SAFETY: a kernel handle, which any thread of the process may use.
unsafe impl Send for Handle {}
// SAFETY: as above; it is only read.
unsafe impl Sync for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is this one's, closed once.
        if let Err(e) = unsafe { CloseHandle(self.0) } {
            eprintln!("pwrs-gui-egui: a shared texture's handle did not close: {}", e.message());
        }
    }
}

/// A shared texture as the paint callback reaches it: its handle and size,
/// the number of the copy out of it recorded last, wgpu's texture of it on
/// the device the renderer draws with, and why wgpu could not take it.
pub(crate) struct Share {
    handle: Handle,
    size: [u32; 2],
    copied: AtomicU64,
    imported: ArcSwapOption<(u64, wgpu::Texture)>,
    refused: ArcSwapOption<String>,
}

impl Share {
    /// Records a copy of the frame the texture holds into `target`, the
    /// player's own texture, among `encoder`'s commands for the draw of
    /// egui frame `draw`; or, where wgpu's device cannot take the texture,
    /// says why to the engine, which then hands its frames over through the
    /// CPU. Copies are recorded on the renderer's thread alone.
    pub(crate) fn copy(&self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Texture, draw: u64) {
        let source = match self.imported(device) {
            Ok(t) => t,
            Err(why) => {
                self.refused.store(Some(Arc::new(why)));
                return;
            }
        };
        let size = wgpu::Extent3d { width: self.size[0], height: self.size[1], depth_or_array_layers: 1 };
        encoder.copy_texture_to_texture(source.1.as_image_copy(), target.as_image_copy(), size);
        self.copied.store(super::copies::recorded(queue, draw), Ordering::Release);
    }

    /// wgpu's texture of the shared one on `device`, imported once a device.
    fn imported(&self, device: &wgpu::Device) -> Result<Arc<(u64, wgpu::Texture)>, String> {
        let current = DEVICES.load(Ordering::Acquire);
        if let Some(held) = self.imported.load_full()
            && held.0 == current
        {
            return Ok(held);
        }
        let size = wgpu::Extent3d { width: self.size[0], height: self.size[1], depth_or_array_layers: 1 };
        let described = wgpu::TextureDescriptor {
            label: Some("PWRS-GUI MediaPlayer shared"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        let (texture, api) = match BACKEND.load(Ordering::Acquire) {
            DIRECT3D_12 => (self.on_direct3d_12(device, &described)?, "Direct3D 12"),
            VULKAN => (self.on_vulkan(device, &described)?, "Vulkan"),
            _ => return Err(String::from("the renderer no longer draws with a device that takes shared textures")),
        };
        pwrs_gui_proto::trace::line(|| format!("media: wgpu's {api} device took a shared texture of {} by {} pixels, which frames are copied out of on the GPU", self.size[0], self.size[1]));
        let held = Arc::new((current, texture));
        self.imported.store(Some(Arc::clone(&held)));
        Ok(held)
    }

    /// The texture opened on wgpu's Direct3D 12 device through its handle.
    fn on_direct3d_12(&self, device: &wgpu::Device, described: &wgpu::TextureDescriptor<'_>) -> Result<wgpu::Texture, String> {
        let mut resource: Option<ID3D12Resource> = None;
        {
            // SAFETY: wgpu's own device, whose Direct3D 12 device opens the
            // texture's handle and is not changed; the handle lives while
            // this share does.
            let hal = unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }.ok_or("the renderer no longer draws with Direct3D 12")?;
            // SAFETY: as above; the out pointer lives through the call.
            unsafe { hal.raw_device().OpenSharedHandle(self.handle.0, &mut resource) }.map_err(|e| format!("its Direct3D 12 device could not open the shared texture: {}", e.message()))?;
        }
        let resource = resource.ok_or("the shared texture opened as nothing")?;
        // SAFETY: the resource is a texture of the description's size and
        // format, opened on wgpu's own device in the common state, which
        // wgpu takes as unset.
        Ok(unsafe {
            let raw = wgpu::hal::dx12::Device::texture_from_raw(resource, described.format, described.dimension, described.size, 1, 1);
            device.create_texture_from_hal::<wgpu::hal::api::Dx12>(raw, described, wgpu::wgt::TextureUses::UNINITIALIZED)
        })
    }

    /// The texture imported into wgpu's Vulkan device through its handle.
    fn on_vulkan(&self, device: &wgpu::Device, described: &wgpu::TextureDescriptor<'_>) -> Result<wgpu::Texture, String> {
        let raw_described = wgpu::hal::TextureDescriptor {
            label: described.label,
            size: described.size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: described.dimension,
            format: described.format,
            usage: wgpu::wgt::TextureUses::COPY_SRC,
            memory_flags: wgpu::hal::MemoryFlags::empty(),
            view_formats: Vec::new(),
        };
        let raw = {
            // SAFETY: wgpu's own device, which imports the texture's handle;
            // the handle lives while this share does.
            let hal = unsafe { device.as_hal::<wgpu::hal::api::Vulkan>() }.ok_or("the renderer no longer draws with Vulkan")?;
            // SAFETY: as above; the handle names a texture of the
            // description's size and format.
            unsafe { hal.texture_from_d3d11_shared_handle(self.handle.0, &raw_described) }.map_err(|e| format!("its Vulkan device could not import the shared texture: {e}"))?
        };
        // SAFETY: the image was made on wgpu's own device for the
        // description, its contents undefined to Vulkan until it is used.
        Ok(unsafe { device.create_texture_from_hal::<wgpu::hal::api::Vulkan>(raw, described, wgpu::wgt::TextureUses::UNINITIALIZED) })
    }
}

/// A texture an engine writes frames into, and its share with wgpu.
struct Slot {
    texture: ID3D11Texture2D,
    share: Arc<Share>,
}

/// A media engine's side of the shared path: the Direct3D 11 device it
/// decodes and converts on, through Media Foundation's device manager, and
/// the two textures its frames are written into in turn.
pub(super) struct Shared {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    manager: IMFDXGIDeviceManager,
    done: ID3D11Query,
    slots: Vec<Slot>,
    /// The slot written last, which the next frame leaves alone.
    last: usize,
    /// The frames that have stalled in a row.
    stalls: Stalls,
}

impl Shared {
    /// Opens a Direct3D 11 device with video on the adapter wgpu draws with,
    /// for an engine to decode on; none while the renderer draws otherwise
    /// or Media Foundation has no device manager.
    pub(super) fn open(create_manager: Option<CreateManager>) -> Result<Option<Shared>, String> {
        let luid = ADAPTER.load(Ordering::Acquire);
        let Some(create_manager) = create_manager else {
            return Ok(None);
        };
        if luid == 0 {
            return Ok(None);
        }
        let exports = exports()?;
        // SAFETY: every pointer passed lives through its call, and each
        // object given back is owned from the pointer it was written to.
        unsafe {
            let mut raw = std::ptr::null_mut();
            checked((exports.create_factory)(&IDXGIFactory4::IID, &mut raw), "the DXGI factory could not be made")?;
            let factory = IDXGIFactory4::from_raw(raw);
            let adapter: IDXGIAdapter = factory.EnumAdapterByLuid(unpacked(luid)).map_err(|e| format!("the adapter wgpu draws with was not found: {}", e.message()))?;
            let levels = [D3D_FEATURE_LEVEL_11_0.0];
            let (mut device, mut context) = (std::ptr::null_mut(), std::ptr::null_mut());
            let flags = D3D11_CREATE_DEVICE_VIDEO_SUPPORT.0 | D3D11_CREATE_DEVICE_BGRA_SUPPORT.0;
            let made = (exports.create_device)(adapter.as_raw(), D3D_DRIVER_TYPE_UNKNOWN.0, std::ptr::null_mut(), flags, levels.as_ptr(), levels.len() as u32, D3D11_SDK_VERSION, &mut device, std::ptr::null_mut(), &mut context);
            checked(made, "no Direct3D 11 device with video could be made on the adapter wgpu draws with")?;
            let (device, context) = (ID3D11Device::from_raw(device), ID3D11DeviceContext::from_raw(context));
            let threads: ID3D11Multithread = context.cast().map_err(|e| format!("the Direct3D 11 device cannot be shared between threads: {}", e.message()))?;
            // Media Foundation calls the device from its own threads; whether
            // it was protected before does not matter.
            let _protected_before = threads.SetMultithreadProtected(true);
            let mut token = 0u32;
            let mut raw = std::ptr::null_mut();
            checked(create_manager(&mut token, &mut raw), "Media Foundation's device manager could not be made")?;
            let manager = IMFDXGIDeviceManager::from_raw(raw);
            manager.ResetDevice(&device, token).map_err(|e| format!("Media Foundation's device manager took no Direct3D 11 device: {}", e.message()))?;
            let mut done = None;
            device.CreateQuery(&D3D11_QUERY_DESC { Query: D3D11_QUERY_EVENT, MiscFlags: 0 }, Some(&mut done)).map_err(|e| format!("no query of the GPU's progress could be made: {}", e.message()))?;
            let done = done.ok_or("no query of the GPU's progress was made")?;
            Ok(Some(Shared { device, context, manager, done, slots: Vec::new(), last: 0, stalls: Stalls::default() }))
        }
    }

    /// The manager the engine is given, which hands it the device.
    pub(super) fn manager(&self) -> &IMFDXGIDeviceManager {
        &self.manager
    }

    /// Writes the frame due, of `size` pixels, into the texture wgpu is not
    /// copying out of, and hands that texture over once the write has run;
    /// none while wgpu may still be copying out of both, or where the write
    /// stalled past [`WRITE_WAIT`], which skips the frame. [`STALLS`] stalls
    /// in a row, or a texture wgpu could not take, end the path, which the
    /// engine then leaves for the CPU's.
    pub(super) fn write(&mut self, engine: &IMFMediaEngine, size: [u32; 2]) -> Result<Option<Arc<Share>>, String> {
        if self.slots.first().is_some_and(|s| s.share.size != size) {
            self.slots.clear();
        }
        while self.slots.len() < 2 {
            let slot = self.slot(size)?;
            self.slots.push(slot);
        }
        for slot in &self.slots {
            if let Some(why) = slot.share.refused.load_full() {
                return Err(format!("wgpu could not take a shared texture: {why}"));
            }
        }
        let next = self.last ^ 1;
        let slot = &self.slots[next];
        if !super::copies::finished(slot.share.copied.load(Ordering::Acquire)) {
            return Ok(None);
        }
        let whole = RECT { left: 0, top: 0, right: size[0] as i32, bottom: size[1] as i32 };
        let black = MFARGB { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbAlpha: 255 };
        // SAFETY: the texture, the rectangles and the query's result live
        // through the calls, all on the engine's own device.
        unsafe {
            engine.TransferVideoFrame(&slot.texture, None, &whole, Some(&black)).map_err(|e| format!("the frame could not be written into its shared texture: {}", e.message()))?;
            self.context.End(&self.done);
            self.context.Flush();
            let began = Instant::now();
            loop {
                let mut ran = BOOL(0);
                self.context.GetData(&self.done, Some(std::ptr::addr_of_mut!(ran).cast()), std::mem::size_of::<BOOL>() as u32, 0).map_err(|e| format!("the GPU's progress could not be read: {}", e.message()))?;
                if ran.as_bool() {
                    break;
                }
                if began.elapsed() > WRITE_WAIT {
                    // The frame is skipped; the next one tries again, unless
                    // too many have stalled in a row.
                    if self.stalls.stalled() {
                        return Err(format!("{STALLS} frames in a row had not been written on the GPU within {} ms each", WRITE_WAIT.as_millis()));
                    }
                    pwrs_gui_proto::trace::line(|| format!("media: a frame is skipped, its write not run on the GPU within {} ms, stall {} in a row", WRITE_WAIT.as_millis(), self.stalls.0));
                    return Ok(None);
                }
                std::thread::yield_now();
            }
        }
        self.stalls.written();
        self.last = next;
        Ok(Some(Arc::clone(&slot.share)))
    }

    /// A texture of `size` pixels the engine can write frames into, shared
    /// with wgpu through an NT handle.
    fn slot(&self, size: [u32; 2]) -> Result<Slot, String> {
        let described = D3D11_TEXTURE2D_DESC {
            Width: size[0],
            Height: size[1],
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: (D3D11_RESOURCE_MISC_SHARED.0 | D3D11_RESOURCE_MISC_SHARED_NTHANDLE.0) as u32,
        };
        // SAFETY: the description and the out pointer live through the
        // calls, on the engine's own device.
        unsafe {
            let mut texture = None;
            self.device.CreateTexture2D(&described, None, Some(&mut texture)).map_err(|e| format!("no shared texture of {} by {} pixels could be made: {}", size[0], size[1], e.message()))?;
            let texture = texture.ok_or("no shared texture was made")?;
            let resource: IDXGIResource1 = texture.cast().map_err(|e| format!("the shared texture cannot be shared: {}", e.message()))?;
            let handle = resource.CreateSharedHandle(None, DXGI_SHARED_RESOURCE_READ.0 | DXGI_SHARED_RESOURCE_WRITE.0, PCWSTR::null()).map_err(|e| format!("the shared texture gave no handle: {}", e.message()))?;
            let share = Share { handle: Handle(handle), size, copied: AtomicU64::new(0), imported: ArcSwapOption::empty(), refused: ArcSwapOption::empty() };
            Ok(Slot { texture, share: Arc::new(share) })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_player_gives_the_shared_path_up_at_the_third_stall_in_a_row_and_a_frame_written_in_time_starts_the_count_again() {
        let mut stalls = Stalls::default();
        assert!(!stalls.stalled(), "a first stall skips the frame alone");
        assert!(!stalls.stalled(), "a second stall in a row skips the frame alone");
        stalls.written();
        assert!(!stalls.stalled(), "a frame written in time starts the count again");
        assert!(!stalls.stalled());
        assert!(stalls.stalled(), "the third stall in a row gives the path up");
        assert!(stalls.stalled(), "and every stall after it");
    }
}
