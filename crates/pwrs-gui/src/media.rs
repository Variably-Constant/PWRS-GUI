//! `Invoke-GuiMedia`: plays a video or a sound file in a window of its own,
//! a MediaPlayer filling it.

use crate::canvas::TICKS_PER_SECOND;
use crate::options::GuiRenderer;
use crate::session::{self, gui_error};
use crate::{widgets, window};
use pwrs::prelude::*;
use pwrs::sys::PS_TYPE_TIMESPAN;
use pwrs_gui_client::WindowEvent;
use pwrs_gui_client::proto::media::{self, MediaOp, MediaState, Phase, probe};
use pwrs_gui_client::proto::{EventKind, Fit, Value, WindowOptions};

/// The name the window's player goes by, which Update-GuiWidget and the
/// player's events give.
const PLAYER: &str = "Player";

/// The inside size in points of a window showing a clip whose headers give
/// no picture's size.
const PICTURE: [f32; 2] = [640.0, 360.0];

/// The inside size in points of a window showing the clip at `path`: its
/// picture's size in pixels, a point to a pixel, as its headers give it,
/// else [`PICTURE`]. A file's headers are read here; an https clip's by the
/// renderer's loader, through a probe of `renderer`'s session. Why the
/// headers could not be read goes in the Verbose stream; the player's
/// Error event says it again as the clip fails.
fn picture(ps: &Pipeline<'_>, path: &str, renderer: Option<GuiRenderer>) -> PsResult<[f32; 2]> {
    let read = if path.starts_with("https://") {
        let session = match renderer {
            Some(r) => session::widget_session_for(ps, r.into())?,
            None => session::widget_session(ps)?,
        };
        session.probe(path, session::arm(ps)).map_err(|e| e.to_string())
    } else {
        probe::read(std::path::Path::new(&widgets::full_path(ps, path)?))
    };
    match read {
        Ok(format) => Ok(format.size.map_or(PICTURE, |[w, h]| [w as f32, h as f32])),
        Err(why) => {
            pwrs::verbose!(ps, "{path}: the clip's picture's size cannot be read, so its window opens at 640 by 360 points: {why}")?;
            Ok(PICTURE)
        }
    }
}

/// Plays the video or sound file -Path names in a window of its own, the
/// clip filling it, starting at once. The window opens at the picture's
/// size in pixels as points, or at 640 by 360 points where the clip's
/// headers give none. Without -Wait it writes the window once the clip is
/// open, and the window stays open on the clip's last frame when it ends,
/// until it is closed. With -Wait it returns when the clip ends, closing
/// its window, or when the window is closed or Ctrl+C is pressed. A clip
/// that cannot be played ends it with an error naming its format and what
/// plays it instead. With -AudioOnly the clip's sound plays with no window
/// shown, in the egui renderer whatever -Renderer says; the window written
/// still takes Get-GuiMediaPlayer, Set-GuiMediaPlayer and Close-GuiWindow.
///
/// # Examples
/// Invoke-GuiMedia .\clip.mp4 -Wait
/// Invoke-GuiMedia .\song.mp3 -AudioOnly -Volume 0.5
#[cmdlet(verb = "Invoke", noun = "GuiMedia", output = ["PwrsGui.Window"])]
#[derive(Default)]
pub struct InvokeGuiMedia {
    /// The clip's file, resolved from the current location as PowerShell
    /// resolves a path, with wildcards taken literally; or the clip's https
    /// address, which the module's loader fetches by byte ranges.
    #[param(mandatory, position = 0)]
    pub path: String,
    /// Plays the clip's sound alone, with no window shown.
    #[param]
    pub audio_only: bool,
    /// Returns when the clip ends, its window closing, rather than once the
    /// window has opened.
    #[param]
    pub wait: bool,
    /// Plays the clip again from the start each time it ends; with -Wait,
    /// until the window is closed or Ctrl+C is pressed.
    #[param]
    pub r#loop: bool,
    /// Silences the clip's sound.
    #[param]
    pub muted: bool,
    /// The volume, from 0 for silence to 1 for the clip's own level; 1 when
    /// not given.
    #[param]
    pub volume: Option<f64>,
    /// How many times the clip's own speed it plays at, from 0.25 to 4, the
    /// sound's pitch kept; 1 when not given.
    #[param]
    pub rate: Option<f64>,
    /// A SubRip (.srt) or WebVTT (.vtt) file of subtitles, resolved from
    /// the current location, whose cues show over the picture at their
    /// times.
    #[param]
    pub subtitles: Option<String>,
    /// Has the player's own engine fetch an https clip, on the system's
    /// network stack, in place of the module's loader; the clip's server
    /// must allow it under CORS.
    #[param]
    pub engine_fetch: bool,
    /// The window's title; the file's name when none is given.
    #[param]
    pub title: Option<String>,
    /// Draws the window with this renderer instead of the default one
    /// Set-GuiOption -Renderer chose.
    #[param]
    pub renderer: Option<GuiRenderer>,
}

impl InvokeGuiMedia {
    /// Refuses, under the ids New-GuiMediaPlayer raises, a file that is not
    /// there, a volume or a rate out of range and subtitles that cannot be
    /// read, before anything opens.
    fn check(&self, ps: &Pipeline<'_>) -> PsResult<()> {
        if !self.path.starts_with("https://") {
            let full = widgets::full_path(ps, &self.path)?;
            if !std::path::Path::new(&full).is_file() {
                return Err(PsError::new(ErrorCategory::ObjectNotFound, "GuiNoFile", format!("{}: no file is at {full}", self.path)).terminating());
            }
        }
        let bad = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadNumber", text).terminating();
        if let Some(volume) = self.volume
            && !(0.0..=1.0).contains(&volume)
        {
            return Err(bad(format!("-Volume is {volume}; it runs from 0 for silence to 1")));
        }
        if let Some(rate) = self.rate
            && let Err(why) = media::check_rate(rate)
        {
            return Err(bad(format!("-Rate: {why}")));
        }
        if let Some(file) = &self.subtitles {
            let full = widgets::full_path(ps, file)?;
            let read = std::fs::read_to_string(&full).map_err(|e| format!("{full} cannot be read as text: {e}")).and_then(|text| media::subtitles::parse(&text).map(|cues| cues.len()).map_err(|e| e.to_string()));
            if let Err(why) = read {
                return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadProperty", format!("-Subtitles {file}: {why}")).terminating());
            }
        }
        Ok(())
    }
}

impl Cmdlet for InvokeGuiMedia {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        self.check(ps)?;
        let mut given = vec![("Path", self.path.clone().into_ps()?), ("Name", PLAYER.into_ps()?), ("AutoPlay", true.into_ps()?)];
        for (label, on) in [("AudioOnly", self.audio_only), ("Loop", self.r#loop), ("Muted", self.muted), ("EngineFetch", self.engine_fetch)] {
            if on {
                given.push((label, true.into_ps()?));
            }
        }
        for (label, number) in [("Volume", self.volume), ("Rate", self.rate)] {
            if let Some(n) = number {
                given.push((label, n.into_ps()?));
            }
        }
        if let Some(file) = &self.subtitles {
            given.push(("Subtitles", file.clone().into_ps()?));
        }
        let player = ps.invoke("PWRS-GUI\\New-GuiMediaPlayer", &given).map_err(PsError::terminating)?;
        let title = match &self.title {
            Some(t) => t.clone(),
            None => std::path::Path::new(&self.path).file_name().map_or_else(|| self.path.clone(), |n| n.to_string_lossy().into_owned()),
        };
        let (width, height) = if self.audio_only {
            (None, None)
        } else {
            let [w, h] = picture(ps, &self.path, self.renderer)?;
            (Some(w), Some(h))
        };
        let options = WindowOptions {
            title,
            width,
            height,
            resizable: true,
            always_on_top: false,
            close_on_escape: false,
            dialog: self.audio_only,
            fit: if self.audio_only { Fit::DIALOG } else { Fit::WINDOW },
            theme: crate::options::theme(),
            hidden: self.audio_only,
        };
        // A clip's sound alone plays in the egui renderer's hidden window,
        // since no page is needed to play it.
        if self.audio_only {
            self.renderer = Some(GuiRenderer::Egui);
        }
        if !self.wait {
            let path = self.path.clone();
            let mut loaded = |event: &WindowEvent| match event {
                WindowEvent::Event { kind: EventKind::Loaded, .. } => Ok(true),
                WindowEvent::Event { kind: EventKind::Error, value, .. } => Err(unplayable(&path, value)),
                _ => Ok(false),
            };
            if let Some(info) = window::open_widgets_until(ps, &player, options, self.renderer, &mut loaded)? {
                ps.write(info)?;
            }
            return Ok(());
        }
        let built = widgets::build(ps, &player)?;
        let session = match self.renderer {
            Some(r) => session::widget_session_for(ps, r.into())?,
            None => session::widget_session(ps)?,
        };
        let mut shown = session.open_window(options, built.tree.clone()).map_err(gui_error)?;
        let mut problem = None;
        loop {
            let Some(event) = session::next_event(ps, &mut shown)? else {
                return shown.close().map_err(gui_error);
            };
            match event {
                WindowEvent::Event { kind: EventKind::Ended, .. } => return shown.close().map_err(gui_error),
                WindowEvent::Event { kind: EventKind::Error, value, .. } => {
                    shown.close().map_err(gui_error)?;
                    return Err(unplayable(&self.path, &value));
                }
                WindowEvent::Closed(reason) => {
                    return match session::closed_error(reason, problem.as_deref()) {
                        Some(e) => Err(e),
                        None => Ok(()),
                    };
                }
                WindowEvent::Problem(p) => {
                    pwrs::warning!(ps, "the window reported: {p}")?;
                    problem = Some(p);
                }
                _ => {}
            }
        }
    }
}

/// The error a clip that cannot be played ends Invoke-GuiMedia with: its
/// player's Error event's message, which names the clip, the cause and the
/// format, then the remedy where there is one; under GuiMediaUnplayable for
/// a format nothing here plays, else GuiMediaFailed.
fn unplayable(path: &str, value: &Value) -> PsError {
    let map = value.as_map();
    let field = |key: &str| map.and_then(|m| m.get(key)).and_then(Value::as_str);
    let message = match (field("message"), field("remedy")) {
        (Some(m), Some(r)) => format!("{m}; remedy: {r}"),
        (Some(m), None) => m.to_string(),
        (None, _) => format!("{path} could not be played"),
    };
    match field("cause") {
        Some("FormatNotSupported") => PsError::new(ErrorCategory::InvalidData, "GuiMediaUnplayable", message).terminating(),
        _ => PsError::new(ErrorCategory::ResourceUnavailable, "GuiMediaFailed", message).terminating(),
    }
}

/// A refusal of a media command's arguments.
fn refuse(text: String) -> PsError {
    PsError::new(ErrorCategory::InvalidArgument, "GuiBadMedia", text).terminating()
}

/// The seconds from a clip's start `given` names: a number, or a
/// [TimeSpan]'s length, 0 or more.
fn position_of(given: &PsObject) -> PsResult<f64> {
    let seconds = if given.type_tag()? == PS_TYPE_TIMESPAN {
        PsTimeSpan::from_ps(given)?.ticks as f64 / TICKS_PER_SECOND
    } else {
        f64::from_ps(given).map_err(|e| refuse(format!("-Position takes seconds or a [TimeSpan]: {}", e.message)))?
    };
    if seconds.is_finite() && seconds >= 0.0 { Ok(seconds) } else { Err(refuse(format!("-Position is {seconds}; it is the seconds from the clip's start, 0 or more"))) }
}

/// Where a MediaPlayer is.
#[psenum(name = "PwrsGui.MediaPhase")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GuiMediaPhase {
    /// The clip is opening.
    #[default]
    Loading,
    /// The clip is open and stands still.
    Paused,
    /// The clip is playing.
    Playing,
    /// The clip played to its end and stopped there.
    Ended,
    /// The clip could not be played; Error says why.
    Failed,
}

/// What Get-GuiMediaPlayer and Set-GuiMediaPlayer -PassThru write: a
/// MediaPlayer's state.
#[psclass(name = "PwrsGui.MediaPlayer", columns = ["Name", "Phase", "Position", "Duration", "Volume", "Muted"])]
#[derive(Default, Clone)]
pub struct MediaPlayerState {
    /// The player's name.
    pub name: String,
    /// The window's number.
    pub window: u64,
    /// Where the player is: Loading, Paused, Playing, Ended or Failed.
    pub phase: GuiMediaPhase,
    /// The seconds from the clip's start.
    pub position: f64,
    /// The clip's length in seconds; null until the player knows it.
    pub duration: Option<f64>,
    /// How many times the clip's own speed it plays at.
    pub rate: f64,
    /// The volume, from 0 for silence to 1 for the clip's own level.
    pub volume: f64,
    /// Whether the sound is silenced.
    pub muted: bool,
    /// Whether the clip plays again from the start each time it ends.
    pub r#loop: bool,
    /// The picture's width in pixels; null for a clip of sound alone, and until the player knows it.
    pub width: Option<u32>,
    /// The picture's height in pixels; null for a clip of sound alone, and until the player knows it.
    pub height: Option<u32>,
    /// Whether the clip has sound.
    pub has_audio: bool,
    /// Why the clip could not be played; null while it can.
    pub error: Option<String>,
}

impl MediaPlayerState {
    /// `state` as the state of the MediaPlayer named `name` in window
    /// `window`.
    fn of(name: &str, window: u64, state: &MediaState) -> MediaPlayerState {
        MediaPlayerState {
            name: name.to_string(),
            window,
            phase: match state.phase {
                Phase::Loading => GuiMediaPhase::Loading,
                Phase::Paused => GuiMediaPhase::Paused,
                Phase::Playing => GuiMediaPhase::Playing,
                Phase::Ended => GuiMediaPhase::Ended,
                Phase::Failed => GuiMediaPhase::Failed,
            },
            position: state.position,
            duration: state.duration,
            rate: state.rate,
            volume: state.volume,
            muted: state.muted,
            r#loop: state.looped,
            width: state.video.map(|v| v[0]),
            height: state.video.map(|v| v[1]),
            has_audio: state.audio,
            error: state.error.clone(),
        }
    }
}

/// Plays, pauses, seeks and sets a MediaPlayer in an open window, found by
/// its name. -Loop, -Muted, -Volume and -Rate set what the widget's own
/// properties set, until those change again, and -Position moves to that
/// many seconds from the clip's start, or a [TimeSpan]'s, to the frame. The
/// calls are made in that order, then -Play or -Pause; arguments out of
/// range make none. Inside a handler it acts on the handler's window;
/// elsewhere -Window names the window.
///
/// # Examples
/// Set-GuiMediaPlayer Player -Pause
/// Set-GuiMediaPlayer Player -Position 30 -Play -Window $w.Id
/// Set-GuiMediaPlayer Player -Rate 1.5 -Muted -PassThru
#[cmdlet(verb = "Set", noun = "GuiMediaPlayer")]
#[derive(Default)]
pub struct SetGuiMediaPlayer {
    /// The player's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// Starts playing, or plays on from where it paused; at the end, plays again from the start.
    #[param]
    pub play: bool,
    /// Pauses where it is.
    #[param]
    pub pause: bool,
    /// The seconds from the clip's start to move to, or a [TimeSpan], 0 or more.
    #[param]
    pub position: Option<PsObject>,
    /// How many times the clip's own speed it plays at, from 0.25 to 4, the sound's pitch kept.
    #[param]
    pub rate: Option<f64>,
    /// The volume, from 0 for silence to 1 for the clip's own level.
    #[param]
    pub volume: Option<f64>,
    /// Whether the sound is silenced.
    #[param]
    pub muted: Option<bool>,
    /// Whether the clip plays again from the start each time it ends.
    #[param]
    pub r#loop: Option<bool>,
    /// Writes the player's state once the changes are made.
    #[param]
    pub pass_thru: bool,
    /// The window's number; inside a handler, the handler's window when not given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl SetGuiMediaPlayer {
    /// The calls this command's parameters make, in the order made.
    fn calls(&self) -> PsResult<Vec<MediaOp>> {
        if self.play && self.pause {
            return Err(refuse(String::from("-Play starts the clip and -Pause stops it; give one")));
        }
        let mut calls = Vec::new();
        if let Some(looped) = self.r#loop {
            calls.push(MediaOp::Loop { looped });
        }
        if let Some(muted) = self.muted {
            calls.push(MediaOp::Muted { muted });
        }
        if let Some(volume) = self.volume {
            if !(0.0..=1.0).contains(&volume) {
                return Err(refuse(format!("-Volume is {volume}; it runs from 0 for silence to 1")));
            }
            calls.push(MediaOp::Volume { volume });
        }
        if let Some(rate) = self.rate {
            if media::check_rate(rate).is_err() {
                return Err(refuse(format!("-Rate is {rate}; a clip plays at {} to {} times its own speed", media::RATES[0], media::RATES[1])));
            }
            calls.push(MediaOp::Rate { rate });
        }
        if let Some(given) = &self.position {
            calls.push(MediaOp::Seek { seconds: position_of(given)? });
        }
        if self.play {
            calls.push(MediaOp::Play);
        }
        if self.pause {
            calls.push(MediaOp::Pause);
        }
        Ok(calls)
    }
}

impl Cmdlet for SetGuiMediaPlayer {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let calls = self.calls()?;
        let id = window::window_of(ps, self.window)?;
        let node = window::media_player(id, &self.name)?;
        let session = session::of_window(id)?;
        let mut last = None;
        for op in calls {
            last = Some(session.media(id, node, op, session::arm(ps)).map_err(gui_error)?);
        }
        if !self.pass_thru {
            return Ok(());
        }
        let state = match last {
            Some(s) => s,
            None => session.media(id, node, MediaOp::State, session::arm(ps)).map_err(gui_error)?,
        };
        ps.write(MediaPlayerState::of(&self.name, id, &state))
    }
}

/// Writes the state of a MediaPlayer in an open window, found by its name:
/// where it is, its position and length, its rate, volume, mute and loop,
/// its picture's size, whether it has sound, and why it failed. Inside a
/// handler it acts on the handler's window; elsewhere -Window names the
/// window.
///
/// # Examples
/// Get-GuiMediaPlayer Player
/// (Get-GuiMediaPlayer Player -Window $w.Id).Position
#[cmdlet(verb = "Get", noun = "GuiMediaPlayer", output = ["PwrsGui.MediaPlayer"])]
#[derive(Default)]
pub struct GetGuiMediaPlayer {
    /// The player's name, compared without regard to case.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The window's number; inside a handler, the handler's window when not given.
    #[param(value_from_pipeline_by_property_name, alias = ["Id"])]
    pub window: Option<u64>,
}

impl Cmdlet for GetGuiMediaPlayer {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let id = window::window_of(ps, self.window)?;
        let node = window::media_player(id, &self.name)?;
        let session = session::of_window(id)?;
        let state = session.media(id, node, MediaOp::State, session::arm(ps)).map_err(gui_error)?;
        ps.write(MediaPlayerState::of(&self.name, id, &state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The size of every clip's picture.
    const SMALL: Option<[u32; 2]> = Some([160, 120]);

    /// Each clip the media tests play, the format its headers name, and its
    /// picture's size where they give it.
    const CLIPS: &[(&str, &str, Option<[u32; 2]>)] = &[
        ("h264-aac.mp4", "H.264 video and AAC audio in MP4", SMALL),
        ("h264-aac-subs.mp4", "H.264 video and AAC audio in MP4", SMALL),
        ("vp9-opus-subs.mkv", "VP9 video and Opus audio in Matroska", SMALL),
        ("hevc-aac.mp4", "H.265 (HEVC) video and AAC audio in MP4", SMALL),
        ("h264-aac.mov", "H.264 video and AAC audio in QuickTime", SMALL),
        ("h264-aac.mkv", "H.264 video and AAC audio in Matroska", SMALL),
        ("h264-aac.ts", "H.264 video and AAC audio in MPEG-TS", None),
        ("mpeg4-aac.3gp", "MPEG-4 Part 2 video and AAC audio in 3GP", SMALL),
        ("vp9-opus.webm", "VP9 video and Opus audio in WebM", SMALL),
        ("vp8-vorbis.webm", "VP8 video and Vorbis audio in WebM", SMALL),
        ("av1-opus.webm", "AV1 video and Opus audio in WebM", SMALL),
        ("theora-vorbis.ogv", "Theora video and Vorbis audio in Ogg", SMALL),
        ("mjpeg-pcm.avi", "Motion JPEG video and PCM audio in AVI", SMALL),
        ("wmv2-wmav2.wmv", "WMV 8 video and WMA audio in ASF", SMALL),
        ("tone.mp3", "MP3 audio", None),
        ("tone.m4a", "AAC audio in MP4", None),
        ("tone.aac", "AAC audio in ADTS", None),
        ("tone.wav", "PCM audio in WAV", None),
        ("tone.flac", "FLAC audio", None),
        ("tone.ogg", "Vorbis audio in Ogg", None),
        ("tone.opus", "Opus audio in Ogg", None),
        ("tone.aiff", "PCM audio in AIFF", None),
        ("tone.caf", "ALAC audio in CAF", None),
        ("tone.wma", "WMA audio in ASF", None),
        ("tone.ac3", "AC-3 audio", None),
        ("tone.amr", "AMR-NB audio in AMR", None),
    ];

    #[test]
    fn the_header_reader_names_every_clip_the_tests_play() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("media");
        for (file, name, size) in CLIPS {
            let format = match probe::read(&folder.join(file)) {
                Ok(f) => f,
                Err(e) => panic!("{file}: {e}"),
            };
            assert_eq!((format.name().as_str(), format.size), (*name, *size), "{file}");
        }
        let entries = std::fs::read_dir(&folder).expect("the clips' folder");
        for entry in entries {
            let file = entry.expect("an entry of the clips' folder").file_name().to_string_lossy().into_owned();
            let beside = [".ps1", ".srt", ".vtt"].iter().any(|kind| file.ends_with(kind));
            assert!(beside || CLIPS.iter().any(|(c, _, _)| *c == file), "{file} is a clip the reader is not checked against");
        }
    }

    #[test]
    fn the_header_reader_gives_the_edit_lists_of_the_h264_clips() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("media");
        let edits = probe::edits(&folder.join("h264-aac.mp4")).expect("its edit lists");
        let (video, audio) = (edits.video.expect("a picture's edit list"), edits.audio.expect("a sound's edit list"));
        assert!((video - 2.0 / 15.0).abs() < 1e-6, "two frames of B-frame delay at 15 a second, not {video}");
        assert!((audio - 1024.0 / 48000.0).abs() < 1e-6, "the AAC encoder's priming, not {audio}");
        assert_eq!(probe::edits(&folder.join("h264-aac.mkv")).expect("no edit lists"), probe::Edits::default());
    }
}
