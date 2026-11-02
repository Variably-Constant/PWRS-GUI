//! A page's channels to its backend: `New-GuiChannel`, which makes the two
//! SubEtha rings of one, and what `Show-GuiWindow -Channel` takes, a
//! channel New-GuiChannel made or rings a script or backend made.

use crate::options::Env;
use pwrs::prelude::*;
use pwrs_gui_client::Session;
use pwrs_gui_client::proto::channel::{ChannelEnds, ChannelSpec, DEFAULT_ROOM, Window as Room, make_ring, ring_slots};
use pwrs_gui_client::proto::source::{RingKind, RingSource};
use pwrs_gui_client::proto::{Value, WindowId};
use arc_swap::ArcSwap;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A page's channel to its backend that New-GuiChannel made: two SubEtha
/// rings, ToPage, which the backend writes, and FromPage, which it reads,
/// in a folder of the channel's own that only this user reaches.
#[psclass(name = "PwrsGui.Channel")]
#[derive(Clone, Default)]
pub struct GuiChannel {
    /// The name the page opens it by, with window.pwrs.channel(name).
    pub name: String,
    /// The ring the backend writes and the page reads: its path, which the
    /// names of SubEtha's files for it begin with and which the backend
    /// opens it by, with Open-SubEthaRing -Path and the capacity.
    pub to_page: String,
    /// The ring the page writes and the backend reads.
    pub from_page: String,
    /// Each ring's slots, its capacity, one producer and one consumer.
    pub capacity: u64,
    /// Its messages are text, which the page sends and is given as strings.
    pub text: bool,
    /// It carries plain frames rather than messages.
    pub stream: bool,
    /// The most frames in flight each way.
    pub max_frames: u64,
    /// The most bytes in flight each way.
    pub max_bytes: u64,
    /// The process whose exit ends the channel; null when none.
    pub process: PsObject,
}

/// A count a channel takes, above 0.
fn count(given: Option<i64>, default: usize, what: &str) -> PsResult<usize> {
    let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadChannel", text).terminating();
    match given {
        None => Ok(default),
        Some(n) if n > 0 => usize::try_from(n).map_err(|e| refuse(format!("{what} is {n}, more than this machine counts: {e}"))),
        Some(n) => Err(refuse(format!("{what} is {n}; it is a count above 0"))),
    }
}

/// The process a channel is tied to, checked to be one.
fn process_of(given: Option<&PsObject>) -> PsResult<PsObject> {
    let Some(p) = given.filter(|p| !p.is_null()) else {
        return Ok(PsObject::null());
    };
    let kind = p.type_name()?;
    if kind != "System.Diagnostics.Process" {
        let text = format!("-Process takes a System.Diagnostics.Process, as Start-Process -PassThru or Get-Process writes it, not a {kind}");
        return Err(PsError::new(ErrorCategory::InvalidArgument, "GuiBadChannel", text).terminating());
    }
    Ok(p.clone())
}

/// Makes a page's channel to its backend: two SubEtha rings in a folder of
/// the channel's own, named at random, that only this user reaches. The
/// backend opens ToPage, which it writes, and FromPage, which it reads,
/// with Open-SubEthaRing -Path and the capacity, and Show-GuiWindow
/// -Channel gives the channel to a page, which opens it with
/// window.pwrs.channel(name).
///
/// A message channel carries messages of any size, each sent as parts of
/// one ring frame each, at most 8,192 bytes, whose first byte says whole
/// (0), more (1) or last (2). With -Text its messages are text, as UTF-8.
/// -Stream carries plain frames, for a protocol that frames its own
/// messages, as the language server protocol does. At most -MaxFrames
/// frames and -MaxBytes bytes are in flight each way, 64 and 1 MiB unless
/// given. With -Process the channel ends when that process exits, and the
/// page's onclose runs. Show-GuiWindow removes the rings' folder once the
/// app it gave the channel to has ended.
///
/// # Examples
/// $ch = New-GuiChannel -Name Lsp -Stream
/// $ch = New-GuiChannel -Name Status -Text -MaxFrames 16
#[cmdlet(verb = "New", noun = "GuiChannel", output = ["PwrsGui.Channel"])]
#[derive(Default)]
pub struct NewGuiChannel {
    /// The name the page opens the channel by.
    #[param(mandatory, position = 0)]
    pub name: String,
    /// The channel's messages are text, which the page is given as strings.
    #[param]
    pub text: bool,
    /// The channel carries plain frames of at most 8,192 bytes rather than
    /// messages, for a protocol that frames its own.
    #[param]
    pub stream: bool,
    /// The most frames in flight each way; 64 when not given.
    #[param]
    pub max_frames: Option<i64>,
    /// The most bytes in flight each way; 1 MiB when not given.
    #[param]
    pub max_bytes: Option<i64>,
    /// The process whose exit ends the channel, as Start-Process -PassThru
    /// writes it.
    #[param]
    pub process: Option<PsObject>,
}

impl Cmdlet for NewGuiChannel {
    fn process(&mut self, ps: &Pipeline<'_>) -> PsResult<()> {
        let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadChannel", text).terminating();
        if self.name.is_empty() {
            return Err(refuse(String::from("a channel's name is empty")));
        }
        if self.text && self.stream {
            return Err(refuse(String::from("give -Text or -Stream, not both: a stream carries plain frames, and only messages are text")));
        }
        let room = Room { frames: count(self.max_frames, DEFAULT_ROOM.frames, "-MaxFrames")?, bytes: count(self.max_bytes, DEFAULT_ROOM.bytes, "-MaxBytes")? };
        let process = process_of(self.process.as_ref())?;
        let slots = ring_slots(room.frames).map_err(refuse)?;
        let folder = channel_folder(ps)?;
        let (to_page, from_page) = (folder.join("to-page"), folder.join("from-page"));
        let made = make_ring(&to_page, slots).and_then(|()| make_ring(&from_page, slots));
        if let Err(why) = made {
            remove_folder(&folder);
            return Err(PsError::new(ErrorCategory::ResourceUnavailable, "GuiNoChannel", why).terminating());
        }
        let slots = u64::try_from(slots).map_err(|e| refuse(format!("a ring of {slots} slots: {e}")))?;
        let wide = |n: usize| u64::try_from(n).map_err(|e| refuse(format!("{n} is too large: {e}")));
        ps.write(GuiChannel {
            name: self.name.clone(),
            to_page: to_page.display().to_string(),
            from_page: from_page.display().to_string(),
            capacity: slots,
            text: self.text,
            stream: self.stream,
            max_frames: wide(room.frames)?,
            max_bytes: wide(room.bytes)?,
            process,
        })
    }
}

/// A new folder for a channel's rings, under the temporary folder's
/// PWRS-GUI, named as a session's rings are, that only this user reaches:
/// on Linux, FreeBSD and macOS its mode is 0700, and on Windows the user's
/// temporary folder is the user's alone.
fn channel_folder(ps: &Pipeline<'_>) -> PsResult<PathBuf> {
    let failed = |text: String| PsError::new(ErrorCategory::ResourceUnavailable, "GuiNoChannel", text).terminating();
    let temp = crate::session::temp_dir(&Env::Session(ps)).map_err(failed)?;
    let id = pwrs_gui_client::proto::transport::session_id().map_err(|e| failed(e.to_string()))?;
    let parent = temp.join("PWRS-GUI").join("channels");
    std::fs::create_dir_all(&parent).map_err(|e| failed(format!("{} cannot be made: {e}", parent.display())))?;
    let folder = parent.join(id);
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut owner_only = std::fs::DirBuilder::new();
        owner_only.mode(0o700);
        owner_only
    };
    #[cfg(not(unix))]
    let builder = std::fs::DirBuilder::new();
    builder.create(&folder).map_err(|e| failed(format!("{} cannot be made: {e}", folder.display())))?;
    Ok(folder)
}

/// Removes a channel's folder and its rings, saying so when they cannot
/// go, as when the backend still has them open on Windows.
pub(crate) fn remove_folder(folder: &Path) {
    if let Err(e) = std::fs::remove_dir_all(folder) {
        eprintln!("PWRS-GUI: the rings of a channel at {} could not be removed: {e}", folder.display());
    }
}

/// A channel `Show-GuiWindow -Channel` gave a page: its name and settings,
/// the process whose exit ends it, and for one New-GuiChannel made, the
/// folder its rings are in, which goes once the app has ended.
#[derive(Clone)]
pub(crate) struct Given {
    pub(crate) name: String,
    pub(crate) spec: ChannelSpec,
    pub(crate) process: Option<PsObject>,
    pub(crate) folder: Option<PathBuf>,
}

/// The keys a hashtable of `-Channel` gives each channel.
const KEYS: [&str; 7] = ["ToPage", "FromPage", "Text", "Stream", "MaxFrames", "MaxBytes", "Process"];

/// The channels `-Channel` gives: each a PwrsGui.Channel New-GuiChannel
/// wrote, or a hashtable from a channel's name to a hashtable of its
/// ToPage and FromPage SubEtha.Ring objects, with Text, Stream, MaxFrames,
/// MaxBytes and Process as New-GuiChannel takes them. A name given twice is
/// refused.
pub(crate) fn given(values: &[PsObject]) -> PsResult<Vec<Given>> {
    let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadChannel", text).terminating();
    let mut out: Vec<Given> = Vec::new();
    for value in values {
        let kind = value.type_name()?;
        let found = if kind == "PwrsGui.Channel" { vec![made(value)?] } else { tables(value).map_err(|why| refuse(format!("-Channel: {why}")))? };
        for g in found {
            if out.iter().any(|o| o.name.eq_ignore_ascii_case(&g.name)) {
                return Err(refuse(format!("-Channel names {} twice; a page opens one channel by each name", g.name)));
            }
            out.push(g);
        }
    }
    Ok(out)
}

/// A PwrsGui.Channel New-GuiChannel wrote.
fn made(value: &PsObject) -> PsResult<Given> {
    let text = |key: &str| String::from_ps(&pwrs::object::property(value, key)?);
    let number = |key: &str| u64::from_ps(&pwrs::object::property(value, key)?);
    let flag = |key: &str| bool::from_ps(&pwrs::object::property(value, key)?);
    let refuse = |text: String| PsError::new(ErrorCategory::InvalidArgument, "GuiBadChannel", text).terminating();
    let capacity = i64::try_from(number("Capacity")?).map_err(|e| refuse(format!("a channel's capacity: {e}")))?;
    let ring = |path: String| RingSource { kind: RingKind::Ring, path, capacity, producers: 1, consumers: 1 };
    let (to_page, from_page) = (text("ToPage")?, text("FromPage")?);
    let folder = Path::new(&to_page).parent().map(Path::to_path_buf);
    let narrow = |n: u64| usize::try_from(n).map_err(|e| refuse(format!("{n} is too large: {e}")));
    let spec = ChannelSpec {
        ends: ChannelEnds { to_page: ring(to_page), from_page: ring(from_page) },
        text: flag("Text")?,
        stream: flag("Stream")?,
        room: Room { frames: narrow(number("MaxFrames")?)?, bytes: narrow(number("MaxBytes")?)? },
    };
    let process = pwrs::object::property(value, "Process")?;
    Ok(Given { name: text("Name")?, spec, process: if process.is_null() { None } else { Some(process) }, folder })
}

/// The channels a hashtable of names to hashtables of rings gives.
fn tables(value: &PsObject) -> Result<Vec<Given>, String> {
    let message = |e: PsError| e.message;
    let read = PsReadOnlyTable::over(value).map_err(|e| format!("a channel is a PwrsGui.Channel or a hashtable from its name to its rings: {}", e.message))?;
    let mut out = Vec::new();
    for name in read.keys().map_err(message)? {
        if name.is_empty() {
            return Err(String::from("a channel's name is empty"));
        }
        let entry = read.get(&name).map_err(message)?;
        let table = PsReadOnlyTable::over(&entry).map_err(|e| format!("channel {name} is a hashtable of its ToPage and FromPage rings: {}", e.message))?;
        for key in table.keys().map_err(message)? {
            if !KEYS.iter().any(|k| k.eq_ignore_ascii_case(&key)) {
                return Err(format!("channel {name} takes ToPage, FromPage, Text, Stream, MaxFrames, MaxBytes and Process, not {key}"));
            }
        }
        let ring = |key: &str| -> Result<Value, String> {
            let given = table.get(key).map_err(message)?;
            if given.is_null() {
                return Err(format!("channel {name} gives no {key} ring"));
            }
            crate::widgets::ring_of(&given).map_err(|why| format!("channel {name}'s {key}: {why}"))
        };
        let flag = |key: &str| -> Result<bool, String> {
            let given = table.get(key).map_err(message)?;
            if given.is_null() { Ok(false) } else { bool::from_ps(&given).map_err(|e| format!("channel {name}'s {key}: {}", e.message)) }
        };
        let size = |key: &str| -> Result<Option<i64>, String> {
            let given = table.get(key).map_err(message)?;
            if given.is_null() { Ok(None) } else { i64::from_ps(&given).map(Some).map_err(|e| format!("channel {name}'s {key}: {}", e.message)) }
        };
        let mut spec = Value::Map(BTreeMap::from([(String::from("toPage"), ring("ToPage")?), (String::from("fromPage"), ring("FromPage")?)]));
        if let Value::Map(m) = &mut spec {
            m.insert(String::from("text"), Value::Bool(flag("Text")?));
            m.insert(String::from("stream"), Value::Bool(flag("Stream")?));
            for (key, wire, default) in [("MaxFrames", "maxFrames", DEFAULT_ROOM.frames), ("MaxBytes", "maxBytes", DEFAULT_ROOM.bytes)] {
                let n = match size(key)? {
                    Some(n) => n,
                    None => i64::try_from(default).map_err(|e| format!("{default}: {e}"))?,
                };
                m.insert(String::from(wire), Value::Int(n));
            }
        }
        let spec = ChannelSpec::from_value(&spec).map_err(|why| format!("channel {name}: {why}"))?;
        let process = process_of(Some(&table.get("Process").map_err(message)?)).map_err(message)?;
        out.push(Given { name, spec, process: if process.is_null() { None } else { Some(process) }, folder: None });
    }
    Ok(out)
}

/// The node property the page's channels go in, by name.
pub(crate) fn property(given: &[Given]) -> Value {
    Value::Map(given.iter().map(|g| (g.name.clone(), g.spec.value())).collect())
}

/// Waits, on a thread of its own, for the exit of each process a channel
/// of the app is tied to, then ends that channel in each of the app's
/// windows open then, `windows`.
pub(crate) fn watch_processes(session: &Session, given: &[Given], windows: &Arc<ArcSwap<BTreeSet<WindowId>>>) -> PsResult<()> {
    for g in given {
        let Some(process) = g.process.clone() else {
            continue;
        };
        let (session, name, windows) = (session.clone(), g.name.clone(), Arc::clone(windows));
        std::thread::Builder::new()
            .name(format!("pwrs-gui-channel-{name}"))
            .spawn(move || {
                let attached = pwrs::attach_current_thread();
                let waited = process.call("WaitForExit", &[]);
                drop(attached);
                if let Err(e) = waited {
                    eprintln!("PWRS-GUI: the exit of channel {name}'s process cannot be waited for: {}", e.message);
                    return;
                }
                for window in windows.load().iter() {
                    if let Err(e) = session.close_channel(*window, name.clone()) {
                        eprintln!("PWRS-GUI: channel {name} of window {window} could not be ended as its process exited: {e}");
                    }
                }
            })
            .map_err(|e| PsError::new(ErrorCategory::ResourceUnavailable, "GuiThread", format!("the watch of channel {}'s process cannot start: {e}", g.name)).terminating())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_takes_a_count_above_0_or_its_default() {
        assert_eq!(count(None, 64, "-MaxFrames").expect("the default"), 64);
        assert_eq!(count(Some(8), 64, "-MaxFrames").expect("a count"), 8);
        let refused = count(Some(0), 64, "-MaxFrames");
        assert!(matches!(refused, Err(ref e) if e.message.contains("above 0")), "{:?}", refused.map_err(|e| e.message));
    }
}
