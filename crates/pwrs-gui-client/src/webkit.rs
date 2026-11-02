//! The WebKitGTK add-on module, `PWRS-GUI.WebKit.<rid>`, that a web helper
//! starts from on Linux and FreeBSD whenever it is installed, in place of
//! the system's WebKitGTK. The add-on describes the environment its helper
//! takes in a file named `environment`: one `NAME=value` per line, `{addon}`
//! standing for the add-on's folder, and a `+` before the name putting the
//! value in front of the variable's current one, joined by `:`. The helper
//! runs with the add-on's folder as its working directory, since the
//! WebKitGTK it includes finds its own processes from there.
//!
//! `PWRS_GUI_WEBKIT_ADDON` names an add-on folder to use in place of the
//! one found on `PSModulePath`, or says `none` to use only the system's.

use crate::StartOptions;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The variable that names an add-on folder, or `none`.
pub(crate) const VAR: &str = "PWRS_GUI_WEBKIT_ADDON";

/// The add-on's module name on this platform.
pub(crate) fn module_name() -> &'static str {
    if cfg!(target_os = "freebsd") { "PWRS-GUI.WebKit.freebsd-x64" } else { "PWRS-GUI.WebKit.linux-x64" }
}

/// The add-on this process's web helpers start from, once one was needed.
static IN_USE: OnceLock<Addon> = OnceLock::new();

/// An add-on's folder and the environment it gives its helper.
#[derive(Debug)]
pub(crate) struct Addon {
    root: PathBuf,
    lines: Vec<Line>,
}

#[derive(Debug, PartialEq)]
struct Line {
    name: String,
    value: String,
    prepend: bool,
}

/// The add-on in use, if one was adopted.
pub(crate) fn in_use() -> Option<&'static Addon> {
    IN_USE.get()
}

/// Makes `addon` the one every later web helper of this process starts
/// from, unless another was adopted first, and returns the one in use.
pub(crate) fn adopt(addon: Addon) -> &'static Addon {
    IN_USE.get_or_init(|| addon)
}

/// The add-on to start from, as the environment `options` reads says: the
/// folder `PWRS_GUI_WEBKIT_ADDON` names, none when it says `none`, otherwise
/// the highest version of the add-on module in the first `PSModulePath`
/// folder that has it.
pub(crate) fn find(options: &StartOptions) -> Result<Option<Addon>, String> {
    let chosen = crate::addon::text(options, VAR)?;
    let module_path = crate::addon::text(options, "PSModulePath")?;
    match locate(chosen.as_deref(), module_path.as_deref())? {
        Some(root) => Addon::read(root).map(Some),
        None => Ok(None),
    }
}

/// The add-on folder `chosen` (the variable's value) and `module_path`
/// lead to; with no module path, only `chosen` can name one.
fn locate(chosen: Option<&str>, module_path: Option<&str>) -> Result<Option<PathBuf>, String> {
    match chosen {
        Some(v) if v.eq_ignore_ascii_case("none") => return Ok(None),
        Some(v) if !v.is_empty() => {
            let root = PathBuf::from(v);
            return if root.join("environment").is_file() {
                Ok(Some(root))
            } else {
                Err(format!("{VAR} names {}, which has no environment file", root.display()))
            };
        }
        _ => {}
    }
    match module_path {
        Some(module_path) => crate::addon::on_module_path(module_path, module_name(), "environment"),
        None => Ok(None),
    }
}

impl Addon {
    fn read(root: PathBuf) -> Result<Addon, String> {
        let file = root.join("environment");
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{} cannot be read: {e}", file.display()))?;
        let lines = parse(&text, &root.to_string_lossy()).map_err(|e| format!("{}: {e}", file.display()))?;
        Ok(Addon { root, lines })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Gives `cmd` the add-on's environment and folder. A `+` line goes in
    /// front of the value the helper would otherwise take from the
    /// environment `options` gives it.
    pub(crate) fn apply(&self, cmd: &mut Command, options: &StartOptions) {
        cmd.current_dir(&self.root);
        for line in &self.lines {
            let mut value = OsString::from(&line.value);
            if line.prepend
                && let Some(current) = options.var(&line.name)
                && !current.is_empty()
            {
                value.push(":");
                value.push(current);
            }
            cmd.env(&line.name, value);
        }
    }
}

/// The lines of an environment file, `{addon}` replaced by `root`; blank
/// lines and lines starting with `#` say nothing.
fn parse(text: &str, root: &str) -> Result<Vec<Line>, String> {
    let mut lines = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let raw = raw.trim();
        if raw.is_empty() || raw.starts_with('#') {
            continue;
        }
        let (name, value) = raw.split_once('=').ok_or_else(|| format!("line {} has no '=': {raw}", n + 1))?;
        let (name, prepend) = match name.strip_prefix('+') {
            Some(rest) => (rest, true),
            None => (name, false),
        };
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!("line {} names no variable: {raw}", n + 1));
        }
        lines.push(Line { name: name.to_string(), value: value.replace("{addon}", root), prepend });
    }
    Ok(lines)
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
        let dir = std::env::temp_dir().join(format!("pwrs-gui-webkit-test-{}-{name}", std::process::id()));
        clear(&dir);
        std::fs::create_dir_all(&dir).expect("a test folder");
        dir
    }

    #[test]
    fn lines_set_or_prepend_and_name_the_folder() {
        let parsed = parse("# comment\n+LD_LIBRARY_PATH={addon}/lib\nGSETTINGS_BACKEND=memory\n\n", "/a").expect("parsed");
        assert_eq!(
            parsed,
            vec![
                Line { name: "LD_LIBRARY_PATH".into(), value: "/a/lib".into(), prepend: true },
                Line { name: "GSETTINGS_BACKEND".into(), value: "memory".into(), prepend: false },
            ]
        );
        assert!(parse("NOEQUALS", "/a").is_err());
        assert!(parse("BAD NAME=x", "/a").is_err());
    }

    #[test]
    fn the_first_module_folder_with_the_add_on_wins_and_its_highest_version() {
        let first = folder("first");
        let second = folder("second");
        for (base, version) in [(&first, "0.9.0"), (&first, "0.10.0"), (&second, "9.9.9")] {
            let root = base.join(module_name()).join(version);
            std::fs::create_dir_all(&root).expect("a version folder");
            std::fs::write(root.join("environment"), "A=b\n").expect("an environment file");
        }
        std::fs::create_dir_all(first.join(module_name()).join("1.0.0")).expect("a version folder with no environment");
        let path = format!("/nowhere:{}:{}", first.display(), second.display());
        let found = locate(None, Some(&path)).expect("located").expect("an add-on");
        assert_eq!(found, first.join(module_name()).join("0.10.0"));
        clear(&first);
        clear(&second);
    }

    #[test]
    fn the_variable_names_a_folder_or_none() {
        let root = folder("chosen");
        std::fs::write(root.join("environment"), "A=b\n").expect("an environment file");
        assert_eq!(locate(Some(&root.to_string_lossy()), None).expect("located"), Some(root.clone()));
        assert_eq!(locate(Some("none"), Some(&root.to_string_lossy())).expect("located"), None);
        assert!(locate(Some("/no/such/addon"), None).is_err());
        assert_eq!(locate(None, None).expect("located"), None);
        clear(&root);
    }
}
