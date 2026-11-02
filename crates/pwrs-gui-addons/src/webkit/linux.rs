//! Ubuntu 22.04's packages, through an apt state of its own under the work
//! folder: apt reads only jammy, jammy-updates and jammy-security (main and
//! universe), checked against the Ubuntu archive's keyring, installs
//! nothing, and needs no root. The source of each package whose license
//! requires it comes from the same suites' deb-src entries.

use super::{run, write, Packages};
use crate::license::{self, Verdict};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The key the archive's indexes are signed with.
const KEYRING: &str = "/usr/share/keyrings/ubuntu-archive-keyring.gpg";

/// Each suite the add-on takes packages from, and the server it is on.
const SUITES: &[(&str, &str)] = &[
    ("jammy", "http://archive.ubuntu.com/ubuntu"),
    ("jammy-updates", "http://archive.ubuntu.com/ubuntu"),
    ("jammy-security", "http://security.ubuntu.com/ubuntu"),
];

/// What the add-on installs: WebKitGTK, GStreamer's base and good plugins,
/// GIO's TLS module, and the data its libraries read.
const WANTED: &[&str] = &[
    "libwebkit2gtk-4.1-0",
    "gstreamer1.0-plugins-base",
    "gstreamer1.0-plugins-good",
    "glib-networking",
    "gsettings-desktop-schemas",
    "fonts-dejavu-core",
    "shared-mime-info",
    "hicolor-icon-theme",
    "adwaita-icon-theme",
];

/// The packages whose GStreamer plugins ride in the add-on: GStreamer's own
/// core elements, and its base and good plugins.
const PLUGINS: &[&str] = &["libgstreamer1.0-0", "gstreamer1.0-plugins-base", "gstreamer1.0-plugins-good"];

/// Packages whose notice has been read and settled, in place of what this
/// build would make of it. libffi's and MIT Kerberos's name the GPL only
/// for files their libraries do not contain (libffi's documentation and
/// test suite, Kerberos files offered under a BSD license or the GPL), so
/// their notice is all they require. shared-mime-info's puts the whole
/// package, including the MIME database the add-on ships, under the GPL 2
/// or later, so its source ships. adwaita-icon-theme's offers every icon it
/// installs under CC-BY-SA or the LGPL 3 (its one GPL file is a translation
/// it does not install), and the add-on takes CC-BY-SA, which requires the
/// attribution in its notice and no source.
const SETTLED: &[(&str, Verdict)] = &[
    ("libffi8", Verdict::Notice),
    ("libgssapi-krb5-2", Verdict::Notice),
    ("libk5crypto3", Verdict::Notice),
    ("libkrb5-3", Verdict::Notice),
    ("libkrb5support0", Verdict::Notice),
    ("shared-mime-info", Verdict::Source),
    ("adwaita-icon-theme", Verdict::Notice),
];

/// The staged packages.
pub struct Debs {
    work: PathBuf,
    stage: PathBuf,
    apt: Vec<String>,
    owners: BTreeMap<String, String>,
    /// Each package's downloaded file.
    debs: BTreeMap<String, PathBuf>,
    /// Each package's version, as apt's plan gives it.
    versions: BTreeMap<String, String>,
}

/// Downloads the add-on's packages and their dependency closure into
/// `<work>/debs`, reusing those already there, and extracts each into
/// `stage`.
pub fn stage(work: &Path, stage: &Path) -> Result<Debs, String> {
    let root = work.join("apt");
    prepare(&root)?;
    let apt = options(&root);
    run(Command::new("apt-get").args(&apt).args(["-q", "update"]))?;
    let plan = run(Command::new("apt-get").args(&apt).args(["-s", "--no-install-recommends", "install"]).args(WANTED))?;
    let folder = work.join("debs");
    std::fs::create_dir_all(&folder).map_err(|e| format!("{} cannot be made: {e}", folder.display()))?;
    let mut debs = BTreeMap::new();
    let mut versions = BTreeMap::new();
    let mut missing = Vec::new();
    for line in plan.lines().filter(|l| l.starts_with("Inst ")) {
        let (name, version, arch) = install(line)?;
        let file = folder.join(format!("{name}_{}_{arch}.deb", version.replace(':', "%3a")));
        if !file.exists() {
            missing.push(format!("{name}={version}"));
        }
        debs.insert(name.clone(), file);
        versions.insert(name, version);
    }
    if debs.is_empty() {
        return Err(format!("apt plans to install nothing:\n{plan}"));
    }
    if !missing.is_empty() {
        run(Command::new("apt-get").args(&apt).args(["-q", "download"]).args(&missing).current_dir(&folder))?;
    }
    let mut owners = BTreeMap::new();
    for (name, deb) in &debs {
        if !deb.exists() {
            return Err(format!("apt-get download left no {}", deb.display()));
        }
        run(Command::new("dpkg-deb").arg("-x").arg(deb).arg(stage))?;
        for line in run(Command::new("dpkg-deb").arg("-c").arg(deb))?.lines() {
            if let Some(path) = listed(line) {
                owners.insert(path, name.clone());
            }
        }
    }
    println!("staged {} packages from Ubuntu 22.04", debs.len());
    Ok(Debs { work: work.to_path_buf(), stage: stage.to_path_buf(), apt, owners, debs, versions })
}

/// Makes the apt state: its folders, an empty package status, and the
/// suites' deb and deb-src entries.
fn prepare(root: &Path) -> Result<(), String> {
    if !Path::new(KEYRING).exists() {
        return Err(format!("{KEYRING} is missing; the ubuntu-keyring package contains it, and apt checks the archive with it"));
    }
    for dir in [
        "etc/apt/sources.list.d",
        "etc/apt/preferences.d",
        "etc/apt/apt.conf.d",
        "var/lib/apt/lists/partial",
        "var/cache/apt/archives/partial",
        "var/lib/dpkg",
    ] {
        let path = root.join(dir);
        std::fs::create_dir_all(&path).map_err(|e| format!("{} cannot be made: {e}", path.display()))?;
    }
    write(&root.join("var/lib/dpkg/status"), "")?;
    let mut list = String::new();
    for kind in ["deb", "deb-src"] {
        for (suite, server) in SUITES {
            list.push_str(&format!("{kind} [arch=amd64 signed-by={KEYRING}] {server} {suite} main universe\n"));
        }
    }
    write(&root.join("etc/apt/sources.list"), &list)
}

/// The options that point apt at the state under `root`.
fn options(root: &Path) -> Vec<String> {
    let root = root.display();
    [
        format!("Dir={root}"),
        format!("Dir::State::status={root}/var/lib/dpkg/status"),
        String::from("APT::Architecture=amd64"),
        String::from("APT::Architectures::=amd64"),
        String::from("Debug::NoLocking=1"),
    ]
    .into_iter()
    .flat_map(|o| [String::from("-o"), o])
    .collect()
}

/// The name, version and architecture in one `Inst` line of apt's plan:
/// `Inst libfoo (1.2-3 Ubuntu:22.04/jammy-updates [amd64])`, where a list
/// in brackets after the parentheses names packages the step leaves
/// broken until a later one, and the architecture is the bracket inside.
fn install(line: &str) -> Result<(String, String, String), String> {
    let bad = || format!("apt's plan has a line this cannot read: {line}");
    let rest = line.strip_prefix("Inst ").ok_or_else(bad)?;
    let name = rest.split_whitespace().next().ok_or_else(bad)?;
    let open = rest.find('(').ok_or_else(bad)?;
    let close = rest[open..].find(')').ok_or_else(bad)? + open;
    let inside = &rest[open + 1..close];
    let version = inside.split_whitespace().next().ok_or_else(bad)?;
    let bracket = inside.rfind('[').ok_or_else(bad)?;
    let arch = inside[bracket + 1..].strip_suffix(']').ok_or_else(bad)?;
    if arch.is_empty() {
        return Err(bad());
    }
    Ok((name.to_string(), version.to_string(), arch.to_string()))
}

/// The path a `dpkg-deb -c` line lists, from the root, and none for a
/// folder: the line's sixth field on, without a link's target.
fn listed(line: &str) -> Option<String> {
    let mut rest = line;
    for _ in 0..5 {
        rest = rest.trim_start();
        rest = &rest[rest.find(char::is_whitespace)?..];
    }
    let name = rest.trim_start();
    let name = name.split(" -> ").next()?.split(" link to ").next()?;
    let name = name.strip_prefix('.')?;
    if name.is_empty() || name.ends_with('/') {
        None
    } else {
        Some(name.to_string())
    }
}

/// The source package and version a binary package's `dpkg-deb -f` fields
/// name: its Source field, with the version in parentheses when the source
/// version differs, or else its own name and version.
fn source_of(fields: &str) -> Result<(String, String), String> {
    let field = |name: &str| {
        fields.lines().find_map(|l| l.strip_prefix(name).and_then(|v| v.strip_prefix(": ")).map(str::trim))
    };
    let package = field("Package").ok_or_else(|| format!("a package's fields name no package: {fields}"))?;
    let version = field("Version").ok_or_else(|| format!("{package}'s fields name no version"))?;
    match field("Source") {
        None => Ok((package.to_string(), version.to_string())),
        Some(source) => match source.split_once(" (") {
            Some((name, rest)) => Ok((name.to_string(), rest.trim_end_matches(')').to_string())),
            None => Ok((source.to_string(), version.to_string())),
        },
    }
}

/// A version without its epoch, as Debian's file names write it.
fn without_epoch(version: &str) -> &str {
    match version.split_once(':') {
        Some((_, rest)) => rest,
        None => version,
    }
}

/// The files a source package's .dsc lists.
fn dsc_files(dsc: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(dsc).map_err(|e| format!("{} cannot be read: {e}", dsc.display()))?;
    let mut files = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line.starts_with("Files:") {
            inside = true;
        } else if inside && line.starts_with(' ') {
            let name = line.split_whitespace().nth(2).ok_or_else(|| format!("{} has a Files line this cannot read: {line}", dsc.display()))?;
            files.push(name.to_string());
        } else {
            inside = false;
        }
    }
    if files.is_empty() {
        return Err(format!("{} lists no files", dsc.display()));
    }
    Ok(files)
}

/// The libatomic part of a gcc source package. The orig tarball contains
/// the upstream GCC tarball; from it come the libatomic folder, the config
/// folder of autoconf macros libatomic's configure script reads, and GCC's
/// top-level build files and license texts, packed as one tarball beside
/// the source package's Ubuntu packaging and a README saying what the
/// subset is.
fn libatomic(work: &Path, folder: &Path, dest: &Path, source: &str, version: &str, files: &[String]) -> Result<(), String> {
    let orig = files.iter().find(|f| f.contains(".orig.tar.")).ok_or_else(|| format!("{source} {version} has no orig tarball"))?;
    let packaging: Vec<&String> = files.iter().filter(|f| !f.contains(".orig.tar.")).collect();
    let unpack = work.join("libatomic").join(format!("{source}_{}", without_epoch(version)));
    std::fs::create_dir_all(&unpack).map_err(|e| format!("{} cannot be made: {e}", unpack.display()))?;
    run(Command::new("tar").arg("-xf").arg(folder.join(orig)).arg("-C").arg(&unpack).args(["--wildcards", "*/gcc-[0-9]*.tar.*"]))?;
    let upstream = super::files_named(&unpack, &|name| name.starts_with("gcc-") && name.contains(".tar."))?
        .into_iter()
        .next()
        .ok_or_else(|| format!("{orig} has no upstream GCC tarball"))?;
    let members = run(Command::new("tar").arg("-tf").arg(&upstream))?;
    let top = members.lines().next().and_then(|m| m.split('/').next()).ok_or_else(|| format!("{} is empty", upstream.display()))?;
    let mut chosen = String::new();
    for member in members.lines() {
        let Some(rest) = member.strip_prefix(top).and_then(|r| r.strip_prefix('/')) else {
            continue;
        };
        if rest.starts_with("libatomic/") || rest.starts_with("config/") || (!rest.is_empty() && !rest.contains('/')) {
            chosen.push_str(member);
            chosen.push('\n');
        }
    }
    let list = unpack.join("members.txt");
    write(&list, &chosen)?;
    let tree = unpack.join("tree");
    std::fs::create_dir_all(&tree).map_err(|e| format!("{} cannot be made: {e}", tree.display()))?;
    run(Command::new("tar").arg("-xf").arg(&upstream).arg("-C").arg(&tree).arg("--no-recursion").arg("-T").arg(&list))?;
    let packed = format!("{top}-libatomic.tar.xz");
    run(Command::new("tar")
        .args(["--sort=name", "--owner=0", "--group=0", "--numeric-owner", "-cJf"])
        .arg(dest.join(&packed))
        .arg("-C")
        .arg(&tree)
        .arg(top))?;
    let mut names = Vec::new();
    for name in &packaging {
        copy(&folder.join(name.as_str()), &dest.join(name.as_str()))?;
        names.push(name.as_str());
    }
    let dsc = format!("{source}_{}.dsc", without_epoch(version));
    copy(&folder.join(&dsc), &dest.join(&dsc))?;
    write(
        &dest.join("README.txt"),
        &format!(
            "libatomic1, GCC's runtime for atomic operations, is the only part of the {source} source package\n\
             the add-on includes. {packed} contains the libatomic folder of {top} as the upstream tarball in\n\
             {orig} has it, with GCC's top-level build files, its config folder of autoconf macros, and\n\
             its license texts (COPYING, COPYING.LIB, COPYING3, COPYING3.LIB and COPYING.RUNTIME).\n\
             {} and {dsc} are Ubuntu's packaging of that source: its build rules and patches.\n\
             The whole source is the {source} {version} source package in Ubuntu 22.04's archive.\n",
            names.join(" and ")
        ),
    )
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::copy(from, to).map(drop).map_err(|e| format!("{} cannot be copied to {}: {e}", from.display(), to.display()))
}

impl Packages for Debs {
    fn owners(&self) -> &BTreeMap<String, String> {
        &self.owners
    }

    fn version(&self, package: &str) -> Result<String, String> {
        self.versions.get(package).cloned().ok_or_else(|| format!("{package} is not among the staged packages"))
    }

    fn carries_plugins(&self, package: &str) -> bool {
        PLUGINS.contains(&package)
    }

    fn verdict(&self, package: &str) -> Result<Verdict, String> {
        if let Some((_, settled)) = SETTLED.iter().find(|(read, _)| *read == package) {
            return Ok(*settled);
        }
        let path = self.notice(package)?;
        let text = std::fs::read(&path).map_err(|e| format!("{} cannot be read: {e}", path.display()))?;
        Ok(license::debian(&String::from_utf8_lossy(&text)))
    }

    fn notice(&self, package: &str) -> Result<PathBuf, String> {
        let doc = self.stage.join("usr/share/doc").join(package);
        let meta = std::fs::symlink_metadata(&doc).map_err(|e| format!("{package} has no folder {}: {e}", doc.display()))?;
        let folder = if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&doc).map_err(|e| format!("{} cannot be read: {e}", doc.display()))?;
            if target.is_absolute() {
                let inside = target.strip_prefix("/").map_err(|e| format!("{} links to {}: {e}", doc.display(), target.display()))?;
                self.stage.join(inside)
            } else {
                self.stage.join("usr/share/doc").join(target)
            }
        } else {
            doc
        };
        let path = folder.join("copyright");
        if path.is_file() {
            Ok(path)
        } else {
            Err(format!("{package} has no copyright file at {}", path.display()))
        }
    }

    fn sources(&self, out: &Path, packages: &BTreeSet<String>) -> Result<usize, String> {
        let mut by_source: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        for package in packages {
            let deb = self.debs.get(package).ok_or_else(|| format!("{package} is not among the staged packages"))?;
            let fields = run(Command::new("dpkg-deb").arg("-f").arg(deb).args(["Package", "Version", "Source"]))?;
            by_source.entry(source_of(&fields)?).or_default().push(package.clone());
        }
        let cache = self.work.join("sources");
        let mut list = String::from(
            "Each source package below, from Ubuntu 22.04's archive, is in the folder of its name,\n\
             and built the add-on's packages named after it.\n\n",
        );
        for ((source, version), binaries) in &by_source {
            let folder = cache.join(source);
            let dsc = format!("{source}_{}.dsc", without_epoch(version));
            if !folder.join(&dsc).exists() {
                std::fs::create_dir_all(&folder).map_err(|e| format!("{} cannot be made: {e}", folder.display()))?;
                run(Command::new("apt-get")
                    .args(&self.apt)
                    .args(["-q", "source", "--download-only"])
                    .arg(format!("{source}={version}"))
                    .current_dir(&folder))?;
            }
            let files = dsc_files(&folder.join(&dsc))?;
            let dest = out.join(source);
            std::fs::create_dir_all(&dest).map_err(|e| format!("{} cannot be made: {e}", dest.display()))?;
            if source.starts_with("gcc-") && binaries.iter().all(|b| b == "libatomic1") {
                libatomic(&self.work, &folder, &dest, source, version, &files)?;
                writeln!(list, "{source} {version}, libatomic's part: {}", binaries.join(", ")).map_err(|e| e.to_string())?;
            } else {
                for name in files.iter().chain(std::iter::once(&dsc)) {
                    copy(&folder.join(name), &dest.join(name))?;
                }
                writeln!(list, "{source} {version}: {}", binaries.join(", ")).map_err(|e| e.to_string())?;
            }
        }
        write(&out.join("SOURCES.txt"), &list)?;
        Ok(by_source.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inst_line_gives_name_version_and_architecture() {
        let line = "Inst libwebkit2gtk-4.1-0 (2.50.4-0ubuntu0.22.04.1 Ubuntu:22.04/jammy-updates, Ubuntu:22.04/jammy-security [amd64])";
        assert_eq!(
            install(line).expect("read"),
            (String::from("libwebkit2gtk-4.1-0"), String::from("2.50.4-0ubuntu0.22.04.1"), String::from("amd64"))
        );
        let trailing = "Inst humanity-icon-theme (0.6.16 Ubuntu:22.04/jammy [all]) []";
        assert_eq!(install(trailing).expect("read").2, "all");
        let named = "Inst libgtk-3-0 (3.24.33-1ubuntu2.2 Ubuntu:22.04/jammy-updates [amd64]) [libgtk-3-common ]";
        assert_eq!(install(named).expect("read").2, "amd64");
        assert!(install("Inst broken").is_err());
        assert!(install("Inst empty (1.0 Ubuntu:22.04/jammy [])").is_err());
    }

    #[test]
    fn a_listing_line_gives_its_path_and_a_folder_none() {
        assert_eq!(
            listed("-rw-r--r-- root/root    1234 2022-03-01 12:00 ./usr/lib/x86_64-linux-gnu/libz.so.1.2.11").as_deref(),
            Some("/usr/lib/x86_64-linux-gnu/libz.so.1.2.11")
        );
        assert_eq!(
            listed("lrwxrwxrwx root/root       0 2022-03-01 12:00 ./usr/lib/x86_64-linux-gnu/libz.so.1 -> libz.so.1.2.11").as_deref(),
            Some("/usr/lib/x86_64-linux-gnu/libz.so.1")
        );
        assert_eq!(listed("drwxr-xr-x root/root       0 2022-03-01 12:00 ./usr/share/doc/"), None);
    }

    #[test]
    fn a_source_field_names_the_source_and_its_version() {
        let own = "Package: zlib1g\nVersion: 1:1.2.11.dfsg-2ubuntu9.2\nSource: zlib\n";
        assert_eq!(source_of(own).expect("read"), (String::from("zlib"), String::from("1:1.2.11.dfsg-2ubuntu9.2")));
        let other = "Package: libatomic1\nVersion: 12.3.0-1ubuntu1~22.04.3\nSource: gcc-12 (12.3.0-1ubuntu1~22.04.3)\n";
        assert_eq!(source_of(other).expect("read"), (String::from("gcc-12"), String::from("12.3.0-1ubuntu1~22.04.3")));
        let none = "Package: libfoo1\nVersion: 2.0-1\n";
        assert_eq!(source_of(none).expect("read"), (String::from("libfoo1"), String::from("2.0-1")));
        assert_eq!(without_epoch("1:1.2.11.dfsg-2ubuntu9.2"), "1.2.11.dfsg-2ubuntu9.2");
    }
}
