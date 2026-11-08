//! The web helper process. Its main thread runs the HTML renderer, a tao
//! loop with a webview per window; one inbound thread passes it each message
//! on the inbound ring ([`pwrs_gui_proto::helper`]).
//!
//! `pwrs-gui-web --session <id> [--web-view shared|per-process] [--web-view-data-folder <path>]
//! [--webview2-runtime <folder> [--webview2-installed <folder>]...] [--fonts bundled|system]`
//!
//! `--webview2-runtime` starts the webviews on the WebView2 Fixed Version
//! runtime in that folder, the add-on's; each `--webview2-installed` names
//! the runtime of an installed add-on version, whose data folder a start
//! does not remove, whichever runtime it runs on. `--fonts` chooses the
//! fonts pages draw text in, the build's own unless it says `system`.

#![windows_subsystem = "windows"]

use pwrs_gui_proto::Fonts;
use pwrs_gui_web::{Data, Runtimes, Settings};
use std::path::PathBuf;

struct Args {
    session: String,
    data: Data,
    runtimes: Runtimes,
    fonts: Fonts,
}

fn parse_args() -> Result<Args, String> {
    let mut session = None;
    let mut per_process = false;
    let mut folder = None;
    let mut runtimes = Runtimes::default();
    let mut fonts = Fonts::default();
    let mut args = std::env::args_os().skip(1);
    let text = |name: &str, value: Option<std::ffi::OsString>| match value {
        Some(v) => v.into_string().map_err(|v| format!("{name} {} is not text", v.to_string_lossy())),
        None => Err(format!("{name} needs a value")),
    };
    while let Some(a) = args.next() {
        match a.to_str() {
            Some("--session") => session = Some(text("--session", args.next())?),
            Some("--web-view") => match text("--web-view", args.next())?.as_str() {
                "shared" => per_process = false,
                "per-process" => per_process = true,
                other => return Err(format!("--web-view is shared or per-process, not {other}")),
            },
            Some("--web-view-data-folder") => folder = Some(PathBuf::from(args.next().ok_or("--web-view-data-folder needs a value")?)),
            Some("--webview2-runtime") => runtimes.fixed = Some(PathBuf::from(args.next().ok_or("--webview2-runtime needs a folder")?)),
            Some("--webview2-installed") => runtimes.installed.push(PathBuf::from(args.next().ok_or("--webview2-installed needs a folder")?)),
            Some("--fonts") => fonts = Fonts::from_arg(&text("--fonts", args.next())?)?,
            _ => return Err(format!("unknown argument {}", a.to_string_lossy())),
        }
    }
    let data = match (per_process, folder) {
        (false, folder) => Data::Shared(folder),
        (true, None) => Data::PerProcess,
        (true, Some(f)) => return Err(format!("--web-view-data-folder {} names the shared folder, which --web-view per-process does not use", f.display())),
    };
    Ok(Args { session: session.ok_or("--session is required")?, data, runtimes, fonts })
}

fn main() {
    // EGL is probed first, while the process has one thread: the probe
    // forks, and sets the environment WebKitGTK reads as it starts.
    #[cfg(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    // SAFETY: nothing has started a thread yet.
    let log: Vec<String> = unsafe { pwrs_gui_web::choose_renderer() }.into_iter().collect();
    #[cfg(not(any(target_os = "linux", target_os = "dragonfly", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
    let log = Vec::new();
    let code = match parse_args().and_then(|args| run(args, log)) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("pwrs-gui-web: {e}");
            1
        }
    };
    pwrs_gui_web::end_helper(code);
}

fn run(args: Args, log: Vec<String>) -> Result<(), String> {
    let settings = Settings { any_thread: false, data: args.data, bring_to_front: false, log, runtimes: args.runtimes, fonts: args.fonts };
    pwrs_gui_proto::helper::serve(&args.session, "pwrs-gui-web", |link| pwrs_gui_web::run(link, settings))
}
