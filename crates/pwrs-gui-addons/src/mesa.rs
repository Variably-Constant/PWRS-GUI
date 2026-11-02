//! The Mesa add-on, `PWRS-GUI.Mesa.win-x64`: Mesa's llvmpipe, which draws
//! OpenGL on the processor, as `opengl32.dll` and the `libgallium_wgl.dll`
//! it loads, built from Mesa's and LLVM's source with the settings
//! mesa-dist-win builds them with; a module manifest; a `NOTICE` saying
//! what the add-on contains and how it was built; and the license texts of
//! the code in the two libraries, which each build reads file by file (see
//! [`kind`]). None of that code's licenses requires its source.
//!
//! It builds on Windows x64 with Visual Studio 2022's C++ build tools at
//! MSVC 14.44 and a Windows SDK of 10.0.20348 or later, Python 3.10 or
//! later, and Windows' own curl.exe, certutil.exe and tar.exe. The rest is
//! fetched into the work folder: the sources and Win flex-bison, each
//! checked against its SHA-256, and Mesa's Python build tools at fixed
//! versions in a virtual environment of their own. A work folder left from
//! an earlier build is built on: what is there already is not fetched,
//! unpacked or built again.

use crate::manifest::guid;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The add-on's version, which is also its folder name and module version:
/// Mesa's, and a fourth part counting the add-on's builds of that Mesa.
const VERSION: &str = "26.2.4.0";

/// The add-on's module name.
const NAME: &str = "PWRS-GUI.Mesa.win-x64";

/// The MSVC toolset the libraries are built with.
const MSVC: &str = "14.44";

const MESA_VERSION: &str = "26.2.4";
const LLVM_VERSION: &str = "23.1.3";

/// A file fetched into the work folder and checked against its SHA-256.
struct Fetched {
    file: &'static str,
    url: &'static str,
    sha256: &'static str,
}

const MESA: Fetched = Fetched {
    file: "mesa-26.2.4.tar.xz",
    url: "https://archive.mesa3d.org/mesa-26.2.4.tar.xz",
    sha256: "bce5f7fbebb934373b86c999a064d52fb5065878dc57f287f95346648ec832e9",
};

const LLVM: Fetched = Fetched {
    file: "llvm-project-23.1.3.src.tar.xz",
    url: "https://github.com/llvm/llvm-project/releases/download/llvmorg-23.1.3/llvm-project-23.1.3.src.tar.xz",
    sha256: "c44186a7762ed28954be72e5ff6df9808e0779d4f1bf014ecc4e7e211d31ee34",
};

/// Win flex-bison's release publishes no digest; this one pins the archive
/// the add-on is built with.
const FLEX_BISON: Fetched = Fetched {
    file: "win_flex_bison-2.5.25.zip",
    url: "https://github.com/lexxmark/winflexbison/releases/download/v2.5.25/win_flex_bison-2.5.25.zip",
    sha256: "8d324b62be33604b2c45ad1dd34ab93d722534448f55a16ca7292de32b6ac135",
};

/// Mesa's Python build tools, and CMake for LLVM's build.
const PYTHON_PACKAGES: &[&str] = &["meson==1.12.1", "ninja==1.13.2", "mako==1.4.3", "packaging==26.3", "pyyaml==6.0.3", "cmake==4.4.4"];

/// Of LLVM's source, the folders its build reads: `libc` for its common
/// utility headers.
const LLVM_FOLDERS: &[&str] = &["llvm", "cmake", "third-party", "libc"];

/// LLVM static, release, the X86 target alone, RTTI on, the static C
/// runtime, and no tools, tests or compression libraries.
const LLVM_OPTIONS: &[&str] = &[
    "-DCMAKE_BUILD_TYPE=Release",
    "-DCMAKE_CXX_FLAGS=/utf-8",
    "-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded",
    "-DLLVM_TARGETS_TO_BUILD=X86",
    "-DLLVM_ENABLE_PROJECTS=",
    "-DLLVM_ENABLE_RTTI=ON",
    "-DLLVM_OPTIMIZED_TABLEGEN=ON",
    "-DLLVM_BUILD_TOOLS=OFF",
    "-DLLVM_BUILD_LLVM_C_DYLIB=OFF",
    "-DLLVM_INCLUDE_UTILS=OFF",
    "-DLLVM_INCLUDE_RUNTIMES=OFF",
    "-DLLVM_INCLUDE_TESTS=OFF",
    "-DLLVM_INCLUDE_EXAMPLES=OFF",
    "-DLLVM_INCLUDE_BENCHMARKS=OFF",
    "-DLLVM_INCLUDE_DOCS=OFF",
    "-DLLVM_ENABLE_DIA_SDK=OFF",
    "-DLLVM_ENABLE_ZLIB=OFF",
    "-DLLVM_ENABLE_ZSTD=OFF",
    "-DLLVM_ENABLE_LIBXML2=OFF",
    "-DLLVM_ENABLE_LIBEDIT=OFF",
    "-DLLVM_ENABLE_LIBPFM=OFF",
];

/// Mesa release with LLVM linked in statically, the static C runtime and
/// zlib from Mesa's own wrap, and desktop OpenGL through WGL alone.
const MESA_OPTIONS: &[&str] = &[
    "--backend=ninja",
    "--buildtype=release",
    "-Db_ndebug=true",
    "-Db_vscrt=mt",
    "-Dzlib:default_library=static",
    "-Dcpp_rtti=true",
    "-Dllvm=enabled",
    "-Dshared-llvm=disabled",
    "-Dllvm-orcjit=false",
    "-Dplatforms=windows",
    "-Dgallium-drivers=llvmpipe",
    "-Dvulkan-drivers=",
    "-Dopengl=true",
    "-Dgles1=disabled",
    "-Dgles2=disabled",
    "-Degl=disabled",
    "-Dglx=disabled",
    "-Dgallium-d3d10umd=false",
    "-Dgallium-d3d12-graphics=disabled",
    "-Dgallium-va=disabled",
    "-Dgallium-mediafoundation=disabled",
    "-Dvideo-codecs=",
    "-Dmicrosoft-clc=disabled",
    "-Dspirv-to-dxil=false",
    "-Dzstd=disabled",
    "-Dvalgrind=disabled",
    "-Dlibunwind=disabled",
    "-Dxmlconfig=disabled",
    "-Dexpat=disabled",
    "-Dbuild-tests=false",
];

/// The libraries the add-on ships, as Mesa's build names them.
const LIBRARIES: &[&str] = &["src/gallium/targets/libgl-gdi/opengl32.dll", "src/gallium/targets/wgl/libgallium_wgl.dll"];

/// Builds the add-on into `out`, with what the build fetches and makes
/// stored in `work`.
pub fn build(work: &Path, out: &Path) -> Result<(), String> {
    if !cfg!(all(windows, target_arch = "x86_64")) {
        return Err(format!("the Mesa add-on is built on Windows x64, and this is {} on {}", std::env::consts::OS, std::env::consts::ARCH));
    }
    let addon = out.join(NAME).join(VERSION);
    if addon.exists() {
        return Err(format!("{} exists; give another output folder", addon.display()));
    }
    std::fs::create_dir_all(work).map_err(|e| format!("{} cannot be made: {e}", work.display()))?;
    let work = std::path::absolute(work).map_err(|e| format!("{}: {e}", work.display()))?;
    let downloads = work.join("downloads");
    std::fs::create_dir_all(&downloads).map_err(|e| format!("{} cannot be made: {e}", downloads.display()))?;

    let mesa_source = work.join(format!("mesa-{MESA_VERSION}"));
    if !mesa_source.is_dir() {
        unpack(&fetch(&downloads, &MESA)?, &work, &[])?;
    }
    let llvm_source = work.join(format!("llvm-project-{LLVM_VERSION}.src"));
    if !llvm_source.is_dir() {
        let members: Vec<String> = LLVM_FOLDERS.iter().map(|f| format!("llvm-project-{LLVM_VERSION}.src/{f}")).collect();
        unpack(&fetch(&downloads, &LLVM)?, &work, &members)?;
    }
    let flex_bison = work.join("winflexbison");
    if !flex_bison.is_dir() {
        std::fs::create_dir_all(&flex_bison).map_err(|e| format!("{} cannot be made: {e}", flex_bison.display()))?;
        unpack(&fetch(&downloads, &FLEX_BISON)?, &flex_bison, &[])?;
    }

    let venv = work.join("venv");
    let scripts = venv.join("Scripts");
    if !scripts.join("python.exe").is_file() {
        step("Python environment", Command::new("python").args(["-m", "venv"]).arg(&venv))?;
    }
    step(
        "Python tools",
        Command::new(scripts.join("python.exe")).args(["-m", "pip", "install", "--disable-pip-version-check", "--quiet"]).args(PYTHON_PACKAGES),
    )?;
    let environment = environment(&scripts, &flex_bison)?;
    let tool = |name: &str| {
        let mut cmd = Command::new(scripts.join(format!("{name}.exe")));
        cmd.env_clear().envs(environment.iter().map(|(k, v)| (k, v)));
        cmd
    };

    let llvm = work.join(format!("llvm-{LLVM_VERSION}"));
    if !llvm.join("lib").join("cmake").join("llvm").join("LLVMConfig.cmake").is_file() {
        let build = work.join(format!("build-llvm-{LLVM_VERSION}"));
        let mut configure = tool("cmake");
        configure.args(["-G", "Ninja", "-S"]).arg(llvm_source.join("llvm")).arg("-B").arg(&build);
        configure.arg(format!("-DCMAKE_INSTALL_PREFIX={}", llvm.display())).args(LLVM_OPTIONS);
        step("LLVM configure", &mut configure)?;
        step("LLVM build", tool("cmake").arg("--build").arg(&build))?;
        step("LLVM install", tool("cmake").arg("--install").arg(&build))?;
    }

    let mesa_build = work.join(format!("build-mesa-{MESA_VERSION}"));
    let mut configure = tool("meson");
    configure.arg("setup");
    if mesa_build.join("meson-private").join("coredata.dat").is_file() {
        configure.arg("--reconfigure");
    }
    configure.arg(&mesa_build).arg(&mesa_source).arg(format!("--cmake-prefix-path={}", llvm.display())).args(MESA_OPTIONS);
    step("Mesa configure", &mut configure)?;
    step("Mesa build", tool("meson").arg("compile").arg("-C").arg(&mesa_build))?;

    let linked = linked_files(&scripts.join("ninja.exe"), &mesa_build)?;
    let places = Places { mesa: mesa_source.clone(), build: mesa_build.clone(), llvm: llvm.clone() };
    let read = read_licenses(&places, &linked)?;

    std::fs::create_dir_all(addon.join("licenses")).map_err(|e| format!("{} cannot be made: {e}", addon.display()))?;
    for library in LIBRARIES {
        let from = mesa_build.join(library);
        let name = from.file_name().ok_or_else(|| format!("{library} names no file"))?;
        std::fs::copy(&from, addon.join(name)).map_err(|e| format!("{} cannot be copied: {e}", from.display()))?;
    }
    let texts = license_texts(&places, &llvm_source, &read)?;
    for (file, text) in &texts {
        write(&addon.join("licenses").join(file), text)?;
    }
    write(&addon.join(format!("{NAME}.psd1")), &manifest())?;
    write(&addon.join("NOTICE"), &notice(&read, &texts))?;
    println!("{NAME} {VERSION}: {} files read for their licenses, {:.1} MiB", linked.len(), size(&addon)? as f64 / 1048576.0);
    Ok(())
}

/// Fetches `what` into `downloads`, unless a file of its digest is there,
/// and returns its path.
fn fetch(downloads: &Path, what: &Fetched) -> Result<PathBuf, String> {
    let path = downloads.join(what.file);
    if path.is_file() && sha256(&path)? == what.sha256 {
        return Ok(path);
    }
    step(&format!("fetch {}", what.file), Command::new("curl.exe").args(["-sSfL", "--retry", "3", "-o"]).arg(&path).arg(what.url))?;
    let got = sha256(&path)?;
    if got != what.sha256 {
        return Err(format!("{} has SHA-256 {got}, and the build takes {}", path.display(), what.sha256));
    }
    Ok(path)
}

/// The SHA-256 of a file, from certutil.
fn sha256(path: &Path) -> Result<String, String> {
    let out = Command::new("certutil.exe").arg("-hashfile").arg(path).arg("SHA256").output().map_err(|e| format!("certutil cannot run: {e}"))?;
    if !out.status.success() {
        return Err(format!("certutil could not hash {}: {}", path.display(), String::from_utf8_lossy(&out.stdout).trim()));
    }
    hash_of(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| format!("certutil wrote no hash for {}", path.display()))
}

/// The hash in certutil's `-hashfile` output: the line of 64 hex digits,
/// spaces between pairs as older Windows writes them.
fn hash_of(text: &str) -> Option<String> {
    text.lines().map(|l| l.replace(' ', "").to_ascii_lowercase()).find(|l| l.len() == 64 && l.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Unpacks `archive` into `into` with Windows' tar, the whole of it or the
/// `members` named.
fn unpack(archive: &Path, into: &Path, members: &[String]) -> Result<(), String> {
    let tar = Path::new(&std::env::var_os("SystemRoot").ok_or("SystemRoot is not set")?).join("System32").join("tar.exe");
    step(&format!("unpack {}", archive.display()), Command::new(tar).arg("-xf").arg(archive).arg("-C").arg(into).args(members))
}

/// The environment Visual Studio's `vcvars64.bat` sets for MSVC [`MSVC`],
/// with a PATH of `scripts`, Win flex-bison, the MSVC tools and Windows
/// alone, so no MinGW compiler, pkg-config or zlib of another toolchain is
/// found, and CC and CXX naming cl.
fn environment(scripts: &Path, flex_bison: &Path) -> Result<Vec<(String, String)>, String> {
    let installer = Path::new(&std::env::var_os("ProgramFiles(x86)").ok_or("ProgramFiles(x86) is not set")?).join("Microsoft Visual Studio").join("Installer").join("vswhere.exe");
    let found = Command::new(&installer)
        .args(["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"])
        .output()
        .map_err(|e| format!("{} cannot run: {e}; install Visual Studio 2022's C++ build tools", installer.display()))?;
    let studio = String::from_utf8_lossy(&found.stdout).trim().to_string();
    if studio.is_empty() {
        return Err(String::from("vswhere finds no Visual Studio with the x64 C++ tools; install Visual Studio 2022's C++ build tools"));
    }
    let vcvars = Path::new(&studio).join("VC").join("Auxiliary").join("Build").join("vcvars64.bat");
    let out = cmd_line(&format!("\"{}\" -vcvars_ver={MSVC} >nul 2>&1 && set", vcvars.display()))
        .output()
        .map_err(|e| format!("cmd.exe cannot run: {e}"))?;
    let mut vars = parse_set(&String::from_utf8_lossy(&out.stdout));
    let toolset = vars.iter().find(|(k, _)| k.eq_ignore_ascii_case("VCToolsInstallDir")).map(|(_, v)| v.as_str());
    let wanted = format!("\\{MSVC}.");
    if !out.status.success() || !toolset.is_some_and(|t| t.contains(&wanted)) {
        return Err(format!("{} sets no MSVC {MSVC} toolset; install it with Visual Studio's installer", vcvars.display()));
    }
    for (name, value) in vars.iter_mut() {
        if name.eq_ignore_ascii_case("PATH") {
            *value = path_for_build(value, scripts, flex_bison);
        }
    }
    vars.push((String::from("CC"), String::from("cl")));
    vars.push((String::from("CXX"), String::from("cl")));
    Ok(vars)
}

/// cmd.exe running `line` as written: cmd parses its own command line,
/// which the quoting Rust gives a program's arguments would break, and `/s`
/// has it take off just the quotes around `line`.
fn cmd_line(line: &str) -> Command {
    let mut cmd = Command::new("cmd.exe");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.raw_arg(format!("/d /s /c \"{line}\""));
    }
    #[cfg(not(windows))]
    cmd.args(["/d", "/s", "/c", line]);
    cmd
}

/// The variables a `set` with no arguments lists.
fn parse_set(text: &str) -> Vec<(String, String)> {
    text.lines().filter_map(|line| line.split_once('=')).filter(|(k, _)| !k.is_empty()).map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// `path` with `scripts` and `flex_bison` first and no folder of MinGW,
/// MSYS, Cygwin, Strawberry Perl or Git's own Unix tools.
fn path_for_build(path: &str, scripts: &Path, flex_bison: &Path) -> String {
    let foreign = ["strawberry", "mingw", "msys", "cygwin", "\\git\\usr\\bin"];
    let kept = path.split(';').filter(|p| !p.is_empty() && !foreign.iter().any(|f| p.to_ascii_lowercase().contains(f)));
    [scripts.display().to_string(), flex_bison.display().to_string()].into_iter().chain(kept.map(str::to_string)).collect::<Vec<_>>().join(";")
}

/// Runs `cmd` with its output on the console, failing with its exit code.
fn step(what: &str, cmd: &mut Command) -> Result<(), String> {
    println!("== {what}");
    let status = cmd.status().map_err(|e| format!("{what}: {:?} cannot run: {e}", cmd.get_program()))?;
    if status.success() { Ok(()) } else { Err(format!("{what} failed: {status}")) }
}

/// Every file the libraries were built from: the sources their targets take
/// in, from ninja's graph, and the headers each of their objects included,
/// from ninja's dependency log, less the compiler's and the Windows SDK's
/// own, which lie under the Program Files folders.
fn linked_files(ninja: &Path, build: &Path) -> Result<BTreeSet<PathBuf>, String> {
    let ask = |args: &[&str]| -> Result<String, String> {
        let out = Command::new(ninja).arg("-C").arg(build).args(args).output().map_err(|e| format!("{} cannot run: {e}", ninja.display()))?;
        if !out.status.success() {
            return Err(format!("ninja {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let mut args = vec!["-t", "inputs"];
    args.extend(LIBRARIES);
    let inputs = ask(&args)?;
    let full = |p: &str| normal(&build.join(p.trim()));
    let objects: BTreeSet<PathBuf> = inputs.lines().filter(|l| l.trim().ends_with(".obj")).map(full).collect();
    let mut files: BTreeSet<PathBuf> = inputs.lines().filter(|l| is_code(l.trim())).map(full).collect();
    let mut taking = false;
    for line in ask(&["-t", "deps"])?.lines() {
        if !line.is_empty() && !line.starts_with(' ') {
            taking = line.split_once(':').is_some_and(|(object, _)| objects.contains(&full(object)));
        } else if taking && !line.trim().is_empty() {
            files.insert(full(line));
        }
    }
    let mut system = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)"] {
        system.push(std::env::var(var).map_err(|e| format!("{var}: {e}"))?.to_ascii_lowercase());
    }
    files.retain(|f| !system.iter().any(|s| f.to_string_lossy().to_ascii_lowercase().starts_with(s)));
    Ok(files)
}

fn is_code(name: &str) -> bool {
    [".c", ".cc", ".cpp", ".h", ".hpp", ".inc"].iter().any(|e| name.ends_with(e))
}

/// `path` with `.` and `..` resolved, without touching the disk.
fn normal(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Where the code read for its license lies.
struct Places {
    mesa: PathBuf,
    build: PathBuf,
    llvm: PathBuf,
}

/// Where a file lies, which settles the license of a file that names none.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Place {
    /// Mesa's source, whose project license is MIT.
    Mesa,
    /// What Mesa's build generated, from Mesa's own sources and scripts.
    Generated,
    /// The zlib Mesa's build takes from its wrap, all of it under zlib's
    /// license.
    Zlib,
    /// LLVM's installed headers.
    Llvm,
    /// None of these.
    Elsewhere,
}

/// The license a linked file is under, as the add-on records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Mit,
    Llvm,
    Apache,
    SgiB,
    Boost,
    Bsd,
    Zlib,
    /// BLAKE3, under CC0 1.0 or the Apache License 2.0.
    Blake3,
    /// Bison's output, under the GPL 3 or later with Bison's exception.
    Bison,
    /// flex's output, under no conditions.
    Flex,
}

/// The license of a file at `place` whose text, lowercased and its runs of
/// space made one, is `head`; `None` for one this build cannot settle. Generated GLSL is embedded as bytes, so `float64_glsl.h`, from
/// Berkeley SoftFloat, is named here.
fn kind(place: Place, file_name: &str, head: &str) -> Option<Kind> {
    const SPDX: &str = "spdx-license-identifier:";
    if let Some(at) = head.find(SPDX) {
        let expression: Vec<&str> =
            head[at + SPDX.len()..].split_whitespace().take_while(|t| t.chars().all(|c| c.is_ascii_alphanumeric() || "-.+()".contains(c))).collect();
        return expression.join(" ").split(" or ").find_map(|choice| match choice.trim().trim_matches(['(', ')']) {
            "mit" => Some(Kind::Mit),
            "apache-2.0 with llvm-exception" => Some(Kind::Llvm),
            "apache-2.0" => Some(Kind::Apache),
            "bsl-1.0" => Some(Kind::Boost),
            "bsd-2-clause" | "bsd-3-clause" => Some(Kind::Bsd),
            "sgi-b-2.0" => Some(Kind::SgiB),
            "zlib" => Some(Kind::Zlib),
            _ => None,
        });
    }
    if place == Place::Generated && file_name == "float64_glsl.h" {
        return Some(Kind::Bsd);
    }
    if head.contains("a bison parser, made by gnu bison") && head.contains("as a special exception, you may create a larger work") {
        return Some(Kind::Bison);
    }
    if head.contains("a lexical scanner generated by flex") {
        return Some(Kind::Flex);
    }
    if head.contains("general public license") {
        return None;
    }
    let by_text = [
        ("apache license v2.0 with llvm exceptions", Kind::Llvm),
        ("tablegen'erated file", Kind::Llvm),
        ("apache license", Kind::Apache),
        ("sgi free software b license", Kind::SgiB),
        ("sgi free software license b", Kind::SgiB),
        ("boost software license", Kind::Boost),
        ("permission is hereby granted, free of charge", Kind::Mit),
        ("redistribution and use in source and binary forms", Kind::Bsd),
        ("provided 'as-is', without any express or implied warranty", Kind::Zlib),
    ];
    if let Some((_, k)) = by_text.iter().find(|(words, _)| head.contains(words)) {
        return Some(*k);
    }
    match place {
        Place::Zlib => Some(Kind::Zlib),
        Place::Llvm => Some(Kind::Llvm),
        Place::Mesa | Place::Generated if file_name.starts_with("blake3") => Some(Kind::Blake3),
        Place::Mesa | Place::Generated => Some(Kind::Mit),
        Place::Elsewhere => None,
    }
}

fn place_of(places: &Places, file: &Path) -> Place {
    let lower = |p: &Path| p.to_string_lossy().to_ascii_lowercase();
    let at = lower(file);
    if at.starts_with(&lower(&places.mesa.join("subprojects"))) {
        if file.components().any(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase().starts_with("zlib-")) { Place::Zlib } else { Place::Elsewhere }
    } else if at.starts_with(&lower(&places.mesa)) {
        Place::Mesa
    } else if at.starts_with(&lower(&places.build)) {
        Place::Generated
    } else if at.starts_with(&lower(&places.llvm)) {
        Place::Llvm
    } else {
        Place::Elsewhere
    }
}

/// The whole text of `file`, lowercased, its runs of space made one.
fn head_of(file: &Path) -> Result<String, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{} cannot be read: {e}", file.display()))?;
    Ok(String::from_utf8_lossy(&bytes).to_lowercase().split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The name of `file`, which a linked file always has.
fn name_of(file: &Path) -> Result<String, String> {
    file.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| format!("{} names no file", file.display()))
}

/// Each linked file's license, refusing a build with a file under the GPL
/// without Bison's exception, or one no rule settles.
fn read_licenses(places: &Places, linked: &BTreeSet<PathBuf>) -> Result<BTreeMap<Kind, Vec<PathBuf>>, String> {
    let mut read: BTreeMap<Kind, Vec<PathBuf>> = BTreeMap::new();
    let mut unsettled = Vec::new();
    for file in linked {
        match kind(place_of(places, file), &name_of(file)?.to_ascii_lowercase(), &head_of(file)?) {
            Some(k) => read.entry(k).or_default().push(file.clone()),
            None => unsettled.push(file.display().to_string()),
        }
    }
    if !unsettled.is_empty() {
        return Err(format!(
            "the license of {} linked files is not settled: a file under the GPL without Bison's exception, or one this build has no rule for; read each and settle it in mesa.rs: {}",
            unsettled.len(),
            unsettled.join(", ")
        ));
    }
    Ok(read)
}

/// The license texts the add-on includes, by file name: Mesa's and LLVM's,
/// each license whose code is linked, and the notice of each BSD-licensed
/// file, from its opening comment or, for SoftFloat's GLSL, from Mesa's
/// `float64.glsl`.
fn license_texts(places: &Places, llvm_source: &Path, read: &BTreeMap<Kind, Vec<PathBuf>>) -> Result<BTreeMap<String, String>, String> {
    let text = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{} cannot be read: {e}", p.display()));
    let licenses = places.mesa.join("licenses");
    let mut texts = BTreeMap::new();
    texts.insert(String::from("Mesa-MIT.txt"), text(&licenses.join("MIT"))?);
    texts.insert(String::from("LLVM.txt"), text(&llvm_source.join("llvm").join("LICENSE.TXT"))?);
    for (kind, file, from) in [
        (Kind::Apache, "Apache-2.0.txt", "Apache-2.0"),
        (Kind::Blake3, "Apache-2.0.txt", "Apache-2.0"),
        (Kind::SgiB, "SGI-B-2.0.txt", "SGI-B-2.0"),
        (Kind::Boost, "BSL-1.0.txt", "BSL-1.0"),
    ] {
        if read.contains_key(&kind) {
            texts.insert(String::from(file), text(&licenses.join(from))?);
        }
    }
    if let Some(files) = read.get(&Kind::Zlib) {
        let zlib = files.iter().find_map(|f| f.ancestors().find(|a| a.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().starts_with("zlib-"))));
        let folder = zlib.ok_or("zlib's files lie in no zlib folder")?;
        texts.insert(String::from("zlib.txt"), text(&folder.join("LICENSE"))?);
    }
    let mut notices: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in read.get(&Kind::Bsd).into_iter().flatten() {
        let name = name_of(file)?;
        let from = if name.eq_ignore_ascii_case("float64_glsl.h") { places.mesa.join("src").join("compiler").join("glsl").join("float64.glsl") } else { file.clone() };
        let notice = opening_comment(&text(&from)?).ok_or_else(|| format!("{} opens with no comment containing its notice", from.display()))?;
        notices.entry(notice).or_default().push(name);
    }
    for (n, (notice, names)) in notices.into_iter().enumerate() {
        texts.insert(format!("BSD-{}.txt", n + 1), format!("The notice of {}:\n\n{notice}\n", names.join(", ")));
    }
    Ok(texts)
}

/// The first comment of a C source, without its comment marks.
fn opening_comment(source: &str) -> Option<String> {
    let start = source.find("/*")?;
    let end = start + source[start..].find("*/")?;
    let lines: Vec<&str> = source[start + 2..end].lines().map(|l| l.trim().trim_start_matches('*').trim_start()).collect();
    let text = lines.join("\n").trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// The add-on's module manifest: data alone, for Windows PowerShell 5.1 and
/// PowerShell 7.
fn manifest() -> String {
    format!(
        "@{{\n    ModuleVersion = '{VERSION}'\n    GUID = '{}'\n    Author = 'Mark Newton'\n    \
         Description = 'Mesa {MESA_VERSION}''s llvmpipe for PWRS-GUI on Windows x64: OpenGL drawn on the processor, for Windows without a GPU or WARP, as Server Core is.'\n    \
         CompatiblePSEditions = @('Desktop', 'Core')\n    PowerShellVersion = '5.1'\n    \
         PrivateData = @{{ PSData = @{{ Tags = @('PWRS-GUI', 'Mesa', 'OpenGL'); ProjectUri = 'https://github.com/Variably-Constant/PWRS-GUI' }} }}\n}}\n",
        guid(NAME)
    )
}

/// What the add-on contains, how it was built, and the licenses of its code.
fn notice(read: &BTreeMap<Kind, Vec<PathBuf>>, texts: &BTreeMap<String, String>) -> String {
    let count = |k: Kind| read.get(&k).map_or(0, Vec::len);
    let mut out = format!(
        "{NAME} {VERSION}\n\nMesa {MESA_VERSION}'s llvmpipe, which draws OpenGL on the processor: opengl32.dll and libgallium_wgl.dll, with LLVM {LLVM_VERSION} linked in.\n\n\
         Built by PWRS-GUI's pwrs-gui-addons mesa with MSVC {MSVC}, the static C runtime, from\n  {} (SHA-256 {})\n  {} (SHA-256 {})\n\
         LLVM: {}\nMesa: {}\n\n",
        MESA.url,
        MESA.sha256,
        LLVM.url,
        LLVM.sha256,
        LLVM_OPTIONS.join(" "),
        MESA_OPTIONS.join(" ")
    );
    out.push_str("The licenses of the code in the two libraries, read file by file from the build, and the texts in licenses/:\n");
    for (k, says) in [
        (Kind::Mit, "MIT, Mesa's own license (Mesa-MIT.txt)"),
        (Kind::Llvm, "LLVM's, the Apache License 2.0 with LLVM's exceptions (LLVM.txt)"),
        (Kind::Apache, "the Apache License 2.0, Khronos' headers (Apache-2.0.txt)"),
        (Kind::SgiB, "the SGI Free Software License B 2.0 (SGI-B-2.0.txt)"),
        (Kind::Boost, "the Boost Software License 1.0 (BSL-1.0.txt)"),
        (Kind::Bsd, "BSD licenses, each file's notice in BSD-*.txt"),
        (Kind::Zlib, "zlib's (zlib.txt)"),
        (Kind::Blake3, "BLAKE3's, CC0 1.0 or the Apache License 2.0, taken under the latter (Apache-2.0.txt)"),
        (Kind::Bison, "Bison's parser skeleton, under the GPL 3 or later with Bison's exception, which lets a larger work containing it be distributed under terms of its own"),
        (Kind::Flex, "flex's scanner skeleton, which flex places under no conditions"),
    ] {
        match count(k) {
            0 => {}
            1 => out.push_str(&format!("      1 file: {says}\n")),
            n => out.push_str(&format!("  {n:5} files: {says}\n")),
        }
    }
    out.push_str(&format!("\nNo license of this code requires its source. Files in licenses/: {}.\n", texts.keys().cloned().collect::<Vec<_>>().join(", ")));
    out
}

/// The bytes of the regular files under `dir`.
fn size(dir: &Path) -> Result<u64, String> {
    let mut total = 0;
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{} cannot be listed: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{} cannot be listed: {e}", dir.display()))?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        total += if kind.is_dir() { size(&entry.path())? } else { entry.metadata().map_err(|e| e.to_string())?.len() };
    }
    Ok(total)
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{} cannot be made: {e}", parent.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{} cannot be written: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(text: &str) -> String {
        text.to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn each_file_is_settled_by_its_identifier_its_words_or_where_it_lies() {
        let k = |place, name, text: &str| kind(place, name, &head(text));
        assert_eq!(k(Place::Mesa, "a.c", "/* SPDX-License-Identifier: MIT */"), Some(Kind::Mit));
        assert_eq!(k(Place::Mesa, "a.c", "// SPDX-License-Identifier: MIT OR Apache-2.0"), Some(Kind::Mit));
        assert_eq!(k(Place::Mesa, "a.c", "// SPDX-License-Identifier: GPL-2.0-only"), None);
        let llvm = "// Part of the LLVM Project.\n// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception\n//\n//===----------------------------------------------------------------------===//";
        assert_eq!(k(Place::Llvm, "adl.h", llvm), Some(Kind::Llvm));
        let parser = "/* A Bison parser, made by GNU Bison 3.8.2. */ GNU General Public License /* As a special exception, you may create a larger work that contains */";
        assert_eq!(k(Place::Generated, "p.cpp", parser), Some(Kind::Bison));
        assert_eq!(k(Place::Generated, "p.cpp", "/* A Bison parser, made by GNU Bison 3.8.2. */ GNU General Public License"), None);
        assert_eq!(k(Place::Generated, "l.c", "/* A lexical scanner generated by flex */"), Some(Kind::Flex));
        assert_eq!(k(Place::Mesa, "g.c", "under the terms of the GNU General Public License"), None);
        assert_eq!(k(Place::Llvm, "x.h", "Part of the LLVM Project, under the Apache License v2.0 with LLVM Exceptions."), Some(Kind::Llvm));
        assert_eq!(k(Place::Llvm, "x.inc", "#ifdef GET_ATTR_NAMES"), Some(Kind::Llvm));
        assert_eq!(k(Place::Mesa, "vk.h", "Licensed under the Apache License, Version 2.0"), Some(Kind::Apache));
        assert_eq!(k(Place::Mesa, "gl3ext.h", "This document is licensed under the SGI Free Software B License Version 2.0."), Some(Kind::SgiB));
        assert_eq!(k(Place::Mesa, "x.h", "Redistribution and use in source and binary forms, with or without"), Some(Kind::Bsd));
        assert_eq!(k(Place::Generated, "float64_glsl.h", "static const char float64_source[] = { 0x2f, 0x2a"), Some(Kind::Bsd));
        assert_eq!(k(Place::Zlib, "adler32.c", "For conditions of distribution and use, see copyright notice in zlib.h"), Some(Kind::Zlib));
        assert_eq!(k(Place::Mesa, "blake3.c", "#include <assert.h>"), Some(Kind::Blake3));
        assert_eq!(k(Place::Mesa, "box.h", "#ifndef UTIL_BOX_H"), Some(Kind::Mit));
        assert_eq!(k(Place::Zlib, "crc32.h", "/* crc32.h -- tables for rapid CRC calculation * Generated automatically by crc32.c */"), Some(Kind::Zlib));
        assert_eq!(k(Place::Elsewhere, "x.c", "#include <x.h>"), None);
    }

    #[test]
    fn certutil_writes_its_hash_with_or_without_spaces() {
        let new = "SHA256 hash of a.zip:\r\n8d324b62be33604b2c45ad1dd34ab93d722534448f55a16ca7292de32b6ac135\r\nCertUtil: -hashfile command completed successfully.\r\n";
        assert_eq!(hash_of(new).as_deref(), Some("8d324b62be33604b2c45ad1dd34ab93d722534448f55a16ca7292de32b6ac135"));
        let old = "SHA256 hash of file a.zip:\r\n8d 32 4b 62 be 33 60 4b 2c 45 ad 1d d3 4a b9 3d 72 25 34 44 8f 55 a1 6c a7 29 2d e3 2b 6a c1 35\r\n";
        assert_eq!(hash_of(old).as_deref(), Some("8d324b62be33604b2c45ad1dd34ab93d722534448f55a16ca7292de32b6ac135"));
        assert_eq!(hash_of("CertUtil: error"), None);
    }

    #[test]
    fn the_build_path_puts_its_tools_first_and_leaves_other_toolchains_out() {
        let path = path_for_build(r"C:\VS\bin;C:\Strawberry\c\bin;C:\Windows\System32;;C:\Program Files\Git\usr\bin", Path::new(r"C:\w\venv\Scripts"), Path::new(r"C:\w\winflexbison"));
        assert_eq!(path, r"C:\w\venv\Scripts;C:\w\winflexbison;C:\VS\bin;C:\Windows\System32");
        assert_eq!(parse_set("A=1\r\nPath=x;y\r\nnot a variable\r\n=C:=C:\\\r\n"), vec![(String::from("A"), String::from("1")), (String::from("Path"), String::from("x;y"))]);
    }

    #[test]
    fn a_notice_is_the_opening_comment_without_its_marks() {
        assert_eq!(opening_comment("#x\n/*\n * Copyright 1\n *\n * Redistribution and use\n */\nint a;").as_deref(), Some("Copyright 1\n\nRedistribution and use"));
        assert_eq!(opening_comment("int a;"), None);
        let messy: PathBuf = ["w", "build", "..", "mesa", ".", "src", "a.c"].iter().collect();
        let clean: PathBuf = ["w", "mesa", "src", "a.c"].iter().collect();
        assert_eq!(normal(&messy), clean);
    }
}
