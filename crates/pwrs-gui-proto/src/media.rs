//! A MediaPlayer's settings, the calls a session makes on one, the state a
//! renderer answers them with, and the values of its events, which every
//! renderer reads and writes alike.

pub mod probe;
pub mod subtitles;

use crate::{Node, Value};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

/// A call on a MediaPlayer, which [`crate::ToHelper::Media`] carries. Each
/// is answered with the player's state once the renderer has made it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MediaOp {
    /// Starts playing, or plays on from where it paused; at the end, plays
    /// again from the start.
    Play,
    /// Pauses where it is.
    Pause,
    /// Moves to `seconds` from the clip's start, to the frame.
    Seek { seconds: f64 },
    /// Plays at `rate` times the clip's own speed.
    Rate { rate: f64 },
    /// Plays at `volume`, from 0 for silence to 1 for the clip's own level.
    Volume { volume: f64 },
    /// Silences the sound, or lets it be heard again, at its volume.
    Muted { muted: bool },
    /// Plays again from the start each time the clip ends, or stops at its
    /// end.
    Loop { looped: bool },
    /// Changes nothing, so the answer is the state alone.
    State,
}

/// Where a player is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    /// The clip is opening, and the backend does not yet know its length.
    Loading,
    /// The clip is open and stands still.
    Paused,
    Playing,
    /// The clip played to its end and stopped there.
    Ended,
    /// The clip could not be played; the error event said why.
    Failed,
}

impl Phase {
    /// The name PowerShell gives it.
    pub fn name(self) -> &'static str {
        match self {
            Phase::Loading => "Loading",
            Phase::Paused => "Paused",
            Phase::Playing => "Playing",
            Phase::Ended => "Ended",
            Phase::Failed => "Failed",
        }
    }
}

/// A MediaPlayer's state, as a renderer answers a [`MediaOp`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaState {
    pub phase: Phase,
    /// Seconds from the clip's start.
    pub position: f64,
    /// The clip's length in seconds, once the backend knows it.
    pub duration: Option<f64>,
    pub rate: f64,
    pub volume: f64,
    pub muted: bool,
    pub looped: bool,
    /// The picture's size in pixels; none for a clip of sound alone, and
    /// before the backend knows it.
    pub video: Option<[u32; 2]>,
    /// Whether the clip has sound.
    pub audio: bool,
    /// Why the clip could not be played, once it could not.
    pub error: Option<String>,
}

/// The origin of the web helper's pages, as their engine names it: wry
/// serves the `pwrs` scheme as `http://pwrs.localhost` on Windows and as
/// `pwrs://localhost` elsewhere. A player whose engine fetches an https
/// clip itself asks the clip's server under CORS as this origin, in both
/// renderers.
pub const PAGE_ORIGIN: &str = if cfg!(windows) { "http://pwrs.localhost" } else { "pwrs://localhost" };

/// The detail of the Error of an https clip whose engine fetches it itself
/// where its server lets no page of [`PAGE_ORIGIN`] read it.
pub fn refused_by_cors(address: &str) -> String {
    format!("the server of {address} sends no CORS header letting pages of {PAGE_ORIGIN} read it, which a player's own fetch of it needs")
}

/// The sound a MediaPlayer handed the system, as its renderer measures it,
/// which a snapshot gives.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sound {
    /// The highest sample of the last [`PEAK_WINDOW`], from 0 for silence to
    /// 1 for full scale, after the player's volume and mute.
    pub peak: f32,
    /// How many seconds of the clip the player has played: the time its
    /// position moved on while it played, seeks and loops left out.
    pub seconds: f64,
}

/// How far back a [`Sound`]'s peak looks: half a second.
pub const PEAK_WINDOW: Duration = Duration::from_millis(500);

/// What a renderer's tap on a player's sound saw: each block's highest
/// sample and the span of the clip it holds, which give the peak of a
/// player's [`Sound`]. A tap sees each block before it is heard, as the
/// system's player takes its sound ahead of playing it, so blocks are
/// placed by the clip's time rather than by when the tap saw them. A block
/// can hold a second of sound or more, so each counts for all of its span.
#[derive(Clone, Debug, Default)]
pub struct Meter {
    /// Each block's start and end in the clip, in seconds, and its peak.
    blocks: std::collections::VecDeque<(f64, f64, f32)>,
}

impl Meter {
    /// A block of sound whose highest sample is `peak`, before the player's
    /// volume, from `at` to `until` seconds into the clip. A block that
    /// starts before the last one, as after a seek back or a loop, starts
    /// the record again.
    pub fn heard(&mut self, at: f64, until: f64, peak: f32) {
        if self.blocks.back().is_some_and(|(last, _, _)| at < *last) {
            self.blocks.clear();
        }
        self.blocks.push_back((at, until.max(at), peak));
    }

    /// The highest sample of the blocks that sound in the [`PEAK_WINDOW`]
    /// of the clip up to `position` seconds, scaled by `gain`: the player's
    /// volume, or 0 while it is muted.
    pub fn peak(&self, position: f64, gain: f32) -> f32 {
        let from = position - PEAK_WINDOW.as_secs_f64();
        let recent = self.blocks.iter().filter(|(at, until, _)| *at <= position && *until >= from);
        (recent.map(|(_, _, p)| *p).fold(0.0, f32::max) * gain).clamp(0.0, 1.0)
    }

    /// Lets go of the blocks that end more than [`PEAK_WINDOW`] before
    /// `position`, which no peak reads again.
    pub fn forget(&mut self, position: f64) {
        let from = position - PEAK_WINDOW.as_secs_f64();
        while self.blocks.front().is_some_and(|(_, until, _)| *until < from) {
            self.blocks.pop_front();
        }
    }
}

/// What a MediaPlayer node asks for, read from its properties as
/// [`Kind::MediaPlayer`](crate::Kind::MediaPlayer) gives them.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// The clip's file, its full path.
    pub source: String,
    /// Whether it plays the clip's sound alone, with no picture.
    pub audio_only: bool,
    /// Whether it starts playing as soon as the clip is open.
    pub autoplay: bool,
    pub looped: bool,
    pub muted: bool,
    pub volume: f64,
    pub rate: f64,
    /// The time between Position events while it plays, which it also
    /// raises after a seek and at the clip's end; none where it raises no
    /// Position.
    pub position_every: Option<Duration>,
    /// Whether it draws its controls, as it does unless `no_controls`.
    pub controls: bool,
    /// The cues of its `subtitles`, read from a sidecar file.
    pub cues: Vec<subtitles::Cue>,
    /// Whether its engine fetches an https clip itself, as `engine_fetch`
    /// asks, on the system's own network stack, in place of the helper's
    /// loader; a server that lets no page of [`PAGE_ORIGIN`] read the clip
    /// fails it.
    pub engine_fetch: bool,
}

impl Settings {
    /// The settings `node`'s properties give: `source` is required, and
    /// every other has the default its kind documents. A property of the
    /// wrong type, a volume outside 0 to 1, or a rate or a position
    /// interval that is not above 0 is refused.
    pub fn of(node: &Node) -> Result<Settings, String> {
        let source = match node.props.get("source") {
            Some(Value::Str(s)) if !s.is_empty() => s.clone(),
            Some(Value::Str(_)) | None | Some(Value::Null) => return Err(String::from("a MediaPlayer needs source, the full path of its clip's file")),
            Some(other) => return Err(format!("source must be a path, not {other:?}")),
        };
        let flag = |key: &str| match node.props.get(key) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(b)) => Ok(*b),
            Some(other) => Err(format!("{key} must be true or false, not {other:?}")),
        };
        let number = |key: &str, default: f64| match node.props.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v.as_number().filter(|n| n.is_finite()).ok_or_else(|| format!("{key} must be a finite number, not {v:?}")),
        };
        let volume = number("volume", 1.0)?;
        if !(0.0..=1.0).contains(&volume) {
            return Err(format!("volume is {volume}; it runs from 0 for silence to 1"));
        }
        let rate = number("rate", 1.0)?;
        check_rate(rate)?;
        let interval = number("position_interval", POSITION_INTERVAL)?;
        let every = match Duration::try_from_secs_f64(interval) {
            Ok(d) if !d.is_zero() => d,
            Ok(_) => return Err(format!("position_interval is {interval}; it is the seconds between Position events, above 0")),
            Err(e) => return Err(format!("position_interval is {interval}, which is no length of time: {e}")),
        };
        Ok(Settings {
            source,
            audio_only: flag("audio_only")?,
            autoplay: flag("autoplay")?,
            looped: flag("loop")?,
            muted: flag("muted")?,
            volume,
            rate,
            position_every: flag("position")?.then_some(every),
            controls: !flag("no_controls")?,
            cues: match node.props.get("subtitles") {
                None | Some(Value::Null) => Vec::new(),
                Some(v) => subtitles::of(v)?,
            },
            engine_fetch: flag("engine_fetch")?,
        })
    }
}

/// The slowest and the fastest rate a MediaPlayer plays at, as times the
/// clip's own speed; every backend keeps the sound's pitch between them.
pub const RATES: [f64; 2] = [0.25, 4.0];

/// Refuses a rate outside [`RATES`].
pub fn check_rate(rate: f64) -> Result<(), String> {
    if (RATES[0]..=RATES[1]).contains(&rate) { Ok(()) } else { Err(format!("rate is {rate}; a clip plays at {} to {} times its own speed", RATES[0], RATES[1])) }
}

/// How often a playing MediaPlayer whose `position` is true raises
/// Position, in seconds, where its `position_interval` is absent.
pub const POSITION_INTERVAL: f64 = 0.25;

/// A key a focused MediaPlayer takes, as a browser's player takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Space or Enter: plays, or pauses a clip playing.
    PlayPause,
    /// The left arrow: [`STEP`] seconds back.
    Back,
    /// The right arrow: [`STEP`] seconds on.
    Forward,
    /// The up arrow: the volume up by [`VOLUME_STEP`].
    Louder,
    /// The down arrow: the volume down by [`VOLUME_STEP`].
    Quieter,
    /// M: silences the sound, or lets it be heard again.
    Mute,
    /// F: full screen, or back from it.
    FullScreen,
    /// Escape: back from full screen.
    Leave,
}

/// The keys a focused MediaPlayer keeps from the window's `Key` events, by
/// the names a shortcut gives them.
pub const KEYS: [&str; 9] = ["Space", "Enter", "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "M", "F", "Escape"];

/// How far the arrows move a player, in seconds.
pub const STEP: f64 = 5.0;

/// How much the arrows change a player's volume.
pub const VOLUME_STEP: f64 = 0.1;

impl Key {
    /// The key a shortcut's key name gives; none for one the player leaves
    /// to the window.
    pub fn of(key: &str) -> Option<Key> {
        Some(match key {
            "Space" | "Enter" => Key::PlayPause,
            "ArrowLeft" => Key::Back,
            "ArrowRight" => Key::Forward,
            "ArrowUp" => Key::Louder,
            "ArrowDown" => Key::Quieter,
            "M" => Key::Mute,
            "F" => Key::FullScreen,
            "Escape" => Key::Leave,
            _ => return None,
        })
    }

    /// The call the key makes on a player in `state`; none for the keys of
    /// full screen, which the renderer makes itself.
    pub fn call(self, state: &MediaState) -> Option<MediaOp> {
        let end = state.duration.unwrap_or(f64::MAX);
        Some(match self {
            Key::PlayPause if state.phase == Phase::Playing => MediaOp::Pause,
            Key::PlayPause => MediaOp::Play,
            Key::Back => MediaOp::Seek { seconds: (state.position - STEP).max(0.0) },
            Key::Forward => MediaOp::Seek { seconds: (state.position + STEP).min(end) },
            Key::Louder => MediaOp::Volume { volume: (state.volume + VOLUME_STEP).min(1.0) },
            Key::Quieter => MediaOp::Volume { volume: (state.volume - VOLUME_STEP).max(0.0) },
            Key::Mute => MediaOp::Muted { muted: !state.muted },
            Key::FullScreen | Key::Leave => return None,
        })
    }
}

/// The value of a MediaPlayer's Loaded event: the clip's `duration` in
/// seconds, null while the backend cannot tell it, the picture's `width`
/// and `height` in pixels, null for a clip of sound alone, and whether it
/// has a picture and sound.
pub fn loaded_value(duration: Option<f64>, video: Option<[u32; 2]>, audio: bool) -> Value {
    let size = |i: usize| video.map_or(Value::Null, |v| Value::Int(i64::from(v[i])));
    Value::Map(BTreeMap::from([
        (String::from("duration"), duration.map_or(Value::Null, Value::Float)),
        (String::from("width"), size(0)),
        (String::from("height"), size(1)),
        (String::from("has_video"), Value::Bool(video.is_some())),
        (String::from("has_audio"), Value::Bool(audio)),
    ]))
}

/// Why a clip cannot be played: the four causes HTML's `MediaError` gives,
/// onto which each backend's own errors are mapped, so both renderers word
/// an error alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// Reading the clip was stopped before it was read.
    Aborted,
    /// The clip's file or address could not be read.
    Network,
    /// The clip was read, but its picture or sound could not be decoded.
    Decode,
    /// Nothing here plays the clip's format, or the system has no player.
    FormatNotSupported,
}

impl Cause {
    /// The name PowerShell gives it.
    pub fn name(self) -> &'static str {
        match self {
            Cause::Aborted => "Aborted",
            Cause::Network => "Network",
            Cause::Decode => "Decode",
            Cause::FormatNotSupported => "FormatNotSupported",
        }
    }

    /// The cause `MediaError` numbers `code`, 1 to 4.
    pub fn of_code(code: i64) -> Result<Cause, String> {
        match code {
            1 => Ok(Cause::Aborted),
            2 => Ok(Cause::Network),
            3 => Ok(Cause::Decode),
            4 => Ok(Cause::FormatNotSupported),
            other => Err(format!("{other} is no MediaError code; the codes run from 1 to 4")),
        }
    }

    /// What the cause is, in the Error event's message.
    fn said(self) -> &'static str {
        match self {
            Cause::Aborted => "reading it was stopped",
            Cause::Network => "it could not be read",
            Cause::Decode => "it could not be decoded",
            Cause::FormatNotSupported => "its format is not supported here",
        }
    }
}

/// What a MediaPlayer's Error event names as its remedy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remedy<'a> {
    None,
    /// What plays a clip this player cannot: the FFmpeg add-on, or another
    /// renderer. It is named only for a clip whose format is known.
    Player(&'a str),
    /// What to install for the system's player to play at all.
    Install(&'a str),
}

/// The value of a MediaPlayer's Error event, worded alike in both
/// renderers: `message`, which names the clip's `address`, the `cause` and
/// the clip's `format` where known; those three as fields of their own;
/// `remedy`; and `detail`, the engine's or the backend's own text.
pub fn error_value(address: &str, cause: Cause, format: Option<&str>, remedy: Remedy<'_>, detail: &str) -> Value {
    let message = match format {
        Some(f) => format!("{address} could not be played: {}; it holds {f}", cause.said()),
        None => format!("{address} could not be played: {}", cause.said()),
    };
    let remedy = match remedy {
        Remedy::Player(r) if format.is_some() => Value::Str(r.to_string()),
        Remedy::Install(r) => Value::Str(r.to_string()),
        Remedy::Player(_) | Remedy::None => Value::Null,
    };
    Value::Map(BTreeMap::from([
        (String::from("message"), Value::Str(message)),
        (String::from("address"), Value::Str(address.to_string())),
        (String::from("cause"), Value::Str(cause.name().to_string())),
        (String::from("format"), format.map_or(Value::Null, |f| Value::Str(f.to_string()))),
        (String::from("remedy"), remedy),
        (String::from("detail"), Value::Str(detail.to_string())),
    ]))
}

/// The Error value of the clip at `address`, a file's path, that cannot be
/// played for `cause`: its format is read from its headers, and why it
/// cannot be is added to `detail`.
pub fn file_error(address: &str, cause: Cause, remedy: Remedy<'_>, detail: &str) -> Value {
    clip_error(address, cause, probe::read(std::path::Path::new(address)).map(|f| f.name()), remedy, detail)
}

/// The Error value of the clip at `address` that cannot be played for
/// `cause`, `format` the format its headers name or why they name none,
/// which is added to `detail`.
pub fn clip_error(address: &str, cause: Cause, format: Result<String, String>, remedy: Remedy<'_>, detail: &str) -> Value {
    match format {
        Ok(name) => error_value(address, cause, Some(&name), remedy, detail),
        Err(why) => error_value(address, cause, None, remedy, &format!("{detail}; its headers name no format: {why}")),
    }
}

/// The FFmpeg add-on of the platform this build runs on, which plays the
/// formats the system cannot.
pub fn ffmpeg_addon() -> &'static str {
    if cfg!(target_os = "windows") {
        "PWRS-GUI.FFmpeg.win-x64"
    } else if cfg!(target_os = "macos") {
        "PWRS-GUI.FFmpeg.osx-arm64"
    } else if cfg!(target_os = "freebsd") {
        "PWRS-GUI.FFmpeg.freebsd-x64"
    } else {
        "PWRS-GUI.FFmpeg.linux-x64"
    }
}

/// The remedy an Error event names for a clip the system's backend cannot
/// play: the FFmpeg add-on, which is not installed.
pub fn addon_remedy() -> String {
    format!("Install-Module {}", ffmpeg_addon())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Kind;

    fn player() -> Node {
        Node::new(1, Kind::MediaPlayer).prop("source", Value::Str(String::from("C:\\clips\\tone.mp4")))
    }

    #[test]
    fn a_player_reads_its_source_and_takes_the_defaults_for_the_rest() {
        let s = Settings::of(&player()).expect("settings");
        assert_eq!(s.source, "C:\\clips\\tone.mp4");
        assert!(!s.audio_only && !s.autoplay && !s.looped && !s.muted && s.position_every.is_none());
        assert_eq!((s.volume, s.rate), (1.0, 1.0));
        let set = player().prop("loop", Value::Bool(true)).prop("volume", Value::Float(0.5)).prop("rate", Value::Int(2)).prop("position", Value::Bool(true));
        let s = Settings::of(&set).expect("settings");
        assert!(s.looped);
        assert_eq!((s.volume, s.rate, s.position_every), (0.5, 2.0, Some(Duration::from_millis(250))));
        let every = Settings::of(&set.clone().prop("position_interval", Value::Float(1.5))).expect("settings");
        assert_eq!(every.position_every, Some(Duration::from_millis(1500)));
        let quiet = Settings::of(&player().prop("position_interval", Value::Float(1.5))).expect("settings");
        assert_eq!(quiet.position_every, None, "no Position without a handler for it");
    }

    #[test]
    fn a_meter_gives_the_peak_of_the_half_second_of_the_clip_up_to_its_position_after_the_volume() {
        let mut meter = Meter::default();
        assert_eq!(meter.peak(0.0, 1.0), 0.0, "nothing heard yet");
        meter.heard(0.0, 0.1, 0.9);
        meter.heard(0.3, 0.4, 0.4);
        meter.heard(1.5, 1.6, 0.7);
        assert_eq!(meter.peak(0.4, 1.0), 0.9, "a block seen ahead of the position is not heard yet");
        assert_eq!(meter.peak(0.7, 0.5), 0.2, "the louder block ended over half a second back, and the volume halves the rest");
        assert_eq!(meter.peak(0.95, 1.0), 0.0, "silence once every block heard ended half a second back");
        meter.forget(1.0);
        assert_eq!(meter.peak(1.5, 1.0), 0.7, "a block ahead is kept until it is heard");
        meter.heard(0.2, 0.3, 2.0);
        assert_eq!(meter.peak(1.5, 1.0), 0.0, "a block before the last starts the record again");
        assert_eq!(meter.peak(0.2, 0.0), 0.0, "muted");
        assert_eq!(meter.peak(0.2, 1.0), 1.0, "full scale at most");
    }

    #[test]
    fn a_block_that_starts_before_the_half_second_and_still_sounds_in_it_counts() {
        let mut meter = Meter::default();
        meter.heard(0.0, 1.0, 0.5);
        assert_eq!(meter.peak(0.65, 1.0), 0.5, "the block started 0.65 s back and sounds until 1 s");
        meter.heard(1.0, 2.0, 0.25);
        assert_eq!(meter.peak(1.4, 1.0), 0.5, "the first block ended 0.4 s back, inside the half second");
        assert_eq!(meter.peak(1.6, 1.0), 0.25, "the first block ended 0.6 s back, the second sounds on");
    }

    #[test]
    fn forget_keeps_a_block_until_it_ended_half_a_second_back() {
        let mut meter = Meter::default();
        meter.heard(0.0, 1.0, 0.5);
        meter.heard(1.0, 2.0, 0.25);
        meter.forget(1.2);
        assert_eq!(meter.peak(1.2, 1.0), 0.5, "the first block started 1.2 s back but ended only 0.2 s back, so it is kept");
        meter.forget(1.6);
        assert_eq!(meter.peak(1.4, 1.0), 0.25, "once it ended over half a second back it is let go");
    }

    #[test]
    fn a_player_refuses_a_missing_source_a_volume_past_one_and_a_rate_outside_its_range() {
        assert!(Settings::of(&Node::new(1, Kind::MediaPlayer)).expect_err("no source").contains("source"));
        assert!(Settings::of(&player().prop("volume", Value::Float(1.5))).expect_err("too loud").contains("volume is 1.5"));
        assert!(Settings::of(&player().prop("rate", Value::Float(0.0))).expect_err("no speed").contains("rate is 0"));
        assert!(Settings::of(&player().prop("rate", Value::Float(4.5))).expect_err("too fast").contains("0.25 to 4 times"));
        assert!(Settings::of(&player().prop("rate", Value::Float(0.25))).is_ok(), "the slowest rate");
        assert!(Settings::of(&player().prop("position_interval", Value::Float(0.0))).expect_err("no interval").contains("position_interval is 0"));
        assert!(Settings::of(&player().prop("muted", Value::Str(String::from("yes")))).is_err());
    }

    #[test]
    fn a_loaded_event_gives_the_length_and_the_picture_s_size() {
        let v = loaded_value(Some(2.0), Some([320, 240]), true);
        let m = v.as_map().expect("a map");
        assert_eq!(m.get("duration"), Some(&Value::Float(2.0)));
        assert_eq!(m.get("width"), Some(&Value::Int(320)));
        assert_eq!(m.get("has_video"), Some(&Value::Bool(true)));
        let sound = loaded_value(None, None, true);
        assert_eq!(sound.as_map().and_then(|m| m.get("height")), Some(&Value::Null));
        assert_eq!(sound.as_map().and_then(|m| m.get("has_video")), Some(&Value::Bool(false)));
    }

    #[test]
    fn the_remedy_names_this_platform_s_add_on() {
        assert!(addon_remedy().starts_with("Install-Module PWRS-GUI.FFmpeg."));
    }

    #[test]
    fn an_error_names_the_address_the_cause_and_the_format_with_the_engine_s_text_apart() {
        let field = |v: &Value, key: &str| v.as_map().and_then(|m| m.get(key)).cloned().unwrap_or(Value::Null);
        let v = error_value("C:\\clips\\tone.aiff", Cause::FormatNotSupported, Some("PCM audio in AIFF"), Remedy::Player("-Renderer Egui"), "the engine said no");
        assert_eq!(field(&v, "message"), Value::Str(String::from("C:\\clips\\tone.aiff could not be played: its format is not supported here; it holds PCM audio in AIFF")));
        assert_eq!(field(&v, "cause"), Value::Str(String::from("FormatNotSupported")));
        assert_eq!(field(&v, "remedy"), Value::Str(String::from("-Renderer Egui")));
        assert_eq!(field(&v, "detail"), Value::Str(String::from("the engine said no")));
        let unknown = error_value("C:\\notes.txt", Cause::FormatNotSupported, None, Remedy::Player("-Renderer Egui"), "");
        assert_eq!(field(&unknown, "remedy"), Value::Null, "no other player is named for a file of no known format");
        let missing = error_value("C:\\clips\\tone.mp4", Cause::FormatNotSupported, None, Remedy::Install("Install-WindowsFeature Server-Media-Foundation"), "");
        assert_eq!(field(&missing, "remedy"), Value::Str(String::from("Install-WindowsFeature Server-Media-Foundation")));
        let read = file_error("C:\\no\\such.mp4", Cause::Network, Remedy::None, "the backend could not open it");
        assert_eq!(field(&read, "message"), Value::Str(String::from("C:\\no\\such.mp4 could not be played: it could not be read")));
        assert!(matches!(field(&read, "detail"), Value::Str(d) if d.starts_with("the backend could not open it; its headers name no format: ")));
        assert_eq!(Cause::of_code(3).expect("a cause"), Cause::Decode);
        assert!(Cause::of_code(5).is_err());
    }
}
