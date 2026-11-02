//! The Windows backend: Media Foundation's media engine, which opens the
//! clip, plays its sound and keeps its clock. In frame-server mode it hands
//! over no window of its own; each frame due is written into a texture
//! shared with wgpu's Direct3D 12 or Vulkan device ([`super::mf_share`]),
//! or where the renderer draws otherwise, copied into a WIC bitmap and from
//! there into the player's texture. Media Foundation is loaded when the first
//! player opens, so the renderer starts on an edition without it, and a
//! player there fails naming what to install.

use super::{Facts, Failure, Frame, News, Opened};
use arc_swap::ArcSwapOption;
use crossbeam_queue::SegQueue;
use proto::media::{Cause, Settings, probe};
use proto::renderer::WakeLoop;
use pwrs_gui_fetch::Remote;
use pwrs_gui_proto as proto;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, E_NOTIMPL, E_UNEXPECTED, RECT, RPC_E_CHANGED_MODE, S_FALSE, S_OK, STG_E_ACCESSDENIED, STG_E_INVALIDFUNCTION};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, GUID_WICPixelFormat32bppBGRA, IWICBitmap, IWICImagingFactory, WICBitmapCacheOnLoad, WICBitmapLockRead, WICRect};
use windows::Win32::Media::MediaFoundation::{
    CLSID_MFMediaEngineClassFactory, IMFAttributes, IMFByteStream, IMFGetService, IMFMediaEngine, IMFMediaEngineClassFactory, IMFMediaEngineEx, IMFMediaEngineNotify, IMFMediaEngineNotify_Impl, IMFTimedText, IMFTimedTextCue,
    IMFTimedTextNotify, IMFTimedTextNotify_Impl, MF_MEDIA_ENGINE_AUDIOONLY, MF_MEDIA_ENGINE_CALLBACK, MF_MEDIA_ENGINE_ERR, MF_MEDIA_ENGINE_ERR_ABORTED, MF_MEDIA_ENGINE_ERR_DECODE, MF_MEDIA_ENGINE_ERR_ENCRYPTED,
    MF_MEDIA_ENGINE_DXGI_MANAGER, MF_MEDIA_ENGINE_ERR_NETWORK, MF_MEDIA_ENGINE_ERR_SRC_NOT_SUPPORTED, MF_MEDIA_ENGINE_EVENT, MF_MEDIA_ENGINE_EVENT_DURATIONCHANGE, MF_MEDIA_ENGINE_EVENT_ENDED, MF_MEDIA_ENGINE_EVENT_ERROR,
    MF_MEDIA_ENGINE_EVENT_FORMATCHANGE, MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA, MF_MEDIA_ENGINE_EVENT_PAUSE, MF_MEDIA_ENGINE_EVENT_PLAYING, MF_MEDIA_ENGINE_EVENT_SEEKED, MF_MEDIA_ENGINE_EVENT_TIMEUPDATE,
    MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC, MF_MEDIA_ENGINE_SEEK_MODE_APPROXIMATE, MF_MEDIA_ENGINE_TIMEDTEXT, MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, MF_TIMED_TEXT_CUE_EVENT, MF_TIMED_TEXT_CUE_EVENT_ACTIVE,
    MF_TIMED_TEXT_CUE_EVENT_CLEAR, MF_TIMED_TEXT_ERROR_CODE, MF_TIMED_TEXT_TRACK_KIND_CAPTIONS, MF_TIMED_TEXT_TRACK_KIND_SUBTITLES, MF_VERSION, MFARGB, MFSTARTUP_FULL,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, ISequentialStream_Impl, IStream, IStream_Impl, LOCKTYPE, STATFLAG, STATSTG, STGC, STGTY_STREAM, STREAM_SEEK, STREAM_SEEK_CUR,
    STREAM_SEEK_END, STREAM_SEEK_SET,
};
use windows::core::{BOOL, BSTR, HRESULT, Interface, Ref, implement};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW};

/// The name the renderer's hello gives this backend.
pub(super) const NAME: &str = "Media Foundation";

/// Whether the backend measures the sound it hands the system: an audio
/// effect of the backend's own in each media engine does.
pub(super) const MEASURES: bool = true;

/// `MFCreateAttributes`, which makes the media engine's settings.
type CreateAttributes = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;

/// `MFCreateMFByteStreamOnStream`, which makes a byte stream of a COM
/// stream for an https clip.
type ByteStreamOnStream = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT;

/// The parts of Media Foundation's platform library a player calls.
#[derive(Clone, Copy)]
struct Platform {
    create_attributes: CreateAttributes,
    byte_stream_on_stream: ByteStreamOnStream,
    create_media_type: super::mf_tap::CreateMediaType,
    /// `MFCreateDXGIDeviceManager`, from Windows 8, for an engine that
    /// decodes on the GPU the window draws with.
    create_manager: Option<super::mf_share::CreateManager>,
}

/// The parts of Media Foundation's platform library a player calls, found
/// once Media Foundation has started; or why it cannot start here.
static PLATFORM: OnceLock<Result<Platform, String>> = OnceLock::new();

/// What a player says where Media Foundation is missing.
const NO_MEDIA_FOUNDATION: &str = "Media Foundation is not installed on this system: Windows N editions take the Media Feature Pack, and Windows Server its Media Foundation feature";

/// What to install where Media Foundation is missing, as a player's remedy.
const INSTALL: &str = "Add-WindowsCapability -Online -Name Media.MediaFeaturePack~~~~0.0.1.0, or on Windows Server Install-WindowsFeature Server-Media-Foundation";

/// Success, or `what` failed with the system's words for `hr`.
fn checked(hr: HRESULT, what: &str) -> Result<(), String> {
    if hr.is_ok() { Ok(()) } else { Err(format!("{what}: {}", windows::core::Error::from(hr).message())) }
}

/// Loads `mfplat.dll` from System32 and starts Media Foundation, once in the
/// process.
fn platform() -> Result<Platform, String> {
    PLATFORM.get_or_init(start_platform).clone()
}

fn start_platform() -> Result<Platform, String> {
    let name: Vec<u16> = "mfplat.dll".encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: the name is a nul-terminated wide string, and the library is
    // kept loaded for the life of the process.
    let module = unsafe { LoadLibraryExW(name.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
    if module.is_null() {
        return Err(format!("{NO_MEDIA_FOUNDATION} ({})", std::io::Error::last_os_error()));
    }
    // SAFETY: each name is a nul-terminated string of an export
    // mfplat.dll has had since Windows 7.
    let export = |name: &std::ffi::CStr| unsafe { GetProcAddress(module, name.as_ptr().cast()) };
    let (Some(startup), Some(attributes), Some(on_stream), Some(media_type)) = (export(c"MFStartup"), export(c"MFCreateAttributes"), export(c"MFCreateMFByteStreamOnStream"), export(c"MFCreateMediaType")) else {
        return Err(format!("{NO_MEDIA_FOUNDATION}: its mfplat.dll lacks MFStartup, MFCreateAttributes, MFCreateMFByteStreamOnStream or MFCreateMediaType"));
    };
    // SAFETY: the exports' own signatures, as mfapi.h declares them.
    let startup: unsafe extern "system" fn(u32, u32) -> HRESULT = unsafe { std::mem::transmute(startup) };
    let create_attributes: CreateAttributes = unsafe { std::mem::transmute(attributes) };
    let byte_stream_on_stream: ByteStreamOnStream = unsafe { std::mem::transmute(on_stream) };
    let create_media_type: super::mf_tap::CreateMediaType = unsafe { std::mem::transmute(media_type) };
    // SAFETY: as above, for an export mfplat.dll has had since Windows 8.
    let create_manager = export(c"MFCreateDXGIDeviceManager").map(|found| unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, super::mf_share::CreateManager>(found) });
    // SAFETY: MFStartup may be called from any thread; it is called once.
    checked(unsafe { startup(MF_VERSION, MFSTARTUP_FULL) }, "Media Foundation did not start")?;
    Ok(Platform { create_attributes, byte_stream_on_stream, create_media_type, create_manager })
}

/// Enters a single-threaded apartment on this thread unless it is in an
/// apartment already, as winit's window thread and the in-process UI
/// thread are.
fn com() -> Result<(), String> {
    // SAFETY: called on the renderer's own thread, whose apartment lasts
    // for the life of the thread.
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr == RPC_E_CHANGED_MODE {
        return Ok(());
    }
    checked(hr, "COM did not start on the renderer's thread")
}

/// The media engine's events, passed to the player as [`News`] and the loop
/// woken for them; they arrive on Media Foundation's own threads. An https
/// clip's stream keeps in `trouble` why the loader last failed to read it,
/// which the clip's error then gives as its cause.
#[implement(IMFMediaEngineNotify)]
struct Notify {
    news: Arc<crossbeam_queue::SegQueue<News>>,
    wake: Option<WakeLoop>,
    trouble: Arc<ArcSwapOption<String>>,
}

impl IMFMediaEngineNotify_Impl for Notify_Impl {
    fn EventNotify(&self, event: u32, param1: usize, param2: u32) -> windows::core::Result<()> {
        let event = MF_MEDIA_ENGINE_EVENT(event as i32);
        let news = match event {
            MF_MEDIA_ENGINE_EVENT_LOADEDMETADATA | MF_MEDIA_ENGINE_EVENT_DURATIONCHANGE | MF_MEDIA_ENGINE_EVENT_FORMATCHANGE => News::Opened,
            MF_MEDIA_ENGINE_EVENT_PLAYING => News::Playing,
            MF_MEDIA_ENGINE_EVENT_PAUSE => News::Paused,
            MF_MEDIA_ENGINE_EVENT_ENDED => News::Ended,
            MF_MEDIA_ENGINE_EVENT_TIMEUPDATE => News::Moved,
            MF_MEDIA_ENGINE_EVENT_SEEKED => News::Sought,
            MF_MEDIA_ENGINE_EVENT_ERROR => {
                let code = MF_MEDIA_ENGINE_ERR(param1 as i32);
                let detail = windows::core::Error::from(HRESULT(param2 as i32)).message();
                match self.trouble.load_full() {
                    // A clip the loader could not read fails for the
                    // network's cause, whatever the engine made of it.
                    Some(why) => News::Failed(Failure::new(Cause::Network, format!("{}; the loader: {why}", failure(code, &detail)))),
                    None => News::Failed(Failure::new(cause(code), failure(code, &detail))),
                }
            }
            _ => return Ok(()),
        };
        self.news.push(news);
        if let Some(wake) = &self.wake {
            wake();
        }
        Ok(())
    }
}

/// The cause a media engine error is, as HTML's `MediaError` numbers the
/// same codes; a protected clip with no key is one that cannot be decoded.
fn cause(code: MF_MEDIA_ENGINE_ERR) -> Cause {
    match code {
        MF_MEDIA_ENGINE_ERR_ABORTED => Cause::Aborted,
        MF_MEDIA_ENGINE_ERR_NETWORK => Cause::Network,
        MF_MEDIA_ENGINE_ERR_SRC_NOT_SUPPORTED => Cause::FormatNotSupported,
        _ => Cause::Decode,
    }
}

/// What a media engine error means, with the detail its HRESULT gives.
fn failure(code: MF_MEDIA_ENGINE_ERR, detail: &str) -> String {
    let what = match code {
        MF_MEDIA_ENGINE_ERR_SRC_NOT_SUPPORTED => "Media Foundation cannot play this file's format",
        MF_MEDIA_ENGINE_ERR_DECODE => "Media Foundation could not decode the clip",
        MF_MEDIA_ENGINE_ERR_NETWORK => "Media Foundation could not read the file",
        MF_MEDIA_ENGINE_ERR_ENCRYPTED => "the clip is protected, and Media Foundation was given no key for it",
        MF_MEDIA_ENGINE_ERR_ABORTED => "Media Foundation stopped reading the clip",
        _ => "Media Foundation could not play the clip",
    };
    if detail.is_empty() { what.to_string() } else { format!("{what}: {detail}") }
}

/// What the media engine's timed text told of the clip's own text tracks,
/// on Media Foundation's threads.
enum Said {
    /// A track was found, by its number.
    Track(u32),
    /// A cue began to show: its number and its lines.
    Shown(u32, String),
    /// A cue stopped showing.
    Hidden(u32),
    /// Every cue stopped showing.
    Cleared,
    /// A cue that began to show could not be read.
    Unread(String),
}

/// The media engine's timed-text events, passed to the engine as [`Said`]
/// and the loop woken for them; a track that cannot be read is a warning
/// of the player.
#[implement(IMFTimedTextNotify)]
struct TextNotify {
    said: Arc<SegQueue<Said>>,
    news: Arc<SegQueue<News>>,
    wake: Option<WakeLoop>,
}

impl TextNotify {
    fn tell(&self, said: Said) {
        self.said.push(said);
        if let Some(wake) = &self.wake {
            wake();
        }
    }
}

impl IMFTimedTextNotify_Impl for TextNotify_Impl {
    fn TrackAdded(&self, trackid: u32) {
        self.tell(Said::Track(trackid));
    }

    fn TrackRemoved(&self, _trackid: u32) {}

    fn TrackSelected(&self, _trackid: u32, _selected: BOOL) {}

    fn TrackReadyStateChanged(&self, _trackid: u32) {}

    fn Error(&self, errorcode: MF_TIMED_TEXT_ERROR_CODE, extendederrorcode: HRESULT, sourcetrackid: u32) {
        let why = windows::core::Error::from(extendederrorcode).message();
        self.news.push(News::Warned(format!("the clip's text track {sourcetrackid} could not be read (timed-text error {}): {why}", errorcode.0)));
        if let Some(wake) = &self.wake {
            wake();
        }
    }

    fn Cue(&self, cueevent: MF_TIMED_TEXT_CUE_EVENT, _currenttime: f64, cue: Ref<IMFTimedTextCue>) {
        if cueevent == MF_TIMED_TEXT_CUE_EVENT_CLEAR {
            self.tell(Said::Cleared);
            return;
        }
        let given: &Option<IMFTimedTextCue> = &cue;
        let Some(cue) = given else {
            self.tell(Said::Unread(String::from("the media engine told of a cue it did not give")));
            return;
        };
        // SAFETY: the cue lives through the call.
        let id = unsafe { cue.GetId() };
        if cueevent != MF_TIMED_TEXT_CUE_EVENT_ACTIVE {
            self.tell(Said::Hidden(id));
            return;
        }
        match lines(cue) {
            Ok(words) => self.tell(Said::Shown(id, words)),
            Err(why) => self.tell(Said::Unread(why)),
        }
    }

    fn Reset(&self) {
        self.tell(Said::Cleared);
    }
}

/// A cue's text, a line of it to each of its lines of formatted text.
fn lines(cue: &IMFTimedTextCue) -> Result<String, String> {
    let mut lines = Vec::new();
    // SAFETY: the cue lives through the calls, and each line's text is read
    // while its line does.
    unsafe {
        for i in 0..cue.GetLineCount() {
            let text = cue.GetLine(i).and_then(|line| line.GetText()).map_err(|e| format!("a cue's line {i} could not be read: {}", e.message()))?;
            lines.push(text.to_string().map_err(|e| format!("a cue's line {i} is not UTF-16: {e}"))?);
        }
    }
    Ok(lines.join("\n"))
}

/// An https clip as a COM stream, read by the loader's byte ranges on Media
/// Foundation's own threads, which may call it from more than one at once;
/// why a read failed goes to `trouble`.
#[implement(IStream)]
struct RemoteStream {
    remote: Mutex<Remote>,
    trouble: Arc<ArcSwapOption<String>>,
}

impl RemoteStream {
    /// The clip, which a read that panicked while it held it leaves unread.
    fn clip(&self) -> windows::core::Result<std::sync::MutexGuard<'_, Remote>> {
        self.remote.lock().map_err(|e| {
            self.trouble.store(Some(Arc::new(format!("a read of the clip broke off: {e}"))));
            windows::core::Error::from(E_UNEXPECTED)
        })
    }
}

impl ISequentialStream_Impl for RemoteStream_Impl {
    /// Reads `cb` bytes into `pv`, or fewer at the clip's end, which is
    /// S_FALSE.
    fn Read(&self, pv: *mut c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
        if pv.is_null() {
            return E_INVALIDARG;
        }
        let mut clip = match self.clip() {
            Ok(c) => c,
            Err(e) => return e.code(),
        };
        // SAFETY: the caller gives `cb` bytes at `pv` to read into.
        let into = unsafe { std::slice::from_raw_parts_mut(pv.cast::<u8>(), cb as usize) };
        let mut read = 0;
        while read < into.len() {
            match clip.read_into(&mut into[read..]) {
                Ok(0) => break,
                Ok(n) => read += n,
                Err(why) => {
                    self.trouble.store(Some(Arc::new(why)));
                    return E_FAIL;
                }
            }
        }
        if !pcbread.is_null() {
            // SAFETY: the caller's count, given to fill.
            unsafe { *pcbread = read as u32 };
        }
        if read < into.len() { S_FALSE } else { S_OK }
    }

    fn Write(&self, _pv: *const c_void, _cb: u32, _pcbwritten: *mut u32) -> HRESULT {
        STG_E_ACCESSDENIED
    }
}

impl IStream_Impl for RemoteStream_Impl {
    fn Seek(&self, dlibmove: i64, dworigin: STREAM_SEEK, plibnewposition: *mut u64) -> windows::core::Result<()> {
        let mut clip = self.clip()?;
        let base = match dworigin {
            STREAM_SEEK_SET => 0,
            STREAM_SEEK_CUR => clip.position(),
            STREAM_SEEK_END => clip.length(),
            _ => return Err(STG_E_INVALIDFUNCTION.into()),
        };
        let at = base.checked_add_signed(dlibmove).ok_or_else(|| windows::core::Error::from(STG_E_INVALIDFUNCTION))?;
        clip.seek_to(at);
        if !plibnewposition.is_null() {
            // SAFETY: the caller's position, given to fill.
            unsafe { *plibnewposition = at };
        }
        Ok(())
    }

    fn SetSize(&self, _libnewsize: u64) -> windows::core::Result<()> {
        Err(STG_E_ACCESSDENIED.into())
    }

    fn CopyTo(&self, _pstm: Ref<IStream>, _cb: u64, _pcbread: *mut u64, _pcbwritten: *mut u64) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Commit(&self, _grfcommitflags: &STGC) -> windows::core::Result<()> {
        Ok(())
    }

    fn Revert(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn LockRegion(&self, _liboffset: u64, _cb: u64, _dwlocktype: &LOCKTYPE) -> windows::core::Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    fn UnlockRegion(&self, _liboffset: u64, _cb: u64, _dwlocktype: u32) -> windows::core::Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }

    /// The clip's length, with no name.
    fn Stat(&self, pstatstg: *mut STATSTG, _grfstatflag: &STATFLAG) -> windows::core::Result<()> {
        if pstatstg.is_null() {
            return Err(E_INVALIDARG.into());
        }
        let length = self.clip()?.length();
        // SAFETY: the caller's statistics, given to fill.
        unsafe { *pstatstg = STATSTG { r#type: STGTY_STREAM.0 as u32, cbSize: length, ..Default::default() } };
        Ok(())
    }

    fn Clone(&self) -> windows::core::Result<IStream> {
        Err(E_NOTIMPL.into())
    }
}

/// The media engine's timed text, which reports the clip's own text tracks
/// and their cues.
struct Text {
    timed: IMFTimedText,
    said: Arc<SegQueue<Said>>,
    /// The cues showing, by their numbers, in the order they began.
    showing: RefCell<Vec<(u32, String)>>,
    /// Whether one of the clip's own tracks of subtitles or captions is
    /// chosen.
    chosen: Cell<bool>,
}

impl Text {
    /// The engine's timed text, its tracks in the clip read, its notes
    /// going to a queue of its own and a track's errors to `news`.
    fn of(engine: &IMFMediaEngine, news: &Arc<SegQueue<News>>, wake: &Option<WakeLoop>) -> Result<Text, String> {
        let said = Arc::new(SegQueue::new());
        let service: IMFGetService = engine.cast().map_err(|e| e.message())?;
        // SAFETY: the engine lives through the calls, on its own thread.
        unsafe {
            let timed: IMFTimedText = service.GetService(&MF_MEDIA_ENGINE_TIMEDTEXT).map_err(|e| e.message())?;
            timed.SetInBandEnabled(true).map_err(|e| e.message())?;
            let notify: IMFTimedTextNotify = TextNotify { said: Arc::clone(&said), news: Arc::clone(news), wake: wake.clone() }.into();
            timed.RegisterNotifications(&notify).map_err(|e| e.message())?;
            Ok(Text { timed, said, showing: RefCell::new(Vec::new()), chosen: Cell::new(false) })
        }
    }

    /// Chooses track `id` when it is one of the clip's own of subtitles or
    /// captions; whether it was.
    fn choose(&self, id: u32) -> Result<bool, String> {
        // SAFETY: the timed text and its list of tracks live through the
        // calls.
        unsafe {
            let tracks = self.timed.GetTextTracks().map_err(|e| e.message())?;
            for i in 0..tracks.GetLength() {
                let track = tracks.GetTrack(i).map_err(|e| e.message())?;
                if track.GetId() != id {
                    continue;
                }
                let kind = track.GetTrackKind();
                if !(track.IsInBand().as_bool() && (kind == MF_TIMED_TEXT_TRACK_KIND_SUBTITLES || kind == MF_TIMED_TEXT_TRACK_KIND_CAPTIONS)) {
                    return Ok(false);
                }
                self.timed.SelectTrack(id, true).map_err(|e| format!("the clip's text track {id} could not be chosen: {}", e.message()))?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The texts of the cues showing now, from what the timed text told.
    fn showing(&self) -> Result<Vec<String>, String> {
        while let Some(said) = self.said.pop() {
            match said {
                Said::Track(id) => {
                    if !self.chosen.get() && self.choose(id)? {
                        self.chosen.set(true);
                    }
                }
                Said::Shown(id, words) => {
                    let mut showing = self.showing.borrow_mut();
                    showing.retain(|(i, _)| *i != id);
                    showing.push((id, words));
                }
                Said::Hidden(id) => self.showing.borrow_mut().retain(|(i, _)| *i != id),
                Said::Cleared => self.showing.borrow_mut().clear(),
                Said::Unread(why) => return Err(why),
            }
        }
        Ok(self.showing.borrow().iter().map(|(_, words)| words.clone()).collect())
    }
}

/// A clip open in Media Foundation's media engine.
pub(super) struct Engine {
    engine: IMFMediaEngine,
    audio_only: bool,
    /// The bitmap each frame is copied into, at the picture's size; made at
    /// the first frame and again when the size changes.
    bitmap: Option<(IWICBitmap, [u32; 2])>,
    wic: Option<IWICImagingFactory>,
    /// The textures each frame is written into, shared with wgpu, while the
    /// renderer draws on hardware with a device that takes them and the
    /// path holds.
    shared: Option<super::mf_share::Shared>,
    /// The engine's timed text, for a clip with a picture where the system
    /// gives it.
    text: Option<Text>,
    /// The clip's file or https address.
    source: String,
    platform: Platform,
    /// The https clip being opened on a thread of its own, which the next
    /// tend gives the engine once it is open.
    opening: Cell<Option<Receiver<Opened>>>,
    /// Why the loader last failed to read the https clip.
    trouble: Arc<ArcSwapOption<String>>,
    /// How far the audio tap moves each block's time, in 100-nanosecond
    /// units, as the clip's edit lists ask.
    shift: Arc<AtomicI64>,
    /// The clock's lead below, in 100-nanosecond units, which the audio tap
    /// takes off each block's time to place it in the clip.
    ahead: Arc<AtomicI64>,
    /// How far the engine's clock runs ahead of the clip's, in seconds:
    /// the media engine reads no edit list, so it plays each track as late
    /// as its list moves it. Positions and seeks are moved back by it.
    clock: Cell<f64>,
    /// The edit lists of an https clip, read on a thread of their own.
    edits_coming: Cell<Option<Receiver<Result<probe::Edits, String>>>>,
    /// An https clip opened, waiting for its edit lists before the engine
    /// is given it, so its clock is right from its start.
    ready: Cell<Option<Opened>>,
    news: Arc<crossbeam_queue::SegQueue<News>>,
}

impl Engine {
    /// Opens `settings`' clip, its events going to `news`, each waking the
    /// loop through `wake`. The engine starts reading a file at once, and an
    /// https clip once the loader has opened it, and raises its opening as
    /// [`News::Opened`].
    pub(super) fn open(settings: &Settings, news: Arc<crossbeam_queue::SegQueue<News>>, wake: Option<WakeLoop>) -> Result<Engine, Failure> {
        let platform = platform().map_err(|text| Failure::missing(text, INSTALL))?;
        let made = Engine::start(settings, platform, Arc::clone(&news), wake.clone()).map_err(|text| Failure::new(Cause::FormatNotSupported, text))?;
        if pwrs_gui_fetch::is_web(&settings.source) {
            made.read_edits(wake.clone())?;
            made.opening.set(Some(super::open_web(&settings.source, settings.engine_fetch, news, wake)?));
            return Ok(made);
        }
        match probe::edits(std::path::Path::new(&settings.source)) {
            Ok(edits) => made.edit(edits),
            Err(why) => made.unedited(&why),
        }
        made.set_source().map_err(|text| Failure::new(Cause::Network, text))?;
        Ok(made)
    }

    /// Reads the https clip's edit lists on a thread of their own, waking
    /// the loop once they are read.
    fn read_edits(&self, wake: Option<WakeLoop>) -> Result<(), Failure> {
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let url = self.source.clone();
        let reader = move || {
            if let Err(gone) = send.send(pwrs_gui_fetch::clip_edits(&url)) {
                // The player closed before its edit lists were read.
                drop(gone.0);
            }
            if let Some(wake) = &wake {
                wake();
            }
        };
        std::thread::Builder::new().name(String::from("pwrs-gui-media-edits")).spawn(reader).map_err(|e| Failure::new(Cause::Aborted, format!("no thread could be started to read its edit lists: {e}")))?;
        self.edits_coming.set(Some(receive));
        Ok(())
    }

    /// Follows the clip's edit lists: the clock runs ahead by the picture's
    /// offset, or the sound's for a clip of sound alone, and the tap moves
    /// the sound to stand where the clock puts it.
    fn edit(&self, edits: probe::Edits) {
        self.clock.set(edits.clock());
        self.ahead.store((edits.clock() * 1e7).round() as i64, Ordering::Release);
        self.shift.store((edits.sound_shift() * 1e7).round() as i64, Ordering::Release);
    }

    /// The clip's edit lists could not be read: it plays as the engine
    /// plays it, which the player warns of.
    fn unedited(&self, why: &str) {
        self.news.push(News::Warned(format!("its edit lists could not be read, so it may play a little late: {why}")));
    }

    /// Has the engine open the clip at its source itself: a file, or an
    /// https clip on Media Foundation's own network stack.
    fn set_source(&self) -> Result<(), String> {
        // SAFETY: the engine, on the thread that made it.
        unsafe { self.engine.SetSource(&BSTR::from(self.source.as_str())) }.map_err(|e| format!("Media Foundation could not open it: {}", e.message()))
    }

    /// Gives the engine the https clip the loader opened, as a byte stream
    /// Media Foundation reads on its own threads; its address names its
    /// format to the engine by its extension.
    fn take_remote(&self, remote: Remote) -> Result<(), String> {
        let stream: IStream = RemoteStream { remote: Mutex::new(remote), trouble: Arc::clone(&self.trouble) }.into();
        let mut raw = std::ptr::null_mut();
        // SAFETY: the export's own signature; the stream lives through the
        // call, and the byte stream it gives is this one's.
        let bytes = unsafe {
            checked((self.platform.byte_stream_on_stream)(stream.as_raw(), &mut raw), "Media Foundation could not read the clip as a stream")?;
            IMFByteStream::from_raw(raw)
        };
        let ex: IMFMediaEngineEx = self.engine.cast().map_err(|e| format!("the media engine takes no stream: {}", e.message()))?;
        // SAFETY: the engine, on the thread that made it.
        unsafe { ex.SetSourceFromByteStream(&bytes, &BSTR::from(self.source.as_str())) }.map_err(|e| format!("Media Foundation could not open it: {}", e.message()))
    }

    /// Makes the media engine `settings` ask for, with no clip yet. A system
    /// whose engine gives no timed text plays the clip without its own
    /// subtitles, and the player warns of it.
    fn start(settings: &Settings, platform: Platform, news: Arc<crossbeam_queue::SegQueue<News>>, wake: Option<WakeLoop>) -> Result<Engine, String> {
        com()?;
        let trouble = Arc::new(ArcSwapOption::empty());
        let notify: IMFMediaEngineNotify = Notify { news: Arc::clone(&news), wake: wake.clone(), trouble: Arc::clone(&trouble) }.into();
        // SAFETY: COM is started on this thread, Media Foundation is
        // started, and every pointer passed lives through its call.
        unsafe {
            let factory: IMFMediaEngineClassFactory =
                CoCreateInstance(&CLSID_MFMediaEngineClassFactory, None, CLSCTX_INPROC_SERVER).map_err(|e| format!("Media Foundation's media engine is not available: {}", e.message()))?;
            let mut raw = std::ptr::null_mut();
            checked((platform.create_attributes)(&mut raw, 2), "the media engine's settings could not be made")?;
            let attributes = IMFAttributes::from_raw(raw);
            attributes.SetUnknown(&MF_MEDIA_ENGINE_CALLBACK, &notify).map_err(|e| e.message())?;
            let mut shared = None;
            let flags = if settings.audio_only {
                MF_MEDIA_ENGINE_AUDIOONLY.0 as u32
            } else {
                attributes.SetUINT32(&MF_MEDIA_ENGINE_VIDEO_OUTPUT_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM.0 as u32).map_err(|e| e.message())?;
                match super::mf_share::Shared::open(platform.create_manager) {
                    Ok(Some(s)) => {
                        attributes.SetUnknown(&MF_MEDIA_ENGINE_DXGI_MANAGER, s.manager()).map_err(|e| e.message())?;
                        proto::trace::line(|| String::from("media: the engine writes its frames into textures shared with wgpu's device"));
                        shared = Some(s);
                    }
                    Ok(None) => {}
                    Err(why) => eprintln!("pwrs-gui-egui: a player's frames come through the CPU: {why}"),
                }
                0
            };
            let engine = factory.CreateInstance(flags, &attributes).map_err(|e| format!("Media Foundation's media engine did not start: {}", e.message()))?;
            let (shift, ahead) = (Arc::new(AtomicI64::new(0)), Arc::new(AtomicI64::new(0)));
            let tap = super::mf_tap::Tap::transform(Arc::clone(&news), Arc::clone(&shift), Arc::clone(&ahead), platform.create_media_type);
            let tapped = engine.cast::<IMFMediaEngineEx>().and_then(|ex| ex.InsertAudioEffect(&tap, true));
            if let Err(e) = tapped {
                news.push(News::Warned(format!("its sound cannot be measured: the media engine took no audio effect: {}", e.message())));
            }
            let text = if settings.audio_only {
                None
            } else {
                match Text::of(&engine, &news, &wake) {
                    Ok(t) => Some(t),
                    Err(why) => {
                        news.push(News::Warned(format!("the clip's own subtitles cannot be shown: the media engine gives no timed text: {why}")));
                        None
                    }
                }
            };
            let made = Engine {
                engine,
                audio_only: settings.audio_only,
                bitmap: None,
                wic: None,
                shared,
                text,
                source: settings.source.clone(),
                platform,
                opening: Cell::new(None),
                trouble,
                shift,
                ahead,
                clock: Cell::new(0.0),
                edits_coming: Cell::new(None),
                ready: Cell::new(None),
                news: Arc::clone(&news),
            };
            let e = &made.engine;
            e.SetAutoPlay(false).map_err(|e| e.message())?;
            e.SetPreload(MF_MEDIA_ENGINE_PRELOAD_AUTOMATIC).map_err(|e| e.message())?;
            made.set_loop(settings.looped)?;
            made.set_muted(settings.muted)?;
            made.set_volume(settings.volume)?;
            e.SetDefaultPlaybackRate(settings.rate).map_err(|e| e.message())?;
            made.set_rate(settings.rate)?;
            Ok(made)
        }
    }

    pub(super) fn play(&self) -> Result<(), String> {
        // SAFETY: the engine is open until it is dropped.
        unsafe { self.engine.Play() }.map_err(|e| e.message())
    }

    pub(super) fn pause(&self) -> Result<(), String> {
        // SAFETY: as for play.
        unsafe { self.engine.Pause() }.map_err(|e| e.message())
    }

    /// Moves to `seconds` of the clip, the engine's clock that much ahead.
    pub(super) fn seek(&self, seconds: f64) -> Result<(), String> {
        // SAFETY: as for play.
        unsafe { self.engine.SetCurrentTime(seconds + self.clock.get()) }.map_err(|e| e.message())
    }

    /// Moves near `seconds`, to the key frame the engine finds first, as a
    /// slider being dragged seeks.
    pub(super) fn seek_near(&self, seconds: f64) -> Result<(), String> {
        let ex: IMFMediaEngineEx = self.engine.cast().map_err(|e| format!("the media engine seeks to the frame alone: {}", e.message()))?;
        // SAFETY: as for play.
        unsafe { ex.SetCurrentTimeEx(seconds + self.clock.get(), MF_MEDIA_ENGINE_SEEK_MODE_APPROXIMATE) }.map_err(|e| e.message())
    }

    pub(super) fn set_rate(&self, rate: f64) -> Result<(), String> {
        // SAFETY: as for play.
        unsafe { self.engine.SetPlaybackRate(rate) }.map_err(|e| format!("Media Foundation does not play at {rate} times: {}", e.message()))
    }

    pub(super) fn set_volume(&self, volume: f64) -> Result<(), String> {
        // SAFETY: as for play.
        unsafe { self.engine.SetVolume(volume) }.map_err(|e| e.message())
    }

    pub(super) fn set_muted(&self, muted: bool) -> Result<(), String> {
        // SAFETY: as for play.
        unsafe { self.engine.SetMuted(muted) }.map_err(|e| e.message())
    }

    pub(super) fn set_loop(&self, looped: bool) -> Result<(), String> {
        // SAFETY: as for play.
        unsafe { self.engine.SetLoop(looped) }.map_err(|e| e.message())
    }

    /// Gives the engine an https clip once it is open and its edit lists
    /// are read: the loader's stream, or its address for the engine to
    /// fetch itself. The media engine loops and stops at the end itself.
    pub(super) fn tend(&self) -> Result<(), String> {
        if let Some(coming) = self.edits_coming.take() {
            match coming.try_recv() {
                Ok(Ok(edits)) => self.edit(edits),
                Ok(Err(why)) => self.unedited(&why),
                Err(TryRecvError::Empty) => self.edits_coming.set(Some(coming)),
                Err(TryRecvError::Disconnected) => self.unedited("the thread reading them ended without them"),
            }
        }
        if let Some(opening) = self.opening.take() {
            match opening.try_recv() {
                Ok(opened) => self.ready.set(Some(opened)),
                Err(TryRecvError::Empty) => self.opening.set(Some(opening)),
                // It could not be opened, which its thread told as the
                // player's news.
                Err(TryRecvError::Disconnected) => {}
            }
        }
        let editing = self.edits_coming.take();
        let waiting = editing.is_some();
        self.edits_coming.set(editing);
        if waiting {
            return Ok(());
        }
        match self.ready.take() {
            Some(Opened::Remote(remote)) => self.take_remote(remote),
            Some(Opened::Allowed) => self.set_source(),
            None => Ok(()),
        }
    }

    /// The texts of the cues showing now of the clip's own first track of
    /// subtitles or captions, as the media engine's timed text times them.
    pub(super) fn captions(&self, _position: f64) -> Result<Vec<String>, String> {
        match &self.text {
            Some(text) => text.showing(),
            None => Ok(Vec::new()),
        }
    }

    /// Seconds from the clip's start: the engine's clock, less how far its
    /// edit lists put it ahead.
    pub(super) fn position(&self) -> f64 {
        // SAFETY: as for play. The engine counts in 100-nanosecond ticks,
        // to which the difference is rounded.
        let at = ((unsafe { self.engine.GetCurrentTime() } - self.clock.get()) * 1e7).round() / 1e7;
        if at.is_finite() { at.max(0.0) } else { 0.0 }
    }

    /// What the engine knows of the clip.
    pub(super) fn facts(&self) -> Facts {
        // SAFETY: as for play; the size's out pointers live through the call.
        unsafe {
            let length = self.engine.GetDuration();
            let duration = (length.is_finite() && length > 0.0).then_some(length);
            let audio = self.engine.HasAudio().as_bool();
            let mut size = [0u32; 2];
            let sized = !self.audio_only && self.engine.HasVideo().as_bool() && self.engine.GetNativeVideoSize(Some(&mut size[0]), Some(&mut size[1])).is_ok();
            let video = (sized && size[0] > 0 && size[1] > 0).then_some(size);
            Facts { duration, video, audio }
        }
    }

    /// The frame due now, when it is a new one, at the picture's own size:
    /// written into a texture shared with wgpu's device, or copied out in
    /// rows top to bottom in blue, green, red and alpha order. A shared path
    /// that fails leaves the engine's frames to the CPU from then on.
    pub(super) fn frame(&mut self) -> Result<Option<Frame>, String> {
        if self.audio_only {
            return Ok(None);
        }
        let mut pts = 0i64;
        // SAFETY: the vtable's own entry, called on the engine it belongs to;
        // the wrapper the crate generates reads S_FALSE, no new frame, as a
        // success with a time, so the result is read here.
        let ticked = unsafe { (Interface::vtable(&self.engine).OnVideoStreamTick)(Interface::as_raw(&self.engine), &mut pts) };
        if ticked != S_OK {
            return checked(ticked, "the media engine has no frame").map(|()| None);
        }
        let Some([width, height]) = self.facts().video else {
            return Ok(None);
        };
        if let Some(shared) = self.shared.as_mut() {
            match shared.write(&self.engine, [width, height]) {
                Ok(written) => return Ok(written.map(|share| Frame { size: [width, height], bgra: Vec::new(), shared: Some(share) })),
                Err(why) => {
                    eprintln!("pwrs-gui-egui: a player's frames come through the CPU from now on: {why}");
                    self.shared = None;
                }
            }
        }
        let bitmap = self.bitmap([width, height])?;
        let whole = RECT { left: 0, top: 0, right: width as i32, bottom: height as i32 };
        let black = MFARGB { rgbBlue: 0, rgbGreen: 0, rgbRed: 0, rgbAlpha: 255 };
        // SAFETY: the bitmap and both rectangles live through the calls, and
        // the lock's pointer is read while the lock is held.
        unsafe {
            self.engine.TransferVideoFrame(&bitmap, None, &whole, Some(&black)).map_err(|e| format!("the frame could not be copied out: {}", e.message()))?;
            let lock = bitmap.Lock(&WICRect { X: 0, Y: 0, Width: width as i32, Height: height as i32 }, WICBitmapLockRead.0 as u32).map_err(|e| e.message())?;
            let stride = lock.GetStride().map_err(|e| e.message())? as usize;
            let mut length = 0u32;
            let mut data = std::ptr::null_mut();
            lock.GetDataPointer(&mut length, &mut data).map_err(|e| e.message())?;
            let row = width as usize * 4;
            if data.is_null() || stride < row || (length as usize) < stride * (height as usize - 1) + row {
                return Err(format!("the frame's bitmap holds {length} bytes, short of {height} rows of {row}"));
            }
            let bytes = std::slice::from_raw_parts(data, length as usize);
            let mut bgra = Vec::with_capacity(row * height as usize);
            for y in 0..height as usize {
                bgra.extend_from_slice(&bytes[y * stride..y * stride + row]);
            }
            Ok(Some(Frame { size: [width, height], bgra, shared: None }))
        }
    }

    /// The bitmap frames are copied into, at `size`.
    fn bitmap(&mut self, size: [u32; 2]) -> Result<IWICBitmap, String> {
        if let Some((bitmap, made)) = &self.bitmap
            && *made == size
        {
            return Ok(bitmap.clone());
        }
        let wic = match &self.wic {
            Some(w) => w.clone(),
            None => {
                // SAFETY: COM is started on this thread.
                let made: IWICImagingFactory = unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }.map_err(|e| format!("Windows Imaging Component is not available: {}", e.message()))?;
                self.wic = Some(made.clone());
                made
            }
        };
        // SAFETY: the format's GUID lives through the call.
        let bitmap = unsafe { wic.CreateBitmap(size[0], size[1], &GUID_WICPixelFormat32bppBGRA, WICBitmapCacheOnLoad) }.map_err(|e| format!("a bitmap for the frames could not be made: {}", e.message()))?;
        self.bitmap = Some((bitmap.clone(), size));
        Ok(bitmap)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: the engine is shut down once, as it is let go of.
        if let Err(e) = unsafe { self.engine.Shutdown() } {
            eprintln!("pwrs-gui-egui: a media engine did not shut down: {}", e.message());
        }
    }
}
