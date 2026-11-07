//! The egui helper process. Its main thread runs the renderer; one inbound
//! thread passes it each message on the inbound ring
//! ([`pwrs_gui_proto::helper`]).
//!
//! `pwrs-gui-egui --session <id> [--adapter <name>] [--low-power]
//! [--mesa <folder> | --mesa-fallback <folder>] [--fonts bundled|system]`
//!
//! `--mesa` draws OpenGL through the Mesa add-on in `<folder>`;
//! `--mesa-fallback` does so where System32 has no `opengl32.dll`.
//! `--fonts` chooses the fonts text is drawn in, the build's own unless it
//! says `system`.

#![windows_subsystem = "windows"]

use pwrs_gui_egui::{Mesa, Settings};
use pwrs_gui_proto::Fonts;

struct Args {
    session: String,
    adapter: Option<String>,
    low_power: bool,
    mesa: Option<Mesa>,
    fonts: Fonts,
}

fn parse_args() -> Result<Args, String> {
    let mut session = None;
    let mut adapter = None;
    let mut low_power = false;
    let mut mesa = None;
    let mut fonts = Fonts::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--session" => session = Some(args.next().ok_or("--session needs a value")?),
            "--adapter" => adapter = Some(args.next().ok_or("--adapter needs a value")?),
            "--low-power" => low_power = true,
            "--mesa" | "--mesa-fallback" => {
                let folder = args.next().ok_or_else(|| format!("{a} needs a folder"))?;
                mesa = Some(Mesa { folder: folder.into(), chosen: a == "--mesa" });
            }
            "--fonts" => fonts = Fonts::from_arg(&args.next().ok_or("--fonts needs a value")?)?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(Args { session: session.ok_or("--session is required")?, adapter, low_power, mesa, fonts })
}

fn main() {
    let code = match parse_args().and_then(run) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("pwrs-gui-egui: {e}");
            1
        }
    };
    std::process::exit(code);
}

fn run(args: Args) -> Result<(), String> {
    let settings = Settings { adapter: args.adapter, low_power: args.low_power, any_thread: false, bring_to_front: true, mesa: args.mesa, fonts: args.fonts };
    pwrs_gui_proto::helper::serve(&args.session, "pwrs-gui-egui", |link| pwrs_gui_egui::run(link, settings))
}
