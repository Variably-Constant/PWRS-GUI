//! The AVFoundation backend, for macOS: an `AVPlayer` opens the clip,
//! plays its sound and keeps its clock, and an `AVPlayerItemVideoOutput`
//! hands over each frame of the picture in blue, green, red and alpha
//! order: where the renderer draws with Metal on hardware, as a Metal
//! texture of its pixel buffer through a Core Video texture cache, which a
//! copy on the GPU takes into the player's texture, and otherwise copied
//! out through the CPU. An audio tap measures the sound, and an
//! `AVPlayerItemLegibleOutput` gives the texts of the cues of the
//! clip's first track of subtitles or captions. The player's item and its
//! state are watched by key-value observing, and its end by the
//! notification center. An https clip's
//! asset asks a resource loader of the backend's own for its bytes, which
//! the loader fetches by byte ranges.

use super::{Facts, Failure, Frame, News, Opened};
use arc_swap::ArcSwapOption;
use crossbeam_queue::SegQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, MainThreadMarker, define_class, msg_send, sel};
use objc2_av_foundation::{
    AVAssetResourceLoader, AVAssetResourceLoaderDelegate, AVAssetResourceLoadingRequest, AVAudioMixInputParameters, AVAudioTimePitchAlgorithmSpectral, AVFoundationErrorDomain, AVMediaCharacteristicLegible,
    AVMediaTypeAudio, AVMutableAudioMix, AVMutableAudioMixInputParameters, AVPlayer, AVPlayerItem, AVPlayerItemDidPlayToEndTimeNotification, AVPlayerItemFailedToPlayToEndTimeNotification,
    AVPlayerItemLegibleOutput, AVPlayerItemLegibleOutputPushDelegate, AVPlayerItemOutputPushDelegate, AVPlayerItemStatus, AVPlayerItemVideoOutput, AVPlayerTimeControlStatus, AVURLAsset,
};
use objc2_core_audio_types::{AudioBuffer, AudioBufferList, AudioStreamBasicDescription, kAudioFormatFlagIsFloat};
use objc2_core_foundation::{CFRetained, CFString};
use objc2_media_toolbox::{MTAudioProcessingTap, MTAudioProcessingTapCallbacks, MTAudioProcessingTapFlags, kMTAudioProcessingTapCallbacksVersion_0, kMTAudioProcessingTapCreationFlag_PostEffects};
use objc2_core_media::{CMTime, CMTimeFlags, CMTimeRange, kCMTimeZero};
use objc2_core_video::{
    CVMetalTexture, CVMetalTextureCache, CVMetalTextureGetTexture, CVPixelBuffer, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress, kCVPixelBufferMetalCompatibilityKey, kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA, kCVReturnSuccess,
};
use objc2_metal::{MTLPixelFormat, MTLTextureType};
use objc2_foundation::{
    NSArray, NSAttributedString, NSData, NSDictionary, NSError, NSKeyValueObservingOptions, NSNotification, NSNotificationCenter, NSNumber, NSObjectNSKeyValueObserverRegistration, NSString, NSURL, ns_string,
};
use proto::media::{Cause, Settings};
use proto::renderer::WakeLoop;
use pwrs_gui_fetch::{FIRST_PART, Remote};
use pwrs_gui_proto as proto;
use std::cell::Cell;
use std::ffi::{c_char, c_void};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};

/// The name the renderer's hello gives this backend.
pub(super) const NAME: &str = "AVFoundation";

/// Whether the backend measures the sound it hands the system: an audio
/// tap on the item's sound does.
pub(super) const MEASURES: bool = true;

/// The timescale a seek's time is given in: a frame of any common rate
/// falls on one of its ticks.
const TIMESCALE: i32 = 600;

/// AVFoundation's codes for a file whose format or codec it cannot play.
const UNPLAYABLE: [isize; 3] = [-11828, -11829, -11833];

/// What the player's observer tells the player through, from the threads
/// AVFoundation tells it on, and what the engine asks of the next pump.
struct Watch {
    news: Arc<SegQueue<News>>,
    wake: Option<WakeLoop>,
    opened: AtomicBool,
    looped: AtomicBool,
    /// Whether the clip ended while looping, so the pump starts it again.
    restart: AtomicBool,
    /// Whether the player paused, which the pump tells unless the clip is
    /// at its end.
    paused: AtomicBool,
    /// Whether the item failed, so the pump reads its error.
    failed: AtomicBool,
    /// Why the resource loader last failed to give an https clip's bytes,
    /// and the cause it is, which the item's failure then gives.
    trouble: ArcSwapOption<(Cause, String)>,
    /// Whether the clip's opening is held back until its server is found to
    /// let the engine fetch it, and whether it opened meanwhile.
    held: AtomicBool,
    ready: AtomicBool,
    /// The texts of the clip's own cues showing now, as its legible output
    /// last gave them.
    cues: ArcSwapOption<Vec<String>>,
    /// How many seeks have been asked for; the number of the one under
    /// way, 0 for none; and where it goes, the bits of its seconds, which
    /// the position gives until it ends, as AVFoundation seeks on its own.
    seeks: AtomicU64,
    seeking: AtomicU64,
    target: AtomicU64,
}

impl Watch {
    fn tell(&self, news: News) {
        self.news.push(news);
        self.wake_loop();
    }

    /// Tells the clip opened, once nothing holds its opening back: from
    /// the observer as it opens, or as the hold ends, whichever comes last.
    fn opened_unless_held(&self) {
        if !self.held.load(Ordering::SeqCst) && self.ready.swap(false, Ordering::SeqCst) {
            self.tell(News::Opened);
        }
    }

    fn wake_loop(&self) {
        if let Some(wake) = &self.wake {
            wake();
        }
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class
    // implements no method NSObject's own callers rely on otherwise.
    #[unsafe(super(NSObject))]
    #[name = "PwrsGuiMediaObserver"]
    #[ivars = Arc<Watch>]
    struct Observer;

    impl Observer {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(&self, key_path: Option<&NSString>, object: Option<&AnyObject>, _change: Option<&AnyObject>, _context: *mut c_void) {
            let watch = self.ivars();
            let Some(key) = key_path else {
                return;
            };
            if key.to_string() == "status" {
                // SAFETY: the item this observes is the object here.
                let status = object.and_then(|o| o.downcast_ref::<AVPlayerItem>()).map(|item| unsafe { item.status() });
                match status {
                    Some(AVPlayerItemStatus::ReadyToPlay) if !watch.opened.swap(true, Ordering::AcqRel) => {
                        watch.ready.store(true, Ordering::SeqCst);
                        watch.opened_unless_held();
                    }
                    Some(AVPlayerItemStatus::Failed) => {
                        watch.failed.store(true, Ordering::Release);
                        watch.wake_loop();
                    }
                    _ => {}
                }
            } else if key.to_string() == "duration" && watch.opened.load(Ordering::Acquire) && !watch.held.load(Ordering::SeqCst) {
                watch.tell(News::Opened);
            } else if key.to_string() == "timeControlStatus" {
                // SAFETY: the player this observes is the object here.
                let status = object.and_then(|o| o.downcast_ref::<AVPlayer>()).map(|player| unsafe { player.timeControlStatus() });
                match status {
                    Some(AVPlayerTimeControlStatus::Playing) => watch.tell(News::Playing),
                    Some(AVPlayerTimeControlStatus::Paused) => {
                        watch.paused.store(true, Ordering::Release);
                        watch.wake_loop();
                    }
                    _ => {}
                }
            }
        }

        #[unsafe(method(ended:))]
        fn ended(&self, _note: &NSNotification) {
            let watch = self.ivars();
            if watch.looped.load(Ordering::Acquire) {
                watch.restart.store(true, Ordering::Release);
                watch.wake_loop();
            } else {
                watch.tell(News::Ended);
            }
        }

        #[unsafe(method(stalled:))]
        fn stalled(&self, _note: &NSNotification) {
            let watch = self.ivars();
            watch.failed.store(true, Ordering::Release);
            watch.wake_loop();
        }
    }
);

impl Observer {
    fn new(watch: Arc<Watch>) -> Retained<Observer> {
        let this = Observer::alloc().set_ivars(watch);
        // SAFETY: NSObject's init, on an observer whose ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

unsafe extern "C" {
    /// libdispatch's own: a new queue, serial where `attr` is null, which
    /// the caller releases.
    fn dispatch_queue_create(label: *const c_char, attr: *mut c_void) -> *mut c_void;
    fn dispatch_release(object: *mut c_void);
}

/// A serial dispatch queue of the engine's own, released as it drops.
struct Queue(*mut c_void);

impl Drop for Queue {
    fn drop(&mut self) {
        // SAFETY: the queue this made, released once.
        unsafe { dispatch_release(self.0) };
    }
}

/// What the resource loader of an https clip answers AVFoundation's
/// requests from, on its queue: the clip, opened through the loader at the
/// first request.
struct Load {
    url: String,
    remote: Mutex<Option<Remote>>,
    watch: Arc<Watch>,
}

impl Load {
    /// Answers `request`: the clip's type, length and byte ranges where it
    /// asks for them, and its bytes from where it asks to where it asks.
    fn answer(&self, request: &AVAssetResourceLoadingRequest) -> Result<(), (Cause, String)> {
        let network = |why: String| (Cause::Network, why);
        let mut held = self.remote.lock().map_err(|e| network(format!("a read of the clip broke off: {e}")))?;
        let remote = match &mut *held {
            Some(r) => r,
            empty => empty.insert(Remote::open(&self.url).map_err(network)?),
        };
        let length = remote.length();
        // SAFETY: the request AVFoundation gave, answered on its queue.
        unsafe {
            if let Some(info) = request.contentInformationRequest() {
                info.setContentType(Some(&NSString::from_str(uniform_type(remote)?)));
                info.setContentLength(i64::try_from(length).map_err(|e| network(format!("its length of {length} bytes is more than AVFoundation takes: {e}")))?);
                info.setByteRangeAccessSupported(true);
            }
            let Some(data) = request.dataRequest() else {
                return Ok(());
            };
            let start = u64::try_from(data.requestedOffset()).map_err(|e| network(format!("AVFoundation asked for bytes from {}: {e}", data.requestedOffset())))?;
            let end = if data.requestsAllDataToEndOfResource() {
                length
            } else {
                let count = u64::try_from(data.requestedLength()).map_err(|e| network(format!("AVFoundation asked for {} bytes: {e}", data.requestedLength())))?;
                start.saturating_add(count).min(length)
            };
            remote.seek_to(start);
            let mut part = vec![0u8; FIRST_PART as usize];
            let mut at = start;
            while at < end {
                let want = ((end - at) as usize).min(part.len());
                let n = remote.read_into(&mut part[..want]).map_err(network)?;
                if n == 0 {
                    break;
                }
                data.respondWithData(&NSData::with_bytes(&part[..n]));
                at += n as u64;
            }
        }
        Ok(())
    }
}

/// The uniform type AVFoundation reads a clip as, from the container its
/// headers name; a container it plays from no stream is not supported.
fn uniform_type(remote: &mut Remote) -> Result<&'static str, (Cause, String)> {
    let length = remote.length();
    let mut fetch = |at: u64, count: u64| -> Result<Vec<u8>, String> {
        remote.seek_to(at);
        let mut bytes = vec![0u8; count.min(length.saturating_sub(at)) as usize];
        let mut read = 0;
        while read < bytes.len() {
            let n = remote.read_into(&mut bytes[read..])?;
            if n == 0 {
                break;
            }
            read += n;
        }
        bytes.truncate(read);
        Ok(bytes)
    };
    let format = proto::media::probe::read_ranges(Some(length), &mut fetch).map_err(|why| (Cause::Network, format!("its headers could not be read: {why}")))?;
    Ok(match (format.container.as_str(), format.video.is_some()) {
        ("MP4", false) => "com.apple.m4a-audio",
        ("MP4", true) => "public.mpeg-4",
        ("QuickTime", _) => "com.apple.quicktime-movie",
        ("3GP", _) => "public.3gpp",
        ("MP3", _) => "public.mp3",
        ("ADTS", _) => "public.aac-audio",
        ("WAV", _) => "com.microsoft.waveform-audio",
        ("AIFF", _) => "public.aiff-audio",
        ("CAF", _) => "com.apple.coreaudio-format",
        ("AC-3", _) => "public.ac3-audio",
        ("AMR", _) => "org.3gpp.adaptive-multi-rate-audio",
        ("FLAC", _) => "org.xiph.flac",
        _ => return Err((Cause::FormatNotSupported, format!("its headers name {}, which AVFoundation does not play", format.name()))),
    })
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class adds
    // the resource loader's delegate method alone.
    #[unsafe(super(NSObject))]
    #[name = "PwrsGuiMediaLoader"]
    #[ivars = Load]
    struct Loader;

    unsafe impl NSObjectProtocol for Loader {}

    unsafe impl AVAssetResourceLoaderDelegate for Loader {
        /// Answers each request at once, on the loader's queue; a request
        /// that cannot be answered fails, and the item's failure then says
        /// why.
        #[unsafe(method(resourceLoader:shouldWaitForLoadingOfRequestedResource:))]
        fn should_wait(&self, _loader: &AVAssetResourceLoader, request: &AVAssetResourceLoadingRequest) -> bool {
            let load = self.ivars();
            match load.answer(request) {
                // SAFETY: the request answered in full.
                Ok(()) => unsafe { request.finishLoading() },
                Err(trouble) => {
                    load.watch.trouble.store(Some(Arc::new(trouble)));
                    let error = NSError::new(-1, ns_string!("PWRS-GUI media loader"));
                    // SAFETY: the request that could not be answered.
                    unsafe { request.finishLoadingWithError(Some(&error)) };
                }
            }
            true
        }
    }
);

impl Loader {
    fn new(load: Load) -> Retained<Loader> {
        let this = Loader::alloc().set_ivars(load);
        // SAFETY: NSObject's init, on a loader whose ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class adds
    // the legible output's delegate method alone.
    #[unsafe(super(NSObject))]
    #[name = "PwrsGuiMediaCaptions"]
    #[ivars = Arc<Watch>]
    struct Captions;

    unsafe impl NSObjectProtocol for Captions {}

    unsafe impl AVPlayerItemOutputPushDelegate for Captions {}

    unsafe impl AVPlayerItemLegibleOutputPushDelegate for Captions {
        /// Keeps the texts of the cues the legible output gives, those to
        /// show from now on, and wakes the loop to draw them.
        #[unsafe(method(legibleOutput:didOutputAttributedStrings:nativeSampleBuffers:forItemTime:))]
        fn given(&self, _output: &AVPlayerItemLegibleOutput, strings: &NSArray<NSAttributedString>, _native: &NSArray, _at: CMTime) {
            let texts: Vec<String> = strings.iter().map(|s| s.string().to_string()).filter(|t| !t.trim().is_empty()).collect();
            let watch = self.ivars();
            watch.cues.store(Some(Arc::new(texts)));
            watch.wake_loop();
        }
    }
);

impl Captions {
    fn new(watch: Arc<Watch>) -> Retained<Captions> {
        let this = Captions::alloc().set_ivars(watch);
        // SAFETY: NSObject's init, on a delegate whose ivars are set.
        unsafe { msg_send![super(this), init] }
    }
}

/// The item's legible output, which gives the clip's own cues as text, its
/// delegate and the queue it is told on, held as long as the engine.
struct Legible {
    _output: Retained<AVPlayerItemLegibleOutput>,
    _delegate: Retained<Captions>,
    _queue: Queue,
}

/// Puts a legible output on `item`, telling `watch` the texts of the cues
/// of the clip's own subtitles, once one of its tracks is chosen.
fn legible(item: &AVPlayerItem, watch: &Arc<Watch>) -> Result<Legible, String> {
    // SAFETY: a nul-terminated label; null attributes make a serial queue.
    let made = unsafe { dispatch_queue_create(c"PWRS-GUI media captions".as_ptr(), std::ptr::null_mut()) };
    if made.is_null() {
        return Err(String::from("no dispatch queue could be made for the clip's own subtitles"));
    }
    let queue = Queue(made);
    let delegate = Captions::new(Arc::clone(watch));
    let told: &ProtocolObject<dyn AVPlayerItemLegibleOutputPushDelegate> = ProtocolObject::from_ref(&*delegate);
    // SAFETY: an output of every legible track as attributed strings, which
    // draws none itself, given a delegate and a serial queue the engine
    // keeps as long as the output, and added to the item it reads.
    let output = unsafe {
        let output = AVPlayerItemLegibleOutput::initWithMediaSubtypesForNativeRepresentation(AVPlayerItemLegibleOutput::alloc(), &NSArray::new());
        output.setSuppressesPlayerRendering(true);
        let () = msg_send![&*output, setDelegate: told, queue: queue.0.cast::<AnyObject>()];
        item.addOutput(&output);
        output
    };
    Ok(Legible { _output: output, _delegate: delegate, _queue: queue })
}

/// What an audio tap's callbacks reach, kept as its storage: the watch its
/// measures go to, whether the samples it is handed are 32-bit floats, the
/// format an `AVPlayer` hands its taps, and their rate, the bits of the
/// samples a second of each channel.
struct Measure {
    watch: Arc<Watch>,
    floats: AtomicBool,
    rate: AtomicU64,
}

/// The measure of `tap`, kept in its storage by [`tap_init`].
///
/// # Safety
///
/// `tap` is a tap [`Engine::tap_sound`] made, not yet finalized.
unsafe fn measure_of<'a>(tap: NonNull<MTAudioProcessingTap>) -> &'a Measure {
    // SAFETY: as the caller promises, the storage holds the boxed measure.
    unsafe { &*tap.as_ref().storage().as_ptr().cast::<Measure>() }
}

/// Keeps the tap's client, its boxed measure, as its storage.
unsafe extern "C-unwind" fn tap_init(_tap: NonNull<MTAudioProcessingTap>, client: *mut c_void, storage: NonNull<*mut c_void>) {
    // SAFETY: the tap's storage slot, given for this call.
    unsafe { storage.as_ptr().write(client) };
}

/// Frees the tap's measure as the tap goes.
unsafe extern "C-unwind" fn tap_finalize(tap: NonNull<MTAudioProcessingTap>) {
    // SAFETY: the storage init kept: the measure boxed for this tap, freed
    // once, here.
    drop(unsafe { Box::from_raw(tap.as_ref().storage().as_ptr().cast::<Measure>()) });
}

/// Notes whether the samples the tap will be handed are 32-bit floats, and
/// their rate.
unsafe extern "C-unwind" fn tap_prepare(tap: NonNull<MTAudioProcessingTap>, _most: isize, format: NonNull<AudioStreamBasicDescription>) {
    // SAFETY: a tap of this module's, and the format given for this call.
    let (measure, format) = unsafe { (measure_of(tap), format.as_ref()) };
    measure.floats.store(format.mFormatFlags & kAudioFormatFlagIsFloat != 0 && format.mBitsPerChannel == 32, Ordering::Release);
    measure.rate.store(format.mSampleRate.to_bits(), Ordering::Release);
}

unsafe extern "C-unwind" fn tap_unprepare(_tap: NonNull<MTAudioProcessingTap>) {}

/// Takes the item's sound through the tap unchanged, and tells the highest
/// sample of each block as the player's news, with the span of the clip the
/// block holds: from where it starts, for as long as its frames last at the
/// rate the tap was prepared with.
unsafe extern "C-unwind" fn tap_process(tap: NonNull<MTAudioProcessingTap>, frames: isize, _flags: MTAudioProcessingTapFlags, buffers: NonNull<AudioBufferList>, frames_out: NonNull<isize>, flags_out: NonNull<MTAudioProcessingTapFlags>) {
    // SAFETY: Core Media's own constant, read.
    let zero = unsafe { kCMTimeZero };
    let mut range = CMTimeRange { start: zero, duration: zero };
    // SAFETY: a tap of this module's, whose source audio fills the buffers,
    // counts and time range given for this call.
    let (read, measure) = unsafe { (tap.as_ref().source_audio(frames, buffers, flags_out.as_ptr(), &mut range, frames_out.as_ptr()), measure_of(tap)) };
    let Some(at) = seconds(range.start).filter(|_| read == 0 && measure.floats.load(Ordering::Acquire)) else {
        return;
    };
    // SAFETY: the count of frames the source audio wrote for this call.
    let given = unsafe { frames_out.as_ptr().read() };
    let rate = f64::from_bits(measure.rate.load(Ordering::Acquire));
    let until = if rate > 0.0 { at + given as f64 / rate } else { at };
    // SAFETY: the list holds mNumberBuffers buffers in a row from its first,
    // each of mDataByteSize bytes of 32-bit floats, for this call.
    let peak = unsafe {
        let count = (*buffers.as_ptr()).mNumberBuffers as usize;
        let first = std::ptr::addr_of!((*buffers.as_ptr()).mBuffers).cast::<AudioBuffer>();
        std::slice::from_raw_parts(first, count)
            .iter()
            .filter(|b| !b.mData.is_null())
            .flat_map(|b| std::slice::from_raw_parts(b.mData.cast::<f32>(), (b.mDataByteSize >> 2) as usize))
            .fold(0.0f32, |most, s| most.max(s.abs()))
    };
    measure.watch.news.push(News::Heard { at, until, peak });
}

/// wgpu's device where the renderer draws with Metal on hardware: frames
/// are then handed to it as Metal textures of the output's pixel buffers,
/// which a copy on the GPU takes into the player's texture.
static METAL: ArcSwapOption<wgpu::Device> = ArcSwapOption::const_empty();

/// How many devices the renderer has drawn with, so a texture cache made
/// for an earlier one is made again.
static DEVICES: AtomicU64 = AtomicU64::new(0);

/// Records the device the renderer draws with from now on.
pub(crate) fn link(adapter: &wgpu::Adapter, device: &wgpu::Device) {
    DEVICES.fetch_add(1, Ordering::AcqRel);
    let info = adapter.get_info();
    let metal = info.backend == wgpu::Backend::Metal && info.device_type != wgpu::DeviceType::Cpu;
    METAL.store(metal.then(|| Arc::new(device.clone())));
}

/// Core Video's objects a frame on the GPU keeps alive, never read: its
/// Metal texture and the pixel buffer under it.
struct Held {
    _texture: CFRetained<CVMetalTexture>,
    _buffer: Retained<CVPixelBuffer>,
}

// SAFETY: Core Video's objects may be retained and released on any thread,
// which is all a frame does with them.
unsafe impl Send for Held {}
// SAFETY: as above; they are only held.
unsafe impl Sync for Held {}

/// A frame handed to wgpu as the Metal texture of the output's pixel
/// buffer, kept with the buffer until the copy out of it has run.
pub(crate) struct MetalFrame {
    texture: wgpu::Texture,
    _held: Held,
    copied: AtomicU64,
}

impl MetalFrame {
    /// Records a copy of the frame into `target`, the player's own texture,
    /// among `encoder`'s commands for the draw of egui frame `draw`.
    pub(crate) fn copy(&self, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Texture, draw: u64) {
        encoder.copy_texture_to_texture(self.texture.as_image_copy(), target.as_image_copy(), self.texture.size());
        self.copied.store(super::copies::recorded(queue, draw), Ordering::Release);
    }
}

/// Whether `frame` may be let go of: its copy has run, or it was never
/// copied and nothing else holds it.
fn let_go(frame: &Arc<MetalFrame>) -> bool {
    match frame.copied.load(Ordering::Acquire) {
        0 => Arc::strong_count(frame) == 1,
        copied => super::copies::finished(copied),
    }
}

/// An https clip's resource loader, which takes the stream the loader
/// opened off its queue, and the queue it answers on, held as long as its
/// asset, which holds neither.
struct Loading {
    loader: Retained<Loader>,
    _queue: Queue,
}

impl Loading {
    /// Hands the resource loader the clip's stream, opened through the
    /// loader on a thread of its own; a request that came first opened one
    /// itself, which it keeps.
    fn take(&self, remote: Remote) -> Result<(), String> {
        let mut held = self.loader.ivars().remote.lock().map_err(|e| format!("the resource loader broke off: {e}"))?;
        if held.is_none() {
            *held = Some(remote);
        }
        Ok(())
    }
}

/// The item of an https clip: an asset at the clip's address under a
/// scheme of the backend's own, so AVFoundation asks the resource loader
/// for every byte, with the loader and the queue it answers on.
fn web_item(source: &str, watch: &Arc<Watch>, mtm: MainThreadMarker) -> Result<(Retained<AVPlayerItem>, Loading), Failure> {
    let address = format!("pwrs-{source}");
    let url = NSURL::URLWithString(&NSString::from_str(&address)).ok_or_else(|| Failure::new(Cause::Network, format!("{source} is no address AVFoundation takes")))?;
    // SAFETY: an address and no options, on the main thread.
    let asset = unsafe { AVURLAsset::URLAssetWithURL_options(&url, None) };
    // SAFETY: a nul-terminated label; null attributes make a serial queue.
    let made = unsafe { dispatch_queue_create(c"PWRS-GUI media loader".as_ptr(), std::ptr::null_mut()) };
    if made.is_null() {
        return Err(Failure::new(Cause::Aborted, String::from("no dispatch queue could be made for the clip's resource loader")));
    }
    let queue = Queue(made);
    let loader = Loader::new(Load { url: source.to_string(), remote: Mutex::new(None), watch: Arc::clone(watch) });
    let delegate: &ProtocolObject<dyn AVAssetResourceLoaderDelegate> = ProtocolObject::from_ref(&*loader);
    // SAFETY: the asset's own resource loader, given a delegate and a
    // serial queue the engine keeps as long as the asset.
    unsafe {
        let resources = asset.resourceLoader();
        let () = msg_send![&*resources, setDelegate: delegate, queue: queue.0.cast::<AnyObject>()];
    }
    // SAFETY: the asset just made, on the main thread.
    let item = unsafe { AVPlayerItem::playerItemWithAsset(&asset, mtm) };
    Ok((item, Loading { loader, _queue: queue }))
}

/// One clip open in an `AVPlayer`.
pub(super) struct Engine {
    player: Retained<AVPlayer>,
    item: Retained<AVPlayerItem>,
    /// The output the picture's frames are taken from; none for sound
    /// alone.
    output: Option<Retained<AVPlayerItemVideoOutput>>,
    observer: Retained<Observer>,
    rate: Cell<f64>,
    /// An https clip's resource loader and the queue it answers on.
    loader: Option<Loading>,
    /// Whether the server of an https clip AVFoundation fetches itself lets
    /// it, as a thread of its own finds; the clip's opening waits for it.
    opening: Cell<Option<Receiver<Opened>>>,
    /// The output of the clip's own cues, for a clip with a picture, and
    /// whether its first track of subtitles or captions is chosen yet.
    legible: Option<Legible>,
    chosen: Cell<bool>,
    /// Whether the item's sound has been given its audio tap, or found to
    /// take none.
    tapped: Cell<bool>,
    /// The texture cache frames become Metal textures through, made for the
    /// device the renderer drew with when it was made, which it names by
    /// its count; whether the path is given up; and the frames handed to
    /// wgpu that it may still be copying out of.
    textures: Option<(u64, CFRetained<CVMetalTextureCache>)>,
    no_metal: bool,
    kept: Vec<Arc<MetalFrame>>,
}

impl Engine {
    /// Opens `settings`' clip in an `AVPlayer`, its observer telling `news`
    /// and waking the loop through `wake`.
    pub(super) fn open(settings: &Settings, news: Arc<SegQueue<News>>, wake: Option<WakeLoop>) -> Result<Engine, Failure> {
        let mtm = MainThreadMarker::new().ok_or_else(|| Failure::new(Cause::Aborted, String::from("an AVFoundation player opens on the main thread alone")))?;
        let watch = Arc::new(Watch {
            news,
            wake,
            opened: AtomicBool::new(false),
            looped: AtomicBool::new(settings.looped),
            restart: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            trouble: ArcSwapOption::empty(),
            held: AtomicBool::new(settings.engine_fetch && pwrs_gui_fetch::is_web(&settings.source)),
            ready: AtomicBool::new(false),
            cues: ArcSwapOption::empty(),
            seeks: AtomicU64::new(0),
            seeking: AtomicU64::new(0),
            target: AtomicU64::new(0),
        });
        let mut opening = None;
        let (item, loader) = if pwrs_gui_fetch::is_web(&settings.source) && settings.engine_fetch {
            // AVFoundation fetches the clip itself, its opening held back
            // until its server is found to let it.
            opening = Some(super::open_web(&settings.source, true, Arc::clone(&watch.news), watch.wake.clone())?);
            let url = NSURL::URLWithString(&NSString::from_str(&settings.source)).ok_or_else(|| Failure::new(Cause::Network, format!("{} is no address AVFoundation takes", settings.source)))?;
            // SAFETY: an https address, on the main thread.
            (unsafe { AVPlayerItem::playerItemWithURL(&url, mtm) }, None)
        } else if pwrs_gui_fetch::is_web(&settings.source) {
            // The loader opens the clip's stream on a thread of its own, which
            // the resource loader then answers AVFoundation's requests from.
            opening = Some(super::open_web(&settings.source, false, Arc::clone(&watch.news), watch.wake.clone())?);
            let (item, loader) = web_item(&settings.source, &watch, mtm)?;
            (item, Some(loader))
        } else {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&settings.source));
            // SAFETY: a file address, on the main thread.
            (unsafe { AVPlayerItem::playerItemWithURL(&url, mtm) }, None)
        };
        // SAFETY: the item just made, on the main thread.
        let player = unsafe { AVPlayer::playerWithPlayerItem(Some(&item), mtm) };
        if loader.is_some() {
            // SAFETY: as above. The loader gives the bytes as they are asked
            // for, which the player's guesses of a stall cannot foresee.
            unsafe { player.setAutomaticallyWaitsToMinimizeStalling(false) };
        }
        let output = if settings.audio_only {
            None
        } else {
            // SAFETY: CoreVideo's keys are CFStrings, which are the same
            // objects as NSStrings.
            let (key, metal): (&NSString, &NSString) = unsafe {
                (&*std::ptr::from_ref::<CFString>(kCVPixelBufferPixelFormatTypeKey).cast::<NSString>(), &*std::ptr::from_ref::<CFString>(kCVPixelBufferMetalCompatibilityKey).cast::<NSString>())
            };
            let (format, compatible) = (NSNumber::new_u32(kCVPixelFormatType_32BGRA), NSNumber::new_bool(true));
            let (value, metal_value): (&AnyObject, &AnyObject) = (&format, &compatible);
            let attributes = NSDictionary::<NSString, AnyObject>::from_slices(&[key, metal], &[value, metal_value]);
            // SAFETY: the attributes are CoreVideo's pixel format key with a
            // format number and its Metal compatibility key with a boolean.
            let output = unsafe { AVPlayerItemVideoOutput::initWithPixelBufferAttributes(AVPlayerItemVideoOutput::alloc(), Some(&attributes)) };
            // SAFETY: an output added to the item it watches.
            unsafe { item.addOutput(&output) };
            Some(output)
        };
        // SAFETY: AVFoundation's own constant, set on the item before it plays.
        if let Some(spectral) = unsafe { AVAudioTimePitchAlgorithmSpectral } {
            // SAFETY: as above.
            unsafe { item.setAudioTimePitchAlgorithm(spectral) };
        }
        let legible = if settings.audio_only {
            None
        } else {
            match legible(&item, &watch) {
                Ok(l) => Some(l),
                Err(why) => {
                    watch.news.push(News::Warned(format!("the clip's own subtitles cannot be shown: {why}")));
                    None
                }
            }
        };
        let observer = Observer::new(watch);
        // SAFETY: the observer outlives its registrations, which the
        // engine's drop removes before letting it go.
        unsafe {
            item.addObserver_forKeyPath_options_context(&observer, ns_string!("status"), NSKeyValueObservingOptions::New, std::ptr::null_mut());
            item.addObserver_forKeyPath_options_context(&observer, ns_string!("duration"), NSKeyValueObservingOptions::New, std::ptr::null_mut());
            player.addObserver_forKeyPath_options_context(&observer, ns_string!("timeControlStatus"), NSKeyValueObservingOptions::New, std::ptr::null_mut());
            let center = NSNotificationCenter::defaultCenter();
            center.addObserver_selector_name_object(&observer, sel!(ended:), Some(AVPlayerItemDidPlayToEndTimeNotification), Some(&item));
            center.addObserver_selector_name_object(&observer, sel!(stalled:), Some(AVPlayerItemFailedToPlayToEndTimeNotification), Some(&item));
        }
        let engine = Engine {
            player,
            item,
            output,
            observer,
            rate: Cell::new(settings.rate),
            loader,
            opening: Cell::new(opening),
            legible,
            chosen: Cell::new(false),
            tapped: Cell::new(false),
            textures: None,
            no_metal: false,
            kept: Vec::new(),
        };
        let decode = |text: String| Failure::new(Cause::Decode, text);
        engine.set_volume(settings.volume).map_err(decode)?;
        engine.set_muted(settings.muted).map_err(decode)?;
        // SAFETY: the player's rate when it plays, on the main thread.
        unsafe { engine.player.setDefaultRate(settings.rate as f32) };
        Ok(engine)
    }

    fn watch(&self) -> &Watch {
        self.observer.ivars()
    }

    pub(super) fn play(&self) -> Result<(), String> {
        // SAFETY: the player, on the main thread.
        unsafe { self.player.play() };
        Ok(())
    }

    pub(super) fn pause(&self) -> Result<(), String> {
        // SAFETY: as for play.
        unsafe { self.player.pause() };
        Ok(())
    }

    /// Moves to `seconds`, to the frame, telling Sought once there; the
    /// position is `seconds` until this seek, or a later one, ends.
    pub(super) fn seek(&self, seconds: f64) -> Result<(), String> {
        let watch = Arc::clone(self.observer.ivars());
        let number = watch.seeks.fetch_add(1, Ordering::AcqRel) + 1;
        watch.target.store(seconds.to_bits(), Ordering::Release);
        watch.seeking.store(number, Ordering::Release);
        let done = block2::RcBlock::new(move |finished: Bool| {
            // A seek a later one cut short leaves the position to the later.
            if watch.seeking.compare_exchange(number, 0, Ordering::AcqRel, Ordering::Acquire).is_err() {
                return;
            }
            if finished.as_bool() {
                watch.tell(News::Sought);
            }
        });
        // SAFETY: a time in seconds, exact both ways, on the main thread;
        // the block only tells the watch.
        unsafe {
            let at = CMTime::with_seconds(seconds, TIMESCALE);
            self.player.seekToTime_toleranceBefore_toleranceAfter_completionHandler(at, kCMTimeZero, kCMTimeZero, &done);
        }
        Ok(())
    }

    /// Moves near `seconds`, within the tolerance AVFoundation chooses, as
    /// a slider being dragged seeks.
    pub(super) fn seek_near(&self, seconds: f64) -> Result<(), String> {
        // SAFETY: a time in seconds, on the main thread.
        unsafe { self.player.seekToTime(CMTime::with_seconds(seconds, TIMESCALE)) };
        Ok(())
    }

    pub(super) fn set_rate(&self, rate: f64) -> Result<(), String> {
        self.rate.set(rate);
        // SAFETY: the player, on the main thread.
        unsafe {
            self.player.setDefaultRate(rate as f32);
            if self.player.timeControlStatus() != AVPlayerTimeControlStatus::Paused {
                self.player.setRate(rate as f32);
            }
        }
        Ok(())
    }

    pub(super) fn set_volume(&self, volume: f64) -> Result<(), String> {
        // SAFETY: as for play.
        unsafe { self.player.setVolume(volume as f32) };
        Ok(())
    }

    pub(super) fn set_muted(&self, muted: bool) -> Result<(), String> {
        // SAFETY: as for play.
        unsafe { self.player.setMuted(muted) };
        Ok(())
    }

    pub(super) fn set_loop(&self, looped: bool) -> Result<(), String> {
        self.watch().looped.store(looped, Ordering::Release);
        Ok(())
    }

    /// Makes what the observer asked for on the renderer's thread: plays
    /// the clip again from its start where it ended while looping, tells a
    /// pause unless the clip is at its end, and tells why the item failed.
    pub(super) fn tend(&self) -> Result<(), String> {
        let watch = self.watch();
        if let Some(opening) = self.opening.take() {
            match opening.try_recv() {
                Ok(Opened::Allowed) => {
                    watch.held.store(false, Ordering::SeqCst);
                    watch.opened_unless_held();
                }
                Ok(Opened::Remote(remote)) => match &self.loader {
                    Some(loading) => loading.take(remote)?,
                    None => return Err(String::from("the clip's opener read it through the loader, where AVFoundation fetches it itself")),
                },
                Err(TryRecvError::Empty) => self.opening.set(Some(opening)),
                // Its server does not let AVFoundation fetch it, which the
                // opener told as the player's news.
                Err(TryRecvError::Disconnected) => {}
            }
        }
        if watch.restart.swap(false, Ordering::AcqRel) {
            // SAFETY: the player, on the main thread.
            unsafe {
                self.player.seekToTime_toleranceBefore_toleranceAfter(kCMTimeZero, kCMTimeZero, kCMTimeZero);
                self.player.play();
            }
        }
        if watch.paused.swap(false, Ordering::AcqRel) && !self.at_end() {
            watch.news.push(News::Paused);
        }
        if watch.failed.swap(false, Ordering::AcqRel) {
            watch.news.push(self.failure());
        }
        self.choose_cues();
        self.tap_sound();
        Ok(())
    }

    /// Puts an audio tap on the item's sound once the item is ready and its
    /// tracks are known; a tap that cannot be put on leaves the sound
    /// unmeasured, which the player warns of.
    fn tap_sound(&self) {
        // SAFETY: the item, on the main thread.
        if self.tapped.get() || unsafe { self.item.status() } != AVPlayerItemStatus::ReadyToPlay {
            return;
        }
        self.tapped.set(true);
        if let Err(why) = self.put_tap() {
            self.watch().news.push(News::Warned(format!("its sound cannot be measured: {why}")));
        }
    }

    /// Puts the audio tap on the item's first track of sound, through an
    /// audio mix of the item's own; a clip with no sound takes none.
    fn put_tap(&self) -> Result<(), String> {
        // SAFETY: AVFoundation's own constant, and the item's tracks, on the
        // main thread.
        let Some(sound) = (unsafe { AVMediaTypeAudio }) else {
            return Err(String::from("AVFoundation names no type of sound"));
        };
        // SAFETY: as above.
        let track = unsafe { self.item.tracks() }.iter().filter_map(|t| unsafe { t.assetTrack() }).find(|a| unsafe { a.mediaType() }.to_string() == sound.to_string());
        let Some(track) = track else {
            return Ok(());
        };
        let measure = Box::into_raw(Box::new(Measure { watch: Arc::clone(self.observer.ivars()), floats: AtomicBool::new(false), rate: AtomicU64::new(0) }));
        let mut callbacks = MTAudioProcessingTapCallbacks {
            version: kMTAudioProcessingTapCallbacksVersion_0,
            clientInfo: measure.cast(),
            init: Some(tap_init),
            finalize: Some(tap_finalize),
            prepare: Some(tap_prepare),
            unprepare: Some(tap_unprepare),
            process: Some(tap_process),
        };
        let mut made: *const MTAudioProcessingTap = std::ptr::null();
        // SAFETY: callbacks of this module's, whose client the tap keeps as
        // its storage until its finalize frees it.
        let status = unsafe { MTAudioProcessingTap::create(None, NonNull::from(&mut callbacks), kMTAudioProcessingTapCreationFlag_PostEffects, NonNull::from(&mut made)) };
        let Some(made) = NonNull::new(made.cast_mut()) else {
            // SAFETY: no tap took the measure, so it is freed here, once.
            drop(unsafe { Box::from_raw(measure) });
            return Err(format!("no audio tap could be made, status {status}"));
        };
        // SAFETY: the tap MTAudioProcessingTapCreate made, owned from here.
        let tap = unsafe { CFRetained::from_raw(made) };
        // SAFETY: the mix's parameters for the sound's track and its tap, and
        // the mix set on the item, on the main thread.
        unsafe {
            let parameters = AVMutableAudioMixInputParameters::audioMixInputParametersWithTrack(Some(&track));
            parameters.setAudioTapProcessor(Some(&tap));
            let mix = AVMutableAudioMix::audioMix();
            let all: Retained<NSArray<AVAudioMixInputParameters>> = NSArray::from_retained_slice(&[Retained::into_super(parameters)]);
            mix.setInputParameters(&all);
            self.item.setAudioMix(Some(&mix));
        }
        Ok(())
    }

    /// Chooses the clip's first track of subtitles or captions for the
    /// legible output, once the item is ready and its tracks are known, as
    /// the player shows none of them by default.
    fn choose_cues(&self) {
        // SAFETY: the item, on the main thread.
        if self.legible.is_none() || self.chosen.get() || unsafe { self.item.status() } != AVPlayerItemStatus::ReadyToPlay {
            return;
        }
        self.chosen.set(true);
        // SAFETY: AVFoundation's own constant.
        let Some(legible) = (unsafe { AVMediaCharacteristicLegible }) else {
            return;
        };
        // SAFETY: the item's own asset, whose tracks are loaded once the
        // item is ready, so the group is read without waiting, on the main
        // thread; an option of the group, chosen in it.
        unsafe {
            #[expect(deprecated, reason = "the item is ready, so its asset gives the group at once, on the thread the item is chosen on")]
            let group = self.item.asset().mediaSelectionGroupForMediaCharacteristic(legible);
            if let Some(group) = group
                && let Some(first) = group.options().firstObject()
            {
                self.item.selectMediaOption_inMediaSelectionGroup(Some(&first), &group);
            }
        }
    }

    /// Whether the clip stands at its end.
    fn at_end(&self) -> bool {
        self.facts().duration.is_some_and(|length| self.position() >= length - 1.0 / f64::from(TIMESCALE))
    }

    /// The news of the item's failure: where the resource loader could not
    /// give an https clip, why it could not; else its cause from the error's
    /// domain and code. A format or codec AVFoundation cannot play is not
    /// supported, a file or address that cannot be read is the network's
    /// cause, and the rest are the decoder's.
    fn failure(&self) -> News {
        if let Some(trouble) = self.watch().trouble.load_full() {
            let (cause, why) = &*trouble;
            return News::Failed(Failure::new(*cause, format!("the resource loader could not give it: {why}")));
        }
        // SAFETY: the item, on the main thread.
        let Some(error) = (unsafe { self.item.error() }) else {
            return News::Failed(Failure::new(Cause::Decode, String::from("it gave no reason")));
        };
        let domain = error.domain().to_string();
        // SAFETY: AVFoundation's own error domain constant.
        let ours = unsafe { AVFoundationErrorDomain }.is_some_and(|d| domain == d.to_string());
        let cause = if ours && UNPLAYABLE.contains(&error.code()) {
            Cause::FormatNotSupported
        } else if domain == "NSURLErrorDomain" || domain == "NSCocoaErrorDomain" {
            Cause::Network
        } else {
            Cause::Decode
        };
        News::Failed(Failure::new(cause, error.localizedDescription().to_string()))
    }

    /// The texts of the cues showing now of the clip's own first track of
    /// subtitles or captions, as its legible output gave them.
    pub(super) fn captions(&self, _position: f64) -> Result<Vec<String>, String> {
        Ok(self.watch().cues.load_full().map_or_else(Vec::new, |texts| texts.to_vec()))
    }

    /// The seconds from the clip's start: where a seek under way goes, as
    /// the player's own time moves only once the seek ends; 0 until the
    /// player can say.
    pub(super) fn position(&self) -> f64 {
        let watch = self.watch();
        if watch.seeking.load(Ordering::Acquire) != 0 {
            return f64::from_bits(watch.target.load(Ordering::Acquire));
        }
        // SAFETY: the player, on the main thread.
        seconds(unsafe { self.player.currentTime() }).unwrap_or(0.0)
    }

    pub(super) fn facts(&self) -> Facts {
        // SAFETY: the item, on the main thread.
        let (length, size, tracks) = unsafe { (self.item.duration(), self.item.presentationSize(), self.item.tracks()) };
        let video = (self.output.is_some() && size.width >= 1.0 && size.height >= 1.0).then(|| [size.width.round() as u32, size.height.round() as u32]);
        // SAFETY: AVFoundation's own media type constant, and the item's
        // tracks' own types.
        let audio = unsafe { AVMediaTypeAudio }.is_some_and(|sound| tracks.iter().any(|t| unsafe { t.assetTrack() }.is_some_and(|a| unsafe { a.mediaType() }.to_string() == sound.to_string())));
        Facts { duration: seconds(length).filter(|s| *s > 0.0), video, audio }
    }

    /// The frame due now, when the output has a new one.
    pub(super) fn frame(&mut self) -> Result<Option<Frame>, String> {
        let Some(output) = &self.output else {
            return Ok(None);
        };
        // SAFETY: the item and its output, on the main thread; a buffer the
        // output gives is the caller's, released as it drops.
        let buffer = unsafe {
            let now = self.item.currentTime();
            if !output.hasNewPixelBufferForItemTime(now) {
                return Ok(None);
            }
            output.copyPixelBufferForItemTime_itemTimeForDisplay(now, std::ptr::null_mut())
        };
        let Some(buffer) = buffer else {
            return Ok(None);
        };
        let (width, height) = (CVPixelBufferGetWidth(&buffer), CVPixelBufferGetHeight(&buffer));
        let size = [u32::try_from(width).map_err(|e| format!("the frame's width {width}: {e}"))?, u32::try_from(height).map_err(|e| format!("the frame's height {height}: {e}"))?];
        self.kept.retain(|f| !let_go(f));
        match self.on_metal(&buffer, size) {
            Ok(Some(frame)) => return Ok(Some(Frame { size, bgra: Vec::new(), metal: Some(frame) })),
            Ok(None) => {}
            Err(why) => {
                eprintln!("pwrs-gui-egui: a player's frames come through the CPU from now on: {why}");
                self.no_metal = true;
            }
        }
        // SAFETY: the buffer the output gave, locked for reading here and
        // unlocked below.
        if unsafe { CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::ReadOnly) } != kCVReturnSuccess {
            return Err(String::from("the frame's pixels could not be locked for reading"));
        }
        let (stride, base) = (CVPixelBufferGetBytesPerRow(&buffer), CVPixelBufferGetBaseAddress(&buffer));
        let row = width * 4;
        let copied = if base.is_null() || stride < row {
            Err(format!("the frame's rows of {stride} bytes are short of {width} pixels"))
        } else {
            // SAFETY: the locked buffer holds `height` rows of `stride` bytes.
            let data = unsafe { std::slice::from_raw_parts(base.cast::<u8>(), stride * height) };
            let mut bgra = Vec::with_capacity(row * height);
            for y in 0..height {
                bgra.extend_from_slice(&data[y * stride..y * stride + row]);
            }
            Ok(Some(Frame { size, bgra, metal: None }))
        };
        // SAFETY: the lock taken above.
        if unsafe { CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::ReadOnly) } != kCVReturnSuccess {
            return Err(String::from("the frame's pixels could not be unlocked"));
        }
        copied
    }

    /// The frame in `buffer`, of `size` pixels, handed to wgpu's Metal
    /// device as a texture of the buffer itself, kept until the copy out of
    /// it has run; none where the renderer draws otherwise or the path is
    /// given up.
    fn on_metal(&mut self, buffer: &Retained<CVPixelBuffer>, size: [u32; 2]) -> Result<Option<Arc<MetalFrame>>, String> {
        let Some(device) = METAL.load_full().filter(|_| !self.no_metal) else {
            return Ok(None);
        };
        let count = DEVICES.load(Ordering::Acquire);
        let cache = match &self.textures {
            Some((made_for, cache)) if *made_for == count => cache.clone(),
            _ => {
                let mut made: *mut CVMetalTextureCache = std::ptr::null_mut();
                let status = {
                    // SAFETY: wgpu's own device, whose Metal device is only
                    // read, and the out pointer, through the call.
                    let hal = unsafe { device.as_hal::<wgpu::hal::api::Metal>() }.ok_or("the renderer no longer draws with Metal")?;
                    // SAFETY: as above.
                    unsafe { CVMetalTextureCache::create(None, None, hal.raw_device(), None, NonNull::from(&mut made)) }
                };
                let made = NonNull::new(made).ok_or_else(|| format!("no Metal texture cache could be made, status {status}"))?;
                // SAFETY: the cache CVMetalTextureCacheCreate made, owned
                // from here.
                let cache = unsafe { CFRetained::from_raw(made) };
                self.textures = Some((count, cache.clone()));
                cache
            }
        };
        let mut image: *mut CVMetalTexture = std::ptr::null_mut();
        // SAFETY: the output's pixel buffer, of BGRA rows of the frame's
        // size and Metal compatible, and the out pointer, through the call.
        let status = unsafe { CVMetalTextureCache::create_texture_from_image(None, &cache, buffer, None, MTLPixelFormat::BGRA8Unorm, size[0] as usize, size[1] as usize, 0, NonNull::from(&mut image)) };
        let image = NonNull::new(image).ok_or_else(|| format!("the frame's pixel buffer gave no Metal texture, status {status}"))?;
        // SAFETY: the texture CVMetalTextureCacheCreateTextureFromImage made,
        // owned from here.
        let image = unsafe { CFRetained::from_raw(image) };
        let raw = CVMetalTextureGetTexture(&image).ok_or("the frame's Metal texture holds no texture")?;
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let described = wgpu::TextureDescriptor {
            label: Some("PWRS-GUI MediaPlayer frame"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        };
        // SAFETY: a texture of the description's size and format on wgpu's
        // own Metal device, kept with its pixel buffer as long as wgpu's
        // texture of it.
        let texture = unsafe {
            let copy = wgpu::hal::CopyExtent { width: size[0], height: size[1], depth: 1 };
            let raw = wgpu::hal::metal::Device::texture_from_raw(raw, described.format, MTLTextureType::Type2D, 1, 1, copy, None);
            device.create_texture_from_hal::<wgpu::hal::api::Metal>(raw, &described, wgpu::wgt::TextureUses::UNINITIALIZED)
        };
        cache.flush(0);
        let frame = Arc::new(MetalFrame { texture, _held: Held { _texture: image, _buffer: buffer.clone() }, copied: AtomicU64::new(0) });
        self.kept.push(Arc::clone(&frame));
        Ok(Some(frame))
    }
}

/// The seconds `time` gives; none for a time that is not a number of
/// seconds, as the length of a clip still opening.
fn seconds(time: CMTime) -> Option<f64> {
    if !time.flags.contains(CMTimeFlags::Valid) || time.flags.contains(CMTimeFlags::Indefinite) {
        return None;
    }
    // SAFETY: a valid, definite time.
    let s = unsafe { time.seconds() };
    s.is_finite().then_some(s)
}

impl Drop for Engine {
    fn drop(&mut self) {
        // SAFETY: each registration the engine made, removed before the
        // observer goes, and the player stopped, on the main thread.
        unsafe {
            self.player.pause();
            self.item.removeObserver_forKeyPath(&self.observer, ns_string!("status"));
            self.item.removeObserver_forKeyPath(&self.observer, ns_string!("duration"));
            self.player.removeObserver_forKeyPath(&self.observer, ns_string!("timeControlStatus"));
            NSNotificationCenter::defaultCenter().removeObserver(&self.observer);
            self.player.replaceCurrentItemWithPlayerItem(None);
        }
    }
}
