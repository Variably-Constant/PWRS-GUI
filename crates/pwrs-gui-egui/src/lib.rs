//! PWRS-GUI's egui renderer. Each window a session opens is a viewport of
//! its own; a hidden root viewport stays open, so the event loop runs while
//! no window is open. It takes the link every renderer takes,
//! [`proto::renderer::Link`], and reads its messages in the frame of
//! whichever viewport runs first, the root's or a window's.
//!
//! The renderer's state belongs to the thread running it: it is stored in
//! a thread-local, so the viewport callbacks eframe calls on that thread
//! reach it without a lock.
//!
//! A property a node lacks takes the default its widget documents. A
//! required property that is missing, or any property of the wrong type,
//! is drawn in the window as an error and reported once.

use arc_swap::{ArcSwap, ArcSwapOption};
use crossbeam_queue::SegQueue;
pub use eframe::egui;
use eframe::egui_wgpu;
use pwrs_gui_proto as proto;
use proto::button_bar::{self, Role};
use proto::gpu::{GpuFailure, GpuOp, GpuValue};
use proto::place;
use proto::renderer::{Inbound, Link, Sink, WakeLoop};
use proto::rows::{self, Selection};
use proto::tokens;
use proto::{
    Align, Capabilities, CloseReason, EventKind, FromHelper, Kind, LogLevel, Node, NodeId, PatchOp, RequestId, RingName, Shortcut, SortKey,
    ToHelper, Value, WindowId, WindowSummary,
};
use egui::load::{BytesPoll, ImageLoadResult, ImageLoader, ImagePoll, LoadError, SizeHint, TexturePoll};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};

mod camera;
mod canvas;
mod chart;
mod choosers;
mod fallback;
mod gpu_thread;
mod input;
mod look;
mod markdown;
mod pointer;
mod routed;
mod standin;
mod surface;
mod view;
mod web;

/// How the renderer chooses its GPU and where its event loop may run.
#[derive(Clone, Debug, Default)]
pub struct Settings {
    /// Part of an adapter's name to prefer, compared without regard to case.
    pub adapter: Option<String>,
    /// Prefer the adapter that draws the least power.
    pub low_power: bool,
    /// Let the event loop run on a thread other than the process's first.
    /// Windows only; the in-process path sets it.
    pub any_thread: bool,
    /// Bring each new window to the front at its first paint. Windows
    /// only. Windows grants the front by its rule for a newly started
    /// process only to a helper's first window, so a helper requests it for
    /// each window too, the client having passed it the right to.
    pub bring_to_front: bool,
    /// The Mesa add-on OpenGL draws through. Windows only.
    pub mesa: Option<Mesa>,
    /// The fonts text is drawn in.
    pub fonts: proto::Fonts,
}

/// A folder of the Mesa add-on, `PWRS-GUI.Mesa.win-x64`, containing Mesa's
/// llvmpipe `opengl32.dll`, which draws OpenGL on the processor.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesa {
    pub folder: std::path::PathBuf,
    /// Whether `PWRS_GUI_MESA_ADDON` chose it, so OpenGL draws through it
    /// on any machine; one found on `PSModulePath` is drawn through only
    /// where System32 has no `opengl32.dll`.
    pub chosen: bool,
}

thread_local! {
    static UI: RefCell<Option<UiState>> = const { RefCell::new(None) };
}

/// Runs `f` on this thread's renderer state, if a renderer runs here.
fn with_ui<R>(f: impl FnOnce(&mut UiState) -> R) -> Option<R> {
    UI.with(|cell| cell.borrow_mut().as_mut().map(f))
}

/// Runs the renderer until the session sends `Shutdown` or drops its end
/// of the inbox. Returns the event loop's error, if it could not start;
/// the start ring then has `StartFailed` in place of `Hello`.
pub fn run(link: Link, settings: Settings) -> Result<(), String> {
    let Link { inbox, sink, on_ready } = link;
    proto::trace::line(|| String::from("egui: run starts"));
    let viewports = Arc::new(ArcSwap::from_pointee(Vec::new()));
    let current = Arc::new(ArcSwapOption::empty());
    let wake = wake_loop(Arc::clone(&current), Arc::clone(&viewports));
    UI.with(|cell| {
        *cell.borrow_mut() = Some(UiState {
            inbox,
            viewports,
            sink,
            dialogs_done: Arc::new(SegQueue::new()),
            shots_arrived: Arc::new(SegQueue::new()),
            stand_ins: standin::StandIns::default(),
            wake: None,
            windows: BTreeMap::new(),
            adapters: Vec::new(),
            closing: false,
            hello_sent: false,
            monitor: None,
            no_focus: false,
            #[cfg(windows)]
            foreign: Vec::new(),
            #[cfg(windows)]
            bring_to_front: settings.bring_to_front,
            forgotten: Vec::new(),
            web: Arc::new(web::WebLoader::new()),
            fallbacks: None,
            sources: None,
            gpu: None,
            restart: None,
            installed: None,
            loss_watchers: Vec::new(),
            held_gpu: Vec::new(),
            target_format: None,
            gpu_reported: Reported::default(),
            measure: None,
        });
    });
    // The system's fonts are found while eframe makes its window and device,
    // and taken once the hello is sent, before any window draws.
    let fonts = settings.fonts;
    let finding = Finding { thread: std::thread::Builder::new().name(String::from("pwrs-gui-fonts")).spawn(move || system_fonts(fonts)), fonts };
    let mut next = Start::First { on_ready, finding };
    let started = loop {
        let ran = start(next, &current, &wake, &settings);
        let again = with_ui(|s| if s.closing { None } else { s.restart.take() }).flatten();
        match ran {
            Err(e) => {
                break match again {
                    Some(reason) => Err(format!("the renderer could not run again after its GPU device was lost ({reason}): {e}")),
                    None => Err(e),
                };
            }
            Ok(()) => match again {
                Some(reason) => next = Start::Again { reason },
                None => break Ok(()),
            },
        }
    };
    let left = UI.with(|cell| cell.borrow_mut().take());
    if let Err(e) = &started
        && let Some(mut state) = left
        && !state.hello_sent
    {
        state.sink.hello(FromHelper::StartFailed { text: e.clone() });
    }
    started
}

/// What an eframe run starts with: the first sends the hello and installs
/// the fonts once they are found; a later one, in place of a run whose GPU
/// device was lost, installs what the first found again, moves the GPU
/// layer to the new device and opens the windows again.
enum Start {
    First { on_ready: Box<dyn FnOnce(WakeLoop) + Send>, finding: Finding },
    Again { reason: String },
}

/// The thread finding the fonts text is drawn in, or why it could not
/// start, and which fonts it finds.
struct Finding {
    thread: std::io::Result<std::thread::JoinHandle<FoundFonts>>,
    fonts: proto::Fonts,
}

/// What eframe's creation callback returns.
type Created = Result<Box<dyn eframe::App>, Box<dyn std::error::Error + Send + Sync>>;

/// What wakes the loop from another thread, in whichever run is under way.
/// Messages are read in the root viewport's frame and in each window's, so
/// a message wakes them all: eframe paints the hidden root at most every
/// 100 ms, and a window that is shown at once. A plain `request_repaint`
/// from another thread names whichever viewport is mid-frame, which is
/// often a window. A ring a widget reads wakes them the same way. Between
/// runs nothing is drawn, and the next run's first frame reads what came.
fn wake_loop(current: Arc<ArcSwapOption<egui::Context>>, windows: Arc<ArcSwap<Vec<egui::ViewportId>>>) -> WakeLoop {
    Arc::new(move || match current.load_full() {
        Some(ctx) => {
            proto::trace::line(|| String::from("egui: woken for a message"));
            ctx.request_repaint_of(egui::ViewportId::ROOT);
            for id in windows.load().iter() {
                ctx.request_repaint_of(*id);
            }
        }
        None => proto::trace::line(|| String::from("egui: woken between runs; the next run's first frame reads the messages")),
    })
}

fn start(next: Start, current: &Arc<ArcSwapOption<egui::Context>>, wake: &WakeLoop, settings: &Settings) -> Result<(), String> {
    // The system's fonts come from fontdb ([`system_fonts`] and [`fallback`]),
    // so eframe adds no font provider of its own.
    let mut options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("PWRS-GUI").with_visible(false).with_taskbar(false).with_inner_size([1.0, 1.0]),
        renderer: eframe::Renderer::Wgpu,
        run_and_return: true,
        system_font_fallback: false,
        ..Default::default()
    };
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = backends(settings.mesa.as_ref())?;
    setup.power_preference = if settings.low_power { wgpu::PowerPreference::LowPower } else { wgpu::PowerPreference::HighPerformance };
    let wanted = settings.adapter.clone();
    let low_power = settings.low_power;
    setup.native_adapter_selector = Some(Arc::new(move |adapters, _surface| choose_adapter(adapters, wanted.as_deref(), low_power)));
    setup.device_descriptor = Arc::new(device_descriptor);
    options.wgpu_options.wgpu_setup = egui_wgpu::WgpuSetup::CreateNew(setup);
    let (no_focus, focus_note) = match place::no_focus() {
        Ok(no_focus) => (no_focus, None),
        Err(why) => (false, Some(why)),
    };
    options.event_loop_builder = loop_settings(settings.any_thread, no_focus)?;
    #[cfg(all(unix, not(target_vendor = "apple")))]
    if no_focus {
        eframe::before_inactive_window_shown(unfocused_on_map);
    }
    proto::trace::line(|| String::from("egui: event loop starting"));
    let held = Arc::clone(current);
    let wake = Arc::clone(wake);
    let ran = eframe::run_native(
        "PWRS-GUI",
        options,
        Box::new(move |cc| {
            proto::trace::line(|| String::from("egui: window and device ready"));
            look::apply(&cc.egui_ctx);
            #[cfg(windows)]
            with_ui(|s| s.foreign = thread_windows());
            held.store(Some(Arc::new(cc.egui_ctx.clone())));
            let Some(rs) = &cc.wgpu_render_state else {
                return Err("eframe started without a wgpu render state".into());
            };
            match next {
                Start::First { on_ready, finding } => first_run(cc, rs, on_ready, finding, wake, no_focus, focus_note),
                Start::Again { reason } => again(cc, rs, &reason),
            }
        }),
    );
    current.store(None);
    proto::trace::line(|| String::from("egui: run ends"));
    ran.map_err(|e| e.to_string())
}

/// The device the windows and the GPU layer share: every feature and limit
/// `adapter` supports but wgpu's experimental features, as the Wgpu module
/// opens its own devices, so a shader the Wgpu module runs, `f16`, `f64` and
/// `i64` ones and timed runs included, runs on this device too.
fn device_descriptor(adapter: &wgpu::Adapter) -> wgpu::DeviceDescriptor<'static> {
    wgpu::DeviceDescriptor {
        label: Some("PWRS-GUI egui device"),
        required_features: adapter.features() - wgpu::Features::all_experimental_mask(),
        required_limits: adapter.limits(),
        ..Default::default()
    }
}

/// The first run's start on the window and device eframe made: the session
/// is told the loop can be woken, the GPU layer starts, the hello is sent,
/// and the fonts are installed once found.
fn first_run(
    cc: &eframe::CreationContext<'_>,
    rs: &egui_wgpu::RenderState,
    on_ready: Box<dyn FnOnce(WakeLoop) + Send>,
    finding: Finding,
    wake: WakeLoop,
    no_focus: bool,
    focus_note: Option<String>,
) -> Created {
    on_ready(Arc::clone(&wake));
    let kept = Arc::clone(&wake);
    if with_ui(|s| {
        s.sources = Some(proto::source::Sources::new(wake));
        s.wake = Some(kept);
    })
    .is_none()
    {
        eprintln!("pwrs-gui-egui: the renderer's state was gone when the rings widgets read were to be set up");
    }
    let info = rs.adapter.get_info();
    let adapters = rs.available_adapters.iter().map(|a| describe(&a.get_info())).collect();
    with_ui(|s| {
        s.adapters = adapters;
        s.target_format = Some(rs.target_format);
    });
    start_gpu(rs);
    let capabilities = Capabilities { backend: format!("{:?}", info.backend), adapter: info.name.clone(), media: None };
    if let Some(why) = keep_driver_hooks() {
        with_ui(|s| s.sink.log(FromHelper::Log { level: LogLevel::Warning, text: why }));
    }
    proto::trace::line(|| String::from("egui: listing monitors"));
    let (wanted, listed) = (place::wanted(), screens());
    proto::trace::line(|| format!("egui: monitors listed: {}", listed.as_ref().map_or_else(|e| e.clone(), |l| l.len().to_string())));
    let wayland_note = if no_focus { focus_on_wayland(cc) } else { None };
    let hello = FromHelper::Hello {
        protocol: proto::PROTOCOL,
        build: env!("CARGO_PKG_VERSION").to_string(),
        renderer: String::from("egui"),
        capabilities,
        notes: place::note(&wanted, &listed).into_iter().chain(focus_note).chain(wayland_note).collect(),
    };
    with_ui(|s| {
        s.sink.hello(hello);
        s.hello_sent = true;
        s.monitor = place::found(&wanted, &listed);
        s.no_focus = no_focus;
    });
    proto::trace::line(|| String::from("egui: hello sent"));
    let waited = std::time::Instant::now();
    let Finding { thread, fonts } = finding;
    let found = match thread {
        Ok(thread) => match thread.join() {
            Ok(found) => found,
            Err(panic) => {
                let why = match (panic.downcast_ref::<&str>(), panic.downcast_ref::<String>()) {
                    (Some(text), _) => text.to_string(),
                    (None, Some(text)) => text.clone(),
                    (None, None) => String::from("a panic with no message"),
                };
                let mut definitions = egui::FontDefinitions::default();
                let drawn = match fonts {
                    proto::Fonts::Bundled => {
                        bundled_first(&mut definitions);
                        "the fonts the build includes, then egui's own"
                    }
                    proto::Fonts::System => "egui's own fonts",
                };
                FoundFonts {
                    definitions,
                    said: Vec::new(),
                    problems: vec![format!("finding the system's fonts failed ({why}), so text is drawn in {drawn}")],
                    database: Arc::new(fontdb::Database::new()),
                    proportional: None,
                }
            }
        },
        Err(e) => {
            proto::trace::line(|| format!("egui: the thread that finds the fonts could not start ({e}), so they are found here"));
            system_fonts(fonts)
        }
    };
    let fallbacks = fallback::Fallbacks::new(Arc::clone(&found.database), &found.definitions);
    let installed = Installed { definitions: found.definitions.clone(), database: Arc::clone(&found.database), proportional: found.proportional.clone() };
    if with_ui(|s| {
        s.fallbacks = Some(fallbacks);
        s.installed = Some(installed);
    })
    .is_none()
    {
        eprintln!("pwrs-gui-egui: the renderer's state was gone when its fallback fonts were to be set up");
    }
    cc.egui_ctx.set_fonts(found.definitions);
    install_picture_loaders(&cc.egui_ctx, found.database, found.proportional);
    match with_ui(|s| Arc::clone(&s.web)) {
        Some(web) => cc.egui_ctx.add_bytes_loader(web),
        None => eprintln!("pwrs-gui-egui: the renderer's state was gone when its https picture loader was to be installed"),
    }
    for line in found.said {
        proto::trace::line(move || format!("egui: fonts: {line}"));
    }
    proto::trace::line(|| format!("egui: fonts: waited {} ms for them after the hello", waited.elapsed().as_millis()));
    with_ui(|s| {
        for text in found.problems {
            s.sink.log(FromHelper::Log { level: LogLevel::Warning, text });
        }
    });
    Ok(Box::new(App))
}

/// A run's start in place of one whose GPU device was lost: the fonts and
/// picture loaders the first run installed go into the new context, every
/// window's text is made drawable again, the GPU layer moves to the new
/// device, and the windows open again where they were.
fn again(cc: &eframe::CreationContext<'_>, rs: &egui_wgpu::RenderState, reason: &str) -> Created {
    proto::trace::line(|| format!("egui: running again on a new device; the last was lost: {reason}"));
    let adapters: Vec<proto::Adapter> = rs.available_adapters.iter().map(|a| describe(&a.get_info())).collect();
    let installed = with_ui(|s| {
        s.adapters = adapters;
        s.installed.as_ref().map(|i| (i.definitions.clone(), Arc::clone(&i.database), i.proportional.clone()))
    });
    match installed {
        Some(Some((definitions, database, proportional))) => {
            let mut fallbacks = fallback::Fallbacks::new(Arc::clone(&database), &definitions);
            cc.egui_ctx.set_fonts(definitions);
            install_picture_loaders(&cc.egui_ctx, database, proportional);
            with_ui(|s| {
                for w in s.windows.values() {
                    fallbacks.cover_tree(&cc.egui_ctx, &w.tree);
                }
                s.fallbacks = Some(fallbacks);
            });
        }
        Some(None) => {
            let text = String::from("the first run installed no fonts to install again, so egui draws its text in its own fonts");
            with_ui(|s| s.sink.log(FromHelper::Log { level: LogLevel::Warning, text }));
        }
        None => eprintln!("pwrs-gui-egui: the renderer's state was gone when its fonts were to be installed again"),
    }
    match with_ui(|s| Arc::clone(&s.web)) {
        Some(web) => cc.egui_ctx.add_bytes_loader(web),
        None => eprintln!("pwrs-gui-egui: the renderer's state was gone when its https picture loader was to be installed again"),
    }
    if with_ui(|s| {
        s.target_format = Some(rs.target_format);
        s.move_gpu(&cc.egui_ctx, rs, reason);
        s.reopen_windows();
    })
    .is_none()
    {
        eprintln!("pwrs-gui-egui: the renderer's state was gone when the GPU layer was to move to the new device");
    }
    Ok(Box::new(App))
}

/// The event loop's platform settings, where it needs any: on Windows it
/// runs off the process's first thread when `any_thread` is set; on macOS
/// the application does not take the focus as it starts while windows open
/// without it, each window's `with_active(false)` stopping it from taking
/// the focus as it opens.
#[cfg(windows)]
fn loop_settings(any_thread: bool, _no_focus: bool) -> Result<Option<eframe::EventLoopBuilderHook>, String> {
    Ok(any_thread.then(|| -> eframe::EventLoopBuilderHook {
        Box::new(|builder| {
            use winit::platform::windows::EventLoopBuilderExtWindows;
            builder.with_any_thread(true);
        })
    }))
}

#[cfg(target_os = "macos")]
fn loop_settings(any_thread: bool, no_focus: bool) -> Result<Option<eframe::EventLoopBuilderHook>, String> {
    if any_thread {
        return Err(String::from("the renderer runs off the process's first thread on Windows only"));
    }
    Ok(no_focus.then(|| -> eframe::EventLoopBuilderHook {
        Box::new(|builder| {
            use winit::platform::macos::EventLoopBuilderExtMacOS;
            builder.with_activate_ignoring_other_apps(false);
        })
    }))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn loop_settings(any_thread: bool, _no_focus: bool) -> Result<Option<eframe::EventLoopBuilderHook>, String> {
    if any_thread { Err(String::from("the renderer runs off the process's first thread on Windows only")) } else { Ok(None) }
}

/// What the hello says while windows are to open without the focus and the
/// renderer draws on Wayland, whose compositor gives a new window the focus
/// as it chooses, or when the display it draws on cannot be read.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn focus_on_wayland(cc: &eframe::CreationContext<'_>) -> Option<String> {
    use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    match cc.display_handle() {
        Ok(display) => matches!(display.as_raw(), RawDisplayHandle::Wayland(_)).then(place::no_focus_on_wayland),
        Err(e) => Some(format!("{} is 1, and the display cannot be read to tell whether windows can open without the focus: {e}", place::NO_FOCUS)),
    }
}

#[cfg(not(all(unix, not(target_vendor = "apple"))))]
fn focus_on_wayland(_cc: &eframe::CreationContext<'_>) -> Option<String> {
    None
}

/// Requests that the X11 window manager not give `window` the focus as it
/// maps it: `_NET_WM_USER_TIME` 0, which a window manager following the
/// EWMH reads as no person having asked for the window. The property goes
/// over winit's own connection, ahead of winit's request to map the window.
/// A Wayland window has no such property. What stops it from being set is
/// logged, since the window then opens taking the focus.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn unfocused_on_map(window: &winit::window::Window) {
    use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    let warn = |why: String| {
        let text = format!("a window opens taking the focus although {} is 1: {why}", place::NO_FOCUS);
        if with_ui(|s| s.sink.log(FromHelper::Log { level: LogLevel::Warning, text: text.clone() })).is_none() {
            eprintln!("pwrs-gui-egui: {text}");
        }
    };
    let handles = window.window_handle().and_then(|w| window.display_handle().map(|d| (w.as_raw(), d.as_raw())));
    let (xid, display) = match handles {
        Ok((RawWindowHandle::Xlib(w), RawDisplayHandle::Xlib(d))) => match d.display {
            Some(display) => (w.window, display.as_ptr().cast::<x11_dl::xlib::Display>()),
            None => return warn(String::from("winit names no X display for it")),
        },
        Ok(_) => return,
        Err(e) => return warn(format!("its window system handles cannot be read: {e}")),
    };
    let xlib = match x11_dl::xlib::Xlib::open() {
        Ok(x) => x,
        Err(e) => return warn(format!("libX11 could not be loaded: {e}")),
    };
    let none: std::ffi::c_long = 0;
    unsafe {
        let user_time = (xlib.XInternAtom)(display, c"_NET_WM_USER_TIME".as_ptr(), x11_dl::xlib::False);
        (xlib.XChangeProperty)(display, xid, user_time, x11_dl::xlib::XA_CARDINAL, 32, x11_dl::xlib::PropModeReplace, std::ptr::from_ref(&none).cast(), 1);
        (xlib.XFlush)(display);
    }
    proto::trace::line(|| format!("egui: X11 window {xid:#x} maps with _NET_WM_USER_TIME 0"));
}

/// The backends the renderer may use: `PWRS_GUI_BACKEND` when set, else
/// DirectX 12, Vulkan and OpenGL on Windows, Metal on macOS, and Vulkan
/// and OpenGL elsewhere, less OpenGL where the system cannot load it. The
/// library and the egui helper load `opengl32.dll` only when OpenGL is
/// used, so on Windows Server Core, which has none, they load and draw with
/// the other backends, or with OpenGL through the Mesa add-on `mesa`.
pub fn backends(mesa: Option<&Mesa>) -> Result<wgpu::Backends, String> {
    let chosen = match std::env::var("PWRS_GUI_BACKEND") {
        Err(std::env::VarError::NotPresent) => default_backends(),
        Err(e) => return Err(format!("PWRS_GUI_BACKEND cannot be read: {e}")),
        Ok(v) => parse_backends(&v)?,
    };
    loadable(chosen, opengl_present(mesa)?)
}

/// `chosen` without OpenGL when `opengl` says the system cannot load it,
/// and an error naming the missing library when no other backend is left.
fn loadable(chosen: wgpu::Backends, opengl: bool) -> Result<wgpu::Backends, String> {
    if opengl || !chosen.contains(wgpu::Backends::GL) {
        return Ok(chosen);
    }
    let left = chosen - wgpu::Backends::GL;
    if left.is_empty() {
        return Err(format!("PWRS_GUI_BACKEND names OpenGL alone, and this system has no opengl32.dll; {MESA_REMEDY}"));
    }
    Ok(left)
}

/// What gives a Windows machine without OpenGL a renderer for it.
const MESA_REMEDY: &str = "Install-Module PWRS-GUI.Mesa.win-x64 gives OpenGL a renderer that draws on the processor";

/// The `opengl32.dll` OpenGL draws through in this process, once the Mesa
/// add-on's was loaded.
#[cfg(windows)]
static MESA_OPENGL: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

/// Whether OpenGL can be loaded: through the Mesa add-on `mesa` when it was
/// chosen, or found and System32 has no `opengl32.dll`, loading it now;
/// otherwise whether System32 has one, which OpenGL loads when used.
#[cfg(windows)]
fn opengl_present(mesa: Option<&Mesa>) -> Result<bool, String> {
    let system = system_library("opengl32.dll");
    match mesa {
        Some(m) if m.chosen || !system => load_mesa(&m.folder.join("opengl32.dll")).map(|()| true),
        _ => Ok(system),
    }
}

/// Loads the Mesa add-on's `opengl32.dll` at `path` for the process's life,
/// with the libraries beside it, so the delay-loaded `opengl32.dll` imports
/// bind to it: a module of that name loaded already is the one they take.
/// An `opengl32.dll` from elsewhere loaded already is an error, since the
/// imports may have bound to it.
#[cfg(windows)]
fn load_mesa(path: &std::path::Path) -> Result<(), String> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW, LOAD_WITH_ALTERED_SEARCH_PATH, LoadLibraryExW};
    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let loaded = unsafe { GetModuleHandleW(wide("opengl32.dll".as_ref()).as_ptr()) };
    if !loaded.is_null() {
        let mut name = vec![0u16; 32768];
        let n = unsafe { GetModuleFileNameW(loaded, name.as_mut_ptr(), name.len() as u32) } as usize;
        let held = std::path::PathBuf::from(std::ffi::OsString::from_wide(&name[..n]));
        let same = held.to_string_lossy().eq_ignore_ascii_case(&path.to_string_lossy());
        return if same {
            Ok(())
        } else {
            Err(format!("this process already draws OpenGL through {}, so the Mesa add-on's {} cannot take its place until the process restarts", held.display(), path.display()))
        };
    }
    let module = unsafe { LoadLibraryExW(wide(path.as_os_str()).as_ptr(), std::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) };
    if module.is_null() {
        return Err(format!("the Mesa add-on's {} cannot be loaded: {}", path.display(), std::io::Error::last_os_error()));
    }
    MESA_OPENGL.get_or_init(|| path.to_path_buf());
    proto::trace::line(|| format!("egui: OpenGL through {}", path.display()));
    Ok(())
}

/// Whether Windows has the library `name` in System32, mapped as data so
/// none of its code runs, and freed at once.
#[cfg(windows)]
fn system_library(name: &str) -> bool {
    use windows_sys::Win32::Foundation::FreeLibrary;
    use windows_sys::Win32::System::LibraryLoader::{LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let module = unsafe { LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_SEARCH_SYSTEM32) };
    if module.is_null() {
        return false;
    }
    unsafe { FreeLibrary(module) };
    true
}

/// Why no backend offered an adapter: each library the system lacks that a
/// backend draws with on a machine without a GPU of its own, and the Mesa
/// add-on as the remedy for OpenGL, or the add-on's `opengl32.dll` when
/// OpenGL drew through it and offered none either.
#[cfg(windows)]
fn no_adapter() -> String {
    let mesa = MESA_OPENGL.get();
    let missing: Vec<&str> = [
        ("d3d10warp.dll", "d3d10warp.dll (WARP, DirectX 12's renderer for a machine without a GPU)"),
        ("vulkan-1.dll", "vulkan-1.dll (Vulkan)"),
        ("opengl32.dll", "opengl32.dll (OpenGL)"),
    ]
    .into_iter()
    .filter(|(dll, _)| !(mesa.is_some() && *dll == "opengl32.dll") && !system_library(dll))
    .map(|(_, named)| named)
    .collect();
    let mut text = String::from("no GPU adapter was found on any backend");
    if !missing.is_empty() {
        text.push_str(&format!(", and this system lacks {}", listed(&missing)));
    }
    match mesa {
        Some(path) => text.push_str(&format!("; OpenGL through {} offered none", path.display())),
        None if missing.iter().any(|m| m.starts_with("opengl32.dll")) => text.push_str(&format!("; {MESA_REMEDY}")),
        None => {}
    }
    text
}

/// Why no backend offered an adapter.
#[cfg(not(windows))]
fn no_adapter() -> String {
    String::from("no GPU adapter was found on any backend")
}

/// `items` as a list in a sentence: "a", "a and b", "a, b and c".
#[cfg(any(windows, test))]
fn listed(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Elsewhere OpenGL's libraries are loaded at run time by the backend
/// itself, which reports a system without them as having no adapter; the
/// Mesa add-on is Windows' alone.
#[cfg(not(windows))]
fn opengl_present(_mesa: Option<&Mesa>) -> Result<bool, String> {
    Ok(true)
}

fn default_backends() -> wgpu::Backends {
    if cfg!(windows) {
        wgpu::Backends::DX12 | wgpu::Backends::VULKAN | wgpu::Backends::GL
    } else if cfg!(target_vendor = "apple") {
        wgpu::Backends::METAL
    } else {
        wgpu::Backends::VULKAN | wgpu::Backends::GL
    }
}

/// Backends named in `text`, separated by commas: `dx12`, `vulkan`,
/// `metal` or `gl`, without regard to case.
fn parse_backends(text: &str) -> Result<wgpu::Backends, String> {
    let mut chosen = wgpu::Backends::empty();
    for word in text.split(',').map(str::trim).filter(|w| !w.is_empty()) {
        chosen |= match word.to_ascii_lowercase().as_str() {
            "dx12" => wgpu::Backends::DX12,
            "vulkan" => wgpu::Backends::VULKAN,
            "metal" => wgpu::Backends::METAL,
            "gl" => wgpu::Backends::GL,
            other => return Err(format!("PWRS_GUI_BACKEND names {other}; the backends are dx12, vulkan, metal and gl")),
        };
    }
    if chosen.is_empty() {
        return Err(format!("PWRS_GUI_BACKEND is '{text}', which names no backend"));
    }
    Ok(chosen)
}

/// Where a backend comes in the order: DirectX 12, Vulkan, then OpenGL
/// on Windows; Metal on macOS; Vulkan, then OpenGL elsewhere.
fn backend_rank(backend: wgpu::Backend) -> u8 {
    match backend {
        wgpu::Backend::Dx12 | wgpu::Backend::Metal => 0,
        wgpu::Backend::Vulkan if cfg!(windows) => 1,
        wgpu::Backend::Vulkan => 0,
        wgpu::Backend::Gl => 2,
        _ => 3,
    }
}

/// Where a device type comes in the order: the preferred kind of GPU, the
/// other kind, a virtual GPU, any other device, then a software adapter
/// such as WARP, llvmpipe or lavapipe.
fn device_rank(device: wgpu::DeviceType, low_power: bool) -> u8 {
    match (device, low_power) {
        (wgpu::DeviceType::DiscreteGpu, false) | (wgpu::DeviceType::IntegratedGpu, true) => 0,
        (wgpu::DeviceType::IntegratedGpu, false) | (wgpu::DeviceType::DiscreteGpu, true) => 1,
        (wgpu::DeviceType::VirtualGpu, _) => 2,
        (wgpu::DeviceType::Other, _) => 3,
        (wgpu::DeviceType::Cpu, _) => 4,
    }
}

/// Picks the adapter: of those whose name contains `wanted`, compared
/// without regard to case, the first in backend order; otherwise the best
/// by device type and then by backend order.
pub fn choose_adapter(adapters: &[wgpu::Adapter], wanted: Option<&str>, low_power: bool) -> Result<wgpu::Adapter, String> {
    if let Some(w) = wanted {
        let needle = w.to_lowercase();
        let named = adapters.iter().filter(|a| a.get_info().name.to_lowercase().contains(&needle)).min_by_key(|a| backend_rank(a.get_info().backend));
        return named.cloned().ok_or_else(|| {
            let names: Vec<String> = adapters.iter().map(|a| format!("{} ({:?})", a.get_info().name, a.get_info().backend)).collect();
            format!("no adapter's name contains {w}; the adapters are: {}", names.join("; "))
        });
    }
    let best = adapters.iter().min_by_key(|a| {
        let info = a.get_info();
        (device_rank(info.device_type, low_power), backend_rank(info.backend))
    });
    match best {
        Some(a) => Ok(a.clone()),
        None => Err(no_adapter()),
    }
}

/// The protocol's description of an adapter.
pub fn describe(info: &wgpu::AdapterInfo) -> proto::Adapter {
    proto::Adapter {
        name: info.name.clone(),
        backend: format!("{:?}", info.backend),
        device_type: format!("{:?}", info.device_type),
        driver: info.driver.clone(),
        driver_info: info.driver_info.clone(),
        vendor: info.vendor,
        device: info.device,
    }
}

/// What [`system_fonts`] finds: egui's fonts with the system's in front of
/// its own, a line for the trace on what each kind of text is drawn in and
/// how long the search took, the problems met, each worth a warning, and
/// the system's fonts themselves, which SVG pictures are drawn in, with the
/// family proportional text is drawn in when the system has it.
struct FoundFonts {
    definitions: egui::FontDefinitions,
    said: Vec<String>,
    problems: Vec<String>,
    database: Arc<fontdb::Database>,
    proportional: Option<String>,
}

/// egui's fonts with the system's in front of its own: the system's UI font
/// for proportional text, and for monospace text the font an HTML window
/// draws with; with `chosen` [`proto::Fonts::Bundled`], Inter and JetBrains
/// Mono in front of those ([`bundled_first`]), the system's then drawing
/// what they lack. A kind of text whose font the system lacks has egui's
/// in front of the rest, so a system with no fonts still draws text. The
/// system's fonts found also stand for the generic sans-serif and monospace
/// families an SVG picture names.
fn system_fonts(chosen: proto::Fonts) -> FoundFonts {
    let started = std::time::Instant::now();
    let mut db = proto::typeface::system_database();
    let mut fonts = egui::FontDefinitions::default();
    let mut said = Vec::new();
    let mut proportional = None;
    let proto::typeface::Families { ui, mono, mut problems } = proto::typeface::wanted();
    for (family, key, kind, wanted) in [(egui::FontFamily::Proportional, "system-ui", "proportional", ui), (egui::FontFamily::Monospace, "system-mono", "monospace", mono)] {
        match proto::typeface::first_found(&db, &wanted) {
            Ok(Some(font)) => {
                said.push(format!("{kind} text in {}, from {}", font.family, font.path));
                if family == egui::FontFamily::Proportional {
                    db.set_sans_serif_family(font.family.clone());
                    proportional = Some(font.family.clone());
                } else {
                    db.set_monospace_family(font.family.clone());
                }
                fonts.font_data.insert(key.to_string(), Arc::new(egui::FontData::from_blob(Arc::new(font.data), font.index)));
                fonts.families.entry(family).or_default().insert(0, key.to_string());
            }
            Ok(None) if wanted.is_empty() => said.push(format!("{kind} text in egui's own font: the system names no font for it")),
            Ok(None) => said.push(format!("{kind} text in egui's own font: the system has none of {wanted:?}")),
            Err(why) => problems.push(why),
        }
    }
    if chosen == proto::Fonts::Bundled {
        bundled_first(&mut fonts);
        said.push(format!("text in {} and monospace text in {}, the build's own, the fonts above behind them", pwrs_gui_fonts::INTER_FAMILY, pwrs_gui_fonts::JETBRAINS_MONO_FAMILY));
    }
    said.push(format!("{} system font faces read in {} ms", db.len(), started.elapsed().as_millis()));
    FoundFonts { definitions: fonts, said, problems, database: Arc::new(db), proportional }
}

/// Puts the fonts the build includes in front of every other in `fonts`:
/// Inter for proportional text and JetBrains Mono NL for monospace text.
fn bundled_first(fonts: &mut egui::FontDefinitions) {
    for (family, key, data) in [(egui::FontFamily::Proportional, "bundled-ui", pwrs_gui_fonts::INTER), (egui::FontFamily::Monospace, "bundled-mono", pwrs_gui_fonts::JETBRAINS_MONO)] {
        fonts.font_data.insert(key.to_string(), Arc::new(egui::FontData::from_static(data)));
        fonts.families.entry(family).or_default().insert(0, key.to_string());
    }
}

/// Lets egui draw an Image's pictures: the image crate's decoders through
/// egui_extras, which animate GIF and WebP pictures as well, and SVG
/// through [`SvgLoader`] in the fonts of `fonts`, `proportional` the
/// family of text an SVG names none for. None of them reads a file or the
/// network: an Image's bytes come with its node. egui tries the loader
/// added last first, so the SVG loader, which takes only SVG, goes last.
fn install_picture_loaders(ctx: &egui::Context, fonts: Arc<fontdb::Database>, proportional: Option<String>) {
    use egui_extras::loaders::{gif_loader::GifLoader, image_loader::ImageCrateLoader, webp_loader::WebPLoader};
    ctx.add_image_loader(Arc::new(ImageCrateLoader::default()));
    ctx.add_image_loader(Arc::new(GifLoader::default()));
    ctx.add_image_loader(Arc::new(WebPLoader::default()));
    ctx.add_image_loader(Arc::new(SvgLoader::new(fonts, proportional)));
}

/// Draws SVG pictures for egui with resvg, in the system's fonts as the
/// helper read them for its own text, so SVG text needs no second search of
/// them. It takes addresses ending in `.svg`, which the session's pictures
/// are given, and an https picture whose bytes are SVG, but never one frame
/// of an animated picture's; of what an SVG's `<image>` elements link to it
/// takes only data in the SVG itself, so a picture never makes the helper
/// read a file or fetch another. A picture is stored at each size it is drawn
/// at, and of several sizes of one picture, those no frame drew lately are
/// let go.
struct SvgLoader {
    options: resvg::usvg::Options<'static>,
    drawn: ArcSwap<HashMap<String, HashMap<SizeHint, Arc<SvgDrawn>>>>,
    pass: AtomicU64,
}

/// One SVG picture drawn at one size, or why it could not be, and the last
/// frame that requested it.
struct SvgDrawn {
    result: Result<Arc<egui::ColorImage>, String>,
    last_pass: AtomicU64,
}

impl SvgLoader {
    const ID: &'static str = egui::generate_loader_id!(SvgLoader);

    fn new(fonts: Arc<fontdb::Database>, proportional: Option<String>) -> SvgLoader {
        let mut options = resvg::usvg::Options { fontdb: fonts, ..resvg::usvg::Options::default() };
        if let Some(family) = proportional {
            options.font_family = family;
        }
        options.image_href_resolver = resvg::usvg::ImageHrefResolver { resolve_data: resvg::usvg::ImageHrefResolver::default_data_resolver(), resolve_string: Box::new(|_, _| None) };
        SvgLoader { options, drawn: ArcSwap::from_pointee(HashMap::new()), pass: AtomicU64::new(0) }
    }
}

impl ImageLoader for SvgLoader {
    fn id(&self) -> &str {
        Self::ID
    }

    fn load(&self, ctx: &egui::Context, uri: &str, size_hint: SizeHint) -> ImageLoadResult {
        let fetched = uri.starts_with("https://") && !names_a_frame(uri);
        if !fetched && !uri.ends_with(".svg") {
            return Err(LoadError::NotSupported);
        }
        let pass = self.pass.load(Ordering::Relaxed);
        if let Some(found) = self.drawn.load().get(uri).and_then(|sizes| sizes.get(&size_hint)) {
            found.last_pass.store(pass, Ordering::Relaxed);
            return match &found.result {
                Ok(image) => Ok(ImagePoll::Ready { image: Arc::clone(image) }),
                Err(why) => Err(LoadError::Loading(why.clone())),
            };
        }
        match ctx.try_load_bytes(uri) {
            Ok(BytesPoll::Ready { bytes, .. }) => {
                if fetched && !proto::picture::is_svg(&bytes) {
                    return Err(LoadError::NotSupported);
                }
                let result = draw_svg(&bytes, size_hint, &self.options).map(Arc::new);
                let entry = Arc::new(SvgDrawn { result: result.clone(), last_pass: AtomicU64::new(pass) });
                self.drawn.rcu(|m| {
                    let mut m = (**m).clone();
                    m.entry(uri.to_string()).or_default().insert(size_hint, Arc::clone(&entry));
                    m
                });
                match result {
                    Ok(image) => Ok(ImagePoll::Ready { image }),
                    Err(why) => Err(LoadError::Loading(why)),
                }
            }
            Ok(BytesPoll::Pending { size }) => Ok(ImagePoll::Pending { size }),
            Err(e) => Err(e),
        }
    }

    fn forget(&self, uri: &str) {
        self.drawn.rcu(|m| {
            let mut m = (**m).clone();
            m.remove(uri);
            m
        });
    }

    fn forget_all(&self) {
        self.drawn.store(Arc::new(HashMap::new()));
    }

    fn byte_size(&self) -> usize {
        let held = |d: &Arc<SvgDrawn>| match &d.result {
            Ok(image) => image.pixels.len() * std::mem::size_of::<egui::Color32>(),
            Err(why) => why.len(),
        };
        self.drawn.load().values().flat_map(|sizes| sizes.values()).map(held).sum()
    }

    fn end_pass(&self, pass_index: u64) {
        self.pass.store(pass_index, Ordering::Relaxed);
        let lately = |d: &Arc<SvgDrawn>| d.last_pass.load(Ordering::Relaxed) + 1 >= pass_index;
        if !self.drawn.load().values().any(|sizes| sizes.len() > 1 && !sizes.values().all(lately)) {
            return;
        }
        self.drawn.rcu(|m| {
            let mut m = (**m).clone();
            for sizes in m.values_mut() {
                if sizes.len() > 1 {
                    sizes.retain(|_, d| lately(d));
                }
            }
            m.retain(|_, sizes| !sizes.is_empty());
            m
        });
    }
}

/// Whether `uri` names one frame of an animated picture, as egui writes
/// one: the picture's address, `#`, and the frame's number.
fn names_a_frame(uri: &str) -> bool {
    uri.rsplit_once('#').is_some_and(|(_, frame)| !frame.is_empty() && frame.bytes().all(|b| b.is_ascii_digit()))
}

/// An SVG picture drawn at the size `hint` requests, in its own proportions
/// unless the hint lets them change.
fn draw_svg(bytes: &[u8], hint: SizeHint, options: &resvg::usvg::Options<'_>) -> Result<egui::ColorImage, String> {
    let tree = resvg::usvg::Tree::from_data(bytes, options).map_err(|e| e.to_string())?;
    let source = egui::vec2(tree.size().width(), tree.size().height());
    let size = match hint {
        SizeHint::Size { width, height, maintain_aspect_ratio: true } => {
            let mut size = source * (width as f32 / source.x);
            if size.y > height as f32 {
                size *= height as f32 / size.y;
            }
            size
        }
        SizeHint::Size { width, height, maintain_aspect_ratio: false } => egui::vec2(width as f32, height as f32),
        SizeHint::Height(h) => source * (h as f32 / source.y),
        SizeHint::Width(w) => source * (w as f32 / source.x),
        SizeHint::Scale(scale) => source * scale.into_inner(),
    }
    .round();
    let (w, h) = (size.x as u32, size.y as u32);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or_else(|| format!("an SVG picture cannot be drawn {w} by {h} pixels"))?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(w as f32 / source.x, h as f32 / source.y), &mut pixmap.as_mut());
    Ok(egui::ColorImage::from_rgba_premultiplied([w as usize, h as usize], pixmap.data()).with_source_size(source))
}

struct App;

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if with_ui(|s| s.pump(ctx)).is_none() {
            eprintln!("pwrs-gui-egui: the root's frame began with no renderer state on this thread");
            return;
        }
        let shown = with_ui(|s| {
            s.deliver_shots();
            s.prune_closed();
            // A run whose device is lost shows no window, which would draw on
            // that device; the next run opens every window again.
            if s.restart.is_some() {
                return (Vec::new(), Vec::new());
            }
            s.size_new_windows(ctx);
            let drawn = s.stand_ins.viewports();
            let ids: Vec<egui::ViewportId> = s.windows.keys().map(|id| viewport_id(*id)).chain(drawn.iter().map(|(request, _)| standin::viewport_id(*request))).collect();
            if **s.viewports.load() != ids {
                s.viewports.store(Arc::new(ids));
            }
            let windows = s
                .windows
                .iter_mut()
                .map(|(id, w)| {
                    w.shown = true;
                    (*id, w.builder(), std::mem::take(&mut w.focus_asked))
                })
                .collect::<Vec<_>>();
            (windows, drawn)
        });
        let Some((shown, drawn)) = shown else {
            eprintln!("pwrs-gui-egui: a frame began with no renderer state on this thread");
            return;
        };
        for (id, builder, focus) in shown {
            ctx.show_viewport_deferred(viewport_id(id), builder, move |ui, _class| {
                if with_ui(|s| s.frame(id, ui)).is_none() {
                    eprintln!("pwrs-gui-egui: window {id} was drawn with no renderer state on this thread");
                }
            });
            if focus {
                ctx.send_viewport_cmd_to(viewport_id(id), egui::ViewportCommand::Focus);
            }
        }
        for (request, builder) in drawn {
            ctx.show_viewport_deferred(standin::viewport_id(request), builder, move |ui, _class| {
                if with_ui(|s| s.stand_in_frame(request, ui)).is_none() {
                    eprintln!("pwrs-gui-egui: dialog {request} was drawn with no renderer state on this thread");
                }
            });
        }
    }

    fn ui(&mut self, _ui: &mut egui::Ui, _frame: &mut eframe::Frame) {}

    /// A window's frame takes the next step of a test's input given it
    /// ([`UiState::give_input`]), which eframe runs for every viewport.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if with_ui(|s| s.give_input(ctx, raw_input)).is_none() {
            eprintln!("pwrs-gui-egui: a frame's input was read with no renderer state on this thread");
        }
    }
}

fn viewport_id(window: WindowId) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("pwrs-gui", window))
}

/// Pins AMD's hook library, `amdihk64.dll`, so it stays loaded for the life
/// of the process. AMD's Direct3D 12 and Vulkan drivers load it into a process
/// that only probes an AMD adapter, and unload it with the GPU objects at a
/// run's end while its hook stays installed, so a later run in the process
/// would call into the unloaded library. `None` when it is pinned or was
/// never loaded; otherwise why it could not be pinned.
#[cfg(windows)]
fn keep_driver_hooks() -> Option<String> {
    use windows_sys::Win32::Foundation::{ERROR_MOD_NOT_FOUND, GetLastError};
    use windows_sys::Win32::System::LibraryLoader::{GET_MODULE_HANDLE_EX_FLAG_PIN, GetModuleHandleExW};
    let name: Vec<u16> = "amdihk64.dll".encode_utf16().chain(std::iter::once(0)).collect();
    let mut module = std::ptr::null_mut();
    if unsafe { GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_PIN, name.as_ptr(), &mut module) } != 0 {
        proto::trace::line(|| String::from("egui: amdihk64.dll pinned"));
        return None;
    }
    match unsafe { GetLastError() } {
        ERROR_MOD_NOT_FOUND => None,
        code => Some(format!("AMD's hook library amdihk64.dll could not be made to stay loaded (error {code}), so a later in-process run may call into it after the driver unloads it")),
    }
}

#[cfg(not(windows))]
fn keep_driver_hooks() -> Option<String> {
    None
}

/// The top-level windows the calling thread owns, as addresses.
#[cfg(windows)]
fn thread_windows() -> Vec<usize> {
    use windows_sys::Win32::Foundation::{HWND, LPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::WindowsAndMessaging::EnumThreadWindows;
    unsafe extern "system" fn add(hwnd: HWND, found: LPARAM) -> windows_sys::core::BOOL {
        unsafe { (*(found as *mut Vec<usize>)).push(hwnd as usize) };
        1
    }
    let mut found: Vec<usize> = Vec::new();
    // FALSE when the thread owns no window, which leaves `found` empty.
    unsafe { EnumThreadWindows(GetCurrentThreadId(), Some(add), &mut found as *mut Vec<usize> as LPARAM) };
    found
}

/// The title of window `hwnd`.
#[cfg(windows)]
fn window_title(hwnd: usize) -> String {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};
    let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
    let len = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
    let mut text = vec![0u16; len + 1];
    let copied = unsafe { GetWindowTextW(hwnd, text.as_mut_ptr(), text.len() as i32) }.max(0) as usize;
    text.truncate(copied);
    String::from_utf16_lossy(&text)
}

/// Requests that Windows make `hwnd` the foreground window, and returns
/// whether it did. Windows refuses a process that may not take the front.
#[cfg(windows)]
fn raise_window(hwnd: usize) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
    unsafe { SetForegroundWindow(hwnd as windows_sys::Win32::Foundation::HWND) != 0 }
}

/// The operating-system window in front, if any.
#[cfg(windows)]
fn foreground() -> Option<usize> {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.is_null()).then_some(hwnd as usize)
}

/// Gives the front back to window `hwnd`, while it exists and this process
/// has the front, as it does while one of its windows is in front: a
/// window closing in front would otherwise pass the front to another window
/// of this thread, the hidden root among them.
#[cfg(windows)]
fn hand_back(hwnd: usize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, IsWindow};
    let front = unsafe { GetForegroundWindow() };
    let mut holder = 0u32;
    unsafe { GetWindowThreadProcessId(front, &mut holder) };
    if holder != std::process::id() || unsafe { IsWindow(hwnd as windows_sys::Win32::Foundation::HWND) } == 0 {
        return;
    }
    let given = raise_window(hwnd);
    proto::trace::line(|| format!("egui: the front given back to window {hwnd:#x}: {given}"));
}

/// The monitors a window can open on, as Windows numbers them.
#[cfg(windows)]
fn screens() -> Result<Vec<place::Monitor>, String> {
    place::monitors()
}

/// The work area in points of the screen a new window opens on: of the
/// monitor `monitor` names, or else the primary one, as Windows lists them;
/// or why it cannot be read.
#[cfg(windows)]
fn work_area(_ctx: &egui::Context, monitor: Option<u32>) -> Result<[f32; 2], String> {
    let listed = screens()?;
    place::opening_on(&listed, monitor).map(place::Monitor::work_points).ok_or_else(|| String::from("Windows lists no primary monitor"))
}

/// The size in points of the monitor the hidden root is on, as egui reads
/// it, which takes no dock or panel off; or why it cannot be read.
#[cfg(not(windows))]
fn work_area(ctx: &egui::Context, _monitor: Option<u32>) -> Result<[f32; 2], String> {
    ctx.input(|i| i.viewport().monitor_size).map(|s| [s.x, s.y]).ok_or_else(|| String::from("the window system names no monitor for the renderer's hidden window"))
}

#[cfg(not(windows))]
fn screens() -> Result<Vec<place::Monitor>, String> {
    Err(String::from("the egui renderer lists monitors on Windows only"))
}

/// Where a window of `width` by `height` points opens centered on monitor
/// `n`, in the points egui gives winit: physical pixels over the primary
/// monitor's scale, since winit converts a new window's position with the
/// scale of the monitor it was made on, the primary one. `None` when `n`
/// names no monitor.
fn position_on(n: u32, width: f32, height: f32) -> Result<Option<[f32; 2]>, String> {
    let listed = screens()?;
    let Some(m) = place::pick(&listed, n) else {
        return Ok(None);
    };
    let (x, y) = place::centered(m, f64::from(width), f64::from(height));
    let primary = listed.iter().find(|p| p.x == 0 && p.y == 0).map_or(1.0, |p| p.scale);
    Ok(Some([(f64::from(x) / primary) as f32, (f64::from(y) / primary) as f32]))
}

/// Starts the GPU thread on the device eframe made. A layer or thread that
/// cannot start leaves every GPU call refused, and the log says why.
fn start_gpu(rs: &egui_wgpu::RenderState) {
    let started = with_ui(|s| -> Result<(), String> {
        let Some(wake) = s.wake.clone() else {
            return Err(String::from("the loop cannot be woken yet"));
        };
        let layer = pwrs_gui_gpu::Layer::adopt(rs.adapter.clone(), rs.device.clone(), rs.queue.clone(), "PWRS-GUI egui").map_err(|f| f.to_string())?;
        s.gpu = Some(gpu_thread::GpuThread::start(layer, wake)?);
        Ok(())
    });
    let why = match started {
        Some(Ok(())) => return,
        Some(Err(why)) => why,
        None => {
            eprintln!("pwrs-gui-egui: the renderer's state was gone when the GPU layer was to start");
            return;
        }
    };
    let text = format!("the GPU layer did not start, so GPU calls are refused: {why}");
    if with_ui(|s| s.sink.log(FromHelper::Log { level: LogLevel::Warning, text: text.clone() })).is_none() {
        eprintln!("pwrs-gui-egui: {text}");
    }
}

/// Everything the renderer stores, owned by the thread running it.
struct UiState {
    /// The session's messages, which the frame of whichever viewport runs
    /// first takes.
    inbox: Receiver<Inbound>,
    /// The viewports of the open windows, which a message wakes besides
    /// the root; set at each of the root's frames.
    viewports: Arc<ArcSwap<Vec<egui::ViewportId>>>,
    sink: Box<dyn Sink + Send>,
    /// The dialogs that have closed, whose results the pump sends.
    dialogs_done: Arc<SegQueue<DoneDialog>>,
    /// The captures wgpu has read back, each with its request and the
    /// viewport it was requested from, which the next frame replies with.
    shots_arrived: Arc<SegQueue<ArrivedShot>>,
    /// The dialogs the renderer draws where the system has none of their
    /// kind ([`standin`]).
    stand_ins: standin::StandIns,
    /// What wakes the loop from another thread; none until it can be woken.
    wake: Option<WakeLoop>,
    windows: BTreeMap<WindowId, WindowState>,
    adapters: Vec<proto::Adapter>,
    closing: bool,
    hello_sent: bool,
    /// The monitor `PWRS_GUI_MONITOR` names, which new windows open
    /// centered on; `None` for where the system places them.
    monitor: Option<u32>,
    /// Whether `PWRS_GUI_NO_FOCUS` opens windows without taking the focus,
    /// so none is activated or brought to the front.
    no_focus: bool,
    /// The top-level windows this thread owned before any window opened,
    /// the hidden root among them; never brought to the front.
    #[cfg(windows)]
    foreign: Vec<usize>,
    /// Whether a new window is brought to the front at its first paint.
    #[cfg(windows)]
    bring_to_front: bool,
    /// The addresses of the pictures of windows that have closed, which
    /// egui forgets at the next frame.
    forgotten: Vec<String>,
    /// The loader of Markdown's https pictures, whose notes each frame
    /// passes on to the session.
    web: Arc<web::WebLoader>,
    /// The system's faces for scripts egui's fonts lack, read as
    /// windows' text needs them; none until the fonts are installed.
    fallbacks: Option<fallback::Fallbacks>,
    /// The SubEtha rings windows' widgets read, each on a thread of its own
    /// that wakes the loop; none until the loop can be woken.
    sources: Option<proto::source::Sources>,
    /// The GPU thread, running the GPU layer on the renderer's device; none
    /// until eframe has made the device, or when the thread could not start.
    gpu: Option<gpu_thread::GpuThread>,
    /// Why the GPU device was lost, while the run ends so the next starts
    /// on a new device.
    restart: Option<String>,
    /// The fonts the first run installed, which a later run installs again.
    installed: Option<Installed>,
    /// GPU calls replied to once the renderer is on a new device: calls
    /// waiting for a lost device, and resets.
    loss_watchers: Vec<(RequestId, RingName)>,
    /// GPU calls that arrived while the layer moves to a new device.
    held_gpu: Vec<HeldGpu>,
    /// The format egui draws its windows in, which a GpuSurface's blit
    /// draws into; none until eframe has made the device.
    target_format: Option<wgpu::TextureFormat>,
    /// The GPU errors raised outside any call that have been reported on
    /// the layer's device.
    gpu_reported: Reported,
    /// What measures a window that fits its content before it opens; none
    /// until the first such window.
    measure: Option<Measure>,
}

/// Measures the content of a window that fits it, before the window opens:
/// an egui context of its own that draws nothing, in the fonts and style of
/// the renderer's, which lays the window out at its widgets' own sizes.
struct Measure {
    ctx: egui::Context,
    /// How many faces its fonts have, so it takes the renderer's again once
    /// the faces for a script the window's text needs are added.
    faces: usize,
}

impl Measure {
    fn new() -> Measure {
        let ctx = egui::Context::default();
        look::apply(&ctx);
        Measure { ctx, faces: 0 }
    }

    /// The inside size in points `w`'s content takes, laid out within the
    /// most its options let each fitted side take on a screen whose work
    /// area is `work`, in the faces `fonts` defines, at `ppp` physical
    /// pixels a point. Fonts set take effect in the pass that follows.
    fn content(&mut self, w: &mut WindowState, fonts: egui::FontDefinitions, ppp: f32, work: [f32; 2]) -> [f32; 2] {
        if fonts.font_data.len() != self.faces {
            self.faces = fonts.font_data.len();
            self.ctx.set_fonts(fonts);
        }
        let [across, down] = w.options.fitted([f32::INFINITY; 2], Some(work));
        let mut input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(across, down))), ..Default::default() };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
        let mut size = egui::Vec2::ZERO;
        // A Grid lays its columns out at the widths its previous frame
        // measured, and at its least column width in its first, so the
        // content's size is read from a second frame.
        for _ in 0..2 {
            self.ctx.run_ui(input.clone(), |ui| size = w.natural(ui)).drop_without_applying_deltas();
        }
        [size.x, size.y]
    }
}


/// The texts of the GPU errors raised outside any call that have been
/// reported while the layer has its device, so an error raised in every
/// frame is reported once; on a new device each is reported again.
#[derive(Default)]
struct Reported(HashSet<String>);

impl Reported {
    /// Whether `text` has not been reported on this device, noting that it
    /// now is.
    fn first(&mut self, text: &str) -> bool {
        if self.0.contains(text) {
            return false;
        }
        self.0.insert(text.to_string())
    }

    /// Forgets every text, for a layer on a new device.
    fn forget(&mut self) {
        self.0.clear();
    }
}

/// A GPU call that waits for the layer to move to a new device.
enum HeldGpu {
    Call { request: RequestId, reply: RingName, op: Box<GpuOp> },
    Free(u64),
    FreePipeline(u64),
    FreeTexture(u64),
    FreeSampler(u64),
}

/// What a camera call requests of a GpuSurface's camera.
enum CameraCall {
    Set(proto::camera::CameraChange),
    Get,
    Remove,
}

/// The fonts the first run installed: egui's definitions with the
/// system's faces in front, the system's font database, and the family SVG
/// pictures draw sans-serif text in.
struct Installed {
    definitions: egui::FontDefinitions,
    database: Arc<fontdb::Database>,
    proportional: Option<String>,
}

/// A dialog the session requested that has closed: its request, the ring
/// its answer goes to, and how it was answered.
struct DoneDialog {
    request: RequestId,
    reply: RingName,
    outcome: Result<proto::DialogAnswer, proto::DialogFailure>,
}

impl UiState {
    /// Handles the session's messages waiting, sends the results of the
    /// dialogs that have closed, opens the ones the renderer draws in the
    /// system's place, passes the https loader's notes on, and refuses the
    /// captures of windows no longer drawn. Both the hidden root's frame and
    /// each window's take them: while a window draws every frame, as a
    /// spinner's does, eframe can go on without painting the root, which
    /// would leave them unread.
    fn pump(&mut self, ctx: &egui::Context) {
        loop {
            match self.inbox.try_recv() {
                Ok(m) => {
                    proto::trace::line(|| format!("egui: message taken in viewport {:?}", ctx.viewport_id()));
                    self.handle(ctx, m);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.shut_down(ctx);
                    break;
                }
            }
        }
        while let Some(DoneDialog { request, reply, outcome }) = self.dialogs_done.pop() {
            self.sink.reply(&reply, FromHelper::DialogResult { request, outcome });
        }
        let (answers, errors, lost) = match &self.gpu {
            Some(g) => (g.answers(), g.take_errors(), g.lost_reason()),
            None => (Vec::new(), Vec::new(), None),
        };
        for answer in answers {
            self.send_answer(ctx, answer);
        }
        match lost {
            // Errors raised once the device is lost come from its loss,
            // which the move to a new device resolves; they go to the log
            // alone.
            Some(reason) => {
                for text in errors {
                    self.sink.log(FromHelper::Error { window: None, context: String::from("GPU"), text });
                }
                self.lose_device(ctx, reason);
            }
            None => {
                for text in errors {
                    self.gpu_error(text);
                }
            }
        }
        for text in self.stand_ins.take(ctx, self.no_focus) {
            self.sink.log(FromHelper::Log { level: LogLevel::Verbose, text });
        }
        let read = match &self.sources {
            Some(s) => s.take(),
            None => Vec::new(),
        };
        for news in read {
            self.ring_news(ctx, news);
        }
        for text in self.web.take_notes() {
            self.sink.log(FromHelper::Log { level: LogLevel::Verbose, text });
        }
        self.refuse_hidden_shots(ctx);
        self.refuse_late_saves(ctx);
    }

    /// A GPU error raised outside any GPU call, the renderer's own drawing
    /// among them: to the command of every open window, and to the log,
    /// once for as long as the layer has its device.
    fn gpu_error(&mut self, text: String) {
        if !self.gpu_reported.first(&text) {
            return;
        }
        let open: Vec<(WindowId, RingName)> = self.windows.iter().filter_map(|(id, w)| w.ring.clone().map(|r| (*id, r))).collect();
        for (window, ring) in open {
            self.send_window(Some(&ring), FromHelper::Error { window: Some(window), context: String::from("GPU"), text: text.clone() });
        }
        self.sink.log(FromHelper::Error { window: None, context: String::from("GPU"), text });
    }

    /// What a widget's ring or shared memory brought: an item becomes the
    /// patch it makes, and a shared memory's rows the changes they make to
    /// its canvas's items, each applied as the session's patches are; an
    /// item or a row the widget cannot take, or a reader that cannot read,
    /// is reported to the window's command, a shared memory's refusal of
    /// its rows once until its rows are taken again.
    fn ring_news(&mut self, ctx: &egui::Context, news: proto::source::News) {
        use proto::source::{Feed, News};
        let gone = "an item came for a widget the window no longer has";
        let (window, feed, text) = match news {
            News::Item { window, node, item } => {
                let Some(w) = self.windows.get(&window) else {
                    return;
                };
                match w.tree.find(node) {
                    Some(n) => match proto::source::item_ops(n, &item) {
                        Ok(ops) => return self.patch_window(ctx, window, ops, "ring"),
                        Err(why) => (window, Feed::Ring, proto::source::refused(n, &why, &item)),
                    },
                    None => (window, Feed::Ring, proto::source::stopped(None, node, Feed::Ring, gone)),
                }
            }
            News::Changes { window, node, item, changes } => {
                let Some(w) = self.windows.get(&window) else {
                    return;
                };
                match w.tree.find(node) {
                    Some(n) => match changes.and_then(|c| proto::source::canvas_ops(n, canvas::board(&w.inputs.canvases, node), c)) {
                        Ok(ops) => return self.patch_window(ctx, window, ops, "ring"),
                        Err(why) => (window, Feed::Ring, proto::source::refused(n, &why, &item)),
                    },
                    None => (window, Feed::Ring, proto::source::stopped(None, node, Feed::Ring, gone)),
                }
            }
            News::Rows { window, node, rows } => {
                let Some(w) = self.windows.get(&window) else {
                    return;
                };
                let Some(n) = w.tree.find(node) else {
                    return;
                };
                let ops = match proto::shared::SharedSource::of(n) {
                    Some(Ok(source)) => proto::shared::canvas_ops(n, canvas::board(&w.inputs.canvases, node), &source, &rows),
                    Some(Err(why)) => Err(why),
                    None => return,
                };
                let refused = match ops {
                    Ok(ops) => self.apply_patch(ctx, window, ops, Feed::SharedMemory.name()).into_iter().next(),
                    Err(why) => Some(why),
                };
                if !self.sources.as_mut().is_some_and(|s| s.refusal(window, node, refused.as_deref())) {
                    return;
                }
                let Some(why) = refused else {
                    return;
                };
                let n = self.windows.get(&window).and_then(|w| w.tree.find(node));
                (window, Feed::SharedMemory, proto::source::stopped(n, node, Feed::SharedMemory, &why))
            }
            News::Problem { window, node, feed, text } => {
                let n = self.windows.get(&window).and_then(|w| w.tree.find(node));
                (window, feed, proto::source::stopped(n, node, feed, &text))
            }
        };
        let ring = self.windows.get(&window).and_then(|w| w.ring.clone());
        self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: feed.name().to_string(), text });
    }

    /// Gives `raw`, the input of the frame of a window's viewport about to
    /// run, the next part of a test's input to the window. It requests a
    /// frame for the next step, while the window's own frame requests one
    /// for when a wait ends ([`UiState::frame`]). It replies to the test once
    /// the frame after its last step has run, or when a wait cannot be
    /// counted.
    fn give_input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let viewport = raw.viewport_id;
        let Some(w) = self.windows.values_mut().find(|w| viewport_id(w.id) == viewport) else {
            return;
        };
        let Some(given) = w.input.as_mut() else {
            return;
        };
        let fed = given.feed(raw, std::time::Instant::now());
        if let Some(step) = given.began() {
            let (window, total) = (w.id, given.total);
            proto::trace::line(|| format!("egui: window {window} input step {step} of {total} begins"));
            self.sink.reply(&given.reply, FromHelper::InputStep { request: given.request, step });
        }
        let outcome = match fed {
            input::Fed::Step => return ctx.request_repaint_of(viewport),
            input::Fed::Wait => return,
            input::Fed::Done => Ok(Value::Null),
            input::Fed::Refused(why) => Err(why),
        };
        if let Some(done) = w.input.take() {
            let window = w.id;
            proto::trace::line(|| format!("egui: window {window} input of request {} ends: {outcome:?}", done.request));
            self.sink.reply(&done.reply, FromHelper::TestEventDone { request: done.request, outcome });
        }
    }

    /// Requests a frame of window `window`, or of the root when no such
    /// window is open or the root has not yet shown it
    /// ([`WindowState::repaint_target`]).
    fn repaint_window(&self, ctx: &egui::Context, window: WindowId) {
        ctx.request_repaint_of(self.windows.get(&window).map_or(egui::ViewportId::ROOT, WindowState::repaint_target));
    }

    /// Applies `ops` to window `window` as a patch from the session is
    /// applied, each that fails reported under `context`, and reads the
    /// rings and shared memory its widgets name once they are applied.
    fn patch_window(&mut self, ctx: &egui::Context, window: WindowId, ops: Vec<PatchOp>, context: &str) {
        let problems = self.apply_patch(ctx, window, ops, context);
        let ring = self.windows.get(&window).and_then(|w| w.ring.clone());
        for p in problems {
            self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: context.to_string(), text: p });
        }
    }

    /// Applies `ops` to window `window` as a patch from the session is
    /// applied, reads the rings and shared memory its widgets name once they
    /// are applied, and returns why each op that failed did; a window not
    /// open is logged under `context`.
    fn apply_patch(&mut self, ctx: &egui::Context, window: WindowId, ops: Vec<PatchOp>, context: &str) -> Vec<String> {
        let Some(w) = self.windows.get_mut(&window) else {
            self.log_error(context, format!("no open window {window}"));
            return Vec::new();
        };
        if let Some(f) = self.fallbacks.as_mut() {
            for op in &ops {
                match op {
                    PatchOp::SetProps { props, .. } => props.values().for_each(|v| f.cover_value(ctx, v)),
                    PatchOp::Insert { node, .. } | PatchOp::Replace { node } => f.cover_tree(ctx, node),
                    PatchOp::Append { values, .. } => values.iter().for_each(|v| f.cover_value(ctx, v)),
                    PatchOp::SetItem { props, .. } => props.values().for_each(|v| f.cover_value(ctx, v)),
                    PatchOp::AddItems { items, .. } => items.iter().for_each(|v| f.cover_value(ctx, v)),
                    PatchOp::Remove { .. } | PatchOp::RemoveRows { .. } | PatchOp::RemoveItems { .. } => {}
                }
            }
        }
        let problems = w.patch(ops);
        if let Some(s) = self.sources.as_mut() {
            s.follow(window, &w.tree);
        }
        self.repaint_window(ctx, window);
        problems
    }

    fn handle(&mut self, ctx: &egui::Context, inbound: Inbound) {
        let Inbound { msg, attach } = inbound;
        if let Some(a) = attach {
            match attached_ring(&msg) {
                Some(ring) => self.sink.attach(ring, a),
                None => self.log_error("attach", format!("an attachment arrived with {msg:?}, which names no ring")),
            }
        }
        match msg {
            ToHelper::OpenWindow { window, ring, options, tree, rows } => {
                if let Some(f) = self.fallbacks.as_mut() {
                    f.cover(ctx, &options.title);
                    f.cover_tree(ctx, &tree);
                    rows.iter().flat_map(|(_, rs)| rs.iter()).flat_map(|r| r.cells.iter()).for_each(|c| f.cover(ctx, &c.text));
                }
                let mut state = WindowState::new(window, ring, options, tree);
                for (node, opened) in rows {
                    if let Err(e) = state.put_rows(node, 0, opened, false) {
                        self.send_window(state.ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("OpenWindow"), text: e });
                    }
                }
                state.takes_focus = !self.no_focus;
                state.inputs.no_focus = self.no_focus;
                state.inputs.surface_format = self.target_format;
                state.inputs.stand_ins = Some(Arc::clone(self.stand_ins.waiting()));
                if let Some(problem) = state.problem.clone() {
                    self.send_window(state.ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("OpenWindow"), text: problem });
                }
                if let Some(s) = self.sources.as_mut() {
                    s.follow(window, &state.tree);
                }
                self.windows.insert(window, state);
                ctx.request_repaint_of(egui::ViewportId::ROOT);
            }
            ToHelper::Patch { window, ops } => self.patch_window(ctx, window, ops, "Patch"),
            ToHelper::Rows { window, node, start, rows, added } => match self.windows.get_mut(&window) {
                Some(w) => {
                    if let Some(f) = self.fallbacks.as_mut() {
                        rows.iter().flat_map(|r| r.cells.iter()).for_each(|c| f.cover(ctx, &c.text));
                    }
                    let target = w.repaint_target();
                    if let Err(e) = w.put_rows(node, start, rows, added) {
                        let ring = w.ring.clone();
                        self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("Rows"), text: e });
                    }
                    ctx.request_repaint_of(target);
                }
                None => self.log_error("Rows", format!("no open window {window}")),
            },
            ToHelper::CloseWindow { window } => {
                if let Some(w) = self.windows.remove(&window) {
                    self.stop_reading(window);
                    self.send_window(w.ring.as_deref(), FromHelper::Closed { window, reason: CloseReason::Program });
                    self.retire(w);
                }
            }
            ToHelper::CloseOpen { request, reply, window } => {
                let outcome = match self.windows.remove(&window) {
                    Some(w) => {
                        self.stop_reading(window);
                        self.send_window(w.ring.as_deref(), FromHelper::Closed { window, reason: CloseReason::Program });
                        self.retire(w);
                        Ok(())
                    }
                    None => Err(format!("no window is open with the number {window}")),
                };
                self.sink.reply(&reply, FromHelper::CloseDone { request, outcome });
            }
            // The reply waits for the window's next frame, which gives the
            // focus, so a snapshot taken after it names the widget.
            ToHelper::FocusNode { request, reply, window, node } => {
                let Some(w) = self.windows.get_mut(&window) else {
                    self.sink.reply(&reply, FromHelper::FocusDone { request, outcome: Err(format!("no window is open with the number {window}")) });
                    return;
                };
                match w.focus_node(node) {
                    Ok(()) => {
                        w.focus_replies.push((request, reply));
                        ctx.request_repaint_of(w.repaint_target());
                    }
                    Err(why) => self.sink.reply(&reply, FromHelper::FocusDone { request, outcome: Err(why) }),
                }
            }
            ToHelper::Detach { window } => match self.windows.get_mut(&window) {
                Some(w) => {
                    if let Some(ring) = w.ring.take() {
                        self.sink.forget(&ring);
                    }
                }
                None => self.log_error("Detach", format!("no open window {window}")),
            },
            ToHelper::Focus { window } => match self.windows.get_mut(&window) {
                Some(w) if w.shown => ctx.send_viewport_cmd_to(viewport_id(window), egui::ViewportCommand::Focus),
                Some(w) => {
                    w.focus_asked = true;
                    ctx.request_repaint_of(egui::ViewportId::ROOT);
                }
                None => self.log_error("Focus", format!("no open window {window}")),
            },
            ToHelper::ListAdapters { request, reply } => {
                let adapters = self.adapters.clone();
                self.sink.reply(&reply, FromHelper::Adapters { request, adapters });
            }
            ToHelper::ListWindows { request, reply } => {
                let windows = self
                    .windows
                    .values()
                    .filter(|w| w.painted)
                    .map(|w| WindowSummary { window: w.id, title: w.options.title.clone(), names: w.tree.names() })
                    .collect();
                self.sink.reply(&reply, FromHelper::Windows { request, windows });
            }
            // egui draws every surface on its own device, and shows no page.
            ToHelper::PageGraphics { request, reply } => self.sink.reply(&reply, FromHelper::PageGraphics { request, graphics: None }),
            ToHelper::TestEvent { request, reply, window, name, kind, value } => {
                let acted = match self.windows.get_mut(&window) {
                    Some(w) => w.test_event(&name, kind, &value),
                    None => Err(format!("no open window {window}")),
                };
                let outcome = match acted {
                    Ok((raised, v)) => {
                        if let Some((node, kind)) = raised {
                            let ring = self.windows.get(&window).and_then(|w| w.ring.clone());
                            let event = FromHelper::Event { window, node, kind, value: v.clone(), at: proto::clock::now_ns() };
                            self.send_window(ring.as_deref(), event);
                        }
                        Ok(v)
                    }
                    Err(text) => Err(text),
                };
                self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome });
                self.repaint_window(ctx, window);
            }
            ToHelper::TestInput { request, reply, window, steps } => {
                let taken = match self.windows.get_mut(&window) {
                    None => Err(format!("no open window {window}")),
                    Some(w) => w.take_input(request, reply.clone(), steps),
                };
                match taken {
                    Ok(()) => self.repaint_window(ctx, window),
                    Err(why) => self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome: Err(why) }),
                }
            }
            ToHelper::StopInput { window, request } => {
                if let Some(w) = self.windows.get_mut(&window)
                    && let Some(given) = w.input.take_if(|given| given.request == request)
                {
                    w.held = given.holding();
                    proto::trace::line(|| format!("egui: window {window} drops the rest of input {request}, which its session no longer waits for"));
                }
            }
            ToHelper::Snapshot { request, reply, window, name, pixels } => self.snapshot(ctx, request, reply, window, name, pixels),
            ToHelper::ForgetReply { reply } => {
                self.loss_watchers.retain(|(_, r)| *r != reply);
                self.sink.forget(&reply);
            }
            ToHelper::Bridge { request, reply, window, .. } => {
                let text = format!("window {window} is drawn by egui, which shows no page, so a value for a page has nowhere to go");
                self.sink.reply(&reply, FromHelper::BridgeDone { request, outcome: Err(text) });
            }
            ToHelper::BridgeReply { window, .. } => {
                let text = format!("window {window} is drawn by egui, which shows no page, so a reply for a page has nowhere to go");
                let ring = self.windows.get(&window).and_then(|w| w.ring.clone());
                match ring {
                    Some(r) => self.send_window(Some(&r), FromHelper::Error { window: Some(window), context: String::from("BridgeReply"), text }),
                    None => self.log_error("BridgeReply", text),
                }
            }
            ToHelper::CallReply { window, .. } => {
                let text = format!("window {window} is drawn by egui, which shows no page, so the answer to a page's call has nowhere to go");
                let ring = self.windows.get(&window).and_then(|w| w.ring.clone());
                match ring {
                    Some(r) => self.send_window(Some(&r), FromHelper::Error { window: Some(window), context: String::from("CallReply"), text }),
                    None => self.log_error("CallReply", text),
                }
            }
            ToHelper::CloseChannel { window, name } => {
                let text = format!("window {window} is drawn by egui, which shows no page, so it has no channel {name} to close");
                let ring = self.windows.get(&window).and_then(|w| w.ring.clone());
                match ring {
                    Some(r) => self.send_window(Some(&r), FromHelper::Error { window: Some(window), context: String::from("CloseChannel"), text }),
                    None => self.log_error("CloseChannel", text),
                }
            }
            ToHelper::Shutdown => self.shut_down(ctx),
            ToHelper::Dialog { request, reply, title, kind } => {
                let done = Arc::clone(&self.dialogs_done);
                let wake = self.wake.clone();
                let woken = self.wake.clone();
                let stand_in = standin::hook(self.stand_ins.waiting(), move || match woken {
                    Some(w) => w(),
                    None => eprintln!("pwrs-gui-egui: dialog {request} is to be drawn before the loop could be woken"),
                });
                pwrs_gui_dialogs::show(
                    request,
                    title,
                    kind,
                    !self.no_focus,
                    Some(stand_in),
                    Box::new(move |outcome| {
                        done.push(DoneDialog { request, reply, outcome });
                        match wake {
                            Some(w) => w(),
                            None => eprintln!("pwrs-gui-egui: dialog {request} closed before the loop could be woken"),
                        }
                    }),
                );
            }
            ToHelper::CloseDialog { dialog } => {
                if let Err(why) = pwrs_gui_dialogs::close(dialog) {
                    self.log_error("CloseDialog", why);
                }
            }
            ToHelper::ListDialogs { request, reply } => self.sink.reply(&reply, FromHelper::Dialogs { request, dialogs: pwrs_gui_dialogs::list() }),
            ToHelper::AnswerDialog { request, reply, dialog, answer } => {
                let outcome = pwrs_gui_dialogs::answer(dialog, answer).map(|()| Value::Null);
                self.sink.reply(&reply, FromHelper::TestEventDone { request, outcome });
            }
            ToHelper::PickerAnswer { window, node, .. } => {
                let text = format!("file chooser {node} of window {window} was answered from elsewhere, and this renderer draws its own choosers' dialogs");
                self.log_error("PickerAnswer", text);
            }
            ToHelper::Gpu { request, reply, op } => self.gpu_call(ctx, request, reply, op),
            ToHelper::GpuFree { buffer } => {
                if self.restart.is_some() {
                    self.held_gpu.push(HeldGpu::Free(buffer));
                    return;
                }
                match &self.gpu {
                    Some(g) => g.free(buffer),
                    None => self.log_error("GpuFree", format!("GPU buffer {buffer} was to be freed, and this renderer's GPU layer did not start")),
                }
            }
            ToHelper::GpuFreePipeline { pipeline } => {
                if self.restart.is_some() {
                    self.held_gpu.push(HeldGpu::FreePipeline(pipeline));
                    return;
                }
                match &self.gpu {
                    Some(g) => g.free_pipeline(pipeline),
                    None => self.log_error("GpuFreePipeline", format!("render pipeline {pipeline} was to be freed, and this renderer's GPU layer did not start")),
                }
            }
            ToHelper::GpuFreeTexture { texture } => {
                if self.restart.is_some() {
                    self.held_gpu.push(HeldGpu::FreeTexture(texture));
                    return;
                }
                match &self.gpu {
                    Some(g) => g.free_texture(texture),
                    None => self.log_error("GpuFreeTexture", format!("texture {texture} was to be freed, and this renderer's GPU layer did not start")),
                }
            }
            ToHelper::GpuFreeSampler { sampler } => {
                if self.restart.is_some() {
                    self.held_gpu.push(HeldGpu::FreeSampler(sampler));
                    return;
                }
                match &self.gpu {
                    Some(g) => g.free_sampler(sampler),
                    None => self.log_error("GpuFreeSampler", format!("sampler {sampler} was to be freed, and this renderer's GPU layer did not start")),
                }
            }
        }
    }

    /// Passes a GPU call to the GPU thread. A reset destroys the device and,
    /// like a call waiting for a lost device, is replied to once the
    /// renderer is on a new one; while it moves there, every call waits for
    /// it. A surface's draws are planned on the GPU thread and installed
    /// here; a save waits for a frame of the surface's window to paint it. A
    /// canvas's view is the window's own, which egui draws without the GPU
    /// layer, so its calls are replied to here whatever the layer is doing.
    fn gpu_call(&mut self, ctx: &egui::Context, request: RequestId, reply: RingName, op: GpuOp) {
        let op = match op {
            GpuOp::SetCanvasView { window, name, change } => return self.view_call(ctx, request, &reply, window, &name, canvas::ViewCall::Set(change)),
            GpuOp::GetCanvasView { window, name } => return self.view_call(ctx, request, &reply, window, &name, canvas::ViewCall::Get),
            GpuOp::RemoveCanvasView { window, name } => return self.view_call(ctx, request, &reply, window, &name, canvas::ViewCall::Remove),
            op => op,
        };
        if self.restart.is_some() {
            match op {
                GpuOp::WatchLoss | GpuOp::ResetDevice => self.loss_watchers.push((request, reply)),
                op => self.held_gpu.push(HeldGpu::Call { request, reply, op: Box::new(op) }),
            }
            return;
        }
        let Some(g) = &self.gpu else {
            let text = String::from("this renderer's GPU layer did not start; the helper's log says why");
            let outcome = Err(GpuFailure { id: String::from("GuiGpuNoLayer"), text });
            self.sink.reply(&reply, FromHelper::GpuResult { request, outcome });
            return;
        };
        match op {
            GpuOp::WatchLoss => self.loss_watchers.push((request, reply)),
            GpuOp::ResetDevice => {
                g.destroy(request, reply.clone());
                self.loss_watchers.push((request, reply));
            }
            GpuOp::SetSurface { window, name, clear, clear_depth, clear_stencil, draws, passes, uniform } => match self.surface_node(window, &name) {
                Ok(node) => g.plan(request, Some(reply), window, node, gpu_thread::Given { clear, clear_depth, clear_stencil, draws, passes, uniform }),
                Err(failure) => self.sink.reply(&reply, FromHelper::GpuResult { request, outcome: Err(failure) }),
            },
            GpuOp::SaveSurface { window, name } => match self.surface_node(window, &name) {
                Ok(node) => self.ask_save(ctx, request, reply, window, node),
                Err(failure) => self.sink.reply(&reply, FromHelper::GpuResult { request, outcome: Err(failure) }),
            },
            GpuOp::SetCamera { window, name, change } => self.camera_call(ctx, request, &reply, window, &name, CameraCall::Set(change)),
            GpuOp::GetCamera { window, name } => self.camera_call(ctx, request, &reply, window, &name, CameraCall::Get),
            GpuOp::RemoveCamera { window, name } => self.camera_call(ctx, request, &reply, window, &name, CameraCall::Remove),
            op => g.send(request, reply, op),
        }
    }

    /// Sets, reads or takes away the view of the Canvas named `name` in
    /// window `window`, as `call` requests, and replies on `reply`; a set or
    /// a removal draws the window again.
    fn view_call(&mut self, ctx: &egui::Context, request: RequestId, reply: &RingName, window: WindowId, name: &str, call: canvas::ViewCall) {
        let outcome = self.canvas_node(window, name).and_then(|node| {
            let w = self.windows.get_mut(&window).ok_or_else(|| GpuFailure { id: String::from("GuiNoCanvas"), text: format!("no open window {window} is drawn by egui") })?;
            let value = canvas::view_call(&mut w.inputs.canvases, node, call, std::time::Instant::now()).map_err(|text| GpuFailure { id: String::from("GuiCanvasView"), text })?;
            ctx.request_repaint_of(w.repaint_target());
            Ok(value)
        });
        self.sink.reply(reply, FromHelper::GpuResult { request, outcome });
    }

    /// The node of the Canvas named `name` in window `window`, or why there
    /// is none.
    fn canvas_node(&self, window: WindowId, name: &str) -> Result<NodeId, GpuFailure> {
        let failure = |text: String| GpuFailure { id: String::from("GuiNoCanvas"), text };
        let w = self.windows.get(&window).ok_or_else(|| failure(format!("no open window {window} is drawn by egui")))?;
        match w.tree.find_named(name) {
            Some(n) if n.kind == Kind::Canvas => Ok(n.id),
            Some(n) => Err(failure(format!("{name} in window {window} is a {:?}, not a Canvas", n.kind))),
            None => Err(failure(format!("window {window} has no widget named {name}; its widgets are {}", w.tree.names().join(", ")))),
        }
    }

    /// Sets, reads or takes away the camera of the GpuSurface named `name`
    /// in window `window`, as `call` requests, and replies on `reply`; a set
    /// starts from the default camera when the surface has none, and a set
    /// or a removal draws the window again.
    fn camera_call(&mut self, ctx: &egui::Context, request: RequestId, reply: &RingName, window: WindowId, name: &str, call: CameraCall) {
        let outcome = self.surface_node(window, name).and_then(|node| {
            let w = self.windows.get_mut(&window).ok_or_else(|| GpuFailure { id: String::from("GuiGpuNoSurface"), text: format!("no open window {window} is drawn by egui") })?;
            let surface = w.inputs.surfaces.entry(node).or_default();
            let value = match call {
                CameraCall::Set(change) => {
                    // A surface without a camera starts from the default one.
                    let base = surface.camera.clone().unwrap_or_default();
                    if let Some(text) = change.problem(&base) {
                        return Err(GpuFailure { id: String::from("GuiGpuCamera"), text });
                    }
                    let made = change.applied_to(&base);
                    surface.camera = Some(made.clone());
                    GpuValue::Camera(Some(made))
                }
                CameraCall::Get => return Ok(GpuValue::Camera(surface.camera.clone())),
                CameraCall::Remove => {
                    surface.camera = None;
                    surface.held = None;
                    GpuValue::Done
                }
            };
            ctx.request_repaint_of(w.repaint_target());
            Ok(value)
        });
        self.sink.reply(reply, FromHelper::GpuResult { request, outcome });
    }

    /// The node of the GpuSurface named `name` in window `window`, or why
    /// there is none.
    fn surface_node(&self, window: WindowId, name: &str) -> Result<NodeId, GpuFailure> {
        let failure = |text: String| GpuFailure { id: String::from("GuiGpuNoSurface"), text };
        let w = self.windows.get(&window).ok_or_else(|| failure(format!("no open window {window} is drawn by egui")))?;
        match w.tree.find_named(name) {
            Some(n) if n.kind == Kind::GpuSurface => Ok(n.id),
            Some(n) => Err(failure(format!("{name} in window {window} is a {:?}, not a GpuSurface", n.kind))),
            None => Err(failure(format!("window {window} has no widget named {name}; its widgets are {}", w.tree.names().join(", ")))),
        }
    }

    /// Requests a save of GpuSurface `node` of window `window`, read once a
    /// frame of the window drawn after now has painted it
    /// ([`UiState::read_due_saves`]), and requests that frame of the window.
    fn ask_save(&mut self, ctx: &egui::Context, request: RequestId, reply: RingName, window: WindowId, node: NodeId) {
        let Some(w) = self.windows.get_mut(&window) else {
            let failure = GpuFailure { id: String::from("GuiGpuNoSurface"), text: format!("no open window {window} is drawn by egui") };
            self.sink.reply(&reply, FromHelper::GpuResult { request, outcome: Err(failure) });
            return;
        };
        let due = w.inputs.surfaces.entry(node).or_default().frames + 2;
        w.saves.push(PendingSave { request, reply, node, due, asked: std::time::Instant::now() });
        ctx.request_repaint_of(w.repaint_target());
    }

    /// Reads each save of window `window`'s GpuSurfaces that a frame drawn
    /// since it was requested has painted: the GPU thread reads the
    /// surface's texture, and a surface the window no longer has, or one no
    /// frame drew a texture for, is refused. Returns whether any save of the
    /// window still waits for its frame.
    fn read_due_saves(&mut self, window: WindowId) -> bool {
        let Some(w) = self.windows.get_mut(&window) else {
            return false;
        };
        if w.saves.is_empty() {
            return false;
        }
        let surfaces = &w.inputs.surfaces;
        let (due, waiting): (Vec<PendingSave>, Vec<PendingSave>) = std::mem::take(&mut w.saves).into_iter().partition(|s| surfaces.get(&s.node).is_none_or(|surface| surface.frames >= s.due));
        let read: Vec<(PendingSave, Result<wgpu::Texture, GpuFailure>)> = due
            .into_iter()
            .map(|s| {
                let texture = match surfaces.get(&s.node) {
                    None => Err(GpuFailure { id: String::from("GuiGpuNoSurface"), text: format!("GpuSurface {} left window {window} before it was saved", s.node) }),
                    Some(surface) => surface.targets.load_full().map(|t| t.texture.clone()).ok_or_else(|| GpuFailure {
                        id: String::from("GuiGpuNotDrawn"),
                        text: format!("GpuSurface {} of window {window} has no texture: no frame drew it, as a surface given no room in its window is not drawn", s.node),
                    }),
                };
                (s, texture)
            })
            .collect();
        w.saves = waiting;
        let still = !w.saves.is_empty();
        for (save, texture) in read {
            match (texture, &self.gpu) {
                (Ok(texture), Some(g)) => g.read(save.request, save.reply, texture),
                (Ok(_), None) => {
                    let failure = GpuFailure { id: String::from("GuiGpuNoLayer"), text: String::from("this renderer's GPU layer is not running, so the surface's texture cannot be read") };
                    self.sink.reply(&save.reply, FromHelper::GpuResult { request: save.request, outcome: Err(failure) });
                }
                (Err(failure), _) => self.sink.reply(&save.reply, FromHelper::GpuResult { request: save.request, outcome: Err(failure) }),
            }
        }
        still
    }

    /// Refuses each save no frame of its window has read within
    /// [`CAPTURE_DEADLINE`], as a surface on a tab not shown or in a
    /// minimized window is not drawn; while any save waits, a frame of the
    /// root is requested [`CAPTURE_LOOKED_AT_EVERY`] later, whose logic runs
    /// this again.
    fn refuse_late_saves(&mut self, ctx: &egui::Context) {
        let now = std::time::Instant::now();
        let mut late = Vec::new();
        let mut waiting = false;
        for w in self.windows.values_mut() {
            if w.saves.is_empty() {
                continue;
            }
            let (old, kept): (Vec<PendingSave>, Vec<PendingSave>) = std::mem::take(&mut w.saves).into_iter().partition(|s| now.duration_since(s.asked) >= CAPTURE_DEADLINE);
            w.saves = kept;
            waiting |= !w.saves.is_empty();
            late.extend(old.into_iter().map(|s| (w.id, s)));
        }
        for (window, save) in late {
            let text = format!(
                "window {window} drew GpuSurface {} in no frame in the {} s after the save was requested, as a surface on a tab not shown or in a minimized window is not drawn",
                save.node,
                CAPTURE_DEADLINE.as_secs()
            );
            self.sink.reply(&save.reply, FromHelper::GpuResult { request: save.request, outcome: Err(GpuFailure { id: String::from("GuiGpuNotDrawn"), text }) });
        }
        if waiting {
            ctx.request_repaint_after_for(CAPTURE_LOOKED_AT_EVERY, egui::ViewportId::ROOT);
        }
    }

    /// Sends what the GPU thread made: a reply to its reply ring, which also
    /// settles a reset that failed, a line for the log, or an error the
    /// device raised outside any call, to every open window and the log. A
    /// GpuSurface's plan goes into the surface, which draws it from the
    /// window's next frame, and its call is replied to; a plan made again on a
    /// new device is traced, and one that failed goes to the window's
    /// command.
    fn send_answer(&mut self, ctx: &egui::Context, answer: gpu_thread::Answer) {
        match answer {
            gpu_thread::Answer::Reply { reply, msg } => {
                if let FromHelper::GpuResult { request, .. } = &msg {
                    let answered = *request;
                    self.loss_watchers.retain(|(r, _)| *r != answered);
                }
                self.sink.reply(&reply, msg);
            }
            gpu_thread::Answer::Log(msg) => self.sink.log(msg),
            gpu_thread::Answer::Outside(text) => self.gpu_error(text),
            gpu_thread::Answer::Planned { request, reply, window, node, given, plan } => {
                let outcome = match self.windows.get_mut(&window) {
                    Some(w) if w.tree.find(node).is_some_and(|n| n.kind == Kind::GpuSurface) => plan.map(|plan| {
                        let surface = w.inputs.surfaces.entry(node).or_default();
                        surface.plan = Some(Arc::new(plan));
                        surface.given = Some(given);
                        ctx.request_repaint_of(w.repaint_target());
                        GpuValue::Done
                    }),
                    _ => Err(GpuFailure { id: String::from("GuiGpuNoSurface"), text: format!("GpuSurface {node} of window {window} was gone before its draws were planned") }),
                };
                match (reply, outcome) {
                    (Some(r), outcome) => self.sink.reply(&r, FromHelper::GpuResult { request, outcome }),
                    (None, Err(f)) => {
                        let ring = self.windows.get(&window).and_then(|w| w.ring.clone());
                        let text = format!("the draws of GpuSurface {node} could not be planned again on the new device: {} ({})", f.text, f.id);
                        self.send_window(ring.as_deref(), FromHelper::Error { window: Some(window), context: String::from("GPU"), text });
                    }
                    (None, Ok(planned)) => proto::trace::line(move || format!("egui: GpuSurface {node} of window {window} planned again on the new device: {planned:?}")),
                }
            }
        }
    }

    /// Ends the run when the GPU device is lost, so the next starts on a
    /// new device; GPU calls wait for it meanwhile.
    fn lose_device(&mut self, ctx: &egui::Context, reason: String) {
        if self.closing || self.restart.is_some() {
            return;
        }
        proto::trace::line(|| format!("egui: GPU device lost ({reason}); the run ends and another starts on a new device"));
        self.restart = Some(reason);
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
    }

    /// Moves the GPU layer to the device eframe made in place of the lost
    /// one: the GPU thread finishes the work given to it and returns its
    /// layer, what it left to send is sent, the layer makes every buffer,
    /// texture, sampler and render pipeline again on the new device, a new
    /// GPU thread takes it, the errors reported on the lost device are
    /// forgotten, every GpuSurface lets go of what it drew with on the lost
    /// device and has its draws planned again, the calls that waited are
    /// passed on, and every reset and call waiting for a loss is replied to.
    fn move_gpu(&mut self, ctx: &egui::Context, rs: &egui_wgpu::RenderState, reason: &str) {
        let moved = match self.gpu.take() {
            None => Err(String::from("the GPU layer had not started")),
            Some(old) => {
                let (layer, left) = old.end();
                for answer in left {
                    self.send_answer(ctx, answer);
                }
                match (layer, self.wake.clone()) {
                    (Err(why), _) => Err(why),
                    (Ok(_), None) => Err(String::from("the loop cannot be woken")),
                    (Ok(mut layer), Some(wake)) => match layer.rebuild(rs.adapter.clone(), rs.device.clone(), rs.queue.clone()) {
                        Err(f) => Err(f.to_string()),
                        Ok(()) => gpu_thread::GpuThread::start(layer, wake),
                    },
                }
            }
        };
        let held = std::mem::take(&mut self.held_gpu);
        let watchers = std::mem::take(&mut self.loss_watchers);
        let mut replans = Vec::new();
        for (window, w) in &mut self.windows {
            w.inputs.surface_format = self.target_format;
            for (node, surface) in &mut w.inputs.surfaces {
                surface.targets.store(None);
                surface.plan = None;
                if let Some(given) = &surface.given {
                    replans.push((*window, *node, given.clone()));
                }
            }
        }
        match moved {
            Ok(thread) => {
                self.gpu_reported.forget();
                for (window, node, given) in replans {
                    thread.plan(0, None, window, node, given);
                }
                self.gpu = Some(thread);
                for call in held {
                    match call {
                        HeldGpu::Call { request, reply, op } => self.gpu_call(ctx, request, reply, *op),
                        HeldGpu::Free(buffer) => {
                            if let Some(g) = &self.gpu {
                                g.free(buffer);
                            }
                        }
                        HeldGpu::FreePipeline(pipeline) => {
                            if let Some(g) = &self.gpu {
                                g.free_pipeline(pipeline);
                            }
                        }
                        HeldGpu::FreeTexture(texture) => {
                            if let Some(g) = &self.gpu {
                                g.free_texture(texture);
                            }
                        }
                        HeldGpu::FreeSampler(sampler) => {
                            if let Some(g) = &self.gpu {
                                g.free_sampler(sampler);
                            }
                        }
                    }
                }
                self.sink.log(FromHelper::DeviceLost { reason: reason.to_string() });
                for (request, reply) in watchers {
                    self.sink.reply(&reply, FromHelper::GpuResult { request, outcome: Ok(GpuValue::Lost { reason: reason.to_string() }) });
                }
            }
            Err(why) => {
                let text = format!("the GPU device was lost ({reason}), and the GPU layer could not move to the new one, so GPU calls are refused: {why}");
                self.sink.log(FromHelper::Log { level: LogLevel::Warning, text: text.clone() });
                let failure = GpuFailure { id: String::from("GuiGpuNoLayer"), text };
                for call in held {
                    match call {
                        HeldGpu::Call { request, reply, .. } => self.sink.reply(&reply, FromHelper::GpuResult { request, outcome: Err(failure.clone()) }),
                        HeldGpu::Free(buffer) => self.log_error("GpuFree", format!("GPU buffer {buffer} went with the lost device")),
                        HeldGpu::FreePipeline(pipeline) => self.log_error("GpuFreePipeline", format!("render pipeline {pipeline} went with the lost device")),
                        HeldGpu::FreeTexture(texture) => self.log_error("GpuFreeTexture", format!("texture {texture} went with the lost device")),
                        HeldGpu::FreeSampler(sampler) => self.log_error("GpuFreeSampler", format!("sampler {sampler} went with the lost device")),
                    }
                }
                for (request, reply) in watchers {
                    self.sink.reply(&reply, FromHelper::GpuResult { request, outcome: Err(failure.clone()) });
                }
            }
        }
    }

    /// Readies every window to open again in the next run, where it was and
    /// at its size, without taking the focus; a capture still waiting for
    /// one is refused, since the window it was of is gone.
    fn reopen_windows(&mut self) {
        let mut refused = Vec::new();
        for w in self.windows.values_mut() {
            w.shown = false;
            w.takes_focus = false;
            w.focus_asked = false;
            if let Some(at) = w.last_outer {
                w.position = Some(at);
            }
            if let Some(size) = w.last_inner {
                w.size = Some(size);
            }
            #[cfg(windows)]
            {
                w.native = None;
            }
            refused.extend(std::mem::take(&mut w.screenshots).into_iter().map(|shot| (w.id, shot)));
        }
        for (window, shot) in refused {
            let why = format!("window {window} was opened again on a new GPU device before its pixels were captured");
            self.sink.reply(&shot.reply, FromHelper::Snapshot { request: shot.request, outcome: Err(why) });
        }
    }

    /// Replies with a snapshot of window `window`, or of its widget named
    /// `name`: without `pixels` from its last frame, or at the end of its
    /// first where it has drawn none yet; with `pixels` from the frame its
    /// capture is taken in.
    fn snapshot(&mut self, ctx: &egui::Context, request: RequestId, reply: RingName, window: WindowId, name: Option<String>, pixels: bool) {
        let Some(w) = self.windows.get_mut(&window) else {
            self.sink.reply(&reply, FromHelper::Snapshot { request, outcome: Err(format!("no open window {window}")) });
            return;
        };
        let node = match &name {
            None => None,
            Some(n) => match w.tree.find_named(n) {
                Some(found) => Some(found.id),
                None => {
                    let why = format!("window {window} has no widget named {n}; its widgets are {}", w.tree.names().join(", "));
                    self.sink.reply(&reply, FromHelper::Snapshot { request, outcome: Err(why) });
                    return;
                }
            },
        };
        if !pixels {
            if !w.painted {
                w.early_shots.push((request, reply));
                ctx.request_repaint_of(w.repaint_target());
                return;
            }
            let outcome = Ok(w.snapshot(None));
            self.sink.reply(&reply, FromHelper::Snapshot { request, outcome });
            return;
        }
        let drawn = ctx.input(|i| i.raw.viewports.get(&viewport_id(window)).and_then(|v| v.visible()));
        if drawn == Some(false) {
            let why = format!("window {window} is minimized or covered, so it is not drawn and has no pixels to capture");
            self.sink.reply(&reply, FromHelper::Snapshot { request, outcome: Err(why) });
            return;
        }
        proto::trace::line(|| format!("egui: capture {request} of window {window} requested, of node {node:?}"));
        w.screenshots.push(PendingShot { request, reply, node, sent: None, asked: std::time::Instant::now() });
        ctx.request_repaint_of(w.repaint_target());
    }

    /// Sizes and places each window not yet shown: inside, the size its
    /// options give it, and a side they leave to it fitted to its content
    /// ([`proto::WindowOptions::fitted`]) as [`Measure`] lays it out, at most
    /// the work area of the screen it opens on, or
    /// [`proto::ASSUMED_WORK_AREA`] where that cannot be read, which the log
    /// says; and centered on the monitor `PWRS_GUI_MONITOR` names, when it
    /// names one.
    fn size_new_windows(&mut self, ctx: &egui::Context) {
        if self.windows.values().all(|w| w.shown || w.size.is_some()) {
            return;
        }
        let work = match work_area(ctx, self.monitor) {
            Ok(work) => work,
            Err(why) => {
                let [across, down] = proto::ASSUMED_WORK_AREA;
                let text = format!("new windows fit {across} by {down} points: the screen they open on cannot be read ({why})");
                self.sink.log(FromHelper::Log { level: LogLevel::Warning, text });
                proto::ASSUMED_WORK_AREA
            }
        };
        let ppp = ctx.pixels_per_point();
        let UiState { windows, measure, sink, monitor, .. } = self;
        for (id, w) in windows.iter_mut().filter(|(_, w)| !w.shown && w.size.is_none()) {
            let content = if w.options.fits() {
                let fonts = ctx.fonts(|f| f.definitions().clone());
                measure.get_or_insert_with(Measure::new).content(w, fonts, ppp, work)
            } else {
                [0.0; 2]
            };
            let [width, height] = w.options.fitted(content, Some(work));
            w.size = Some([width, height]);
            let Some(n) = *monitor else {
                continue;
            };
            match position_on(n, width, height) {
                Ok(Some(at)) => w.position = Some(at),
                Ok(None) => {
                    let text = format!("window {id} opens where the system places it: {} names monitor {n}, which is not connected now", place::VAR);
                    sink.log(FromHelper::Log { level: LogLevel::Warning, text });
                }
                Err(why) => {
                    let text = format!("window {id} opens where the system places it: the monitors cannot be listed ({why})");
                    sink.log(FromHelper::Log { level: LogLevel::Warning, text });
                }
            }
        }
    }

    /// Draws one window's frame and sends what its widgets raised, and
    /// replies with any capture read back since, whichever window it is of.
    /// A capture is requested from inside the window's own frame, once it
    /// has drawn, so it is of that window and no other, and requested again
    /// when [`CAPTURE_ASKED_AGAIN_AFTER`] of the window's frames bring none.
    /// A window with a capture in flight draws again until the capture
    /// arrives: wgpu reads a capture back only while a later frame is
    /// submitted. On Windows a window is brought to the front at its first
    /// paint when the settings request it.
    fn frame(&mut self, window: WindowId, ui: &mut egui::Ui) {
        self.pump(ui.ctx());
        for uri in self.forgotten.drain(..) {
            ui.ctx().forget_image(&uri);
        }
        self.deliver_shots();
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        // egui forgets a frame requested for later before its frame begins,
        // so the end of a test's wait is requested from inside the frame.
        if let Some(left) = w.input.as_ref().and_then(|given| given.wait_left(std::time::Instant::now())) {
            ui.ctx().request_repaint_after(left);
        }
        let out = w.frame(ui);
        // The text a widget of the window copied in this frame, as a field's
        // Ctrl+C or a table's rows, is the window's last copy.
        let copied = ui.ctx().output(|o| o.commands.iter().rev().find_map(|c| if let egui::OutputCommand::CopyText(text) = c { Some(text.clone()) } else { None }));
        if copied.is_some() {
            w.copied = copied;
        }
        let early: Vec<(RingName, FromHelper)> = if w.painted && !ui.ctx().will_discard() {
            let asked = std::mem::take(&mut w.early_shots);
            asked.into_iter().map(|(request, reply)| (reply, FromHelper::Snapshot { request, outcome: Ok(w.snapshot(None)) })).collect()
        } else {
            Vec::new()
        };
        if let Some(f) = self.fallbacks.as_mut() {
            for m in &out.events {
                if let FromHelper::Event { kind: EventKind::Change | EventKind::Submit, value: Value::Str(typed), .. } = m {
                    f.cover(ui.ctx(), typed);
                }
            }
        }
        let now = ui.ctx().cumulative_frame_nr();
        for shot in &mut w.screenshots {
            let Some(again) = ask_for_capture(shot.sent, now) else {
                continue;
            };
            let (arrived, ctx, request, of) = (Arc::clone(&self.shots_arrived), ui.ctx().clone(), shot.request, ui.ctx().viewport_id());
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
                arrived.push((request, of, image));
                ctx.request_repaint_of(of);
            })));
            shot.sent = Some(now);
            if again {
                proto::trace::line(|| format!("egui: capture {} of window {window} requested again with frame {now}: none arrived in {CAPTURE_ASKED_AGAIN_AFTER} frames", shot.request));
            } else {
                proto::trace::line(|| format!("egui: capture {} of window {window} sent with frame {now}", shot.request));
            }
        }
        let ring = w.ring.clone();
        let capturing = !w.screenshots.is_empty();
        #[cfg(windows)]
        let opened = out.events.iter().any(|m| matches!(m, FromHelper::Opened { .. }));
        if out.closed {
            self.stop_reading(window);
        }
        for m in out.events {
            self.send_window(ring.as_deref(), m);
        }
        for (reply, m) in out.replies.into_iter().chain(early) {
            self.sink.reply(&reply, m);
        }
        let saving = self.read_due_saves(window);
        if out.closed {
            ui.ctx().request_repaint_of(egui::ViewportId::ROOT);
        } else if capturing || saving {
            ui.ctx().request_repaint_of(viewport_id(window));
        }
        #[cfg(windows)]
        if opened && self.bring_to_front && !self.no_focus {
            self.raise(window);
        }
    }

    /// Draws one frame of dialog `request`, which the renderer draws in the
    /// system's place, after taking the session's messages, as a window's
    /// frame does.
    fn stand_in_frame(&mut self, request: RequestId, ui: &mut egui::Ui) {
        self.pump(ui.ctx());
        self.stand_ins.frame(request, ui);
    }

    /// Brings window `window` to the front: its operating-system window is
    /// the one this thread owns with its title, among those no other open
    /// window has claimed and the thread did not own before any window
    /// opened. Windows grants the front only to a process that may take it,
    /// as pwsh may while a person's command runs in its console; otherwise
    /// the window stays where it opened.
    #[cfg(windows)]
    fn raise(&mut self, window: WindowId) {
        let skip: Vec<usize> = self.foreign.iter().copied().chain(self.windows.values().filter_map(|w| w.native)).collect();
        let Some(w) = self.windows.get_mut(&window) else {
            return;
        };
        let Some(native) = thread_windows().into_iter().find(|h| !skip.contains(h) && window_title(*h) == w.options.title) else {
            proto::trace::line(|| format!("egui: window {window} has no operating-system window titled {:?} to bring to the front", w.options.title));
            return;
        };
        w.native = Some(native);
        let granted = raise_window(native);
        proto::trace::line(|| format!("egui: window {window} brought to the front: {granted}"));
    }

    /// Replies with each capture wgpu has read back to the request for it. A
    /// capture's callback runs on whichever thread wgpu reads it back on,
    /// queues it and requests a frame of its viewport, so a capture is
    /// matched by its request number, not by the frame that takes it, and
    /// checked against the viewport it was requested from: one of another
    /// viewport is requested again. A capture requested again can arrive
    /// twice.
    fn deliver_shots(&mut self) {
        while let Some((request, of, image)) = self.shots_arrived.pop() {
            proto::trace::line(|| format!("egui: capture {request} arrived, of viewport {of:?}"));
            let found = self.windows.values_mut().find_map(|w| {
                let at = w.screenshots.iter().position(|s| s.request == request)?;
                Some((w, at))
            });
            let Some((w, at)) = found else {
                let text = format!("a screenshot for request {request} arrived after its request was replied to");
                self.sink.log(FromHelper::Log { level: LogLevel::Verbose, text });
                continue;
            };
            if of != viewport_id(w.id) {
                w.screenshots[at].sent = None;
                let text = format!("a screenshot for window {} was of another viewport, so it is requested again", w.id);
                self.sink.log(FromHelper::Log { level: LogLevel::Verbose, text });
                continue;
            }
            let shot = w.screenshots.remove(at);
            let crop = match shot.node {
                None => None,
                Some(n) => match w.inputs.rects.get(&n) {
                    Some(r) => Some(*r),
                    None => {
                        let why = format!("widget {n} is not drawn in this window");
                        self.sink.reply(&shot.reply, FromHelper::Snapshot { request, outcome: Err(why) });
                        continue;
                    }
                },
            };
            let outcome = pixels_of(&image, crop, w.ppp).map(|p| w.snapshot(Some(p)));
            proto::trace::line(|| format!("egui: capture {request} replied to: {}", if outcome.is_ok() { "pixels" } else { "refused" }));
            self.sink.reply(&shot.reply, FromHelper::Snapshot { request, outcome });
        }
    }

    /// Refuses each capture in flight for a window that is now minimized or
    /// covered: egui stops drawing such a window, so no later frame of it
    /// would read the capture back; and each that no frame has taken within
    /// [`CAPTURE_DEADLINE`]. While a capture waits, each frame traces its
    /// window's visibility, and a frame of the root is requested
    /// [`CAPTURE_LOOKED_AT_EVERY`] later, whose logic runs this again;
    /// requesting a frame of the root every frame would take the thread from
    /// the windows whose frames read the captures back.
    fn refuse_hidden_shots(&mut self, ctx: &egui::Context) {
        let mut waiting = false;
        let now = std::time::Instant::now();
        for w in self.windows.values_mut() {
            if w.screenshots.is_empty() {
                continue;
            }
            let (late, kept) = late_shots(std::mem::take(&mut w.screenshots), now);
            w.screenshots = kept;
            for shot in late {
                proto::trace::line(|| format!("egui: capture {} of window {} refused: no frame took it in {} s", shot.request, w.id, CAPTURE_DEADLINE.as_secs()));
                let why = format!("window {} drew no frame in the {} s after the capture was requested, so it has no pixels to capture", w.id, CAPTURE_DEADLINE.as_secs());
                self.sink.reply(&shot.reply, FromHelper::Snapshot { request: shot.request, outcome: Err(why) });
            }
            if w.screenshots.is_empty() {
                continue;
            }
            let visible = ctx.input(|i| i.raw.viewports.get(&viewport_id(w.id)).and_then(|v| v.visible()));
            proto::trace::line(|| format!("egui: {} capture(s) of window {} wait at frame {}, the window visible: {visible:?}", w.screenshots.len(), w.id, ctx.cumulative_frame_nr()));
            if visible != Some(false) {
                waiting = true;
                continue;
            }
            for shot in w.screenshots.drain(..) {
                let why = format!("window {} is minimized or covered, so it is not drawn and has no pixels to capture", w.id);
                self.sink.reply(&shot.reply, FromHelper::Snapshot { request: shot.request, outcome: Err(why) });
            }
        }
        if waiting {
            ctx.request_repaint_after_for(CAPTURE_LOOKED_AT_EVERY, egui::ViewportId::ROOT);
        }
    }

    /// Drops the windows a person closed.
    fn prune_closed(&mut self) {
        let closed: Vec<WindowId> = self.windows.iter().filter(|(_, w)| !w.open).map(|(id, _)| *id).collect();
        for id in closed {
            if let Some(w) = self.windows.remove(&id) {
                self.retire(w);
            }
        }
    }

    /// Stops reading the rings and shared memory window `window`'s widgets
    /// read, once each reader has let go of them; called before the window's
    /// Closed goes out, so a script that removes a ring as the window closes
    /// can delete its files.
    fn stop_reading(&mut self, window: WindowId) {
        if let Some(s) = self.sources.as_mut() {
            s.forget(window);
        }
    }

    /// Refuses the captures, saves and focus requests still waiting for a
    /// window that is gone, forgets its ring, stops reading the rings its
    /// widgets read, and has egui forget its pictures.
    fn retire(&mut self, w: WindowState) {
        let window = w.id;
        #[cfg(windows)]
        if let Some(behind) = w.behind {
            hand_back(behind);
        }
        self.stop_reading(window);
        for shot in w.screenshots {
            let why = format!("window {window} closed before its pixels were captured");
            self.sink.reply(&shot.reply, FromHelper::Snapshot { request: shot.request, outcome: Err(why) });
        }
        for (request, reply) in w.early_shots {
            let why = format!("window {window} closed before it was drawn");
            self.sink.reply(&reply, FromHelper::Snapshot { request, outcome: Err(why) });
        }
        for save in w.saves {
            let failure = GpuFailure { id: String::from("GuiGpuNoSurface"), text: format!("window {window} closed before its GpuSurface {} was saved", save.node) };
            self.sink.reply(&save.reply, FromHelper::GpuResult { request: save.request, outcome: Err(failure) });
        }
        for (request, reply) in w.focus_replies {
            let why = format!("window {window} closed before the widget took the focus");
            self.sink.reply(&reply, FromHelper::FocusDone { request, outcome: Err(why) });
        }
        if let Some(given) = w.input {
            let outcome = if given.unfinished() { Err(format!("window {window} closed before it was given every step of its input")) } else { Ok(Value::Null) };
            self.sink.reply(&given.reply, FromHelper::TestEventDone { request: given.request, outcome });
        }
        if let Some(ring) = &w.ring {
            self.sink.forget(ring);
        }
        self.forgotten.extend(w.inputs.pictures.into_values());
        self.forgotten.extend(w.inputs.documents.values().flat_map(Document::uris));
        self.forgotten.extend(w.inputs.forgotten);
    }

    /// Sends to a window's ring; a detached window has none, and its events
    /// go nowhere.
    fn send_window(&mut self, ring: Option<&str>, msg: FromHelper) {
        if let Some(r) = ring {
            self.sink.window(r, msg);
        }
    }

    fn log_error(&mut self, context: &str, text: String) {
        self.sink.log(FromHelper::Error { window: None, context: context.to_string(), text });
    }

    fn shut_down(&mut self, ctx: &egui::Context) {
        if self.closing {
            return;
        }
        self.closing = true;
        self.restart = None;
        let ended = GpuFailure { id: String::from("GuiGpuEnded"), text: String::from("the renderer ended") };
        for (request, reply) in std::mem::take(&mut self.loss_watchers) {
            self.sink.reply(&reply, FromHelper::GpuResult { request, outcome: Err(ended.clone()) });
        }
        for call in std::mem::take(&mut self.held_gpu) {
            match call {
                HeldGpu::Call { request, reply, .. } => self.sink.reply(&reply, FromHelper::GpuResult { request, outcome: Err(ended.clone()) }),
                HeldGpu::Free(buffer) => self.sink.log(FromHelper::Log { level: LogLevel::Verbose, text: format!("GPU buffer {buffer} goes with the renderer as it ends") }),
                HeldGpu::FreePipeline(pipeline) => self.sink.log(FromHelper::Log { level: LogLevel::Verbose, text: format!("render pipeline {pipeline} goes with the renderer as it ends") }),
                HeldGpu::FreeTexture(texture) => self.sink.log(FromHelper::Log { level: LogLevel::Verbose, text: format!("texture {texture} goes with the renderer as it ends") }),
                HeldGpu::FreeSampler(sampler) => self.sink.log(FromHelper::Log { level: LogLevel::Verbose, text: format!("sampler {sampler} goes with the renderer as it ends") }),
            }
        }
        let windows = std::mem::take(&mut self.windows);
        for (id, w) in windows {
            self.stop_reading(id);
            self.send_window(w.ring.as_deref(), FromHelper::Closed { window: id, reason: CloseReason::Shutdown });
            self.retire(w);
        }
        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
    }
}

/// The ring a message names that an attachment belongs to.
fn attached_ring(msg: &ToHelper) -> Option<&str> {
    match msg {
        ToHelper::OpenWindow { ring, .. } => Some(ring),
        ToHelper::ListAdapters { reply, .. }
        | ToHelper::ListWindows { reply, .. }
        | ToHelper::PageGraphics { reply, .. }
        | ToHelper::TestEvent { reply, .. }
        | ToHelper::TestInput { reply, .. }
        | ToHelper::Snapshot { reply, .. }
        | ToHelper::Bridge { reply, .. }
        | ToHelper::CloseOpen { reply, .. }
        | ToHelper::FocusNode { reply, .. }
        | ToHelper::Dialog { reply, .. }
        | ToHelper::ListDialogs { reply, .. }
        | ToHelper::AnswerDialog { reply, .. }
        | ToHelper::Gpu { reply, .. } => Some(reply),
        ToHelper::Patch { .. }
        | ToHelper::Rows { .. }
        | ToHelper::CloseWindow { .. }
        | ToHelper::Detach { .. }
        | ToHelper::Focus { .. }
        | ToHelper::ForgetReply { .. }
        | ToHelper::StopInput { .. }
        | ToHelper::BridgeReply { .. }
        | ToHelper::Shutdown
        | ToHelper::CloseDialog { .. }
        | ToHelper::PickerAnswer { .. }
        | ToHelper::GpuFree { .. }
        | ToHelper::GpuFreePipeline { .. }
        | ToHelper::GpuFreeTexture { .. }
        | ToHelper::GpuFreeSampler { .. }
        | ToHelper::CallReply { .. }
        | ToHelper::CloseChannel { .. } => None,
    }
}

/// The input state of a window's widgets, stored apart from the tree so a
/// frame can read the tree while it changes the inputs.
#[derive(Default)]
struct Inputs {
    lists: HashMap<NodeId, BTreeSet<usize>>,
    /// Where each list's keyboard is.
    list_keys: HashMap<NodeId, ListKeys>,
    tables: HashMap<NodeId, TableState>,
    /// Each tree's selection and keyboard.
    trees: HashMap<NodeId, TreeState>,
    /// The tree items a person has open, in every tree of the window.
    open_items: HashSet<NodeId>,
    /// The tree items whose Expand was raised, waiting on the session for
    /// their children.
    expanding: HashSet<NodeId>,
    /// Each check box's state.
    checks: HashMap<NodeId, bool>,
    /// Each text field's and editable drop-down's text.
    texts: HashMap<NodeId, String>,
    /// Whether each password field offers its peek button.
    peeks: HashMap<NodeId, proto::peek::Peek>,
    /// The password fields whose peek button a test holds.
    peek_held: HashSet<NodeId>,
    /// Where each password field's peek button was drawn last.
    peek_rects: HashMap<NodeId, egui::Rect>,
    /// Each color chooser's color.
    colors: HashMap<NodeId, [u8; 3]>,
    /// Each number field's and slider's number.
    numbers: HashMap<NodeId, f64>,
    /// Each date chooser's minute and the month its calendar shows.
    dates: HashMap<NodeId, choosers::DateState>,
    /// Each file chooser's paths and its open dialog.
    files: HashMap<NodeId, choosers::FileState>,
    /// The dialogs the window's file choosers open leave the focus alone, as
    /// its window does under `PWRS_GUI_NO_FOCUS` 1.
    no_focus: bool,
    /// Where a file chooser's dialog the system has none of waits for the
    /// renderer to draw it.
    stand_ins: Option<standin::Waiting>,
    /// Each drop-down's, option group's and set of tabs' chosen row.
    choices: HashMap<NodeId, Option<usize>>,
    /// Each split's share of its space given to its first child, from 0 to
    /// 1.
    splits: HashMap<NodeId, f32>,
    /// Nodes whose property errors have been reported, so each is reported
    /// once rather than once per frame.
    reported: HashSet<NodeId>,
    /// Where each node was drawn in the last frame, in points; a node that
    /// frame did not draw, such as one on a tab not shown, has none.
    rects: HashMap<NodeId, egui::Rect>,
    /// Where each node was drawn in the frame before, while this one draws:
    /// what a Column shares its height by and a Row places its widgets by.
    /// After the window's first measure, its sizes there.
    last_rects: HashMap<NodeId, egui::Rect>,
    /// The address each Image's picture is loaded under, which a new
    /// picture for the Image changes.
    pictures: HashMap<NodeId, String>,
    /// Each Markdown widget's pictures and what its document stores.
    documents: HashMap<NodeId, Document>,
    /// The addresses of pictures no Image or Markdown widget shows any
    /// more, which egui forgets at the window's next frame.
    forgotten: Vec<String>,
    /// Each GpuSurface's draws and texture.
    surfaces: HashMap<NodeId, surface::Surface>,
    /// Each Canvas's items, the frame it last drew and its textures.
    canvases: HashMap<NodeId, canvas::Canvas>,
    /// The format egui draws the window in, which a GpuSurface's and a
    /// Canvas's texture is shown into; none until eframe has made the device.
    surface_format: Option<wgpu::TextureFormat>,
    /// The node a Set-GuiFocus gave the focus, which takes it as the
    /// window's next frame draws it.
    focus: Option<NodeId>,
    /// Whether a grid drawn as a sizing pass in this frame passed over the
    /// focus a Set-GuiFocus gave, which then waits for the next frame.
    focus_waits: bool,
    /// The node that had the keyboard focus when the last frame ended.
    focused: Option<NodeId>,
}

impl Inputs {
    /// Whether node `id` is to take the focus in this frame: true once, for
    /// the node a Set-GuiFocus named.
    fn take_focus(&mut self, id: NodeId) -> bool {
        if self.focus == Some(id) {
            self.focus = None;
            return true;
        }
        false
    }
}

/// What a Markdown widget stores: the picture each image of its document
/// shows, by the image's address in the document; what the document stores
/// from frame to frame; and the images whose picture could not be drawn
/// that have been reported, each once until the document changes.
#[derive(Default)]
struct Document {
    pictures: HashMap<String, markdown::Picture>,
    kept: markdown::Kept,
    failed: HashSet<String>,
}

impl Document {
    /// The addresses its pictures are loaded under in egui's caches.
    fn uris(&self) -> impl Iterator<Item = String> + '_ {
        self.pictures.values().filter_map(markdown::Picture::uri).map(str::to_string)
    }
}

/// The number the next picture's address takes, across every window, so
/// that no two pictures share an address in egui's caches.
static PICTURES: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct TableState {
    columns: Vec<TableColumn>,
    rows: Vec<proto::Row>,
    view: Vec<usize>,
    dirty: bool,
    filter: String,
    sort: Option<(usize, bool)>,
    selected: BTreeSet<usize>,
    /// The row its Shift ranges run from.
    anchor: Option<usize>,
    /// The row its keyboard is on.
    current: Option<usize>,
    /// The rows a page key moves by, as many as its last frame showed.
    page: usize,
    /// Whether Add-GuiItem added rows since its last frame, whether that
    /// frame showed the last of the rows shown, and the height they
    /// scrolled in, so the last row stays in sight when rows are added
    /// while it did.
    added: bool,
    at_end: bool,
    room: f32,
}

/// A tree's selection, the item its Shift ranges run from, the item its
/// keyboard is on, the rows a page key moves by, and whether its next
/// frame scrolls the keyboard's row into sight.
#[derive(Default)]
struct TreeState {
    selected: BTreeSet<NodeId>,
    anchor: Option<NodeId>,
    focus: Option<NodeId>,
    page: usize,
    scroll: bool,
}

/// Where a list's keyboard is: the row it is on, the row its Shift ranges
/// run from, and the rows a page key moves by.
#[derive(Clone, Copy, Default)]
struct ListKeys {
    current: Option<usize>,
    anchor: Option<usize>,
    page: usize,
}

/// The keys of this frame a list, table or tree uses ([`rows::RowKey`]),
/// in order, each with Shift and Ctrl as pressed; a move repeats while its
/// key is down, and Space, Enter or Ctrl+A acts once however long it is
/// down.
fn row_keys(ui: &egui::Ui) -> Vec<(rows::RowKey, bool, bool)> {
    ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key { key, pressed: true, repeat, modifiers, .. } => {
                    let k = rows::RowKey::named(routed::shortcut_name(*key), modifiers.command, modifiers.shift, modifiers.alt)?;
                    let once = matches!(k, rows::RowKey::Space | rows::RowKey::Enter | rows::RowKey::All);
                    (!once || !*repeat).then_some((k, modifiers.shift, modifiers.command))
                }
                _ => None,
            })
            .collect()
    })
}

/// The box of the widget with the keyboard focus, as this pass has drawn it
/// or else the last pass did. It is read from egui's widget boxes, not with
/// `Context::read_response`, which judges a click elsewhere as the widget's
/// own response does. In a frame where a click went to another widget,
/// `read_response` takes the focus from a widget that requested it, as a
/// table does when a click on one of its rows asks the focus for it.
fn focused_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    let id = ctx.memory(|m| m.focused())?;
    ctx.viewport(|vp| vp.this_pass.widgets.get(id).or_else(|| vp.prev_pass.widgets.get(id)).map(|w| w.rect))
}

/// Stops egui moving its focus away from a focused list, table or tree at
/// an arrow pressed this frame. egui takes a widget's key filter, which
/// leaves the arrows to it, only from the frame after it gains the focus,
/// so without this an arrow pressed in that first frame would move the
/// rows and also move the focus to the widget beside the list. A Tab in
/// the same frame still moves the focus.
fn stay_focused_on_arrows(ui: &egui::Ui) {
    let arrows = ui.input(|i| {
        let arrow = |e: &egui::Event| matches!(e, egui::Event::Key { key: egui::Key::ArrowUp | egui::Key::ArrowDown | egui::Key::ArrowLeft | egui::Key::ArrowRight, pressed: true, modifiers, .. } if !modifiers.alt);
        i.events.iter().any(arrow) && !i.key_pressed(egui::Key::Tab)
    });
    if arrows {
        ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
    }
}

/// Draws a row of a list of check boxes as egui draws a check box with its
/// text, `on` checked, the text wrapped as `wrap` says, sensing clicks
/// without a Tab stop of its own, since its list takes the focus as one
/// widget; across, it takes the room a justified layout gives it, or its
/// box and text.
fn check_row(ui: &mut egui::Ui, on: bool, text: &str, wrap: egui::TextWrapMode) -> egui::Response {
    let spacing = ui.spacing();
    let (side, check, gap, least) = (spacing.icon_width, spacing.icon_width_inner, spacing.icon_spacing, spacing.interact_size.y);
    let galley = egui::WidgetText::from(text).into_galley(ui, Some(wrap), ui.available_width() - side - gap, egui::TextStyle::Body);
    let natural = side + gap + galley.size().x;
    let across = if ui.layout().horizontal_justify() && !ui.is_sizing_pass() { ui.available_width().max(natural) } else { natural };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(across, galley.size().y.max(least)), egui::Sense::CLICK);
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let boxed = egui::Rect::from_center_size(egui::pos2(rect.left() + side * 0.5, rect.center().y), egui::Vec2::splat(side));
        let mark = egui::Rect::from_center_size(boxed.center(), egui::Vec2::splat(check));
        // The box's corners round no more than egui rounds a check box's.
        let corner = visuals.corner_radius.at_most((0.3 * side).round() as u8);
        ui.painter().add(egui::epaint::RectShape::new(boxed, corner, visuals.bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside));
        if on {
            let points = vec![egui::pos2(mark.left(), mark.center().y), egui::pos2(mark.center().x, mark.bottom()), egui::pos2(mark.right(), mark.top())];
            ui.painter().add(egui::Shape::line(points, visuals.fg_stroke));
        }
        let at = egui::pos2(boxed.right() + gap, rect.center().y - galley.size().y / 2.0);
        ui.painter().galley(at, galley, visuals.text_color());
    }
    response
}

/// Whether the window has the system's focus, without which egui draws no
/// widget as focused (`Response::has_focus`); a list's, table's or tree's
/// ring shows only while it does, as a field's does.
fn window_focused(ui: &egui::Ui) -> bool {
    ui.input(|i| i.focused)
}

/// Paints the ring that shows the row a focused list, table or tree has
/// its keyboard on, about `rect` in `stroke`, the theme's selection stroke,
/// cut to `shown`, the part in sight.
fn row_ring(ctx: &egui::Context, layer: egui::LayerId, rect: egui::Rect, shown: egui::Rect, stroke: egui::Stroke) {
    egui::Painter::new(ctx.clone(), layer, shown).rect_stroke(rect, 0.0, stroke, egui::StrokeKind::Inside);
}

impl TableState {
    fn refresh(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let needle = self.filter.to_lowercase();
        self.view = (0..self.rows.len())
            .filter(|&r| needle.is_empty() || self.rows[r].cells.iter().any(|c| c.text.to_lowercase().contains(&needle)))
            .collect();
        if let Some((col, ascending)) = self.sort {
            let rows = &self.rows;
            self.view.sort_by(|&a, &b| {
                let ord = compare_keys(rows[a].cells.get(col).map(|c| &c.sort), rows[b].cells.get(col).map(|c| &c.sort));
                if ascending { ord } else { ord.reverse() }
            });
        }
    }
}

/// Amounts and dates by their values, before text; text without regard to
/// case; empty cells last.
fn compare_keys(a: Option<&SortKey>, b: Option<&SortKey>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Some(SortKey::Number(x) | SortKey::Date(x)), Some(SortKey::Number(y) | SortKey::Date(y))) => x.total_cmp(y),
        (Some(SortKey::Text(x)), Some(SortKey::Text(y))) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Some(SortKey::Number(_) | SortKey::Date(_)), Some(SortKey::Text(_))) => Ordering::Less,
        (Some(SortKey::Text(_)), Some(SortKey::Number(_) | SortKey::Date(_))) => Ordering::Greater,
        (Some(SortKey::Empty) | None, Some(SortKey::Empty) | None) => Ordering::Equal,
        (Some(SortKey::Empty) | None, _) => Ordering::Greater,
        (_, Some(SortKey::Empty) | None) => Ordering::Less,
    }
}

/// A screenshot requested and not yet replied to.
struct PendingShot {
    request: RequestId,
    reply: RingName,
    /// The node to crop to; the whole window when `None`.
    node: Option<NodeId>,
    /// The window's frame number when its frame requested the capture from
    /// eframe; `None` until it has.
    sent: Option<u64>,
    /// When the session requested it, which [`CAPTURE_DEADLINE`] counts from.
    asked: std::time::Instant,
}

/// A GpuSurface's save requested and not yet read: the surface's node, the
/// count of the surface's frames at which a frame drawn since the request
/// has painted it, and when it was requested, which [`CAPTURE_DEADLINE`]
/// counts from.
struct PendingSave {
    request: RequestId,
    reply: RingName,
    node: NodeId,
    due: u64,
    asked: std::time::Instant,
}

/// How many of a window's frames a capture it requested from eframe may
/// take before its frame requests it again. A capture arrives within three
/// frames, read back once a later frame is submitted, but eframe takes a
/// frame's capture requests before painting, and a frame that does not
/// paint, as one whose surface no configure has set up yet, drops them. A
/// capture requested twice that arrives twice is replied with once.
const CAPTURE_ASKED_AGAIN_AFTER: u64 = 30;

/// How long a capture waits for a frame of its window to take it before it
/// is refused.
const CAPTURE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// How often the root's frame looks at the captures waiting, while any wait.
const CAPTURE_LOOKED_AT_EVERY: std::time::Duration = std::time::Duration::from_millis(100);

/// Splits `shots` into those requested [`CAPTURE_DEADLINE`] or more before
/// `now`, and the rest.
fn late_shots(shots: Vec<PendingShot>, now: std::time::Instant) -> (Vec<PendingShot>, Vec<PendingShot>) {
    shots.into_iter().partition(|s| now.duration_since(s.asked) >= CAPTURE_DEADLINE)
}

/// Whether a window's frame `now` requests from eframe a capture its frame
/// `sent` requested: `Some(false)` the first time, `Some(true)` again once
/// [`CAPTURE_ASKED_AGAIN_AFTER`] frames have brought none, `None` to wait.
fn ask_for_capture(sent: Option<u64>, now: u64) -> Option<bool> {
    match sent {
        None => Some(false),
        Some(at) if now.saturating_sub(at) >= CAPTURE_ASKED_AGAIN_AFTER => Some(true),
        Some(_) => None,
    }
}

/// What one frame of a window produced: its events, whether it closed, and
/// replies to requests the frame settled, each with its reply ring.
struct FrameOut {
    events: Vec<FromHelper>,
    closed: bool,
    replies: Vec<(RingName, FromHelper)>,
}

/// A capture wgpu has read back: the request for it, the viewport it was
/// requested from, and its image.
type ArrivedShot = (RequestId, egui::ViewportId, Arc<egui::ColorImage>);

struct WindowState {
    id: WindowId,
    /// The ring its events go to; `None` once detached.
    ring: Option<RingName>,
    options: proto::WindowOptions,
    tree: Node,
    inputs: Inputs,
    /// Its first frame is drawn and Opened sent; the windows a session lists
    /// are these.
    painted: bool,
    open: bool,
    screenshots: Vec<PendingShot>,
    /// Snapshots without pixels requested before its first frame, each with
    /// its reply ring, replied to at the end of the first frame egui does
    /// not discard, so each reads the window as it is drawn.
    early_shots: Vec<(RequestId, RingName)>,
    /// Its GpuSurfaces' saves, each waiting for a frame to paint it.
    saves: Vec<PendingSave>,
    /// What went wrong reading the tree when the window opened.
    problem: Option<String>,
    /// Physical pixels per point in the window's last frame.
    ppp: f32,
    /// Where the window opens, in the points egui gives winit; `None` for
    /// where the system places it.
    position: Option<[f32; 2]>,
    /// The size the window opens at, in points, in place of its options'
    /// when it opens again on a new device.
    size: Option<[f32; 2]>,
    /// Where the window was and its size at its last frame, in points,
    /// which it opens at again on a new device.
    last_outer: Option<[f32; 2]>,
    last_inner: Option<[f32; 2]>,
    /// Whether the window takes the focus as it opens; not while
    /// `PWRS_GUI_NO_FOCUS` tells windows to leave it.
    takes_focus: bool,
    /// Whether the root's frame has shown the window's viewport to egui.
    shown: bool,
    /// Whether the focus was requested before the window was shown.
    focus_asked: bool,
    /// Set-GuiFocus requests waiting for the window's next frame, each with
    /// its reply ring, replied to once that frame has given the focus.
    focus_replies: Vec<(RequestId, RingName)>,
    /// A test's input its frames are giving it, a step a frame.
    input: Option<input::Given>,
    /// The buttons tests' input leaves the pointer holding, by their places
    /// in [`proto::test_input::Button::ALL`].
    held: [bool; 3],
    /// The text the window last put on the clipboard, which its snapshot
    /// reports; `None` until it has put any.
    copied: Option<String>,
    /// Its operating-system window, found at its first paint.
    #[cfg(windows)]
    native: Option<usize>,
    /// The operating-system window in front when it opened, which gets the
    /// front back when it closes.
    #[cfg(windows)]
    behind: Option<usize>,
}

impl WindowState {
    fn new(id: WindowId, ring: RingName, options: proto::WindowOptions, tree: Node) -> WindowState {
        let mut state = WindowState {
            id,
            ring: Some(ring),
            options,
            tree,
            inputs: Inputs::default(),
            painted: false,
            open: true,
            screenshots: Vec::new(),
            early_shots: Vec::new(),
            saves: Vec::new(),
            problem: None,
            ppp: 1.0,
            position: None,
            size: None,
            last_outer: None,
            last_inner: None,
            takes_focus: true,
            shown: false,
            focus_asked: false,
            focus_replies: Vec::new(),
            input: None,
            held: [false; 3],
            copied: None,
            #[cfg(windows)]
            native: None,
            #[cfg(windows)]
            behind: foreground(),
        };
        let tree = state.tree.clone();
        if let Err(e) = seed_inputs(&tree, &mut state.inputs) {
            state.problem = Some(e);
        }
        state
    }

    /// Takes a test's `steps` of input, which the window's frames give it a
    /// step a frame ([`UiState::give_input`]); refused while it takes
    /// another test's, before its first frame, for a point outside its
    /// inside, and for a step egui cannot take.
    fn take_input(&mut self, request: RequestId, reply: RingName, steps: Vec<proto::test_input::Step>) -> Result<(), String> {
        if self.input.is_some() {
            return Err(format!("window {} is still taking another test's input", self.id));
        }
        let Some([across, down]) = self.last_inner.filter(|_| self.painted) else {
            return Err(format!("window {} has not been drawn yet, so there is no place to put the pointer on", self.id));
        };
        if let Some(why) = proto::test_input::refusal(&steps, self.id, [f64::from(across), f64::from(down)], self.held) {
            return Err(why);
        }
        let held = proto::test_input::held_after(&steps, self.held);
        self.input = Some(input::Given::new(request, reply, steps, self.held)?);
        self.held = held;
        Ok(())
    }

    /// The viewport to request a frame of the window from: its own once the
    /// root's frame has shown it to egui, the root's before, whose frame
    /// shows it. egui adds a viewport of default settings for a repaint
    /// requested of one it has not been shown, and eframe opens a window for
    /// it.
    fn repaint_target(&self) -> egui::ViewportId {
        if self.shown { viewport_id(self.id) } else { egui::ViewportId::ROOT }
    }

    /// The window's viewport as it opens: at the size [`UiState::size_new_windows`]
    /// gave it, or on a new device the size it had.
    fn builder(&self) -> egui::ViewportBuilder {
        let [w, h] = self.size.unwrap_or_else(|| self.options.fitted([0.0; 2], None));
        let mut b = egui::ViewportBuilder::default().with_title(self.options.title.clone()).with_resizable(self.options.resizable).with_inner_size([w, h]).with_active(self.takes_focus);
        if self.options.always_on_top {
            b = b.with_always_on_top();
        }
        if let Some(at) = self.position {
            b = b.with_position(at);
        }
        b
    }

    /// The inside size the window's content takes at its widgets' own sizes,
    /// as its frame lays it out: its menu bars, its widgets inside its
    /// margin, its button bar with the line over it and its status bars, one
    /// under another, as wide as the widest. Nothing drawn here is shown, its
    /// events go nowhere, and a problem met here is reported by the window's
    /// own frame, which also gives the focus a Set-GuiFocus named. Its
    /// widgets' ids are the window's own, so what one remembers, such as a
    /// grid's column widths, is never another window's measured in the same
    /// context.
    fn natural(&mut self, ui: &mut egui::Ui) -> egui::Vec2 {
        let window = self.id;
        let WindowState { tree, inputs, options, .. } = self;
        let reported = inputs.reported.clone();
        let focus = inputs.focus.take();
        let mut out = Vec::new();
        let mut size = egui::Vec2::ZERO;
        ui.scope_builder(egui::UiBuilder::new().id_salt(("pwrs-window", window)).sizing_pass().invisible(), |ui| {
            let mut add = |r: egui::Rect| {
                size.x = size.x.max(r.width());
                size.y += r.height();
            };
            // egui's top and bottom panels leave room for a separator line on
            // the side facing the window's middle.
            let line = f32::from(look::bar_line(ui.style(), true));
            for bar in own_of(tree, Kind::MenuBar) {
                // egui's menu bar fills the width it is given, so the bar is
                // as wide as its entries' own row and its frame's margins.
                let frame = look::bar_frame(ui.style());
                let shown = frame.show(ui, |ui| egui::MenuBar::new().ui(ui, |ui| ui.horizontal(|ui| menu_entries(ui, &bar.children, inputs, window, &mut out)).response.rect).inner);
                let entries = shown.inner.width() + frame.total_margin().sum().x;
                add(egui::Rect::from_min_size(shown.response.rect.min, egui::vec2(entries, shown.response.rect.height() + line)));
            }
            let button_bar = own_of(tree, Kind::ButtonBar).into_iter().next();
            let central = if button_bar.is_some() || options.dialog { look::dialog_frame(ui.style()) } else { look::central_frame(ui.style()) };
            add(central.show(ui, |ui| draw(ui, tree, inputs, window, &mut out)).response.rect);
            if let Some(bar) = button_bar {
                let strip = button_bar::WINDOWS_ORDER;
                let line = egui::Margin { top: look::bar_line(ui.style(), strip), ..egui::Margin::ZERO };
                add(look::button_bar_frame(ui.style(), strip).outer_margin(line).show(ui, |ui| draw_button_bar(ui, bar, inputs, window, &mut out)).response.rect);
            }
            for bar in own_of(tree, Kind::StatusBar) {
                let shown = look::bar_frame(ui.style()).show(ui, |ui| widget_row(ui, |ui| bar.children.iter().for_each(|c| draw(ui, c, inputs, window, &mut out)))).response.rect;
                add(egui::Rect::from_min_size(shown.min, egui::vec2(shown.width(), shown.height() + line)));
            }
        });
        inputs.reported = reported;
        inputs.focus = focus;
        size
    }

    /// Draws one frame in the window's theme: the shortcuts pressed act
    /// first and the answers of open file dialogs are taken, then the menu
    /// bar along the top, the status bar along the bottom and the button bar
    /// above it, the window's other widgets in the space between, inside a
    /// dialog's margin when it has a button bar or its options say it is a
    /// dialog, and an open context menu over them; files dropped and keys
    /// pressed then go to the widgets they land on, and Enter and Escape to
    /// its default and cancel buttons. A widget a Set-GuiFocus named takes
    /// the focus as it is drawn, or in the next frame when a grid drawn as a
    /// sizing pass passed over it, and at the frame's end the widget with the
    /// focus is noted for the window's snapshot. Its widgets' ids are the
    /// window's own, since every window draws in one context and two
    /// windows' trees number their nodes alike.
    fn frame(&mut self, ui: &mut egui::Ui) -> FrameOut {
        let ctx = ui.ctx().clone();
        ctx.set_theme(look::theme(self.options.theme));
        ui.set_style(ctx.style_of(ctx.theme()));
        let mut out = FrameOut { events: Vec::new(), closed: false, replies: Vec::new() };
        self.ppp = ctx.pixels_per_point();
        let (outer, inner) = ctx.input(|i| (i.viewport().outer_rect, i.viewport().inner_rect));
        if let Some(r) = outer {
            self.last_outer = Some([r.min.x, r.min.y]);
        }
        if let Some(r) = inner {
            self.last_inner = Some([r.width(), r.height()]);
        }
        let window = self.id;
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if !close_requested {
            press_shortcuts(&ctx, &self.tree, &mut self.inputs, window, &mut out.events);
        }
        let escape = self.options.close_on_escape && button_bar::role_button(&self.tree, Role::Cancel).is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if close_requested || escape {
            self.open = false;
            out.events.push(FromHelper::Closed { window, reason: CloseReason::User });
            out.closed = true;
            return out;
        }
        let WindowState { tree, inputs, options, .. } = self;
        let focused = focused_rect(&ctx);
        let asked = inputs.focus.is_some();
        let popup_open = egui::Popup::is_any_open(&ctx);
        choosers::answer_dialogs(inputs, window, &mut out.events);
        inputs.last_rects = std::mem::take(&mut inputs.rects);
        for uri in inputs.forgotten.drain(..) {
            ctx.forget_image(&uri);
        }
        let menu_bars = own_of(tree, Kind::MenuBar);
        if !menu_bars.is_empty() {
            egui::Panel::top(("pwrs-menu-bar", window)).frame(look::bar_frame(ui.style())).show(ui, |ui| {
                for bar in menu_bars {
                    let r = egui::MenuBar::new().ui(ui, |ui| menu_entries(ui, &bar.children, inputs, window, &mut out.events));
                    inputs.rects.insert(bar.id, r.response.rect);
                }
            });
        }
        let status_bars = own_of(tree, Kind::StatusBar);
        if !status_bars.is_empty() {
            egui::Panel::bottom(("pwrs-status-bar", window)).frame(look::bar_frame(ui.style())).show(ui, |ui| {
                for bar in status_bars {
                    let r = widget_row(ui, |ui| bar.children.iter().for_each(|c| draw(ui, c, inputs, window, &mut out.events)));
                    inputs.rects.insert(bar.id, r.response.rect);
                }
            });
        }
        let button_bar = own_of(tree, Kind::ButtonBar).into_iter().next();
        if let Some(bar) = button_bar {
            let strip = button_bar::WINDOWS_ORDER;
            egui::Panel::bottom(("pwrs-button-bar", window)).frame(look::button_bar_frame(ui.style(), strip)).show_separator_line(strip).show(ui, |ui| {
                let r = draw_button_bar(ui, bar, inputs, window, &mut out.events);
                inputs.rects.insert(bar.id, r);
            });
        }
        let central = if button_bar.is_some() || options.dialog { look::dialog_frame(ui.style()) } else { look::central_frame(ui.style()) };
        egui::CentralPanel::default().frame(central).show(ui, |ui| ui.push_id(("pwrs-window", window), |ui| draw(ui, tree, inputs, window, &mut out.events)));
        context_menus(ui, tree, inputs, window, &mut out.events);
        routed::route_drops(&ctx, tree, &inputs.rects, window, &mut out.events);
        let canvases = &inputs.canvases;
        let kept = |n: &Node, s: &Shortcut| n.kind == Kind::Canvas && canvas::keeps(canvases, n.id, s);
        routed::route_keys(&ctx, tree, &inputs.rects, focused, &kept, window, &mut out.events);
        routed::route_roles(&ctx, tree, &inputs.rects, focused, popup_open, window, &mut out.events);
        let holding = focused_rect(&ctx);
        inputs.focused = routed::focus_holder(tree, &inputs.rects, holding);
        let took = asked && inputs.focus.is_none();
        let waits = std::mem::take(&mut inputs.focus_waits) && inputs.focus.is_some();
        if asked {
            // The focus a widget took in this frame shows in the next, as
            // does one a grid's sizing pass passed over, which the next
            // frame gives.
            if !waits {
                inputs.focus = None;
            }
            ctx.request_repaint();
        }
        if !self.painted {
            self.painted = true;
            out.events.push(FromHelper::Opened { window: self.id });
        }
        if !waits {
            for (request, reply) in self.focus_replies.drain(..) {
                let outcome = if took { Ok(()) } else { Err(String::from("the window did not draw the widget, as on a tab that is not shown")) };
                out.replies.push((reply, FromHelper::FocusDone { request, outcome }));
            }
        }
        out
    }

    fn put_rows(&mut self, node: NodeId, start: u64, rows: Vec<proto::Row>, added: bool) -> Result<(), String> {
        let at = usize::try_from(start).map_err(|e| format!("row {start} is beyond this platform's address space: {e}"))?;
        let table = self.inputs.tables.entry(node).or_default();
        let at = at.min(table.rows.len());
        table.rows.truncate(at);
        table.rows.extend(rows);
        table.dirty = true;
        table.added |= added;
        Ok(())
    }

    /// Applies `ops`, returning what went wrong with each that failed.
    fn patch(&mut self, ops: Vec<PatchOp>) -> Vec<String> {
        let mut problems = Vec::new();
        for op in ops {
            if let Err(e) = self.apply(op) {
                problems.push(e);
            }
        }
        problems
    }

    fn apply(&mut self, op: PatchOp) -> Result<(), String> {
        let window = self.id;
        match op {
            PatchOp::SetProps { node, props } => {
                let n = self.tree.find_mut(node).ok_or_else(|| format!("window {window} has no node {node}"))?;
                let set: BTreeSet<String> = props.keys().cloned().collect();
                n.props.extend(props);
                self.inputs.reported.remove(&node);
                seed_node(n, &mut self.inputs, Some(&set)).map_err(|e| format!("node {node}: {e}"))
            }
            PatchOp::Insert { parent, index, node } => {
                seed_inputs(&node, &mut self.inputs)?;
                let at = usize::try_from(index).map_err(|e| format!("position {index} is beyond this platform's address space: {e}"))?;
                let p = self.tree.find_mut(parent).ok_or_else(|| format!("window {window} has no node {parent}"))?;
                let at = at.min(p.children.len());
                p.children.insert(at, node);
                Ok(())
            }
            PatchOp::Remove { node } => match remove_node(&mut self.tree, node) {
                Some(removed) => {
                    forget_pictures(&removed, &HashSet::new(), &mut self.inputs);
                    forget_surfaces(&removed, &HashSet::new(), &mut self.inputs);
                    Ok(())
                }
                None => Err(format!("window {window} has no node {node} to remove")),
            },
            PatchOp::Replace { node } => {
                seed_inputs(&node, &mut self.inputs)?;
                let id = node.id;
                let mut kept = HashSet::new();
                ids_of(&node, &mut kept);
                let n = self.tree.find_mut(id).ok_or_else(|| format!("window {window} has no node {id} to replace"))?;
                let old = std::mem::replace(n, node);
                forget_pictures(&old, &kept, &mut self.inputs);
                forget_surfaces(&old, &kept, &mut self.inputs);
                Ok(())
            }
            PatchOp::Append { node, prop, values } => {
                let n = self.tree.find_mut(node).ok_or_else(|| format!("window {window} has no node {node}"))?;
                match n.props.entry(prop.clone()).or_insert_with(|| Value::List(Vec::new())) {
                    Value::List(items) => items.extend(values),
                    other => return Err(format!("node {node}'s {prop} is {other:?}, not a list to add to")),
                }
                self.inputs.reported.remove(&node);
                seed_node(n, &mut self.inputs, Some(&BTreeSet::from([prop]))).map_err(|e| format!("node {node}: {e}"))
            }
            PatchOp::RemoveRows { node, rows } => {
                let table = self.inputs.tables.get_mut(&node).ok_or_else(|| format!("window {window} has no table {node}"))?;
                let moved = proto::remove_rows(&mut table.rows, &rows);
                let to = |r: &usize| moved.get(*r).copied().flatten();
                table.selected = table.selected.iter().filter_map(to).collect();
                table.anchor = table.anchor.as_ref().and_then(to);
                table.dirty = true;
                Ok(())
            }
            PatchOp::SetItem { node, item, props } => {
                let now = std::time::Instant::now();
                self.change_items(node, |canvases| canvas::set_item(canvases, node, &item, &props, now))
            }
            PatchOp::AddItems { node, group, items: added } => {
                let now = std::time::Instant::now();
                self.change_items(node, |canvases| canvas::add_items(canvases, node, group.as_deref(), &added, now))
            }
            PatchOp::RemoveItems { node, names } => self.change_items(node, |canvases| canvas::remove_items(canvases, node, &names)),
        }
    }

    /// Changes the items of the canvas `node` with `change`, on the board
    /// its canvas has; its node still has the items it was given, and a
    /// snapshot shows the board's.
    fn change_items(&mut self, node: NodeId, change: impl FnOnce(&mut HashMap<NodeId, canvas::Canvas>) -> Result<(), String>) -> Result<(), String> {
        let window = self.id;
        let n = self.tree.find(node).ok_or_else(|| format!("window {window} has no node {node}"))?;
        if n.kind != Kind::Canvas {
            return Err(format!("node {node} is a {:?}, which has no canvas items", n.kind));
        }
        change(&mut self.inputs.canvases).map_err(|e| format!("node {node}: {e}"))?;
        self.inputs.reported.remove(&node);
        Ok(())
    }

    /// Acts on the node named `name` as a person would, and returns the
    /// node and the kind of the event that raises, none when what a person
    /// does there raises nothing, and the value it took. A shortcut names
    /// no node: the enabled menu item it belongs to is clicked, or else the
    /// window's own shortcut raises Shortcut on the root.
    fn test_event(&mut self, name: &str, kind: EventKind, value: &Value) -> Result<(Option<(NodeId, EventKind)>, Value), String> {
        let window = self.id;
        if kind == EventKind::Shortcut {
            let written = value.as_str().ok_or_else(|| format!("{value:?} is not a shortcut"))?;
            let mut items = Vec::new();
            nodes_of(&self.tree, Kind::MenuItem, &mut items);
            for item in items {
                if item_shortcut(item)? == Some(written) {
                    return Ok((Some((item.id, EventKind::Click)), Value::Null));
                }
            }
            if own_shortcuts(&self.tree)?.contains(&written) {
                return Ok((Some((self.tree.id, EventKind::Shortcut)), Value::Str(written.to_string())));
            }
            return Err(format!("window {window} has no enabled menu item and no shortcut of its own for {written}"));
        }
        if kind == EventKind::Drop && name.is_empty() {
            if !flag(&self.tree, "drop", false)? {
                return Err(format!("window {window} takes no drop of its own; Show-GuiWindow -OnDrop gives it one"));
            }
            return Ok((Some((self.tree.id, EventKind::Drop)), dropped_files(value)?));
        }
        let found = self.tree.find_named(name).ok_or_else(|| format!("window {window} has no widget named {name}; its widgets are {}", self.tree.names().join(", ")))?;
        let (node, node_kind) = (found.id, found.kind);
        let link = node_kind == Kind::Text && opt_str(found, "style")? == Some("link");
        let editable = node_kind == Kind::ComboBox && flag(found, "editable", false)?;
        let one_line = node_kind == Kind::TextBox && !flag(found, "multiline", false)?;
        if !flag(found, "enabled", true)? && matches!(kind, EventKind::Click | EventKind::Change | EventKind::Submit | EventKind::Key) {
            let what = if node_kind == Kind::MenuItem { String::from("menu item") } else { format!("{node_kind:?}") };
            return Err(format!("the {what} is disabled, so it raises nothing"));
        }
        let (drops, keys) = (flag(found, "drop", false)?, flag(found, "keys", false)?);
        let many_paths = node_kind == Kind::FilePicker && flag(found, "multiple", false)?;
        let v = match (node_kind, kind) {
            (_, EventKind::Drop) if drops => dropped_files(value)?,
            (_, EventKind::Drop) => return Err(format!("{name} takes no drop; its command's -OnDrop gives it one")),
            (k, EventKind::Key) if keys => {
                let written = value.as_str().ok_or_else(|| format!("{value:?} is not a key written as a shortcut"))?;
                let pressed = Shortcut::parse(written)?;
                let typed = matches!(k, Kind::TextBox | Kind::NumberBox) || editable;
                if pressed.node_keeps(found, typed) {
                    return Err(format!("a {k:?} uses {} itself, so it raises no Key", pressed.written()));
                }
                if k == Kind::Canvas && canvas::keeps(&self.inputs.canvases, node, &pressed) {
                    return Err(format!("the canvas's view takes {} itself, so it raises no Key", pressed.written()));
                }
                Value::Str(pressed.written())
            }
            (_, EventKind::Key) => return Err(format!("{name} raises no Key; its command's -OnKey gives it one")),
            (Kind::Chart, EventKind::Select) => {
                let chart = self.tree.find(node).ok_or_else(|| format!("window {window} has no node {node}"))?;
                proto::chart::Chart::of(chart)?.find(value)?
            }
            (Kind::DatePicker, EventKind::Change) => {
                let picker = self.tree.find(node).ok_or_else(|| format!("window {window} has no node {node}"))?;
                let wanted = match value {
                    Value::Str(text) => proto::date::Minute::parse(text)?,
                    other => return Err(format!("{other:?} is not a date and time")),
                };
                let (low, high) = (choosers::minute(picker, "minimum")?, choosers::minute(picker, "maximum")?);
                if choosers::within(wanted, low, high) != wanted {
                    return Err(format!("{} is outside the chooser's range", wanted.written()));
                }
                self.inputs.dates.entry(node).or_default().chosen = Some(wanted);
                Value::Str(wanted.written())
            }
            (Kind::FilePicker, EventKind::Change) => {
                let paths = match value {
                    Value::Str(p) => vec![p.clone()],
                    Value::List(items) => items.iter().map(|p| p.as_str().map(str::to_string).ok_or_else(|| format!("{p:?} is not a path"))).collect::<Result<Vec<String>, String>>()?,
                    other => return Err(format!("{other:?} is not a path or a list of paths")),
                };
                if paths.is_empty() {
                    return Err(String::from("a person chooses at least one path"));
                }
                if paths.len() > 1 && !many_paths {
                    return Err(format!("the chooser takes one path without Multiple, and {} were given", paths.len()));
                }
                self.inputs.files.entry(node).or_default().paths = paths.clone();
                Value::List(paths.into_iter().map(Value::Str).collect())
            }
            (Kind::Button, EventKind::Click) => Value::Null,
            (Kind::Text, EventKind::Click) if link => Value::Null,
            (Kind::ListBox | Kind::Table, EventKind::Select) => {
                let rows = rows_of(value)?;
                self.select(node_kind, node, rows.iter().copied().collect());
                Value::List(rows.into_iter().map(row_value).collect())
            }
            (Kind::ListBox | Kind::Table, EventKind::Activate) => {
                let r = row_number(value)?;
                self.select(node_kind, node, BTreeSet::from([r]));
                row_value(r)
            }
            (Kind::CheckBox, EventKind::Change) => {
                let on = value.as_bool().ok_or_else(|| format!("{value:?} is not true or false"))?;
                self.inputs.checks.insert(node, on);
                Value::Bool(on)
            }
            (Kind::TextBox, EventKind::Change) | (Kind::TextBox, EventKind::Submit) if kind == EventKind::Change || one_line => {
                let text = value.as_str().ok_or_else(|| format!("{value:?} is not text"))?.to_string();
                self.inputs.texts.insert(node, text.clone());
                // The peek rule takes a test's text as it lands, as it takes
                // a person's at each frame they type in.
                let focused = self.inputs.focused == Some(node);
                if let Some(peek) = self.inputs.peeks.get_mut(&node) {
                    peek.update(focused, &text);
                }
                Value::Str(text)
            }
            (Kind::TextBox, EventKind::Peek) => {
                let hold = value.as_bool().ok_or_else(|| format!("{value:?} is not true or false"))?;
                let offered = self.inputs.peeks.get(&node).ok_or_else(|| format!("{name} is no password field, so it has no peek button"))?.offered();
                if hold && !offered {
                    return Err(format!("{name} offers no peek button now: a password field offers it once it has the focus and is typed into, and not again after the focus leaves until it is cleared"));
                }
                if hold {
                    self.inputs.peek_held.insert(node);
                } else {
                    self.inputs.peek_held.remove(&node);
                }
                return Ok((None, Value::Bool(hold)));
            }
            (Kind::ComboBox, EventKind::Change) if editable => {
                let text = value.as_str().ok_or_else(|| format!("{value:?} is not text"))?.to_string();
                self.inputs.texts.insert(node, text.clone());
                Value::Str(text)
            }
            (Kind::ColorPicker, EventKind::Change) => {
                let color = parse_color(value.as_str().ok_or_else(|| format!("{value:?} is not a color"))?)?;
                self.inputs.colors.insert(node, color);
                Value::Str(hex(color))
            }
            (Kind::NumberBox | Kind::Slider, EventKind::Change) => {
                let n = finite("the value", value)?;
                let (low, high) = bounds(self.tree.find(node).ok_or_else(|| format!("window {window} has no node {node}"))?)?;
                if n < low || n > high {
                    return Err(format!("{n} is outside the range {low} to {high}"));
                }
                self.inputs.numbers.insert(node, n);
                Value::Float(n)
            }
            (Kind::ComboBox | Kind::RadioGroup, EventKind::Change) => {
                let r = row_number(value)?;
                let found = self.tree.find(node).ok_or_else(|| format!("window {window} has no node {node}"))?;
                let count = req_strings(found, "items")?.len();
                if r >= count {
                    return Err(format!("row {r} is beyond the {count} items"));
                }
                self.inputs.choices.insert(node, Some(r));
                row_value(r)
            }
            (Kind::Tabs, EventKind::Change) => {
                let r = row_number(value)?;
                let count = self.tree.find(node).ok_or_else(|| format!("window {window} has no node {node}"))?.children.len();
                if r >= count {
                    return Err(format!("tab {r} is beyond the {count} tabs"));
                }
                self.inputs.choices.insert(node, Some(r));
                row_value(r)
            }
            (Kind::Markdown, EventKind::Click) => return self.test_link(node, value),
            (Kind::MenuItem, EventKind::Click) => Value::Null,
            (Kind::FilePicker, EventKind::Click) => {
                let chooser = self.tree.find(node).ok_or_else(|| format!("window {window} has no node {node}"))?;
                kept(&mut self.inputs.files, chooser)?.browse = true;
                return Ok((None, Value::Null));
            }
            (Kind::TreeView, EventKind::Select | EventKind::Activate | EventKind::Expand) => return self.test_tree(node, kind, value),
            (Kind::GpuSurface, EventKind::PointerDown | EventKind::PointerUp | EventKind::PointerMove | EventKind::Wheel) => {
                let surface = self.tree.find(node).ok_or_else(|| format!("window {window} has no node {node}"))?;
                if !pointer::Wanted::of(surface)?.has(kind) {
                    return Err(format!("{name} raises no {kind:?}; its command's -On{kind:?} gives it one"));
                }
                let place = self.inputs.surfaces.get(&node).and_then(|s| s.place).ok_or_else(|| format!("GpuSurface {name} has not been drawn, so there is no place to put the pointer on"))?;
                pointer::test_value(place, kind, value)?
            }
            (Kind::Canvas, _) => {
                let found = self.tree.find(node).ok_or_else(|| format!("window {window} has no node {node}"))?;
                let drawn = self.inputs.canvases.get(&node).ok_or_else(|| format!("Canvas {name} has not been drawn, so there is no place to put the pointer on"))?;
                match canvas::test_event(drawn, found, name, kind, value)? {
                    Some(v) => v,
                    None => return Ok((None, Value::Null)),
                }
            }
            (k, e) => return Err(format!("a {k:?} does not raise {e:?}")),
        };
        Ok((Some((node, kind)), v))
    }

    /// Clicks the link to `value` in the Markdown widget `node` as a person
    /// would: a link to a heading's `{#id}` scrolls the document there and
    /// raises nothing, and any other raises Click with its address. An
    /// address the document links nowhere is refused, naming its links.
    fn test_link(&mut self, node: NodeId, value: &Value) -> Result<(Option<(NodeId, EventKind)>, Value), String> {
        let window = self.id;
        let wanted = value.as_str().ok_or_else(|| format!("{value:?} is not a link's address"))?;
        let found = self.tree.find(node).ok_or_else(|| format!("window {window} has no node {node}"))?;
        let (text, _) = document_of(found)?;
        let links = proto::markdown::link_addresses(text);
        if !links.iter().any(|l| l == wanted) {
            let listed = if links.is_empty() { String::from("it has none") } else { format!("its links are {}", links.join(", ")) };
            return Err(format!("the document has no link to {wanted}; {listed}"));
        }
        match wanted.strip_prefix('#') {
            Some(heading) => {
                self.inputs.documents.entry(node).or_default().kept.scroll_to(heading);
                Ok((None, Value::Null))
            }
            None => Ok((Some((node, EventKind::Click)), Value::Str(wanted.to_string()))),
        }
    }

    /// Acts on the tree `tree` as a person would, its items named by their
    /// rows as shown, from 0: Select selects the rows given, Activate
    /// selects and activates one, and Expand opens one, raising Expand when
    /// its children are still to come and nothing when it has them.
    fn test_tree(&mut self, tree: NodeId, kind: EventKind, value: &Value) -> Result<(Option<(NodeId, EventKind)>, Value), String> {
        let window = self.id;
        let found = self.tree.find(tree).ok_or_else(|| format!("window {window} has no node {tree}"))?;
        check_items(&found.children)?;
        let multiple = flag(found, "multiple", false)?;
        let mut rows = Vec::new();
        shown_items(&found.children, &self.inputs.open_items, 0, &mut rows);
        let row_item = |r: usize| rows.get(r).map(|(item, _)| *item).ok_or_else(|| format!("row {r} is beyond the tree's {} rows as shown", rows.len()));
        match kind {
            EventKind::Select => {
                let picked = rows_of(value)?;
                if !multiple && picked.len() > 1 {
                    return Err(format!("the tree selects one row, and {} were given", picked.len()));
                }
                let mut selected = BTreeSet::new();
                for r in &picked {
                    selected.insert(row_item(*r)?.id);
                }
                let state = self.inputs.trees.entry(tree).or_default();
                state.selected = selected;
                state.anchor = picked.last().and_then(|r| rows.get(*r)).map(|(item, _)| item.id);
                Ok((Some((tree, EventKind::Select)), tree_selection(found, &state.selected)))
            }
            EventKind::Activate => {
                let item = row_item(row_number(value)?)?;
                let state = self.inputs.trees.entry(tree).or_default();
                state.selected = BTreeSet::from([item.id]);
                state.anchor = Some(item.id);
                Ok((Some((tree, EventKind::Activate)), item_value(item.id)))
            }
            EventKind::Expand => {
                let r = row_number(value)?;
                let item = row_item(r)?;
                if !opens(item) {
                    return Err(format!("row {r} is a leaf, with nothing to open"));
                }
                if lazy(item) {
                    if !self.inputs.expanding.insert(item.id) {
                        return Err(format!("row {r} is waiting for its children already"));
                    }
                    self.inputs.open_items.insert(item.id);
                    return Ok((Some((tree, EventKind::Expand)), item_value(item.id)));
                }
                self.inputs.open_items.insert(item.id);
                Ok((None, item_value(item.id)))
            }
            other => Err(format!("a TreeView does not raise {other:?}")),
        }
    }

    /// Selects `rows` of a list or table, as a test event does, the
    /// keyboard going to the last of them as a click there would take it.
    fn select(&mut self, kind: Kind, node: NodeId, rows: BTreeSet<usize>) {
        let last = rows.last().copied();
        if kind == Kind::ListBox {
            let keys = self.inputs.list_keys.entry(node).or_default();
            if last.is_some() {
                (keys.current, keys.anchor) = (last, last);
            }
            self.inputs.lists.insert(node, rows);
        } else {
            let table = self.inputs.tables.entry(node).or_default();
            if last.is_some() {
                (table.current, table.anchor) = (last, last);
            }
            table.selected = rows;
        }
    }

    fn snapshot(&self, pixels: Option<proto::Image>) -> proto::Snapshot {
        let mut state = BTreeMap::new();
        for (id, sel) in &self.inputs.lists {
            state.insert(*id, Value::List(sel.iter().copied().map(row_value).collect()));
        }
        for (id, t) in &self.inputs.tables {
            let mut m = BTreeMap::new();
            m.insert(String::from("selected"), Value::List(t.selected.iter().copied().map(row_value).collect()));
            m.insert(String::from("rows"), row_value(t.rows.len()));
            m.insert(String::from("shown"), row_value(t.view.len()));
            m.insert(String::from("filter"), Value::Str(t.filter.clone()));
            state.insert(*id, Value::Map(m));
        }
        for (id, t) in &self.inputs.trees {
            let Some(tree) = self.tree.find(*id) else {
                continue;
            };
            let mut shown = Vec::new();
            shown_items(&tree.children, &self.inputs.open_items, 0, &mut shown);
            let mut order = Vec::new();
            tree_order(&tree.children, &mut order);
            let mut m = BTreeMap::new();
            m.insert(String::from("selected"), tree_selection(tree, &t.selected));
            m.insert(String::from("open"), Value::List(order.into_iter().filter(|i| self.inputs.open_items.contains(i)).map(item_value).collect()));
            m.insert(String::from("shown"), Value::List(shown.iter().map(|(item, _)| item_value(item.id)).collect()));
            state.insert(*id, Value::Map(m));
        }
        for (id, on) in &self.inputs.checks {
            state.insert(*id, Value::Bool(*on));
        }
        for (id, text) in &self.inputs.texts {
            state.insert(*id, Value::Str(text.clone()));
        }
        for (id, color) in &self.inputs.colors {
            state.insert(*id, Value::Str(hex(*color)));
        }
        for (id, n) in &self.inputs.numbers {
            state.insert(*id, Value::Float(*n));
        }
        for (id, d) in &self.inputs.dates {
            state.insert(*id, d.chosen.map_or(Value::Null, |m| Value::Str(m.written())));
        }
        for (id, f) in &self.inputs.files {
            state.insert(*id, Value::List(f.paths.iter().cloned().map(Value::Str).collect()));
        }
        for (id, chosen) in &self.inputs.choices {
            let v = match chosen {
                Some(r) => row_value(*r),
                None => Value::Null,
            };
            state.insert(*id, v);
        }
        let mut bounds = BTreeMap::new();
        named_bounds(&self.tree, &self.inputs.rects, &mut bounds);
        let mut tree = self.tree.clone();
        canvas::current_items(&mut tree, &self.inputs.canvases);
        for (id, peek) in &self.inputs.peeks {
            if let Some(field) = tree.find_mut(*id) {
                field.props.insert(String::from("peek"), Value::Bool(peek.offered()));
            }
        }
        let focused = self.inputs.focused.filter(|id| self.tree.find(*id).is_some());
        proto::Snapshot { tree, state, pixels, bounds, engine: None, focused, copied: self.copied.clone(), moving: None }
    }

    /// Has the window's next frame give node `node` the keyboard focus;
    /// refused for a node the window does not have, one that takes no
    /// focus, a disabled one, a Canvas that takes no keys, and, once the
    /// window has drawn, one its last frame did not draw, as on a tab not
    /// shown.
    fn focus_node(&mut self, node: NodeId) -> Result<(), String> {
        let found = self.tree.find(node).ok_or_else(|| format!("the window has no widget with the number {node}"))?;
        if let Some(why) = found.focus_refusal() {
            return Err(why);
        }
        if found.kind == Kind::Canvas && !canvas::focusable(&self.inputs.canvases, found)? {
            return Err(String::from("the canvas takes no keys, neither for a key handler nor for its view's controls"));
        }
        if self.painted && !self.inputs.rects.contains_key(&node) {
            return Err(String::from("the window has not drawn the widget, as on a tab that is not shown"));
        }
        self.inputs.focus = Some(node);
        Ok(())
    }
}

/// The box each named node of `node`'s subtree was drawn in, from `rects`;
/// a node the last frame did not draw has none.
fn named_bounds(node: &Node, rects: &HashMap<NodeId, egui::Rect>, out: &mut BTreeMap<NodeId, proto::Bounds>) {
    if node.name.is_some()
        && let Some(r) = rects.get(&node.id)
    {
        out.insert(node.id, proto::Bounds { x: r.min.x, y: r.min.y, width: r.width(), height: r.height() });
    }
    for child in &node.children {
        named_bounds(child, rects, out);
    }
}

/// A screenshot as protocol pixels, cropped to `crop` (in points) when
/// given.
fn pixels_of(image: &egui::ColorImage, crop: Option<egui::Rect>, pixels_per_point: f32) -> Result<proto::Image, String> {
    let [w, h] = image.size;
    let (x0, y0, x1, y1) = match crop {
        None => (0, 0, w, h),
        Some(r) => {
            let px = |v: f32, max: usize| -> usize {
                let scaled = (v * pixels_per_point).round().max(0.0);
                (scaled as usize).min(max)
            };
            (px(r.min.x, w), px(r.min.y, h), px(r.max.x, w), px(r.max.y, h))
        }
    };
    if x1 <= x0 || y1 <= y0 {
        return Err(format!("the widget, at {crop:?} in points, lies outside the {w}x{h} screenshot"));
    }
    let width = u32::try_from(x1 - x0).map_err(|e| format!("a screenshot {} pixels wide does not fit the protocol: {e}", x1 - x0))?;
    let height = u32::try_from(y1 - y0).map_err(|e| format!("a screenshot {} pixels high does not fit the protocol: {e}", y1 - y0))?;
    let mut rgba = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for y in y0..y1 {
        for c in &image.pixels[y * w + x0..y * w + x1] {
            rgba.extend_from_slice(&c.to_srgba_unmultiplied());
        }
    }
    Ok(proto::Image { width, height, rgba })
}

/// A row number as a protocol value. Row counts fit an `i64` on every
/// platform the renderer runs on.
fn row_value(r: usize) -> Value {
    Value::Int(i64::try_from(r).expect("a row number fits an i64"))
}

/// One row number: a non-negative integer.
fn row_number(v: &Value) -> Result<usize, String> {
    let i = v.as_int().ok_or_else(|| format!("{v:?} is not a row number"))?;
    usize::try_from(i).map_err(|e| format!("{i} is not a row number: {e}"))
}

/// Row numbers from an event or property value: one row number or a list
/// of them, sorted and without repeats.
fn rows_of(value: &Value) -> Result<Vec<usize>, String> {
    let mut rows = match value {
        Value::List(items) => items.iter().map(row_number).collect::<Result<Vec<usize>, String>>()?,
        other => vec![row_number(other)?],
    };
    rows.sort_unstable();
    rows.dedup();
    Ok(rows)
}

/// The paths a test drops, a list of at least one; text is refused, since
/// egui's window layer drops files alone.
fn dropped_files(value: &Value) -> Result<Value, String> {
    let paths = match value {
        Value::List(items) => items.iter().map(|p| p.as_str().map(|s| Value::Str(s.to_string())).ok_or_else(|| format!("{p:?} is not a path"))).collect::<Result<Vec<Value>, String>>()?,
        Value::Str(_) => return Err(String::from("an egui window takes files dropped on it, given as a list of their paths; its window layer drops no text")),
        other => return Err(format!("{other:?} is not a list of paths")),
    };
    if paths.is_empty() {
        return Err(String::from("a drop has at least one file"));
    }
    Ok(Value::List(paths))
}

/// A table's column: its label, and where its header and cells sit across
/// when its `align` says.
#[derive(Clone, Debug, PartialEq)]
struct TableColumn {
    label: String,
    align: Option<egui::Align>,
}

impl TableColumn {
    /// Where the column's header and cells sit across: as its `align` says,
    /// else at the right when `first`, its first row's cell, is an amount,
    /// as PowerShell's tables align amounts, and at the left, a date among
    /// them.
    fn placed(&self, first: Option<&proto::Cell>) -> egui::Align {
        match (self.align, first) {
            (Some(a), _) => a,
            (None, Some(c)) if matches!(c.sort, SortKey::Number(_)) => egui::Align::Max,
            (None, _) => egui::Align::Min,
        }
    }
}

/// A table's columns from its `columns` property: a list of maps, each with
/// a `label` and an `align` of `left`, `center` or `right`, none when absent
/// or null.
fn columns_of(node: &Node) -> Result<Vec<TableColumn>, String> {
    match node.props.get("columns") {
        None => Ok(Vec::new()),
        Some(Value::List(items)) => items.iter().map(column_of).collect(),
        Some(other) => Err(format!("columns must be a list, not {other:?}")),
    }
}

fn column_of(c: &Value) -> Result<TableColumn, String> {
    let shape = || format!("each column must be a map with a string label, not {c:?}");
    let m = c.as_map().ok_or_else(shape)?;
    let label = m.get("label").and_then(Value::as_str).ok_or_else(shape)?.to_string();
    let align = match m.get("align") {
        None | Some(Value::Null) => None,
        Some(Value::Str(a)) => match a.as_str() {
            "left" => Some(egui::Align::Min),
            "center" => Some(egui::Align::Center),
            "right" => Some(egui::Align::Max),
            other => return Err(format!("a column's align must be left, center or right, not {other}")),
        },
        Some(other) => return Err(format!("a column's align must be left, center or right, not {other:?}")),
    };
    Ok(TableColumn { label, align })
}

/// Where a table cell's widget is placed across: at the left while the
/// table measures its columns, which egui_extras does in a sizing pass on
/// the table's first frame, so a column is as wide as its widest widget
/// rather than the room a cell placed at its right fills; `align` after.
fn measured_at(ui: &egui::Ui, align: egui::Align) -> egui::Align {
    if ui.is_sizing_pass() { egui::Align::Min } else { align }
}

/// egui's layout for a table cell's widget, placed across by `align` and
/// centered top to bottom.
fn cell_layout(align: egui::Align) -> egui::Layout {
    match align {
        egui::Align::Max => egui::Layout::right_to_left(egui::Align::Center),
        egui::Align::Center => egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
        egui::Align::Min => egui::Layout::left_to_right(egui::Align::Center),
    }
}

/// Sets up the input state of a subtree's widgets from their properties.
fn seed_inputs(node: &Node, inputs: &mut Inputs) -> Result<(), String> {
    seed_node(node, inputs, None).map_err(|e| format!("node {}: {e}", node.id))?;
    node.children.iter().try_for_each(|c| seed_inputs(c, inputs))
}

/// Sets up one node's input state from its properties: from all of them,
/// or with `set`, from the ones a patch set, leaving what a person changed
/// in the rest as it is. New items set without a selection leave the chosen
/// rows that are still among them chosen.
fn seed_node(node: &Node, inputs: &mut Inputs, set: Option<&BTreeSet<String>>) -> Result<(), String> {
    let given = |key: &str| set.is_none_or(|s| s.contains(key));
    let editable = node.kind == Kind::ComboBox && flag(node, "editable", false)?;
    match node.kind {
        Kind::ComboBox if editable => {
            if given("text") {
                inputs.texts.insert(node.id, req_str(node, "text")?.to_string());
            }
        }
        Kind::ColorPicker => {
            if given("color") {
                inputs.colors.insert(node.id, parse_color(req_str(node, "color")?)?);
            }
        }
        Kind::Image => {
            if given("image") {
                let (_, format) = picture_of(node)?;
                if let Some(old) = inputs.pictures.insert(node.id, picture_uri(extension_of(format)?)) {
                    inputs.forgotten.push(old);
                }
            }
        }
        Kind::Markdown => {
            if given("document") {
                seed_document(node, inputs)?;
            }
        }
        Kind::Canvas => {
            if given("items") {
                canvas::seed(node, inputs)?;
            }
        }
        Kind::ListBox => {
            if given("selected") {
                let selected = match node.props.get("selected") {
                    Some(v) => rows_of(v).map_err(|e| format!("selected: {e}"))?,
                    None => Vec::new(),
                };
                inputs.lists.insert(node.id, selected.into_iter().collect());
            } else if given("items") {
                let count = req_strings(node, "items")?.len();
                if let Some(selected) = inputs.lists.get_mut(&node.id) {
                    selected.retain(|r| *r < count);
                }
            }
        }
        Kind::Table => {
            if given("columns") {
                let columns = columns_of(node)?;
                let table = inputs.tables.entry(node.id).or_default();
                if table.columns != columns {
                    table.columns = columns;
                    table.dirty = true;
                }
            }
        }
        Kind::CheckBox => {
            if given("checked") {
                inputs.checks.insert(node.id, flag(node, "checked", false)?);
            }
        }
        Kind::TextBox => {
            if given("text") {
                let text = req_str(node, "text")?;
                inputs.texts.insert(node.id, text.to_string());
                if flag(node, "password", false)? {
                    inputs.peeks.insert(node.id, proto::peek::Peek::new(text));
                    inputs.peek_held.remove(&node.id);
                }
            }
        }
        Kind::NumberBox | Kind::Slider => {
            if given("value") {
                inputs.numbers.insert(node.id, req_number(node, "value")?);
            }
        }
        Kind::DatePicker => {
            let state = inputs.dates.entry(node.id).or_default();
            if given("value") {
                state.chosen = choosers::minute(node, "value")?;
            }
        }
        Kind::FilePicker => {
            let state = inputs.files.entry(node.id).or_default();
            if given("paths") {
                state.paths = choosers::paths(node)?;
            }
        }
        Kind::ComboBox | Kind::RadioGroup | Kind::Tabs => {
            let count = if node.kind == Kind::Tabs { node.children.len() } else { req_strings(node, "items")?.len() };
            if given("selected") {
                let chosen = match node.props.get("selected") {
                    None | Some(Value::Null) if node.kind == Kind::Tabs && count > 0 => Some(0),
                    None | Some(Value::Null) => None,
                    Some(v) => {
                        let r = row_number(v).map_err(|e| format!("selected: {e}"))?;
                        if r >= count {
                            return Err(format!("selected is row {r} of {count} items"));
                        }
                        Some(r)
                    }
                };
                inputs.choices.insert(node.id, chosen);
            } else if given("items")
                && let Some(chosen) = inputs.choices.get_mut(&node.id)
                && chosen.is_some_and(|r| r >= count)
            {
                *chosen = None;
            }
        }
        Kind::Split => {
            if given("position") {
                inputs.splits.insert(node.id, share(node)?);
            }
        }
        Kind::TreeView => {
            if given("selected") {
                let selected = match node.props.get("selected") {
                    None | Some(Value::Null) => BTreeSet::new(),
                    Some(Value::List(ids)) => ids.iter().map(item_id).collect::<Result<BTreeSet<NodeId>, String>>().map_err(|e| format!("selected: {e}"))?,
                    Some(one) => BTreeSet::from([item_id(one).map_err(|e| format!("selected: {e}"))?]),
                };
                inputs.trees.entry(node.id).or_default().selected = selected;
            }
        }
        Kind::TreeItem => {
            if given("more") {
                inputs.expanding.remove(&node.id);
                if flag(node, "more", false)? && node.children.is_empty() {
                    inputs.open_items.remove(&node.id);
                }
            }
        }
        Kind::Column
        | Kind::Row
        | Kind::Grid
        | Kind::Group
        | Kind::Tab
        | Kind::Scroll
        | Kind::Text
        | Kind::ProgressBar
        | Kind::Spinner
        | Kind::LogView
        | Kind::Chart
        | Kind::GpuSurface
        | Kind::Button
        | Kind::Separator
        | Kind::Spacer
        | Kind::MenuBar
        | Kind::Menu
        | Kind::MenuItem
        | Kind::ContextMenu
        | Kind::StatusBar
        | Kind::ButtonBar
        | Kind::Html => {}
    }
    Ok(())
}

/// A color written `#rrggbb`, in either case.
fn parse_color(text: &str) -> Result<[u8; 3], String> {
    let refuse = || format!("color must be #rrggbb, not {text}");
    let digits = text.strip_prefix('#').filter(|d| d.len() == 6 && d.bytes().all(|b| b.is_ascii_hexdigit())).ok_or_else(refuse)?;
    let byte = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).map_err(|e| format!("{}: {e}", refuse()));
    Ok([byte(0)?, byte(2)?, byte(4)?])
}

/// A color as `#rrggbb` in lower case.
fn hex(color: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

/// A split's `position`: the share of its space its first child takes,
/// from 0 to 1, and 0.5 when absent.
fn share(node: &Node) -> Result<f32, String> {
    let position = number(node, "position", 0.5)?;
    if (0.0..=1.0).contains(&position) { Ok(position as f32) } else { Err(format!("position must be from 0 to 1, not {position}")) }
}

/// Takes the node `id`, with the nodes under it, out of `tree`; none when
/// the tree has no such node.
fn remove_node(tree: &mut Node, id: NodeId) -> Option<Node> {
    if let Some(at) = tree.children.iter().position(|c| c.id == id) {
        return Some(tree.children.remove(at));
    }
    tree.children.iter_mut().find_map(|c| remove_node(c, id))
}

/// A string property the node may lack.
fn opt_str<'a>(node: &'a Node, key: &str) -> Result<Option<&'a str>, String> {
    match node.props.get(key) {
        None => Ok(None),
        Some(Value::Str(s)) => Ok(Some(s)),
        Some(other) => Err(format!("{key} must be a string, not {other:?}")),
    }
}

/// A string property the node must have.
fn req_str<'a>(node: &'a Node, key: &str) -> Result<&'a str, String> {
    opt_str(node, key)?.ok_or_else(|| format!("{key} is required"))
}

/// A flag property, `default` when the node lacks it.
fn flag(node: &Node, key: &str, default: bool) -> Result<bool, String> {
    match node.props.get(key) {
        None => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(other) => Err(format!("{key} must be true or false, not {other:?}")),
    }
}

/// A size in points, `default` when the node lacks it.
fn size(node: &Node, key: &str, default: f32) -> Result<f32, String> {
    match node.props.get(key) {
        None => Ok(default),
        Some(Value::Int(i)) => Ok(*i as f32),
        Some(Value::Float(f)) => Ok(*f as f32),
        Some(other) => Err(format!("{key} must be a number, not {other:?}")),
    }
}

/// A number property, `default` when the node lacks it.
fn number(node: &Node, key: &str, default: f64) -> Result<f64, String> {
    match node.props.get(key) {
        None => Ok(default),
        Some(v) => finite(key, v),
    }
}

/// A number property the node must have.
fn req_number(node: &Node, key: &str) -> Result<f64, String> {
    match node.props.get(key) {
        None => Err(format!("{key} is required")),
        Some(v) => finite(key, v),
    }
}

fn finite(key: &str, v: &Value) -> Result<f64, String> {
    match v.as_number() {
        Some(n) if n.is_finite() => Ok(n),
        _ => Err(format!("{key} must be a finite number, not {v:?}")),
    }
}

/// A number field's or slider's range: a slider's `minimum` and `maximum`,
/// which it must have, or a number field's, unbounded where it lacks them.
fn bounds(node: &Node) -> Result<(f64, f64), String> {
    let (low, high) = if node.kind == Kind::Slider {
        (req_number(node, "minimum")?, req_number(node, "maximum")?)
    } else {
        (number(node, "minimum", f64::NEG_INFINITY)?, number(node, "maximum", f64::INFINITY)?)
    };
    if low > high {
        return Err(format!("minimum {low} is above maximum {high}"));
    }
    Ok((low, high))
}

/// A number field's or slider's step: above 0, and 1 when absent.
fn step(node: &Node) -> Result<f64, String> {
    let step = number(node, "step", 1.0)?;
    if step > 0.0 { Ok(step) } else { Err(format!("step must be above 0, not {step}")) }
}

/// A list-of-strings property the node must have.
fn req_strings<'a>(node: &'a Node, key: &str) -> Result<Vec<&'a str>, String> {
    match node.props.get(key) {
        None => Err(format!("{key} is required")),
        Some(Value::List(items)) => items.iter().map(|v| v.as_str().ok_or_else(|| format!("{key} must contain only strings, not {v:?}"))).collect(),
        Some(other) => Err(format!("{key} must be a list of strings, not {other:?}")),
    }
}

fn event(out: &mut Vec<FromHelper>, window: WindowId, node: NodeId, kind: EventKind, value: Value) {
    out.push(FromHelper::Event { window, node, kind, value, at: proto::clock::now_ns() });
}

/// Draws a node and records where it was drawn; a property error is drawn
/// in its place and reported once. A menu bar, a status bar, a button bar
/// and a context menu take no place here: the window's frame draws them.
/// While a window is measured, a chart, a GPU surface and a canvas take
/// the size they are given, or egui's default area size, and draw nothing.
fn draw(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) {
    if ui.is_sizing_pass() && matches!(node.kind, Kind::Chart | Kind::GpuSurface | Kind::Canvas) {
        let rect = natural_box(ui, node);
        inputs.rects.insert(node.id, rect);
        return;
    }
    let drawn = match node.kind {
        Kind::Column | Kind::Tab => draw_column(ui, node, inputs, window, out),
        Kind::Row => draw_row(ui, node, inputs, window, out),
        Kind::Grid => draw_grid(ui, node, inputs, window, out),
        Kind::Group => draw_group(ui, node, inputs, window, out),
        Kind::Tabs => draw_tabs(ui, node, inputs, window, out),
        Kind::Split => draw_split(ui, node, inputs, window, out),
        Kind::Scroll => draw_scroll(ui, node, inputs, window, out),
        Kind::Text => draw_text(ui, node, inputs.take_focus(node.id), window, out),
        Kind::Image => draw_image(ui, node, inputs, window, out),
        Kind::Markdown => draw_markdown(ui, node, inputs, window, out),
        Kind::Button => draw_button(ui, node, inputs.take_focus(node.id), window, out),
        Kind::ProgressBar => draw_progress(ui, node),
        Kind::Spinner => Ok(ui.spinner().rect),
        Kind::LogView => draw_log(ui, node),
        Kind::Chart => chart::draw_chart(ui, node, window, out),
        Kind::GpuSurface => surface::draw_surface(ui, node, inputs, window, out),
        Kind::Canvas => canvas::draw_canvas(ui, node, inputs, window, out),
        Kind::Separator => Ok(ui.separator().rect),
        Kind::Spacer => spacer_size(node).map(|(s, _)| {
            let before = ui.cursor().min;
            ui.add_space(s);
            egui::Rect::from_min_max(before, ui.cursor().min.max(before))
        }),
        Kind::ListBox => draw_list(ui, node, inputs, window, out),
        Kind::Table => draw_table(ui, node, inputs, window, out),
        Kind::TreeView => draw_tree(ui, node, inputs, window, out),
        Kind::TreeItem => Err(String::from("a tree's items are drawn by their TreeView")),
        Kind::CheckBox => draw_check(ui, node, inputs, window, out),
        Kind::TextBox => draw_text_box(ui, node, inputs, window, out),
        Kind::NumberBox => draw_number(ui, node, inputs, window, out),
        Kind::Slider => draw_slider(ui, node, inputs, window, out),
        Kind::ComboBox => draw_combo(ui, node, inputs, window, out),
        Kind::RadioGroup => draw_radio(ui, node, inputs, window, out),
        Kind::ColorPicker => draw_color(ui, node, inputs, window, out),
        Kind::DatePicker => choosers::draw_date(ui, node, inputs, window, out),
        Kind::FilePicker => choosers::draw_file(ui, node, inputs),
        Kind::MenuBar | Kind::StatusBar | Kind::ButtonBar | Kind::ContextMenu => return,
        Kind::Menu | Kind::MenuItem => Err(String::from("a menu's entries are drawn in its menu bar, menu or context menu")),
        Kind::Html => Err(String::from("a page is drawn by the HTML renderer alone")),
    };
    match drawn {
        Ok(rect) => {
            inputs.rects.insert(node.id, rect);
        }
        Err(problem) => {
            let text = label(node, &problem);
            let rect = ui.colored_label(ui.visuals().error_fg_color, &text).rect;
            inputs.rects.insert(node.id, rect);
            report_once(inputs, out, window, node.id, "draw", text);
        }
    }
}

/// Draws a Text in its style; a link takes the focus when `take` says
/// Set-GuiFocus named it.
fn draw_text(ui: &mut egui::Ui, node: &Node, take: bool, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let text = req_str(node, "text")?;
    let r = match opt_str(node, "style")? {
        Some("heading") => ui.heading(text),
        Some("mono") => ui.monospace(text),
        Some("body") | None => ui.label(text),
        Some("instruction") => ui.label(egui::RichText::new(text).text_style(look::instruction()).color(ui.visuals().strong_text_color())),
        Some("link") => {
            let r = ui.link(text);
            if take {
                r.request_focus();
            }
            if r.clicked() {
                event(out, window, node.id, EventKind::Click, Value::Null);
            }
            r
        }
        Some(other) => return Err(format!("style must be heading, body, mono, link or instruction, not {other}")),
    };
    Ok(r.rect)
}

/// Draws a Markdown widget's document, its images showing the pictures
/// [`seed_document`] gave them. A link clicked raises Click with its
/// address; an image whose picture cannot be drawn shows its alt text and
/// is reported, once until the document changes.
fn draw_markdown(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let (text, _) = document_of(node)?;
    let Document { pictures, kept, failed } = inputs.documents.entry(node.id).or_default();
    let picture = |address: &str| match pictures.get(address) {
        Some(shown) => shown.clone(),
        None => markdown::Picture::Absent,
    };
    let drawn = ui.scope(|ui| markdown::show(ui, egui::Id::unique(("pwrs-gui-markdown", window, node.id)), text, &picture, kept));
    let shown = drawn.inner;
    for address in shown.clicked {
        event(out, window, node.id, EventKind::Click, Value::Str(address));
    }
    for (address, why) in shown.failed {
        if failed.insert(address.clone()) {
            out.push(FromHelper::Error { window: Some(window), context: String::from("draw"), text: format!("the image {address} shows its alt text: {why}") });
        }
    }
    Ok(drawn.response.rect)
}

/// An Image's picture: its bytes and their format.
fn picture_of(node: &Node) -> Result<(Arc<[u8]>, &str), String> {
    picture_parts(node.props.get("image").ok_or_else(|| String::from("image is required, a map of the picture's bytes and format"))?)
}

/// A picture's bytes and their format, from the map the session sends for
/// an Image or a Markdown document's image.
fn picture_parts(picture: &Value) -> Result<(Arc<[u8]>, &str), String> {
    let image = picture.as_map().ok_or_else(|| String::from("an image's picture must be a map of its bytes and format"))?;
    let bytes = match image.get("bytes") {
        Some(Value::Bytes(b)) => Arc::clone(b),
        Some(other) => return Err(format!("image's bytes must be bytes, not {other:?}")),
        None => return Err(String::from("image's bytes are required")),
    };
    let format = match image.get("format") {
        Some(Value::Str(f)) => f.as_str(),
        Some(other) => return Err(format!("image's format must be a string, not {other:?}")),
        None => return Err(String::from("image's format is required")),
    };
    Ok((bytes, format))
}

/// The extension that tells egui's loaders a picture's `format`.
fn extension_of(format: &str) -> Result<&'static str, String> {
    Ok(match format {
        "png" => "png",
        "jpeg" => "jpg",
        "gif" => "gif",
        "webp" => "webp",
        "bmp" => "bmp",
        "ico" => "ico",
        "svg" => "svg",
        other => return Err(format!("image's format must be png, jpeg, gif, webp, bmp, ico or svg, not {other}")),
    })
}

/// A new address in egui's caches for a picture the session read, which no
/// picture had before, ending in `extension`.
fn picture_uri(extension: &str) -> String {
    format!("bytes://pwrs-gui/picture-{}.{extension}", PICTURES.fetch_add(1, Ordering::Relaxed))
}

/// A new address in egui's caches for the https picture at `address`: the
/// address with its scheme in lower case and without its fragment, and a
/// fragment of its own naming the image, so each image's picture is
/// fetched and forgotten apart.
fn web_uri(address: &str) -> String {
    let rest = &address["https:".len()..];
    let rest = match rest.split_once('#') {
        Some((before, _fragment)) => before,
        None => rest,
    };
    format!("https:{rest}#pwrs-gui-picture-{}", PICTURES.fetch_add(1, Ordering::Relaxed))
}

/// A Markdown widget's document: its text, and the pictures the session
/// read for its images, by their addresses.
fn document_of(node: &Node) -> Result<(&str, &BTreeMap<String, Value>), String> {
    let document = node.props.get("document").and_then(Value::as_map).ok_or_else(|| String::from("document is required, a map of its text and pictures"))?;
    let text = match document.get("text") {
        Some(Value::Str(t)) => t.as_str(),
        Some(other) => return Err(format!("document's text must be a string, not {other:?}")),
        None => return Err(String::from("document's text is required")),
    };
    let pictures = match document.get("pictures") {
        Some(Value::Map(p)) => p,
        Some(_not_a_map) => return Err(String::from("document's pictures must be a map from each image's address to its picture")),
        None => return Err(String::from("document's pictures are required")),
    };
    Ok((text, pictures))
}

/// A Markdown image's picture as its document gives it.
enum Given {
    /// Bytes the session read, and the extension of their format.
    Read(Arc<[u8]>, &'static str),
    /// An https address.
    Web(String),
    /// None, so the image shows its alt text.
    Nothing,
}

/// Gives each image of a Markdown widget's document its picture. A picture
/// the widget showed before for the same address stays under its address in
/// egui's caches, so a document whose text alone changed neither decodes
/// nor fetches its pictures again; every other picture it showed is
/// forgotten. Every picture is read before any is given, so a document the
/// session sent wrongly changes nothing.
fn seed_document(node: &Node, inputs: &mut Inputs) -> Result<(), String> {
    let (text, read) = document_of(node)?;
    let mut wanted = Vec::new();
    for address in proto::markdown::image_addresses(text) {
        let picture = match read.get(&address) {
            Some(picture) => {
                let (bytes, format) = picture_parts(picture)?;
                Given::Read(bytes, extension_of(format)?)
            }
            None => match proto::markdown::source(&address) {
                proto::markdown::Source::Web(url) => Given::Web(url.to_string()),
                proto::markdown::Source::Data(_) | proto::markdown::Source::File(_) | proto::markdown::Source::Refused(_) => Given::Nothing,
            },
        };
        wanted.push((address, picture));
    }
    let mut before = match inputs.documents.remove(&node.id) {
        Some(document) => document.pictures,
        None => HashMap::new(),
    };
    let mut pictures = HashMap::new();
    for (address, picture) in wanted {
        let shown = match (picture, before.remove(&address)) {
            (Given::Read(bytes, _), Some(markdown::Picture::Bytes { uri, bytes: had, size })) if had[..] == bytes[..] => markdown::Picture::Bytes { uri, bytes: had, size },
            (Given::Web(_), Some(markdown::Picture::Web(uri))) => markdown::Picture::Web(uri),
            (picture, had) => {
                inputs.forgotten.extend(had.as_ref().and_then(markdown::Picture::uri).map(str::to_string));
                match picture {
                    Given::Read(bytes, extension) => markdown::Picture::Bytes { uri: picture_uri(extension), size: picture_size(&bytes, extension), bytes },
                    Given::Web(url) => markdown::Picture::Web(web_uri(&url)),
                    Given::Nothing => markdown::Picture::Absent,
                }
            }
        };
        pictures.insert(address, shown);
    }
    inputs.forgotten.extend(before.values().filter_map(markdown::Picture::uri).map(str::to_string));
    inputs.documents.insert(node.id, Document { pictures, ..Document::default() });
    Ok(())
}

/// Lets go of the pictures of the nodes of the subtree `old` whose ids are
/// not in `kept`, as a removed or replaced subtree's: egui forgets
/// each at the window's next frame.
fn forget_pictures(old: &Node, kept: &HashSet<NodeId>, inputs: &mut Inputs) {
    if !kept.contains(&old.id) {
        inputs.forgotten.extend(inputs.pictures.remove(&old.id));
        if let Some(document) = inputs.documents.remove(&old.id) {
            inputs.forgotten.extend(document.uris());
        }
    }
    for child in &old.children {
        forget_pictures(child, kept, inputs);
    }
}

/// Lets go of the GpuSurfaces and Canvases among the nodes of the subtree
/// `old` whose ids are not in `kept`, as a removed or replaced subtree's:
/// their draws, items and textures.
fn forget_surfaces(old: &Node, kept: &HashSet<NodeId>, inputs: &mut Inputs) {
    if !kept.contains(&old.id) {
        inputs.surfaces.remove(&old.id);
        inputs.canvases.remove(&old.id);
    }
    for child in &old.children {
        forget_surfaces(child, kept, inputs);
    }
}

/// The ids of a subtree's nodes.
fn ids_of(node: &Node, out: &mut HashSet<NodeId>) {
    out.insert(node.id);
    for child in &node.children {
        ids_of(child, out);
    }
}

/// A size in points the node may lack, above 0 when it has it.
fn opt_points(node: &Node, key: &str) -> Result<Option<f32>, String> {
    match node.props.get(key) {
        None => Ok(None),
        Some(v) => {
            let n = finite(key, v)?;
            if n > 0.0 { Ok(Some(n as f32)) } else { Err(format!("{key} must be above 0 points, not {n}")) }
        }
    }
}

/// How an Image's picture fills the box its width and height make.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Fit {
    Contain,
    Cover,
    Fill,
    None,
}

/// Where an Image's picture goes: the size of the space the Image takes,
/// the rectangle within that space the picture is painted in, the part of
/// the picture painted there, as texture coordinates from 0 to 1, and the
/// size the whole picture is drawn at, which an SVG is drawn to.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placed {
    size: egui::Vec2,
    at: egui::Rect,
    uv: egui::Rect,
    whole: egui::Vec2,
}

/// Where a picture of `natural` size in points goes, given the Image's
/// `width` and `height` in points, its `fit`, and the `room` across its
/// place leaves it: with neither, the picture at its own size, shrunk to the
/// room in its own proportions; with one, that side and the other in
/// proportion; with both, a box the fit fills.
fn place_picture(natural: egui::Vec2, width: Option<f32>, height: Option<f32>, fit: Fit, room: f32) -> Placed {
    let all = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    let filled = |size: egui::Vec2| Placed { size, at: egui::Rect::from_min_size(egui::Pos2::ZERO, size), uv: all, whole: size };
    let (nw, nh) = (natural.x.max(1.0), natural.y.max(1.0));
    let (w, h) = match (width, height) {
        (None, None) => {
            let scale = if nw > room && room > 0.0 { room / nw } else { 1.0 };
            return filled(egui::vec2(nw * scale, nh * scale));
        }
        (Some(w), None) => return filled(egui::vec2(w, w * nh / nw)),
        (None, Some(h)) => return filled(egui::vec2(h * nw / nh, h)),
        (Some(w), Some(h)) => (w, h),
    };
    let size = egui::vec2(w, h);
    let middle = egui::pos2(w / 2.0, h / 2.0);
    match fit {
        Fit::Fill => filled(size),
        Fit::Contain => {
            let scale = (w / nw).min(h / nh);
            let whole = egui::vec2(nw * scale, nh * scale);
            Placed { size, at: egui::Rect::from_center_size(middle, whole), uv: all, whole }
        }
        Fit::Cover => {
            let scale = (w / nw).max(h / nh);
            let whole = egui::vec2(nw * scale, nh * scale);
            Placed { size, at: egui::Rect::from_min_size(egui::Pos2::ZERO, size), uv: middle_of(w / whole.x, h / whole.y), whole }
        }
        Fit::None => {
            let shown = egui::vec2(nw.min(w), nh.min(h));
            Placed { size, at: egui::Rect::from_center_size(middle, shown), uv: middle_of(shown.x / nw, shown.y / nh), whole: egui::vec2(nw, nh) }
        }
    }
}

/// A picture's own size, in pixels as points, read from its bytes without
/// drawing it: an SVG's from its size, any other format's from its header;
/// `None` where the bytes say none, which the picture's drawing reports.
fn picture_size(bytes: &[u8], format: &str) -> Option<egui::Vec2> {
    if format == "svg" {
        let tree = resvg::usvg::Tree::from_data(bytes, &resvg::usvg::Options::default()).ok()?;
        return Some(egui::vec2(tree.size().width(), tree.size().height()));
    }
    let (w, h) = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?.into_dimensions().ok()?;
    Some(egui::vec2(w as f32, h as f32))
}

/// The middle `across` and `down` of a texture, as texture coordinates.
fn middle_of(across: f32, down: f32) -> egui::Rect {
    egui::Rect::from_min_max(egui::pos2((1.0 - across) / 2.0, (1.0 - down) / 2.0), egui::pos2((1.0 + across) / 2.0, (1.0 + down) / 2.0))
}

/// Draws an Image: its picture through egui's loaders, under the address
/// its node was given, placed as [`place_picture`] says, an SVG drawn at the
/// size the whole picture takes. A spinner stands in while the picture
/// loads; one that cannot be drawn shows its alt text, or why, and is
/// reported once.
fn draw_image(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let (bytes, _) = picture_of(node)?;
    let uri = inputs.pictures.get(&node.id).cloned().ok_or_else(|| String::from("its picture has no address"))?;
    let (width, height) = (opt_points(node, "width")?, opt_points(node, "height")?);
    let fit = match opt_str(node, "fit")? {
        None | Some("contain") => Fit::Contain,
        Some("cover") => Fit::Cover,
        Some("fill") => Fit::Fill,
        Some("none") => Fit::None,
        Some(other) => return Err(format!("fit must be contain, cover, fill or none, not {other}")),
    };
    let alt = opt_str(node, "alt")?;
    let room = ui.available_width();
    if ui.is_sizing_pass() {
        let (_, format) = picture_of(node)?;
        let natural = picture_size(&bytes, format).unwrap_or(egui::Vec2::splat(24.0));
        return Ok(ui.allocate_exact_size(place_picture(natural, width, height, fit, room).size, egui::Sense::hover()).0);
    }
    let image = egui::Image::new(egui::ImageSource::Bytes { uri: uri.into(), bytes: egui::load::Bytes::Shared(bytes) });
    let loaded = match image.load_for_size(ui.ctx(), ui.available_size()) {
        Ok(TexturePoll::Ready { texture }) => {
            let placed = place_picture(texture.size, width, height, fit, room);
            image.fit_to_exact_size(placed.whole).maintain_aspect_ratio(false).load_for_size(ui.ctx(), placed.whole).map(|poll| (placed, poll))
        }
        Ok(TexturePoll::Pending { size }) => Ok((place_picture(size.unwrap_or(egui::Vec2::splat(24.0)), width, height, fit, room), TexturePoll::Pending { size })),
        Err(e) => Err(e),
    };
    match loaded {
        Ok((placed, TexturePoll::Ready { texture })) => {
            let (rect, response) = ui.allocate_exact_size(placed.size, egui::Sense::hover());
            ui.painter().image(texture.id, placed.at.translate(rect.min.to_vec2()), placed.uv, egui::Color32::WHITE);
            if let Some(a) = alt {
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, a));
            }
            Ok(rect)
        }
        Ok((placed, TexturePoll::Pending { .. })) => {
            let (rect, _) = ui.allocate_exact_size(placed.size, egui::Sense::hover());
            egui::Spinner::new().size(rect.size().min_elem().min(24.0)).paint_at(ui, rect);
            Ok(rect)
        }
        Err(e) => {
            let why = format!("the picture could not be drawn: {e}");
            report_once(inputs, out, window, node.id, "draw", why.clone());
            let shown = match alt {
                Some(a) => ui.label(a),
                None => ui.colored_label(ui.visuals().error_fg_color, why),
            };
            Ok(shown.rect)
        }
    }
}

/// Draws a progress bar as Windows and macOS draw one: a line with its
/// text at the left, cut short where it would reach the figure, and how
/// much is done at the right, over a thin bar [`tokens::PROGRESS_BAR`]
/// high, the selection's fill on a control's rail; as wide as the room it
/// is given and 96 points at least, as egui's own bar is, and in a sizing
/// pass as wide as its text, the gap and the figure, 96 points at least.
/// A screen reader reads it as egui's bar, with its text and how much is
/// done.
fn draw_progress(ui: &mut egui::Ui, node: &Node) -> Result<egui::Rect, String> {
    let value = req_number(node, "value")?;
    let maximum = number(node, "maximum", 100.0)?;
    if maximum <= 0.0 {
        return Err(format!("maximum must be above 0, not {maximum}"));
    }
    if !(0.0..=maximum).contains(&value) {
        return Err(format!("value must be from 0 to {maximum}, not {value}"));
    }
    let done = (value / maximum * 100.0).floor();
    let text = opt_str(node, "text")?.unwrap_or("");
    let gap = ui.spacing().item_spacing;
    let figure = egui::WidgetText::from(format!("{done}%")).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
    let width = if ui.is_sizing_pass() {
        let whole = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
        whole.size().x + gap.x + figure.size().x
    } else {
        ui.available_size_before_wrap().x
    }
    .max(96.0);
    let said = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Truncate), (width - figure.size().x - gap.x).max(0.0), egui::TextStyle::Body);
    let thick = tokens::PROGRESS_BAR;
    let line = figure.size().y.max(said.size().y);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, line + gap.y + thick), egui::Sense::hover());
    response.widget_info(|| {
        let mut info = if text.is_empty() { egui::WidgetInfo::new(egui::Role::ProgressIndicator) } else { egui::WidgetInfo::labeled(egui::Role::ProgressIndicator, ui.is_enabled(), text) };
        info.value = Some(done);
        info
    });
    if ui.is_rect_visible(rect) {
        let ink = ui.visuals().text_color();
        let painter = ui.painter();
        painter.galley(rect.left_top(), said, ink);
        painter.galley(egui::pos2(rect.right() - figure.size().x, rect.top()), figure, ink);
        let rail = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - thick), rect.right_bottom());
        let round = egui::CornerRadius::same((thick / 2.0).round() as u8);
        painter.rect_filled(rail, round, ui.visuals().widgets.inactive.bg_fill);
        let filled = (value / maximum) as f32;
        if filled > 0.0 {
            painter.rect_filled(rail.with_max_x(rail.left() + rail.width() * filled), round, ui.visuals().selection.bg_fill);
        }
    }
    Ok(rect)
}

/// Draws a log's lines in an area that scrolls, at its newest: as wide
/// and as high as its `width` and `height` where they are given, however
/// many lines it has; without a width, as wide as its widest line while its
/// window is measured, and without a height, as high as its lines within
/// the room it is given. Either side goes below the least size egui gives
/// an area that scrolls, 64 points, where these make it smaller.
fn draw_log(ui: &mut egui::Ui, node: &Node) -> Result<egui::Rect, String> {
    let lines = req_strings(node, "lines")?;
    let size = |key: &str| -> Result<Option<f32>, String> {
        match node.props.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(given) => {
                let n = finite(key, given)?;
                if n < 0.0 { Err(format!("{key} must be 0 or more points, not {n}")) } else { Ok(Some(n as f32)) }
            }
        }
    };
    let (width, height) = (size("width")?, size("height")?);
    let mut area = egui::ScrollArea::vertical()
        .id_salt(("log", node.id))
        .auto_shrink([ui.is_sizing_pass() && width.is_none(), height.is_none()])
        .stick_to_bottom(true)
        .min_scrolled_height(height.unwrap_or(0.0));
    if let Some(w) = width {
        area = area.max_width(w).min_scrolled_width(w);
    }
    if let Some(h) = height {
        area = area.max_height(h);
    }
    let row = ui.text_style_height(&egui::TextStyle::Monospace);
    let shown = ui.scope(|ui| {
        area.show_rows(ui, row, lines.len(), |ui, rows| {
            for line in &lines[rows] {
                ui.monospace(*line);
            }
        })
    });
    Ok(shown.response.rect)
}

/// Draws a button, which takes the focus when `take` says Set-GuiFocus
/// named it.
fn draw_button(ui: &mut egui::Ui, node: &Node, take: bool, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let text = req_str(node, "text")?;
    let enabled = flag(node, "enabled", true)?;
    let r = ui.add_enabled(enabled, button_of(ui, text, Role::of(node)?, enabled));
    if take {
        r.request_focus();
    }
    if r.clicked() {
        event(out, window, node.id, EventKind::Click, Value::Null);
    }
    Ok(r.rect)
}

/// A button showing `text`: the default one, while it is enabled, in the
/// selection's colors, the accent egui draws selected rows in; a disabled
/// default button is drawn as any disabled button is, as Windows and macOS
/// draw one.
fn button_of<'a>(ui: &egui::Ui, text: &'a str, role: Role, enabled: bool) -> egui::Button<'a> {
    if role == Role::Default && enabled {
        let selection = &ui.visuals().selection;
        egui::Button::new(egui::RichText::new(text).color(selection.stroke.color)).fill(selection.bg_fill)
    } else {
        egui::Button::new(text)
    }
}

/// Draws a button bar's buttons at its right in the order its system gives
/// them ([`button_bar::order`]), each at least [`tokens::DIALOG`]'s button
/// size and its gap apart, and returns where; a child that is not a button,
/// or a button that cannot be drawn, shows its problem in its place. The
/// buttons are drawn left to right, so Tab moves through them in the order
/// shown.
fn draw_button_bar(ui: &mut egui::Ui, bar: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> egui::Rect {
    // A button whose role cannot be read stays in its place among the
    // others, and draws its problem.
    let roles: Vec<Role> = bar.children.iter().map(|b| Role::of(b).unwrap_or(Role::Other)).collect();
    let order = button_bar::order(&roles, button_bar::WINDOWS_ORDER);
    let gap = tokens::DIALOG.button_gap;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        if !ui.is_sizing_pass() {
            // The space takes no item spacing after it, so the buttons and
            // the gaps between them end at the bar's margin.
            let widths: f32 = order.iter().map(|&i| bar_button_width(ui, &bar.children[i])).sum();
            let used = widths + gap * order.len().saturating_sub(1) as f32;
            ui.add_space((ui.available_width() - used).max(0.0));
        }
        for &i in &order {
            let c = &bar.children[i];
            let drawn = if c.kind == Kind::Button {
                draw_bar_button(ui, c, inputs.take_focus(c.id), window, out)
            } else {
                Err(format!("a ButtonBar contains only Button widgets, and this one contains a {:?}", c.kind))
            };
            match drawn {
                Ok(rect) => {
                    inputs.rects.insert(c.id, rect);
                }
                Err(problem) => {
                    let text = label(c, &problem);
                    let rect = ui.colored_label(ui.visuals().error_fg_color, &text).rect;
                    inputs.rects.insert(c.id, rect);
                    report_once(inputs, out, window, c.id, "draw", text);
                }
            }
        }
    })
    .response
    .rect
}

/// How wide a button bar's button is drawn: its text and padding, at least
/// [`tokens::DIALOG`]'s button width; a child that is not a button, or has
/// no text, at that width.
fn bar_button_width(ui: &egui::Ui, node: &Node) -> f32 {
    let least = tokens::DIALOG.button[0];
    let Ok(text) = req_str(node, "text") else {
        return least;
    };
    let shown = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    (shown.size().x + 2.0 * ui.spacing().button_padding.x).max(least)
}

/// Draws a button bar's button at the bar's button size, which takes the
/// focus when `take` says Set-GuiFocus named it.
fn draw_bar_button(ui: &mut egui::Ui, node: &Node, take: bool, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let text = req_str(node, "text")?;
    let enabled = flag(node, "enabled", true)?;
    let [width, height] = tokens::DIALOG.button;
    let r = ui.add_enabled(enabled, button_of(ui, text, Role::of(node)?, enabled).min_size(egui::vec2(width, height)));
    if take {
        r.request_focus();
    }
    if r.clicked() {
        event(out, window, node.id, EventKind::Click, Value::Null);
    }
    Ok(r.rect)
}

/// egui's layout for widgets one under another, each placed across by
/// `align`.
fn column_layout(align: Align) -> egui::Layout {
    match align {
        Align::Start => egui::Layout::top_down(egui::Align::Min),
        Align::Center => egui::Layout::top_down(egui::Align::Center),
        Align::End => egui::Layout::top_down(egui::Align::Max),
        Align::Stretch => egui::Layout::top_down_justified(egui::Align::Min),
    }
}

/// Draws a Column's children one under another, each placed across by its
/// `align`, a gap after each but the last and a Spacer, which adds its room
/// alone. Where the room has an end, the children that fill down
/// ([`proto::fills_down`]) share the height the others laid out in the
/// Column ([`proto::in_flow`]) took in the frame before leaves of it, three
/// rows of widgets each at least, so a long list scrolls in its place rather
/// than push the widgets after it out of the window; while the window is
/// measured, each takes its own height. When the others are drawn at heights
/// other than those, as in a window's first frame or after a text is
/// changed, it asks for another frame, in which the shares settle.
fn draw_column(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let align = Align::of(node)?;
    let fills: Vec<bool> = node.children.iter().map(proto::fills_down).collect();
    let count = fills.iter().filter(|f| **f).count();
    let room = ui.available_height();
    let placed: Vec<(&Node, bool)> = node.children.iter().zip(fills.iter().copied()).filter(|(c, _)| proto::in_flow(c)).collect();
    let others = |rects: &HashMap<NodeId, egui::Rect>| -> f32 { placed.iter().filter(|(_, f)| !*f).map(|(c, _)| rects.get(&c.id).map_or(0.0, egui::Rect::height)).sum() };
    let before = others(&inputs.last_rects);
    let share = (count > 0 && room.is_finite() && !ui.is_sizing_pass()).then(|| {
        let gapped = placed.iter().rev().skip(1).filter(|(c, _)| c.kind != Kind::Spacer).count();
        let gaps = ui.spacing().item_spacing.y * gapped as f32;
        ((room - before - gaps) / count as f32).max(3.0 * ui.spacing().interact_size.y)
    });
    let r = ui.with_layout(column_layout(align), |ui| {
        for (c, fills) in node.children.iter().zip(&fills) {
            match share {
                Some(height) if *fills && c.kind == Kind::Spacer => {
                    let least = spacer_size(c).map_or(0.0, |(size, _)| size);
                    let before = ui.cursor().min;
                    ui.add_space(height.max(least));
                    inputs.rects.insert(c.id, egui::Rect::from_min_max(before, ui.cursor().min.max(before)));
                }
                Some(height) if *fills => {
                    let size = egui::vec2(ui.available_width(), height);
                    ui.allocate_ui_with_layout(size, egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                        ui.set_min_height(height);
                        draw(ui, c, inputs, window, out);
                    });
                }
                _ => draw(ui, c, inputs, window, out),
            }
        }
    });
    if share.is_some() && others(&inputs.rects) != before {
        ui.ctx().request_repaint();
    }
    Ok(r.response.rect)
}

/// Lays `add` out in a row at least as high as an interactive widget, as
/// egui draws a row: its sizing pass measures a row by what it contains, so a
/// row of text alone would measure lower than it is drawn.
fn widget_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    ui.horizontal(|ui| {
        ui.set_min_height(ui.spacing().interact_size.y);
        add(ui)
    })
}

/// Draws a Row's children side by side, each centered top to bottom, placed
/// as a group across by its `align`: at the left; in the middle or at the
/// right by the width they took in the frame before; or, with `stretch`,
/// sharing the room equally, each in a box of its own with a gap after each
/// box but the last. Otherwise a gap follows each child but the last and a
/// Spacer, which adds its room alone. A Spacer with `fill` takes the room
/// the others and those gaps took in the frame before leave, shared with
/// any other such Spacer. While the window is measured, each takes its own
/// width.
fn draw_row(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let align = Align::of(node)?;
    let measuring = ui.is_sizing_pass();
    let gap = ui.spacing().item_spacing.x;
    let placed: Vec<&Node> = node.children.iter().filter(|c| proto::in_flow(c)).collect();
    let box_gaps = gap * placed.len().saturating_sub(1) as f32;
    let gaps = gap * placed.iter().rev().skip(1).filter(|c| c.kind != Kind::Spacer).count() as f32;
    let filling = |c: &Node| c.kind == Kind::Spacer && spacer_size(c).is_ok_and(|(_, fill)| fill);
    let spacers = placed.iter().filter(|c| filling(c)).count();
    let room = ui.available_width();
    let others: Option<f32> = placed.iter().filter(|c| !filling(c)).map(|c| inputs.last_rects.get(&c.id).map(egui::Rect::width)).sum();
    let spare = others.map(|w| (room - w - gaps).max(0.0));
    let r = widget_row(ui, |ui| {
        if align == Align::Stretch && !measuring && !placed.is_empty() {
            let each = ((room - box_gaps) / placed.len() as f32).max(0.0);
            for c in &node.children {
                if !proto::in_flow(c) {
                    draw(ui, c, inputs, window, out);
                    continue;
                }
                ui.allocate_ui_with_layout(egui::vec2(each, ui.spacing().interact_size.y), egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                    ui.set_min_width(each);
                    draw(ui, c, inputs, window, out);
                });
            }
            return;
        }
        if spare.is_none() && !measuring && (spacers > 0 || matches!(align, Align::Center | Align::End)) {
            ui.ctx().request_repaint();
        }
        if spacers == 0 && !measuring {
            match (align, spare) {
                (Align::Center, Some(s)) => ui.add_space(s / 2.0),
                (Align::End, Some(s)) => ui.add_space(s),
                _ => {}
            }
        }
        for c in &node.children {
            if filling(c) && !measuring {
                let least = spacer_size(c).map_or(0.0, |(size, _)| size);
                let before = ui.cursor().min;
                ui.add_space(spare.map_or(least, |s| (s / spacers as f32).max(least)));
                inputs.rects.insert(c.id, egui::Rect::from_min_max(before, ui.cursor().min.max(before)));
            } else {
                draw(ui, c, inputs, window, out);
            }
        }
    });
    Ok(r.response.rect)
}

/// A Spacer's `size`, 8 when absent, and whether it fills.
fn spacer_size(node: &Node) -> Result<(f32, bool), String> {
    Ok((size(node, "size", 8.0)?, flag(node, "fill", false)?))
}

/// The box a chart, GPU surface or canvas takes while its window is
/// measured: the `width` and `height` it is given, else egui's default area
/// size.
fn natural_box(ui: &mut egui::Ui, node: &Node) -> egui::Rect {
    let fallback = ui.spacing().default_area_size;
    let side = |key: &str, otherwise: f32| node.props.get(key).and_then(Value::as_number).map_or(otherwise, |v| v as f32);
    let size = egui::vec2(side("width", fallback.x), side("height", fallback.y));
    ui.allocate_exact_size(size, egui::Sense::hover()).0
}

/// A grid's `columns`: a whole number above 0.
fn column_count(node: &Node) -> Result<usize, String> {
    let given = node.props.get("columns").ok_or_else(|| String::from("columns is required"))?;
    let c = given.as_int().ok_or_else(|| format!("columns must be a whole number, not {given:?}"))?;
    if c < 1 {
        return Err(format!("columns must be above 0, not {c}"));
    }
    usize::try_from(c).map_err(|e| format!("columns {c} is beyond this platform's address space: {e}"))
}

/// Draws a Grid's children a row at a time, each placed in its cell as a
/// Column places its children by the Grid's `align`; with `stretch` the
/// last column also takes the room left across. A column is as wide as its
/// widest cell, so text in a cell stays on one line, but in a stretched
/// last column, which wraps it in the room it takes. In a sizing pass, as
/// egui makes a grid's first frame and as a window is measured, a cell has
/// the room the grid has left across, so it takes its own size as a
/// Column's widget does, and a cell of a stretched last column a control's
/// least width, so the window's other widgets set the width it fills.
fn draw_grid(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let columns = column_count(node)?;
    let align = Align::of(node)?;
    let r = egui::Grid::new(("grid", node.id)).num_columns(columns).show(ui, |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        for (i, c) in node.children.iter().enumerate() {
            let last = (i + 1) % columns == 0;
            if ui.is_sizing_pass() {
                // A cell measured is drawn from the top of its row in a scope
                // of its own, whose widget ids a drawn frame's do not share,
                // so the focus a Set-GuiFocus gave waits for a drawn frame.
                let least = ui.spacing().interact_size.x;
                let room = if last && align == Align::Stretch { least } else { (ui.max_rect().right() - ui.cursor().left()).max(least) };
                let cell = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(room, ui.available_height()));
                let focus = inputs.focus.take();
                ui.scope_builder(egui::UiBuilder::new().max_rect(cell).layout(column_layout(Align::Start)), |ui| draw(ui, c, inputs, window, out));
                inputs.focus_waits |= focus.is_some();
                inputs.focus = focus;
            } else if align == Align::Start {
                draw(ui, c, inputs, window, out);
            } else if align == Align::Stretch && last {
                let width = ui.available_width();
                ui.allocate_ui_with_layout(egui::vec2(width, ui.spacing().interact_size.y), column_layout(Align::Stretch), |ui| {
                    ui.style_mut().wrap_mode = None;
                    ui.set_min_width(width);
                    draw(ui, c, inputs, window, out);
                });
            } else {
                ui.with_layout(column_layout(align), |ui| draw(ui, c, inputs, window, out));
            }
            if last {
                ui.end_row();
            }
        }
    });
    Ok(r.response.rect)
}

fn draw_group(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let title = opt_str(node, "title")?;
    let r = look::group_frame(ui.style()).show(ui, |ui| {
        ui.vertical(|ui| {
            if let Some(t) = title {
                ui.strong(t);
            }
            node.children.iter().for_each(|c| draw(ui, c, inputs, window, out));
        })
    });
    Ok(r.response.rect)
}

/// A tab's title, as Fluent's TabList draws one: a tab, to a screen reader
/// as to the page's runtime, the size egui gives a button of its text,
/// without a frame, its text in the weak text color, the hovered one under
/// the pointer or while pressed, and the strong one when `chosen`, with the
/// selection's stroke about it while it has the keyboard's focus.
fn tab_title(ui: &mut egui::Ui, title: &str, chosen: bool) -> egui::Response {
    let padding = ui.spacing().button_padding;
    let galley = egui::WidgetText::from(title).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button);
    let size = egui::vec2(galley.size().x + 2.0 * padding.x, (galley.size().y + 2.0 * padding.y).max(ui.spacing().interact_size.y));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::CLICK);
    let enabled = ui.is_enabled();
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, enabled, chosen, title));
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        let ink = if chosen {
            visuals.strong_text_color()
        } else if response.hovered() || response.is_pointer_button_down_on() {
            visuals.widgets.hovered.text_color()
        } else {
            visuals.weak_text_color()
        };
        let ring = response.has_focus().then_some((visuals.widgets.inactive.corner_radius, visuals.selection.stroke));
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, ink);
        if let Some((corner, stroke)) = ring {
            ui.painter().rect_stroke(rect, corner, stroke, egui::StrokeKind::Inside);
        }
    }
    response
}

/// Draws a Tabs: its titles in a row as [`tab_title`] draws them, a
/// separator under them, a line [`tokens::TAB_MARK`] thick lying on the
/// separator's line under the chosen title's text in the selection's stroke
/// and under a hovered or pressed title's in the hovered stroke, and the
/// chosen tab's widgets.
fn draw_tabs(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let titles = node
        .children
        .iter()
        .map(|tab| if tab.kind == Kind::Tab { req_str(tab, "title") } else { Err(format!("a Tabs contains only Tab widgets, and this one contains a {:?}", tab.kind)) })
        .collect::<Result<Vec<&str>, String>>()?;
    let current = chosen_row(*kept(&mut inputs.choices, node)?, &titles)?;
    let mut picked = None;
    let r = ui.vertical(|ui| {
        let mut drawn = Vec::with_capacity(titles.len());
        widget_row(ui, |ui| {
            for (i, title) in titles.iter().enumerate() {
                let r = tab_title(ui, title, current == Some(i));
                if r.clicked() && current != Some(i) {
                    picked = Some(i);
                }
                drawn.push(r);
            }
        });
        let line = ui.separator().rect;
        let shown = picked.or(current);
        let bottom = line.center().y + ui.visuals().widgets.noninteractive.bg_stroke.width / 2.0;
        let inset = ui.spacing().button_padding.x;
        for (i, r) in drawn.iter().enumerate() {
            let color = if shown == Some(i) {
                ui.visuals().selection.stroke.color
            } else if r.hovered() || r.is_pointer_button_down_on() {
                ui.visuals().widgets.hovered.bg_stroke.color
            } else {
                continue;
            };
            let mark = egui::Rect::from_x_y_ranges(r.rect.x_range().shrink(inset), (bottom - tokens::TAB_MARK)..=bottom);
            ui.painter().rect_filled(mark, tokens::TAB_MARK / 2.0, color);
        }
        if let Some(shown) = shown {
            draw(ui, &node.children[shown], inputs, window, out);
        }
    });
    if let Some(i) = picked {
        *kept(&mut inputs.choices, node)? = Some(i);
        event(out, window, node.id, EventKind::Change, row_value(i));
    }
    Ok(r.response.rect)
}

/// The width of a split's divider, in points.
const DIVIDER: f32 = 6.0;

fn draw_split(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let [first, second] = node.children.as_slice() else {
        return Err(format!("a Split contains two widgets, and this one contains {}", node.children.len()));
    };
    let vertical = match opt_str(node, "orientation")? {
        None | Some("horizontal") => false,
        Some("vertical") => true,
        Some(other) => return Err(format!("orientation must be horizontal or vertical, not {other}")),
    };
    if ui.is_sizing_pass() {
        // Measured, the two widgets take their own sizes either side of
        // the divider.
        let both = |ui: &mut egui::Ui| {
            draw(ui, first, inputs, window, out);
            ui.add_space(DIVIDER);
            draw(ui, second, inputs, window, out);
        };
        return Ok(if vertical { ui.vertical(both).response.rect } else { ui.horizontal(both).response.rect });
    }
    let full = ui.available_rect_before_wrap();
    if vertical && !full.height().is_finite() {
        return Err(String::from("a vertical Split takes the height the window leaves it, and inside a Scroll that height has no end"));
    }
    let share = *kept(&mut inputs.splits, node)?;
    let extent = if vertical { full.height() } else { full.width() } - DIVIDER;
    let first_extent = (extent * share).max(0.0);
    let (first_rect, second_rect, divider_at) = if vertical {
        let y = full.min.y + first_extent;
        (egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, y)), egui::Rect::from_min_max(egui::pos2(full.min.x, y + DIVIDER), full.max), y)
    } else {
        let x = full.min.x + first_extent;
        (egui::Rect::from_min_max(full.min, egui::pos2(x, full.max.y)), egui::Rect::from_min_max(egui::pos2(x + DIVIDER, full.min.y), full.max), x)
    };
    let pane = |r: egui::Rect| egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Min));
    let mut a = ui.new_child(pane(first_rect));
    draw(&mut a, first, inputs, window, out);
    let mut b = ui.new_child(pane(second_rect));
    draw(&mut b, second, inputs, window, out);
    let used = if vertical { full } else { egui::Rect::from_min_size(full.min, egui::vec2(full.width(), a.min_rect().height().max(b.min_rect().height()))) };
    let divider = if vertical {
        egui::Rect::from_min_size(egui::pos2(full.min.x, divider_at), egui::vec2(full.width(), DIVIDER))
    } else {
        egui::Rect::from_min_size(egui::pos2(divider_at, full.min.y), egui::vec2(DIVIDER, used.height()))
    };
    let cursor = if vertical { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::ResizeHorizontal };
    let drag = ui.interact(divider, ui.scope_id().with(("split", node.id)), egui::Sense::drag()).on_hover_cursor(cursor);
    if drag.dragged()
        && extent > 0.0
        && let Some(at) = drag.interact_pointer_pos()
    {
        let moved = if vertical { at.y - full.min.y } else { at.x - full.min.x } - DIVIDER / 2.0;
        inputs.splits.insert(node.id, (moved / extent).clamp(0.0, 1.0));
    }
    let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
    if vertical {
        ui.painter().hline(divider.x_range(), divider.center().y, stroke);
    } else {
        ui.painter().vline(divider.center().x, divider.y_range(), stroke);
    }
    ui.allocate_rect(used, egui::Sense::hover());
    Ok(used)
}

/// Draws a Scroll's children in an area that scrolls, as high as they are
/// within its `height` where it is given and the room it is given, below
/// the least size egui gives an area that scrolls, 64 points, too; while
/// its window is measured, as wide as they are.
fn draw_scroll(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let mut area = egui::ScrollArea::vertical().id_salt(("scroll", node.id)).auto_shrink([ui.is_sizing_pass(), true]).min_scrolled_height(0.0);
    if let Some(given) = node.props.get("height") {
        let height = finite("height", given)?;
        if height < 0.0 {
            return Err(format!("height must be 0 or more points, not {height}"));
        }
        area = area.max_height(height as f32);
    }
    Ok(ui.scope(|ui| area.show(ui, |ui| node.children.iter().for_each(|c| draw(ui, c, inputs, window, out)))).response.rect)
}

/// What a node's problem is drawn and reported as: its kind, its name or
/// number, and the problem.
fn label(node: &Node, problem: &str) -> String {
    match &node.name {
        Some(name) => format!("{:?} {name}: {problem}", node.kind),
        None => format!("{:?} {}: {problem}", node.kind, node.id),
    }
}

/// Reports a node's problem once, until a patch sets its properties again.
fn report_once(inputs: &mut Inputs, out: &mut Vec<FromHelper>, window: WindowId, node: NodeId, context: &str, text: String) {
    if inputs.reported.insert(node) {
        out.push(FromHelper::Error { window: Some(window), context: context.to_string(), text });
    }
}

/// Every node of `kind` in a subtree, in tree order.
fn nodes_of<'a>(node: &'a Node, kind: Kind, out: &mut Vec<&'a Node>) {
    if node.kind == kind {
        out.push(node);
    }
    for c in &node.children {
        nodes_of(c, kind, out);
    }
}

/// The window's own widgets of `kind`, the root or its children, which is
/// where a menu bar and a status bar stand.
fn own_of(tree: &Node, kind: Kind) -> Vec<&Node> {
    if tree.kind == kind { vec![tree] } else { tree.children.iter().filter(|c| c.kind == kind).collect() }
}

/// The shortcut a menu item acts on: its `shortcut` while it is enabled.
fn item_shortcut(item: &Node) -> Result<Option<&str>, String> {
    if flag(item, "enabled", true)? { opt_str(item, "shortcut") } else { Ok(None) }
}

/// The window's own shortcuts, its root's `shortcuts`.
fn own_shortcuts(tree: &Node) -> Result<Vec<&str>, String> {
    if tree.props.contains_key("shortcuts") { req_strings(tree, "shortcuts") } else { Ok(Vec::new()) }
}

/// A shortcut as egui reads it: its modifiers, Ctrl as Command on macOS,
/// and its key.
fn egui_shortcut(s: &Shortcut) -> Result<(egui::Modifiers, egui::Key), String> {
    let key = egui::Key::from_name(&s.key).ok_or_else(|| format!("egui has no key named {}", s.key))?;
    Ok((egui::Modifiers { alt: s.alt, ctrl: false, shift: s.shift, mac_cmd: false, command: s.ctrl }, key))
}

/// The key one of egui's clipboard events stands for, with the modifiers
/// pressed: egui's window layer delivers Ctrl+C, Ctrl+X and Ctrl+V, and on
/// Windows Shift+Delete and Shift+Insert, as copy, cut and paste in place
/// of key presses, and Ctrl+V only while the clipboard has text.
fn clipboard_key(event: &egui::Event, held: egui::Modifiers) -> Option<egui::Key> {
    match event {
        egui::Event::Copy if held.command => Some(egui::Key::C),
        egui::Event::Cut if held.command => Some(egui::Key::X),
        egui::Event::Cut if held.shift => Some(egui::Key::Delete),
        egui::Event::Paste(_) if held.command => Some(egui::Key::V),
        egui::Event::Paste(_) if held.shift => Some(egui::Key::Insert),
        _ => None,
    }
}

/// Takes from this frame's input every press of `key` with exactly
/// `modifiers` pressed, a key counting as a shortcut counts it (a digit or
/// letter typed as a symbol by the key at its place), a clipboard event
/// standing for one among them, and returns whether there was one.
fn take_press(input: &mut egui::InputState, modifiers: egui::Modifiers, key: egui::Key) -> bool {
    let held = input.modifiers;
    let before = input.events.len();
    input.events.retain(|e| match e {
        egui::Event::Key { key: pressed, physical_key, pressed: true, modifiers: with, .. } => !(routed::counted(*pressed, *physical_key) == key && with.matches_exact(modifiers)),
        other => !(clipboard_key(other, held) == Some(key) && held.matches_exact(modifiers)),
    });
    input.events.len() < before
}

/// Acts on the shortcuts pressed this frame before any widget reads the
/// keys: an enabled menu item's raises its Click, and one of the window's
/// own raises Shortcut on the root, the menu item's where both have the
/// same keys. While a text field has the focus it keeps the keys it edits
/// with ([`Shortcut::field_keeps`]), and a password field Alt+F8, which
/// shows its password ([`Shortcut::is_peek`]). A shortcut that cannot be
/// read is reported once against its node.
fn press_shortcuts(ctx: &egui::Context, tree: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) {
    let in_field = ctx.text_edit_focused();
    let in_password = inputs.focused.and_then(|id| tree.find(id)).is_some_and(Node::is_password);
    let mut wanted: Vec<(&Node, EventKind, &str)> = Vec::new();
    let mut items = Vec::new();
    nodes_of(tree, Kind::MenuItem, &mut items);
    for item in items {
        match item_shortcut(item) {
            Ok(Some(written)) => wanted.push((item, EventKind::Click, written)),
            Ok(None) => {}
            Err(problem) => report_once(inputs, out, window, item.id, "shortcut", label(item, &problem)),
        }
    }
    match own_shortcuts(tree) {
        Ok(own) => wanted.extend(own.into_iter().map(|written| (tree, EventKind::Shortcut, written))),
        Err(problem) => report_once(inputs, out, window, tree.id, "shortcut", label(tree, &problem)),
    }
    for (node, kind, written) in wanted {
        match Shortcut::parse(written).and_then(|s| egui_shortcut(&s).map(|keys| (s, keys))) {
            Ok((s, (modifiers, key))) => {
                if in_field && (s.field_keeps() || (in_password && s.is_peek())) {
                    continue;
                }
                if ctx.input_mut(|i| take_press(i, modifiers, key)) {
                    let value = if kind == EventKind::Shortcut { Value::Str(written.to_string()) } else { Value::Null };
                    event(out, window, node.id, kind, value);
                }
            }
            Err(problem) => report_once(inputs, out, window, node.id, "shortcut", label(node, &problem)),
        }
    }
}

/// A shortcut as a menu shows it: as written, with Ctrl shown as Cmd on
/// macOS, where it stands for the Command key.
fn shown_shortcut(written: &str) -> String {
    match written.strip_prefix("Ctrl+") {
        Some(rest) if cfg!(target_os = "macos") => format!("Cmd+{rest}"),
        _ => written.to_string(),
    }
}

/// Draws the entries of a menu bar or a menu; one that cannot be drawn
/// shows its problem in its place, reported once.
fn menu_entries(ui: &mut egui::Ui, entries: &[Node], inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) {
    for entry in entries {
        if let Err(problem) = menu_entry(ui, entry, inputs, window, out) {
            let text = label(entry, &problem);
            let rect = ui.colored_label(ui.visuals().error_fg_color, &text).rect;
            inputs.rects.insert(entry.id, rect);
            report_once(inputs, out, window, entry.id, "draw", text);
        }
    }
}

/// Draws one entry of a menu bar or a menu: a Menu opens its own entries,
/// and a MenuItem, its shortcut shown beside its text, raises Click and
/// closes the menus it is in.
fn menu_entry(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<(), String> {
    match node.kind {
        Kind::Menu => {
            let title = req_str(node, "title")?;
            let r = ui.menu_button(title, |ui| menu_entries(ui, &node.children, inputs, window, out));
            inputs.rects.insert(node.id, r.response.rect);
        }
        Kind::MenuItem => {
            let text = req_str(node, "text")?;
            let enabled = flag(node, "enabled", true)?;
            let mut item = egui::Button::new(text);
            if let Some(written) = opt_str(node, "shortcut")? {
                item = item.shortcut_text(shown_shortcut(written));
            }
            let r = ui.add_enabled(enabled, item);
            inputs.rects.insert(node.id, r.rect);
            if r.clicked() {
                event(out, window, node.id, EventKind::Click, Value::Null);
                ui.close();
            }
        }
        Kind::Separator => {
            inputs.rects.insert(node.id, ui.separator().rect);
        }
        other => return Err(format!("a menu contains only MenuItem, Menu and Separator widgets, and this one contains a {other:?}")),
    }
    Ok(())
}

/// Opens a context menu where a person right-clicks the widget its `for`
/// names, the innermost such widget where several contain the point, and
/// draws the menu open over the window at the pointer. A choice, a click
/// elsewhere or Escape closes it. `ui` is the window's own, on the layer
/// its widgets are drawn on.
fn context_menus(ui: &mut egui::Ui, tree: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) {
    let mut menus = Vec::new();
    nodes_of(tree, Kind::ContextMenu, &mut menus);
    if menus.is_empty() {
        return;
    }
    let ctx = ui.ctx().clone();
    let layer = ui.layer_id();
    let right_click = ctx.input(|i| if i.pointer.secondary_clicked() { i.pointer.interact_pos() } else { None }).filter(|at| ctx.layer_id_at(*at) == Some(layer));
    let mut opening: Option<(NodeId, f32)> = None;
    for menu in &menus {
        let target = match req_str(menu, "for") {
            Ok(name) => tree.find_named(name),
            Err(problem) => {
                report_once(inputs, out, window, menu.id, "draw", label(menu, &problem));
                None
            }
        };
        if let (Some(at), Some(rect)) = (right_click, target.and_then(|t| inputs.rects.get(&t.id)))
            && rect.contains(at)
            && opening.is_none_or(|(_, area)| rect.area() < area)
        {
            opening = Some((menu.id, rect.area()));
        }
    }
    for menu in menus {
        let open = (opening.map(|(id, _)| id) == Some(menu.id)).then_some(egui::SetOpenCommand::Bool(true));
        let shown = egui::Popup::new(egui::Id::unique(("pwrs-context-menu", window, menu.id)), ctx.clone(), egui::PopupAnchor::PointerFixed, layer)
            .kind(egui::PopupKind::Menu)
            .layout(egui::Layout::top_down_justified(egui::Align::Min))
            .style(egui::containers::menu::menu_style)
            .open_memory(open)
            .show(|ui| menu_entries(ui, &menu.children, inputs, window, out));
        if let Some(r) = shown {
            inputs.rects.insert(menu.id, r.response.rect);
        }
    }
}

/// The state of a widget's input, set up when its node was seeded.
fn kept<'a, T>(map: &'a mut HashMap<NodeId, T>, node: &Node) -> Result<&'a mut T, String> {
    map.get_mut(&node.id).ok_or_else(|| String::from("its input was never set up from its properties"))
}

fn draw_check(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let text = req_str(node, "text")?;
    let enabled = flag(node, "enabled", true)?;
    let take = inputs.take_focus(node.id);
    let checked = kept(&mut inputs.checks, node)?;
    let r = ui.add_enabled(enabled, egui::Checkbox::new(checked, text));
    if take {
        r.request_focus();
    }
    if r.changed() {
        event(out, window, node.id, EventKind::Change, Value::Bool(*checked));
    }
    Ok(r.rect)
}

fn draw_text_box(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let multiline = flag(node, "multiline", false)?;
    let password = flag(node, "password", false)?;
    if password && multiline {
        return Err(String::from("a password field is one line"));
    }
    let placeholder = opt_str(node, "placeholder")?;
    let enabled = flag(node, "enabled", true)?;
    let take = inputs.take_focus(node.id);
    // A password field's peek button stands in the field's right margin, so
    // a press on it is a press on the field, which keeps the focus.
    let offered = password && inputs.peeks.get(&node.id).is_some_and(proto::peek::Peek::offered);
    let pressed = offered && inputs.peek_rects.get(&node.id).is_some_and(|b| ui.input(|i| i.pointer.primary_down() && i.pointer.press_origin().is_some_and(|p| b.contains(p))));
    let alt_f8 = ui.input(|i| i.modifiers.alt && !i.modifiers.ctrl && !i.modifiers.shift && i.key_down(egui::Key::F8));
    let shown = offered && (pressed || alt_f8 || inputs.peek_held.contains(&node.id));
    let mut margin = look::text_edit_margin();
    if offered {
        margin.right = margin.right.saturating_add(tokens::PEEK.button as i8);
    }
    let text = kept(&mut inputs.texts, node)?;
    let mut edit = if multiline { egui::TextEdit::multiline(text) } else { egui::TextEdit::singleline(text) };
    edit = edit.id_salt(("text", node.id)).password(password && !shown).margin(margin);
    if let Some(hint) = placeholder {
        edit = edit.hint_text(hint);
    }
    let r = look::field(ui, |ui| ui.add_enabled(enabled, edit));
    if take {
        r.request_focus();
    }
    if r.changed() {
        event(out, window, node.id, EventKind::Change, Value::Str(text.clone()));
    }
    if !multiline && r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        event(out, window, node.id, EventKind::Submit, Value::Str(text.clone()));
    }
    // The window's own focus, which the window losing the system's keeps.
    let focused = ui.memory(|m| m.has_focus(r.id));
    if let Some(peek) = inputs.peeks.get_mut(&node.id) {
        if peek.update(focused, text) {
            let button = egui::Rect::from_min_max(egui::pos2(r.rect.max.x - tokens::PEEK.button, r.rect.min.y), r.rect.max);
            let color = if shown { ui.visuals().selection.stroke.color } else { ui.visuals().weak_text_color() };
            paint_eye(ui.painter(), button.center(), color);
            inputs.peek_rects.insert(node.id, button);
        } else {
            inputs.peek_rects.remove(&node.id);
            inputs.peek_held.remove(&node.id);
        }
    }
    Ok(r.rect)
}

/// Paints a password field's peek eye [`tokens::PEEK`] whose middle is
/// `center`, in `color`: an almond of two curves around a round pupil.
fn paint_eye(painter: &egui::Painter, center: egui::Pos2, color: egui::Color32) {
    let tokens::PeekButton { eye: [width, height], stroke, pupil, .. } = tokens::PEEK;
    let (left, right) = (center - egui::vec2(width / 2.0, 0.0), center + egui::vec2(width / 2.0, 0.0));
    for bulge in [-height, height] {
        let curve = egui::epaint::QuadraticBezierShape::from_points_stroke([left, center + egui::vec2(0.0, bulge), right], false, egui::Color32::TRANSPARENT, egui::Stroke::new(stroke, color));
        painter.add(curve);
    }
    painter.circle_filled(center, pupil / 2.0, color);
}

fn draw_color(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let enabled = flag(node, "enabled", true)?;
    let take = inputs.take_focus(node.id);
    let color = kept(&mut inputs.colors, node)?;
    let r = ui.add_enabled_ui(enabled, |ui| ui.color_edit_button_srgb(color)).inner;
    if take {
        r.request_focus();
    }
    if r.changed() {
        event(out, window, node.id, EventKind::Change, Value::Str(hex(*color)));
    }
    Ok(r.rect)
}

/// An editable drop-down: a text field, and beside it the list of `items`,
/// one of which picked becomes the text. The field takes the focus
/// Set-GuiFocus gives the drop-down.
fn draw_editable_combo(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let items = req_strings(node, "items")?;
    let enabled = flag(node, "enabled", true)?;
    let take = inputs.take_focus(node.id);
    let text = kept(&mut inputs.texts, node)?;
    let mut changed = false;
    let r = ui.add_enabled_ui(enabled, |ui| {
        ui.horizontal(|ui| {
            let field = look::field(ui, |ui| ui.add(egui::TextEdit::singleline(&mut *text).id_salt(("combo-text", node.id)).margin(look::text_edit_margin())));
            if take {
                field.request_focus();
            }
            if field.changed() {
                changed = true;
            }
            look::well(ui, |ui| {
                egui::ComboBox::from_id_salt(("combo", node.id)).selected_text("").width(ui.spacing().interact_size.y).show_ui(ui, |ui| {
                    for item in &items {
                        if ui.selectable_label(text.as_str() == *item, *item).clicked() && text.as_str() != *item {
                            *text = item.to_string();
                            changed = true;
                        }
                    }
                })
            });
        })
    });
    if changed {
        event(out, window, node.id, EventKind::Change, Value::Str(text.clone()));
    }
    Ok(r.response.rect)
}

fn draw_number(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let (low, high) = bounds(node)?;
    let step = step(node)?;
    let enabled = flag(node, "enabled", true)?;
    let take = inputs.take_focus(node.id);
    let value = kept(&mut inputs.numbers, node)?;
    let r = look::well(ui, |ui| ui.add_enabled(enabled, egui::DragValue::new(value).range(low..=high).speed(step)));
    if take {
        r.request_focus();
    }
    if r.changed() {
        event(out, window, node.id, EventKind::Change, Value::Float(*value));
    }
    Ok(r.rect)
}

fn draw_slider(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let (low, high) = bounds(node)?;
    let step = step(node)?;
    let enabled = flag(node, "enabled", true)?;
    let take = inputs.take_focus(node.id);
    let value = kept(&mut inputs.numbers, node)?;
    let r = ui.add_enabled(enabled, egui::Slider::new(value, low..=high).step_by(step));
    if take {
        r.request_focus();
    }
    if r.changed() {
        event(out, window, node.id, EventKind::Change, Value::Float(*value));
    }
    Ok(r.rect)
}

/// The row a drop-down or option group has chosen, checked against its
/// items.
fn chosen_row(chosen: Option<usize>, items: &[&str]) -> Result<Option<usize>, String> {
    match chosen {
        Some(c) if c >= items.len() => Err(format!("selected is row {c} of {} items", items.len())),
        other => Ok(other),
    }
}

fn draw_combo(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    if flag(node, "editable", false)? {
        return draw_editable_combo(ui, node, inputs, window, out);
    }
    let items = req_strings(node, "items")?;
    let enabled = flag(node, "enabled", true)?;
    let take = inputs.take_focus(node.id);
    let chosen = kept(&mut inputs.choices, node)?;
    let current = chosen_row(*chosen, &items)?;
    let shown = match current {
        Some(c) => items[c],
        None => "",
    };
    let mut picked = None;
    let r = ui.add_enabled_ui(enabled, |ui| {
        let mut combo = egui::ComboBox::from_id_salt(("combo", node.id)).selected_text(shown);
        if ui.layout().horizontal_justify() && !ui.is_sizing_pass() {
            combo = combo.width(ui.available_width());
        }
        look::well(ui, |ui| {
            combo.show_ui(ui, |ui| {
                for (i, item) in items.iter().enumerate() {
                    if ui.selectable_label(current == Some(i), *item).clicked() {
                        picked = Some(i);
                    }
                }
            })
        })
    });
    if take {
        r.inner.response.request_focus();
    }
    if let Some(i) = picked
        && current != Some(i)
    {
        *chosen = Some(i);
        event(out, window, node.id, EventKind::Change, row_value(i));
    }
    Ok(r.inner.response.rect)
}

/// Draws option buttons one under another; the focus Set-GuiFocus gives
/// the group goes to the option chosen, or the first while none is.
fn draw_radio(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let items = req_strings(node, "items")?;
    let enabled = flag(node, "enabled", true)?;
    let take = inputs.take_focus(node.id);
    let chosen = kept(&mut inputs.choices, node)?;
    let current = chosen_row(*chosen, &items)?;
    let focused = current.unwrap_or(0);
    let mut picked = None;
    let r = ui.add_enabled_ui(enabled, |ui| {
        ui.vertical(|ui| {
            for (i, item) in items.iter().enumerate() {
                let option = ui.radio(current == Some(i), *item);
                if take && i == focused {
                    option.request_focus();
                }
                if option.clicked() {
                    picked = Some(i);
                }
            }
        })
    });
    if let Some(i) = picked
        && current != Some(i)
    {
        *chosen = Some(i);
        event(out, window, node.id, EventKind::Change, row_value(i));
    }
    Ok(r.response.rect)
}

/// Draws a list in a frame of the text fields' color, its rows touching and
/// each taking the frame's width: selectable rows, or with `multiple` check
/// boxes, an item too long for the frame cut short. It takes the room it is
/// given and scrolls; while its window is measured, it takes the size of its
/// rows, each item whole, and the room its scroll bar takes beside them. The
/// list takes the focus as one widget, at a click, Tab or Set-GuiFocus, its
/// keyboard then on its first selected row or its first; with the focus,
/// the keys [`rows::row_key`] gives act before the rows are drawn, check
/// boxes moving without checking, and the keyboard's row scrolls into
/// sight, showing a ring while the window has the system's focus.
fn draw_list(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let items = req_strings(node, "items")?;
    let multiple = flag(node, "multiple", false)?;
    let take = inputs.take_focus(node.id);
    let Inputs { lists, list_keys, .. } = inputs;
    let selected = lists.entry(node.id).or_default();
    let keys = list_keys.entry(node.id).or_default();
    let selection = if multiple { Selection::Checks } else { Selection::Single };
    let shown: Vec<usize> = (0..items.len()).collect();
    let list_id = egui::Id::unique(("pwrs-list", window, node.id));
    let start = |keys: &mut ListKeys, selected: &BTreeSet<usize>| {
        if keys.current.is_none_or(|c| c >= items.len()) {
            keys.current = selected.iter().next().copied().or_else(|| shown.first().copied());
        }
    };
    if take {
        ui.memory_mut(|m| m.request_focus(list_id));
        start(keys, selected);
    }
    let focused = ui.memory(|m| m.has_focus(list_id));
    let mut changed = false;
    let mut activated = None;
    let mut scroll_to = None;
    if focused {
        ui.memory_mut(|m| m.set_focus_lock_filter(list_id, egui::EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: false }));
        stay_focused_on_arrows(ui);
        for (key, shift, ctrl) in row_keys(ui) {
            let state = rows::RowState { count: items.len(), selection, current: keys.current, current_selected: keys.current.is_some_and(|c| selected.contains(&c)), anchor: keys.anchor, page: keys.page };
            let Some(m) = rows::row_key(key, shift, ctrl, state) else {
                continue;
            };
            keys.current = Some(m.current);
            keys.anchor = m.anchor;
            changed |= m.select.apply(selected, &shown);
            if m.activate {
                activated = Some(m.current);
            }
            scroll_to = Some(m.current);
        }
    }
    let measuring = ui.is_sizing_pass();
    let wrap = if measuring { egui::TextWrapMode::Extend } else { egui::TextWrapMode::Truncate };
    let mut clicked = None;
    let mut doubled = None;
    let ring = if focused && window_focused(ui) { keys.current } else { None };
    let mut draw_rows = |ui: &mut egui::Ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for (i, item) in items.iter().enumerate() {
            let is = selected.contains(&i);
            let r = if multiple { check_row(ui, is, item, wrap) } else { ui.add(egui::Button::selectable(is, *item).wrap_mode(wrap).sense(egui::Sense::CLICK)) };
            if r.clicked() {
                clicked = Some(i);
            }
            if !multiple && r.double_clicked() {
                doubled = Some(i);
            }
            if ring == Some(i) {
                row_ring(ui.ctx(), ui.layer_id(), r.rect, ui.clip_rect(), ui.visuals().selection.stroke);
            }
            if scroll_to == Some(i) {
                r.scroll_to_me(None);
            }
        }
    };
    let visuals = ui.visuals();
    let frame = egui::Frame::new()
        .fill(visuals.extreme_bg_color)
        .stroke(visuals.widgets.noninteractive.bg_stroke)
        .corner_radius(visuals.widgets.noninteractive.corner_radius)
        .inner_margin(egui::Margin::same(2));
    let bounded = ui.available_height().is_finite();
    let rect = frame
        .show(ui, |ui| {
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                if measuring {
                    draw_rows(ui);
                    ui.set_min_width(ui.min_rect().width() + ui.spacing().scroll.allocated_width());
                } else {
                    egui::ScrollArea::vertical().id_salt(("list", node.id)).auto_shrink([false, !bounded]).min_scrolled_height(0.0).show(ui, |ui| draw_rows(ui));
                }
            })
        })
        .response
        .rect;
    if measuring {
        return Ok(rect);
    }
    if ui.interact(rect, list_id, egui::Sense::focusable_noninteractive()).gained_focus() {
        start(keys, selected);
        ui.ctx().request_repaint();
    }
    keys.page = rows::page_rows(rect.height() - 4.0, ui.spacing().interact_size.y);
    // A click on a check box checks or clears it; one on a row selects it,
    // raising Select only when the row was not selected already.
    if let Some(i) = clicked {
        let select = if multiple { rows::RowSelect::Toggle(i) } else { rows::RowSelect::Only(i) };
        changed |= select.apply(selected, &shown);
        keys.current = Some(i);
        keys.anchor = Some(i);
        ui.memory_mut(|m| m.request_focus(list_id));
        ui.ctx().request_repaint();
    }
    if doubled.is_some() {
        activated = doubled;
    }
    if changed {
        let list = selected.iter().copied().map(row_value).collect();
        event(out, window, node.id, EventKind::Select, Value::List(list));
    }
    if let Some(i) = activated {
        event(out, window, node.id, EventKind::Activate, row_value(i));
    }
    Ok(rect)
}

fn selection_mode(node: &Node) -> Result<Selection, String> {
    match opt_str(node, "selection")? {
        None | Some("none") => Ok(Selection::None),
        Some("single") => Ok(Selection::Single),
        Some("multiple") => Ok(Selection::Multiple),
        Some(other) => Err(format!("selection must be none, single or multiple, not {other}")),
    }
}

/// The arrows a table's header gives the column it is sorted by: up, then
/// down.
const SORT_MARKS: [&str; 2] = [" \u{2191}", " \u{2193}"];

/// The least a table column is across: its heading with the wider of
/// [`SORT_MARKS`], which every column has room for from the start so
/// sorting moves no column, and an interactive widget's least width.
fn header_room(ui: &egui::Ui, label: &str) -> f32 {
    let spacing = ui.spacing();
    let marked = SORT_MARKS.map(|mark| egui::WidgetText::from(format!("{label}{mark}")).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Button).size().x);
    (marked[0].max(marked[1]) + 2.0 * spacing.button_padding.x).max(spacing.interact_size.x)
}

/// A table column's heading: its label and the arrow of the column's sort,
/// if any, in the strong text color, on a button framed only under the
/// pointer.
fn heading(label: &str, arrow: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(format!("{label}{arrow}")).strong()).frame_when_inactive(false)
}

/// Draws a table: its filter row when it has one, a header of headings in
/// the strong text color, each a button framed only under the pointer that
/// sorts by its column, a line under the header, and the rows. Each
/// column's heading and cells are placed across by its `align`, and each
/// column is [`header_room`] across at least. The last takes the room left
/// when its cells stand at the left; otherwise it stays at its own width
/// and an empty column after it takes the room. A person drags the line
/// between two columns to resize the one before it. It takes the room it is
/// given and scrolls, and when Add-GuiItem adds rows while it shows its last
/// row, the last row stays in sight; while its window is measured, it takes
/// the size
/// [`natural_table`] gives it. Its rows take the focus as one widget, at a
/// click or Set-GuiFocus, the filter field being a Tab stop of its own;
/// with the focus, the keys [`rows::row_key`] gives act before the rows are
/// drawn, and the row the keyboard is on scrolls into sight, showing a ring
/// while the window has the system's focus.
fn draw_table(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let mode = selection_mode(node)?;
    let filtered = flag(node, "filter", false)?;
    let take = inputs.take_focus(node.id);
    let table = inputs.tables.entry(node.id).or_default();
    let top = ui.cursor().min;
    if filtered {
        ui.horizontal(|ui| {
            ui.label("Filter");
            if look::field(ui, |ui| ui.text_edit_singleline(&mut table.filter)).changed() {
                table.dirty = true;
            }
            ui.label(format!("{} of {}", table.view.len(), table.rows.len()));
        });
    } else if !table.filter.is_empty() {
        table.filter.clear();
        table.dirty = true;
    }
    table.refresh();
    let row_height = ui.spacing().interact_size.y;
    if ui.is_sizing_pass() {
        let size = natural_table(ui, table, row_height);
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        return Ok(egui::Rect::from_min_max(top, rect.max));
    }
    let rows_id = egui::Id::unique(("pwrs-table-rows", window, node.id));
    if take {
        ui.memory_mut(|m| m.request_focus(rows_id));
        if table.current.is_none_or(|c| !table.view.contains(&c)) {
            table.current = table.view.first().copied();
        }
    }
    let focused = ui.memory(|m| m.has_focus(rows_id));
    let ringed = focused && window_focused(ui);
    let modifiers = ui.input(|i| i.modifiers);
    let mut select_changed = false;
    let mut activated = None;
    let mut scroll_to = None;
    if focused {
        ui.memory_mut(|m| m.set_focus_lock_filter(rows_id, egui::EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: false }));
        stay_focused_on_arrows(ui);
        for (key, shift, ctrl) in row_keys(ui) {
            let at = |row: Option<usize>| row.and_then(|r| table.view.iter().position(|v| *v == r));
            let state = rows::RowState { count: table.view.len(), selection: mode, current: at(table.current), current_selected: table.current.is_some_and(|r| table.selected.contains(&r)), anchor: at(table.anchor), page: table.page };
            let Some(m) = rows::row_key(key, shift, ctrl, state) else {
                continue;
            };
            table.current = table.view.get(m.current).copied();
            table.anchor = m.anchor.and_then(|a| table.view.get(a).copied());
            select_changed |= m.select.apply(&mut table.selected, &table.view);
            if m.activate {
                activated = table.current;
            }
            scroll_to = Some(m.current);
        }
    }
    let mut sort_clicked = None;
    let mut clicked = None;
    let mut header_bottom = top.y;
    let mut header_x = (f32::INFINITY, f32::NEG_INFINITY);
    let ring = ui.visuals().selection.stroke;
    let rooms: Vec<f32> = table.columns.iter().map(|c| header_room(ui, &c.label)).collect();
    // The last column takes the room the table has to spare only when its
    // cells stand at the left, as text does; a column of numbers or of
    // centered values stays at its own width, and an empty column after it
    // takes the room, so its values stay beside the rest of their row.
    let last = table.columns.len().checked_sub(1);
    let filler = last.is_some_and(|i| table.columns[i].placed(table.rows.first().and_then(|r| r.cells.get(i))) != egui::Align::Min);
    let bounded = ui.available_height().is_finite();
    let pitch = row_height + ui.spacing().item_spacing.y;
    // When Add-GuiItem added rows while its last frame showed its last row,
    // the last row comes into sight at once, as a log shows its newest line;
    // rows a person scrolled away from, or the keyboard moves through, stay
    // where they are, as do the rows a grid view's pipeline brings.
    let follow = scroll_to.is_none() && table.at_end && table.added;
    let mut builder = egui_extras::TableBuilder::new(ui)
        .id_salt(("table", node.id))
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::CLICK)
        .auto_shrink([false, !bounded])
        .min_scrolled_height(0.0)
        .max_scroll_height(f32::INFINITY)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
    if let Some(at) = scroll_to {
        builder = builder.scroll_to_row(at, None);
    } else if follow {
        builder = builder.vertical_scroll_offset((table.view.len() as f32 * pitch - table.room).max(0.0));
    }
    for (i, room) in rooms.iter().enumerate() {
        builder = if i + 1 == rooms.len() && !filler {
            builder.column(egui_extras::Column::remainder().at_least(*room))
        } else {
            builder.column(egui_extras::Column::auto().at_least(*room).resizable(true))
        };
    }
    if filler {
        builder = builder.column(egui_extras::Column::remainder());
    }
    let TableState { columns: cols, rows, view, sort, selected, current, .. } = &mut *table;
    let mut row_ids = Vec::new();
    let scrolled = builder
        .header(row_height, |mut header| {
            for (i, column) in cols.iter().enumerate() {
                header.col(|ui| {
                    let cell = ui.max_rect();
                    header_bottom = header_bottom.max(cell.bottom());
                    header_x = (header_x.0.min(cell.left()), header_x.1.max(cell.right()));
                    let arrow = match sort {
                        Some((col, true)) if *col == i => SORT_MARKS[0],
                        Some((col, false)) if *col == i => SORT_MARKS[1],
                        _ => "",
                    };
                    ui.with_layout(cell_layout(measured_at(ui, column.placed(rows.first().and_then(|r| r.cells.get(i))))), |ui| {
                        if ui.add(heading(&column.label, arrow)).clicked() {
                            sort_clicked = Some(i);
                        }
                    });
                });
            }
            if filler {
                header.col(|ui| {
                    let cell = ui.max_rect();
                    header_x = (header_x.0.min(cell.left()), header_x.1.max(cell.right()));
                });
            }
        })
        .body(|body| {
            body.rows(row_height, view.len(), |mut row| {
                let at = row.index();
                let r = view[at];
                row.set_selected(selected.contains(&r));
                // A cell's text is a label that cannot be selected, so a
                // click on it is the row's.
                for (i, cell) in rows[r].cells.iter().enumerate() {
                    let align = cols.get(i).map_or(egui::Align::Min, |c| c.placed(rows.first().and_then(|f| f.cells.get(i))));
                    row.col(|ui| {
                        ui.with_layout(cell_layout(measured_at(ui, align)), |ui| ui.add(egui::Label::new(&cell.text).selectable(false)));
                    });
                }
                if filler {
                    row.col(|_| {});
                }
                let resp = row.response();
                row_ids.push(resp.id);
                if ringed && *current == Some(r) {
                    row_ring(&resp.ctx, resp.layer_id, resp.rect, resp.interact_rect, ring);
                }
                if resp.clicked() {
                    clicked = Some(at);
                }
                if resp.double_clicked() && mode != Selection::None {
                    activated = Some(r);
                }
            });
        });
    // The line under the header stands in the middle of the gap above the
    // rows, as wide as a row's band, which reaches half a gap past the
    // cells at each end.
    if header_x.0 < header_x.1 {
        let gap = ui.spacing().item_spacing;
        let y = ui.painter().round_to_pixel_center(header_bottom + gap.y / 2.0);
        ui.painter().hline(egui::Rangef::new(header_x.0 - gap.x / 2.0, header_x.1 + gap.x / 2.0), y, ui.visuals().widgets.noninteractive.bg_stroke);
    }
    table.added = false;
    table.at_end = scrolled.state.clamped_offset().y + scrolled.inner_rect.height() >= scrolled.content_size.y - 1.0;
    table.room = scrolled.inner_rect.height();
    let bottom = ui.min_rect().max;
    let body = egui::Rect::from_min_max(egui::pos2(top.x, header_bottom), bottom);
    // A press anywhere in the rows' room, below the last row too, gives the
    // rows the focus and selects no row, as a press in Explorer's and
    // Out-GridView's lists does. A press egui's hit test gives to another
    // widget, as a column's edge or the scroll bar, is that widget's alone.
    let pressed = ui.input(|i| if i.pointer.any_pressed() { i.pointer.press_origin() } else { None });
    if let Some(at) = pressed
        && body.intersect(ui.clip_rect()).contains(at)
        && ui.ctx().layer_id_at(at) == Some(ui.layer_id())
        && ui.ctx().viewport(|vp| [vp.hits.click, vp.hits.drag].iter().flatten().all(|w| row_ids.contains(&w.id)))
    {
        ui.memory_mut(|m| m.request_focus(rows_id));
    }
    if ui.interact(body, rows_id, egui::Sense::focusable_noninteractive()).gained_focus() && table.current.is_none_or(|c| !table.view.contains(&c)) {
        table.current = table.view.first().copied();
        ui.ctx().request_repaint();
    }
    table.page = rows::page_rows(body.height(), pitch);
    // Ctrl+C, egui's Copy, copies the rows selected while the rows have the
    // focus, as Out-GridView copies its own; with none selected, nothing.
    if focused && ui.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Copy))) {
        let chosen: Vec<Vec<&str>> = table.view.iter().filter(|r| table.selected.contains(r)).map(|r| table.rows[*r].cells.iter().map(|c| c.text.as_str()).collect()).collect();
        let text = rows::copied(&chosen, rows::NEWLINE);
        if !text.is_empty() {
            ui.ctx().copy_text(text);
        }
    }
    // A click selects its row, with -Selection Multiple Ctrl adding or
    // taking it away and Shift taking the range from the anchor, which
    // stays; it raises Select even where the selection stays the same.
    if let Some(at) = clicked {
        let r = table.view[at];
        let ranged = mode == Selection::Multiple && modifiers.shift && table.anchor.is_some();
        if mode != Selection::None {
            let select = if mode == Selection::Multiple && modifiers.command {
                rows::RowSelect::Toggle(at)
            } else if ranged {
                rows::RowSelect::Range(table.anchor.and_then(|a| table.view.iter().position(|v| *v == a)).unwrap_or(at), at)
            } else {
                rows::RowSelect::Only(at)
            };
            select.apply(&mut table.selected, &table.view);
            select_changed = true;
        }
        if !ranged {
            table.anchor = Some(r);
        }
        table.current = Some(r);
        ui.memory_mut(|m| m.request_focus(rows_id));
        ui.ctx().request_repaint();
    }
    if let Some(i) = sort_clicked {
        table.sort = match table.sort {
            Some((col, true)) if col == i => Some((i, false)),
            _ => Some((i, true)),
        };
        table.dirty = true;
    }
    if select_changed {
        let list = table.selected.iter().copied().map(row_value).collect();
        event(out, window, node.id, EventKind::Select, Value::List(list));
    }
    if let Some(r) = activated {
        event(out, window, node.id, EventKind::Activate, row_value(r));
    }
    Ok(egui::Rect::from_min_max(top, bottom))
}

/// The size a table's columns and rows take below its filter row, if it has
/// one: each column as wide as its widest cell or its [`header_room`], as
/// the table draws it, and the room its scroll bar takes beside them; down,
/// the header and every row, each row a gap below the one before, as
/// egui_extras lays them out.
fn natural_table(ui: &egui::Ui, table: &TableState, row_height: f32) -> egui::Vec2 {
    let width_of = |text: &str| egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body).size().x;
    let spacing = ui.spacing();
    let columns: f32 = table
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let cells = table.rows.iter().filter_map(|r| r.cells.get(i)).map(|cell| width_of(&cell.text)).fold(0.0, f32::max);
            header_room(ui, &c.label).max(cells) + spacing.item_spacing.x
        })
        .sum();
    let rows = table.rows.len() as f32;
    egui::vec2(columns + spacing.scroll.allocated_width(), (rows + 1.0) * row_height + rows * spacing.item_spacing.y)
}

/// A tree item's node id as a protocol value.
fn item_value(id: NodeId) -> Value {
    Value::Int(i64::from(id))
}

/// A tree item's node id from a protocol value.
fn item_id(v: &Value) -> Result<NodeId, String> {
    let i = v.as_int().ok_or_else(|| format!("{v:?} is not a tree item's node"))?;
    NodeId::try_from(i).map_err(|e| format!("{i} is not a tree item's node: {e}"))
}

/// Fails unless every node under a tree is a tree item with its text.
fn check_items(items: &[Node]) -> Result<(), String> {
    for item in items {
        if item.kind != Kind::TreeItem {
            return Err(format!("a TreeView contains only TreeItem nodes, and node {} is a {:?}", item.id, item.kind));
        }
        req_str(item, "text").map_err(|e| format!("tree item {}: {e}", item.id))?;
        flag(item, "more", false).map_err(|e| format!("tree item {}: {e}", item.id))?;
        check_items(&item.children)?;
    }
    Ok(())
}

/// A tree's items as shown, top to bottom, each with its depth: an item,
/// then, while it is open, the items inside it.
fn shown_items<'a>(items: &'a [Node], open: &HashSet<NodeId>, depth: usize, out: &mut Vec<(&'a Node, usize)>) {
    for item in items {
        out.push((item, depth));
        if open.contains(&item.id) {
            shown_items(&item.children, open, depth + 1, out);
        }
    }
}

/// Every item of a tree in tree order, open or not.
fn tree_order(items: &[Node], out: &mut Vec<NodeId>) {
    for item in items {
        out.push(item.id);
        tree_order(&item.children, out);
    }
}

/// A tree's selection as its Select event gives it: the items' node ids
/// in tree order.
fn tree_selection(tree: &Node, selected: &BTreeSet<NodeId>) -> Value {
    let mut order = Vec::new();
    tree_order(&tree.children, &mut order);
    Value::List(order.into_iter().filter(|id| selected.contains(id)).map(item_value).collect())
}

/// Whether a tree item opens: it has children, or more to come.
fn opens(item: &Node) -> bool {
    !item.children.is_empty() || matches!(item.props.get("more"), Some(Value::Bool(true)))
}

/// Whether a tree item's children are still to come from the session.
fn lazy(item: &Node) -> bool {
    item.children.is_empty() && matches!(item.props.get("more"), Some(Value::Bool(true)))
}

/// Opens or closes a tree item as a person does. Opening one whose children
/// are still to come raises Expand on the tree, once, and the item waits
/// for them.
fn toggle_item(item: &Node, tree: NodeId, open_items: &mut HashSet<NodeId>, expanding: &mut HashSet<NodeId>, window: WindowId, out: &mut Vec<FromHelper>) {
    if open_items.remove(&item.id) {
        return;
    }
    open_items.insert(item.id);
    if lazy(item) && expanding.insert(item.id) {
        event(out, window, tree, EventKind::Expand, item_value(item.id));
    }
}

/// Selects the item on row `at` of the rows shown, `ids`, as a click with
/// `modifiers` pressed does: alone; in a tree of several, Ctrl adds it or
/// takes it away, and Shift selects the rows from the anchor's to it, as
/// shown. Returns whether the selection changed.
fn select_row(state: &mut TreeState, ids: &[NodeId], at: usize, multiple: bool, modifiers: egui::Modifiers) -> bool {
    let id = ids[at];
    let before = state.selected.clone();
    if multiple && modifiers.command {
        if !state.selected.remove(&id) {
            state.selected.insert(id);
        }
        state.anchor = Some(id);
    } else if multiple && modifiers.shift {
        let from = match state.anchor.and_then(|a| ids.iter().position(|i| *i == a)) {
            Some(p) => p,
            None => at,
        };
        let (lo, hi) = if from <= at { (from, at) } else { (at, from) };
        state.selected = ids[lo..=hi].iter().copied().collect();
    } else {
        state.selected = BTreeSet::from([id]);
        state.anchor = Some(id);
    }
    state.focus = Some(id);
    state.selected != before
}

/// Paints a chevron [`tokens::CHEVRON`] whose middle is `center`, in
/// `color`, pointing down when `open` and right when not.
fn paint_chevron(painter: &egui::Painter, center: egui::Pos2, open: bool, color: egui::Color32) {
    let tokens::Chevron { span, stroke } = tokens::CHEVRON;
    let (half, depth) = (span / 2.0, span / 4.0);
    let points = if open {
        vec![center + egui::vec2(-half, -depth), center + egui::vec2(0.0, depth), center + egui::vec2(half, -depth)]
    } else {
        vec![center + egui::vec2(-depth, -half), center + egui::vec2(depth, 0.0), center + egui::vec2(-depth, half)]
    };
    painter.add(egui::Shape::line(points, egui::Stroke::new(stroke, color)));
}

/// Draws a tree: a row per item shown, indented by its depth, a chevron
/// before each item that opens, its text ending in an ellipsis where the
/// tree is too narrow for it, and a spinner after one waiting for its
/// children. The chevron, in the weak text color or a selected row's text
/// color, stands in the middle of an icon's box that with the icon's gap
/// after it is as wide as a level's indent, so an item's chevron stands
/// under its parent's text, as Fluent's and macOS's trees place it. A click
/// selects as [`select_row`] says, the chevron's box and gap open or close,
/// a double click activates. The tree takes the focus as one widget, at a
/// click or Set-GuiFocus, on its first row when no row has the keyboard;
/// with the focus, the keys [`rows::row_key`] gives move and select, Right
/// opens an item or moves into an open one, and Left closes an open item or
/// moves to its parent, the keyboard's row scrolling into sight, its ring
/// showing while the window has the system's focus. It takes the room it
/// is given and scrolls; while its window is measured, it takes the size
/// of its rows.
fn draw_tree(ui: &mut egui::Ui, node: &Node, inputs: &mut Inputs, window: WindowId, out: &mut Vec<FromHelper>) -> Result<egui::Rect, String> {
    let multiple = flag(node, "multiple", false)?;
    check_items(&node.children)?;
    let take = inputs.take_focus(node.id);
    let Inputs { trees, open_items, expanding, .. } = inputs;
    let state = trees.entry(node.id).or_default();
    let mut shown = Vec::new();
    shown_items(&node.children, open_items, 0, &mut shown);
    let rows: Vec<(&Node, usize, &str)> = shown.into_iter().map(|(item, depth)| req_str(item, "text").map(|text| (item, depth, text))).collect::<Result<_, String>>()?;
    let ids: Vec<NodeId> = rows.iter().map(|(item, _, _)| item.id).collect();
    let tree_id = egui::Id::unique(("pwrs-tree", window, node.id));
    let focused = ui.memory(|m| m.has_focus(tree_id));
    let ringed = focused && window_focused(ui);
    let row_height = ui.spacing().interact_size.y;
    let pitch = row_height + ui.spacing().item_spacing.y;
    let indent = ui.spacing().indent;
    let gap = ui.spacing().item_spacing.x;
    let icon = ui.spacing().icon_width;
    let slot = icon + ui.spacing().icon_spacing;
    let modifiers = ui.input(|i| i.modifiers);
    let scroll_to = if std::mem::take(&mut state.scroll) { state.focus.and_then(|f| ids.iter().position(|i| *i == f)) } else { None };
    let mut clicked = None;
    let mut toggled = None;
    let mut activated = None;
    let measuring = ui.is_sizing_pass();
    let bounded = ui.available_height().is_finite();
    let area = ui.scope(|ui| egui::ScrollArea::vertical().id_salt(("tree", node.id)).auto_shrink([measuring, measuring || !bounded]).min_scrolled_height(0.0).show_rows(ui, row_height, rows.len(), |ui, range| {
        // The rows from `range.start` are drawn from the top of this ui, so
        // the keyboard's row is found by its place even when out of sight.
        if let Some(at) = scroll_to {
            let top = ui.max_rect().top() + (at as f32 - range.start as f32) * pitch;
            ui.scroll_to_rect(egui::Rect::from_min_size(egui::pos2(ui.max_rect().left(), top), egui::vec2(1.0, row_height)), None);
        }
        for at in range {
            let (item, depth, text) = rows[at];
            // Measured, a row is as wide as its chevron's slot and its text;
            // drawn, it spans the tree, so its selection fills the row.
            let width = if measuring {
                let text_width = ui.painter().layout_no_wrap(text.to_owned(), egui::TextStyle::Body.resolve(ui.style()), ui.visuals().text_color()).size().x;
                indent * depth as f32 + slot + text_width
            } else {
                ui.available_width()
            };
            let (rect, _) = ui.allocate_exact_size(egui::vec2(width, row_height), egui::Sense::hover());
            let row = ui.interact(rect, egui::Id::unique(("pwrs-tree-row", window, item.id)), egui::Sense::CLICK);
            let selected = state.selected.contains(&item.id);
            let visuals = ui.style().interact_selectable(&row, selected);
            if selected || row.hovered() {
                ui.painter().rect_filled(rect, visuals.corner_radius, visuals.weak_bg_fill);
            }
            if ringed && state.focus == Some(item.id) {
                ui.painter().rect_stroke(rect, visuals.corner_radius, ui.visuals().selection.stroke, egui::StrokeKind::Inside);
            }
            let arrow = egui::Rect::from_min_size(egui::pos2(rect.left() + indent * depth as f32, rect.top()), egui::vec2(slot, row_height));
            if opens(item) {
                let open = ui.interact(arrow, egui::Id::unique(("pwrs-tree-open", window, item.id)), egui::Sense::CLICK);
                let ink = if selected { visuals.text_color() } else { ui.visuals().weak_text_color() };
                paint_chevron(ui.painter(), egui::pos2(arrow.left() + icon / 2.0, arrow.center().y), open_items.contains(&item.id), ink);
                if open.clicked() {
                    toggled = Some(at);
                }
            }
            // Text too long for the row ends in an ellipsis, as a list's
            // item does; a row measured is as wide as its whole text.
            let name = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Truncate), rect.right() - arrow.right(), egui::TextStyle::Body);
            let written = egui::Rect::from_min_size(egui::pos2(arrow.right(), rect.center().y - name.size().y / 2.0), name.size());
            ui.painter_at(rect).galley(written.min, name, visuals.text_color());
            if expanding.contains(&item.id) {
                egui::Spinner::new().paint_at(ui, egui::Rect::from_min_size(egui::pos2(written.right() + gap, rect.top()), egui::vec2(row_height, row_height)));
            }
            if row.clicked() {
                clicked = Some(at);
            }
            if row.double_clicked() {
                activated = Some(item.id);
            }
        }
        if measuring {
            ui.set_min_width(ui.min_rect().width() + ui.spacing().scroll.allocated_width());
        }
    }));
    let tree = ui.interact(area.response.rect, tree_id, egui::Sense::focusable_noninteractive());
    if take {
        tree.request_focus();
    }
    if (take || tree.gained_focus()) && state.focus.is_none() {
        state.focus = ids.first().copied();
    }
    state.page = rows::page_rows(area.response.rect.height(), pitch);
    let mut changed = false;
    if let Some(at) = toggled {
        toggle_item(rows[at].0, node.id, open_items, expanding, window, out);
        ui.memory_mut(|m| m.request_focus(tree_id));
    }
    if let Some(at) = clicked {
        changed |= select_row(state, &ids, at, multiple, modifiers);
        ui.memory_mut(|m| m.request_focus(tree_id));
    }
    if focused && !rows.is_empty() {
        ui.memory_mut(|m| m.set_focus_lock_filter(tree_id, egui::EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: false }));
        stay_focused_on_arrows(ui);
        let selection = if multiple { Selection::Multiple } else { Selection::Single };
        let place = |id: Option<NodeId>| id.and_then(|f| ids.iter().position(|i| *i == f));
        for (key, shift, ctrl) in row_keys(ui) {
            let at = rows::RowState { count: ids.len(), selection, current: place(state.focus), current_selected: state.focus.is_some_and(|f| state.selected.contains(&f)), anchor: place(state.anchor), page: state.page };
            let Some(m) = rows::row_key(key, shift, ctrl, at) else {
                continue;
            };
            state.focus = ids.get(m.current).copied();
            state.anchor = m.anchor.and_then(|a| ids.get(a).copied());
            changed |= m.select.apply(&mut state.selected, &ids);
            if m.activate {
                activated = state.focus;
            }
            state.scroll = true;
        }
        let pressed = |key: egui::Key| ui.input(|i| i.key_pressed(key) && !i.modifiers.alt);
        if let Some(a) = place(state.focus) {
            let (item, depth, _) = rows[a];
            let last = rows.len() - 1;
            if pressed(egui::Key::ArrowRight) {
                if opens(item) && !open_items.contains(&item.id) {
                    toggle_item(item, node.id, open_items, expanding, window, out);
                } else if a < last && rows[a + 1].1 > depth {
                    changed |= select_row(state, &ids, a + 1, multiple, egui::Modifiers::NONE);
                    state.scroll = true;
                }
            } else if pressed(egui::Key::ArrowLeft) {
                if opens(item) && open_items.contains(&item.id) {
                    toggle_item(item, node.id, open_items, expanding, window, out);
                } else if let Some(parent) = (0..a).rev().find(|r| depth > 0 && rows[*r].1 == depth - 1) {
                    changed |= select_row(state, &ids, parent, multiple, egui::Modifiers::NONE);
                    state.scroll = true;
                }
            }
        }
        if state.scroll {
            ui.ctx().request_repaint();
        }
    }
    if changed {
        event(out, window, node.id, EventKind::Select, tree_selection(node, &state.selected));
    }
    if let Some(id) = activated {
        event(out, window, node.id, EventKind::Activate, item_value(id));
    }
    Ok(area.response.rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_numbers_are_sorted_and_deduplicated() {
        let v = Value::List(vec![Value::Int(3), Value::Int(1), Value::Int(3)]);
        assert_eq!(rows_of(&v).expect("valid rows"), vec![1, 3]);
        assert_eq!(rows_of(&Value::Int(2)).expect("one row"), vec![2]);
    }

    /// Two errors the device raises outside any call in every frame, which
    /// alternate as a broken pipeline's do, are reported once each for as
    /// long as the layer has the device, and again on a new one.
    #[test]
    fn an_error_raised_in_every_frame_is_reported_once_on_a_device() {
        let gpu = pwrs_gui_gpu::Gpu::open(&pwrs_gui_gpu::GpuOptions::default()).expect("a device opens");
        let layer = pwrs_gui_gpu::Layer::adopt(gpu.adapter().clone(), gpu.device().clone(), gpu.queue().clone(), "PWRS-GUI report test").expect("the layer takes the device");
        let target = gpu.device().create_buffer(&wgpu::BufferDescriptor { label: Some("target"), size: 16, usage: wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let frame = || {
            for offset in [1, 2] {
                gpu.queue().write_buffer(&target, offset, &[0; 4]);
            }
            layer.take_errors()
        };
        let mut reported = Reported::default();
        let mut shown = Vec::new();
        for _ in 0..3 {
            let raised = frame();
            assert_eq!(raised.len(), 2, "each frame raises both: {raised:?}");
            shown.extend(raised.into_iter().filter(|text| reported.first(text)));
        }
        assert_eq!(shown.len(), 2, "each text once over three frames: {shown:?}");
        assert_ne!(shown[0], shown[1], "the two texts differ");
        reported.forget();
        assert_eq!(frame().into_iter().filter(|text| reported.first(text)).count(), 2, "a new device reports both again");
    }

    /// The row heights a page is given are epaint's own: for the fonts the
    /// build includes, egui's and the system's text is drawn in, at every
    /// size of text the tokens name, proto reads the height epaint lays a
    /// row out at.
    #[test]
    fn proto_reads_each_fonts_row_height_as_epaint_lays_text_out() {
        let found = system_fonts(proto::Fonts::Bundled);
        let t = &proto::tokens::TEXT;
        let drawn = ["bundled-ui", "bundled-mono", "Ubuntu-Light", "Hack", "system-ui", "system-mono"];
        let faces: Vec<(&String, &Arc<egui::FontData>)> = found.definitions.font_data.iter().filter(|(name, _)| drawn.contains(&name.as_str())).collect();
        assert!(faces.len() >= 4, "the build's fonts and egui's own are among those text is drawn in: {:?}", found.definitions.font_data.keys().collect::<Vec<_>>());
        for (name, data) in faces {
            let mut fonts = egui::FontDefinitions::empty();
            fonts.font_data.insert(name.clone(), Arc::clone(data));
            fonts.families.insert(egui::FontFamily::Proportional, vec![name.clone()]);
            let ctx = egui::Context::default();
            ctx.set_fonts(fonts);
            ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
            for size in [t.small, t.body, t.button, t.heading, t.monospace, proto::tokens::DIALOG.instruction] {
                let epaint = ctx.fonts_mut(|f| f.row_height(&egui::FontId::proportional(size)));
                let read = proto::typeface::row_height(data.bytes(), data.index, size).expect("proto reads the face's metrics");
                assert_eq!(read, epaint, "{name} at {size} points");
            }
        }
    }

    #[test]
    fn a_value_that_is_not_a_row_number_is_refused() {
        assert!(rows_of(&Value::List(vec![Value::Int(1), Value::Str("x".into())])).is_err());
        assert!(rows_of(&Value::Int(-1)).is_err());
        assert!(rows_of(&Value::Null).is_err());
    }

    #[test]
    fn a_property_of_the_wrong_type_is_an_error_and_a_missing_one_takes_its_default() {
        let n = Node::new(1, Kind::ListBox).prop("items", Value::Str("a".into())).prop("multiple", Value::Bool(true));
        assert!(req_strings(&n, "items").is_err());
        assert!(flag(&n, "multiple", false).expect("a flag"));
        assert!(flag(&n, "enabled", true).expect("a missing flag takes its default"));
        assert!(req_str(&n, "text").is_err());
    }

    #[test]
    fn numbers_sort_before_text_and_empty_cells_last() {
        use std::cmp::Ordering;
        let n = SortKey::Number(2.0);
        let t = SortKey::Text("b".into());
        let e = SortKey::Empty;
        assert_eq!(compare_keys(Some(&n), Some(&t)), Ordering::Less);
        assert_eq!(compare_keys(Some(&t), Some(&e)), Ordering::Less);
        assert_eq!(compare_keys(Some(&SortKey::Number(f64::NAN)), Some(&n)), Ordering::Greater);
    }

    #[test]
    fn a_table_filters_without_regard_to_case_and_sorts_by_column() {
        let row = |a: &str, n: f64| proto::Row {
            cells: vec![proto::Cell { text: a.into(), sort: SortKey::Text(a.into()) }, proto::Cell { text: n.to_string(), sort: SortKey::Number(n) }],
        };
        let mut t = TableState { rows: vec![row("Beta", 2.0), row("alpha", 3.0), row("Gamma", 1.0)], dirty: true, ..Default::default() };
        t.sort = Some((1, true));
        t.refresh();
        assert_eq!(t.view, vec![2, 0, 1]);
        t.filter = "A".into();
        t.dirty = true;
        t.refresh();
        assert_eq!(t.view, vec![2, 0, 1]);
        t.filter = "ph".into();
        t.dirty = true;
        t.refresh();
        assert_eq!(t.view, vec![1]);
    }

    /// A column of dates sorts by when they are, not by their text, in which
    /// 10:01 AM comes before 9:39 AM, and stands at the left as a label does.
    #[test]
    fn a_date_column_sorts_by_time_and_stands_at_the_left() {
        let row = |text: &str, ticks: f64| proto::Row { cells: vec![proto::Cell { text: text.into(), sort: SortKey::Date(ticks) }] };
        // The [datetime] values' Ticks, which a table's cell sorts a date by.
        let mut t = TableState { rows: vec![row("10/20/2026 10:01:00 AM", 639_280_872_600_000_000.0), row("10/20/2026 9:39:55 AM", 639_280_859_950_000_000.0)], dirty: true, ..Default::default() };
        t.sort = Some((0, true));
        t.refresh();
        assert_eq!(t.view, vec![1, 0], "9:39 AM comes first");
        let time = TableColumn { label: String::from("Time"), align: None };
        assert_eq!(time.placed(t.rows[0].cells.first()), egui::Align::Min);
        assert_eq!(compare_keys(Some(&SortKey::Date(1.0)), Some(&SortKey::Text(String::from("a")))), std::cmp::Ordering::Less, "a date sorts before text, as a number does");
    }

    #[test]
    fn columns_come_from_the_table_property_with_where_their_cells_sit() {
        let col = |l: &str| Value::Map(BTreeMap::from([(String::from("label"), Value::Str(l.into()))]));
        let right = Value::Map(BTreeMap::from([(String::from("label"), Value::Str("Id".into())), (String::from("align"), Value::Str("right".into()))]));
        let n = Node::new(1, Kind::Table).prop("columns", Value::List(vec![col("Name"), right]));
        let columns = columns_of(&n).expect("two columns");
        assert_eq!(columns.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["Name", "Id"]);
        assert_eq!(columns.iter().map(|c| c.align).collect::<Vec<_>>(), [None, Some(egui::Align::Max)], "none when absent, right as given");
        let number = proto::Cell { text: String::from("12"), sort: SortKey::Number(12.0) };
        let word = proto::Cell { text: String::from("web"), sort: SortKey::Text(String::from("web")) };
        assert_eq!(columns[0].placed(Some(&number)), egui::Align::Max, "a number column without an align is right-aligned");
        assert_eq!(columns[0].placed(Some(&word)), egui::Align::Min);
        assert_eq!(columns[1].placed(Some(&word)), egui::Align::Max, "an align given wins");
        let bad = Node::new(2, Kind::Table).prop("columns", Value::List(vec![Value::Str("Name".into())]));
        assert!(columns_of(&bad).is_err());
        let sideways = Value::Map(BTreeMap::from([(String::from("label"), Value::Str("Id".into())), (String::from("align"), Value::Str("up".into()))]));
        assert!(columns_of(&Node::new(3, Kind::Table).prop("columns", Value::List(vec![sideways]))).is_err());
    }

    /// A Column shares its room among the widgets that fill down after a
    /// gap for each widget but the last and a Spacer, which adds its room
    /// alone: a dialog fitted to a prompt, a Spacer and a list draws the list
    /// at the height it was measured at, every item whole.
    #[test]
    fn a_fitted_dialog_draws_a_list_after_a_spacer_at_the_height_it_was_measured_at() {
        let text = |t: &str| Value::Str(String::from(t));
        let items = Value::List((1..=25).map(|i| Value::Str(format!("choice number {i}"))).collect());
        let ok = Node::new(6, Kind::Button).prop("text", text("OK")).prop("default", Value::Bool(true));
        let tree = Node::new(1, Kind::Column)
            .prop("align", text("stretch"))
            .child(Node::new(2, Kind::Text).prop("text", text("Pick one of many")).prop("style", text("instruction")))
            .child(Node::new(3, Kind::Spacer))
            .child(Node::new(4, Kind::ListBox).prop("items", items))
            .child(Node::new(5, Kind::ButtonBar).child(ok));
        let options = proto::WindowOptions { fit: proto::Fit::DIALOG, ..proto::WindowOptions::titled("Pick") };
        let mut w = WindowState::new(1, String::from("ring"), options, tree);
        let work = proto::ASSUMED_WORK_AREA;
        let content = Measure::new().content(&mut w, egui::FontDefinitions::default(), 1.0, work);
        let measured = w.inputs.rects.get(&4).expect("the list is measured").height();
        let [width, height] = w.options.fitted(content, Some(work));
        let ctx = egui::Context::default();
        look::apply(&ctx);
        let mut closed = false;
        for _ in 0..2 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))), ..Default::default() };
            ctx.run_ui(input, |ui| closed |= w.frame(ui).closed).drop_without_applying_deltas();
        }
        assert!(!closed, "the dialog stays open");
        let drawn = w.inputs.rects.get(&4).expect("the list is drawn").height();
        assert!((drawn - measured).abs() < 0.5, "the list was measured {measured} points high and drawn {drawn} high in a {width} by {height} window");
    }

    /// A list that fills down takes the share its Column gives it, below the
    /// least height egui gives an area that scrolls, 64 points, too: a
    /// window fitted to a line, a list of three items and a button draws the
    /// list at the height it was measured at and the button inside the
    /// window's margin.
    #[test]
    fn a_fitted_window_draws_a_short_list_at_the_height_it_was_measured_at() {
        let text = |t: &str| Value::Str(String::from(t));
        let items = Value::List(["alpha", "beta", "gamma"].into_iter().map(text).collect());
        let tree = Node::new(1, Kind::Column)
            .child(Node::new(2, Kind::Text).prop("text", text("Pick one of these")))
            .child(Node::new(3, Kind::ListBox).prop("items", items))
            .child(Node::new(4, Kind::Row).child(Node::new(5, Kind::Button).prop("text", text("OK"))));
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Pick"), tree);
        let work = proto::ASSUMED_WORK_AREA;
        let content = Measure::new().content(&mut w, egui::FontDefinitions::default(), 1.0, work);
        let measured = w.inputs.rects.get(&3).expect("the list is measured").height();
        let [width, height] = w.options.fitted(content, Some(work));
        let ctx = egui::Context::default();
        look::apply(&ctx);
        let mut closed = false;
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))), ..Default::default() };
            ctx.run_ui(input, |ui| closed |= w.frame(ui).closed).drop_without_applying_deltas();
        }
        assert!(!closed, "the window stays open");
        let drawn = w.inputs.rects.get(&3).expect("the list is drawn").height();
        assert!(measured < 64.0 + 6.0, "a list of three items measures under egui's least scrolled height and the list's frame, 70 points, not {measured}");
        assert!((drawn - measured).abs() < 0.5, "the list was measured {measured} points high and drawn {drawn} high in a {width} by {height} window");
        let inside = height - f32::from(tokens::SPACING.panel_margin);
        let bottom = w.inputs.rects.get(&5).expect("the button is drawn").max.y;
        assert!(bottom <= inside + 0.5, "the button ends at {bottom}, past the window's margin at {inside}");
    }

    /// A Scroll given a height takes that height, below the least height
    /// egui gives an area that scrolls, 64 points, too, as the page's
    /// scroll area takes it as its greatest height.
    #[test]
    fn a_scroll_given_a_height_under_64_points_is_drawn_that_high() {
        let text = |t: &str| Value::Str(String::from(t));
        let mut lines = Node::new(2, Kind::Scroll).prop("height", Value::Float(40.0));
        for at in 0..10 {
            lines = lines.child(Node::new(3 + at, Kind::Text).prop("text", text(&format!("line {at}"))));
        }
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Lines"), Node::new(1, Kind::Column).child(lines));
        let ctx = egui::Context::default();
        look::apply(&ctx);
        let mut closed = false;
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0))), ..Default::default() };
            ctx.run_ui(input, |ui| closed |= w.frame(ui).closed).drop_without_applying_deltas();
        }
        assert!(!closed, "the window stays open");
        let drawn = w.inputs.rects.get(&2).expect("the scroll area is drawn").height();
        assert!((drawn - 40.0).abs() < 0.5, "the scroll area given 40 points is drawn {drawn} high");
    }

    /// A Row counts a gap after each widget but the last and a Spacer, which
    /// adds its room alone, so a Spacer that fills ends the widget after it
    /// at the Row's right edge, and a Row placed at the end ends its last
    /// widget there across a Spacer of its own size.
    #[test]
    fn a_row_ends_its_last_widget_at_its_right_edge_across_a_spacer() {
        let text = |t: &str| Value::Str(String::from(t));
        let tree = Node::new(1, Kind::Column)
            .child(Node::new(2, Kind::Separator))
            .child(
                Node::new(3, Kind::Row)
                    .child(Node::new(4, Kind::Text).prop("text", text("Step 1 of 3")))
                    .child(Node::new(5, Kind::Spacer).prop("fill", Value::Bool(true)))
                    .child(Node::new(6, Kind::Button).prop("text", text("Next"))),
            )
            .child(
                Node::new(7, Kind::Row)
                    .prop("align", text("end"))
                    .child(Node::new(8, Kind::Button).prop("text", text("Back")))
                    .child(Node::new(9, Kind::Spacer))
                    .child(Node::new(10, Kind::Button).prop("text", text("Finish"))),
            );
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Steps"), tree);
        let ctx = egui::Context::default();
        look::apply(&ctx);
        let mut closed = false;
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 240.0))), ..Default::default() };
            ctx.run_ui(input, |ui| closed |= w.frame(ui).closed).drop_without_applying_deltas();
        }
        assert!(!closed, "the window stays open");
        let edge = w.inputs.rects.get(&2).expect("the separator is drawn").max.x;
        for id in [6, 10] {
            let end = w.inputs.rects.get(&id).expect("the row's last button is drawn").max.x;
            assert!((end - edge).abs() < 0.5, "the last button of row {id} ends at {end}, and the room the column gives at {edge}");
        }
    }

    /// The widgets that fill down take equal shares of a Column's height,
    /// whatever their rows: a list, a table under its filter row, whose
    /// scrolled body takes what its share leaves, and a tree of one item, all
    /// inside the window.
    #[test]
    fn a_list_a_table_and_a_tree_take_equal_shares_of_a_column() {
        let text = |t: &str| Value::Str(String::from(t));
        let col = |l: &str| Value::Map(BTreeMap::from([(String::from("label"), text(l))]));
        let tree = Node::new(1, Kind::Column)
            .child(Node::new(2, Kind::ListBox).prop("items", Value::List(vec![text("alpha"), text("beta")])))
            .child(Node::new(3, Kind::Table).prop("columns", Value::List(vec![col("Name"), col("Size")])).prop("filter", Value::Bool(true)))
            .child(Node::new(4, Kind::TreeView).child(Node::new(5, Kind::TreeItem).prop("text", text("Leek"))));
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Lists"), tree);
        let ctx = egui::Context::default();
        look::apply(&ctx);
        let mut closed = false;
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 500.0))), ..Default::default() };
            ctx.run_ui(input, |ui| closed |= w.frame(ui).closed).drop_without_applying_deltas();
        }
        assert!(!closed, "the window stays open");
        let heights: Vec<f32> = [2, 3, 4].iter().map(|id| w.inputs.rects.get(id).expect("each fills its share").height()).collect();
        assert!(heights.iter().all(|h| (h - heights[0]).abs() < 0.5), "the list, table and tree are {heights:?} high");
        let bottom = w.inputs.rects.get(&4).expect("the tree is drawn").max.y;
        assert!(bottom <= 500.0, "the tree ends at {bottom}, inside the 500-point window");
    }

    /// A Column shares its height by the heights the widgets beside what
    /// fills it took in the frame before. A frame that draws one of them at
    /// another height, as a text an update makes two lines, asks for another,
    /// since the renderer draws one only when asked; the frames that follow
    /// end with the text after a filling Spacer inside the window and nothing
    /// more asked.
    #[test]
    fn a_column_asks_for_a_frame_when_the_heights_it_shared_by_change() {
        let text = |t: &str| Value::Str(String::from(t));
        let tree = Node::new(1, Kind::Column)
            .child(Node::new(2, Kind::Text).prop("text", text("top")))
            .child(Node::new(3, Kind::Spacer).prop("fill", Value::Bool(true)))
            .child(Node::new(4, Kind::Text).prop("text", text("bottom")));
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Fill"), tree);
        let ctx = egui::Context::default();
        look::apply(&ctx);
        let mut closed = false;
        let mut frame = |w: &mut WindowState| {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 300.0))), ..Default::default() };
            let output = ctx.run_ui(input, |ui| closed |= w.frame(ui).closed);
            let wait = output.viewport_output.get(&egui::ViewportId::ROOT).map_or(std::time::Duration::MAX, |v| v.repaint_delay);
            output.drop_without_applying_deltas();
            wait
        };
        let heights = |w: &WindowState| -> Vec<Option<f32>> { (2..=4).map(|id| w.inputs.rects.get(&id).map(egui::Rect::height)).collect() };
        frame(&mut w);
        frame(&mut w);
        assert!(frame(&mut w) > std::time::Duration::ZERO, "the window's third frame, drawing {:?}, still asks for another", heights(&w));
        w.tree.find_mut(2).expect("the top text is in the tree").props.insert(String::from("text"), text("top\nand a second line"));
        assert_eq!(frame(&mut w), std::time::Duration::ZERO, "the frame drawing two lines, {:?}, asks for another", heights(&w));
        let mut settled = frame(&mut w);
        if settled == std::time::Duration::ZERO {
            settled = frame(&mut w);
        }
        assert!(!closed, "the window stays open");
        assert!(settled > std::time::Duration::ZERO, "the frames asked for end asking for another, drawing {:?}", heights(&w));
        let bottom = w.inputs.rects.get(&4).expect("the bottom text is drawn").max.y;
        assert!(bottom <= 300.0, "the text after the Spacer ends at {bottom}, inside the 300-point window");
    }

    /// A menu bar fills the width it is given and a tree's rows span the
    /// tree, so a window is measured by the menu's entries and by the tree's
    /// arrows and names: one with a short menu and a tree of short names
    /// opens at the least width, however wide the screen.
    #[test]
    fn a_menu_bar_and_a_tree_are_measured_by_their_entries_not_the_screen() {
        let text = |t: &str| Value::Str(String::from(t));
        let menu = Node::new(3, Kind::Menu).prop("title", text("Edit")).child(Node::new(4, Kind::MenuItem).prop("text", text("Find")));
        let mut sources = Node::new(5, Kind::TreeView);
        for (at, name) in ["web01", "web02", "db01"].iter().enumerate() {
            sources = sources.child(Node::new(6 + at as u32, Kind::TreeItem).prop("text", text(name)));
        }
        let tree = Node::new(1, Kind::Column).child(Node::new(2, Kind::MenuBar).child(menu)).child(sources);
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Sources"), tree);
        let work = proto::ASSUMED_WORK_AREA;
        let content = Measure::new().content(&mut w, egui::FontDefinitions::default(), 1.0, work);
        assert!(content[0] < 200.0, "the menu and the tree measured {} points across", content[0]);
        let [width, _] = w.options.fitted(content, Some(work));
        assert!((width - 320.0).abs() < 0.5, "the window opens {width} points across, not at its least, 320");
    }

    /// A grid is measured with the item gap between its columns, as it is
    /// drawn, so a window fitted to it shows its last column whole.
    #[test]
    fn a_grid_is_measured_with_the_gap_between_its_columns() {
        let surface = |id: NodeId, across: f64| Node::new(id, Kind::GpuSurface).prop("width", Value::Float(across)).prop("height", Value::Float(120.0));
        let grid = Node::new(2, Kind::Grid).prop("columns", Value::Int(2)).child(surface(3, 60.0)).child(surface(4, 600.0));
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Heat"), Node::new(1, Kind::Column).child(grid));
        let content = Measure::new().content(&mut w, egui::FontDefinitions::default(), 1.0, proto::ASSUMED_WORK_AREA);
        let least = 60.0 + proto::tokens::SPACING.item_spacing[0] + 600.0;
        assert!(content[0] >= least, "the grid measured {} points across, less than its columns and the gap between them, {least}", content[0]);
    }

    /// A text box in a grid's first column is measured at its own width, as
    /// one in a Column is, not at the least width egui gives a column in a
    /// grid's first frame.
    #[test]
    fn a_text_box_in_a_grids_first_column_is_measured_at_its_own_width() {
        let field = |id: NodeId| Node::new(id, Kind::TextBox).prop("text", Value::Str(String::from("web01")));
        let grid = Node::new(2, Kind::Grid).prop("columns", Value::Int(2)).child(field(3)).child(field(4));
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Pair"), Node::new(1, Kind::Column).child(grid));
        let content = Measure::new().content(&mut w, egui::FontDefinitions::default(), 1.0, proto::ASSUMED_WORK_AREA);
        let own = proto::tokens::SPACING.text_edit_width;
        let least = 2.0 * own + proto::tokens::SPACING.item_spacing[0];
        assert!(content[0] >= least, "the two fields measured {} points across, less than two fields of {own} and the gap, {least}", content[0]);
    }

    /// A stretched grid's last column counts a control's least width when
    /// its window is measured, so a form's labels set its width and its
    /// fields fill what is left, as WinUI's ContentDialog sizes a form.
    #[test]
    fn a_stretched_grids_last_column_is_measured_at_a_controls_least_width() {
        let text = |t: &str| Value::Str(String::from(t));
        let grid = Node::new(2, Kind::Grid)
            .prop("columns", Value::Int(2))
            .prop("align", text("stretch"))
            .child(Node::new(3, Kind::Text).prop("text", text("Server")))
            .child(Node::new(4, Kind::TextBox).prop("text", text("web01")));
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Form"), Node::new(1, Kind::Column).child(grid));
        let work = proto::ASSUMED_WORK_AREA;
        let content = Measure::new().content(&mut w, egui::FontDefinitions::default(), 1.0, work);
        let own = proto::tokens::SPACING.text_edit_width;
        assert!(content[0] < own, "the form measured {} points across, its field counting more than a control's least width", content[0]);
        let [width, _] = w.options.fitted(content, Some(work));
        assert!((width - 320.0).abs() < 0.5, "the window opens {width} points across, not at its least, 320");
    }

    /// A progress bar is measured by its text and figure, as the page
    /// measures one, not by the room it fills, so a window fitted to one
    /// opens at its least width rather than across the screen.
    #[test]
    fn a_progress_bar_is_measured_by_its_text_not_the_screen() {
        let bar = Node::new(2, Kind::ProgressBar).prop("value", Value::Float(40.0)).prop("text", Value::Str(String::from("Copying")));
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Copying"), Node::new(1, Kind::Column).child(bar));
        let work = proto::ASSUMED_WORK_AREA;
        let content = Measure::new().content(&mut w, egui::FontDefinitions::default(), 1.0, work);
        assert!(content[0] < 200.0, "the progress bar measured {} points across", content[0]);
        let [width, _] = w.options.fitted(content, Some(work));
        assert!((width - 320.0).abs() < 0.5, "the window opens {width} points across, not at its least, 320");
    }

    /// A grid's row is measured as tall as its tallest cell, each cell laid
    /// out from the row's top, so a window fitted to it is no taller than
    /// the cell alone would make it.
    #[test]
    fn a_grid_row_is_measured_as_tall_as_its_tallest_cell() {
        let surface = |id: NodeId, down: f64| Node::new(id, Kind::GpuSurface).prop("width", Value::Float(120.0)).prop("height", Value::Float(down));
        let measured = |inside: Node| {
            let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Rows"), Node::new(1, Kind::Column).child(inside));
            Measure::new().content(&mut w, egui::FontDefinitions::default(), 1.0, proto::ASSUMED_WORK_AREA)[1]
        };
        let alone = measured(surface(2, 80.0));
        let row = measured(Node::new(2, Kind::Grid).prop("columns", Value::Int(2)).child(surface(3, 80.0)).child(surface(4, 30.0)));
        assert!((row - alone).abs() < 0.5, "the grid's row measured {row} points down, and its tallest cell alone {alone}");
    }

    /// A text box in a grid takes the focus a Set-GuiFocus gave before the
    /// window's first frame, which egui draws as the grid's sizing pass.
    #[test]
    fn a_text_box_in_a_grid_takes_the_focus_given_before_the_first_frame() {
        let text = |t: &str| Value::Str(String::from(t));
        let grid = Node::new(2, Kind::Grid)
            .prop("columns", Value::Int(2))
            .child(Node::new(3, Kind::Text).prop("text", text("Server")))
            .child(Node::new(4, Kind::TextBox).prop("text", text("web01")));
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Focus"), Node::new(1, Kind::Column).child(grid));
        w.focus_node(4).expect("the field takes the focus");
        let ctx = egui::Context::default();
        look::apply(&ctx);
        let size = egui::vec2(400.0, 200.0);
        for _ in 0..3 {
            painted(&ctx, &mut w, size, true);
        }
        assert_eq!(w.snapshot(None).focused, Some(4), "the field has the keyboard's focus");
    }

    /// A log given a width and a height is that size measured and drawn,
    /// however few lines it has, below egui's least scrolled size of 64
    /// points as above it, so an empty log opens its window around the room
    /// its lines will take.
    #[test]
    fn a_log_keeps_the_width_and_height_it_is_given() {
        for (across, down) in [(400.0_f32, 120.0_f32), (50.0, 40.0)] {
            let log = Node::new(2, Kind::LogView).prop("lines", Value::List(Vec::new())).prop("width", Value::Float(f64::from(across))).prop("height", Value::Float(f64::from(down)));
            let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Log"), Node::new(1, Kind::Column).child(log));
            let work = proto::ASSUMED_WORK_AREA;
            let content = Measure::new().content(&mut w, egui::FontDefinitions::default(), 1.0, work);
            let measured = *w.inputs.rects.get(&2).expect("the log is measured");
            assert!((measured.width() - across).abs() < 0.5 && (measured.height() - down).abs() < 0.5, "the empty log given {across} by {down} was measured {measured:?}");
            let [width, height] = w.options.fitted(content, Some(work));
            let ctx = egui::Context::default();
            look::apply(&ctx);
            let mut closed = false;
            for _ in 0..2 {
                let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))), ..Default::default() };
                ctx.run_ui(input, |ui| closed |= w.frame(ui).closed).drop_without_applying_deltas();
            }
            assert!(!closed, "the window stays open");
            let drawn = *w.inputs.rects.get(&2).expect("the log is drawn");
            assert!((drawn.width() - across).abs() < 0.5 && (drawn.height() - down).abs() < 0.5, "the log given {across} by {down} was drawn {drawn:?} in a {width} by {height} window");
        }
    }

    /// A tree's row too long for the tree ends in an ellipsis, as a list's
    /// item does, while one that fits is written whole; a window that fits
    /// its content is measured with the long row's whole text.
    #[test]
    fn a_tree_row_too_long_for_the_tree_ends_in_an_ellipsis() {
        let text = |t: &str| Value::Str(String::from(t));
        let long = "09:33:30 retrying POST /orders after the gateway timed out";
        let sources = Node::new(2, Kind::TreeView).child(Node::new(3, Kind::TreeItem).prop("text", text("web02"))).child(Node::new(4, Kind::TreeItem).prop("text", text(long)));
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Sources"), Node::new(1, Kind::Column).child(sources));
        let content = Measure::new().content(&mut w, egui::FontDefinitions::default(), 1.0, proto::ASSUMED_WORK_AREA);
        assert!(content[0] > 300.0, "the tree measured {} points across, less than its long row", content[0]);
        let ctx = egui::Context::default();
        look::apply(&ctx);
        let mut closed = false;
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(200.0, 160.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| closed |= w.frame(ui).closed);
            shapes = std::mem::take(&mut out.shapes);
            out.drop_without_applying_deltas();
        }
        assert!(!closed, "the window stays open");
        let elided = |wanted: &str| shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.job.text == wanted => Some(t.galley.elided),
            _ => None,
        });
        assert_eq!(elided(long), Some(true), "the long row ends in an ellipsis");
        assert_eq!(elided("web02"), Some(false), "the short row is written whole");
    }

    /// The shapes a frame of `w` paints in `ctx`, in a window `size` across
    /// and down, with the system's focus on the window as `focused` says.
    fn painted(ctx: &egui::Context, w: &mut WindowState, size: egui::Vec2, focused: bool) -> Vec<egui::epaint::ClippedShape> {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), focused, ..Default::default() };
        let mut closed = false;
        let mut out = ctx.run_ui(input, |ui| closed |= w.frame(ui).closed);
        assert!(!closed, "the window stays open");
        let shapes = std::mem::take(&mut out.shapes);
        out.drop_without_applying_deltas();
        shapes
    }

    /// The chosen tab's title is written in the strong text color and the
    /// other's in the weak one, and a line TAB_MARK thick in the selection's
    /// stroke lies under the chosen title's text alone, its bottom the
    /// separator line's.
    #[test]
    fn the_chosen_tab_is_marked_by_a_line_under_its_title() {
        let text = |t: &str| Value::Str(String::from(t));
        let tab = |id: NodeId, title: &str, page: &str| Node::new(id, Kind::Tab).prop("title", text(title)).child(Node::new(id + 10, Kind::Text).prop("text", text(page)));
        let tabs = Node::new(2, Kind::Tabs).child(tab(3, "Lines", "first page")).child(tab(4, "Errors", "second page"));
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Tabs"), Node::new(1, Kind::Column).child(tabs));
        let ctx = egui::Context::default();
        look::apply(&ctx);
        let size = egui::vec2(300.0, 200.0);
        painted(&ctx, &mut w, size, true);
        let shapes = painted(&ctx, &mut w, size, true);
        let visuals = ctx.global_style().visuals.clone();
        let title = |wanted: &str| {
            shapes
                .iter()
                .find_map(|c| match &c.shape {
                    egui::Shape::Text(t) if t.galley.job.text == wanted => Some((t.pos.x, t.pos.x + t.galley.size().x, t.fallback_color)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the title {wanted} is written"))
        };
        let (left, right, chosen) = title("Lines");
        assert_eq!(chosen, visuals.strong_text_color(), "the chosen title is in the strong text color");
        assert_eq!(title("Errors").2, visuals.weak_text_color(), "the other title is in the weak text color");
        let marks: Vec<egui::Rect> = shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == visuals.selection.stroke.color => Some(r.rect),
                _ => None,
            })
            .collect();
        let [mark] = marks.as_slice() else { panic!("one mark under the titles, not {marks:?}") };
        assert!((mark.height() - tokens::TAB_MARK).abs() < 0.01, "the mark is {} points thick", mark.height());
        assert!((mark.left() - left).abs() < 0.01 && (mark.right() - right).abs() < 0.01, "the mark {mark:?} spans the chosen title's text, {left} to {right}");
        let line = visuals.widgets.noninteractive.bg_stroke;
        let separator = shapes.iter().find_map(|c| match &c.shape {
            egui::Shape::LineSegment { points, stroke } if *stroke == line && points[0].y == points[1].y => Some(points[0].y),
            _ => None,
        });
        let Some(y) = separator else { panic!("the separator's line is drawn") };
        assert!((mark.bottom() - (y + line.width / 2.0)).abs() < 0.01, "the mark ends at {}, the separator's line at {}", mark.bottom(), y + line.width / 2.0);
    }

    /// An open item's child stands a level's indent in, so its chevron's box
    /// starts where its parent's text does, and each item's text follows its
    /// chevron's box and the icon's gap. The chevrons are drawn in the weak
    /// text color, the open item's pointing down and the closed one's right.
    #[test]
    fn a_tree_item_s_chevron_stands_under_its_parent_s_text() {
        let tree = tree_of(vec![item(2, "Fruit", vec![item(3, "Citrus", vec![item(4, "Lime", vec![])]), item(5, "Pear", vec![])])]);
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Food"), Node::new(10, Kind::Column).child(tree));
        w.inputs.open_items.insert(2);
        let ctx = egui::Context::default();
        look::apply(&ctx);
        let size = egui::vec2(300.0, 200.0);
        painted(&ctx, &mut w, size, true);
        let shapes = painted(&ctx, &mut w, size, true);
        let style = ctx.global_style();
        let (icon, slot) = (style.spacing.icon_width, style.spacing.icon_width + style.spacing.icon_spacing);
        let starts = |wanted: &str| {
            shapes
                .iter()
                .find_map(|c| match &c.shape {
                    egui::Shape::Text(t) if t.galley.job.text == wanted => Some(t.pos.x),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("the item {wanted} is written"))
        };
        let mut chevrons: Vec<&egui::epaint::PathShape> = shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Path(p) if p.points.len() == 3 && !p.closed => Some(p),
                _ => None,
            })
            .collect();
        chevrons.sort_by(|a, b| a.points[0].y.total_cmp(&b.points[0].y));
        let [fruit, citrus] = chevrons.as_slice() else { panic!("a chevron for each item that opens, not {chevrons:?}") };
        for chevron in [fruit, citrus] {
            assert_eq!(chevron.stroke.color, egui::epaint::ColorMode::Solid(style.visuals.weak_text_color()), "a chevron is in the weak text color");
            assert!((chevron.stroke.width - tokens::CHEVRON.stroke).abs() < 0.01);
        }
        assert!(fruit.points[1].y > fruit.points[0].y, "the open item's chevron points down: {:?}", fruit.points);
        assert!(citrus.points[1].x > citrus.points[0].x, "the closed item's chevron points right: {:?}", citrus.points);
        let middle = |p: &egui::epaint::PathShape| (p.points.iter().map(|q| q.x).fold(f32::INFINITY, f32::min) + p.points.iter().map(|q| q.x).fold(f32::NEG_INFINITY, f32::max)) / 2.0;
        let fruit_box = middle(fruit) - icon / 2.0;
        let citrus_box = middle(citrus) - icon / 2.0;
        assert!((starts("Fruit") - (fruit_box + slot)).abs() < 0.01, "Fruit's text follows its chevron's box and the icon's gap");
        assert!((citrus_box - starts("Fruit")).abs() < 0.01, "Citrus's chevron box starts at {citrus_box}, where Fruit's text does, {}", starts("Fruit"));
        assert!((starts("Pear") - starts("Citrus")).abs() < 0.01, "a leaf's text lines up with its sibling's");
    }

    /// A focused list's keyboard row shows its ring only while the window
    /// has the system's focus, as egui draws a focused field's edge, and the
    /// list still has the keyboard's focus while the ring is hidden.
    #[test]
    fn a_focused_list_shows_its_ring_only_while_the_window_has_the_focus() {
        let text = |t: &str| Value::Str(String::from(t));
        let list = Node::new(2, Kind::ListBox).prop("items", Value::List(vec![text("one"), text("two")]));
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Ring"), Node::new(1, Kind::Column).child(list));
        let ctx = egui::Context::default();
        look::apply(&ctx);
        let size = egui::vec2(300.0, 200.0);
        painted(&ctx, &mut w, size, true);
        w.focus_node(2).expect("the list takes the focus");
        painted(&ctx, &mut w, size, true);
        let ring = ctx.global_style().visuals.selection.stroke;
        let ringed = |shapes: &[egui::epaint::ClippedShape]| shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.stroke == ring && r.fill == egui::Color32::TRANSPARENT));
        assert!(ringed(&painted(&ctx, &mut w, size, true)), "the focused list's row has its ring while the window has the focus");
        assert!(!ringed(&painted(&ctx, &mut w, size, false)), "the ring goes while another window has the focus");
        assert_eq!(w.snapshot(None).focused, Some(2), "the list has the keyboard's focus all the while");
    }

    /// A window that fits its content shows every row of the table it opens
    /// with, under a button bar, with a menu bar over it, a status bar along
    /// its bottom, a row of text alone above the table and the table's
    /// filter row, or without them: each is measured as high as it is drawn.
    #[test]
    fn a_fitted_window_shows_every_row_of_its_table() {
        let text = |t: &str| Value::Str(String::from(t));
        let column = Value::Map(BTreeMap::from([(String::from("label"), text("Name"))]));
        let mut seen = Vec::new();
        for (menu, status, words, filter) in [(false, false, false, false), (false, false, false, true), (false, true, false, false), (true, false, false, false), (false, false, true, false), (true, true, true, true)] {
            let mut tree = Node::new(1, Kind::Column);
            if words {
                tree = tree.child(Node::new(10, Kind::Row).child(Node::new(11, Kind::Text).prop("text", text("Find"))));
            }
            tree = tree.child(Node::new(2, Kind::Table).prop("columns", Value::List(vec![column.clone()])).prop("filter", Value::Bool(filter)));
            tree = tree.child(Node::new(3, Kind::ButtonBar).child(Node::new(4, Kind::Button).prop("text", text("Close"))));
            if menu {
                tree = tree.child(Node::new(5, Kind::MenuBar).child(Node::new(6, Kind::Menu).prop("title", text("Edit")).child(Node::new(7, Kind::MenuItem).prop("text", text("Find")))));
            }
            if status {
                tree = tree.child(Node::new(8, Kind::StatusBar).child(Node::new(9, Kind::Text).prop("text", text("28 lines, 6 errors"))));
            }
            let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Rows"), tree);
            let rows = (0..16).map(|i| proto::Row { cells: vec![proto::Cell { text: format!("row {i}"), sort: SortKey::Number(f64::from(i)) }] }).collect();
            w.put_rows(2, 0, rows, false).expect("the table takes its rows");
            let content = Measure::new().content(&mut w, egui::FontDefinitions::default(), 1.0, proto::ASSUMED_WORK_AREA);
            let [width, height] = w.options.fitted(content, Some(proto::ASSUMED_WORK_AREA));
            let ctx = egui::Context::default();
            look::apply(&ctx);
            let mut closed = false;
            for _ in 0..3 {
                let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))), ..Default::default() };
                ctx.run_ui(input, |ui| closed |= w.frame(ui).closed).drop_without_applying_deltas();
            }
            assert!(!closed, "the window stays open");
            let table = w.inputs.tables.get(&2).expect("the table is drawn");
            let rect = |id: NodeId| w.inputs.rects.get(&id).copied();
            seen.push(format!("menu {menu} status {status} words {words} filter {filter}: content {content:?}, window {width} x {height}, table {:?} room {}, status {:?}, buttons {:?}, last row shown {}", rect(2), table.room, rect(8), rect(3), table.at_end));
        }
        assert!(seen.iter().all(|s| s.ends_with("last row shown true")), "{}", seen.join("\n"));
    }

    /// Rows Add-GuiItem adds to a table showing its last row leave its last
    /// row in sight, while rows a grid view's pipeline brings, and rows added
    /// to a table showing its first rows, leave the table where it is.
    #[test]
    fn a_table_follows_its_end_only_for_rows_added_while_it_shows_it() {
        let column = Value::Map(BTreeMap::from([(String::from("label"), Value::Str(String::from("Name")))]));
        let rows = |from: usize, to: usize| (from..to).map(|i| proto::Row { cells: vec![proto::Cell { text: format!("row {i}"), sort: SortKey::Number(i as f64) }] }).collect::<Vec<_>>();
        // A table opened with `opened` rows, then given 20 more, as
        // Add-GuiItem sends them when `added`; whether it then shows its
        // last row.
        let shows_end = |opened: usize, added: bool| {
            let table = Node::new(2, Kind::Table).prop("columns", Value::List(vec![column.clone()]));
            let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Rows"), Node::new(1, Kind::Column).child(table));
            w.put_rows(2, 0, rows(0, opened), false).expect("the table takes its rows");
            let ctx = egui::Context::default();
            look::apply(&ctx);
            let mut closed = false;
            let mut frames = |w: &mut WindowState| {
                for _ in 0..2 {
                    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(240.0, 240.0))), ..Default::default() };
                    ctx.run_ui(input, |ui| closed |= w.frame(ui).closed).drop_without_applying_deltas();
                }
            };
            frames(&mut w);
            w.put_rows(2, opened as u64, rows(opened, opened + 20), added).expect("the table takes the rows added");
            frames(&mut w);
            assert!(!closed, "the window stays open");
            w.inputs.tables.get(&2).expect("the table is drawn").at_end
        };
        assert!(shows_end(3, true), "rows added to a table showing its last row leave its last row in sight");
        assert!(!shows_end(3, false), "a grid view's pipeline rows leave the table at its first rows");
        assert!(!shows_end(40, true), "rows added to a table showing its first rows leave it there");
    }

    /// A list, a table and a list of check boxes given the focus as
    /// Set-GuiFocus gives it move and select by the keys the protocol
    /// crate's rule gives: Enter on the list's first row activates nothing
    /// while the row is not selected, and Down selects the next row, which
    /// Enter then activates; Shift+End takes a range in a table of several
    /// from the row its keyboard was on; and Space checks a check box, Down
    /// moving without checking.
    #[test]
    fn a_focused_list_table_and_check_list_move_and_select_by_key() {
        let text = |t: &str| Value::Str(String::from(t));
        let col = |l: &str| Value::Map(BTreeMap::from([(String::from("label"), text(l))]));
        let tree = Node::new(1, Kind::Column)
            .child(Node::new(2, Kind::ListBox).prop("items", Value::List(vec![text("alpha"), text("beta"), text("gamma")])))
            .child(Node::new(3, Kind::Table).prop("columns", Value::List(vec![col("Name")])).prop("selection", text("multiple")))
            .child(Node::new(4, Kind::ListBox).prop("multiple", Value::Bool(true)).prop("items", Value::List(vec![text("one"), text("two")])));
        let mut w = WindowState::new(1, String::from("ring"), proto::WindowOptions::titled("Keys"), tree);
        let rows = (0..4).map(|i| proto::Row { cells: vec![proto::Cell { text: format!("row {i}"), sort: SortKey::Text(format!("row {i}")) }] }).collect();
        w.put_rows(3, 0, rows, false).expect("the table takes its rows");
        let ctx = egui::Context::default();
        look::apply(&ctx);
        // A press and its release, so egui counts the next press of the key
        // as a press of its own rather than a repeat.
        let key = |key: egui::Key, modifiers: egui::Modifiers| -> Vec<egui::Event> {
            [true, false].map(|pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }).to_vec()
        };
        let frame = |w: &mut WindowState, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 500.0))), events, ..Default::default() };
            let mut raised = Vec::new();
            ctx.run_ui(input, |ui| raised = w.frame(ui).events).drop_without_applying_deltas();
            raised.into_iter().filter_map(|m| match m {
                FromHelper::Event { node, kind, value, .. } => Some((node, kind, value)),
                _ => None,
            }).collect::<Vec<_>>()
        };
        let list = |rows: &[i64]| Value::List(rows.iter().map(|r| Value::Int(*r)).collect());
        frame(&mut w, Vec::new());
        w.focus_node(2).expect("the list takes the focus");
        frame(&mut w, Vec::new());
        assert_eq!(frame(&mut w, key(egui::Key::Enter, egui::Modifiers::NONE)), Vec::new(), "Enter on the first row, not selected, activates nothing");
        assert_eq!(frame(&mut w, key(egui::Key::ArrowDown, egui::Modifiers::NONE)), vec![(2, EventKind::Select, list(&[1]))], "Down from the first row selects the second");
        assert_eq!(frame(&mut w, key(egui::Key::Enter, egui::Modifiers::NONE)), vec![(2, EventKind::Activate, Value::Int(1))]);
        w.focus_node(3).expect("the table takes the focus");
        frame(&mut w, Vec::new());
        assert_eq!(frame(&mut w, key(egui::Key::End, egui::Modifiers::SHIFT)), vec![(3, EventKind::Select, list(&[0, 1, 2, 3]))], "Shift+End takes every row from the first");
        w.focus_node(4).expect("the check list takes the focus");
        frame(&mut w, Vec::new());
        assert_eq!(frame(&mut w, key(egui::Key::Space, egui::Modifiers::NONE)), vec![(4, EventKind::Select, list(&[0]))]);
        assert_eq!(frame(&mut w, key(egui::Key::ArrowDown, egui::Modifiers::NONE)), Vec::new(), "Down moves without checking");
        assert_eq!(frame(&mut w, key(egui::Key::Space, egui::Modifiers::NONE)), vec![(4, EventKind::Select, list(&[0, 1]))]);
        assert_eq!(w.snapshot(None).focused, Some(4), "the snapshot names the check list");
    }

    /// A window measured right after another in the same context takes the
    /// size it takes measured alone: its widgets' ids are its own, so its
    /// grid does not start from the column widths of the other window's grid
    /// at the same node.
    #[test]
    fn a_window_measured_after_another_takes_its_own_size() {
        let text = |t: &str| Value::Str(String::from(t));
        let form = |window: WindowId, label: &str| {
            let tree = Node::new(1, Kind::Column).child(
                Node::new(2, Kind::Grid)
                    .prop("columns", Value::Int(2))
                    .child(Node::new(3, Kind::Text).prop("text", text(label)))
                    .child(Node::new(4, Kind::Text).prop("text", text("value"))),
            );
            let options = proto::WindowOptions { fit: proto::Fit::DIALOG, ..proto::WindowOptions::titled("Form") };
            WindowState::new(window, String::from("ring"), options, tree)
        };
        let work = proto::ASSUMED_WORK_AREA;
        let alone = Measure::new().content(&mut form(2, "Name"), egui::FontDefinitions::default(), 1.0, work);
        let mut shared = Measure::new();
        let wide = shared.content(&mut form(1, "A label much wider than the other window's label"), egui::FontDefinitions::default(), 1.0, work);
        let after = shared.content(&mut form(2, "Name"), egui::FontDefinitions::default(), 1.0, work);
        assert!(wide[0] > alone[0], "the wider window measures {wide:?}, wider than {alone:?}");
        assert_eq!(after, alone, "measured alone {alone:?}, after the wider window {after:?}");
    }

    /// A dialog fitted to its content, containing a Markdown document of the
    /// text `markdown` and the pictures `pictures` read for its images, by
    /// their addresses.
    fn document_window(markdown: &str, pictures: BTreeMap<String, Value>) -> WindowState {
        let document = Value::Map(BTreeMap::from([(String::from("text"), Value::Str(String::from(markdown))), (String::from("pictures"), Value::Map(pictures))]));
        let tree = Node::new(1, Kind::Column).child(Node::new(2, Kind::Markdown).prop("document", document));
        let options = proto::WindowOptions { fit: proto::Fit::DIALOG, ..proto::WindowOptions::titled("Doc") };
        WindowState::new(1, String::from("ring"), options, tree)
    }

    /// A document's picture read from bytes is measured at its own size, so
    /// the window fitted to the document draws it at the height it was
    /// measured at, the picture whole.
    #[test]
    fn a_document_is_drawn_at_the_height_it_was_measured_at_with_its_picture() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><rect width="120" height="80" fill="#ff0000"/></svg>"##;
        let picture = Value::Map(BTreeMap::from([(String::from("bytes"), Value::Bytes(Arc::from(svg.as_bytes()))), (String::from("format"), Value::Str(String::from("svg")))]));
        let mut w = document_window("![a red box](box.svg)\n\nUnder the box.", BTreeMap::from([(String::from("box.svg"), picture)]));
        let work = proto::ASSUMED_WORK_AREA;
        let content = Measure::new().content(&mut w, egui::FontDefinitions::default(), 1.0, work);
        let measured = w.inputs.rects.get(&2).expect("the document is measured").height();
        assert!(measured > 80.0, "the document is measured {measured} high, with its 80-point picture");
        let [width, height] = w.options.fitted(content, Some(work));
        let ctx = egui::Context::default();
        look::apply(&ctx);
        install_picture_loaders(&ctx, Arc::new(fontdb::Database::new()), None);
        let mut closed = false;
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))), ..Default::default() };
            ctx.run_ui(input, |ui| closed |= w.frame(ui).closed).drop_without_applying_deltas();
        }
        assert!(!closed, "the window stays open");
        let drawn = w.inputs.rects.get(&2).expect("the document is drawn").height();
        assert!((drawn - measured).abs() < 0.5, "the document was measured {measured} points high and drawn {drawn} high in a {width} by {height} window");
    }

    /// Measuring a window loads none of its document's pictures and counts
    /// none as reported, so the window's own frames report the picture they
    /// cannot draw, once.
    #[test]
    fn measuring_a_document_leaves_its_pictures_to_be_reported_by_its_window() {
        let gone = "https://127.0.0.1:9/gone.png";
        let mut w = document_window(&format!("![a picture from nowhere]({gone})"), BTreeMap::new());
        let work = proto::ASSUMED_WORK_AREA;
        let content = Measure::new().content(&mut w, egui::FontDefinitions::default(), 1.0, work);
        assert!(w.inputs.documents.get(&2).is_some_and(|d| d.failed.is_empty()), "measuring counts no picture as reported");
        let [width, height] = w.options.fitted(content, Some(work));
        let ctx = egui::Context::default();
        look::apply(&ctx);
        let mut reported = 0;
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, height))), ..Default::default() };
            ctx.run_ui(input, |ui| {
                let out = w.frame(ui);
                reported += out.events.iter().filter(|e| matches!(e, FromHelper::Error { text, .. } if text.contains(&format!("the image {gone} shows its alt text")))).count();
            })
            .drop_without_applying_deltas();
        }
        assert_eq!(reported, 1, "the window's frames report the picture they cannot draw once");
    }

    /// A table column has the room its heading takes with either sort
    /// arrow, as egui lays the heading out, so sorting by it moves no
    /// column; a short heading's column is an interactive widget's width at
    /// least.
    #[test]
    fn a_table_column_keeps_room_for_its_heading_with_either_sort_arrow() {
        let ctx = egui::Context::default();
        look::apply(&ctx);
        let mut checked = 0;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            checked = 0;
            for label in ["ProcessName", "WS(M)", "SI"] {
                let room = header_room(ui, label);
                for mark in SORT_MARKS {
                    let drawn = ui.add(heading(label, mark)).rect.width();
                    assert!(drawn <= room + 0.01, "{label}{mark} is drawn {drawn} wide in a column {room} wide");
                    checked += 1;
                }
                assert!(room >= ui.spacing().interact_size.x, "{label}'s column is {room} wide");
            }
        })
        .drop_without_applying_deltas();
        assert_eq!(checked, 6, "each heading was drawn with each arrow");
    }

    #[test]
    fn a_system_without_opengl_falls_back_to_the_other_backends() {
        let all = wgpu::Backends::DX12 | wgpu::Backends::VULKAN | wgpu::Backends::GL;
        assert_eq!(loadable(all, false).expect("the others"), wgpu::Backends::DX12 | wgpu::Backends::VULKAN);
        assert_eq!(loadable(all, true).expect("all three"), all);
        assert_eq!(loadable(wgpu::Backends::DX12, false).expect("no OpenGL requested"), wgpu::Backends::DX12);
        let alone = loadable(wgpu::Backends::GL, false).expect_err("nothing else to draw with");
        assert!(alone.contains("opengl32.dll") && alone.contains("PWRS-GUI.Mesa.win-x64"), "{alone}");
    }

    /// On a machine whose System32 has `opengl32.dll`, as every test host
    /// does, a Mesa add-on found on `PSModulePath` is left alone, and one
    /// chosen that this process cannot draw through is an error naming it,
    /// whether another `opengl32.dll` is loaded already or the folder has
    /// none.
    #[cfg(windows)]
    #[test]
    fn a_mesa_add_on_is_drawn_through_when_chosen_or_where_the_system_has_no_opengl() {
        let folder = std::env::temp_dir().join(format!("pwrs-gui-mesa-load-test-{}", std::process::id()));
        let found = Mesa { folder: folder.clone(), chosen: false };
        assert!(opengl_present(Some(&found)).expect("the system's OpenGL"));
        assert!(MESA_OPENGL.get().is_none());
        let chosen = Mesa { folder: folder.clone(), chosen: true };
        let refused = opengl_present(Some(&chosen)).expect_err("a Mesa add-on this process cannot draw through");
        assert!(refused.contains(&folder.join("opengl32.dll").display().to_string()), "{refused}");
    }

    #[test]
    fn missing_libraries_are_listed_as_a_sentence_lists_them() {
        assert_eq!(listed(&["a"]), "a");
        assert_eq!(listed(&["a", "b"]), "a and b");
        assert_eq!(listed(&["a", "b", "c"]), "a, b and c");
    }

    #[test]
    fn backends_parse_by_name_and_refuse_an_unknown_one() {
        assert_eq!(parse_backends("dx12, Vulkan").expect("two backends"), wgpu::Backends::DX12 | wgpu::Backends::VULKAN);
        assert_eq!(parse_backends("GL").expect("one backend"), wgpu::Backends::GL);
        assert!(parse_backends("webgpu").is_err());
        assert!(parse_backends(" , ").is_err());
    }

    #[test]
    fn hardware_comes_before_software_and_the_backend_order_breaks_ties() {
        let key = |d, b| (device_rank(d, false), backend_rank(b));
        assert!(key(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Vulkan) < key(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Dx12));
        assert!(key(wgpu::DeviceType::IntegratedGpu, wgpu::Backend::Gl) < key(wgpu::DeviceType::Cpu, wgpu::Backend::Dx12));
        if cfg!(windows) {
            assert!(key(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Dx12) < key(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Vulkan));
        }
        assert!(key(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Vulkan) < key(wgpu::DeviceType::DiscreteGpu, wgpu::Backend::Gl));
        assert_eq!(device_rank(wgpu::DeviceType::IntegratedGpu, true), 0);
    }

    #[test]
    fn a_crop_takes_the_widget_in_physical_pixels() {
        let size = [4usize, 3usize];
        let pixels = (0..12u8).map(|i| egui::Color32::from_rgba_unmultiplied(i, 0, 0, 255)).collect();
        let image = egui::ColorImage { size, pixels, source_size: egui::Vec2::new(4.0, 3.0) };
        let whole = pixels_of(&image, None, 1.0).expect("the whole image");
        assert_eq!((whole.width, whole.height), (4, 3));
        let rect = egui::Rect::from_min_max(egui::pos2(0.5, 0.5), egui::pos2(1.5, 1.5));
        let crop = pixels_of(&image, Some(rect), 2.0).expect("a crop");
        assert_eq!((crop.width, crop.height), (2, 2));
        assert_eq!(crop.rgba[0], 5, "the crop starts at pixel (1, 1)");
    }

    #[test]
    fn a_shortcut_takes_only_its_exact_keys_and_reads_clipboard_events_as_them() {
        let ctrl = egui::Modifiers { ctrl: true, command: true, ..egui::Modifiers::NONE };
        let press = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let mut input = egui::InputState::default();
        input.events = vec![press(egui::Key::S, ctrl | egui::Modifiers::SHIFT)];
        assert!(!take_press(&mut input, egui::Modifiers::COMMAND, egui::Key::S), "Ctrl+S does not act on Ctrl+Shift+S");
        assert!(take_press(&mut input, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::S));
        assert!(input.events.is_empty(), "the press is taken from the input");
        input.modifiers = ctrl;
        input.events = vec![egui::Event::Copy, egui::Event::Paste(String::from("x"))];
        assert!(take_press(&mut input, egui::Modifiers::COMMAND, egui::Key::C), "a copy event reads as Ctrl+C");
        assert_eq!(input.events, vec![egui::Event::Paste(String::from("x"))], "the paste is left for Ctrl+V or a text field");
        input.modifiers = egui::Modifiers::SHIFT;
        input.events = vec![egui::Event::Cut];
        assert!(!take_press(&mut input, egui::Modifiers::COMMAND, egui::Key::X), "a cut with Shift alone is Shift+Delete");
        assert!(take_press(&mut input, egui::Modifiers::SHIFT, egui::Key::Delete));
        let at = |key, place, modifiers| egui::Event::Key { key, physical_key: Some(place), pressed: true, repeat: false, modifiers };
        input.events = vec![at(egui::Key::Exclamationmark, egui::Key::Num1, ctrl | egui::Modifiers::SHIFT)];
        assert!(take_press(&mut input, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, egui::Key::Num1), "Ctrl+Shift+1 typing ! is the 1 key");
        input.events = vec![at(egui::Key::Plus, egui::Key::Equals, egui::Modifiers::SHIFT)];
        assert!(!take_press(&mut input, egui::Modifiers::SHIFT, egui::Key::Equals), "+ is Plus, not the key at its place");
        assert!(take_press(&mut input, egui::Modifiers::SHIFT, egui::Key::Plus));
    }

    #[test]
    fn the_fonts_the_build_carries_stand_before_the_systems_and_eguis_own() {
        let FoundFonts { definitions: fonts, said, problems, .. } = system_fonts(proto::Fonts::Bundled);
        assert!(problems.is_empty(), "{problems:?}");
        for (family, key, system) in [(egui::FontFamily::Proportional, "bundled-ui", "system-ui"), (egui::FontFamily::Monospace, "bundled-mono", "system-mono")] {
            let order = &fonts.families[&family];
            assert_eq!(order[0], key, "{said:?}");
            if fonts.font_data.contains_key(system) {
                assert_eq!(order[1], system, "the system's font draws what the build's lacks: {order:?}");
            }
        }
        let no_bundled = system_fonts(proto::Fonts::System).definitions;
        assert!(!no_bundled.font_data.contains_key("bundled-ui") && !no_bundled.font_data.contains_key("bundled-mono"), "the system's fonts alone");
    }

    #[test]
    fn the_system_fonts_stand_before_eguis_own() {
        let FoundFonts { definitions: fonts, said, problems, .. } = system_fonts(proto::Fonts::System);
        assert!(problems.is_empty(), "{problems:?}");
        for (family, key) in [(egui::FontFamily::Proportional, "system-ui"), (egui::FontFamily::Monospace, "system-mono")] {
            let order = &fonts.families[&family];
            if fonts.font_data.contains_key(key) {
                assert_eq!(order[0], key, "{said:?}");
                assert!(order.len() > 1, "egui's own fonts follow the system's: {order:?}");
            } else {
                assert!(!order.iter().any(|k| k == key), "{order:?}");
            }
        }
        if cfg!(windows) || cfg!(target_vendor = "apple") {
            assert!(fonts.font_data.contains_key("system-ui") && fonts.font_data.contains_key("system-mono"), "every Windows and macOS has its UI and monospace fonts: {said:?}");
        }
    }

    fn tree_of(items: Vec<Node>) -> Node {
        let mut tree = Node::new(1, Kind::TreeView);
        tree.children = items;
        tree
    }

    fn item(id: NodeId, text: &str, children: Vec<Node>) -> Node {
        let mut n = Node::new(id, Kind::TreeItem).prop("text", Value::Str(text.into()));
        n.children = children;
        n
    }

    #[test]
    fn a_tree_shows_an_open_items_children_below_it_and_selects_in_tree_order() {
        let tree = tree_of(vec![item(2, "Fruit", vec![item(3, "Apple", vec![]), item(4, "Pear", vec![])]), item(5, "Veg", vec![item(6, "Leek", vec![])])]);
        let mut open = HashSet::new();
        let mut shown = Vec::new();
        shown_items(&tree.children, &open, 0, &mut shown);
        assert_eq!(shown.iter().map(|(n, d)| (n.id, *d)).collect::<Vec<_>>(), [(2, 0), (5, 0)]);
        open.insert(2);
        shown.clear();
        shown_items(&tree.children, &open, 0, &mut shown);
        assert_eq!(shown.iter().map(|(n, d)| (n.id, *d)).collect::<Vec<_>>(), [(2, 0), (3, 1), (4, 1), (5, 0)]);
        assert_eq!(tree_selection(&tree, &BTreeSet::from([6, 3])), Value::List(vec![item_value(3), item_value(6)]));
        assert!(check_items(&tree.children).is_ok());
        assert!(check_items(&[Node::new(9, Kind::Text)]).is_err());
    }

    #[test]
    fn a_tree_selects_one_row_alone_and_ranges_from_its_anchor_with_shift_in_a_tree_of_several() {
        let ids = [10, 11, 12, 13];
        let mut state = TreeState::default();
        let ctrl = egui::Modifiers::COMMAND;
        let shift = egui::Modifiers::SHIFT;
        assert!(select_row(&mut state, &ids, 1, true, egui::Modifiers::NONE));
        assert!(select_row(&mut state, &ids, 3, true, shift));
        assert_eq!(state.selected, BTreeSet::from([11, 12, 13]));
        assert_eq!(state.anchor, Some(11), "Shift leaves the anchor where it is");
        assert!(select_row(&mut state, &ids, 2, true, ctrl));
        assert_eq!(state.selected, BTreeSet::from([11, 13]));
        assert_eq!(state.anchor, Some(12), "Ctrl moves the anchor");
        assert!(select_row(&mut state, &ids, 0, false, egui::Modifiers::NONE));
        assert_eq!(state.selected, BTreeSet::from([10]));
        assert!(!select_row(&mut state, &ids, 0, false, shift), "a tree of one selects the row alone whatever is pressed");
        assert_eq!(state.focus, Some(10));
    }

    #[test]
    fn opening_an_item_whose_children_are_to_come_raises_expand_once() {
        let share = item(2, "share", vec![]).prop("more", Value::Bool(true));
        let mut open = HashSet::new();
        let mut expanding = HashSet::new();
        let mut out = Vec::new();
        toggle_item(&share, 1, &mut open, &mut expanding, 7, &mut out);
        assert!(open.contains(&2) && expanding.contains(&2));
        assert!(matches!(out.as_slice(), [FromHelper::Event { window: 7, node: 1, kind: EventKind::Expand, value: Value::Int(2), .. }]), "{out:?}");
        toggle_item(&share, 1, &mut open, &mut expanding, 7, &mut out);
        toggle_item(&share, 1, &mut open, &mut expanding, 7, &mut out);
        assert_eq!(out.len(), 1, "closing and opening again while it waits raises Expand once");
        let leaf = item(3, "file", vec![]);
        assert!(!opens(&leaf) && opens(&share) && lazy(&share));
    }

    #[test]
    fn a_shortcut_reads_as_egui_keys_with_ctrl_as_command() {
        let s = Shortcut::parse("Ctrl+Alt+F5").expect("a shortcut");
        let (modifiers, key) = egui_shortcut(&s).expect("egui keys");
        assert_eq!(key, egui::Key::F5);
        assert!(modifiers.command && modifiers.alt && !modifiers.ctrl && !modifiers.shift && !modifiers.mac_cmd);
        for name in Shortcut::KEYS {
            assert!(egui_shortcut(&Shortcut::parse(name).expect("a named key")).is_ok(), "egui reads the key {name}");
        }
    }

    #[test]
    fn a_capture_is_asked_for_once_and_again_only_when_its_frames_bring_none() {
        assert_eq!(ask_for_capture(None, 7), Some(false), "a capture not yet requested is requested");
        assert_eq!(ask_for_capture(Some(7), 8), None, "the next frame waits for it");
        assert_eq!(ask_for_capture(Some(7), 7 + CAPTURE_ASKED_AGAIN_AFTER - 1), None);
        assert_eq!(ask_for_capture(Some(7), 7 + CAPTURE_ASKED_AGAIN_AFTER), Some(true), "it is requested again once its frames brought none");
        assert_eq!(ask_for_capture(Some(9), 3), None, "a frame number below the requesting one waits");
    }

    #[test]
    fn a_capture_untaken_for_the_deadline_is_late_and_a_younger_one_waits() {
        let now = std::time::Instant::now();
        let shot = |request: u64, age: std::time::Duration| PendingShot {
            request,
            reply: String::from("ring"),
            node: None,
            sent: Some(3),
            asked: now.checked_sub(age).expect("the clock has run longer than the deadline"),
        };
        let one_ms = std::time::Duration::from_millis(1);
        let (late, kept) = late_shots(vec![shot(1, CAPTURE_DEADLINE), shot(2, CAPTURE_DEADLINE - one_ms), shot(3, CAPTURE_DEADLINE + one_ms)], now);
        assert_eq!(late.iter().map(|s| s.request).collect::<Vec<_>>(), vec![1, 3], "captures requested the deadline or longer ago are late");
        assert_eq!(kept.iter().map(|s| s.request).collect::<Vec<_>>(), vec![2], "a younger capture waits");
    }

    #[cfg(windows)]
    #[test]
    fn a_window_the_thread_owns_is_listed_with_its_title() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WS_OVERLAPPED};
        let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let title = "PWRS-GUI front \u{e9}\u{7a93}";
        let before = thread_windows();
        let (class, text) = (wide("STATIC"), wide(title));
        let hwnd = unsafe {
            CreateWindowExW(0, class.as_ptr(), text.as_ptr(), WS_OVERLAPPED, 0, 0, 10, 10, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null())
        };
        assert!(!hwnd.is_null(), "a STATIC window could not be created");
        let listed = thread_windows();
        let read = window_title(hwnd as usize);
        assert!(unsafe { DestroyWindow(hwnd) } != 0, "the window could not be destroyed");
        assert!(!before.contains(&(hwnd as usize)), "the window was listed before it existed");
        assert!(listed.contains(&(hwnd as usize)), "the thread's windows {listed:?} lack the one it made");
        assert_eq!(read, title);
    }

    #[test]
    fn a_picture_is_placed_by_its_own_size_the_sides_given_and_its_fit() {
        let natural = egui::vec2(200.0, 100.0);
        let all = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let middle_half = egui::Rect::from_min_max(egui::pos2(0.25, 0.0), egui::pos2(0.75, 1.0));
        let own = place_picture(natural, None, None, Fit::Contain, 500.0);
        assert_eq!((own.size, own.uv), (natural, all), "at its own size when there is room");
        assert_eq!(place_picture(natural, None, None, Fit::Contain, 100.0).size, egui::vec2(100.0, 50.0), "shrunk to the room, in proportion");
        assert_eq!(place_picture(natural, Some(50.0), None, Fit::Contain, 10.0).size, egui::vec2(50.0, 25.0), "a width given, the height follows");
        assert_eq!(place_picture(natural, None, Some(50.0), Fit::Contain, 10.0).size, egui::vec2(100.0, 50.0), "a height given, the width follows");
        let contain = place_picture(natural, Some(100.0), Some(100.0), Fit::Contain, 500.0);
        assert_eq!(contain.size, egui::vec2(100.0, 100.0));
        assert_eq!((contain.at, contain.uv), (egui::Rect::from_min_size(egui::pos2(0.0, 25.0), egui::vec2(100.0, 50.0)), all), "the whole picture, centered in the box");
        let cover = place_picture(natural, Some(100.0), Some(100.0), Fit::Cover, 500.0);
        assert_eq!((cover.at.size(), cover.uv, cover.whole), (egui::vec2(100.0, 100.0), middle_half, natural), "the box filled with the picture's middle");
        let fill = place_picture(natural, Some(100.0), Some(100.0), Fit::Fill, 500.0);
        assert_eq!((fill.at.size(), fill.uv), (egui::vec2(100.0, 100.0), all), "the picture stretched to the box");
        let none = place_picture(natural, Some(100.0), Some(100.0), Fit::None, 500.0);
        assert_eq!((none.at, none.uv, none.whole), (egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 100.0)), middle_half, natural), "its own size, centered and cropped");
    }

    #[test]
    fn an_svg_picture_is_drawn_at_the_size_asked_for_and_never_from_a_file() {
        let blue = std::env::temp_dir().join(format!("pwrs-gui-svg-href-{}.png", std::process::id()));
        image::RgbaImage::from_pixel(4, 2, image::Rgba([0, 0, 255, 255])).save(&blue).expect("a PNG to link to");
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="4" height="2"><rect width="4" height="2" fill="#ff0000"/><image xlink:href="{}" width="4" height="2"/></svg>"##,
            blue.display()
        );
        let loader = SvgLoader::new(Arc::new(fontdb::Database::new()), None);
        let drawn = draw_svg(svg.as_bytes(), SizeHint::Size { width: 8, height: 4, maintain_aspect_ratio: true }, &loader.options);
        std::fs::remove_file(&blue).expect("the PNG removed");
        let drawn = drawn.expect("the SVG drawn");
        assert_eq!(drawn.size, [8, 4], "drawn at the size requested");
        assert_eq!(drawn.source_size, egui::vec2(4.0, 2.0), "its own size in points unchanged");
        assert!(drawn.pixels.iter().all(|p| *p == egui::Color32::from_rgb(255, 0, 0)), "the linked file was not drawn: {:?}", &drawn.pixels[..4]);
    }
}
