//! A window's watch on files. A change to one of them comes among the
//! window's events as [`crate::WindowEvent::FileChanged`], sent into the
//! window's own ring or channel, so a wait for the window's next event wakes
//! for it as it does for a click.
//!
//! The system's change notifications drive the watch: notify's watcher on
//! each file's folder, which reports the file made, written, renamed or
//! removed, and on FreeBSD on the file too, since kqueue reports a write
//! only to a watch on the file itself. On Windows the watch also compares
//! each file's length and last write once a second: NTFS reports no write
//! that a writer leaves in the system's cache until the writer closes the
//! file, so a log a service has open would show nothing new. Given an
//! interval, the watch compares at that pace on every system and takes no
//! notifications, for a file system that gives none, such as a network
//! share. A file has at most one change waiting: the next is sent once the
//! window's reader has taken it.

use crate::GuiError;
use notify::event::{AccessKind, AccessMode, ModifyKind};
use notify::{EventKind, RecursiveMode, Watcher};
use pwrs_gui_proto::trace;
use pwrs_gui_proto::{FromHelper, WindowId, transport};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, SystemTime};

/// How often the watch compares each file beside the system's
/// notifications: once a second on Windows, the pace Get-Content -Wait
/// reads a file at, and never on the other systems, whose notifications
/// report every write.
const SYSTEM_CHECK: Option<Duration> = if cfg!(windows) { Some(Duration::from_secs(1)) } else { None };

/// Where a watch sends the changes it finds: the window's ring, or on the
/// in-process path its channel.
pub(crate) enum Post {
    Ring(transport::Sender),
    #[cfg(all(windows, feature = "in-process"))]
    Channel(mpsc::Sender<crate::Delivery>),
}

impl Post {
    fn send(&self, msg: FromHelper) -> Result<(), String> {
        match self {
            Post::Ring(sender) => sender.send(&msg).map_err(|e| e.to_string()),
            #[cfg(all(windows, feature = "in-process"))]
            Post::Channel(tx) => tx.send(crate::Delivery::Msg(Box::new(msg))).map_err(|e| e.to_string()),
        }
    }
}

/// What a watch's thread is told.
enum Seen {
    /// The system reported a change, or a failure to watch.
    Notified(notify::Result<notify::Event>),
    /// The window's reader has taken the change sent for this file.
    Taken(usize),
    /// The window has let the watch go.
    Stop,
}

/// Passes notify's reports to the watch's thread.
struct Forward(mpsc::Sender<Seen>);

impl notify::EventHandler for Forward {
    fn handle_event(&mut self, event: notify::Result<notify::Event>) {
        match self.0.send(Seen::Notified(event)) {
            Ok(()) => {}
            Err(mpsc::SendError(_)) => trace::line(|| String::from("client: a file notification came after its watch's thread ended")),
        }
    }
}

/// A file a window watches: the full path it was given, and the same path
/// through its folder's real path, which macOS reports changes under (its
/// /var is /private/var); none when the folder's real path cannot be read.
struct Watched {
    given: PathBuf,
    real: Option<PathBuf>,
}

impl Watched {
    fn new(given: PathBuf) -> Watched {
        let real = match (given.parent(), given.file_name()) {
            (Some(folder), Some(name)) => match std::fs::canonicalize(folder) {
                Ok(f) => Some(f.join(name)),
                Err(e) => {
                    trace::line(|| format!("client: {}'s real path cannot be read, so changes are matched by the path given: {e}", folder.display()));
                    None
                }
            },
            _ => None,
        };
        Watched { given, real }
    }

    /// Whether `path`, as a notification names it, is this file: without
    /// regard to case on Windows and macOS, whose file systems mostly
    /// ignore it.
    fn is(&self, path: &Path) -> bool {
        same(&self.given, path) || self.real.as_deref().is_some_and(|r| same(r, path))
    }
}

fn same(a: &Path, b: &Path) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        a.as_os_str().to_string_lossy().to_lowercase() == b.as_os_str().to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// A file's length and last write, none while it is not there.
type Stamp = Option<(u64, SystemTime)>;

fn stamp(path: &Path) -> Result<Stamp, std::io::Error> {
    match std::fs::metadata(path) {
        Ok(m) => Ok(Some((m.len(), m.modified()?))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The files a window watches, and the thread that watches them, which
/// ends when this is dropped.
pub(crate) struct FileWatch {
    paths: Arc<[PathBuf]>,
    /// Whether a change to each file has been sent and not yet taken.
    waiting: Arc<[AtomicBool]>,
    stop: mpsc::Sender<Seen>,
}

impl FileWatch {
    /// Starts watching `paths`, each a full path, for window `window`,
    /// sending what changes through `post`; with `every`, by comparing the
    /// files at that pace rather than through the system's notifications.
    /// Fails when a file's folder cannot be watched.
    pub(crate) fn start(window: WindowId, paths: Vec<PathBuf>, every: Option<Duration>, post: Post) -> Result<FileWatch, GuiError> {
        let files: Vec<Watched> = paths.iter().cloned().map(Watched::new).collect();
        let waiting: Arc<[AtomicBool]> = files.iter().map(|_| AtomicBool::new(false)).collect();
        let (tx, rx) = mpsc::channel();
        let watcher = match every {
            Some(_) => None,
            None => Some(watcher(&files, tx.clone())?),
        };
        let mut seen = Vec::with_capacity(files.len());
        for f in &files {
            seen.push(stamp(&f.given).map_err(|e| GuiError::Watch(format!("{} cannot be read: {e}", f.given.display())))?);
        }
        let count = files.len();
        let run = Run { window, files, waiting: Arc::clone(&waiting), post, watcher, sent: seen.clone(), seen, failing: vec![false; count], every: every.or(SYSTEM_CHECK) };
        std::thread::Builder::new()
            .name(String::from("pwrs-gui-file-watch"))
            .spawn(move || run.run(rx))
            .map_err(|e| GuiError::Watch(format!("the watch's thread could not start: {e}")))?;
        trace::line(|| format!("client: window {window} watches {count} files, compared every {every:?}"));
        Ok(FileWatch { paths: paths.into(), waiting, stop: tx })
    }

    /// The full path of file `file`, by its place among the files watched.
    pub(crate) fn path(&self, file: usize) -> Option<&Path> {
        self.paths.get(file).map(PathBuf::as_path)
    }

    /// Notes that the change sent for file `file` has been taken, so the
    /// next change to it is sent, and tells the watch's thread, which sends
    /// one at once when the file changed while the change waited.
    pub(crate) fn taken(&self, file: usize) {
        let Some(w) = self.waiting.get(file) else {
            return;
        };
        w.store(false, Ordering::Release);
        match self.stop.send(Seen::Taken(file)) {
            Ok(()) => {}
            Err(mpsc::SendError(_)) => trace::line(|| String::from("client: a file change was taken after its watch's thread ended")),
        }
    }
}

impl Drop for FileWatch {
    fn drop(&mut self) {
        match self.stop.send(Seen::Stop) {
            Ok(()) => {}
            Err(mpsc::SendError(_)) => trace::line(|| String::from("client: a file watch's thread had ended before its window let it go")),
        }
    }
}

/// notify's watcher on each file's folder, and on FreeBSD on each file
/// there now.
fn watcher(files: &[Watched], tx: mpsc::Sender<Seen>) -> Result<notify::RecommendedWatcher, GuiError> {
    let mut w = notify::recommended_watcher(Forward(tx)).map_err(|e| GuiError::Watch(format!("the system's file notifications could not start: {e}")))?;
    let mut folders: Vec<&Path> = Vec::new();
    for f in files {
        let folder = f.given.parent().ok_or_else(|| GuiError::Watch(format!("{} names no file in a folder", f.given.display())))?;
        if !folders.contains(&folder) {
            w.watch(folder, RecursiveMode::NonRecursive).map_err(|e| GuiError::Watch(format!("{} cannot be watched: {e}", folder.display())))?;
            folders.push(folder);
        }
        if cfg!(target_os = "freebsd") && f.given.exists() {
            w.watch(&f.given, RecursiveMode::NonRecursive).map_err(|e| GuiError::Watch(format!("{} cannot be watched: {e}", f.given.display())))?;
        }
    }
    Ok(w)
}

/// A watch's thread.
struct Run {
    window: WindowId,
    files: Vec<Watched>,
    waiting: Arc<[AtomicBool]>,
    post: Post,
    watcher: Option<notify::RecommendedWatcher>,
    /// Each file's stamp when last read.
    seen: Vec<Stamp>,
    /// Each file's stamp when its last change was sent, or when the watch
    /// began.
    sent: Vec<Stamp>,
    /// Whether each file could not be read when last compared, so a file
    /// that stays unreadable is reported once.
    failing: Vec<bool>,
    /// How often the files are compared; none to wait on notifications
    /// alone.
    every: Option<Duration>,
}

impl Run {
    /// Watches until the window lets the watch go or its events can no
    /// longer be sent.
    fn run(mut self, rx: mpsc::Receiver<Seen>) {
        loop {
            let seen = match self.every {
                Some(every) => match rx.recv_timeout(every) {
                    Ok(s) => Some(s),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                },
                None => match rx.recv() {
                    Ok(s) => Some(s),
                    Err(mpsc::RecvError) => return,
                },
            };
            let going = match seen {
                Some(Seen::Stop) => return,
                Some(Seen::Notified(Ok(event))) => self.notified(&event),
                Some(Seen::Notified(Err(e))) => self.problem(format!("the system's file notifications failed: {e}")) && self.compare(),
                Some(Seen::Taken(file)) => self.taken(file),
                None => self.compare(),
            };
            if !going {
                return;
            }
        }
    }

    /// Takes a notification: each watched file it names has changed when its
    /// length or last write differs from when it was last read; one that
    /// calls for a rescan may have changed any of them. An open, a read or a
    /// close without a write, which Linux reports too, is passed over: it
    /// changes neither, and a stamp read as it arrives can meet a write half
    /// done, which sets its new last write before its length. False once the
    /// window's events can no longer be sent.
    fn notified(&mut self, event: &notify::Event) -> bool {
        if event.need_rescan() {
            return self.compare();
        }
        if matches!(event.kind, EventKind::Access(kind) if kind != AccessKind::Close(AccessMode::Write)) {
            return true;
        }
        for file in 0..self.files.len() {
            if !event.paths.iter().any(|p| self.files[file].is(p)) {
                continue;
            }
            if cfg!(target_os = "freebsd") && matches!(event.kind, EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(_))) && !self.rewatch(file) {
                return false;
            }
            match stamp(&self.files[file].given) {
                Ok(now) => {
                    self.failing[file] = false;
                    if now == self.seen[file] {
                        continue;
                    }
                    self.seen[file] = now;
                }
                Err(e) => {
                    if !self.unreadable(file, &e) {
                        return false;
                    }
                    continue;
                }
            }
            if !self.changed(file) {
                return false;
            }
        }
        true
    }

    /// Compares each file with its stamp when last read, sending a change
    /// for each that differs. False once the window's events can no longer
    /// be sent.
    fn compare(&mut self) -> bool {
        for file in 0..self.files.len() {
            match stamp(&self.files[file].given) {
                Ok(now) => {
                    self.failing[file] = false;
                    if now != self.seen[file] {
                        self.seen[file] = now;
                        if !self.changed(file) {
                            return false;
                        }
                    }
                }
                Err(e) => {
                    if !self.unreadable(file, &e) {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// Takes word that the window's reader has taken the change sent for
    /// `file`: when the file has changed since that change was sent, as when
    /// a write came while it waited, sends the file as it is now, since no
    /// notification may follow. False once the window's events can no longer
    /// be sent.
    fn taken(&mut self, file: usize) -> bool {
        match stamp(&self.files[file].given) {
            Ok(now) => {
                self.failing[file] = false;
                if now == self.sent[file] {
                    return true;
                }
                self.seen[file] = now;
                self.changed(file)
            }
            Err(e) => self.unreadable(file, &e),
        }
    }

    /// Reports once that `file` cannot be read, until it can be again.
    /// False once the window's events can no longer be sent.
    fn unreadable(&mut self, file: usize, e: &std::io::Error) -> bool {
        if std::mem::replace(&mut self.failing[file], true) {
            return true;
        }
        self.problem(format!("{} cannot be read: {e}", self.files[file].given.display()))
    }

    /// On FreeBSD, watches the file again once it has been made or
    /// renamed, since kqueue reports a write only to a watch on the file and
    /// the file there now is not the one watched before. False once the
    /// window's events can no longer be sent.
    fn rewatch(&mut self, file: usize) -> bool {
        let Some(w) = &mut self.watcher else {
            return true;
        };
        let path = self.files[file].given.clone();
        match w.unwatch(&path) {
            Ok(()) => {}
            Err(e) => trace::line(|| format!("client: {} was not watched before it was made or renamed: {e}", path.display())),
        }
        if !path.exists() {
            return true;
        }
        match w.watch(&path, RecursiveMode::NonRecursive) {
            Ok(()) => true,
            Err(e) => self.problem(format!("{} cannot be watched: {e}", path.display())),
        }
    }

    /// Sends a change to `file`, unless one is waiting to be taken, and
    /// notes the stamp it sent. False once the window's events can no longer
    /// be sent.
    fn changed(&mut self, file: usize) -> bool {
        if self.waiting[file].swap(true, Ordering::AcqRel) {
            return true;
        }
        self.sent[file] = self.seen[file];
        let length = self.seen[file].map(|(len, _)| len);
        self.send(FromHelper::FileChanged { window: self.window, file: file as u32, length })
    }

    /// Tells the window of something its watch could not do. False once
    /// the window's events can no longer be sent.
    fn problem(&self, text: String) -> bool {
        self.send(FromHelper::Error { window: Some(self.window), context: String::from("watching files"), text })
    }

    fn send(&self, msg: FromHelper) -> bool {
        match self.post.send(msg) {
            Ok(()) => true,
            Err(why) => {
                trace::line(|| format!("client: window {}'s file watch ends, since its events cannot be sent: {why}", self.window));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(all(windows, feature = "in-process"))]
    use std::io::Write;

    /// A folder of the test's own under the temporary folder, removed with
    /// the files the test made in it.
    #[cfg(all(windows, feature = "in-process"))]
    struct Folder(PathBuf);

    #[cfg(all(windows, feature = "in-process"))]
    impl Folder {
        fn new(name: &str) -> Folder {
            let path = std::env::temp_dir().join(format!("pwrs-gui-watch-{name}-{}", std::process::id()));
            std::fs::create_dir(&path).expect("a folder of the test's own");
            Folder(path)
        }
    }

    #[cfg(all(windows, feature = "in-process"))]
    impl Drop for Folder {
        fn drop(&mut self) {
            for entry in std::fs::read_dir(&self.0).expect("the test's folder is read") {
                let entry = entry.expect("an entry of the test's folder is read");
                std::fs::remove_file(entry.path()).expect("the test's file goes");
            }
            std::fs::remove_dir(&self.0).expect("the test's folder goes");
        }
    }

    #[cfg(all(windows, feature = "in-process"))]
    fn post() -> (Post, mpsc::Receiver<crate::Delivery>) {
        let (tx, rx) = mpsc::channel();
        (Post::Channel(tx), rx)
    }

    #[cfg(all(windows, feature = "in-process"))]
    fn next(rx: &mpsc::Receiver<crate::Delivery>) -> FromHelper {
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(crate::Delivery::Msg(m)) => *m,
            Ok(crate::Delivery::Wake) => panic!("a wake came where a file change was awaited"),
            Err(e) => panic!("no file change came: {e}"),
        }
    }

    /// A line from a writer that has the file open throughout and leaves
    /// its lines in the system's cache, as most loggers do, reaches the
    /// window, through the system's notifications or, on Windows, its
    /// once-a-second comparison; one change waits at a time, the next sent
    /// once the first is taken.
    #[cfg(all(windows, feature = "in-process"))]
    #[test]
    fn a_line_from_a_writer_that_keeps_the_file_open_is_seen() {
        let folder = Folder::new("open");
        let log = folder.0.join("grow.log");
        std::fs::write(&log, "start\n").expect("the log is written");
        let (post, rx) = post();
        let watch = FileWatch::start(7, vec![log.clone()], None, post).expect("the watch starts");
        let mut writer = std::fs::OpenOptions::new().append(true).open(&log).expect("the writer opens the log");
        writer.write_all(b"line 1\n").expect("a line is written");
        writer.flush().expect("the line reaches the system");
        match next(&rx) {
            FromHelper::FileChanged { window: 7, file: 0, length: Some(13) } => {}
            other => panic!("expected the log's change at 13 bytes, got {other:?}"),
        }
        watch.taken(0);
        writer.write_all(b"line 2\n").expect("a second line is written");
        writer.flush().expect("the second line reaches the system");
        match next(&rx) {
            FromHelper::FileChanged { window: 7, file: 0, length: Some(20) } => {}
            other => panic!("expected the log's change at 20 bytes, got {other:?}"),
        }
    }

    /// With an interval the files are compared at that pace, a file made
    /// after the watch began included.
    #[cfg(all(windows, feature = "in-process"))]
    #[test]
    fn an_interval_compares_the_files_and_sees_one_made_later() {
        let folder = Folder::new("later");
        let log = folder.0.join("later.log");
        let (post, rx) = post();
        let _watch = FileWatch::start(8, vec![log.clone()], Some(Duration::from_millis(50)), post).expect("the watch starts");
        std::fs::write(&log, "made\n").expect("the log is made");
        match next(&rx) {
            FromHelper::FileChanged { window: 8, file: 0, length: Some(5) } => {}
            other => panic!("expected the new log at 5 bytes, got {other:?}"),
        }
    }

    /// A notification of an open or a read, which Linux sends as
    /// Add-Content opens a log to read its encoding before it appends,
    /// raises nothing; one after a write raises the new length.
    #[cfg(all(windows, feature = "in-process"))]
    #[test]
    fn a_notification_that_leaves_the_length_and_last_write_raises_nothing() {
        let folder = Folder::new("read");
        let log = folder.0.join("read.log");
        std::fs::write(&log, "start\n").expect("the log is written");
        let (post, rx) = post();
        let first = stamp(&log).expect("the log's stamp is read");
        let mut run = Run { window: 9, files: vec![Watched::new(log.clone())], waiting: vec![AtomicBool::new(false)].into(), post, watcher: None, seen: vec![first], sent: vec![first], failing: vec![false], every: None };
        let opened = notify::Event::new(EventKind::Access(notify::event::AccessKind::Open(notify::event::AccessMode::Any))).add_path(log.clone());
        assert!(run.notified(&opened), "the window's events can be sent");
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)), "an open changes nothing");
        let mut writer = std::fs::OpenOptions::new().append(true).open(&log).expect("the writer opens the log");
        writer.write_all(b"line\n").expect("a line is written");
        writer.flush().expect("the line reaches the system");
        let written = notify::Event::new(EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any))).add_path(log.clone());
        assert!(run.notified(&written), "the window's events can be sent");
        match next(&rx) {
            FromHelper::FileChanged { window: 9, file: 0, length: Some(11) } => {}
            other => panic!("expected the log's change at 11 bytes, got {other:?}"),
        }
    }

    /// An open, a read and a close without a write raise nothing even when
    /// the file's stamp has moved since it was last read, as one read while
    /// a write runs can show the write's last write beside the length from
    /// before it; a close after a write raises the file as it is.
    #[cfg(all(windows, feature = "in-process"))]
    #[test]
    fn an_open_a_read_and_a_close_without_a_write_raise_nothing_though_the_stamp_moved() {
        let folder = Folder::new("access");
        let log = folder.0.join("access.log");
        std::fs::write(&log, "start\n").expect("the log is written");
        let (post, rx) = post();
        let earlier = Some((6, SystemTime::UNIX_EPOCH));
        let mut run = Run { window: 11, files: vec![Watched::new(log.clone())], waiting: vec![AtomicBool::new(false)].into(), post, watcher: None, seen: vec![earlier], sent: vec![earlier], failing: vec![false], every: None };
        for kind in [AccessKind::Open(AccessMode::Any), AccessKind::Read, AccessKind::Close(AccessMode::Read)] {
            let event = notify::Event::new(EventKind::Access(kind)).add_path(log.clone());
            assert!(run.notified(&event), "the window's events can be sent");
            assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)), "{kind:?} raises nothing");
        }
        let closed = notify::Event::new(EventKind::Access(AccessKind::Close(AccessMode::Write))).add_path(log.clone());
        assert!(run.notified(&closed), "the window's events can be sent");
        match next(&rx) {
            FromHelper::FileChanged { window: 11, file: 0, length: Some(6) } => {}
            other => panic!("expected the log's change at 6 bytes, got {other:?}"),
        }
    }

    /// A write that comes while the change before it waits to be taken is
    /// sent once that change is taken, though no notification follows it.
    #[cfg(all(windows, feature = "in-process"))]
    #[test]
    fn a_write_while_a_change_waits_is_sent_once_that_change_is_taken() {
        let folder = Folder::new("waits");
        let log = folder.0.join("waits.log");
        std::fs::write(&log, "start\n").expect("the log is written");
        let (post, rx) = post();
        let first = stamp(&log).expect("the log's stamp is read");
        let mut run = Run { window: 10, files: vec![Watched::new(log.clone())], waiting: vec![AtomicBool::new(false)].into(), post, watcher: None, seen: vec![first], sent: vec![first], failing: vec![false], every: None };
        let written = notify::Event::new(EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any))).add_path(log.clone());
        let mut writer = std::fs::OpenOptions::new().append(true).open(&log).expect("the writer opens the log");
        writer.write_all(b"line 1\n").expect("a line is written");
        writer.flush().expect("the line reaches the system");
        assert!(run.notified(&written), "the window's events can be sent");
        match next(&rx) {
            FromHelper::FileChanged { window: 10, file: 0, length: Some(13) } => {}
            other => panic!("expected the log's change at 13 bytes, got {other:?}"),
        }
        writer.write_all(b"line 2\n").expect("a second line is written");
        writer.flush().expect("the second line reaches the system");
        assert!(run.notified(&written), "the window's events can be sent");
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)), "the second change waits while the first is not taken");
        run.waiting[0].store(false, Ordering::Release);
        assert!(run.taken(0), "the window's events can be sent");
        match next(&rx) {
            FromHelper::FileChanged { window: 10, file: 0, length: Some(20) } => {}
            other => panic!("expected the log's change at 20 bytes, got {other:?}"),
        }
    }

    #[test]
    fn a_watch_on_a_folder_that_is_not_there_is_refused() {
        let missing = std::env::temp_dir().join(format!("pwrs-gui-watch-missing-{}", std::process::id())).join("app.log");
        let (tx, _rx) = mpsc::channel::<Seen>();
        let refused = watcher(&[Watched::new(missing)], tx).expect_err("a folder that is not there cannot be watched");
        assert!(matches!(&refused, GuiError::Watch(why) if why.contains("cannot be watched")), "{refused:?}");
    }

    #[test]
    fn a_path_names_a_watched_file_as_the_system_compares_names() {
        let base = std::env::temp_dir().join("Logs");
        let watched = Watched { given: base.join("App.log"), real: None };
        assert!(watched.is(&base.join("App.log")));
        assert_eq!(watched.is(&base.join("app.log")), cfg!(any(windows, target_os = "macos")), "case counts on Linux and FreeBSD alone");
        assert!(!watched.is(&base.join("App.log.1")));
    }
}
