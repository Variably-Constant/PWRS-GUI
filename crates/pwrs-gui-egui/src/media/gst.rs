//! The GStreamer backend, for Linux and the BSDs: `playbin` opens the clip,
//! plays its sound through the system's output and keeps its clock, and an
//! `appsink` hands over each frame of the picture in blue, green, red and
//! alpha order. An https clip comes through an `appsrc` the loader feeds by
//! byte ranges. GStreamer is loaded when the first player opens, through
//! bindings of this crate's own, so the renderer starts where GStreamer is
//! missing and a player there names what to install.

use super::{Facts, Failure, Frame, News, Opened};
use crossbeam_queue::SegQueue;
use dlopen2::wrapper::{Container, WrapperApi};
use proto::media::{Cause, Settings};
use proto::renderer::WakeLoop;
use pwrs_gui_fetch::{FIRST_PART, Remote};
use pwrs_gui_proto as proto;
use std::cell::Cell;
use std::ffi::{CStr, CString, c_char, c_int, c_uint, c_void};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};

/// The name the renderer's hello gives this backend.
pub(super) const NAME: &str = "GStreamer";

/// Whether the backend measures the sound it hands the system: a meter in
/// the pipeline's audio filter does.
pub(super) const MEASURES: bool = true;

/// What a player says where GStreamer or its base plugins are missing.
const INSTALL: &str = "install GStreamer's base and good plugins: gstreamer1.0-plugins-good on Debian and Ubuntu, gstreamer1-plugins-good on FreeBSD";

/// What to install where GStreamer or its base plugins are missing, as a
/// player's remedy.
#[cfg(target_os = "freebsd")]
const REMEDY: &str = "pkg install gstreamer1-plugins-good";
#[cfg(not(target_os = "freebsd"))]
const REMEDY: &str = "apt install gstreamer1.0-plugins-good, or dnf install gstreamer1-plugins-good";

/// A GLib error, as `GError` lays it out.
#[repr(C)]
struct GError {
    domain: u32,
    code: c_int,
    message: *mut c_char,
}

/// A GLib value, as `GValue` lays it out: its type and two words of data.
#[repr(C)]
struct GValue {
    g_type: usize,
    data: [u64; 2],
}

/// The fundamental type of a `gint`, as GLib numbers it.
const G_TYPE_INT: usize = 6 << 2;

/// The head every GStreamer message begins with, as `GstMiniObject` and
/// `GstMessage` lay it out.
#[repr(C)]
struct GstMiniObject {
    g_type: usize,
    refcount: c_int,
    lockstate: c_int,
    flags: c_uint,
    copy: *mut c_void,
    dispose: *mut c_void,
    free: *mut c_void,
    priv_uint: c_uint,
    priv_pointer: *mut c_void,
}

/// A message on the pipeline's bus, as `GstMessage` lays it out.
#[repr(C)]
struct GstMessage {
    mini_object: GstMiniObject,
    kind: c_uint,
    timestamp: u64,
    src: *mut c_void,
    seqnum: u32,
}

/// The head of a buffer, as `GstBuffer` lays it out: its times.
#[repr(C)]
struct GstBuffer {
    mini_object: GstMiniObject,
    pool: *mut c_void,
    pts: u64,
    dts: u64,
    duration: u64,
}

/// A buffer's time GStreamer leaves unset.
const CLOCK_TIME_NONE: u64 = u64::MAX;

/// What a mapped buffer gives, as `GstMapInfo` lays it out.
#[repr(C)]
struct GstMapInfo {
    memory: *mut c_void,
    flags: c_int,
    data: *mut u8,
    size: usize,
    maxsize: usize,
    user_data: [*mut c_void; 4],
    reserved: [*mut c_void; 4],
}

impl GstMapInfo {
    fn empty() -> GstMapInfo {
        GstMapInfo { memory: std::ptr::null_mut(), flags: 0, data: std::ptr::null_mut(), size: 0, maxsize: 0, user_data: [std::ptr::null_mut(); 4], reserved: [std::ptr::null_mut(); 4] }
    }
}

const MESSAGE_EOS: c_uint = 1 << 0;
const MESSAGE_ERROR: c_uint = 1 << 1;
const MESSAGE_STATE_CHANGED: c_uint = 1 << 6;
const MESSAGE_DURATION_CHANGED: c_uint = 1 << 20;
const MESSAGE_ASYNC_DONE: c_uint = 1 << 21;
const STATE_NULL: c_int = 1;
const STATE_PAUSED: c_int = 3;
const STATE_PLAYING: c_int = 4;
const STATE_CHANGE_FAILURE: c_int = 0;
const FORMAT_TIME: c_int = 3;
const SEEK_FLUSH: c_int = 1 << 0;
const SEEK_ACCURATE: c_int = 1 << 1;
const SEEK_KEY_UNIT: c_int = 1 << 2;
const SEEK_SET: c_int = 1;
const SEEK_NONE: c_int = 0;
const BUS_DROP: c_int = 0;
const MAP_READ: c_int = 1;
const CORE_ERROR_MISSING_PLUGIN: c_int = 12;
const STREAM_ERROR_TYPE_NOT_FOUND: c_int = 4;
const STREAM_ERROR_WRONG_TYPE: c_int = 5;
const STREAM_ERROR_CODEC_NOT_FOUND: c_int = 6;
const NANOS: f64 = 1e9;

type SyncHandler = unsafe extern "C" fn(bus: *mut c_void, message: *mut GstMessage, data: *mut c_void) -> c_int;
type DestroyNotify = unsafe extern "C" fn(data: *mut c_void);
type ClosureNotify = unsafe extern "C" fn(data: *mut c_void, closure: *mut c_void);
type SourceSetup = unsafe extern "C" fn(playbin: *mut c_void, source: *mut c_void, data: *mut c_void);
type Handoff = unsafe extern "C" fn(identity: *mut c_void, buffer: *mut c_void, data: *mut c_void);
type NeedData = unsafe extern "C" fn(source: *mut c_void, length: c_uint, data: *mut c_void);
type EnoughData = unsafe extern "C" fn(source: *mut c_void, data: *mut c_void);
type SeekData = unsafe extern "C" fn(source: *mut c_void, offset: u64, data: *mut c_void) -> c_int;

/// The functions an appsrc calls for its data, as `GstAppSrcCallbacks`
/// lays them out.
#[repr(C)]
struct AppSrcCallbacks {
    need_data: Option<NeedData>,
    enough_data: Option<EnoughData>,
    seek_data: Option<SeekData>,
    reserved: [*mut c_void; 4],
}

/// An appsrc whose stream can be read from any byte, as
/// `GST_APP_STREAM_TYPE_RANDOM_ACCESS` numbers it.
const STREAM_RANDOM_ACCESS: c_int = 2;

#[derive(WrapperApi)]
struct GLib {
    g_free: unsafe extern "C" fn(mem: *mut c_void),
    g_error_free: unsafe extern "C" fn(error: *mut GError),
}

#[derive(WrapperApi)]
struct GObject {
    g_object_get_property: unsafe extern "C" fn(object: *mut c_void, name: *const c_char, value: *mut GValue),
    g_object_set_property: unsafe extern "C" fn(object: *mut c_void, name: *const c_char, value: *const GValue),
    g_value_init: unsafe extern "C" fn(value: *mut GValue, g_type: usize) -> *mut GValue,
    g_value_unset: unsafe extern "C" fn(value: *mut GValue),
    g_value_get_int: unsafe extern "C" fn(value: *const GValue) -> c_int,
    g_value_set_object: unsafe extern "C" fn(value: *mut GValue, object: *mut c_void),
    g_signal_connect_data: unsafe extern "C" fn(instance: *mut c_void, signal: *const c_char, handler: *const c_void, data: *mut c_void, destroy: Option<ClosureNotify>, flags: c_int) -> std::ffi::c_ulong,
}

/// GStreamer's own library. The wrapper's methods take each function's
/// arguments, and a seek takes eight.
#[expect(clippy::too_many_arguments, reason = "gst_element_seek's own eight arguments, which the wrapper's method takes as C declares them")]
mod core_api {
    use super::{DestroyNotify, GError, GstMapInfo, GstMessage, SyncHandler};
    use dlopen2::wrapper::WrapperApi;
    use std::ffi::{c_char, c_int, c_uint, c_void};

    #[derive(WrapperApi)]
    pub(super) struct Gst {
        gst_init_check: unsafe extern "C" fn(argc: *mut c_int, argv: *mut *mut *mut c_char, error: *mut *mut GError) -> c_int,
        gst_element_factory_make: unsafe extern "C" fn(factory: *const c_char, name: *const c_char) -> *mut c_void,
        gst_element_get_type: unsafe extern "C" fn() -> usize,
        gst_element_set_state: unsafe extern "C" fn(element: *mut c_void, state: c_int) -> c_int,
        gst_element_get_bus: unsafe extern "C" fn(element: *mut c_void) -> *mut c_void,
        gst_bus_set_sync_handler: unsafe extern "C" fn(bus: *mut c_void, func: Option<SyncHandler>, data: *mut c_void, notify: Option<DestroyNotify>),
        gst_element_query_position: unsafe extern "C" fn(element: *mut c_void, format: c_int, position: *mut i64) -> c_int,
        gst_element_query_duration: unsafe extern "C" fn(element: *mut c_void, format: c_int, duration: *mut i64) -> c_int,
        gst_element_seek: unsafe extern "C" fn(element: *mut c_void, rate: f64, format: c_int, flags: c_int, start_type: c_int, start: i64, stop_type: c_int, stop: i64) -> c_int,
        gst_filename_to_uri: unsafe extern "C" fn(filename: *const c_char, error: *mut *mut GError) -> *mut c_char,
        gst_util_set_object_arg: unsafe extern "C" fn(object: *mut c_void, name: *const c_char, value: *const c_char),
        gst_object_ref: unsafe extern "C" fn(object: *mut c_void) -> *mut c_void,
        gst_object_unref: unsafe extern "C" fn(object: *mut c_void),
        gst_mini_object_unref: unsafe extern "C" fn(object: *mut c_void),
        gst_message_parse_state_changed: unsafe extern "C" fn(message: *mut GstMessage, old: *mut c_int, new: *mut c_int, pending: *mut c_int),
        gst_message_parse_error: unsafe extern "C" fn(message: *mut GstMessage, error: *mut *mut GError, debug: *mut *mut c_char),
        gst_core_error_quark: unsafe extern "C" fn() -> u32,
        gst_stream_error_quark: unsafe extern "C" fn() -> u32,
        gst_resource_error_quark: unsafe extern "C" fn() -> u32,
        gst_sample_get_buffer: unsafe extern "C" fn(sample: *mut c_void) -> *mut c_void,
        gst_sample_get_caps: unsafe extern "C" fn(sample: *mut c_void) -> *mut c_void,
        gst_caps_get_structure: unsafe extern "C" fn(caps: *const c_void, index: c_uint) -> *mut c_void,
        gst_structure_get_int: unsafe extern "C" fn(structure: *const c_void, field: *const c_char, value: *mut c_int) -> c_int,
        gst_buffer_map: unsafe extern "C" fn(buffer: *mut c_void, info: *mut GstMapInfo, flags: c_int) -> c_int,
        gst_buffer_unmap: unsafe extern "C" fn(buffer: *mut c_void, info: *mut GstMapInfo),
        gst_buffer_new_allocate: unsafe extern "C" fn(allocator: *mut c_void, size: usize, params: *mut c_void) -> *mut c_void,
        gst_buffer_fill: unsafe extern "C" fn(buffer: *mut c_void, offset: usize, src: *const c_void, size: usize) -> usize,
        gst_parse_bin_from_description: unsafe extern "C" fn(description: *const c_char, ghost_unlinked_pads: c_int, error: *mut *mut GError) -> *mut c_void,
        gst_bin_get_by_name: unsafe extern "C" fn(bin: *mut c_void, name: *const c_char) -> *mut c_void,
        gst_element_get_static_pad: unsafe extern "C" fn(element: *mut c_void, name: *const c_char) -> *mut c_void,
        gst_pad_get_current_caps: unsafe extern "C" fn(pad: *mut c_void) -> *mut c_void,
    }
}
use core_api::Gst;

#[derive(WrapperApi)]
struct GstApp {
    gst_app_sink_try_pull_sample: unsafe extern "C" fn(sink: *mut c_void, timeout: u64) -> *mut c_void,
    gst_app_sink_try_pull_preroll: unsafe extern "C" fn(sink: *mut c_void, timeout: u64) -> *mut c_void,
    gst_app_src_set_callbacks: unsafe extern "C" fn(source: *mut c_void, callbacks: *const AppSrcCallbacks, data: *mut c_void, notify: Option<DestroyNotify>),
    gst_app_src_set_stream_type: unsafe extern "C" fn(source: *mut c_void, kind: c_int),
    gst_app_src_set_size: unsafe extern "C" fn(source: *mut c_void, size: i64),
    gst_app_src_push_buffer: unsafe extern "C" fn(source: *mut c_void, buffer: *mut c_void) -> c_int,
    gst_app_src_end_of_stream: unsafe extern "C" fn(source: *mut c_void) -> c_int,
}

/// GStreamer's libraries, loaded and started once for the process.
struct Libraries {
    glib: Container<GLib>,
    gobject: Container<GObject>,
    gst: Container<Gst>,
    app: Container<GstApp>,
}

static LIBRARIES: OnceLock<Result<Libraries, String>> = OnceLock::new();

/// GStreamer's libraries, loaded and started the first time a player
/// opens, or why they cannot be.
fn libraries() -> Result<&'static Libraries, String> {
    LIBRARIES.get_or_init(load).as_ref().map_err(String::clone)
}

/// Loads the library whose names `names` give, trying each in turn.
fn library<T: WrapperApi>(names: &[&str]) -> Result<Container<T>, String> {
    let mut why = Vec::new();
    for name in names {
        // SAFETY: each of the library's symbols the wrapper names is
        // declared with the signature its C header gives it.
        match unsafe { Container::<T>::load(name) } {
            Ok(c) => return Ok(c),
            Err(e) => why.push(format!("{name}: {e}")),
        }
    }
    Err(why.join("; "))
}

fn load() -> Result<Libraries, String> {
    let missing = |why: String| format!("GStreamer could not be loaded ({why}); {INSTALL}");
    let glib = library::<GLib>(&["libglib-2.0.so.0", "libglib-2.0.so"]).map_err(missing)?;
    let gobject = library::<GObject>(&["libgobject-2.0.so.0", "libgobject-2.0.so"]).map_err(missing)?;
    let gst = library::<Gst>(&["libgstreamer-1.0.so.0", "libgstreamer-1.0.so"]).map_err(missing)?;
    let app = library::<GstApp>(&["libgstapp-1.0.so.0", "libgstapp-1.0.so"]).map_err(missing)?;
    let mut error: *mut GError = std::ptr::null_mut();
    // SAFETY: GStreamer starts with no arguments; a failure gives an error
    // this frees.
    let started = unsafe { gst.gst_init_check(std::ptr::null_mut(), std::ptr::null_mut(), &mut error) };
    if started == 0 {
        return Err(format!("GStreamer did not start: {}", take_error(&glib, error)));
    }
    Ok(Libraries { glib, gobject, gst, app })
}

/// The text of `error`, which this frees.
fn take_error(glib: &Container<GLib>, error: *mut GError) -> String {
    if error.is_null() {
        return String::from("it gave no reason");
    }
    // SAFETY: a GError GStreamer gave, its message a nul-terminated string.
    unsafe {
        let text = if (*error).message.is_null() { String::from("it gave no reason") } else { CStr::from_ptr((*error).message).to_string_lossy().into_owned() };
        glib.g_error_free(error);
        text
    }
}

/// A C string of `text`, which holds no nul.
fn c(text: &str) -> Result<CString, String> {
    CString::new(text).map_err(|e| format!("{text:?} holds a nul: {e}"))
}

/// A new element of factory `factory`, which names a known factory; none
/// where no plugin installed gives it.
fn find(libs: &Libraries, factory: &CStr) -> Option<*mut c_void> {
    // SAFETY: a nul-terminated factory name and no element name.
    let element = unsafe { libs.gst.gst_element_factory_make(factory.as_ptr(), std::ptr::null()) };
    (!element.is_null()).then_some(element)
}

/// A new element of factory `factory`, or why there is none.
fn make(libs: &Libraries, factory: &CStr) -> Result<*mut c_void, String> {
    find(libs, factory).ok_or_else(|| format!("GStreamer has no {} element; {INSTALL}", factory.to_string_lossy()))
}

/// What the pipeline's bus tells the player through, from GStreamer's own
/// threads, and what the engine asks of the next pump.
struct Watch {
    libs: &'static Libraries,
    news: Arc<SegQueue<News>>,
    wake: Option<WakeLoop>,
    /// The pipeline, whose own messages are the player's.
    pipeline: usize,
    /// Whether the clip has prerolled once, so a later preroll ends a seek.
    opened: AtomicBool,
    /// Whether a seek a script asked for is under way.
    seeking: AtomicBool,
    looped: AtomicBool,
    /// Whether the clip ended while looping, so the pump starts it again.
    restart: AtomicBool,
    /// Whether the clip ended, so the pump pauses the pipeline at its end.
    ended: AtomicBool,
    /// The phase last told: 0 none, 1 paused, 2 playing.
    told: AtomicU8,
}

impl Watch {
    fn tell(&self, news: News) {
        self.news.push(news);
        self.wake_loop();
    }

    fn wake_loop(&self) {
        if let Some(wake) = &self.wake {
            wake();
        }
    }
}

/// Turns the pipeline's messages into the player's news, on whichever
/// thread posts them, and drops each.
unsafe extern "C" fn on_message(_bus: *mut c_void, message: *mut GstMessage, data: *mut c_void) -> c_int {
    // SAFETY: `data` is the watch the engine gave the bus, which the bus
    // holds until it lets the handler go; `message` is the message posted.
    let (watch, kind, src) = unsafe { (&*(data as *const Watch), (*message).kind, (*message).src as usize) };
    match kind {
        MESSAGE_EOS if watch.looped.load(Ordering::Acquire) => {
            watch.restart.store(true, Ordering::Release);
            watch.wake_loop();
        }
        MESSAGE_EOS => {
            // The pipeline is paused at the end, which is no pause of its
            // own to tell.
            watch.told.store(1, Ordering::Release);
            watch.ended.store(true, Ordering::Release);
            watch.tell(News::Ended);
        }
        MESSAGE_ERROR => watch.tell(failure(watch.libs, message)),
        MESSAGE_STATE_CHANGED if src == watch.pipeline => {
            let (mut old, mut new, mut pending) = (0, 0, 0);
            // SAFETY: a state-changed message, as its type says.
            unsafe { watch.libs.gst.gst_message_parse_state_changed(message, &mut old, &mut new, &mut pending) };
            if old != new && new == STATE_PLAYING && watch.told.swap(2, Ordering::AcqRel) != 2 {
                watch.tell(News::Playing);
            } else if old == STATE_PLAYING && new == STATE_PAUSED && watch.told.swap(1, Ordering::AcqRel) != 1 {
                watch.tell(News::Paused);
            }
        }
        MESSAGE_ASYNC_DONE if src == watch.pipeline => {
            if !watch.opened.swap(true, Ordering::AcqRel) {
                watch.tell(News::Opened);
            } else if watch.seeking.swap(false, Ordering::AcqRel) {
                watch.tell(News::Sought);
            }
        }
        MESSAGE_DURATION_CHANGED if watch.opened.load(Ordering::Acquire) => watch.tell(News::Opened),
        _ => {}
    }
    BUS_DROP
}

/// Lets go of the watch the bus held, once the bus lets its handler go.
unsafe extern "C" fn release_watch(data: *mut c_void) {
    // SAFETY: `data` is the watch the engine passed to the bus as its own
    // reference, which nothing else releases.
    drop(unsafe { Arc::from_raw(data as *const Watch) });
}

/// Lets go of the watch the meter's handoff signal held, once it lets its
/// handler go.
unsafe extern "C" fn release_meter(data: *mut c_void, _closure: *mut c_void) {
    // SAFETY: `data` is the watch's reference the engine gave the signal.
    unsafe { release_watch(data) };
}

/// Measures a buffer of the sound handed on, 32-bit floats as the audio
/// filter makes them: its highest sample, told as the player's news with
/// the span its samples last at the rate the meter's pad negotiated. A
/// buffer that cannot be mapped, or whose caps are not yet negotiated, is
/// not measured.
unsafe extern "C" fn on_handoff(identity: *mut c_void, buffer: *mut c_void, data: *mut c_void) {
    // SAFETY: `data` is the watch the signal holds, `identity` the meter
    // handing the buffer on, and `buffer` that buffer, mapped and unmapped
    // here.
    unsafe {
        let watch = &*(data as *const Watch);
        let gst = &watch.libs.gst;
        // A block with no time cannot be placed in the clip.
        let pts = (*buffer.cast::<GstBuffer>()).pts;
        if pts == CLOCK_TIME_NONE {
            return;
        }
        let Some([rate, channels]) = negotiated(gst, identity, c"sink", [c"rate", c"channels"]) else {
            return;
        };
        let mut map = GstMapInfo::empty();
        if gst.gst_buffer_map(buffer, &mut map, MAP_READ) == 0 {
            return;
        }
        let bytes = if map.data.is_null() { &[][..] } else { std::slice::from_raw_parts(map.data, map.size) };
        let (samples, _) = bytes.as_chunks::<4>();
        let peak = samples.iter().map(|b| f32::from_le_bytes(*b).abs()).fold(0.0, f32::max);
        gst.gst_buffer_unmap(buffer, &mut map);
        let at = pts as f64 / NANOS;
        let frames = samples.len() / channels as usize;
        watch.news.push(News::Heard { at, until: at + frames as f64 / f64::from(rate), peak });
    }
}

/// The whole-number fields `names` of the caps pad `pad` of `element` has
/// negotiated, each above 0; none before it has, or where one is missing.
///
/// # Safety
/// `element` must be a live element.
unsafe fn negotiated<const N: usize>(gst: &Gst, element: *mut c_void, pad: &CStr, names: [&CStr; N]) -> Option<[c_int; N]> {
    let mut values = [0; N];
    // SAFETY: a live element, as the caller says; the pad and the caps it
    // gives are this one's, released here.
    let read = unsafe {
        let pad = gst.gst_element_get_static_pad(element, pad.as_ptr());
        let caps = if pad.is_null() { std::ptr::null_mut() } else { gst.gst_pad_get_current_caps(pad) };
        if !pad.is_null() {
            gst.gst_object_unref(pad);
        }
        let read = !caps.is_null() && {
            let structure = gst.gst_caps_get_structure(caps, 0);
            !structure.is_null() && names.iter().zip(values.iter_mut()).all(|(name, value)| gst.gst_structure_get_int(structure, name.as_ptr(), value) != 0)
        };
        if !caps.is_null() {
            gst.gst_mini_object_unref(caps);
        }
        read
    };
    (read && values.iter().all(|v| *v > 0)).then_some(values)
}

/// An https clip the appsrc playbin makes for `appsrc://` reads, by the
/// loader's byte ranges on GStreamer's streaming threads.
struct Feed {
    libs: &'static Libraries,
    remote: Mutex<Remote>,
    watch: Arc<Watch>,
    /// Whether a read failed, which the player is told once.
    failed: AtomicBool,
}

impl Feed {
    /// The clip cannot be read: the player fails for the network's cause.
    fn fail(&self, why: String) {
        if !self.failed.swap(true, Ordering::AcqRel) {
            self.watch.tell(News::Failed(Failure::new(Cause::Network, format!("the loader could not read it: {why}"))));
        }
    }
}

/// Sets up the appsrc playbin made for an https clip: read from any byte
/// over the clip's length, its data from the feed.
unsafe extern "C" fn on_source_setup(_playbin: *mut c_void, source: *mut c_void, data: *mut c_void) {
    // SAFETY: `data` is the feed the engine gave the signal, which the
    // signal holds until it lets the handler go; `source` is the appsrc.
    let feed = unsafe { &*(data as *const Feed) };
    let length = match feed.remote.lock() {
        Ok(remote) => remote.length(),
        Err(e) => return feed.fail(format!("a read of the clip broke off: {e}")),
    };
    let size = match i64::try_from(length) {
        Ok(s) => s,
        Err(e) => return feed.fail(format!("its length of {length} bytes is more than GStreamer takes: {e}")),
    };
    let callbacks = AppSrcCallbacks { need_data: Some(on_need_data), enough_data: None, seek_data: Some(on_seek_data), reserved: [std::ptr::null_mut(); 4] };
    let app = &feed.libs.app;
    // SAFETY: a live appsrc; it copies the callbacks, and holds its own
    // reference to the feed until it lets them go.
    unsafe {
        app.gst_app_src_set_stream_type(source, STREAM_RANDOM_ACCESS);
        app.gst_app_src_set_size(source, size);
        Arc::increment_strong_count(data as *const Feed);
        app.gst_app_src_set_callbacks(source, &callbacks, data, Some(release_feed));
    }
}

/// Gives the appsrc the clip's next bytes, about `length` of them, or ends
/// its stream at the clip's end.
unsafe extern "C" fn on_need_data(source: *mut c_void, length: c_uint, data: *mut c_void) {
    // SAFETY: `data` is the feed the appsrc holds.
    let feed = unsafe { &*(data as *const Feed) };
    let want = if length == 0 || length == c_uint::MAX { FIRST_PART as usize } else { length as usize };
    let mut bytes = vec![0u8; want];
    let read = match feed.remote.lock() {
        Ok(mut remote) => remote.read_into(&mut bytes),
        Err(e) => Err(format!("a read of the clip broke off: {e}")),
    };
    let (gst, app) = (&feed.libs.gst, &feed.libs.app);
    // The flow either call returns is the pipeline's own to act on: one
    // flushing for a seek or stopping takes no data, and its next need asks
    // again.
    match read {
        // SAFETY: a live appsrc.
        Ok(0) => unsafe {
            app.gst_app_src_end_of_stream(source);
        },
        // SAFETY: a buffer of `n` bytes made, filled and handed to the
        // appsrc, which takes it.
        Ok(n) => unsafe {
            let buffer = gst.gst_buffer_new_allocate(std::ptr::null_mut(), n, std::ptr::null_mut());
            if buffer.is_null() {
                return feed.fail(format!("GStreamer gave no buffer of {n} bytes"));
            }
            gst.gst_buffer_fill(buffer, 0, bytes.as_ptr().cast(), n);
            app.gst_app_src_push_buffer(source, buffer);
        },
        Err(why) => feed.fail(why),
    }
}

/// Moves where the appsrc's next bytes are read from.
unsafe extern "C" fn on_seek_data(_source: *mut c_void, offset: u64, data: *mut c_void) -> c_int {
    // SAFETY: `data` is the feed the appsrc holds.
    let feed = unsafe { &*(data as *const Feed) };
    match feed.remote.lock() {
        Ok(mut remote) => {
            remote.seek_to(offset);
            1
        }
        Err(e) => {
            feed.fail(format!("a read of the clip broke off: {e}"));
            0
        }
    }
}

/// Lets go of a reference to a feed its holder took.
unsafe extern "C" fn release_feed(data: *mut c_void) {
    // SAFETY: `data` is a reference to the feed its holder took, which
    // nothing else releases.
    drop(unsafe { Arc::from_raw(data as *const Feed) });
}

/// Lets go of the feed the source-setup signal held, once it lets its
/// handler go.
unsafe extern "C" fn release_setup(data: *mut c_void, _closure: *mut c_void) {
    // SAFETY: as for release_feed.
    unsafe { release_feed(data) };
}

/// The news of error message `message`, its cause from the error's domain
/// and code: a format GStreamer has no plugin to play is not supported, a
/// resource that cannot be read is the network's cause, and the rest are
/// the decoder's.
fn failure(libs: &Libraries, message: *mut GstMessage) -> News {
    let (mut error, mut debug): (*mut GError, *mut c_char) = (std::ptr::null_mut(), std::ptr::null_mut());
    // SAFETY: an error message, as its type says; the debug text it gives
    // is freed here, and the error below.
    let (domain, code) = unsafe {
        libs.gst.gst_message_parse_error(message, &mut error, &mut debug);
        if !debug.is_null() {
            libs.glib.g_free(debug.cast());
        }
        if error.is_null() { (0, 0) } else { ((*error).domain, (*error).code) }
    };
    // SAFETY: the quarks' own functions, which take nothing.
    let (core, stream, resource) = unsafe { (libs.gst.gst_core_error_quark(), libs.gst.gst_stream_error_quark(), libs.gst.gst_resource_error_quark()) };
    let unplayable = (domain == core && code == CORE_ERROR_MISSING_PLUGIN) || (domain == stream && matches!(code, STREAM_ERROR_TYPE_NOT_FOUND | STREAM_ERROR_WRONG_TYPE | STREAM_ERROR_CODEC_NOT_FOUND));
    let cause = if unplayable {
        Cause::FormatNotSupported
    } else if domain == resource {
        Cause::Network
    } else {
        Cause::Decode
    };
    News::Failed(Failure::new(cause, take_error(&libs.glib, error)))
}

/// One clip open in a `playbin`.
pub(super) struct Engine {
    libs: &'static Libraries,
    pipeline: *mut c_void,
    bus: *mut c_void,
    /// The sink the picture's frames are taken from; none for sound alone.
    sink: Option<*mut c_void>,
    /// The sink the text of the clip's own subtitle tracks comes to; none
    /// for sound alone.
    text: Option<*mut c_void>,
    /// The cues the text sink has given.
    cues: std::cell::RefCell<Vec<proto::media::subtitles::Cue>>,
    watch: Arc<Watch>,
    /// Whether `scaletempo` keeps the sound's pitch at another rate.
    tempo: bool,
    rate: Cell<f64>,
    /// A rate asked for before the clip prerolled, set once it has.
    pending_rate: Cell<Option<f64>>,
    /// The picture's size, once a frame or the sink's negotiated caps gave
    /// it.
    size: Cell<Option<[u32; 2]>>,
    /// Where a seek under way goes, which the position gives until the
    /// pipeline has got there.
    target: Cell<f64>,
    /// The https clip being opened on a thread of its own, which the next
    /// tend gives the pipeline once it is open.
    opening: Cell<Option<Receiver<Opened>>>,
    /// The clip's file or https address.
    source: String,
}

impl Engine {
    /// Opens `settings`' clip in a `playbin`, its bus telling `news` and
    /// waking the loop through `wake`, and prerolls it, paused at its start.
    pub(super) fn open(settings: &Settings, news: Arc<SegQueue<News>>, wake: Option<WakeLoop>) -> Result<Engine, Failure> {
        let missing = |text: String| Failure::missing(text, REMEDY);
        let decode = |text: String| Failure::new(Cause::Decode, text);
        let libs = libraries().map_err(missing)?;
        let pipeline = make(libs, c"playbin").map_err(missing)?;
        let watch = Arc::new(Watch {
            libs,
            news,
            wake,
            pipeline: pipeline as usize,
            opened: AtomicBool::new(false),
            seeking: AtomicBool::new(false),
            looped: AtomicBool::new(settings.looped),
            restart: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            told: AtomicU8::new(0),
        });
        // SAFETY: the pipeline is a live element; the bus it gives is held
        // until the engine drops it.
        let bus = unsafe { libs.gst.gst_element_get_bus(pipeline) };
        let mut engine = Engine {
            libs,
            pipeline,
            bus,
            sink: None,
            text: None,
            cues: std::cell::RefCell::new(Vec::new()),
            watch,
            tempo: false,
            rate: Cell::new(1.0),
            pending_rate: Cell::new(None),
            size: Cell::new(None),
            target: Cell::new(0.0),
            opening: Cell::new(None),
            source: settings.source.clone(),
        };
        if settings.audio_only {
            engine.set(pipeline, c"flags", "audio+soft-volume").map_err(decode)?;
        } else {
            let sink = make(libs, c"appsink").map_err(missing)?;
            // SAFETY: the sink is a live element; the reference taken here
            // is the engine's, beside the one the pipeline takes.
            unsafe { libs.gst.gst_object_ref(sink) };
            engine.sink = Some(sink);
            for (name, value) in [(c"caps", "video/x-raw,format=BGRA"), (c"max-buffers", "1"), (c"drop", "true"), (c"enable-last-sample", "false")] {
                engine.set(sink, name, value).map_err(decode)?;
            }
            engine.set_element(c"video-sink", sink);
            let text = make(libs, c"appsink").map_err(missing)?;
            // SAFETY: as for the picture's sink.
            unsafe { libs.gst.gst_object_ref(text) };
            engine.text = Some(text);
            for (name, value) in [(c"caps", "text/x-raw"), (c"sync", "false"), (c"enable-last-sample", "false")] {
                engine.set(text, name, value).map_err(decode)?;
            }
            engine.set_element(c"text-sink", text);
        }
        let filter = match engine.audio_filter(true) {
            Ok(bin) => {
                engine.tempo = true;
                Ok(bin)
            }
            Err(with_tempo) => engine.audio_filter(false).map_err(|without| format!("{with_tempo}; without scaletempo, {without}")),
        };
        match filter {
            Ok(bin) => {
                if let Err(why) = engine.meter(bin) {
                    engine.watch.news.push(News::Warned(format!("its sound cannot be measured: {why}")));
                }
                engine.set_element(c"audio-filter", bin);
            }
            Err(why) => engine.watch.news.push(News::Warned(format!("its sound cannot be measured, nor its pitch kept at another rate: {why}"))),
        }
        engine.set_volume(settings.volume).map_err(decode)?;
        engine.set_muted(settings.muted).map_err(decode)?;
        if settings.rate != 1.0 {
            engine.pending_rate.set(Some(settings.rate));
        }
        let data = Arc::into_raw(Arc::clone(&engine.watch)) as *mut c_void;
        // SAFETY: the handler and its watch live until the bus lets them
        // go, which releases the watch.
        unsafe { libs.gst.gst_bus_set_sync_handler(bus, Some(on_message), data, Some(release_watch)) };
        if pwrs_gui_fetch::is_web(&settings.source) {
            let opening = super::open_web(&settings.source, settings.engine_fetch, Arc::clone(&engine.watch.news), engine.watch.wake.clone())?;
            engine.opening.set(Some(opening));
            return Ok(engine);
        }
        let path = c(&settings.source).map_err(|text| Failure::new(Cause::Network, text))?;
        let mut error: *mut GError = std::ptr::null_mut();
        // SAFETY: a nul-terminated path; the address it gives is freed here.
        let uri = unsafe { libs.gst.gst_filename_to_uri(path.as_ptr(), &mut error) };
        if uri.is_null() {
            return Err(Failure::new(Cause::Network, format!("it cannot be given to GStreamer as an address: {}", take_error(&libs.glib, error))));
        }
        // SAFETY: `uri` is the nul-terminated address GStreamer made.
        unsafe {
            libs.gst.gst_util_set_object_arg(pipeline, c"uri".as_ptr(), uri);
            libs.glib.g_free(uri.cast());
        }
        engine.preroll();
        Ok(engine)
    }

    /// Prerolls the clip, paused at its start. A clip that cannot be
    /// prerolled posts an error the bus's handler turns into the player's
    /// Error, so the state change's own result says nothing more.
    fn preroll(&self) {
        // SAFETY: the pipeline is a live element.
        unsafe { self.libs.gst.gst_element_set_state(self.pipeline, STATE_PAUSED) };
    }

    /// Has playbin fetch the https clip itself, through GStreamer's own
    /// https source, and prerolls it.
    fn engine_fetch(&self) -> Result<(), String> {
        let address = c(&self.source)?;
        // SAFETY: a live pipeline and a nul-terminated address.
        unsafe { self.libs.gst.gst_util_set_object_arg(self.pipeline, c"uri".as_ptr(), address.as_ptr()) };
        self.preroll();
        Ok(())
    }

    /// Plays the https clip the loader opened through the appsrc playbin
    /// makes for `appsrc://`, fed by the loader, and prerolls it.
    fn feed(&self, remote: Remote) -> Result<(), String> {
        let feed = Arc::new(Feed { libs: self.libs, remote: Mutex::new(remote), watch: Arc::clone(&self.watch), failed: AtomicBool::new(false) });
        let data = Arc::into_raw(feed) as *mut c_void;
        let setup = on_source_setup as SourceSetup;
        // SAFETY: a live pipeline; the signal holds the feed's reference
        // until it lets its handler go, which releases it.
        let handler = unsafe {
            self.libs.gst.gst_util_set_object_arg(self.pipeline, c"uri".as_ptr(), c"appsrc://".as_ptr());
            self.libs.gobject.g_signal_connect_data(self.pipeline, c"source-setup".as_ptr(), setup as *const c_void, data, Some(release_setup), 0)
        };
        if handler == 0 {
            // SAFETY: the reference the signal did not take.
            unsafe { release_feed(data) };
            return Err(String::from("GStreamer's playbin gives no source-setup signal to feed an https clip through"));
        }
        self.preroll();
        Ok(())
    }

    /// Sets property `name` of `object` from its text, as gst-launch does.
    fn set(&self, object: *mut c_void, name: &CStr, value: &str) -> Result<(), String> {
        let text = c(value)?;
        // SAFETY: a live object and nul-terminated strings.
        unsafe { self.libs.gst.gst_util_set_object_arg(object, name.as_ptr(), text.as_ptr()) };
        Ok(())
    }

    /// Sets the pipeline's element-valued property `name` to `element`,
    /// which the pipeline takes.
    fn set_element(&self, name: &CStr, element: *mut c_void) {
        let o = &self.libs.gobject;
        let mut value = GValue { g_type: 0, data: [0; 2] };
        // SAFETY: a zeroed value given the element type, set, passed and
        // unset.
        unsafe {
            o.g_value_init(&mut value, self.libs.gst.gst_element_get_type());
            o.g_value_set_object(&mut value, element);
            o.g_object_set_property(self.pipeline, name.as_ptr(), &value);
            o.g_value_unset(&mut value);
        }
    }

    /// The pipeline's audio filter: with `tempo`, scaletempo, so another
    /// rate keeps the sound's pitch; then a meter of the sound handed on,
    /// as 32-bit floats. Or why GStreamer cannot make it.
    fn audio_filter(&self, tempo: bool) -> Result<*mut c_void, String> {
        let description = if tempo {
            c"scaletempo ! audioconvert ! audio/x-raw,format=F32LE,layout=interleaved ! identity name=pwrs-meter signal-handoffs=true"
        } else {
            c"audioconvert ! audio/x-raw,format=F32LE,layout=interleaved ! identity name=pwrs-meter signal-handoffs=true"
        };
        let mut error: *mut GError = std::ptr::null_mut();
        // SAFETY: a nul-terminated description; an error it gives is freed.
        let bin = unsafe { self.libs.gst.gst_parse_bin_from_description(description.as_ptr(), 1, &mut error) };
        if bin.is_null() {
            return Err(take_error(&self.libs.glib, error));
        }
        if !error.is_null() {
            let why = take_error(&self.libs.glib, error);
            self.watch.news.push(News::Warned(format!("GStreamer made the sound's filter with a warning: {why}")));
        }
        Ok(bin)
    }

    /// Tells each buffer the meter of `filter` hands on as the player's
    /// news.
    fn meter(&self, filter: *mut c_void) -> Result<(), String> {
        // SAFETY: a live bin; the meter it gives is this one's reference,
        // let go of below.
        let identity = unsafe { self.libs.gst.gst_bin_get_by_name(filter, c"pwrs-meter".as_ptr()) };
        if identity.is_null() {
            return Err(String::from("the audio filter has no meter"));
        }
        let data = Arc::into_raw(Arc::clone(&self.watch)) as *mut c_void;
        let handoff = on_handoff as Handoff;
        // SAFETY: a live element; the signal holds the watch's reference
        // until it lets its handler go, which releases it.
        let handler = unsafe {
            let handler = self.libs.gobject.g_signal_connect_data(identity, c"handoff".as_ptr(), handoff as *const c_void, data, Some(release_meter), 0);
            self.libs.gst.gst_object_unref(identity);
            handler
        };
        if handler == 0 {
            // SAFETY: the reference the signal did not take.
            unsafe { release_watch(data) };
            return Err(String::from("its meter gives no handoff signal"));
        }
        Ok(())
    }

    /// The pipeline's whole-number property `name`.
    fn int(&self, name: &CStr) -> c_int {
        let o = &self.libs.gobject;
        let mut value = GValue { g_type: 0, data: [0; 2] };
        // SAFETY: a zeroed value given the property's type, read and unset.
        unsafe {
            o.g_value_init(&mut value, G_TYPE_INT);
            o.g_object_get_property(self.pipeline, name.as_ptr(), &mut value);
            let n = o.g_value_get_int(&value);
            o.g_value_unset(&mut value);
            n
        }
    }

    fn state(&self, state: c_int, what: &str) -> Result<(), String> {
        // SAFETY: the pipeline is a live element.
        if unsafe { self.libs.gst.gst_element_set_state(self.pipeline, state) } == STATE_CHANGE_FAILURE { Err(format!("GStreamer could not {what} the clip")) } else { Ok(()) }
    }

    /// Moves to `seconds` at `rate`, to the frame.
    fn seek_at(&self, seconds: f64, rate: f64) -> Result<(), String> {
        self.seek_with(seconds, rate, SEEK_FLUSH | SEEK_ACCURATE)
    }

    /// Moves to `seconds` at `rate`, as `flags` ask.
    fn seek_with(&self, seconds: f64, rate: f64, flags: c_int) -> Result<(), String> {
        let at = (seconds * NANOS).round() as i64;
        // SAFETY: the pipeline is a live element.
        let done = unsafe { self.libs.gst.gst_element_seek(self.pipeline, rate, FORMAT_TIME, flags, SEEK_SET, at, SEEK_NONE, -1) };
        if done == 0 { Err(format!("GStreamer could not move to {seconds} seconds at {rate} times")) } else { Ok(()) }
    }

    /// Moves near `seconds`, to the key frame before it, as a slider being
    /// dragged seeks.
    pub(super) fn seek_near(&self, seconds: f64) -> Result<(), String> {
        self.seek_with(seconds, self.rate.get(), SEEK_FLUSH | SEEK_KEY_UNIT)
    }

    pub(super) fn play(&self) -> Result<(), String> {
        self.watch.ended.store(false, Ordering::Release);
        self.state(STATE_PLAYING, "play")
    }

    pub(super) fn pause(&self) -> Result<(), String> {
        self.state(STATE_PAUSED, "pause")
    }

    pub(super) fn seek(&self, seconds: f64) -> Result<(), String> {
        self.target.set(seconds);
        self.watch.seeking.store(true, Ordering::Release);
        let done = self.seek_at(seconds, self.rate.get());
        if done.is_err() {
            self.watch.seeking.store(false, Ordering::Release);
        }
        done
    }

    pub(super) fn set_rate(&self, rate: f64) -> Result<(), String> {
        if rate != 1.0 && !self.tempo {
            return Err(String::from("GStreamer has no scaletempo element, from its good plugins, to keep the sound's pitch at another rate"));
        }
        if !self.watch.opened.load(Ordering::Acquire) {
            self.pending_rate.set(Some(rate));
            return Ok(());
        }
        self.seek_at(self.position(), rate)?;
        self.rate.set(rate);
        Ok(())
    }

    pub(super) fn set_volume(&self, volume: f64) -> Result<(), String> {
        self.set(self.pipeline, c"volume", &volume.to_string())
    }

    pub(super) fn set_muted(&self, muted: bool) -> Result<(), String> {
        self.set(self.pipeline, c"mute", if muted { "true" } else { "false" })
    }

    pub(super) fn set_loop(&self, looped: bool) -> Result<(), String> {
        self.watch.looped.store(looped, Ordering::Release);
        Ok(())
    }

    /// Makes what the bus asked for on the renderer's thread: gives the
    /// pipeline an https clip once it is open, plays the clip
    /// again from its start where it ended while looping, pauses it at its
    /// end, and sets a rate asked for before it prerolled.
    pub(super) fn tend(&self) -> Result<(), String> {
        if let Some(opening) = self.opening.take() {
            match opening.try_recv() {
                Ok(Opened::Remote(remote)) => self.feed(remote)?,
                Ok(Opened::Allowed) => self.engine_fetch()?,
                Err(TryRecvError::Empty) => self.opening.set(Some(opening)),
                // It could not be opened, which its thread told as the
                // player's news.
                Err(TryRecvError::Disconnected) => {}
            }
        }
        if self.watch.restart.swap(false, Ordering::AcqRel) {
            self.seek_at(0.0, self.rate.get())?;
        }
        if self.watch.ended.swap(false, Ordering::AcqRel) {
            self.pause()?;
        }
        if self.watch.opened.load(Ordering::Acquire)
            && let Some(rate) = self.pending_rate.take()
        {
            self.set_rate(rate)?;
        }
        Ok(())
    }

    /// The texts of the clip's own subtitle tracks showing at `position`,
    /// from the cues the text sink has given, its new ones taken first: the
    /// cue it prerolled with while paused, as after a seek, which a pull of
    /// its rendered samples would let go of, then those it rendered.
    pub(super) fn captions(&self, position: f64) -> Result<Vec<String>, String> {
        let Some(text) = self.text else {
            return Ok(Vec::new());
        };
        // SAFETY: the text sink is a live appsink; a sample it gives is
        // this one's, which take_cue releases.
        let preroll = unsafe { self.libs.app.gst_app_sink_try_pull_preroll(text, 0) };
        if !preroll.is_null() {
            self.take_cue(preroll, position)?;
        }
        loop {
            // SAFETY: as above.
            let sample = unsafe { self.libs.app.gst_app_sink_try_pull_sample(text, 0) };
            if sample.is_null() {
                break;
            }
            self.take_cue(sample, position)?;
        }
        Ok(proto::media::subtitles::showing(&self.cues.borrow(), position).into_iter().map(String::from).collect())
    }

    /// Keeps the cue of `sample`, a sample of the text sink, which this
    /// releases; a cue held already, as a preroll is rendered again once
    /// the clip plays, is kept once. `position` is where the clip stands.
    fn take_cue(&self, sample: *mut c_void, position: f64) -> Result<(), String> {
        let gst = &self.libs.gst;
        // SAFETY: a live sample the text sink gave, released once here; its
        // buffer lives as long as it, mapped and unmapped here.
        unsafe {
            let buffer = gst.gst_sample_get_buffer(sample);
            if !buffer.is_null() {
                let (pts, duration) = ((*buffer.cast::<GstBuffer>()).pts, (*buffer.cast::<GstBuffer>()).duration);
                let mut map = GstMapInfo::empty();
                // A cue with no time cannot be shown at one.
                if pts != CLOCK_TIME_NONE {
                    if gst.gst_buffer_map(buffer, &mut map, MAP_READ) == 0 {
                        gst.gst_mini_object_unref(sample);
                        return Err(String::from("the text sink gave a cue whose text could not be read"));
                    }
                    let bytes = if map.data.is_null() { &[][..] } else { std::slice::from_raw_parts(map.data, map.size) };
                    let words = proto::media::subtitles::plain(&String::from_utf8_lossy(bytes));
                    gst.gst_buffer_unmap(buffer, &mut map);
                    // A cue given no length shows until the next one.
                    let start = pts as f64 / NANOS;
                    let end = if duration == CLOCK_TIME_NONE { f64::MAX } else { start + duration as f64 / NANOS };
                    let mut cues = self.cues.borrow_mut();
                    if !cues.iter().any(|c| c.start == start && c.text == words) {
                        proto::trace::line(|| {
                            let until = if end == f64::MAX { String::from("the next") } else { format!("{end:.3} s") };
                            format!("media: the text sink gives a cue from {start:.3} s to {until}, at {position:.3} s: {words:?}")
                        });
                        for open in cues.iter_mut().filter(|c| c.end == f64::MAX) {
                            open.end = start;
                        }
                        cues.push(proto::media::subtitles::Cue { start, end, text: words });
                    }
                }
            }
            gst.gst_mini_object_unref(sample);
        }
        Ok(())
    }

    /// The seconds from the clip's start: where a seek under way goes, as
    /// the pipeline gives none while it flushes; 0 until it can say.
    pub(super) fn position(&self) -> f64 {
        if self.watch.seeking.load(Ordering::Acquire) {
            return self.target.get();
        }
        let mut at = 0i64;
        // SAFETY: the pipeline is a live element.
        let known = unsafe { self.libs.gst.gst_element_query_position(self.pipeline, FORMAT_TIME, &mut at) } != 0;
        if known && at >= 0 { at as f64 / NANOS } else { 0.0 }
    }

    pub(super) fn facts(&self) -> Facts {
        let mut length = 0i64;
        // SAFETY: the pipeline is a live element.
        let known = unsafe { self.libs.gst.gst_element_query_duration(self.pipeline, FORMAT_TIME, &mut length) } != 0;
        let duration = (known && length > 0).then(|| length as f64 / NANOS);
        let video = if self.sink.is_some() && self.int(c"n-video") > 0 { self.size.get().or_else(|| self.negotiated()) } else { None };
        Facts { duration, video, audio: self.int(c"n-audio") > 0 }
    }

    /// The picture's size the sink's pad has negotiated, which it has once
    /// the clip prerolled, before any frame is taken; kept once read.
    fn negotiated(&self) -> Option<[u32; 2]> {
        // SAFETY: the sink is a live element.
        let [width, height] = unsafe { negotiated(&self.libs.gst, self.sink?, c"sink", [c"width", c"height"]) }?;
        let size = [width as u32, height as u32];
        self.size.set(Some(size));
        Some(size)
    }

    /// The newest frame the sink holds: while paused, as at the start and
    /// after a seek, the frame it prerolled with, which a pull of its
    /// rendered samples would let go of; else the last it rendered; the
    /// later of the two where it holds both. None when no new one came.
    pub(super) fn frame(&mut self) -> Result<Option<Frame>, String> {
        let Some(sink) = self.sink else {
            return Ok(None);
        };
        // SAFETY: the sink is a live appsink; a sample it gives is the
        // caller's, which `sample_frame` or the release below lets go of.
        let (preroll, rendered) = unsafe { (self.libs.app.gst_app_sink_try_pull_preroll(sink, 0), self.libs.app.gst_app_sink_try_pull_sample(sink, 0)) };
        let sample = match (preroll.is_null(), rendered.is_null()) {
            (true, true) => return Ok(None),
            (true, false) => rendered,
            (false, true) => preroll,
            (false, false) => {
                let (later, earlier) = if self.pts(rendered) >= self.pts(preroll) { (rendered, preroll) } else { (preroll, rendered) };
                // SAFETY: a sample the sink gave, released once.
                unsafe { self.libs.gst.gst_mini_object_unref(earlier) };
                later
            }
        };
        let frame = self.sample_frame(sample)?;
        self.size.set(Some(frame.size));
        Ok(Some(frame))
    }

    /// The time of the buffer of `sample`, a sample a sink gave that the
    /// caller holds; CLOCK_TIME_NONE for one with no buffer.
    fn pts(&self, sample: *mut c_void) -> u64 {
        // SAFETY: a live sample the caller holds; its buffer lives as long
        // as it.
        unsafe {
            let buffer = self.libs.gst.gst_sample_get_buffer(sample);
            if buffer.is_null() { CLOCK_TIME_NONE } else { (*buffer.cast::<GstBuffer>()).pts }
        }
    }

    /// The frame `sample` holds, which this unrefs.
    fn sample_frame(&self, sample: *mut c_void) -> Result<Frame, String> {
        let gst = &self.libs.gst;
        // SAFETY: a live sample, its caps and buffer read and its buffer
        // mapped and unmapped here.
        let frame = unsafe {
            let caps = gst.gst_sample_get_caps(sample);
            let buffer = gst.gst_sample_get_buffer(sample);
            let (mut width, mut height) = (0, 0);
            if caps.is_null() || buffer.is_null() {
                Err(String::from("the sink gave a frame with no picture"))
            } else if gst.gst_structure_get_int(gst.gst_caps_get_structure(caps, 0), c"width".as_ptr(), &mut width) == 0 || gst.gst_structure_get_int(gst.gst_caps_get_structure(caps, 0), c"height".as_ptr(), &mut height) == 0 {
                Err(String::from("the sink's frame gives no size"))
            } else {
                let mut map = GstMapInfo::empty();
                if gst.gst_buffer_map(buffer, &mut map, MAP_READ) == 0 {
                    Err(String::from("the sink's frame could not be read"))
                } else {
                    let copied = copy_rows(&map, width, height);
                    gst.gst_buffer_unmap(buffer, &mut map);
                    copied
                }
            }
        };
        // SAFETY: the sample the sink gave, released once.
        unsafe { gst.gst_mini_object_unref(sample) };
        frame
    }
}

/// The rows of a mapped frame of `width` by `height` pixels, four bytes a
/// pixel, its stride what its size gives.
///
/// # Safety
/// `map` must be a buffer mapped for reading.
unsafe fn copy_rows(map: &GstMapInfo, width: c_int, height: c_int) -> Result<Frame, String> {
    let size = [u32::try_from(width).map_err(|e| format!("the frame's width {width}: {e}"))?, u32::try_from(height).map_err(|e| format!("the frame's height {height}: {e}"))?];
    let (row, rows) = (size[0] as usize * 4, size[1] as usize);
    let short = || format!("the frame holds {} bytes, short of {rows} rows of {row}", map.size);
    let Some(stride) = map.size.checked_div(rows) else {
        return Err(short());
    };
    if stride < row || map.data.is_null() {
        return Err(short());
    }
    // SAFETY: the mapped buffer holds `map.size` bytes from `map.data`.
    let data = unsafe { std::slice::from_raw_parts(map.data, map.size) };
    let mut bgra = Vec::with_capacity(row * rows);
    for y in 0..rows {
        bgra.extend_from_slice(&data[y * stride..y * stride + row]);
    }
    Ok(Frame { size, bgra })
}

impl Drop for Engine {
    fn drop(&mut self) {
        let gst = &self.libs.gst;
        // SAFETY: the engine's own references, each released once, the
        // pipeline stopped first so no thread of it runs on.
        unsafe {
            if gst.gst_element_set_state(self.pipeline, STATE_NULL) == STATE_CHANGE_FAILURE {
                eprintln!("pwrs-gui: GStreamer could not stop a media player's pipeline");
            }
            gst.gst_object_unref(self.bus);
            for sink in [self.sink, self.text].into_iter().flatten() {
                gst.gst_object_unref(sink);
            }
            gst.gst_object_unref(self.pipeline);
        }
    }
}
