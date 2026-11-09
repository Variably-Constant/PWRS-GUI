//! Extracts every Rust and C program the Guide shows into this crate. A
//! Rust program becomes a module of its own; a C program is compiled by
//! the host's C compiler, every warning an error, with its `main` renamed
//! and a `#line` that points its diagnostics at the page. `run` calls one
//! by the page and line it is shown at. A program for another operating
//! system is left out of this build.

#[allow(dead_code)]
#[path = "src/page.rs"]
mod page;

use std::fmt::Write as _;
use std::path::PathBuf;

use page::Surface;

fn main() {
    let manifest = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let content = manifest.join("..").join("..").join("wiki").join("content");
    let include = manifest.join("..").join("subetha-ffi").join("include");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").expect("cargo sets CARGO_CFG_TARGET_OS");
    println!("cargo:rerun-if-changed={}", content.display());
    println!("cargo:rerun-if-changed={}", include.join("subetha.h").display());

    let rust_dir = out.join("rust");
    let c_dir = out.join("c");
    for dir in [&rust_dir, &c_dir] {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("clear {}: {e}", dir.display()),
        }
        std::fs::create_dir_all(dir).unwrap_or_else(|e| panic!("create {}: {e}", dir.display()));
    }

    let pages = page::pages(&content).unwrap_or_else(|e| panic!("{e}"));
    let mut modules = String::new();
    let mut externs = String::new();
    let mut arms = String::new();
    let mut n = 0usize;
    for (rel, path) in &pages {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let blocks = page::blocks(&text).unwrap_or_else(|e| panic!("{rel}: {e}"));
        let programs = page::programs(rel, &blocks).unwrap_or_else(|e| panic!("{e}"));
        for program in programs {
            if program.os.as_deref().is_some_and(|os| os != target_os) {
                continue;
            }
            let at = format!("{rel}:{}", program.line);
            let stem = format!("p{n:03}_{}_{}", slug(rel), program.line);
            if program.surface == Surface::Rust {
                n += 1;
                if !program.source.lines().any(|l| l.starts_with("fn main(")) {
                    panic!("{at}: a Rust program with no `fn main(` at the start of a line");
                }
                if program.source.lines().any(|l| l.starts_with("#![")) {
                    panic!("{at}: a Rust program with an inner attribute, which a module of this crate cannot hold");
                }
                let file = rust_dir.join(format!("{stem}.rs"));
                std::fs::write(&file, &program.source).unwrap_or_else(|e| panic!("write {}: {e}", file.display()));
                writeln!(
                    modules,
                    "/// {at}, {name}\nmod {stem} {{\n    include!(concat!(env!(\"OUT_DIR\"), \"/rust/{stem}.rs\"));\n\n    \
                     pub(super) fn subetha_wiki_entry() -> std::process::ExitCode {{\n        \
                     std::process::Termination::report(main())\n    }}\n}}\n",
                    name = program.filename,
                )
                .expect("writing to a String");
                writeln!(arms, "        ({rel:?}, {}) => Some({stem}::subetha_wiki_entry()),", program.line)
                    .expect("writing to a String");
            } else if program.surface == Surface::C {
                n += 1;
                if !program.source.contains("int main(void)") {
                    panic!("{at}: a C program with no `int main(void)`");
                }
                let file = c_dir.join(format!("{stem}.c"));
                let source = format!("#line {} \"{rel}\"\n{}", program.line + 1, program.source);
                std::fs::write(&file, source).unwrap_or_else(|e| panic!("write {}: {e}", file.display()));
                let symbol = format!("subetha_wiki_{stem}");
                cc::Build::new()
                    .file(&file)
                    .include(&include)
                    .define("main", Some(symbol.as_str()))
                    .warnings(true)
                    .extra_warnings(true)
                    .warnings_into_errors(true)
                    .compile(&symbol);
                writeln!(externs, "    fn {symbol}() -> core::ffi::c_int;").expect("writing to a String");
                writeln!(arms, "        ({rel:?}, {}) => Some(c_exit(unsafe {{ {symbol}() }})),", program.line)
                    .expect("writing to a String");
            }
        }
    }

    let mut generated = modules;
    if externs.is_empty() {
        generated.push('\n');
    } else {
        write!(
            generated,
            "\nunsafe extern \"C\" {{\n{externs}}}\n\n\
             /// A C program's return value as the exit status a process reports.\n\
             fn c_exit(code: core::ffi::c_int) -> std::process::ExitCode {{\n    \
             match u8::try_from(code) {{\n        \
             Ok(0) => std::process::ExitCode::SUCCESS,\n        \
             Ok(status) => std::process::ExitCode::from(status),\n        \
             Err(e) => {{\n            \
             eprintln!(\"the program returned {{code}}, which no exit status carries: {{e}}\");\n            \
             std::process::ExitCode::FAILURE\n        }}\n    }}\n}}\n\n"
        )
        .expect("writing to a String");
    }
    generated.push_str(
        "/// Run the program the page shows at `line` to its end in this process,\n\
         /// or `None` when this build holds no program there.\n",
    );
    if arms.is_empty() {
        generated.push_str("pub fn run(_page: &str, _line: usize) -> Option<std::process::ExitCode> {\n    None\n}\n");
    } else {
        write!(
            generated,
            "pub fn run(page: &str, line: usize) -> Option<std::process::ExitCode> {{\n    \
             match (page, line) {{\n{arms}        _ => None,\n    }}\n}}\n"
        )
        .expect("writing to a String");
    }
    let file = out.join("programs.rs");
    std::fs::write(&file, generated).unwrap_or_else(|e| panic!("write {}: {e}", file.display()));
}

/// `rel` with every character that cannot sit in an identifier made `_`.
fn slug(rel: &str) -> String {
    rel.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).collect()
}
