//! The WebKitGTK add-on: WebKitGTK and every library it needs that the
//! system does not have (see [`crate::classify`]), its processes, GIO's TLS
//! modules, GStreamer's core, base and good plugins with its plugin
//! scanner, the folders its library has compiled in for its processes
//! rewritten to ones relative to the add-on, GSettings schemas, fonts, the
//! MIME database and icon themes, the `environment` file PWRS-GUI's client
//! reads, a module manifest, the license notice of every package it
//! includes, and in a sibling module the corresponding source of every
//! package whose license requires it (see [`crate::license`]).
//!
//! Linux takes Ubuntu 22.04's packages (glibc 2.35, PWRS's Linux floor);
//! FreeBSD takes its own release's. Each file's place in the add-on is its
//! path in the staging tree without the leading `usr/`, the same rule that
//! makes a rewritten folder `././/...` resolve from the add-on's folder.

use crate::classify::{self, Platform};
use crate::license::Verdict;
use crate::manifest::guid;
use crate::{elf, rewrite};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The fourth part of the add-on's version, counting its builds of one
/// WebKitGTK; the first three are that WebKitGTK's own ([`module_version`]).
const BUILD: u32 = 0;

/// GStreamer plugins the add-on leaves out: the ASCII-art video sinks,
/// which bring GPL-only gpm and S-Lang, and JACK's sink, which brings
/// Berkeley DB under Sleepycat's license, none of which WebKit picks; and
/// the camera source, since PWRS-GUI's pages are refused a camera (the web
/// helper grants no media permission), and FreeBSD's libv4l under it takes
/// the whole Linux kernel's source for its headers.
const LEFT_OUT: &[&str] = &["libgstaasink.so", "libgstcacasink.so", "libgstjack.so", "libgstvideo4linux2.so"];

/// What a platform's packages give the build.
pub trait Packages {
    /// Each staged path, from the root (`/usr/lib/...`), and the package
    /// that owns it.
    fn owners(&self) -> &BTreeMap<String, String>;
    /// The staged package's version, as the platform's catalog gives it.
    fn version(&self, package: &str) -> Result<String, String>;
    /// Whether a package's GStreamer plugins ride in the add-on.
    fn carries_plugins(&self, package: &str) -> bool;
    /// What the package's license requires of the add-on.
    fn verdict(&self, package: &str) -> Result<Verdict, String>;
    /// The package's license notice: a file, or a folder of them.
    fn notice(&self, package: &str) -> Result<PathBuf, String>;
    /// Puts the corresponding source of `packages` into `out`, and returns
    /// how many entries its list of sources has.
    fn sources(&self, out: &Path, packages: &BTreeSet<String>) -> Result<usize, String>;
}

/// Builds the add-on and its source module for the system this runs on.
pub fn build(work: &Path, out: &Path) -> Result<(), String> {
    let platform = match std::env::consts::OS {
        "linux" => Platform::Linux,
        "freebsd" => Platform::FreeBsd,
        other => return Err(format!("the WebKitGTK add-on is built on Linux or FreeBSD, and this is {other}")),
    };
    let rid = if platform == Platform::Linux { "linux-x64" } else { "freebsd-x64" };
    let name = format!("PWRS-GUI.WebKit.{rid}");
    let (modules, source_modules) = (out.join(&name), out.join(format!("{name}.Source")));
    for folder in [&modules, &source_modules] {
        if folder.exists() {
            return Err(format!("{} exists; give another output folder", folder.display()));
        }
    }
    std::fs::create_dir_all(work).map_err(|e| format!("{} cannot be made: {e}", work.display()))?;
    let work = work.canonicalize().map_err(|e| format!("{}: {e}", work.display()))?;
    let stage = work.join("stage");
    if stage.exists() {
        return Err(format!("{} exists; each build makes its staging tree afresh, so remove it or give another work folder", stage.display()));
    }
    std::fs::create_dir(&stage).map_err(|e| format!("{} cannot be made: {e}", stage.display()))?;
    let packages: Box<dyn Packages> = match platform {
        Platform::Linux => Box::new(linux::stage(&work, &stage)?),
        Platform::FreeBsd => Box::new(freebsd::stage(&work, &stage)?),
    };
    let library = find(&stage, "libwebkit2gtk-4.1.so.0")?;
    let owner = format!("/{}", rel(&stage, &library)?);
    let webkit = packages.owners().get(&owner).ok_or_else(|| format!("{owner} belongs to none of the staged packages"))?;
    let version = module_version(platform, &packages.version(webkit)?)?;
    let (addon, source) = (modules.join(&version), source_modules.join(&version));
    let processes = folder_of(&find(&stage, "WebKitWebProcess")?)?;
    let bundle = folder_of(&find(&stage, "libwebkit2gtkinjectedbundle.so")?)?;
    let scanner = find(&stage, "gst-plugin-scanner")?;
    let libdir = folder_of(&library)?;
    let roots = roots(&stage, packages.as_ref(), &library, &[&processes, &bundle], &scanner)?;
    let carried = closure(&stage, platform, &roots)?;
    for file in &carried {
        place(&stage, &addon, file)?;
    }
    let placed = addon.join(strip_usr(&rel(&stage, &library)?));
    for prefix in prefixes(&[rel(&stage, &processes)?, rel(&stage, &bundle)?]) {
        let count = rewrite_library(&placed, &prefix)?;
        println!("rewrote {count} occurrences of {prefix}");
    }
    let copied = data(&stage, &addon, platform)?;
    let libdir = strip_usr(&rel(&stage, &libdir)?);
    write(&addon.join("environment"), &environment(&libdir, &strip_usr(&rel(&stage, &scanner)?)))?;
    write(&addon.join(format!("{name}.psd1")), &manifest(&name, rid, &version))?;
    let mut shipped = BTreeSet::new();
    for file in carried.iter().chain(&copied) {
        let path = format!("/{}", rel(&stage, file)?);
        let package = packages.owners().get(&path).ok_or_else(|| format!("{path} belongs to none of the staged packages"))?;
        shipped.insert(package.clone());
    }
    let mut with_source = BTreeSet::new();
    let mut unclear = Vec::new();
    for package in &shipped {
        match packages.verdict(package)? {
            Verdict::Source => {
                with_source.insert(package.clone());
            }
            Verdict::Notice => {}
            Verdict::Unclear => unclear.push(package.as_str()),
        }
    }
    if !unclear.is_empty() {
        return Err(format!(
            "the license notices of {} name the GPL in a form this build cannot settle; read them, and add each to the platform's list of read notices with what it says",
            unclear.join(", ")
        ));
    }
    notices(packages.as_ref(), &addon.join("notices"), &shipped)?;
    let listed = packages.sources(&source, &with_source)?;
    write(&source.join(format!("{name}.Source.psd1")), &source_manifest(&name, rid, &version))?;
    println!(
        "{name}: {} files from {} packages, {:.1} MiB; {name}.Source: {listed} sources for {} packages, {:.1} MiB",
        carried.len() + copied.len(),
        shipped.len(),
        size(&addon)? as f64 / 1048576.0,
        with_source.len(),
        size(&source)? as f64 / 1048576.0
    );
    Ok(())
}

/// The canonical path of the first file or link named `name` in the
/// staging tree, searched breadth-first.
fn find(stage: &Path, name: &str) -> Result<PathBuf, String> {
    let mut queue = std::collections::VecDeque::from([stage.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("{} cannot be listed: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("{} cannot be listed: {e}", dir.display()))?;
            if entry.file_name() == name {
                let path = entry.path();
                return path.canonicalize().map_err(|e| format!("{}: {e}", path.display()));
            }
            if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                queue.push_back(entry.path());
            }
        }
    }
    Err(format!("the staging tree {} has no {name}", stage.display()))
}

fn folder_of(path: &Path) -> Result<PathBuf, String> {
    path.parent().map(Path::to_path_buf).ok_or_else(|| format!("{} has no folder", path.display()))
}

/// The regular files under `dir`, links left out; none when `dir` does not
/// exist.
fn files_under(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{} cannot be listed: {e}", dir.display())),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{} cannot be listed: {e}", dir.display()))?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            found.extend(files_under(&entry.path())?);
        } else if kind.is_file() {
            found.push(entry.path());
        }
    }
    Ok(found)
}

/// The ELF files under `dir`.
fn elves(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    for path in files_under(dir)? {
        if elf::is_elf(&path)? {
            found.push(path);
        }
    }
    Ok(found)
}

/// The files under `dir` whose names `wanted` takes.
fn files_named(dir: &Path, wanted: &dyn Fn(&str) -> bool) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    for path in files_under(dir)? {
        if path.file_name().is_some_and(|n| wanted(&n.to_string_lossy())) {
            found.push(path);
        }
    }
    Ok(found)
}

/// Where the closure starts: WebKitGTK's library, the files in its
/// processes' and injected bundle's folders but MiniBrowser, GIO's TLS
/// module, GnuTLS's or OpenSSL's as glib-networking builds it, its
/// environment-proxy module where glib-networking builds one
/// (FreeBSD's builds none), the plugins of the packages whose plugins ride
/// along but those left out, and GStreamer's plugin scanner.
fn roots(stage: &Path, packages: &dyn Packages, library: &Path, folders: &[&Path], scanner: &Path) -> Result<Vec<PathBuf>, String> {
    let libdir = folder_of(library)?;
    let mut roots = vec![library.to_path_buf()];
    for folder in folders {
        roots.extend(elves(folder)?.into_iter().filter(|p| !p.ends_with("MiniBrowser")));
    }
    let modules = libdir.join("gio/modules");
    let tls: Vec<PathBuf> = ["libgiognutls.so", "libgioopenssl.so"].iter().map(|m| modules.join(m)).filter(|p| p.exists()).collect();
    if tls.is_empty() {
        return Err(format!("{} has neither libgiognutls.so nor libgioopenssl.so; GIO reaches TLS through one of them", modules.display()));
    }
    roots.extend(tls);
    let proxy = modules.join("libgioenvironmentproxy.so");
    if proxy.exists() {
        roots.push(proxy);
    }
    for plugin in elves(&libdir.join("gstreamer-1.0"))? {
        let path = format!("/{}", rel(stage, &plugin)?);
        let owner = packages.owners().get(&path).ok_or_else(|| format!("{path} belongs to none of the staged packages"))?;
        let left_out = plugin.file_name().is_some_and(|n| LEFT_OUT.iter().any(|l| n == *l));
        if packages.carries_plugins(owner) && !left_out {
            roots.push(plugin);
        }
    }
    roots.push(scanner.to_path_buf());
    Ok(roots)
}

/// Every file the roots need, the roots included, outside the system's set.
fn closure(stage: &Path, platform: Platform, roots: &[PathBuf]) -> Result<BTreeSet<PathBuf>, String> {
    let mut seen = BTreeSet::new();
    let mut queue = Vec::new();
    for root in roots {
        queue.push(root.canonicalize().map_err(|e| format!("{}: {e}", root.display()))?);
    }
    let mut missing = BTreeMap::new();
    while let Some(path) = queue.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        let dynamic = elf::read(&path)?;
        for name in dynamic.needed {
            let in_base = Path::new("/lib").join(&name).exists() || Path::new("/usr/lib").join(&name).exists();
            if classify::is_system(&name, platform, in_base) {
                continue;
            }
            match resolve(stage, &name, &dynamic.runpaths, &path)? {
                Some(target) if !seen.contains(&target) => queue.push(target),
                Some(_) => {}
                None => {
                    missing.entry(name).or_insert_with(|| path.clone());
                }
            }
        }
    }
    if let Some((name, user)) = missing.into_iter().next() {
        return Err(format!("{name}, which {} needs, is in none of the staged packages", user.display()));
    }
    Ok(seen)
}

/// Where the loader would find `name` for `user` inside the staging tree.
fn resolve(stage: &Path, name: &str, runpaths: &[String], user: &Path) -> Result<Option<PathBuf>, String> {
    let origin = format!("/{}", rel(stage, &folder_of(user)?)?);
    let mut dirs: Vec<String> = runpaths.iter().map(|r| r.replace("$ORIGIN", &origin)).collect();
    dirs.extend(["/usr/lib/x86_64-linux-gnu", "/lib/x86_64-linux-gnu", "/usr/local/lib", "/usr/lib", "/lib"].map(String::from));
    for dir in dirs {
        let candidate = stage.join(dir.trim_start_matches('/')).join(name);
        if candidate.exists() {
            return candidate.canonicalize().map(Some).map_err(|e| format!("{}: {e}", candidate.display()));
        }
    }
    Ok(None)
}

/// `path` relative to the canonical `stage`, with `/` between its parts.
fn rel(stage: &Path, path: &Path) -> Result<String, String> {
    let relative = path.strip_prefix(stage).map_err(|e| format!("{} is outside the staging tree {}: {e}", path.display(), stage.display()))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

/// A staging path's place in the add-on: without its leading `usr/`.
fn strip_usr(relative: &str) -> String {
    match relative.strip_prefix("usr/") {
        Some(rest) => rest.to_string(),
        None => relative.to_string(),
    }
}

/// Copies one file the add-on ships into it and gives it its soname as a
/// link beside it when the two names differ.
fn place(stage: &Path, addon: &Path, file: &Path) -> Result<(), String> {
    let dest = addon.join(strip_usr(&rel(stage, file)?));
    let parent = folder_of(&dest)?;
    std::fs::create_dir_all(&parent).map_err(|e| format!("{} cannot be made: {e}", parent.display()))?;
    std::fs::copy(file, &dest).map_err(|e| format!("{} cannot be copied: {e}", file.display()))?;
    if let Some(soname) = elf::read(file)?.soname {
        let link = parent.join(&soname);
        let file_name = dest.file_name().ok_or_else(|| format!("{} has no name", dest.display()))?;
        if soname != file_name.to_string_lossy() && !link.exists() {
            symlink(Path::new(file_name), &link)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(target, link).map_err(|e| format!("{} cannot be linked: {e}", link.display()))
}

#[cfg(not(unix))]
fn symlink(_target: &Path, link: &Path) -> Result<(), String> {
    Err(format!("{} cannot be linked: the add-on is built on Linux or FreeBSD", link.display()))
}

/// The absolute forms of the folders, without any inside another, whose
/// rewrite already covers it.
fn prefixes(folders: &[String]) -> Vec<String> {
    let all: BTreeSet<String> = folders.iter().map(|f| format!("/{f}")).collect();
    all.iter().filter(|p| !all.iter().any(|q| q != *p && p.starts_with(&format!("{q}/")))).cloned().collect()
}

/// Rewrites a folder the library has compiled in for its processes or its
/// injected bundle to the same folder relative to the add-on; none found is
/// an error, since the library would then look where the system installs
/// them.
fn rewrite_library(path: &Path, prefix: &str) -> Result<usize, String> {
    let mut data = std::fs::read(path).map_err(|e| format!("{} cannot be read: {e}", path.display()))?;
    let count = rewrite::same_length(&mut data, prefix.as_bytes(), rewrite::relative(prefix)?.as_bytes())?;
    if count == 0 {
        return Err(format!("{} has no {prefix} compiled in to rewrite", path.display()));
    }
    std::fs::write(path, data).map_err(|e| format!("{} cannot be written: {e}", path.display()))?;
    Ok(count)
}

/// The data the libraries read, returning each staged file it copied:
/// GSettings schemas compiled, the DejaVu fonts and a fontconfig file that
/// also takes the system's, the MIME database compiled, and the hicolor
/// and Adwaita icon themes.
fn data(stage: &Path, addon: &Path, platform: Platform) -> Result<Vec<PathBuf>, String> {
    let share = if platform == Platform::Linux { stage.join("usr/share") } else { stage.join("usr/local/share") };
    let mut copied = Vec::new();
    let schemas = addon.join("share/glib-2.0/schemas");
    copied.extend(copy_files(&share.join("glib-2.0/schemas"), &schemas, &|n| n.ends_with(".xml") || n.ends_with(".override"))?);
    run(Command::new("glib-compile-schemas").arg(&schemas))?;
    let fonts = ["fonts/truetype/dejavu", "fonts/dejavu"]
        .iter()
        .map(|f| share.join(f))
        .find(|f| f.is_dir())
        .ok_or_else(|| format!("{} has no DejaVu fonts", share.display()))?;
    copied.extend(copy_tree(&fonts, &addon.join("share/fonts/dejavu"))?);
    write(
        &addon.join("etc/fonts/fonts.conf"),
        "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"urn:fontconfig:fonts.dtd\">\n<fontconfig>\n  \
         <include ignore_missing=\"yes\">/etc/fonts/fonts.conf</include>\n  \
         <include ignore_missing=\"yes\">/usr/local/etc/fonts/fonts.conf</include>\n  \
         <dir prefix=\"relative\">../../share/fonts</dir>\n  \
         <cachedir prefix=\"xdg\">pwrs-gui-webkit-fontconfig</cachedir>\n</fontconfig>\n",
    )?;
    let mime = addon.join("share/mime");
    copied.extend(copy_files(&share.join("mime/packages"), &mime.join("packages"), &|n| n.ends_with(".xml"))?);
    run(Command::new("update-mime-database").arg(&mime))?;
    for theme in ["hicolor", "Adwaita"] {
        copied.extend(copy_tree(&share.join("icons").join(theme), &addon.join("share/icons").join(theme))?);
    }
    Ok(copied)
}

/// The `environment` file: the add-on's libraries first on the loader's
/// path, its GIO modules, schemas, fonts, data, GStreamer plugins and
/// plugin scanner.
fn environment(libdir: &str, scanner: &str) -> String {
    format!(
        "# The environment PWRS-GUI's web helper takes when it starts from this\n\
         # add-on: NAME=value, {{addon}} for this folder, + to put the value first.\n\
         +LD_LIBRARY_PATH={{addon}}/{libdir}\n\
         GIO_MODULE_DIR={{addon}}/{libdir}/gio/modules\n\
         GSETTINGS_SCHEMA_DIR={{addon}}/share/glib-2.0/schemas\n\
         GSETTINGS_BACKEND=memory\n\
         FONTCONFIG_FILE={{addon}}/etc/fonts/fonts.conf\n\
         +XDG_DATA_DIRS={{addon}}/share:/usr/local/share:/usr/share\n\
         GST_PLUGIN_SYSTEM_PATH_1_0={{addon}}/{libdir}/gstreamer-1.0\n\
         GST_PLUGIN_SCANNER_1_0={{addon}}/{scanner}\n"
    )
}

/// The add-on's version: the upstream version of the WebKitGTK package it
/// includes, as the platform's catalog gives it less the distribution's own
/// parts (Debian's epoch and revision, a FreeBSD port's revision and
/// epoch), and [`BUILD`].
fn module_version(platform: Platform, package_version: &str) -> Result<String, String> {
    let upstream = match platform {
        Platform::Linux => {
            let no_epoch = match package_version.split_once(':') {
                Some((_, rest)) => rest,
                None => package_version,
            };
            match no_epoch.rsplit_once('-') {
                Some((upstream, _revision)) => upstream,
                None => no_epoch,
            }
        }
        Platform::FreeBsd => match package_version.find(['_', ',']) {
            Some(at) => &package_version[..at],
            None => package_version,
        },
    };
    let parts: Vec<&str> = upstream.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return Err(format!("WebKitGTK's package version {package_version} gives {upstream}, which is not three numbers, so the add-on's version cannot be made from it"));
    }
    Ok(format!("{upstream}.{BUILD}"))
}

/// The add-on's module manifest: data alone, for PowerShell 7.4 and later.
fn manifest(name: &str, rid: &str, version: &str) -> String {
    format!(
        "@{{\n    ModuleVersion = '{version}'\n    GUID = '{}'\n    Author = 'Mark Newton'\n    \
         Description = 'WebKitGTK for PWRS-GUI''s HTML windows on {rid}, with every library it needs that the system lacks. Its corresponding source is the module {name}.Source.'\n    \
         CompatiblePSEditions = @('Core')\n    PowerShellVersion = '7.4'\n    \
         PrivateData = @{{ PSData = @{{ Tags = @('PWRS-GUI', 'WebKitGTK'); ProjectUri = 'https://github.com/Variably-Constant/PWRS-GUI' }} }}\n}}\n",
        guid(name)
    )
}

/// The source module's manifest.
fn source_manifest(name: &str, rid: &str, version: &str) -> String {
    format!(
        "@{{\n    ModuleVersion = '{version}'\n    GUID = '{}'\n    Author = 'Mark Newton'\n    \
         Description = 'The corresponding source of the packages in {name}, the WebKitGTK add-on for PWRS-GUI on {rid}, whose licenses require it.'\n    \
         CompatiblePSEditions = @('Core')\n    PowerShellVersion = '7.4'\n}}\n",
        guid(&format!("{name}.Source"))
    )
}

/// Copies each shipped package's license notice into `notices/<package>`.
fn notices(packages: &dyn Packages, notices: &Path, shipped: &BTreeSet<String>) -> Result<(), String> {
    std::fs::create_dir_all(notices).map_err(|e| format!("{} cannot be made: {e}", notices.display()))?;
    for package in shipped {
        let from = packages.notice(package)?;
        if from.is_dir() {
            copy_tree(&from, &notices.join(package))?;
        } else {
            std::fs::copy(&from, notices.join(package)).map_err(|e| format!("{} cannot be copied: {e}", from.display()))?;
        }
    }
    Ok(())
}

/// Copies the tree at `from` to `to`, links as links, and returns each
/// file and link it copied.
fn copy_tree(from: &Path, to: &Path) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{} cannot be made: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("{} cannot be listed: {e}", from.display()))?;
    let mut copied = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{} cannot be listed: {e}", from.display()))?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let path = entry.path();
        let dest = to.join(entry.file_name());
        if kind.is_dir() {
            copied.extend(copy_tree(&path, &dest)?);
        } else if kind.is_symlink() {
            let target = std::fs::read_link(&path).map_err(|e| format!("{} cannot be read: {e}", path.display()))?;
            symlink(&target, &dest)?;
            copied.push(path);
        } else {
            std::fs::copy(&path, &dest).map_err(|e| format!("{} cannot be copied: {e}", path.display()))?;
            copied.push(path);
        }
    }
    Ok(copied)
}

/// Copies the files directly in `from` whose names `wanted` takes into
/// `to`, and returns each file it copied.
fn copy_files(from: &Path, to: &Path, wanted: &dyn Fn(&str) -> bool) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{} cannot be made: {e}", to.display()))?;
    let entries = std::fs::read_dir(from).map_err(|e| format!("{} cannot be listed: {e}", from.display()))?;
    let mut copied = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("{} cannot be listed: {e}", from.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().map_err(|e| e.to_string())?.is_file() && wanted(&name) {
            std::fs::copy(entry.path(), to.join(&name)).map_err(|e| format!("{} cannot be copied: {e}", entry.path().display()))?;
            copied.push(entry.path());
        }
    }
    Ok(copied)
}

/// The bytes of the regular files under `dir`.
fn size(dir: &Path) -> Result<u64, String> {
    let mut total = 0;
    for path in files_under(dir)? {
        total += std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?.len();
    }
    Ok(total)
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{} cannot be made: {e}", parent.display()))?;
    }
    std::fs::write(path, text).map_err(|e| format!("{} cannot be written: {e}", path.display()))
}

/// Runs `cmd` and returns what it wrote, failing with what it said.
pub fn run(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("{cmd:?} cannot run: {e}"))?;
    if !out.status.success() {
        return Err(format!("{cmd:?} failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

mod freebsd;
mod linux;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_add_on_is_versioned_by_the_webkitgtk_it_carries_and_its_build() {
        let made = |platform, version| module_version(platform, version).expect("a version of three numbers");
        assert_eq!(made(Platform::Linux, "2.50.4-0ubuntu0.22.04.1"), "2.50.4.0");
        assert_eq!(made(Platform::Linux, "1:2.50.4-1"), "2.50.4.0");
        assert_eq!(made(Platform::Linux, "2.50.4"), "2.50.4.0");
        assert_eq!(made(Platform::FreeBsd, "2.46.6_8"), "2.46.6.0");
        assert_eq!(made(Platform::FreeBsd, "2.46.6_8,1"), "2.46.6.0");
        let refused = module_version(Platform::Linux, "2.50.4+dfsg-1").expect_err("+dfsg is no version number");
        assert!(refused.contains("2.50.4+dfsg") && refused.contains("not three numbers"), "{refused}");
    }

    #[test]
    fn a_staged_path_loses_its_usr_in_the_add_on() {
        assert_eq!(strip_usr("usr/lib/x86_64-linux-gnu/libgtk-3.so.0"), "lib/x86_64-linux-gnu/libgtk-3.so.0");
        assert_eq!(strip_usr("usr/local/libexec/webkit2gtk-4.1"), "local/libexec/webkit2gtk-4.1");
        assert_eq!(strip_usr("lib/x86_64-linux-gnu/libz.so.1"), "lib/x86_64-linux-gnu/libz.so.1");
    }

    #[test]
    fn a_folder_inside_another_is_rewritten_with_it() {
        let linux = prefixes(&[
            String::from("usr/lib/x86_64-linux-gnu/webkit2gtk-4.1"),
            String::from("usr/lib/x86_64-linux-gnu/webkit2gtk-4.1/injected-bundle"),
        ]);
        assert_eq!(linux, vec![String::from("/usr/lib/x86_64-linux-gnu/webkit2gtk-4.1")]);
        let freebsd = prefixes(&[
            String::from("usr/local/libexec/webkit2gtk-4.1"),
            String::from("usr/local/lib/webkit2gtk-4.1/injected-bundle"),
        ]);
        assert_eq!(freebsd.len(), 2);
    }

    #[test]
    fn the_environment_names_the_library_folder_and_the_scanner() {
        let text = environment("lib/x86_64-linux-gnu", "lib/x86_64-linux-gnu/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner");
        assert!(text.contains("+LD_LIBRARY_PATH={addon}/lib/x86_64-linux-gnu\n"));
        assert!(text.contains("GST_PLUGIN_SYSTEM_PATH_1_0={addon}/lib/x86_64-linux-gnu/gstreamer-1.0\n"));
        assert!(text.contains("GST_PLUGIN_SCANNER_1_0={addon}/lib/x86_64-linux-gnu/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner\n"));
    }

    #[test]
    fn a_module_s_guid_is_its_own_and_lasting() {
        let a = guid("PWRS-GUI.WebKit.linux-x64");
        assert_eq!(a, guid("PWRS-GUI.WebKit.linux-x64"));
        assert_ne!(a, guid("PWRS-GUI.WebKit.freebsd-x64"));
        assert_eq!(a.len(), 36);
        let made = manifest("PWRS-GUI.WebKit.linux-x64", "linux-x64", "2.50.4.0");
        assert!(made.contains(&a) && made.contains("ModuleVersion = '2.50.4.0'"), "{made}");
    }
}
