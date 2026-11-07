//! An add-on module's folder: the highest version folder of the module on
//! `PSModulePath` that has the add-on's marker file.

use crate::StartOptions;
use std::path::{Path, PathBuf};

/// The value of `name` in the environment `options` reads, as text; `None`
/// when it is not set.
pub(crate) fn text(options: &StartOptions, name: &str) -> Result<Option<String>, String> {
    match options.var(name) {
        None => Ok(None),
        Some(v) => v.into_string().map(Some).map_err(|v| format!("{name} is not Unicode: {}", v.to_string_lossy())),
    }
}

/// The folder of the add-on module `name` in the first folder of
/// `module_path`, split as this system splits `PSModulePath`, that has
/// one: the highest version folder with `marker` in it, or the module
/// folder itself when `marker` is in it.
pub(crate) fn on_module_path(module_path: &str, name: &str, marker: &str) -> Result<Option<PathBuf>, String> {
    for folder in std::env::split_paths(module_path).filter(|f| !f.as_os_str().is_empty()) {
        if let Some(root) = newest(&folder.join(name), marker)? {
            return Ok(Some(root));
        }
    }
    Ok(None)
}

/// Every folder of the add-on module `name` with `marker` in it, in every
/// folder of `module_path`: each version folder, and the module folder
/// itself when `marker` is in it.
#[cfg(windows)]
pub(crate) fn all_on_module_path(module_path: &str, name: &str, marker: &str) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    for folder in std::env::split_paths(module_path).filter(|f| !f.as_os_str().is_empty()) {
        let module = folder.join(name);
        let entries = match std::fs::read_dir(&module) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{} cannot be listed: {e}", module.display())),
        };
        for entry in entries {
            let root = entry.map_err(|e| format!("{} cannot be listed: {e}", module.display()))?.path();
            if root.join(marker).is_file() {
                found.push(root);
            }
        }
        if module.join(marker).is_file() {
            found.push(module);
        }
    }
    Ok(found)
}

/// The add-on folder inside one module folder: the highest version folder
/// with `marker` in it, or the module folder itself; `None` when the module
/// folder does not exist or neither has it.
fn newest(module: &Path, marker: &str) -> Result<Option<PathBuf>, String> {
    let entries = match std::fs::read_dir(module) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{} cannot be listed: {e}", module.display())),
    };
    let mut best: Option<(Vec<(usize, String)>, PathBuf)> = None;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{} cannot be listed: {e}", module.display()))?;
        let Some(version) = version_of(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        let root = entry.path();
        if root.join(marker).is_file() && best.as_ref().is_none_or(|(v, _)| version > *v) {
            best = Some((version, root));
        }
    }
    Ok(match best {
        Some((_, root)) => Some(root),
        None => module.join(marker).is_file().then(|| module.to_path_buf()),
    })
}

/// A folder name read as a version, each part as its digits without
/// leading zeros, ordered by length then digits, which orders them as
/// numbers; `None` for a name with a part that is not all digits.
fn version_of(name: &str) -> Option<Vec<(usize, String)>> {
    name.split('.')
        .map(|p| {
            let digits = p.trim_start_matches('0');
            (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).then(|| (digits.len(), digits.to_string()))
        })
        .collect()
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
        let dir = std::env::temp_dir().join(format!("pwrs-gui-addon-test-{}-{name}", std::process::id()));
        clear(&dir);
        std::fs::create_dir_all(&dir).expect("a test folder");
        dir
    }

    #[test]
    fn the_first_module_folder_holding_the_add_on_wins_with_its_highest_version() {
        let first = folder("first");
        let second = folder("second");
        for (base, version) in [(&first, "0.9.0"), (&first, "0.10.0"), (&second, "9.9.9")] {
            let root = base.join("Some.AddOn").join(version);
            std::fs::create_dir_all(&root).expect("a version folder");
            std::fs::write(root.join("marker"), "x").expect("a marker file");
        }
        std::fs::create_dir_all(first.join("Some.AddOn").join("1.0.0")).expect("a version folder with no marker");
        let path = std::env::join_paths([first.parent().expect("a temp folder").join("nowhere"), first.clone(), second.clone()]).expect("a module path");
        let found = on_module_path(&path.to_string_lossy(), "Some.AddOn", "marker").expect("located");
        assert_eq!(found, Some(first.join("Some.AddOn").join("0.10.0")));
        assert_eq!(on_module_path(&path.to_string_lossy(), "Other.AddOn", "marker").expect("located"), None);
        clear(&first);
        clear(&second);
    }

    #[test]
    fn a_module_folder_holding_the_marker_itself_is_the_add_on() {
        let base = folder("flat");
        std::fs::create_dir_all(base.join("Flat.AddOn")).expect("a module folder");
        std::fs::write(base.join("Flat.AddOn").join("marker"), "x").expect("a marker file");
        let found = on_module_path(&base.to_string_lossy(), "Flat.AddOn", "marker").expect("located");
        assert_eq!(found, Some(base.join("Flat.AddOn")));
        clear(&base);
    }

    #[test]
    fn versions_order_as_numbers() {
        assert!(version_of("0.10.0") > version_of("0.9.0"));
        assert!(version_of("10.0") > version_of("9.99"));
        assert_eq!(version_of("26.2.04"), version_of("26.2.4"));
        assert_eq!(version_of("1.x"), None);
        assert_eq!(version_of(""), None);
    }
}
