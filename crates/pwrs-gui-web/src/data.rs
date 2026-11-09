//! Where a renderer's webviews store their data. [`Data::Shared`] gives
//! every process of the user one folder, and on Windows one WebView2
//! browser process for all of them. [`Data::PerProcess`] gives each process
//! a folder of its own, on macOS a data store, named by the process's id and
//! start time, so a later start can tell those whose process has ended and
//! remove them.

use std::path::PathBuf;

/// Where the webviews store their data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Data {
    /// One folder for every process: the one named, or on Windows
    /// `<LocalAppData>/PWRS-GUI/WebView2`, elsewhere the engine's own.
    Shared(Option<PathBuf>),
    /// A folder of this process's own; on macOS 14 and later a data store.
    PerProcess,
}

impl Default for Data {
    fn default() -> Data {
        Data::Shared(None)
    }
}

/// What a run's webviews take.
#[derive(Debug, Default)]
pub(crate) struct Prepared {
    /// The folder the webviews store their data in; `None` for the engine's
    /// own.
    pub folder: Option<PathBuf>,
    /// The macOS data store; `None` for WebKit's default store.
    #[cfg(target_os = "macos")]
    pub store: Option<[u8; 16]>,
    /// What the person should hear: folders of ended processes that could
    /// not be removed, or a setting this platform cannot honor.
    pub notes: Vec<String>,
}

/// `<LocalAppData>/PWRS-GUI/WebView2` on Windows; elsewhere `None`, the
/// engine's own default.
pub fn default_shared_folder() -> Option<PathBuf> {
    if cfg!(windows) { std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("PWRS-GUI").join("WebView2")) } else { None }
}

/// The versions of the WebView2 Fixed Version runtimes a start finds: the
/// one the renderer runs on, `None` for the evergreen runtime, and those of
/// every version of the add-on installed, `None` when one of them could not
/// be read, so no runtime's data counts as unused.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Versions {
    pub fixed: Option<String>,
    pub installed: Option<Vec<String>>,
}

/// The shared folder of the Fixed Version runtime `version`:
/// `<LocalAppData>/PWRS-GUI/WebView2-<version>`, so its browser process
/// never meets the evergreen runtime's or another version's, and no runtime
/// meets a profile a newer one wrote.
#[cfg(windows)]
fn fixed_shared_folder(version: &str) -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("PWRS-GUI").join(format!("WebView2-{version}")))
}

/// The folder and notes for `data`, on the runtimes `versions` names. Every
/// start also removes the per-process folders of processes that have
/// ended, whichever mode it runs in, so folders left from an earlier choice
/// go too, and on Windows the shared folders of Fixed Version runtimes that
/// no installed add-on has, which no browser process is using.
#[cfg(not(target_os = "macos"))]
#[cfg_attr(not(windows), expect(unused_variables, unused_mut, reason = "a Fixed Version runtime is Windows' alone"))]
pub(crate) fn prepare(data: &Data, versions: &Versions) -> Result<Prepared, String> {
    let place = Owner::current().and_then(|owner| Ok((owner, per_process_root()?)));
    let mut notes = match &place {
        Ok((owner, root)) => sweep(root, *owner, started_at),
        Err(why) if *data != Data::PerProcess => vec![format!("the web data folders of ended processes were not looked for: {why}")],
        Err(why) => return Err(why.clone()),
    };
    #[cfg(windows)]
    if let (Some(root), Some(installed)) = (std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("PWRS-GUI")), &versions.installed) {
        let keep: Vec<String> = installed.iter().chain(versions.fixed.iter()).cloned().collect();
        notes.extend(sweep_versions(&root, &keep));
    }
    let folder = match (data, place) {
        (Data::Shared(Some(f)), _) => Some(f.clone()),
        #[cfg(windows)]
        (Data::Shared(None), _) if versions.fixed.is_some() => versions.fixed.as_deref().and_then(fixed_shared_folder),
        (Data::Shared(None), _) => default_shared_folder(),
        (Data::PerProcess, Ok((owner, root))) => {
            let folder = root.join(owner.name());
            std::fs::create_dir_all(&folder).map_err(|e| format!("this process's web data folder {} cannot be created: {e}", folder.display()))?;
            Some(folder)
        }
        (Data::PerProcess, Err(why)) => return Err(why),
    };
    Ok(Prepared { folder, notes })
}

/// The store and notes for `data`. WebKit on macOS takes a data store, not
/// a folder; the stores of ended processes are removed once the event loop
/// runs ([`crate::sweep_stores`]).
#[cfg(target_os = "macos")]
pub(crate) fn prepare(data: &Data, _versions: &Versions) -> Result<Prepared, String> {
    match data {
        Data::Shared(None) => Ok(Prepared::default()),
        Data::Shared(Some(f)) => {
            let note = format!("WebKit on macOS puts its data in a store of its own, so the data folder {} is not used", f.display());
            Ok(Prepared { folder: None, store: None, notes: vec![note] })
        }
        Data::PerProcess => {
            if !stores_exist() {
                let note = String::from("a data store of each process's own needs macOS 14 or later, so this process's pages use WebKit's default store");
                return Ok(Prepared { folder: None, store: None, notes: vec![note] });
            }
            Ok(Prepared { folder: None, store: Some(Owner::current()?.store()), notes: Vec::new() })
        }
    }
}

/// Whether WebKit has separate data stores here, which it does from
/// macOS 14.
#[cfg(target_os = "macos")]
pub(crate) fn stores_exist() -> bool {
    use objc2_foundation::{NSOperatingSystemVersion, NSProcessInfo};
    NSProcessInfo::processInfo().isOperatingSystemAtLeastVersion(NSOperatingSystemVersion { majorVersion: 14, minorVersion: 0, patchVersion: 0 })
}

/// The folder per-process data folders are made in.
#[cfg(not(target_os = "macos"))]
fn per_process_root() -> Result<PathBuf, String> {
    if cfg!(windows) {
        let local = std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set, so there is no folder for this process's web data")?;
        Ok(PathBuf::from(local).join("PWRS-GUI").join("WebView2-PerProcess"))
    } else {
        let data = match (std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME")) {
            (Some(d), _) if !d.is_empty() => PathBuf::from(d),
            (_, Some(h)) => PathBuf::from(h).join(".local").join("share"),
            (_, None) => return Err(String::from("neither XDG_DATA_HOME nor HOME is set, so there is no folder for this process's web data")),
        };
        Ok(data.join("PWRS-GUI").join("WebKitGTK-PerProcess"))
    }
}

/// A process by its id and start time; the pair never names two processes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Owner {
    pid: u32,
    started: u64,
}

impl Owner {
    pub(crate) fn current() -> Result<Owner, String> {
        let pid = std::process::id();
        match started_at(pid) {
            Started::Running(started) => Ok(Owner { pid, started }),
            Started::Gone => Err(String::from("this process's start time reads as a process that has ended")),
            Started::Unreadable(why) => Err(format!("this process's start time cannot be read: {why}")),
        }
    }

    #[cfg(any(not(target_os = "macos"), test))]
    fn name(&self) -> String {
        format!("{}-{}", self.pid, self.started)
    }

    #[cfg(any(not(target_os = "macos"), test))]
    fn parse(name: &str) -> Option<Owner> {
        let (pid, started) = name.split_once('-')?;
        Some(Owner { pid: pid.parse().ok()?, started: started.parse().ok()? })
    }

    /// The macOS data store of this owner: `PWRS`, the id and the start
    /// time, little-endian.
    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn store(&self) -> [u8; 16] {
        let mut id = [0u8; 16];
        id[..4].copy_from_slice(b"PWRS");
        id[4..8].copy_from_slice(&self.pid.to_le_bytes());
        id[8..].copy_from_slice(&self.started.to_le_bytes());
        id
    }

    /// The owner of a data store this module made, or `None` for another's.
    #[cfg(any(target_os = "macos", test))]
    pub(crate) fn of_store(id: &[u8; 16]) -> Option<Owner> {
        let (tag, rest) = id.split_at(4);
        let (pid, started) = rest.split_at(4);
        if tag != b"PWRS" {
            return None;
        }
        Some(Owner { pid: u32::from_le_bytes(pid.try_into().ok()?), started: u64::from_le_bytes(started.try_into().ok()?) })
    }

    /// Whether this owner has ended, as `probe` reads its id now: no
    /// process has it, or another process does. A process that cannot be
    /// read counts as running.
    pub(crate) fn has_ended(&self, probe: impl Fn(u32) -> Started) -> bool {
        match probe(self.pid) {
            Started::Running(t) => t != self.started,
            Started::Gone => true,
            Started::Unreadable(_) => false,
        }
    }
}

/// What a process id names now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Started {
    /// A running process, started at this time on the system's own clock.
    Running(u64),
    /// No process.
    Gone,
    /// A process this one may not read.
    Unreadable(String),
}

/// Removes the per-process folders under `root` whose processes have
/// ended, as `probe` reads each id, leaving this process's own and any
/// folder whose name is not a process's; what could not be removed comes
/// back as notes.
#[cfg(any(not(target_os = "macos"), test))]
fn sweep(root: &std::path::Path, current: Owner, probe: impl Fn(u32) -> Started) -> Vec<String> {
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => return vec![format!("the folder of per-process web data {} cannot be read: {e}", root.display())],
    };
    let mut notes = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                notes.push(format!("an entry of {} cannot be read: {e}", root.display()));
                continue;
            }
        };
        let Some(owner) = entry.file_name().to_str().and_then(Owner::parse) else {
            continue;
        };
        if owner == current || !owner.has_ended(&probe) {
            continue;
        }
        let path = entry.path();
        match std::fs::remove_dir_all(&path) {
            Ok(()) => pwrs_gui_proto::trace::line(|| format!("web: removed {}, whose process has ended", path.display())),
            Err(e) => notes.push(format!("the web data folder {} of a process that has ended could not be removed: {e}; a later start tries again", path.display())),
        }
    }
    notes
}

/// Removes the shared folders of Fixed Version runtimes under `root`,
/// `WebView2-<version>`, whose version is not in `keep` and which no
/// browser process has open; what could not be removed comes back as notes.
#[cfg(windows)]
fn sweep_versions(root: &std::path::Path, keep: &[String]) -> Vec<String> {
    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => return vec![format!("the web data folder {} cannot be read: {e}", root.display())],
    };
    let mut notes = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                notes.push(format!("an entry of {} cannot be read: {e}", root.display()));
                continue;
            }
        };
        let name = entry.file_name();
        let Some(version) = name.to_str().and_then(|n| n.strip_prefix("WebView2-")) else {
            continue;
        };
        if version == "PerProcess" || keep.iter().any(|k| k == version) {
            continue;
        }
        let path = entry.path();
        match held(&path) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(why) => {
                notes.push(why);
                continue;
            }
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => pwrs_gui_proto::trace::line(|| format!("web: removed {}, the data of a WebView2 runtime no installed add-on has", path.display())),
            Err(e) => notes.push(format!("the web data folder {} of a WebView2 runtime no longer installed could not be removed: {e}; a later start tries again", path.display())),
        }
    }
    notes
}

/// Whether a browser process has the WebView2 data folder `folder` open.
/// One that runs has `EBWebView\lockfile` open, shared for reading alone,
/// so it cannot then be opened for writing.
#[cfg(windows)]
fn held(folder: &std::path::Path) -> Result<bool, String> {
    use windows::Win32::Foundation::ERROR_SHARING_VIOLATION;
    let lock = folder.join("EBWebView").join("lockfile");
    match std::fs::OpenOptions::new().write(true).open(&lock) {
        Ok(file) => {
            drop(file);
            Ok(false)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION.0.cast_signed()) => Ok(true),
        Err(e) => Err(format!("whether a browser process has the web data folder {} open cannot be told: {e}", folder.display())),
    }
}

/// What process id `pid` names now.
#[cfg(windows)]
pub(crate) fn started_at(pid: u32) -> Started {
    use windows::Win32::Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, STILL_ACTIVE};
    use windows::Win32::System::Threading::{GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
        Ok(h) => h,
        Err(e) if e.code() == ERROR_INVALID_PARAMETER.to_hresult() => return Started::Gone,
        Err(e) => return Started::Unreadable(e.message()),
    };
    let mut code = 0u32;
    let exit = unsafe { GetExitCodeProcess(handle, &mut code) };
    let (mut created, mut exited, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    let times = unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) };
    if let Err(e) = unsafe { CloseHandle(handle) } {
        return Started::Unreadable(format!("process {pid}'s handle did not close: {}", e.message()));
    }
    match (exit, times) {
        (Ok(()), _) if code != STILL_ACTIVE.0.cast_unsigned() => Started::Gone,
        (Ok(()), Ok(())) => Started::Running((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime)),
        (Err(e), _) | (_, Err(e)) => Started::Unreadable(e.message()),
    }
}

/// What process id `pid` names now: field 22 of its `stat`, its start in
/// clock ticks since boot.
#[cfg(target_os = "linux")]
pub(crate) fn started_at(pid: u32) -> Started {
    let path = format!("/proc/{pid}/stat");
    let stat = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Started::Gone,
        Err(e) => return Started::Unreadable(format!("{path}: {e}")),
    };
    // The command name is field 2, in parentheses, and may contain any
    // character, so the fields are counted from the last parenthesis:
    // field 3 is the first after it.
    match stat.rsplit_once(')').and_then(|(_, rest)| rest.split_whitespace().nth(22 - 3)) {
        Some(t) => t.parse().map_or_else(|e| Started::Unreadable(format!("{path}: start time {t}: {e}")), Started::Running),
        None => Started::Unreadable(format!("{path} has no start time")),
    }
}

/// What process id `pid` names now: its start time from the kernel's
/// process table, in microseconds since the epoch.
#[cfg(target_os = "freebsd")]
pub(crate) fn started_at(pid: u32) -> Started {
    let Ok(id) = libc::c_int::try_from(pid) else {
        return Started::Gone;
    };
    let mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, id];
    let mut info = std::mem::MaybeUninit::<libc::kinfo_proc>::zeroed();
    let mut size = std::mem::size_of::<libc::kinfo_proc>();
    let r = unsafe { libc::sysctl(mib.as_ptr(), 4, info.as_mut_ptr().cast(), &mut size, std::ptr::null(), 0) };
    if r != 0 {
        let e = std::io::Error::last_os_error();
        return if e.raw_os_error() == Some(libc::ESRCH) { Started::Gone } else { Started::Unreadable(e.to_string()) };
    }
    if size == 0 {
        return Started::Gone;
    }
    let info = unsafe { info.assume_init() };
    match (u64::try_from(info.ki_start.tv_sec), u64::try_from(info.ki_start.tv_usec)) {
        (Ok(seconds), Ok(micros)) => Started::Running(seconds * 1_000_000 + micros),
        _ => Started::Unreadable(format!("process {pid} has a start time before the epoch")),
    }
}

/// What process id `pid` names now: its start time in microseconds since
/// the epoch.
#[cfg(target_os = "macos")]
pub(crate) fn started_at(pid: u32) -> Started {
    let Ok(id) = libc::c_int::try_from(pid) else {
        return Started::Gone;
    };
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_bsdinfo>()).expect("the process information block is a few hundred bytes");
    let written = unsafe { libc::proc_pidinfo(id, libc::PROC_PIDTBSDINFO, 0, info.as_mut_ptr().cast(), size) };
    if written != size {
        let e = std::io::Error::last_os_error();
        return if e.raw_os_error() == Some(libc::ESRCH) { Started::Gone } else { Started::Unreadable(e.to_string()) };
    }
    let info = unsafe { info.assume_init() };
    Started::Running(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "freebsd", target_os = "macos")))]
pub(crate) fn started_at(_pid: u32) -> Started {
    Started::Unreadable(String::from("process start times are read on Windows, Linux, FreeBSD and macOS"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owner_names_its_folder_and_store_and_reads_back_from_both() {
        let owner = Owner { pid: 4242, started: 133_700_000_000_000_123 };
        assert_eq!(owner.name(), "4242-133700000000000123");
        assert_eq!(Owner::parse(&owner.name()), Some(owner));
        assert_eq!(Owner::of_store(&owner.store()), Some(owner));
        assert_eq!(Owner::parse("EBWebView"), None);
        assert_eq!(Owner::parse("12-x"), None);
        assert_eq!(Owner::of_store(&[0u8; 16]), None);
    }

    #[test]
    fn this_process_reads_as_running_since_its_own_start() {
        let owner = Owner::current().expect("this process's start time reads");
        assert_eq!(started_at(owner.pid), Started::Running(owner.started));
        assert!(!owner.has_ended(started_at));
    }

    #[test]
    fn an_owner_has_ended_when_its_id_is_free_or_held_by_another_start() {
        let owner = Owner { pid: 7, started: 100 };
        assert!(owner.has_ended(|_| Started::Gone));
        assert!(owner.has_ended(|_| Started::Running(101)));
        assert!(!owner.has_ended(|_| Started::Running(100)));
        assert!(!owner.has_ended(|_| Started::Unreadable(String::from("denied"))));
    }

    #[test]
    fn a_sweep_removes_only_the_folders_of_ended_processes() {
        let root = std::env::temp_dir().join(format!("pwrs-gui-web-sweep-{}", std::process::id()));
        let current = Owner { pid: 1, started: 10 };
        let running = Owner { pid: 2, started: 20 };
        let ended = Owner { pid: 3, started: 30 };
        let reused = Owner { pid: 4, started: 40 };
        let unreadable = Owner { pid: 5, started: 50 };
        for name in [current.name(), running.name(), ended.name(), reused.name(), unreadable.name(), String::from("not-a-process")] {
            std::fs::create_dir_all(root.join(&name).join("EBWebView")).expect("the test folder is made");
        }
        let probe = |pid: u32| match pid {
            2 => Started::Running(20),
            4 => Started::Running(41),
            5 => Started::Unreadable(String::from("denied")),
            _ => Started::Gone,
        };
        let notes = sweep(&root, current, probe);
        let mut left: Vec<String> = std::fs::read_dir(&root).expect("the test folder reads").map(|e| e.expect("an entry reads").file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        std::fs::remove_dir_all(&root).expect("the test folder is removed");
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(left, vec![current.name(), running.name(), unreadable.name(), String::from("not-a-process")]);
    }

    #[test]
    fn a_sweep_of_a_missing_folder_has_nothing_to_say() {
        let root = std::env::temp_dir().join(format!("pwrs-gui-web-sweep-missing-{}", std::process::id()));
        assert!(sweep(&root, Owner { pid: 1, started: 1 }, |_| Started::Gone).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn a_sweep_removes_the_data_of_runtimes_no_add_on_holds_unless_a_browser_holds_it() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
        let root = std::env::temp_dir().join(format!("pwrs-gui-web-versions-{}", std::process::id()));
        for name in ["WebView2-1.0.0.0", "WebView2-2.0.0.0", "WebView2-3.0.0.0", "WebView2-PerProcess", "WebView2"] {
            std::fs::create_dir_all(root.join(name).join("EBWebView")).expect("the test folder is made");
        }
        let lock = root.join("WebView2-3.0.0.0").join("EBWebView").join("lockfile");
        let holder = std::fs::OpenOptions::new().write(true).create(true).truncate(false).share_mode(FILE_SHARE_READ.0).open(&lock).expect("the lock is open as a browser has it open");
        let notes = sweep_versions(&root, &[String::from("2.0.0.0")]);
        let mut left: Vec<String> = std::fs::read_dir(&root).expect("the test folder reads").map(|e| e.expect("an entry reads").file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        drop(holder);
        std::fs::remove_dir_all(&root).expect("the test folder is removed");
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(left, ["WebView2", "WebView2-2.0.0.0", "WebView2-3.0.0.0", "WebView2-PerProcess"]);
    }
}
