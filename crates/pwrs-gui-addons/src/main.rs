//! Builds PWRS-GUI's add-on modules. `pwrs-gui-addons webkit <work> <out>`
//! builds `PWRS-GUI.WebKit.<rid>` on the Linux or FreeBSD machine it runs
//! on, from that system's packages, into `<out>`, and beside it
//! `PWRS-GUI.WebKit.<rid>.Source` with the corresponding source of every
//! package the add-on includes whose license requires it. `<work>` stores
//! the package catalogs, the downloads and the staging tree, so the
//! machine's own package state is left as it is and no root is needed.
//!
//! `pwrs-gui-addons mesa <work> <out>` builds `PWRS-GUI.Mesa.win-x64` on
//! Windows x64, Mesa's llvmpipe built from source with the license texts of
//! the code in it, into `<out>`; `<work>` stores the downloads, the sources
//! and the builds.
//!
//! `pwrs-gui-addons webview2 <out>` writes `PWRS-GUI.WebView2.win-x64` into
//! `<out>` on any system: a module whose `Install-GuiWebView2Runtime`
//! fetches WebView2's Fixed Version runtime into it.

mod classify;
mod elf;
mod license;
mod manifest;
mod mesa;
mod rewrite;
mod webkit;
mod webview2;

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [what, work, out] if what == "webkit" => webkit::build(Path::new(work), Path::new(out)),
        [what, work, out] if what == "mesa" => mesa::build(Path::new(work), Path::new(out)),
        [what, out] if what == "webview2" => webview2::build(Path::new(out)),
        _ => Err(String::from("usage: pwrs-gui-addons webkit|mesa <work folder> <output folder>, or pwrs-gui-addons webview2 <output folder>")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pwrs-gui-addons: {e}");
            ExitCode::FAILURE
        }
    }
}
