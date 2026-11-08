//! The Mesa add-on module, `PWRS-GUI.Mesa.win-x64`, whose `opengl32.dll`
//! is Mesa's llvmpipe, which draws OpenGL on the processor: the renderer
//! the egui helper draws with on Windows without a GPU and without WARP,
//! as Server Core is.
//!
//! `PWRS_GUI_MESA_ADDON` naming a folder with `opengl32.dll` in it makes
//! the egui renderer draw OpenGL through it on any machine, and `none`
//! leaves OpenGL to the system. Otherwise the add-on on `PSModulePath`
//! is given to the renderer, which draws through it where System32 has no
//! `opengl32.dll`.

use crate::StartOptions;
use std::path::PathBuf;

/// The variable that names an add-on folder, or `none`.
pub(crate) const VAR: &str = "PWRS_GUI_MESA_ADDON";

/// The add-on's module name.
pub(crate) const MODULE: &str = "PWRS-GUI.Mesa.win-x64";

/// The file that marks an add-on folder.
const OPENGL: &str = "opengl32.dll";

/// An add-on folder, and whether `PWRS_GUI_MESA_ADDON` chose it.
#[derive(Debug, PartialEq)]
pub(crate) struct Mesa {
    pub(crate) folder: PathBuf,
    pub(crate) chosen: bool,
}

/// The add-on the egui renderer is given, as the environment `options`
/// reads says.
pub(crate) fn find(options: &StartOptions) -> Result<Option<Mesa>, String> {
    locate(crate::addon::text(options, VAR)?.as_deref(), crate::addon::text(options, "PSModulePath")?.as_deref())
}

/// The add-on folder `chosen` (the variable's value) names, or else the one
/// `module_path` leads to.
fn locate(chosen: Option<&str>, module_path: Option<&str>) -> Result<Option<Mesa>, String> {
    match chosen {
        Some(v) if v.eq_ignore_ascii_case("none") => return Ok(None),
        Some(v) if !v.is_empty() => {
            let folder = PathBuf::from(v);
            return if folder.join(OPENGL).is_file() {
                Ok(Some(Mesa { folder, chosen: true }))
            } else {
                Err(format!("{VAR} names {}, which has no {OPENGL}", folder.display()))
            };
        }
        _ => {}
    }
    let Some(module_path) = module_path else {
        return Ok(None);
    };
    Ok(crate::addon::on_module_path(module_path, MODULE, OPENGL)?.map(|folder| Mesa { folder, chosen: false }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Removes `dir`, which need not exist.
    fn clear(dir: &Path) {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("{} cannot be removed: {e}", dir.display()),
        }
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pwrs-gui-mesa-test-{}-{name}", std::process::id()));
        clear(&dir);
        std::fs::create_dir_all(&dir).expect("a test folder");
        dir
    }

    #[test]
    fn the_variable_chooses_a_folder_or_none_and_the_module_path_finds_one() {
        let chosen = folder("chosen");
        std::fs::write(chosen.join(OPENGL), "x").expect("an opengl32.dll");
        let modules = folder("modules");
        let version = modules.join(MODULE).join("26.2.4");
        std::fs::create_dir_all(&version).expect("a version folder");
        std::fs::write(version.join(OPENGL), "x").expect("an opengl32.dll");
        let module_path = modules.to_string_lossy().into_owned();

        assert_eq!(locate(Some(&chosen.to_string_lossy()), Some(&module_path)).expect("located"), Some(Mesa { folder: chosen.clone(), chosen: true }));
        assert_eq!(locate(Some("None"), Some(&module_path)).expect("located"), None);
        assert_eq!(locate(None, Some(&module_path)).expect("located"), Some(Mesa { folder: version.clone(), chosen: false }));
        assert_eq!(locate(Some(""), Some(&module_path)).expect("located"), Some(Mesa { folder: version, chosen: false }));
        assert_eq!(locate(None, None).expect("located"), None);
        let refused = locate(Some(&modules.to_string_lossy()), None).expect_err("a folder without opengl32.dll");
        assert!(refused.contains("has no opengl32.dll"), "{refused}");
        clear(&chosen);
        clear(&modules);
    }
}
