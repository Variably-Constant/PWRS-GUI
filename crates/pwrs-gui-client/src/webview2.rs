//! The WebView2 add-on module, `PWRS-GUI.WebView2.win-x64`, which includes
//! WebView2's Fixed Version runtime in its `runtime` folder: the web helper
//! starts on it whenever it is installed, so an HTML window on Windows draws
//! in the one Chromium build the add-on pins.
//!
//! `PWRS_GUI_WEBVIEW2_ADDON` names an add-on folder to use in place of the
//! one found on `PSModulePath`, or says `none` to use the evergreen runtime.

use crate::StartOptions;
use std::path::{Path, PathBuf};

/// The variable that names an add-on folder, or `none`.
pub(crate) const VAR: &str = "PWRS_GUI_WEBVIEW2_ADDON";

/// The add-on's module name.
pub(crate) const MODULE: &str = "PWRS-GUI.WebView2.win-x64";

/// The browser executable that marks an add-on folder, in its runtime.
const BROWSER: &str = "runtime\\msedgewebview2.exe";

/// The runtime the web helper starts on, `None` for the evergreen runtime,
/// and the runtime of every version of the add-on installed, whose data
/// folders a start does not remove, whichever runtime it runs on.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Runtimes {
    pub(crate) fixed: Option<PathBuf>,
    pub(crate) installed: Vec<PathBuf>,
}

/// The runtime folder of the add-on folder `root`.
fn runtime_of(root: &Path) -> PathBuf {
    root.join("runtime")
}

/// The add-on's runtimes, as the environment `options` reads says.
pub(crate) fn find(options: &StartOptions) -> Result<Runtimes, String> {
    locate(crate::addon::text(options, VAR)?.as_deref(), crate::addon::text(options, "PSModulePath")?.as_deref())
}

/// The runtime of the add-on folder `chosen` (the variable's value) names,
/// none when it says `none`, or else of the highest version of the add-on
/// in the first folder of `module_path` that has it; and the runtimes of
/// every version installed anywhere on `module_path`.
fn locate(chosen: Option<&str>, module_path: Option<&str>) -> Result<Runtimes, String> {
    let installed = match module_path {
        Some(path) => crate::addon::all_on_module_path(path, MODULE, BROWSER)?.iter().map(|root| runtime_of(root)).collect(),
        None => Vec::new(),
    };
    let fixed = match (chosen, module_path) {
        (Some(v), _) if v.eq_ignore_ascii_case("none") => None,
        (Some(v), _) if !v.is_empty() => {
            let root = PathBuf::from(v);
            if !root.join(BROWSER).is_file() {
                return Err(format!("{VAR} names {}, which has no {BROWSER}", root.display()));
            }
            Some(runtime_of(&root))
        }
        (_, Some(path)) => crate::addon::on_module_path(path, MODULE, BROWSER)?.map(|root| runtime_of(&root)),
        (_, None) => None,
    };
    Ok(Runtimes { fixed, installed })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Removes `dir`, which need not exist.
    fn clear(dir: &Path) {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("{} cannot be removed: {e}", dir.display()),
        }
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pwrs-gui-webview2-test-{}-{name}", std::process::id()));
        clear(&dir);
        std::fs::create_dir_all(&dir).expect("a test folder");
        dir
    }

    fn add_on(at: &Path) {
        std::fs::create_dir_all(at.join("runtime")).expect("a runtime folder");
        std::fs::write(at.join(BROWSER), "x").expect("a browser executable");
    }

    #[test]
    fn the_variable_chooses_a_folder_or_none_and_the_module_path_finds_the_highest_with_every_one_installed() {
        let chosen = folder("chosen");
        add_on(&chosen);
        let modules = folder("modules");
        let (old, new) = (modules.join(MODULE).join("0.1.0"), modules.join(MODULE).join("0.2.0"));
        add_on(&old);
        add_on(&new);
        let module_path = modules.to_string_lossy().into_owned();
        let found = locate(None, Some(&module_path)).expect("located");
        assert_eq!(found.fixed, Some(new.join("runtime")));
        let mut installed = found.installed.clone();
        installed.sort();
        assert_eq!(installed, vec![old.join("runtime"), new.join("runtime")]);
        let picked = locate(Some(&chosen.to_string_lossy()), Some(&module_path)).expect("located");
        assert_eq!(picked.fixed, Some(chosen.join("runtime")));
        let evergreen = locate(Some("None"), Some(&module_path)).expect("located");
        assert_eq!(evergreen.fixed, None);
        assert_eq!(evergreen.installed.len(), 2, "the installed runtimes are named whichever runtime is used");
        assert_eq!(locate(None, None).expect("located"), Runtimes::default());
        let refused = locate(Some(&modules.to_string_lossy()), None).expect_err("a folder without the runtime");
        assert!(refused.contains("has no runtime\\msedgewebview2.exe"), "{refused}");
        clear(&chosen);
        clear(&modules);
    }
}
