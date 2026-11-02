//! Where a new window opens, and whether it takes the focus.
//! `PWRS_GUI_MONITOR` names a monitor by its number, and each window a
//! renderer opens is centered on it; without the variable the system places
//! windows. On Windows the number is the display's own, as in
//! `\\.\DISPLAY2`, which Windows' display settings show; elsewhere it counts
//! the monitors in the order the windowing system lists them, from 1.
//! `PWRS_GUI_NO_FOCUS` set to 1 opens every window without taking the
//! focus, so the windows a test opens leave a person's work alone, on
//! Windows, macOS and X11; a Wayland compositor gives the focus as it
//! chooses.

/// The variable naming the monitor new windows open on.
pub const VAR: &str = "PWRS_GUI_MONITOR";

/// The variable that, set to 1, opens windows without taking the focus.
pub const NO_FOCUS: &str = "PWRS_GUI_NO_FOCUS";

/// Whether `PWRS_GUI_NO_FOCUS` has windows open without taking the focus:
/// 1 does, and unset, empty or 0 does not; any other value is why
/// windows take the focus as they open. A renderer on Wayland cannot act
/// on it and says so.
pub fn no_focus() -> Result<bool, String> {
    parse_no_focus(std::env::var(NO_FOCUS))
}

/// What a renderer says when `PWRS_GUI_NO_FOCUS` is 1 and it draws on
/// Wayland, where the compositor gives the focus as it chooses.
pub fn no_focus_on_wayland() -> String {
    format!("{NO_FOCUS} is 1, and on Wayland the compositor gives a new window the focus as it chooses")
}

fn parse_no_focus(value: Result<String, std::env::VarError>) -> Result<bool, String> {
    match value {
        Err(std::env::VarError::NotPresent) => Ok(false),
        Err(std::env::VarError::NotUnicode(v)) => Err(format!("{NO_FOCUS} is not text ({}), so windows take the focus as they open", v.to_string_lossy())),
        Ok(v) => match v.trim() {
            "" | "0" => Ok(false),
            "1" => Ok(true),
            other => Err(format!("{NO_FOCUS} is '{other}', which is neither 1 nor 0, so windows take the focus as they open")),
        },
    }
}

/// A monitor, in physical pixels on the virtual screen.
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    /// The system's name for it: `\\.\DISPLAY2` on Windows.
    pub name: Option<String>,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// The part of it windows open in, across and down: what the taskbar
    /// leaves on Windows, the whole monitor where the system does not say.
    pub work: [u32; 2],
    /// Physical pixels per point.
    pub scale: f64,
}

impl Monitor {
    /// Its work area in points.
    pub fn work_points(&self) -> [f32; 2] {
        [(f64::from(self.work[0]) / self.scale) as f32, (f64::from(self.work[1]) / self.scale) as f32]
    }
}

/// The monitor a new window opens on: the one numbered `n` among
/// `monitors` when it is listed, else the primary one, whose corner is the
/// virtual screen's origin.
pub fn opening_on(monitors: &[Monitor], n: Option<u32>) -> Option<&Monitor> {
    n.and_then(|n| pick(monitors, n)).or_else(|| monitors.iter().find(|m| m.x == 0 && m.y == 0))
}

/// The monitor number `PWRS_GUI_MONITOR` gives; `None` when it is not set
/// or is empty; or why its value names no monitor.
pub fn wanted() -> Result<Option<u32>, String> {
    parse(std::env::var(VAR))
}

fn parse(value: Result<String, std::env::VarError>) -> Result<Option<u32>, String> {
    match value {
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(v)) => Err(format!("{VAR} is not text ({}), so windows open where the system places them", v.to_string_lossy())),
        Ok(v) if v.trim().is_empty() => Ok(None),
        Ok(v) => match v.trim().parse::<u32>() {
            Ok(n) if n >= 1 => Ok(Some(n)),
            _not_a_number => Err(format!("{VAR} is '{v}', which is not a monitor's number from 1, so windows open where the system places them")),
        },
    }
}

/// The monitor numbered `n` among `monitors`.
pub fn pick(monitors: &[Monitor], n: u32) -> Option<&Monitor> {
    if cfg!(windows) { monitors.iter().find(|m| number(m) == Some(n)) } else { monitors.get(usize::try_from(n).ok()?.checked_sub(1)?) }
}

/// The number a Windows display's device name ends in, `\\.\DISPLAY<n>`.
pub fn number(m: &Monitor) -> Option<u32> {
    let name = m.name.as_deref()?;
    let at = name.rfind("DISPLAY")?;
    name[at + "DISPLAY".len()..].parse().ok()
}

/// What a renderer that starts says about the variable: nothing when it is
/// unset or names one of `monitors`, and otherwise why windows open where
/// the system places them.
pub fn note(wanted: &Result<Option<u32>, String>, monitors: &Result<Vec<Monitor>, String>) -> Option<String> {
    match (wanted, monitors) {
        (Ok(None), _) => None,
        (Err(why), _) => Some(why.clone()),
        (Ok(Some(n)), Err(why)) => Some(format!("{VAR} names monitor {n}, but the monitors cannot be listed ({why}), so windows open where the system places them")),
        (Ok(Some(n)), Ok(list)) if pick(list, *n).is_some() => None,
        (Ok(Some(n)), Ok(list)) => {
            let mut numbers: Vec<u32> = if cfg!(windows) { list.iter().filter_map(number).collect() } else { (1..=u32::try_from(list.len()).unwrap_or(u32::MAX)).collect() };
            numbers.sort_unstable();
            let numbers: Vec<String> = numbers.iter().map(u32::to_string).collect();
            Some(format!("{VAR} names monitor {n}, but this machine's monitors are numbered {}, so windows open where the system places them", numbers.join(", ")))
        }
    }
}

/// The monitor number a renderer places its windows on: the one
/// `PWRS_GUI_MONITOR` names when it is among `monitors`, which [`note`]
/// otherwise explains.
pub fn found(wanted: &Result<Option<u32>, String>, monitors: &Result<Vec<Monitor>, String>) -> Option<u32> {
    match (wanted, monitors) {
        (Ok(Some(n)), Ok(list)) if pick(list, *n).is_some() => Some(*n),
        _elsewhere => None,
    }
}

/// The top-left corner, in physical pixels, that centers a window of
/// `width` by `height` points on `m`; a window larger than the monitor
/// opens with its top-left corner on it.
pub fn centered(m: &Monitor, width: f64, height: f64) -> (i32, i32) {
    let spare_width = (f64::from(m.width) - width * m.scale).max(0.0);
    let spare_height = (f64::from(m.height) - height * m.scale).max(0.0);
    (m.x + (spare_width / 2.0) as i32, m.y + (spare_height / 2.0) as i32)
}

/// The monitors Windows lists, in physical pixels when the process is aware
/// of each monitor's scale, as winit and tao make it once their event loop
/// is built.
#[cfg(windows)]
pub fn monitors() -> Result<Vec<Monitor>, String> {
    use windows_sys::Win32::Foundation::{LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW};
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows_sys::core::BOOL;

    struct Found {
        monitors: Vec<Monitor>,
        failed: Option<String>,
    }

    /// One monitor of the enumeration, added to the `Found` that `data`
    /// points to; a monitor that cannot be read ends the enumeration.
    unsafe extern "system" fn each(monitor: HMONITOR, _dc: HDC, _clip: *mut RECT, data: LPARAM) -> BOOL {
        let found = unsafe { &mut *(data as *mut Found) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = u32::try_from(std::mem::size_of::<MONITORINFOEXW>()).expect("the monitor information block is a few dozen bytes");
        if unsafe { GetMonitorInfoW(monitor, (&raw mut info).cast::<MONITORINFO>()) } == 0 {
            found.failed = Some(format!("GetMonitorInfoW failed: {}", std::io::Error::last_os_error()));
            return 0;
        }
        let (mut dpi_x, mut dpi_y) = (0u32, 0u32);
        let hr = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
        if hr < 0 {
            found.failed = Some(format!("GetDpiForMonitor failed with {hr:#010x}"));
            return 0;
        }
        let r = info.monitorInfo.rcMonitor;
        let work = info.monitorInfo.rcWork;
        let end = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
        found.monitors.push(Monitor {
            name: Some(String::from_utf16_lossy(&info.szDevice[..end])),
            x: r.left,
            y: r.top,
            width: (r.right - r.left).unsigned_abs(),
            height: (r.bottom - r.top).unsigned_abs(),
            work: [(work.right - work.left).unsigned_abs(), (work.bottom - work.top).unsigned_abs()],
            scale: f64::from(dpi_x) / 96.0,
        });
        1
    }

    let mut found = Found { monitors: Vec::new(), failed: None };
    let listed = unsafe { EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(each), (&raw mut found) as LPARAM) };
    match (listed, found.failed) {
        (_, Some(why)) => Err(why),
        (0, None) => Err(format!("EnumDisplayMonitors failed: {}", std::io::Error::last_os_error())),
        (_, None) => Ok(found.monitors),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(name: &str, x: i32, width: u32, scale: f64) -> Monitor {
        Monitor { name: Some(name.to_string()), x, y: 0, width, height: 1200, work: [width, 1152], scale }
    }

    #[test]
    fn a_window_opens_on_the_monitor_named_or_else_the_primary_and_fits_its_work_area() {
        let listed = vec![monitor("\\\\.\\DISPLAY2", 2560, 1920, 1.5), monitor("\\\\.\\DISPLAY1", 0, 2560, 1.0)];
        assert_eq!(opening_on(&listed, None).map(|m| m.x), Some(0), "the primary, at the origin");
        assert_eq!(opening_on(&listed, Some(9)).map(|m| m.x), Some(0), "a number naming none opens on the primary");
        let named = opening_on(&listed, Some(if cfg!(windows) { 2 } else { 1 })).expect("the monitor named");
        assert_eq!(named.work_points(), [1280.0, 768.0], "the work area in points, at its scale");
    }

    #[test]
    fn the_variable_takes_a_number_from_one_and_names_anything_else() {
        assert_eq!(parse(Err(std::env::VarError::NotPresent)), Ok(None));
        assert_eq!(parse(Ok(String::from(" "))), Ok(None));
        assert_eq!(parse(Ok(String::from(" 2 "))), Ok(Some(2)));
        let zero = parse(Ok(String::from("0")));
        assert!(matches!(zero, Err(ref why) if why.contains("'0'")), "{zero:?}");
        assert!(parse(Ok(String::from("second"))).is_err());
    }

    #[test]
    fn windows_open_without_the_focus_only_when_the_variable_says_1() {
        assert!(!parse_no_focus(Err(std::env::VarError::NotPresent)).expect("unset reads"));
        assert!(!parse_no_focus(Ok(String::from(""))).expect("empty reads"));
        assert!(!parse_no_focus(Ok(String::from(" 0 "))).expect("0 reads"));
        assert!(parse_no_focus(Ok(String::from("1"))).expect("1 reads"));
        let refused = parse_no_focus(Ok(String::from("yes"))).expect_err("yes is not 1 or 0");
        assert!(refused.contains("'yes'") && refused.contains("take the focus"), "{refused}");
    }

    #[test]
    fn a_monitor_is_found_by_its_display_number_on_windows_and_its_place_elsewhere() {
        // Windows lists the second display first on a machine whose first
        // display is the primary one.
        let listed = vec![monitor("\\\\.\\DISPLAY2", 2560, 1920, 1.0), monitor("\\\\.\\DISPLAY1", 0, 2560, 1.0)];
        let second = pick(&listed, 2).expect("monitor 2 is listed");
        if cfg!(windows) {
            assert_eq!(second.x, 2560);
            assert!(pick(&listed, 3).is_none());
        } else {
            assert_eq!(second.x, 0);
        }
    }

    #[test]
    fn a_window_is_centered_in_physical_pixels_and_kept_on_its_monitor() {
        let m = monitor("\\\\.\\DISPLAY2", 2560, 1920, 1.5);
        assert_eq!(centered(&m, 400.0, 200.0), (2560 + (1920 - 600) / 2, (1200 - 300) / 2));
        assert_eq!(centered(&m, 4000.0, 4000.0), (2560, 0), "a window larger than the monitor opens with its corner on it");
    }

    #[test]
    fn a_renderer_says_why_windows_open_where_the_system_places_them() {
        let listed = Ok(vec![monitor("\\\\.\\DISPLAY1", 0, 2560, 1.0)]);
        assert_eq!(note(&Ok(None), &listed), None);
        assert_eq!(note(&Ok(Some(1)), &listed), None);
        let missing = note(&Ok(Some(4)), &listed).expect("a note for a monitor not listed");
        assert!(missing.contains("monitor 4") && missing.contains("numbered 1"), "{missing}");
        let unlisted = note(&Ok(Some(1)), &Err(String::from("denied"))).expect("a note when nothing is listed");
        assert!(unlisted.contains("denied"), "{unlisted}");
        assert_eq!(found(&Ok(Some(1)), &listed), Some(1));
        assert_eq!(found(&Ok(Some(4)), &listed), None, "a monitor not listed places nothing");
        assert_eq!(found(&Err(String::from("bad")), &listed), None);
    }

    /// A session with no display, as a service's, lists one monitor named
    /// `WinDisc` with no display number, which no number picks.
    #[cfg(windows)]
    #[test]
    fn windows_lists_at_least_one_monitor_with_a_size_and_a_scale() {
        let listed = monitors().expect("the monitors list");
        assert!(!listed.is_empty());
        for m in &listed {
            assert!(m.width > 0 && m.height > 0 && m.scale > 0.0, "{m:?}");
            if m.name.as_deref().is_some_and(|n| n.starts_with("\\\\.\\DISPLAY")) {
                assert!(number(m).is_some(), "{m:?}");
            }
        }
    }
}
