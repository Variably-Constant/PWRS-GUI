//! Writes a copy of Wgpu's psd_merge.rs into OUT_DIR with its module doc
//! lines left out and the probe's merge_harness.rs after it, so the
//! harness sits in the merge's own module and reaches what is private
//! there, while Wgpu's source holds nothing of the probe's.

fn main() {
    let merge = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR")).join("../../../src/psd_merge.rs");
    let merge = merge.display().to_string();
    let source = std::fs::read_to_string(&merge).unwrap_or_else(|e| panic!("reading {merge}: {e}"));
    let harness = std::fs::read_to_string("src/merge_harness.rs").unwrap_or_else(|e| panic!("reading src/merge_harness.rs: {e}"));
    let body: Vec<&str> = source.lines().filter(|line| !line.starts_with("//!")).collect();
    let out = std::path::Path::new(&std::env::var("OUT_DIR").expect("OUT_DIR")).join("psd_merge.rs");
    std::fs::write(&out, format!("{}\n{harness}", body.join("\n"))).unwrap_or_else(|e| panic!("writing {}: {e}", out.display()));
    println!("cargo:rerun-if-changed={merge}");
    println!("cargo:rerun-if-changed=src/merge_harness.rs");
}
