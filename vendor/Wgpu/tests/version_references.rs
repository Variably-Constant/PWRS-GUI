//! The crate's version appears as a literal where nothing else checks it:
//! in the dependency lines of the README crates.io shows, which crates.io
//! keeps as each version published it, and of the Rust library's wiki page,
//! and in the CHANGELOG's heading for the version. The crates.io package
//! carries the README and the CHANGELOG but not the wiki, so there the
//! wiki's check prints that it was skipped and returns.

use std::path::{Path, PathBuf};

const WIKI_PAGE: &str = "wiki/content/docs/how-to/How-To-Use-The-Rust-Library.md";

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// The version each wgpu-pwsh dependency line in `text` names.
fn versions_named(text: &str) -> Vec<&str> {
    text.match_indices("wgpu-pwsh = { version = \"")
        .filter_map(|(i, m)| text[i + m.len()..].split('"').next())
        .collect()
}

/// Every wgpu-pwsh dependency line of the page at `path` names the version
/// being built, and it has at least one.
fn assert_names_this_version(path: &Path) {
    let version = env!("CARGO_PKG_VERSION");
    let text = read(path);
    let named = versions_named(&text);
    assert!(!named.is_empty(), "{} has no wgpu-pwsh dependency line, so nothing was checked", path.display());
    assert!(named.iter().all(|v| *v == version), "{} names wgpu-pwsh {named:?}, not {version}", path.display());
}

#[test]
fn each_dependency_line_is_read_with_its_version() {
    let text = "a = 1\nwgpu-pwsh = { version = \"9.8.7\", default-features = false }\nwgpu-pwsh = { version = \"0.0.1\" }\n";
    assert_eq!(versions_named(text), ["9.8.7", "0.0.1"]);
    assert!(versions_named("wgpu-pwsh = { git = \"https://example.invalid\" }").is_empty());
}

#[test]
fn the_crates_io_readme_names_the_version_being_built() {
    assert_names_this_version(&root().join("README.crates.md"));
}

#[test]
fn the_rust_library_page_names_the_version_being_built() {
    let page = root().join(WIKI_PAGE);
    if !root().join("wiki").is_dir() {
        println!("skipped: {} is in the repository, not in the crates.io package", WIKI_PAGE);
        return;
    }
    assert_names_this_version(&page);
}

#[test]
fn the_changelog_has_an_entry_for_the_version_being_built() {
    let heading = format!("## {}", env!("CARGO_PKG_VERSION"));
    assert!(read(&root().join("CHANGELOG.md")).lines().any(|line| line == heading), "CHANGELOG.md has no `{heading}` heading");
}
