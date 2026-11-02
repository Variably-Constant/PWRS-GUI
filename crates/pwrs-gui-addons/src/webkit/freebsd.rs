//! FreeBSD's packages, read from the package repository as pkg reads it,
//! with no root and the system's own pkg state left as it is. pkg refuses a
//! database folder its user owns, so this fetches the catalog itself,
//! checks its key against the trusted fingerprints in /usr/share/keys/pkg
//! and its signature (made over the hex SHA-256 of the catalog), resolves
//! dependencies from it, and checks each package against the catalog's sum,
//! a SHA-256 or a digest pkg tags with its kind. The source of each package
//! whose license requires it is its port's folder from the ports tree, at
//! the commit that built the package, and the distribution files that
//! folder's distinfo names, from FreeBSD's distfile cache, each checked
//! against distinfo's SHA-256.

use super::{run, write, Packages};
use crate::license::{self, Verdict};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The repository branch a FreeBSD release's pkg reads by default.
const BRANCH: &str = "quarterly";

/// FreeBSD's cache of every port's distribution files, over plain HTTP as
/// the ports framework fetches it: its HTTPS certificates name other hosts.
/// Each file is checked against its port's distinfo, which comes from the
/// ports tree over HTTPS.
const DISTCACHE: &str = "http://distcache.FreeBSD.org/ports-distfiles";

/// The ports tree, mirrored where git can fetch a commit by its name alone.
const PORTS: &str = "https://github.com/freebsd/freebsd-ports.git";

/// What the add-on installs besides its GStreamer plugins: WebKitGTK,
/// GIO's TLS module, and the data its libraries read.
const WANTED: &[&str] = &[
    "webkit2-gtk_41",
    "glib-networking",
    "gsettings-desktop-schemas",
    "dejavu",
    "shared-mime-info",
    "hicolor-icon-theme",
    "adwaita-icon-theme",
];

/// The packages whose GStreamer plugins ride in the add-on, matching
/// Ubuntu's gstreamer1.0-plugins-base and -good: GStreamer's core elements,
/// the core of base and good, and the plugins FreeBSD packages apart because
/// each needs a library. aalib, libcaca, jack and v4l2 are left out, as on
/// Linux.
const PLUGINS: &[&str] = &[
    "gstreamer1",
    "gstreamer1-plugins",
    "gstreamer1-plugins-cdparanoia",
    "gstreamer1-plugins-libvisual",
    "gstreamer1-plugins-ogg",
    "gstreamer1-plugins-opus",
    "gstreamer1-plugins-theora",
    "gstreamer1-plugins-vorbis",
    "gstreamer1-plugins-good",
    "gstreamer1-plugins-cairo",
    "gstreamer1-plugins-dv",
    "gstreamer1-plugins-flac",
    "gstreamer1-plugins-gdkpixbuf",
    "gstreamer1-plugins-jpeg",
    "gstreamer1-plugins-lame",
    "gstreamer1-plugins-mpg123",
    "gstreamer1-plugins-png",
    "gstreamer1-plugins-pulse",
    "gstreamer1-plugins-shout2",
    "gstreamer1-plugins-soup",
    "gstreamer1-plugins-speex",
    "gstreamer1-plugins-taglib",
    "gstreamer1-plugins-twolame",
    "gstreamer1-plugins-vpx",
    "gstreamer1-plugins-wavpack",
    "gstreamer1-plugins-ximagesrc",
];

/// Packages whose license has been read and settled, in place of what this
/// build would make of their catalog entry: adwaita-icon-theme offers its
/// icons under the LGPL 3 or CC-BY-SA 3.0, and the add-on takes CC-BY-SA,
/// which requires the attribution in its notice and no source.
const SETTLED: &[(&str, Verdict)] = &[("adwaita-icon-theme", Verdict::Notice)];

/// One package as the catalog describes it.
struct Entry {
    version: String,
    origin: String,
    repopath: String,
    sum: String,
    deps: Vec<String>,
    licenses: Vec<String>,
    /// The ports tree commit that last changed the package's port when it
    /// was built.
    port_commit: Option<String>,
}

/// The staged packages and the catalog they came from.
pub struct Pkgs {
    work: PathBuf,
    stage: PathBuf,
    url: String,
    owners: BTreeMap<String, String>,
    catalog: BTreeMap<String, Entry>,
}

/// Fetches the add-on's packages and their dependency closure into
/// `<work>/pkgs`, reusing those already there that match the catalog, and
/// extracts each into `stage`.
pub fn stage(work: &Path, stage: &Path) -> Result<Pkgs, String> {
    let (url, catalog) = catalog(work)?;
    let wanted: Vec<&str> = WANTED.iter().chain(PLUGINS).copied().collect();
    let files = fetch(&url, &catalog, &closure(&catalog, &wanted)?, &work.join("pkgs"))?;
    let mut owners = BTreeMap::new();
    for (name, file) in &files {
        extract(file, stage)?;
        for path in run(Command::new("tar").arg("-tf").arg(file))?.lines() {
            if !path.starts_with('+') && !path.ends_with('/') {
                owners.insert(format!("/{}", path.trim_start_matches('/')), name.clone());
            }
        }
    }
    println!("staged {} packages from {url}", files.len());
    Ok(Pkgs { work: work.to_path_buf(), stage: stage.to_path_buf(), url, owners, catalog })
}

/// Fetches the catalog for this system's ABI, checks its key and signature,
/// and reads it.
fn catalog(work: &Path) -> Result<(String, BTreeMap<String, Entry>), String> {
    let abi = run(Command::new("pkg").args(["config", "ABI"]))?.trim().to_string();
    let url = format!("https://pkg.FreeBSD.org/{abi}/{BRANCH}");
    let repo = work.join("repo");
    std::fs::create_dir_all(&repo).map_err(|e| format!("{} cannot be made: {e}", repo.display()))?;
    let site = repo.join("packagesite.pkg");
    run(Command::new("fetch").arg("-q").arg("-o").arg(&site).arg(format!("{url}/packagesite.pkg")))?;
    run(Command::new("tar").arg("-xf").arg(&site).arg("-C").arg(&repo))?;
    let key = repo.join("packagesite.yaml.pub");
    let fingerprint = sha256(&key)?;
    if !trusted()?.contains(&fingerprint) {
        return Err(format!("the catalog's key at {url} has fingerprint {fingerprint}, which /usr/share/keys/pkg does not trust"));
    }
    let yaml = repo.join("packagesite.yaml");
    let digest = repo.join("packagesite.yaml.sha256");
    write(&digest, &sha256(&yaml)?)?;
    run(Command::new("openssl")
        .args(["dgst", "-sha256", "-verify"])
        .arg(&key)
        .arg("-signature")
        .arg(repo.join("packagesite.yaml.sig"))
        .arg(&digest))?;
    let text = std::fs::read_to_string(&yaml).map_err(|e| format!("{} cannot be read: {e}", yaml.display()))?;
    let mut catalog = BTreeMap::new();
    for line in text.lines() {
        let value: Value = serde_json::from_str(line).map_err(|e| format!("the catalog has a line that is not JSON: {e}"))?;
        let (name, entry) = entry(&value)?;
        catalog.insert(name, entry);
    }
    println!("read {} packages from {url}, its signature checked", catalog.len());
    Ok((url, catalog))
}

/// The fingerprints in the files of `folder`; none when it does not exist.
fn fingerprints(folder: &str) -> Result<BTreeSet<String>, String> {
    let mut found = BTreeSet::new();
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(found),
        Err(e) => return Err(format!("{folder} cannot be listed: {e}")),
    };
    for entry in entries {
        let path = entry.map_err(|e| format!("{folder} cannot be listed: {e}"))?.path();
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{} cannot be read: {e}", path.display()))?;
        for line in text.lines().filter(|l| l.trim_start().starts_with("fingerprint")) {
            let value = line.split('"').nth(1).ok_or_else(|| format!("{} has a fingerprint line this cannot read: {line}", path.display()))?;
            found.insert(value.to_string());
        }
    }
    Ok(found)
}

/// The fingerprints /usr/share/keys/pkg trusts and has not revoked.
fn trusted() -> Result<BTreeSet<String>, String> {
    let revoked = fingerprints("/usr/share/keys/pkg/revoked")?;
    Ok(fingerprints("/usr/share/keys/pkg/trusted")?.difference(&revoked).cloned().collect())
}

/// One catalog line's package. A field the catalog leaves out is empty; one
/// of the wrong shape is an error.
fn entry(value: &Value) -> Result<(String, Entry), String> {
    let name = value["name"].as_str().ok_or("a catalog line has no name")?.to_string();
    let text = |key: &str| -> Result<String, String> {
        value[key].as_str().map(str::to_string).ok_or_else(|| format!("the catalog's {name} has no {key}"))
    };
    let deps = match value.get("deps") {
        None => Vec::new(),
        Some(Value::Object(deps)) => deps.keys().cloned().collect(),
        Some(other) => return Err(format!("the catalog's {name} has deps that are not a table: {other}")),
    };
    let licenses = match value.get("licenses") {
        None => Vec::new(),
        Some(Value::Array(list)) => list
            .iter()
            .map(|l| l.as_str().map(str::to_string).ok_or_else(|| format!("the catalog's {name} has a license that is not a name: {l}")))
            .collect::<Result<Vec<_>, _>>()?,
        Some(other) => return Err(format!("the catalog's {name} has licenses that are not a list: {other}")),
    };
    let port_commit = match value.get("annotations").and_then(|a| a.get("port_git_hash")) {
        None => None,
        Some(Value::String(commit)) => Some(commit.clone()),
        Some(other) => return Err(format!("the catalog's {name} has a port commit that is not text: {other}")),
    };
    let repopath = match value["repopath"].as_str().or(value["path"].as_str()) {
        Some(path) => path.to_string(),
        None => return Err(format!("the catalog's {name} has no repopath")),
    };
    let entry = Entry { version: text("version")?, origin: text("origin")?, repopath, sum: text("sum")?, deps, licenses, port_commit };
    Ok((name, entry))
}

/// `wanted` and every package they depend on.
fn closure(catalog: &BTreeMap<String, Entry>, wanted: &[&str]) -> Result<BTreeSet<String>, String> {
    let mut seen = BTreeSet::new();
    let mut queue: Vec<String> = wanted.iter().map(|w| w.to_string()).collect();
    while let Some(name) = queue.pop() {
        if seen.contains(&name) {
            continue;
        }
        let entry = catalog.get(&name).ok_or_else(|| format!("{name} is not in the catalog"))?;
        queue.extend(entry.deps.iter().cloned());
        seen.insert(name);
    }
    Ok(seen)
}

/// Fetches each package into `folder` unless a file matching the catalog's
/// sum is there, and returns each package's file.
fn fetch(url: &str, catalog: &BTreeMap<String, Entry>, names: &BTreeSet<String>, folder: &Path) -> Result<BTreeMap<String, PathBuf>, String> {
    std::fs::create_dir_all(folder).map_err(|e| format!("{} cannot be made: {e}", folder.display()))?;
    let mut files = BTreeMap::new();
    for name in names {
        let entry = catalog.get(name).ok_or_else(|| format!("{name} is not in the catalog"))?;
        let file = folder.join(format!("{name}-{}.pkg", entry.version));
        if !file.exists() || !matches_sum(&file, &entry.sum)? {
            run(Command::new("fetch").arg("-q").arg("-o").arg(&file).arg(format!("{url}/{}", entry.repopath)))?;
            if !matches_sum(&file, &entry.sum)? {
                return Err(format!("{} does not match the catalog's sum {}", file.display(), entry.sum));
            }
        }
        files.insert(name.clone(), file);
    }
    Ok(files)
}

/// Extracts a package's files, its metadata left out, into `into`.
fn extract(file: &Path, into: &Path) -> Result<(), String> {
    run(Command::new("tar").arg("-xf").arg(file).arg("-C").arg(into).args(["--exclude", "+*"])).map(drop)
}

/// The SHA-256 of a file, from FreeBSD's sha256.
fn sha256(path: &Path) -> Result<String, String> {
    Ok(run(Command::new("sha256").arg("-q").arg(path))?.trim().to_string())
}

/// Whether `file` is the package a catalog sum names. pkg writes a sum as
/// the file's SHA-256 in hex, or tagged with its kind as `<kind>$<digest>`;
/// kind 2 is the file's BLAKE2b-512 digest as [`zbase32`] writes it. A sum
/// of another kind is refused by name.
fn matches_sum(file: &Path, sum: &str) -> Result<bool, String> {
    match sum.split_once('$') {
        None => Ok(sha256(file)? == sum),
        Some(("2", digest)) => Ok(zbase32(&blake2b(file)?) == digest),
        Some((kind, _)) => Err(format!("the catalog's sum for {} is of kind {kind}, which this cannot check", file.display())),
    }
}

/// The BLAKE2b-512 digest of a file, from openssl.
fn blake2b(path: &Path) -> Result<Vec<u8>, String> {
    let printed = run(Command::new("openssl").args(["dgst", "-blake2b512"]).arg(path))?;
    let hex = printed.trim().rsplit_once("= ").map(|(_, hex)| hex).ok_or_else(|| format!("openssl printed no digest of {}: {printed}", path.display()))?;
    hex_bytes(hex).map_err(|why| format!("openssl printed a digest of {} this cannot read: {why}", path.display()))
}

/// The bytes a string of hex digits names.
fn hex_bytes(hex: &str) -> Result<Vec<u8>, String> {
    if !hex.len().is_multiple_of(2) {
        return Err(format!("{hex} has an odd number of digits"));
    }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for i in (0..hex.len()).step_by(2) {
        let pair = hex.get(i..i + 2).ok_or_else(|| format!("{hex} is not hex digits"))?;
        bytes.push(u8::from_str_radix(pair, 16).map_err(|e| format!("{pair} in {hex} is not a hex byte: {e}"))?);
    }
    Ok(bytes)
}

/// A digest as libpkg writes it in a tagged sum: the alphabet
/// `ybndrfg8ejkmcpqxot1uwisza345h769`, least significant bits first, five
/// bytes making eight characters and bits left at the end making one more.
fn zbase32(data: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ybndrfg8ejkmcpqxot1uwisza345h769";
    fn push(out: &mut String, bits: u32) {
        out.push(char::from(ALPHABET[(bits & 0x1F) as usize]));
    }
    let mut out = String::with_capacity(data.len() * 8 / 5 + 1);
    let mut kept: Option<u32> = None;
    for (i, &byte) in data.iter().enumerate() {
        let byte = u32::from(byte);
        let low = kept.unwrap_or(0);
        kept = match i % 5 {
            0 => {
                push(&mut out, byte);
                Some(byte >> 5)
            }
            1 => {
                let x = low | byte << 3;
                push(&mut out, x);
                push(&mut out, x >> 5);
                Some(x >> 10)
            }
            2 => {
                let x = low | byte << 1;
                push(&mut out, x);
                Some(x >> 5)
            }
            3 => {
                let x = low | byte << 4;
                push(&mut out, x);
                push(&mut out, x >> 5);
                Some(x >> 10 & 0x3)
            }
            _ => {
                let x = low | byte << 2;
                push(&mut out, x);
                push(&mut out, x >> 5);
                None
            }
        };
    }
    if let Some(bits) = kept {
        push(&mut out, bits);
    }
    out
}

/// git, from the catalog's git-lite package, run from the work folder with
/// its libraries, programs and templates found there.
struct Git {
    root: PathBuf,
    repo: PathBuf,
}

impl Git {
    /// Stages git-lite into `<work>/git` and makes a partial clone of the
    /// ports tree in `<work>/ports.git` that has commits alone and fetches
    /// the trees and files it reads.
    fn new(pkgs: &Pkgs) -> Result<Git, String> {
        let root = pkgs.work.join("git");
        let files = fetch(&pkgs.url, &pkgs.catalog, &closure(&pkgs.catalog, &["git-lite"])?, &pkgs.work.join("pkgs"))?;
        std::fs::create_dir_all(&root).map_err(|e| format!("{} cannot be made: {e}", root.display()))?;
        for file in files.values() {
            extract(file, &root)?;
        }
        let git = Git { root, repo: pkgs.work.join("ports.git") };
        if !git.repo.exists() {
            run(git.bare().args(["clone", "-q", "--filter=tree:0", "--no-checkout", "--depth=1", PORTS]).arg(&git.repo))?;
        }
        Ok(git)
    }

    /// A git command with the staged git's environment.
    fn bare(&self) -> Command {
        let mut cmd = Command::new(self.root.join("usr/local/bin/git"));
        cmd.env("LD_LIBRARY_PATH", self.root.join("usr/local/lib"))
            .env("GIT_EXEC_PATH", self.root.join("usr/local/libexec/git-core"))
            .env("GIT_TEMPLATE_DIR", self.root.join("usr/local/share/git-core/templates"))
            .env("GIT_SSL_CAINFO", "/etc/ssl/cert.pem")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        cmd
    }

    /// A git command run in the ports clone.
    fn command(&self) -> Command {
        let mut cmd = self.bare();
        cmd.arg("-C").arg(&self.repo);
        cmd
    }
}

/// The port folder a port's Makefile names as its master, from
/// `MASTERDIR= ${.CURDIR}/../../category/port`, as a path in the tree.
fn master(origin: &str, makefile: &str) -> Option<String> {
    let line = makefile.lines().find(|l| l.trim_start().starts_with("MASTERDIR"))?;
    let relative = line.split("${.CURDIR}/").nth(1)?.split_whitespace().next()?;
    let mut parts: Vec<&str> = origin.split('/').collect();
    for piece in relative.split('/') {
        match piece {
            ".." => {
                parts.pop()?;
            }
            "." | "" => {}
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// The distribution files a distinfo names, with each one's SHA-256.
fn distinfo(text: &str) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    for line in text.lines() {
        if let Some((name, sum)) = line.strip_prefix("SHA256 (").and_then(|rest| rest.split_once(") = ")) {
            files.insert(name.to_string(), sum.trim().to_string());
        }
    }
    files
}

impl Pkgs {
    /// The license text of a package whose catalog entry declares no
    /// license: the COPYING or LICENSE file in its documentation folder.
    fn undeclared(&self, package: &str) -> Option<PathBuf> {
        let doc = self.stage.join("usr/local/share/doc").join(package);
        ["COPYING", "COPYING.LIB", "LICENSE", "LICENSE.txt"].iter().map(|f| doc.join(f)).find(|p| p.is_file())
    }

    /// The distribution file `name` in the cache folder `cache`, checked
    /// against `sum`: left from an earlier build, fetched from FreeBSD's
    /// distfile cache, or, for a file the cache lacks, fetched from the
    /// sites the port names, as the ports framework lists them for the
    /// port's `folders` at `commit`.
    fn distfile(&self, git: &Git, cache: &Path, name: &str, sum: &str, commit: &str, folders: &[String]) -> Result<PathBuf, String> {
        let cached = cache.join(name);
        if cached.exists() && sha256(&cached)? == sum {
            return Ok(cached);
        }
        let parent = cached.parent().ok_or("a distfile has no folder")?;
        std::fs::create_dir_all(parent).map_err(|e| format!("{} cannot be made: {e}", parent.display()))?;
        let missed = match get(&format!("{DISTCACHE}/{name}"), &cached, sum) {
            Ok(()) => return Ok(cached),
            Err(why) => why,
        };
        let file = match name.rsplit_once('/') {
            Some((_, file)) => format!("/{file}"),
            None => format!("/{name}"),
        };
        let mut tried = vec![missed];
        for url in sites(git, &self.work, commit, folders)?.into_iter().filter(|u| u.ends_with(&file)) {
            match get(&url, &cached, sum) {
                Ok(()) => {
                    println!("{name} came from {url}, which the port names; FreeBSD's distfile cache lacks it ({})", tried[0]);
                    return Ok(cached);
                }
                Err(why) => tried.push(why),
            }
        }
        Err(format!("{name} could not be fetched: {}", tried.join("; ")))
    }
}

/// Fetches `url` into `dest` and checks it against `sum`, a distinfo's
/// SHA-256.
fn get(url: &str, dest: &Path, sum: &str) -> Result<(), String> {
    run(Command::new("fetch").arg("-q").arg("-o").arg(dest).arg(url))?;
    let got = sha256(dest)?;
    if got == sum {
        Ok(())
    } else {
        Err(format!("{url} gave a file whose SHA-256 is {got}, not its distinfo's {sum}"))
    }
}

/// The URLs the ports framework fetches a port's distribution files from,
/// as `make fetch-urlall-list` prints them in a tree of the framework's
/// `Mk`, `Templates` and `Keywords` and the port's `folders`, the first its
/// own, as they stood at `commit`. The tree stays under the work folder and
/// is added to as other ports need it.
fn sites(git: &Git, work: &Path, commit: &str, folders: &[String]) -> Result<Vec<String>, String> {
    let tree = work.join("ports-tree").join(commit);
    let mut missing: Vec<&str> = ["Mk", "Templates", "Keywords"].into_iter().filter(|f| !tree.join(f).exists()).collect();
    missing.extend(folders.iter().map(String::as_str).filter(|f| !tree.join(f).exists()));
    if !missing.is_empty() {
        std::fs::create_dir_all(&tree).map_err(|e| format!("{} cannot be made: {e}", tree.display()))?;
        let archive = tree.join("added.tar");
        run(git.command().arg("archive").arg("-o").arg(&archive).arg(commit).args(&missing))?;
        run(Command::new("tar").arg("-xf").arg(&archive).arg("-C").arg(&tree))?;
    }
    let port = folders.first().ok_or("a port has no folder")?;
    let listed = run(Command::new("make")
        .arg("-C")
        .arg(tree.join(port))
        .arg("fetch-urlall-list")
        .arg(format!("PORTSDIR={}", tree.display()))
        .env("BATCH", "yes"))?;
    Ok(listed.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect())
}

impl Packages for Pkgs {
    fn owners(&self) -> &BTreeMap<String, String> {
        &self.owners
    }

    fn version(&self, package: &str) -> Result<String, String> {
        self.catalog.get(package).map(|entry| entry.version.clone()).ok_or_else(|| format!("{package} is not in the catalog"))
    }

    fn carries_plugins(&self, package: &str) -> bool {
        PLUGINS.contains(&package)
    }

    fn verdict(&self, package: &str) -> Result<Verdict, String> {
        if let Some((_, settled)) = SETTLED.iter().find(|(read, _)| *read == package) {
            return Ok(*settled);
        }
        let entry = self.catalog.get(package).ok_or_else(|| format!("{package} is not in the catalog"))?;
        match (license::freebsd(&entry.licenses), self.undeclared(package)) {
            (Verdict::Unclear, Some(text)) => {
                let content = std::fs::read(&text).map_err(|e| format!("{} cannot be read: {e}", text.display()))?;
                Ok(license::debian(&String::from_utf8_lossy(&content)))
            }
            (verdict, _) => Ok(verdict),
        }
    }

    fn notice(&self, package: &str) -> Result<PathBuf, String> {
        let entry = self.catalog.get(package).ok_or_else(|| format!("{package} is not in the catalog"))?;
        let folder = self.stage.join("usr/local/share/licenses").join(format!("{package}-{}", entry.version));
        if folder.is_dir() {
            return Ok(folder);
        }
        self.undeclared(package).ok_or_else(|| format!("{package} has no license folder at {} and no license text in its documentation", folder.display()))
    }

    fn sources(&self, out: &Path, packages: &BTreeSet<String>) -> Result<usize, String> {
        let git = Git::new(self)?;
        let mut commits = BTreeSet::new();
        for package in packages {
            let entry = self.catalog.get(package).ok_or_else(|| format!("{package} is not in the catalog"))?;
            commits.insert(entry.port_commit.clone().ok_or_else(|| format!("the catalog names no ports commit for {package}"))?);
        }
        run(git.command().args(["fetch", "-q", "--depth=1", "origin"]).args(&commits))?;
        let ports = out.join("ports");
        let distfiles = out.join("distfiles");
        let cache = self.work.join("distfiles");
        std::fs::create_dir_all(&ports).map_err(|e| format!("{} cannot be made: {e}", ports.display()))?;
        let mut list = String::from(
            "Each package below was built from the port whose folder, from the FreeBSD ports tree as it\n\
             stood when the package was built, is the tarball in ports of the package's name, with the\n\
             distribution files that folder's distinfo names, in distfiles.\n\n",
        );
        for package in packages {
            let entry = self.catalog.get(package).ok_or_else(|| format!("{package} is not in the catalog"))?;
            let commit = entry.port_commit.as_deref().ok_or_else(|| format!("the catalog names no ports commit for {package}"))?;
            let makefile = run(git.command().arg("show").arg(format!("{commit}:{}/Makefile", entry.origin)))?;
            let mut folders = vec![entry.origin.clone()];
            if let Some(master) = master(&entry.origin, &makefile) {
                folders.push(master);
            }
            let tarball = ports.join(format!("{package}-{}.tar.gz", entry.version));
            run(git.command().arg("archive").arg("-o").arg(&tarball).arg(commit).args(&folders))?;
            let mut names = Vec::new();
            for folder in &folders {
                let listed = run(git.command().args(["ls-tree", "--name-only"]).arg(format!("{commit}:{folder}")))?;
                if !listed.lines().any(|l| l == "distinfo") {
                    continue;
                }
                let text = run(git.command().arg("show").arg(format!("{commit}:{folder}/distinfo")))?;
                for (name, sum) in distinfo(&text) {
                    let cached = self.distfile(&git, &cache, &name, &sum, commit, &folders)?;
                    let dest = distfiles.join(&name);
                    let parent = dest.parent().ok_or("a distfile has no folder")?;
                    std::fs::create_dir_all(parent).map_err(|e| format!("{} cannot be made: {e}", parent.display()))?;
                    if !dest.exists() {
                        std::fs::copy(&cached, &dest).map_err(|e| format!("{} cannot be copied: {e}", cached.display()))?;
                    }
                    names.push(name);
                }
            }
            let files = if names.is_empty() { String::from("no distribution files") } else { names.join(", ") };
            writeln!(list, "{package} {} (port {}): {files}", entry.version, entry.origin).map_err(|e| e.to_string())?;
        }
        write(&out.join("SOURCES.txt"), &list)?;
        Ok(packages.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slave_port_names_its_master_folder() {
        let makefile = "PORTNAME=\tgstreamer1-plugins-vpx\nMASTERDIR=\t${.CURDIR}/../../multimedia/gstreamer1-plugins\n";
        assert_eq!(master("multimedia/gstreamer1-plugins-vpx", makefile).as_deref(), Some("multimedia/gstreamer1-plugins"));
        assert_eq!(master("www/webkit2-gtk", "PORTNAME=\twebkit2-gtk\n"), None);
    }

    #[test]
    fn distinfo_gives_each_file_and_its_sum() {
        let text = "TIMESTAMP = 1700000000\nSHA256 (gnome/glib-2.86.4.tar.xz) = abc123\nSIZE (gnome/glib-2.86.4.tar.xz) = 42\n";
        let files = distinfo(text);
        assert_eq!(files.get("gnome/glib-2.86.4.tar.xz").map(String::as_str), Some("abc123"));
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn a_catalog_line_gives_its_package() {
        let line = r#"{"name":"webkit2-gtk_41","origin":"www/webkit2-gtk","version":"2.46.6_8","sum":"8955","repopath":"All/Hashed/webkit2-gtk_41-2.46.6_8~89558223e6.pkg","licenselogic":"or","licenses":["BSD2CLAUSE","GPLv2"],"deps":{"cairo":{"origin":"graphics/cairo","version":"1.18.2,3"}},"annotations":{"port_git_hash":"e9e40e40"}}"#;
        let value: Value = serde_json::from_str(line).expect("json");
        let (name, entry) = entry(&value).expect("entry");
        assert_eq!(name, "webkit2-gtk_41");
        assert_eq!(entry.deps, vec![String::from("cairo")]);
        assert_eq!(entry.licenses, vec![String::from("BSD2CLAUSE"), String::from("GPLv2")]);
        assert_eq!(entry.port_commit.as_deref(), Some("e9e40e40"));
        assert_eq!(license::freebsd(&entry.licenses), Verdict::Source);
    }

    #[test]
    fn a_tagged_sum_is_the_blake2b_digest_as_libpkg_encodes_it() {
        // BLAKE2b-512 of the eight bytes PWRS-GUI, and the sum pkg writes for it.
        let digest = hex_bytes("9223460d782e0cd41d625a5b8b514b9105cd87502d20fa24edebb397f818e1dcc6b4ac2090fe4619dbba5e870f36c327b950ea98f50b5d42fddb84ebb6ad53ae").expect("hex");
        assert_eq!(digest.len(), 64);
        assert_eq!(zbase32(&digest), "1hecwgyxqbde7qec44ssaepjtcb4hdnkpbewx1w7m9cxjhddb8zpc41iybr7xdfd5sq7id6bs3ox1hnkk8gm9fwmnk9zpnq7spm8fzn");
    }

    #[test]
    fn a_sum_of_an_unknown_kind_is_refused_by_name() {
        let refused = matches_sum(Path::new("x-1.pkg"), "7$abc").expect_err("kind 7 is not checked");
        assert!(refused.contains("kind 7"), "{refused}");
    }

    #[test]
    fn text_that_is_not_hex_bytes_is_refused_with_why() {
        assert!(hex_bytes("abc").expect_err("odd").contains("odd number"));
        assert!(hex_bytes("zz").expect_err("not hex").contains("not a hex byte"));
    }

    #[test]
    fn a_field_of_the_wrong_shape_is_an_error() {
        let line = r#"{"name":"x","origin":"a/x","version":"1","sum":"0","repopath":"All/x-1.pkg","deps":["not","a","table"]}"#;
        let value: Value = serde_json::from_str(line).expect("json");
        assert!(entry(&value).is_err());
    }
}
