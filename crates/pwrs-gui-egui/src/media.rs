//! The MediaPlayer widget. The operating system's own player plays the
//! clip, with its sound and its clock ([`backend`]): Media Foundation's
//! media engine on Windows, AVFoundation's `AVPlayer` on macOS, and
//! GStreamer's `playbin` on Linux and the BSDs. Each new frame of the
//! picture is written into a texture of the clip's size, which egui-wgpu's
//! paint callback shows fitted inside the widget, the rest of the widget
//! black: copied out through the CPU, or on Windows with wgpu's Direct3D
//! 12 or Vulkan device copied on the GPU out of a texture the two share.
//! The backend's events wake the loop, and every pump turns them into the
//! widget's events, whether or not its window is drawn.

use crate::{Node, NodeId, WindowId, egui, label, nodes_of, opt_points, proto, surface};
use arc_swap::ArcSwapOption;
use crossbeam_queue::SegQueue;
use eframe::egui_wgpu;
use proto::media::{self, Cause, MediaOp, MediaState, Phase, Remedy, Settings};
use proto::renderer::WakeLoop;
use proto::tokens;
use proto::{EventKind, FromHelper, Kind, Value};
use pwrs_gui_gpu::wgpu;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

mod controls;
#[cfg(windows)]
mod mf;
#[cfg(windows)]
mod mf_share;
#[cfg(windows)]
mod mf_tap;
#[cfg(windows)]
use mf as backend;
#[cfg(all(unix, not(target_vendor = "apple")))]
mod gst;
#[cfg(all(unix, not(target_vendor = "apple")))]
use gst as backend;
#[cfg(target_os = "macos")]
mod avf;
#[cfg(target_os = "macos")]
use avf as backend;
#[cfg(not(any(windows, target_os = "macos", all(unix, not(target_vendor = "apple")))))]
mod none;
#[cfg(not(any(windows, target_os = "macos", all(unix, not(target_vendor = "apple")))))]
use none as backend;

/// The media backend the renderer's hello names; none where it plays no
/// media.
pub(crate) fn backend_name() -> Option<String> {
    (!backend::NAME.is_empty()).then(|| backend::NAME.to_string())
}

/// What a backend tells its player, from the backend's own threads.
pub(crate) enum News {
    /// The clip is open, or its length or format changed.
    Opened,
    Playing,
    Paused,
    Ended,
    /// The position moved, as playing moves it: news that wakes the loop to
    /// draw the player.
    #[cfg_attr(not(windows), expect(dead_code, reason = "Media Foundation alone reports the position moving on"))]
    Moved,
    /// A seek ended where it went.
    Sought,
    /// The clip cannot be played.
    Failed(Failure),
    /// The Error value of a clip that cannot be played, worded on a thread
    /// that read an https clip's headers for its format.
    Worded(Value),
    /// Something of the clip the backend cannot show, such as a track of
    /// its text, which the window reports as a problem while the clip plays
    /// on.
    Warned(String),
    /// A block of sound whose highest sample, before the player's volume,
    /// is `peak`, from `at` to `until` seconds into the clip, on its way to
    /// the system. It wakes no loop: a pump takes it.
    #[cfg_attr(not(any(windows, target_os = "macos", all(unix, not(target_vendor = "apple")))), expect(dead_code, reason = "a system with no media backend measures no sound"))]
    Heard { at: f64, until: f64, peak: f32 },
}

/// An https clip as the thread that opened it found it.
pub(crate) enum Opened {
    /// The loader opened it, for the backend to read by its byte ranges.
    Remote(pwrs_gui_fetch::Remote),
    /// Its server lets pages of [`media::PAGE_ORIGIN`] read it, so the
    /// backend's own engine fetches it.
    Allowed,
}

/// Opens the https clip `source` on a thread of its own, so the window
/// stays drawn meanwhile: through the loader, or with `engine_fetch` by
/// asking its server whether it lets the engine fetch it, under CORS as the
/// web helper's pages are asked. What it opened comes on the receiver, the
/// loop woken; why it could not comes as the player's news, for the
/// network's cause, worded as the HTML renderer words it.
pub(crate) fn open_web(source: &str, engine_fetch: bool, news: Arc<SegQueue<News>>, wake: Option<WakeLoop>) -> Result<std::sync::mpsc::Receiver<Opened>, Failure> {
    let (send, receive) = std::sync::mpsc::sync_channel(1);
    let url = source.to_string();
    let opener = move || {
        let opened = if engine_fetch {
            match pwrs_gui_fetch::allows(&url, media::PAGE_ORIGIN) {
                Ok(true) => Ok(Opened::Allowed),
                Ok(false) => Err(Failure::helpers(Cause::Network, media::refused_by_cors(&url))),
                Err(why) => Err(Failure::helpers(Cause::Network, format!("its server could not be asked whether it lets the player fetch it: {why}"))),
            }
        } else {
            pwrs_gui_fetch::Remote::open(&url).map(Opened::Remote).map_err(|why| Failure::new(Cause::Network, format!("the loader could not open it: {why}")))
        };
        match opened {
            Ok(o) => {
                if let Err(gone) = send.send(o) {
                    // The player closed before its clip opened.
                    drop(gone.0);
                }
            }
            Err(failure) => news.push(News::Failed(failure)),
        }
        if let Some(wake) = &wake {
            wake();
        }
    };
    std::thread::Builder::new().name(String::from("pwrs-gui-media-open")).spawn(opener).map_err(|e| Failure::new(Cause::Aborted, format!("no thread could be started to open it: {e}")))?;
    Ok(receive)
}

/// Why a backend cannot play a clip: the cause it maps its own error onto,
/// its own text, and what to install where the system has no player.
pub(crate) struct Failure {
    pub cause: Cause,
    pub detail: String,
    /// What the Error names as its remedy where the system has no player;
    /// the FFmpeg add-on is named otherwise, for a format the backend cannot
    /// play.
    pub install: Option<&'static str>,
    /// Whether the detail is the backend's, which the Error names it by, or
    /// the helper's own, worded as the HTML renderer words it.
    pub backends: bool,
}

impl Failure {
    pub(crate) fn new(cause: Cause, detail: String) -> Failure {
        Failure { cause, detail, install: None, backends: true }
    }

    /// The system has no player at all: its format cannot be played until
    /// `install` is. Media Foundation and GStreamer can be missing; macOS
    /// always has AVFoundation.
    #[cfg(any(windows, all(unix, not(target_vendor = "apple"))))]
    pub(crate) fn missing(detail: String, install: &'static str) -> Failure {
        Failure { cause: Cause::FormatNotSupported, detail, install: Some(install), backends: true }
    }

    /// The helper itself found the clip cannot be played, as the web helper
    /// finds it: its detail is the same in both renderers.
    pub(crate) fn helpers(cause: Cause, detail: String) -> Failure {
        Failure { cause, detail, install: None, backends: false }
    }
}

/// What a backend knows of an open clip.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Facts {
    /// Its length in seconds, once known.
    pub duration: Option<f64>,
    /// The picture's size in pixels; none for sound alone.
    pub video: Option<[u32; 2]>,
    pub audio: bool,
}

/// A frame of the picture: its size in pixels and its rows, top to bottom,
/// four bytes a pixel in blue, green, red and alpha order; or, where the
/// backend wrote it into a texture shared with the window's GPU, that
/// texture and no rows.
pub(crate) struct Frame {
    pub size: [u32; 2],
    pub bgra: Vec<u8>,
    #[cfg(windows)]
    pub shared: Option<Arc<mf_share::Share>>,
    #[cfg(target_os = "macos")]
    pub metal: Option<Arc<avf::MetalFrame>>,
}

impl Frame {
    /// Records a copy of the frame into `target` on the GPU, for the draw
    /// of egui frame `draw`, where the backend wrote it into a texture
    /// shared with wgpu; false for a frame of rows.
    #[cfg(windows)]
    fn copy_on_gpu(&self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Texture, draw: u64) -> bool {
        match &self.shared {
            Some(share) => {
                share.copy(device, queue, encoder, target, draw);
                true
            }
            None => false,
        }
    }

    #[cfg(target_os = "macos")]
    fn copy_on_gpu(&self, _device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, target: &wgpu::Texture, draw: u64) -> bool {
        match &self.metal {
            Some(frame) => {
                frame.copy(queue, encoder, target, draw);
                true
            }
            None => false,
        }
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    fn copy_on_gpu(&self, _device: &wgpu::Device, _queue: &wgpu::Queue, _encoder: &mut wgpu::CommandEncoder, _target: &wgpu::Texture, _draw: u64) -> bool {
        false
    }
}

/// Copies out of textures a backend shares with wgpu, numbered as they are
/// recorded, and how far the GPU has run them, so a texture is written
/// again, or let go of, only once wgpu has done with it. Copies are
/// recorded on the renderer's thread alone.
#[cfg(any(windows, target_os = "macos"))]
mod copies {
    use super::wgpu;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// The number the next copy takes, from 1.
    static COPIES: AtomicU64 = AtomicU64::new(1);

    /// Every copy numbered up to this one has run on the GPU.
    static FINISHED: AtomicU64 = AtomicU64::new(0);

    /// The egui frame whose draw the copies being recorded go out with, and
    /// the number of the first of them: every copy numbered below it went
    /// out with an earlier draw, which wgpu has been given.
    static DRAW: AtomicU64 = AtomicU64::new(u64::MAX);
    static DRAW_FIRST: AtomicU64 = AtomicU64::new(1);

    /// The number of a copy recorded now for the draw of egui frame `draw`.
    /// `queue` notes, once the work it has been given so far has run, that
    /// the copies of earlier draws have, not this draw's, which goes out
    /// after it.
    pub(crate) fn recorded(queue: &wgpu::Queue, draw: u64) -> u64 {
        let number = COPIES.fetch_add(1, Ordering::AcqRel);
        if DRAW.swap(draw, Ordering::AcqRel) != draw {
            DRAW_FIRST.store(number, Ordering::Release);
        }
        let given = DRAW_FIRST.load(Ordering::Acquire) - 1;
        queue.on_submitted_work_done(move || {
            FINISHED.fetch_max(given, Ordering::AcqRel);
        });
        number
    }

    /// Whether copy `number` has run on the GPU; 0 numbers no copy, which
    /// has.
    pub(crate) fn finished(number: u64) -> bool {
        number <= FINISHED.load(Ordering::Acquire)
    }
}

/// Tells the backend the device the renderer draws with from now on, so
/// the frames it can keep on that GPU are copied there without the CPU.
#[cfg(windows)]
pub(crate) fn link_gpu(adapter: &wgpu::Adapter, device: &wgpu::Device) {
    mf_share::link(adapter, device);
}

#[cfg(target_os = "macos")]
pub(crate) fn link_gpu(adapter: &wgpu::Adapter, device: &wgpu::Device) {
    avf::link(adapter, device);
}

#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) fn link_gpu(_adapter: &wgpu::Adapter, _device: &wgpu::Device) {}

/// A frame with the number its player gave it, so the texture is written
/// once for each.
struct Numbered {
    serial: u64,
    frame: Frame,
}

/// One MediaPlayer's clip, open in the backend, and what the widget shows
/// and raises from it.
pub(crate) struct Player {
    /// The settings its node gave last, which a change to the node is told
    /// apart from.
    given: Settings,
    /// The volume, mute, loop and rate it plays with now, which a call or a
    /// change to the node set last.
    live: Settings,
    /// The backend's clip; none once it could not be opened.
    engine: Option<backend::Engine>,
    news: Arc<SegQueue<News>>,
    /// What wakes the loop for news told from another thread.
    wake: Option<WakeLoop>,
    phase: Phase,
    facts: Facts,
    loaded: bool,
    error: Option<String>,
    /// When Position was last raised.
    told: Option<Instant>,
    /// The newest frame, which the paint callback writes into the texture.
    latest: Option<Arc<Numbered>>,
    serial: u64,
    texture: Arc<ArcSwapOption<Texture>>,
    /// Whether it shows alone, filling the screen.
    full: bool,
    /// When the pointer last moved over it in full screen, which shows its
    /// controls a while.
    moved: Instant,
    /// Where a seek of its slider being dragged last went.
    scrub: Option<f64>,
    /// Whether a seek is under way, until its backend says it is done. The
    /// window is drawn each frame meanwhile, since Media Foundation's media
    /// engine finishes a seek only once its frame there has been taken.
    seeking: bool,
    /// The sound the backend's tap saw it hand the system.
    meter: media::Meter,
    /// The seconds of the clip it has played, and where it stood as the last
    /// pump found it playing, and when; none since a seek, or while it does
    /// not play.
    played: f64,
    last_at: Option<f64>,
    counted: Option<Instant>,
}

impl Player {
    /// Opens `settings`' clip, its backend waking the loop through `wake`.
    /// A clip that cannot be opened is a player that raises its Error.
    fn open(settings: Settings, wake: Option<&WakeLoop>) -> Player {
        let news = Arc::new(SegQueue::new());
        let engine = match backend::Engine::open(&settings, Arc::clone(&news), wake.cloned()) {
            Ok(e) => Some(e),
            Err(failure) => {
                news.push(News::Failed(failure));
                None
            }
        };
        Player {
            given: settings.clone(),
            live: settings,
            engine,
            news,
            wake: wake.cloned(),
            phase: Phase::Loading,
            facts: Facts::default(),
            loaded: false,
            error: None,
            told: None,
            latest: None,
            serial: 0,
            texture: Arc::default(),
            full: false,
            moved: Instant::now(),
            scrub: None,
            seeking: false,
            meter: media::Meter::default(),
            played: 0.0,
            last_at: None,
            counted: None,
        }
    }

    /// The sound it handed the system, where its backend measures it: the
    /// peak of the half second of the clip up to where it plays now, and
    /// the seconds played up to now. Those since the last pump count while
    /// it plays, as a window without the focus may be pumped seldom, no more
    /// than that time at its rate; a pause or a seek since counts nothing.
    fn sound(&self) -> Option<media::Sound> {
        let gain = if self.live.muted { 0.0 } else { self.live.volume as f32 };
        let position = self.engine.as_ref().map_or(0.0, backend::Engine::position);
        let since = match (self.last_at, self.counted) {
            (Some(was), Some(when)) if self.phase == Phase::Playing && !self.seeking && position > was => (position - was).min(when.elapsed().as_secs_f64() * self.live.rate),
            _ => 0.0,
        };
        backend::MEASURES.then(|| media::Sound { peak: self.meter.peak(position, gain), seconds: self.played + since })
    }

    /// Counts the time its position moved on since the last pump while it
    /// plays; a seek, a loop's jump back and a pause count nothing.
    fn count_played(&mut self) {
        let Some(engine) = &self.engine else {
            return;
        };
        if self.phase != Phase::Playing {
            self.last_at = None;
            return;
        }
        let at = engine.position();
        if let Some(was) = self.last_at
            && at > was
        {
            self.played += at - was;
        }
        self.last_at = Some(at);
        self.counted = Some(Instant::now());
    }

    /// Follows a change to the node's settings: a new clip is opened in
    /// place of the old, and a new volume, mute, loop or rate is set.
    /// Returns why each that could not be set was not.
    fn follow(&mut self, settings: Settings, wake: Option<&WakeLoop>) -> Vec<String> {
        if settings.source != self.given.source || settings.audio_only != self.given.audio_only || settings.engine_fetch != self.given.engine_fetch {
            *self = Player::open(settings, wake);
            return Vec::new();
        }
        let mut problems = Vec::new();
        let was = std::mem::replace(&mut self.given, settings.clone());
        let mut changes = Vec::new();
        if settings.volume != was.volume {
            changes.push(MediaOp::Volume { volume: settings.volume });
        }
        if settings.muted != was.muted {
            changes.push(MediaOp::Muted { muted: settings.muted });
        }
        if settings.looped != was.looped {
            changes.push(MediaOp::Loop { looped: settings.looped });
        }
        if settings.rate != was.rate {
            changes.push(MediaOp::Rate { rate: settings.rate });
        }
        for op in changes {
            if let Err(why) = self.act(op) {
                problems.push(why);
            }
        }
        self.live.autoplay = settings.autoplay;
        self.live.position_every = settings.position_every;
        problems
    }

    /// Makes `op`, or says why it cannot be made.
    fn act(&mut self, op: MediaOp) -> Result<(), String> {
        let Some(engine) = &self.engine else {
            return match op {
                MediaOp::State => Ok(()),
                _ => Err(format!("the clip could not be played: {}", self.error.as_deref().unwrap_or("it did not open"))),
            };
        };
        match op {
            MediaOp::Play => {
                if self.phase == Phase::Ended {
                    engine.seek(0.0)?;
                    self.seeking = true;
                }
                engine.play()
            }
            MediaOp::Pause => engine.pause(),
            MediaOp::Seek { seconds } => {
                if !seconds.is_finite() || seconds < 0.0 {
                    return Err(format!("a position is seconds from the clip's start, 0 or more, not {seconds}"));
                }
                let to = self.facts.duration.map_or(seconds, |d| seconds.min(d));
                engine.seek(to)?;
                self.seeking = true;
                self.last_at = None;
                if self.phase == Phase::Ended && self.facts.duration.is_none_or(|d| to < d) {
                    self.phase = Phase::Paused;
                }
                Ok(())
            }
            MediaOp::Rate { rate } => {
                media::check_rate(rate)?;
                engine.set_rate(rate)?;
                self.live.rate = rate;
                Ok(())
            }
            MediaOp::Volume { volume } => {
                if !(0.0..=1.0).contains(&volume) {
                    return Err(format!("a volume runs from 0 for silence to 1, not {volume}"));
                }
                engine.set_volume(volume)?;
                self.live.volume = volume;
                Ok(())
            }
            MediaOp::Muted { muted } => {
                engine.set_muted(muted)?;
                self.live.muted = muted;
                Ok(())
            }
            MediaOp::Loop { looped } => {
                engine.set_loop(looped)?;
                self.live.looped = looped;
                Ok(())
            }
            MediaOp::State => Ok(()),
        }
    }

    /// The player's state now.
    fn state(&self) -> MediaState {
        MediaState {
            phase: self.phase,
            position: self.engine.as_ref().map_or(0.0, backend::Engine::position),
            duration: self.facts.duration,
            rate: self.live.rate,
            volume: self.live.volume,
            muted: self.live.muted,
            looped: self.live.looped,
            video: self.facts.video,
            audio: self.facts.audio,
            error: self.error.clone(),
        }
    }

    /// Turns what the backend told into the events of the player's node
    /// `widget`, and raises Position when it is due; returns how long until
    /// the next is due while one will be.
    fn pump(&mut self, window: WindowId, widget: &Node, out: &mut Vec<FromHelper>) -> Option<Duration> {
        let node = widget.id;
        if let Some(Err(text)) = self.engine.as_ref().map(backend::Engine::tend) {
            self.fail(window, node, Failure::new(Cause::Decode, text), out);
        }
        let position = |engine: &Option<backend::Engine>| Value::Float(engine.as_ref().map_or(0.0, backend::Engine::position));
        while let Some(news) = self.news.pop() {
            match news {
                News::Opened => {
                    let Some(engine) = &self.engine else {
                        continue;
                    };
                    self.facts = engine.facts();
                    if self.loaded {
                        continue;
                    }
                    self.loaded = true;
                    if self.phase == Phase::Loading {
                        self.phase = Phase::Paused;
                    }
                    raise(out, window, node, EventKind::Loaded, media::loaded_value(self.facts.duration, self.facts.video, self.facts.audio));
                    if self.live.autoplay
                        && let Err(text) = engine.play()
                    {
                        self.fail(window, node, Failure::new(Cause::Aborted, format!("the clip could not start: {text}")), out);
                    }
                }
                News::Playing => {
                    self.phase = Phase::Playing;
                    self.told = Some(Instant::now());
                    raise(out, window, node, EventKind::Playing, position(&self.engine));
                }
                News::Paused => {
                    if self.phase != Phase::Ended {
                        self.phase = Phase::Paused;
                    }
                    raise(out, window, node, EventKind::Paused, position(&self.engine));
                }
                News::Ended => {
                    self.phase = Phase::Ended;
                    if self.live.position_every.is_some() {
                        raise(out, window, node, EventKind::Position, position(&self.engine));
                    }
                    raise(out, window, node, EventKind::Ended, position(&self.engine));
                }
                News::Moved => {}
                News::Sought => {
                    self.seeking = false;
                    self.last_at = None;
                    if self.live.position_every.is_some() {
                        self.told = Some(Instant::now());
                        raise(out, window, node, EventKind::Position, position(&self.engine));
                    }
                }
                News::Failed(failure) => self.fail(window, node, failure, out),
                News::Worded(value) => self.report(window, node, value, out),
                News::Warned(text) => out.push(FromHelper::Error { window: Some(window), context: String::from("MediaPlayer"), text: label(widget, &text) }),
                News::Heard { at, until, peak } => self.meter.heard(at, until, peak),
            }
        }
        self.meter.forget(self.engine.as_ref().map_or(0.0, backend::Engine::position));
        self.count_played();
        let (Some(every), Phase::Playing) = (self.live.position_every, self.phase) else {
            return None;
        };
        let since = self.told.map_or(every, |t| t.elapsed());
        if since < every {
            return Some(every - since);
        }
        self.told = Some(Instant::now());
        raise(out, window, node, EventKind::Position, position(&self.engine));
        Some(every)
    }

    /// The clip cannot be played: the player fails and raises its Error,
    /// worded as every renderer words it, the backend's own text its
    /// detail. A format the backend cannot play names the FFmpeg add-on as
    /// the remedy, and a system with no player what to install.
    fn fail(&mut self, window: WindowId, node: NodeId, failure: Failure, out: &mut Vec<FromHelper>) {
        self.phase = Phase::Failed;
        self.seeking = false;
        let addon = media::addon_remedy();
        let remedy = match (failure.install, failure.cause) {
            (Some(install), _) => Remedy::Install(install),
            (None, Cause::FormatNotSupported) => Remedy::Player(&addon),
            (None, _) => Remedy::None,
        };
        let detail = if failure.backends { format!("{}: {}", backend::NAME, failure.detail) } else { failure.detail.clone() };
        let source = self.given.source.clone();
        if pwrs_gui_fetch::is_web(&source) {
            // An https clip's headers are read by the loader, off the
            // renderer's thread; the Error comes once they are.
            let (news, wake, cause) = (Arc::clone(&self.news), self.wake.clone(), failure.cause);
            let (install, addon, worded) = (failure.install, media::addon_remedy(), detail.clone());
            let spawned = std::thread::Builder::new().name(String::from("pwrs-gui-media-error")).spawn(move || {
                let remedy = match (install, cause) {
                    (Some(install), _) => Remedy::Install(install),
                    (None, Cause::FormatNotSupported) => Remedy::Player(&addon),
                    (None, _) => Remedy::None,
                };
                let format = pwrs_gui_fetch::clip_format(&source).map(|f| f.name());
                news.push(News::Worded(media::clip_error(&source, cause, format, remedy, &worded)));
                if let Some(wake) = &wake {
                    wake();
                }
            });
            if let Err(e) = spawned {
                let value = media::clip_error(&self.given.source, failure.cause, Err(format!("no thread could be started to read its headers: {e}")), Remedy::None, &detail);
                self.report(window, node, value, out);
            }
            return;
        }
        let value = media::file_error(&source, failure.cause, remedy, &detail);
        self.report(window, node, value, out);
    }

    /// Raises Error with `value`, keeping its message as the player's error.
    fn report(&mut self, window: WindowId, node: NodeId, value: Value, out: &mut Vec<FromHelper>) {
        self.error = value.as_map().and_then(|m| m.get("message")).and_then(Value::as_str).map(String::from);
        raise(out, window, node, EventKind::Error, value);
    }

    /// Takes the frame due now from the backend, when there is a new one.
    fn take_frame(&mut self) -> Result<(), String> {
        let Some(engine) = self.engine.as_mut() else {
            return Ok(());
        };
        if let Some(frame) = engine.frame()? {
            self.serial += 1;
            self.latest = Some(Arc::new(Numbered { serial: self.serial, frame }));
        }
        Ok(())
    }
}

/// Raises event `kind` of player `node` with `value`.
fn raise(out: &mut Vec<FromHelper>, window: WindowId, node: NodeId, kind: EventKind, value: Value) {
    out.push(FromHelper::Event { window, node, kind, value, at: proto::clock::now_ns() });
}

/// Opens a player for each MediaPlayer of `tree` without one, lets go of
/// those whose node is gone, follows changes to their settings, and turns
/// what each backend told into its events, put in `out`. A node whose
/// settings cannot be read is reported once, as its window's frame reports
/// a widget's problem. Returns how long until a Position is next due, while
/// one will be.
pub(crate) fn pump(players: &mut HashMap<NodeId, Player>, reported: &mut HashSet<NodeId>, tree: &Node, window: WindowId, wake: Option<&WakeLoop>, out: &mut Vec<FromHelper>) -> Option<Duration> {
    let mut nodes = Vec::new();
    nodes_of(tree, Kind::MediaPlayer, &mut nodes);
    players.retain(|id, _| nodes.iter().any(|n| n.id == *id));
    let mut soonest: Option<Duration> = None;
    for node in nodes {
        let settings = match Settings::of(node) {
            Ok(s) => s,
            Err(problem) => {
                players.remove(&node.id);
                if reported.insert(node.id) {
                    out.push(FromHelper::Error { window: Some(window), context: String::from("draw"), text: label(node, &problem) });
                }
                continue;
            }
        };
        let player = match players.get_mut(&node.id) {
            Some(p) => {
                if p.given != settings {
                    for text in p.follow(settings, wake) {
                        out.push(FromHelper::Error { window: Some(window), context: String::from("MediaPlayer"), text: label(node, &text) });
                    }
                }
                p
            }
            None => players.entry(node.id).or_insert(Player::open(settings, wake)),
        };
        if let Some(due) = player.pump(window, node, out) {
            soonest = Some(soonest.map_or(due, |s| s.min(due)));
        }
    }
    soonest
}

/// Makes call `op` on MediaPlayer `node` of `tree`, opening its clip first
/// when no pump has yet, and answers with its state.
pub(crate) fn call(players: &mut HashMap<NodeId, Player>, tree: &Node, node: NodeId, op: MediaOp, wake: Option<&WakeLoop>) -> Result<MediaState, String> {
    let n = tree.find(node).ok_or_else(|| format!("the window has no node {node}"))?;
    if n.kind != Kind::MediaPlayer {
        return Err(format!("node {node} is a {:?}, not a MediaPlayer", n.kind));
    }
    let settings = Settings::of(n)?;
    let player = players.entry(node).or_insert_with(|| Player::open(settings, wake));
    player.act(op)?;
    Ok(player.state())
}

/// Lets go of every player's texture, made on a device that is gone; the
/// next frame makes it again on the new one.
pub(crate) fn lose_textures(players: &mut HashMap<NodeId, Player>) {
    for p in players.values_mut() {
        p.texture.store(None);
    }
}

/// The sound each MediaPlayer of `tree` handed the system, where the
/// backend measures it; a player not opened yet has handed none.
pub(crate) fn sounds(players: &HashMap<NodeId, Player>, tree: &Node) -> BTreeMap<NodeId, Option<media::Sound>> {
    let mut nodes = Vec::new();
    nodes_of(tree, Kind::MediaPlayer, &mut nodes);
    let none = backend::MEASURES.then_some(media::Sound { peak: 0.0, seconds: 0.0 });
    nodes.into_iter().map(|n| (n.id, players.get(&n.id).map_or(none, Player::sound))).collect()
}

/// Draws a MediaPlayer: its picture fitted inside a box of its `width` and
/// `height`, the room across and the height its place leaves where absent,
/// the rest of the box black, with its controls across the box's foot; or
/// with `audio_only` its controls alone. A click gives the player the
/// focus, whose keys work it as a browser's player's do; a double click
/// and its full screen switch show it alone, filling the screen, its
/// controls hiding once the pointer rests.
pub(crate) fn draw_player(ui: &mut egui::Ui, node: &Node, players: &mut HashMap<NodeId, Player>, format: Option<wgpu::TextureFormat>) -> Result<egui::Rect, String> {
    let settings = Settings::of(node)?;
    let (width, height) = (opt_points(node, "width")?, opt_points(node, "height")?);
    let id = egui::Id::unique(("pwrs-gui-media", node.id));
    let room = ui.available_size();
    let player = players.get_mut(&node.id);
    if settings.audio_only {
        let Some(player) = player else {
            return Ok(ui.label(Phase::Loading.name()).rect);
        };
        if !settings.controls {
            return Ok(ui.label(describe(player)).rect);
        }
        let across = width.unwrap_or(room.x);
        if !across.is_finite() {
            return Err(String::from("a MediaPlayer of sound alone without a width takes the room across its place leaves, and inside a Scroll that room has no end"));
        }
        let (rect, response) = ui.allocate_exact_size(egui::vec2(across, controls::MEDIA_HEIGHT), egui::Sense::click());
        let state = player.state();
        let asked = controls::draw(ui, id, rect, &state, false, false);
        player.take_keys(ui, &response)?;
        player.apply(ui.ctx(), asked)?;
        if player.phase == Phase::Playing {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        return Ok(rect);
    }
    let size = egui::vec2(width.unwrap_or(room.x), height.unwrap_or(room.y));
    if !size.x.is_finite() || !size.y.is_finite() {
        return Err(String::from("a MediaPlayer without a width and a height takes the room its place leaves, and inside a Scroll that room has no end"));
    }
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);
    let Some(player) = player else {
        return Ok(rect);
    };
    player.take_frame().map_err(|why| format!("its frame could not be copied out: {why}"))?;
    if player.phase == Phase::Playing || player.seeking {
        ui.ctx().request_repaint();
    }
    if response.double_clicked() {
        player.apply(ui.ctx(), vec![controls::Asked::FullScreen])?;
    }
    player.take_keys(ui, &response)?;
    if player.full {
        let screen = ui.ctx().viewport_rect();
        let area = egui::Area::new(id.with("full")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ui.ctx(), |ui| {
            let (full, cover) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
            ui.painter().rect_filled(full, 0.0, egui::Color32::BLACK);
            if cover.double_clicked() {
                return player.apply(ui.ctx(), vec![controls::Asked::FullScreen]);
            }
            player.take_keys(ui, &cover)?;
            let moved = ui.input(|i| i.pointer.is_moving() || i.pointer.any_down());
            if moved {
                player.moved = Instant::now();
            }
            player.show(ui, id, full, format, settings.controls && player.moved.elapsed().as_secs_f32() < tokens::MEDIA_BAR.idle)
        });
        area.inner?;
        if player.full {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        return Ok(rect);
    }
    player.show(ui, id, rect, format, settings.controls)?;
    Ok(rect)
}

impl Player {
    /// Shows the picture fitted inside `rect`, and the controls across its
    /// foot where `controls`, making what a person asks of them.
    fn show(&mut self, ui: &mut egui::Ui, id: egui::Id, rect: egui::Rect, format: Option<wgpu::TextureFormat>, controls: bool) -> Result<(), String> {
        if let (Some(shown), Some(format)) = (self.latest.clone(), format) {
            let fitted = fit(rect, shown.frame.size);
            let ctx = ui.ctx();
            let ppp = ctx.pixels_per_point();
            let screen = ctx.viewport_rect().size() * ppp;
            let ((left, right), (shown_left, shown_right)) = surface::edges(fitted.min.x, fitted.max.x, ppp, screen.x.round());
            let ((top, bottom), (shown_top, shown_bottom)) = surface::edges(fitted.min.y, fitted.max.y, ppp, screen.y.round());
            let (across, down) = (right - left, bottom - top);
            if across >= 1.0 && down >= 1.0 {
                let window = [(shown_left - left) / across, (shown_top - top) / down, (shown_right - left) / across, (shown_bottom - top) / down];
                let picture = Picture { frame: shown, window, format, texture: Arc::clone(&self.texture), draw: ctx.cumulative_frame_nr() };
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(fitted, picture));
            }
        }
        let barred = controls && rect.height() >= controls::MEDIA_HEIGHT;
        let foot = if barred { rect.max.y - controls::MEDIA_HEIGHT } else { rect.max.y };
        let state = self.state();
        let mut texts: Vec<String> = media::subtitles::showing(&self.given.cues, state.position).into_iter().map(String::from).collect();
        if let Some(engine) = &self.engine {
            texts.extend(engine.captions(state.position).map_err(|why| format!("the clip's own subtitles could not be read: {why}"))?);
        }
        draw_cues(ui, rect, foot, &texts);
        if !barred {
            return Ok(());
        }
        let bar = egui::Rect::from_min_max(egui::pos2(rect.min.x, foot), rect.max);
        let asked = controls::draw(ui, id, bar, &state, true, self.full);
        self.apply(ui.ctx(), asked)
    }

    /// Makes what a person asked of the controls: a seek while the slider
    /// is dragged goes to a key frame, and to the frame where it is let go.
    fn apply(&mut self, ctx: &egui::Context, asked: Vec<controls::Asked>) -> Result<(), String> {
        for a in asked {
            match a {
                controls::Asked::Toggle => {
                    let op = if self.phase == Phase::Playing { MediaOp::Pause } else { MediaOp::Play };
                    self.act(op)?;
                }
                controls::Asked::Seek { seconds, exact: true } => {
                    self.scrub = None;
                    self.act(MediaOp::Seek { seconds })?;
                }
                controls::Asked::Seek { seconds, exact: false } => {
                    let moved = self.scrub.is_none_or(|was| (was - seconds).abs() >= 0.1);
                    if moved && let Some(engine) = &self.engine {
                        engine.seek_near(seconds)?;
                        self.scrub = Some(seconds);
                        self.last_at = None;
                    }
                }
                controls::Asked::Mute => {
                    let muted = !self.live.muted;
                    self.act(MediaOp::Muted { muted })?;
                }
                controls::Asked::Volume(volume) => {
                    self.act(MediaOp::Volume { volume })?;
                    if self.live.muted && volume > 0.0 {
                        self.act(MediaOp::Muted { muted: false })?;
                    }
                }
                controls::Asked::FullScreen => {
                    self.full = !self.full;
                    self.moved = Instant::now();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.full));
                }
            }
        }
        Ok(())
    }

    /// Takes the focus where the player was clicked, and makes the keys the
    /// focused player takes ([`media::Key`]).
    fn take_keys(&mut self, ui: &mut egui::Ui, response: &egui::Response) -> Result<(), String> {
        if response.clicked() {
            response.request_focus();
        }
        if !response.has_focus() && !self.full {
            return Ok(());
        }
        let pressed: Vec<&'static str> = ui.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } if !modifiers.ctrl && !modifiers.alt && !modifiers.command => key_name(*key),
                    _ => None,
                })
                .collect()
        });
        for name in pressed {
            match media::Key::of(name) {
                Some(media::Key::FullScreen) => self.apply(ui.ctx(), vec![controls::Asked::FullScreen])?,
                Some(media::Key::Leave) if self.full => self.apply(ui.ctx(), vec![controls::Asked::FullScreen])?,
                Some(key) => {
                    if let Some(op) = key.call(&self.state()) {
                        self.act(op)?;
                    }
                }
                None => {}
            }
        }
        Ok(())
    }
}

/// Draws the subtitles `texts` centered across `rect`, their foot
/// [`tokens::MEDIA_CUE`]'s lift above `foot`, as the HTML renderer draws
/// them: each line in the cue's ink on its shade.
fn draw_cues(ui: &egui::Ui, rect: egui::Rect, foot: f32, texts: &[String]) {
    if texts.is_empty() {
        return;
    }
    let cue = &tokens::MEDIA_CUE;
    let color = |c: tokens::Color| egui::Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3]);
    let ink = color(cue.ink);
    let lines: Vec<_> = texts.iter().flat_map(|t| t.lines()).map(|line| ui.painter().layout_no_wrap(line.to_string(), egui::FontId::proportional(cue.size), ink)).collect();
    let across = lines.iter().map(|g| g.size().x).fold(0.0, f32::max);
    let down: f32 = lines.iter().map(|g| g.size().y).sum();
    let size = egui::vec2(across + 2.0 * cue.pad[0], down + 2.0 * cue.pad[1]);
    let shade = egui::Rect::from_min_size(egui::pos2(rect.center().x - size.x / 2.0, foot - cue.lift - size.y), size);
    ui.painter().rect_filled(shade, cue.round, color(cue.shade));
    let mut y = shade.min.y + cue.pad[1];
    for line in lines {
        let height = line.size().y;
        ui.painter().galley(egui::pos2(rect.center().x - line.size().x / 2.0, y), line, ink);
        y += height;
    }
}

/// The name a shortcut gives one of the keys a player takes.
fn key_name(key: egui::Key) -> Option<&'static str> {
    Some(match key {
        egui::Key::Space => "Space",
        egui::Key::Enter => "Enter",
        egui::Key::ArrowLeft => "ArrowLeft",
        egui::Key::ArrowRight => "ArrowRight",
        egui::Key::ArrowUp => "ArrowUp",
        egui::Key::ArrowDown => "ArrowDown",
        egui::Key::M => "M",
        egui::Key::F => "F",
        egui::Key::Escape => "Escape",
        _ => return None,
    })
}

/// A line saying where a player of sound alone is: its phase, and the
/// position and length once the clip is open.
fn describe(p: &Player) -> String {
    let state = p.state();
    let clock = |s: f64| format!("{}:{:02}", (s / 60.0).floor() as u64, (s % 60.0).floor() as u64);
    match state.duration {
        Some(d) if p.loaded => format!("{} {} / {}", state.phase.name(), clock(state.position), clock(d)),
        _ => state.phase.name().to_string(),
    }
}

/// The largest box of a picture of `size` pixels inside `rect`, keeping
/// its proportions, centered in it.
fn fit(rect: egui::Rect, size: [u32; 2]) -> egui::Rect {
    let natural = egui::vec2(size[0] as f32, size[1] as f32);
    let scale = (rect.width() / natural.x).min(rect.height() / natural.y);
    egui::Rect::from_center_size(rect.center(), natural * scale)
}

/// The texture a player's frames are written into, at the picture's size,
/// the number of the frame it holds, and what the blit shows it through.
pub(crate) struct Texture {
    size: [u32; 2],
    texture: wgpu::Texture,
    window: wgpu::Buffer,
    shown: wgpu::BindGroup,
    written: AtomicU64,
}

impl Texture {
    fn new(device: &wgpu::Device, blit: &Blit, size: [u32; 2]) -> Texture {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("PWRS-GUI MediaPlayer"),
            size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Bgra8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let window = device.create_buffer(&wgpu::BufferDescriptor { label: Some("PWRS-GUI MediaPlayer window"), size: 16, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let shown = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PWRS-GUI MediaPlayer shown"),
            layout: &blit.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&blit.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &window, offset: 0, size: None }) },
            ],
        });
        Texture { size, texture, window, shown, written: AtomicU64::new(0) }
    }
}

/// Shows a player's texture in egui's pass, smoothly scaled to the box the
/// callback is given, the part of it the window shows. The picture is
/// opaque. A target in an sRGB format is given the texture's bytes as
/// linear values, so the target's encoding stores the same bytes.
const BLIT: &str = "
struct Shown { top_left: vec2<f32>, bottom_right: vec2<f32> }
@group(0) @binding(0) var texels: texture_2d<f32>;
@group(0) @binding(1) var filtered: sampler;
@group(0) @binding(2) var<uniform> shown: Shown;
struct Corner { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs(@builtin(vertex_index) i: u32) -> Corner {
    let at = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var out: Corner;
    out.position = vec4<f32>(at.x * 2.0 - 1.0, 1.0 - at.y * 2.0, 0.0, 1.0);
    out.uv = shown.top_left + at * (shown.bottom_right - shown.top_left);
    return out;
}
@fragment fn fs_gamma(in: Corner) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(texels, filtered, in.uv).rgb, 1.0);
}
@fragment fn fs_linear(in: Corner) -> @location(0) vec4<f32> {
    let c = textureSample(texels, filtered, in.uv).rgb;
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return vec4<f32>(select(high, low, c <= vec3<f32>(0.04045)), 1.0);
}
";

/// The pipeline the blit draws with for egui's target format, stored in
/// egui-wgpu's callback resources.
struct Blit {
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    pipeline: wgpu::RenderPipeline,
}

impl Blit {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Blit {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("PWRS-GUI MediaPlayer blit"), source: wgpu::ShaderSource::Wgsl(BLIT.into()) });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PWRS-GUI MediaPlayer blit"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { multisampled: false, sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2 },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("PWRS-GUI MediaPlayer blit"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("PWRS-GUI MediaPlayer blit"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: wgpu::PipelineCompilationOptions::default() },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(if format.is_srgb() { "fs_linear" } else { "fs_gamma" }),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("PWRS-GUI MediaPlayer blit"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        Blit { format, layout, sampler, pipeline }
    }
}

/// One frame of a player's picture: the newest frame, written into the
/// texture when the texture does not hold it yet, and the part `window`
/// names, its top-left and bottom-right corners as texture coordinates,
/// shown in egui's pass of target format `format`, in the draw of egui
/// frame `draw`.
struct Picture {
    frame: Arc<Numbered>,
    window: [f32; 4],
    format: wgpu::TextureFormat,
    texture: Arc<ArcSwapOption<Texture>>,
    draw: u64,
}

impl egui_wgpu::CallbackTrait for Picture {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if resources.get::<Blit>().is_none_or(|b| b.format != self.format) {
            resources.insert(Blit::new(device, self.format));
        }
        let Some(blit) = resources.get::<Blit>() else {
            return Vec::new();
        };
        let size = self.frame.frame.size;
        let texture = match self.texture.load_full() {
            Some(t) if t.size == size => t,
            _ => {
                let made = Arc::new(Texture::new(device, blit, size));
                self.texture.store(Some(Arc::clone(&made)));
                made
            }
        };
        let window: Vec<u8> = self.window.iter().flat_map(|v| v.to_le_bytes()).collect();
        queue.write_buffer(&texture.window, 0, &window);
        if texture.written.load(Ordering::Acquire) != self.frame.serial {
            if !self.frame.frame.copy_on_gpu(device, queue, encoder, &texture.texture, self.draw) {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo { texture: &texture.texture, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                    &self.frame.frame.bgra,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size[0] * 4), rows_per_image: Some(size[1]) },
                    wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                );
            }
            texture.written.store(self.frame.serial, Ordering::Release);
        }
        Vec::new()
    }

    fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
        let (Some(blit), Some(texture)) = (resources.get::<Blit>(), self.texture.load_full()) else {
            return;
        };
        pass.set_pipeline(&blit.pipeline);
        pass.set_bind_group(0, &texture.shown, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_wgpu::CallbackTrait;
    use pwrs_gui_gpu::{Gpu, GpuOptions};

    /// A frame two pixels square, in blue, green, red and alpha order: blue
    /// and green in the top row, red and a gray of 100 below them, each with
    /// an alpha of 0, which the blit shows opaque.
    const QUARTERS: [u8; 16] = [255, 0, 0, 0, 0, 255, 0, 0, 0, 0, 255, 0, 100, 100, 100, 0];

    /// `picture` prepared and painted as egui-wgpu prepares and paints a
    /// paint callback, into a target of its format two pixels square, and
    /// read back.
    fn painted(gpu: &Gpu, picture: &Picture, resources: &mut egui_wgpu::CallbackResources) -> Vec<u8> {
        let usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC;
        let target = gpu.create_texture(2, 2, picture.format, Some(usage), "painted").expect("the target");
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: [2, 2], pixels_per_point: 1.0 };
        let mut encoder = gpu.device().create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("painted") });
        let mut commands = picture.prepare(gpu.device(), gpu.queue(), &screen, &mut encoder, resources);
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("painted"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            let whole = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(2.0, 2.0));
            picture.paint(egui::PaintCallbackInfo { viewport: whole, clip_rect: whole, pixels_per_point: 1.0, screen_size_px: [2, 2] }, &mut pass, resources);
        }
        commands.push(encoder.finish());
        gpu.queue().submit(commands);
        gpu.read_texture(&target).expect("the pixels")
    }

    #[test]
    fn the_blit_shows_a_frame_opaque_texel_for_pixel_in_targets_that_encode_srgb_and_ones_that_do_not() {
        let gpu = Gpu::open(&GpuOptions::default()).expect("a device opens");
        let frame = Arc::new(Numbered {
            serial: 1,
            frame: Frame {
                size: [2, 2],
                bgra: QUARTERS.to_vec(),
                #[cfg(windows)]
                shared: None,
                #[cfg(target_os = "macos")]
                metal: None,
            },
        });
        for format in [wgpu::TextureFormat::Rgba8Unorm, wgpu::TextureFormat::Rgba8UnormSrgb] {
            let mut resources = egui_wgpu::CallbackResources::default();
            let picture = Picture { frame: Arc::clone(&frame), window: [0.0, 0.0, 1.0, 1.0], format, texture: Arc::default(), draw: 0 };
            let shown = painted(&gpu, &picture, &mut resources);
            // A target that encodes sRGB rounds the blit's linear values back
            // to bytes, each within one of the frame's.
            let tolerance = u8::from(format.is_srgb());
            let want = [0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 100, 100, 100, 255];
            assert!(shown.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= tolerance), "{format:?}: blue, green, red and gray, opaque: {shown:?}");
            let written = picture.texture.load_full().expect("the texture made").written.load(Ordering::Acquire);
            assert_eq!(written, 1, "the texture holds the frame written");
        }
    }

    #[test]
    fn a_picture_fits_inside_its_box_keeping_its_proportions() {
        let wide = fit(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0)), [320, 160]);
        assert_eq!((wide.width(), wide.height()), (400.0, 200.0));
        assert_eq!(wide.center(), egui::pos2(200.0, 200.0), "centered, black above and below");
        let tall = fit(egui::Rect::from_min_size(egui::pos2(10.0, 0.0), egui::vec2(400.0, 100.0)), [320, 240]);
        assert_eq!((tall.width().round(), tall.height()), (133.0, 100.0));
    }
}
