//! What a page of the caller's own does to its own window: the build of a
//! window whose page draws its own frame, what its page is told of its
//! window, and what a page asks of its window from script: minimizing,
//! maximizing and restoring it, moving and resizing it to a place and a
//! size, giving it the focus, filling its monitor with it, naming it,
//! closing it as its person would, and starting the system's move or resize
//! of it while the pointer's button is held.

use pwrs_gui_proto::WindowId;
use serde_json::{Value as Json, json};
use tao::dpi::{LogicalPosition, LogicalSize};
use tao::window::{Fullscreen, ResizeDirection, UserAttentionType, Window, WindowBuilder};

/// How much of a window, across and down, in points, a page's move or
/// resize of it leaves on a monitor.
pub(super) const KEPT_ON_SCREEN: f64 = 48.0;

/// What a page asked of its window, as `window.pwrs` posts it.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum WindowOp {
    Minimize,
    Maximize,
    /// Neither minimized nor maximized.
    Restore,
    /// Maximized, or restored when it is maximized.
    ToggleMaximize,
    /// Closed as its person closes it, with the system's close button;
    /// `confirmed` once the page's onclose handlers have let the close go.
    Close { confirmed: bool },
    /// The page's onclose handlers hold the system's close of the window
    /// for the page to let go or keep.
    HoldClose,
    /// The page's onclose handlers kept the window open at a close it was
    /// asked about.
    CloseKept,
    /// The system's move of the window, which follows the pointer while
    /// the button that pressed the page stays held.
    Drag,
    /// The system's resize of the window from that edge or corner, while
    /// the button that pressed the page stays held.
    Resize(ResizeDirection),
    /// Moved so its top left is at this point of the screen, in points.
    MoveTo { x: f64, y: f64 },
    /// Its inside made this wide and high, in points.
    ResizeTo { width: f64, height: f64 },
    /// Given the keyboard's focus, as far as the system allows.
    Focus,
    /// Filling its monitor, or back as it was.
    Fullscreen(bool),
    /// Named so in its title bar, and to Get-GuiWindow.
    SetTitle(String),
}

impl WindowOp {
    /// What a page's `window` message `msg` asks for: its `op`, and the
    /// edge, place, size, state or title the op takes.
    pub(super) fn of(msg: &Json) -> Result<WindowOp, String> {
        let number = |key: &str| match msg.get(key).and_then(Json::as_f64) {
            Some(v) if v.is_finite() => Ok(v),
            Some(_) | None => Err(format!("a page's move or resize of its window has no {key} that is a number: {msg}")),
        };
        match msg.get("op").and_then(Json::as_str) {
            Some("minimize") => Ok(WindowOp::Minimize),
            Some("maximize") => Ok(WindowOp::Maximize),
            Some("restore") => Ok(WindowOp::Restore),
            Some("toggleMaximize") => Ok(WindowOp::ToggleMaximize),
            Some("close") => Ok(WindowOp::Close { confirmed: msg.get("confirmed").and_then(Json::as_bool) == Some(true) }),
            Some("holdClose") => Ok(WindowOp::HoldClose),
            Some("closeKept") => Ok(WindowOp::CloseKept),
            Some("drag") => Ok(WindowOp::Drag),
            Some("resize") => edge(msg.get("edge")).map(WindowOp::Resize),
            Some("moveTo") => Ok(WindowOp::MoveTo { x: number("x")?, y: number("y")? }),
            Some("resizeTo") => {
                let (width, height) = (number("width")?, number("height")?);
                if width <= 0.0 || height <= 0.0 {
                    return Err(format!("a window is resized to a width and a height above 0, not {width} by {height}"));
                }
                Ok(WindowOp::ResizeTo { width, height })
            }
            Some("focus") => Ok(WindowOp::Focus),
            Some("fullscreen") => match msg.get("on").and_then(Json::as_bool) {
                Some(on) => Ok(WindowOp::Fullscreen(on)),
                None => Err(format!("a page's fullscreen of its window says neither true nor false: {msg}")),
            },
            Some("setTitle") => match msg.get("title").and_then(Json::as_str) {
                Some(title) => Ok(WindowOp::SetTitle(title.to_string())),
                None => Err(format!("a page's title for its window is no string: {msg}")),
            },
            Some(other) => Err(format!("a page asked its window for {other:?}, which is nothing a window does")),
            None => Err(String::from("a page's window message names nothing to do")),
        }
    }

    /// The move as a snapshot names one a test started: `drag`, or
    /// `resize` and the edge.
    pub(super) fn started(&self) -> Option<String> {
        match self {
            WindowOp::Drag => Some(String::from("drag")),
            WindowOp::Resize(edge) => Some(format!("resize {}", edge_name(*edge))),
            WindowOp::Minimize
            | WindowOp::Maximize
            | WindowOp::Restore
            | WindowOp::ToggleMaximize
            | WindowOp::Close { .. }
            | WindowOp::HoldClose
            | WindowOp::CloseKept
            | WindowOp::MoveTo { .. }
            | WindowOp::ResizeTo { .. }
            | WindowOp::Focus
            | WindowOp::Fullscreen(_)
            | WindowOp::SetTitle(_) => None,
        }
    }
}

/// The edges and corners a resize starts from, by the names a page gives
/// them.
const EDGES: [(&str, ResizeDirection); 8] = [
    ("n", ResizeDirection::North),
    ("ne", ResizeDirection::NorthEast),
    ("e", ResizeDirection::East),
    ("se", ResizeDirection::SouthEast),
    ("s", ResizeDirection::South),
    ("sw", ResizeDirection::SouthWest),
    ("w", ResizeDirection::West),
    ("nw", ResizeDirection::NorthWest),
];

/// The edge or corner a resize starts from: `n`, `ne`, `e`, `se`, `s`,
/// `sw`, `w` or `nw`.
fn edge(given: Option<&Json>) -> Result<ResizeDirection, String> {
    let named = given.and_then(Json::as_str);
    match EDGES.iter().find(|(name, _)| Some(*name) == named) {
        Some((_, edge)) => Ok(*edge),
        None => Err(format!("a window resizes from n, ne, e, se, s, sw, w or nw, not {named:?}")),
    }
}

fn edge_name(edge: ResizeDirection) -> &'static str {
    match EDGES.iter().find(|(_, e)| *e == edge) {
        Some((name, _)) => name,
        None => "",
    }
}

/// Makes `op` on `window`. A move or a resize leaves [`KEPT_ON_SCREEN`]
/// points of the window on a monitor ([`kept`]). The focus is given as far
/// as the system allows, and where it gives none the window asks for the
/// person's attention instead. A close is the window's owner's to make,
/// since it ends the window, so it is refused here.
pub(super) fn act(window: &Window, op: &WindowOp) -> Result<(), String> {
    match op {
        WindowOp::Minimize => window.set_minimized(true),
        WindowOp::Maximize => window.set_maximized(true),
        WindowOp::Restore => {
            window.set_minimized(false);
            window.set_maximized(false);
        }
        WindowOp::ToggleMaximize => window.set_maximized(!window.is_maximized()),
        WindowOp::Drag => window.drag_window().map_err(|e| format!("the system's move of the window could not start: {e}"))?,
        WindowOp::Resize(edge) => window.drag_resize_window(*edge).map_err(|e| format!("the system's resize of the window from {edge:?} could not start: {e}"))?,
        WindowOp::MoveTo { x, y } => {
            let size = window.outer_size().to_logical::<f64>(window.scale_factor());
            let [x, y] = kept([*x, *y, size.width, size.height], &monitors(window));
            window.set_outer_position(LogicalPosition::new(x, y));
        }
        WindowOp::ResizeTo { width, height } => {
            let scale = window.scale_factor();
            let (outer, inner) = (window.outer_size().to_logical::<f64>(scale), window.inner_size().to_logical::<f64>(scale));
            let (width, height) = (width.max(KEPT_ON_SCREEN), height.max(KEPT_ON_SCREEN));
            window.set_inner_size(LogicalSize::new(width, height));
            let at = window.outer_position().map_err(|e| format!("the window was resized, and its place cannot be read to keep it on a monitor: {e}"))?.to_logical::<f64>(scale);
            let rect = [at.x, at.y, width + outer.width - inner.width, height + outer.height - inner.height];
            let [x, y] = kept(rect, &monitors(window));
            if (x, y) != (at.x, at.y) {
                window.set_outer_position(LogicalPosition::new(x, y));
            }
        }
        WindowOp::Focus => {
            window.set_focus();
            if !window.is_focused() {
                window.request_user_attention(Some(UserAttentionType::Informational));
            }
        }
        WindowOp::Fullscreen(on) => window.set_fullscreen(if *on { Some(Fullscreen::Borderless(None)) } else { None }),
        WindowOp::SetTitle(title) => window.set_title(title),
        WindowOp::Close { .. } | WindowOp::HoldClose | WindowOp::CloseKept => return Err(String::from("a window's close is made by the renderer that owns it, not here")),
    }
    Ok(())
}

/// Each monitor's box, `[x, y, width, height]` in points.
fn monitors(window: &Window) -> Vec<[f64; 4]> {
    window
        .available_monitors()
        .map(|m| {
            let scale = m.scale_factor();
            let (at, size) = (m.position().to_logical::<f64>(scale), m.size().to_logical::<f64>(scale));
            [at.x, at.y, size.width, size.height]
        })
        .collect()
}

/// Where a window whose box is `rect`, `[x, y, width, height]` in points,
/// goes so that at least [`KEPT_ON_SCREEN`] points of it across and down,
/// or all of it where it is smaller, lie on one of `monitors`: where it is
/// when they already do, otherwise the nearest such place on the monitor
/// nearest its middle. Where no monitor is known, where it is.
pub(super) fn kept(rect: [f64; 4], monitors: &[[f64; 4]]) -> [f64; 2] {
    let [x, y, w, h] = rect;
    let (least_x, least_y) = (KEPT_ON_SCREEN.min(w), KEPT_ON_SCREEN.min(h));
    let on = |m: &[f64; 4]| (x + w).min(m[0] + m[2]) - x.max(m[0]) >= least_x && (y + h).min(m[1] + m[3]) - y.max(m[1]) >= least_y;
    if monitors.iter().any(on) {
        return [x, y];
    }
    let middle = (x + w / 2.0, y + h / 2.0);
    let away = |m: &[f64; 4]| {
        let across = (m[0] - middle.0).max(middle.0 - (m[0] + m[2])).max(0.0);
        let down = (m[1] - middle.1).max(middle.1 - (m[1] + m[3])).max(0.0);
        across * across + down * down
    };
    match monitors.iter().min_by(|a, b| away(a).total_cmp(&away(b))) {
        Some(m) => [x.max(m[0] - w + least_x).min(m[0] + m[2] - least_x), y.max(m[1] - h + least_y).min(m[1] + m[3] - least_y)],
        None => [x, y],
    }
}

/// `builder` for a window whose page draws its own frame. On macOS the
/// window keeps its frame with its title bar see-through and its page
/// under it, so the system still resizes, rounds and shadows the window,
/// with its three buttons over the page's top left unless `hide_buttons`.
/// Elsewhere it has no frame of the system's; on Windows it keeps the
/// system's shadow, whose edges resize it.
#[cfg(target_os = "macos")]
pub(super) fn frameless(builder: WindowBuilder, hide_buttons: bool) -> WindowBuilder {
    use tao::platform::macos::WindowBuilderExtMacOS;
    builder.with_titlebar_transparent(true).with_title_hidden(true).with_fullsize_content_view(true).with_titlebar_buttons_hidden(hide_buttons)
}

#[cfg(not(target_os = "macos"))]
#[expect(unused_variables, reason = "only macOS draws window buttons over a frameless page")]
pub(super) fn frameless(builder: WindowBuilder, hide_buttons: bool) -> WindowBuilder {
    let builder = builder.with_decorations(false);
    #[cfg(windows)]
    let builder = {
        use tao::platform::windows::WindowBuilderExtWindows;
        builder.with_undecorated_shadow(true)
    };
    builder
}

/// The edges of a frameless window its page's bridge starts the system's
/// resize from: on Linux and the BSDs every edge and corner, since the page
/// covers the whole window; on Windows the top edge and its corners, which
/// lie inside the page, the others lying in the system's shadow; on macOS
/// none, since the system's own frame resizes it.
pub(super) fn bridge_edges() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &[]
    } else if cfg!(windows) {
        &["n", "ne", "nw"]
    } else {
        &["n", "ne", "e", "se", "s", "sw", "w", "nw"]
    }
}

/// Where the window's own buttons are, in points from the top left of its
/// page, `[x, y, width, height]`: the box around macOS's three buttons of a
/// frameless window that shows them; none elsewhere.
#[cfg(target_os = "macos")]
pub(super) fn window_buttons(window: &Window) -> Option<[f64; 4]> {
    use objc2_app_kit::{NSWindow, NSWindowButton};
    use tao::platform::macos::WindowExtMacOS;
    // The helper's loop runs on the main thread, where AppKit's windows are
    // used, and tao's pointer is the window it built.
    let ns_window = unsafe { &*(window.ns_window() as *const NSWindow) };
    let content = ns_window.contentView()?;
    let height = content.frame().size.height;
    let mut found: Option<[f64; 4]> = None;
    for which in [NSWindowButton::CloseButton, NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton] {
        let Some(button) = ns_window.standardWindowButton(which) else { continue };
        if button.isHidden() {
            continue;
        }
        let r = button.convertRect_toView(button.bounds(), Some(&content));
        let (x, y, w, h) = (r.origin.x, height - r.origin.y - r.size.height, r.size.width, r.size.height);
        found = Some(match found {
            None => [x, y, w, h],
            Some([fx, fy, fw, fh]) => {
                let (left, top) = (fx.min(x), fy.min(y));
                [left, top, (fx + fw).max(x + w) - left, (fy + fh).max(y + h) - top]
            }
        });
    }
    found
}

#[cfg(not(target_os = "macos"))]
#[expect(unused_variables, reason = "only macOS draws window buttons over a frameless page")]
pub(super) fn window_buttons(window: &Window) -> Option<[f64; 4]> {
    None
}

/// The origin a page's own files and channels are at, as the engine names
/// `pwrs://localhost`: WebView2 maps it to `http://pwrs.localhost`.
pub(super) const OWN_ORIGIN: &str = if cfg!(windows) { "http://pwrs.localhost" } else { "pwrs://localhost" };

/// The script, run before any of the page's own, that tells the bridge of
/// the page's window: its number, as text, since a JavaScript number cannot
/// hold every window's number exactly; the origin its channels are at, its
/// own, which a page from another origin reaches them at too; whether the
/// page draws its own frame, the box of the system's buttons over it, the
/// edges the bridge starts resizes from, and whether the window's command
/// takes files dropped where the page takes none (`drop`).
pub(super) fn page_script(window: WindowId, frameless: bool, buttons: Option<[f64; 4]>, drop: bool) -> String {
    let edges: &[&str] = if frameless { bridge_edges() } else { &[] };
    let buttons = match buttons {
        Some([x, y, width, height]) => json!({ "x": x, "y": y, "width": width, "height": height }),
        None => Json::Null,
    };
    format!("window.__pwrsPage = {};", json!({ "id": window.to_string(), "origin": OWN_ORIGIN, "frameless": frameless, "buttons": buttons, "edges": edges, "drop": drop }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_s_window_message_names_its_move_and_a_resize_its_edge() {
        for (op, expected) in [
            ("minimize", WindowOp::Minimize),
            ("maximize", WindowOp::Maximize),
            ("restore", WindowOp::Restore),
            ("toggleMaximize", WindowOp::ToggleMaximize),
            ("close", WindowOp::Close { confirmed: false }),
            ("holdClose", WindowOp::HoldClose),
            ("closeKept", WindowOp::CloseKept),
            ("drag", WindowOp::Drag),
            ("focus", WindowOp::Focus),
        ] {
            assert_eq!(WindowOp::of(&json!({ "t": "window", "op": op })).expect(op), expected);
        }
        assert_eq!(WindowOp::of(&json!({ "t": "window", "op": "close", "confirmed": true })).expect("a close let go"), WindowOp::Close { confirmed: true });
        assert_eq!(WindowOp::of(&json!({ "t": "window", "op": "moveTo", "x": -20, "y": 40.5 })).expect("a move"), WindowOp::MoveTo { x: -20.0, y: 40.5 });
        assert_eq!(WindowOp::of(&json!({ "t": "window", "op": "resizeTo", "width": 420, "height": 300 })).expect("a resize"), WindowOp::ResizeTo { width: 420.0, height: 300.0 });
        assert_eq!(WindowOp::of(&json!({ "t": "window", "op": "fullscreen", "on": true })).expect("fullscreen"), WindowOp::Fullscreen(true));
        assert_eq!(WindowOp::of(&json!({ "t": "window", "op": "setTitle", "title": "Report" })).expect("a title"), WindowOp::SetTitle(String::from("Report")));
        assert!(WindowOp::of(&json!({ "t": "window", "op": "resizeTo", "width": 0, "height": 300 })).is_err(), "a width of 0");
        assert!(WindowOp::of(&json!({ "t": "window", "op": "moveTo", "x": "left", "y": 0 })).is_err(), "a place that is no number");
        assert!(WindowOp::of(&json!({ "t": "window", "op": "fullscreen" })).is_err(), "fullscreen saying neither");
        let resize = WindowOp::of(&json!({ "t": "window", "op": "resize", "edge": "se" })).expect("a resize from the bottom right corner");
        assert_eq!(resize, WindowOp::Resize(ResizeDirection::SouthEast));
        assert_eq!(resize.started().as_deref(), Some("resize se"), "a snapshot names the edge as the page did");
        assert_eq!(WindowOp::Drag.started().as_deref(), Some("drag"));
        assert_eq!(WindowOp::Maximize.started(), None, "the system runs no move of its own for a maximize");
        let refused = WindowOp::of(&json!({ "t": "window", "op": "resize", "edge": "middle" })).expect_err("no edge of that name");
        assert!(refused.contains("nw"), "{refused}");
        let refused = WindowOp::of(&json!({ "t": "window", "op": "fly" })).expect_err("no such move");
        assert!(refused.contains("fly"), "{refused}");
        assert!(WindowOp::of(&json!({ "t": "window" })).is_err(), "a message naming no move");
    }

    #[test]
    fn a_move_or_a_resize_leaves_48_points_of_the_window_on_a_monitor() {
        let monitors = [[0.0, 0.0, 1920.0, 1080.0], [1920.0, 0.0, 1280.0, 1024.0]];
        assert_eq!(kept([100.0, 100.0, 400.0, 300.0], &monitors), [100.0, 100.0], "a window on a monitor stays where it is");
        assert_eq!(kept([1800.0, 50.0, 400.0, 300.0], &monitors), [1800.0, 50.0], "a window across two monitors stays where it is");
        assert_eq!(kept([-100_000.0, -100_000.0, 400.0, 300.0], &monitors), [48.0 - 400.0, 48.0 - 300.0], "far up and left, 48 points stay on the first monitor");
        assert_eq!(kept([5000.0, 500.0, 400.0, 300.0], &monitors), [3200.0 - 48.0, 500.0], "far right, 48 points stay on the second monitor");
        assert_eq!(kept([-10.0, -10.0, 30.0, 20.0], &monitors), [0.0, 0.0], "a window smaller than 48 points stays wholly on the monitor");
        assert_eq!(kept([5.0, 5.0, 400.0, 300.0], &[]), [5.0, 5.0], "no monitor known");
    }

    #[test]
    fn a_page_is_told_of_its_window_before_its_own_scripts_run() {
        let framed = page_script(4_611_686_018_427_387_905, false, None, false);
        let expected = format!(r#"window.__pwrsPage = {{"id":"4611686018427387905","origin":"{OWN_ORIGIN}","frameless":false,"buttons":null,"edges":[],"drop":false}};"#);
        assert_eq!(framed, expected, "a framed page resizes from no edge of its own, and its number is text");
        let frameless = page_script(9, true, Some([7.0, 6.0, 52.0, 14.0]), true);
        assert!(frameless.contains(r#""buttons":{"#), "{frameless}");
        assert!(frameless.contains(r#""x":7.0"#) && frameless.contains(r#""width":52.0"#), "{frameless}");
        assert!(frameless.contains(r#""drop":true"#), "{frameless}");
        for edge in bridge_edges() {
            assert!(frameless.contains(&format!("\"{edge}\"")), "{frameless} names {edge}");
        }
    }
}
