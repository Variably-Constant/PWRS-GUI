//! Every program the SubEtha Guide shows, run as a reader would run it,
//! and every other block it shows checked for what it is.
//!
//! A block in Rust, Python, PowerShell or C that names a file
//! (```` ```rust {filename="producer/src/main.rs"} ````) is a program. This
//! test runs the Rust and C ones, each as a process of its own: a page's
//! programs for one surface run in page order from one fresh folder, which
//! their temporary directory is pointed at as well, each to its end before
//! the next starts unless it is marked `alongside="true"`, and none for
//! longer than [`DEADLINE`]. A program marked for one operating system
//! (`os="linux"`) runs there only. A program passes when it exits
//! successfully and prints what the Output block after it shows. The
//! Python and PowerShell programs run in the Python package's and the
//! PowerShell module's own suites.
//!
//! Every other block is an Output block; a Rust listing
//! (`listing="true"`), on a reference page only, every line of which is a
//! line of a crate's source; a command, in a `sh`, `bash`, `console` or
//! `toml` block or a `powershell` block of `PS> ` lines, whose SubEtha
//! package names and versions must be the workspace's; or a block in some
//! other language, which is shown only. A Rust, Python, PowerShell or C
//! block that is none of these fails, and so does a fence that names no
//! language, except on a page in [`UNCONVERTED`].
//!
//! `SUBETHA_WIKI_PAGE=<text>` limits a run to the pages whose path holds
//! the text.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use subetha_wiki_tests::page::{self, Block, Program, Surface};

/// How long one program may run.
const DEADLINE: Duration = Duration::from_secs(60);

/// The environment variable that makes this binary play one program, named
/// as `<page>:<line>`.
const PLAY: &str = "SUBETHA_WIKI_PROGRAM";

/// Pages whose blocks are not yet written to these conventions. A block on
/// one of them that is none of the kinds above is counted rather than
/// failed. The list only shrinks: a page on it with no such block fails
/// until it is taken off.
const UNCONVERTED: &[&str] = &[
    "_index.md",
    "docs/explanation/powershell-binding.md",
    "docs/how-to/cross-host-bridge.md",
    "docs/how-to/custom-policy.md",
    "docs/how-to/powershell/bridge-two-hosts.md",
    "docs/how-to/powershell/choose-a-structure.md",
    "docs/how-to/powershell/install.md",
    "docs/how-to/powershell/make-it-fast.md",
    "docs/how-to/powershell/runspaces-and-lifetimes.md",
    "docs/how-to/powershell/troubleshoot.md",
    "docs/how-to/python/bridge-two-hosts.md",
    "docs/how-to/python/choose-a-structure.md",
    "docs/how-to/python/install.md",
    "docs/how-to/python/make-it-fast.md",
    "docs/how-to/python/threads-and-lifetimes.md",
    "docs/how-to/python/troubleshoot.md",
    "docs/how-to/sidecar-box.md",
    "docs/how-to/tune-sidecar.md",
    "docs/how-to/tuning-overrides.md",
    "docs/reference/subetha-core/_index.md",
    "docs/reference/subetha-core/handshake.md",
    "docs/reference/subetha-core/migration.md",
    "docs/reference/subetha-core/observation.md",
    "docs/reference/subetha-core/swap-cell.md",
    "docs/reference/subetha-cxc/_index.md",
    "docs/reference/subetha-cxc/arenas/shared-handle-table.md",
    "docs/reference/subetha-cxc/arenas/shared-region.md",
    "docs/reference/subetha-cxc/arenas/shared-string-arena.md",
    "docs/reference/subetha-cxc/async-engine.md",
    "docs/reference/subetha-cxc/atomics/shared-atomic.md",
    "docs/reference/subetha-cxc/bridges/blocking-tcp-bridge.md",
    "docs/reference/subetha-cxc/bridges/reliable-udp-bridge.md",
    "docs/reference/subetha-cxc/bridges/sens-rlc.md",
    "docs/reference/subetha-cxc/bridges/unified-code-switch.md",
    "docs/reference/subetha-cxc/caches/shared-lru-cache.md",
    "docs/reference/subetha-cxc/cells/shared-cell.md",
    "docs/reference/subetha-cxc/cells/shared-once-cell.md",
    "docs/reference/subetha-cxc/coordination-types/blocking-rw-lock.md",
    "docs/reference/subetha-cxc/coordination-types/blocking-semaphore.md",
    "docs/reference/subetha-cxc/coordination-types/cross-process-waker.md",
    "docs/reference/subetha-cxc/coordination-types/epoch-barrier.md",
    "docs/reference/subetha-cxc/coordination-types/event-state-log.md",
    "docs/reference/subetha-cxc/coordination-types/failover.md",
    "docs/reference/subetha-cxc/coordination-types/heartbeat.md",
    "docs/reference/subetha-cxc/coordination-types/holder-table.md",
    "docs/reference/subetha-cxc/coordination-types/k-tower-cascade.md",
    "docs/reference/subetha-cxc/coordination-types/pass-registry.md",
    "docs/reference/subetha-cxc/coordination-types/priority-fanout.md",
    "docs/reference/subetha-cxc/coordination-types/progress-task.md",
    "docs/reference/subetha-cxc/coordination-types/qos-policy.md",
    "docs/reference/subetha-cxc/coordination-types/scheduler.md",
    "docs/reference/subetha-cxc/coordination-types/shared-async-pointer.md",
    "docs/reference/subetha-cxc/coordination-types/shared-condvar.md",
    "docs/reference/subetha-cxc/coordination-types/shared-epochs.md",
    "docs/reference/subetha-cxc/coordination-types/subscriber-position.md",
    "docs/reference/subetha-cxc/coordination-types/virtual-endpoint.md",
    "docs/reference/subetha-cxc/coordination.md",
    "docs/reference/subetha-cxc/high-level-api.md",
    "docs/reference/subetha-cxc/linux/direct-file-ring.md",
    "docs/reference/subetha-cxc/linux/fd-handoff.md",
    "docs/reference/subetha-cxc/linux/locale-vsock.md",
    "docs/reference/subetha-cxc/locks/shared-fence-clock.md",
    "docs/reference/subetha-cxc/locks/shared-rate-limiter.md",
    "docs/reference/subetha-cxc/locks/shared-rw-lock.md",
    "docs/reference/subetha-cxc/locks/shared-semaphore.md",
    "docs/reference/subetha-cxc/maps/laned-versioned-map.md",
    "docs/reference/subetha-cxc/maps/shared-btree-map.md",
    "docs/reference/subetha-cxc/maps/shared-hash-map.md",
    "docs/reference/subetha-cxc/maps/shared-linked-list.md",
    "docs/reference/subetha-cxc/maps/versioned-btree-map.md",
    "docs/reference/subetha-cxc/ownership-types/lazy-config.md",
    "docs/reference/subetha-cxc/ownership-types/owner-lease.md",
    "docs/reference/subetha-cxc/ownership-types/shared-arc.md",
    "docs/reference/subetha-cxc/ownership-types/shared-leader-election.md",
    "docs/reference/subetha-cxc/ownership.md",
    "docs/reference/subetha-cxc/pointers/offset-ptr.md",
    "docs/reference/subetha-cxc/pointers/tagged-offset-ptr.md",
    "docs/reference/subetha-cxc/rings/adaptive-ordering.md",
    "docs/reference/subetha-cxc/rings/async-spsc-ring.md",
    "docs/reference/subetha-cxc/rings/blocking-mpmc-ring.md",
    "docs/reference/subetha-cxc/rings/blocking-mpsc-ring.md",
    "docs/reference/subetha-cxc/rings/blocking-spsc-ring.md",
    "docs/reference/subetha-cxc/rings/capacity-adaptive-ring.md",
    "docs/reference/subetha-cxc/rings/capacity-broadcast-ring.md",
    "docs/reference/subetha-cxc/rings/dispatch-deque.md",
    "docs/reference/subetha-cxc/rings/frame-ring.md",
    "docs/reference/subetha-cxc/rings/locale-adaptive-ring.md",
    "docs/reference/subetha-cxc/rings/pubsub-ring.md",
    "docs/reference/subetha-cxc/rings/shared-broadcast-ring.md",
    "docs/reference/subetha-cxc/rings/shared-deque-khl.md",
    "docs/reference/subetha-cxc/rings/shared-deque-khpd.md",
    "docs/reference/subetha-cxc/rings/shared-deque-loh.md",
    "docs/reference/subetha-cxc/rings/shared-deque-urd.md",
    "docs/reference/subetha-cxc/rings/shared-deque.md",
    "docs/reference/subetha-cxc/rings/shared-ring-adaptive.md",
    "docs/reference/subetha-cxc/rings/shared-ring-mpmc.md",
    "docs/reference/subetha-cxc/rings/shared-ring-mpsc.md",
    "docs/reference/subetha-cxc/rings/shared-ring-spsc.md",
    "docs/reference/subetha-cxc/rings/shared-ring.md",
    "docs/reference/subetha-cxc/rings/shared-treiber-stack.md",
    "docs/reference/subetha-cxc/shared-atomic.md",
    "docs/reference/subetha-cxc/shared-cell.md",
    "docs/reference/subetha-cxc/shared-hash-map.md",
    "docs/reference/subetha-cxc/shared-locks.md",
    "docs/reference/subetha-cxc/shared-lru-cache.md",
    "docs/reference/subetha-cxc/shared-ring.md",
    "docs/reference/subetha-cxc/shared-sketches.md",
    "docs/reference/subetha-cxc/sketches/shared-bit-vec.md",
    "docs/reference/subetha-cxc/sketches/shared-blocked-bloom-filter.md",
    "docs/reference/subetha-cxc/sketches/shared-bloom-filter.md",
    "docs/reference/subetha-cxc/sketches/shared-count-min-sketch.md",
    "docs/reference/subetha-cxc/sketches/shared-histogram.md",
    "docs/reference/subetha-cxc/sketches/shared-hyper-log-log.md",
    "docs/reference/subetha-cxc/sketches/shared-reservoir-sampler.md",
    "docs/reference/subetha-cxc/specialized/shared-array.md",
    "docs/reference/subetha-cxc/specialized/shared-graph.md",
    "docs/reference/subetha-cxc/specialized/shared-nan-tagged-value.md",
    "docs/reference/subetha-cxc/specialized/shared-nan-value.md",
    "docs/reference/subetha-cxc/specialized/shared-slab.md",
    "docs/reference/subetha-cxc/specialized/shared-time-point.md",
    "docs/reference/subetha-cxc/specialized/shared-topology-map.md",
    "docs/reference/subetha-cxc/specialized/shared-umbra-pointer.md",
    "docs/reference/subetha-cxc/specialized/shared-universal.md",
    "docs/reference/subetha-cxc/specialized/shared-vec.md",
    "docs/reference/subetha-cxc/specialized/shared-versioned-chain.md",
    "docs/reference/subetha-cxc/specialized/shared-versioned-slab.md",
    "docs/reference/subetha-cxc/specialized/shm-file.md",
    "docs/reference/subetha-cxc/windows/large-pages.md",
    "docs/reference/subetha-ffi/_index.md",
    "docs/reference/subetha-pointers/bounds-check/cheri-capability.md",
    "docs/reference/subetha-pointers/bounds-check/rasp-pointer.md",
    "docs/reference/subetha-pointers/exotic-pointers/bloom-pointer.md",
    "docs/reference/subetha-pointers/exotic-pointers/cardinality-pointer.md",
    "docs/reference/subetha-pointers/exotic-pointers/kstep-pointer.md",
    "docs/reference/subetha-pointers/exotic-pointers/ktower-pointer.md",
    "docs/reference/subetha-pointers/exotic-pointers/self-desc-pointer.md",
    "docs/reference/subetha-pointers/exotic-pointers/umbra-pointer.md",
    "docs/reference/subetha-pointers/exotic-pointers/versioned-pointer.md",
    "docs/reference/subetha-pwrs/_index.md",
    "docs/reference/subetha-pwrs/enums.md",
    "docs/reference/subetha-pwrs/shm-drive.md",
    "docs/reference/subetha-pwrs/values.md",
    "docs/reference/subetha-py/_index.md",
    "docs/reference/subetha-py/values.md",
    "docs/reference/subetha-sidecar/_index.md",
    "docs/reference/subetha-sidecar/adaptive-instance.md",
    "docs/reference/subetha-sidecar/instance-stats.md",
    "docs/reference/subetha-sidecar/policy.md",
    "docs/reference/subetha-sidecar/registry.md",
    "docs/reference/subetha-sidecar/sidecar-box.md",
    "docs/tutorial/c-and-cpp.md",
    "docs/tutorial/powershell.md",
    "docs/tutorial/python.md",
];

fn main() -> ExitCode {
    if let Some(which) = std::env::var_os(PLAY) {
        return play(&which.to_string_lossy());
    }
    let filter = match std::env::var("SUBETHA_WIKI_PAGE") {
        Ok(filter) => Some(filter),
        Err(std::env::VarError::NotPresent) => None,
        Err(e) => {
            eprintln!("SUBETHA_WIKI_PAGE: {e}");
            return ExitCode::FAILURE;
        }
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    match check(&root, filter.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failures) => {
            eprintln!("\n{} failure(s):\n", failures.len());
            for failure in &failures {
                eprintln!("{failure}\n");
            }
            ExitCode::FAILURE
        }
    }
}

/// Run the program at `which` in this process, as the child a run starts.
fn play(which: &str) -> ExitCode {
    let Some((page, line)) = which.rsplit_once(':') else {
        eprintln!("{PLAY}={which} is not <page>:<line>");
        return ExitCode::FAILURE;
    };
    let line: usize = match line.parse() {
        Ok(line) => line,
        Err(e) => {
            eprintln!("{PLAY}={which}: the line is not a number: {e}");
            return ExitCode::FAILURE;
        }
    };
    match subetha_wiki_tests::run(page, line) {
        Some(code) => code,
        None => {
            eprintln!("this build holds no program at {which}");
            ExitCode::FAILURE
        }
    }
}

/// What a block is, by the conventions in this file's header.
#[derive(Debug, PartialEq, Eq)]
enum Kind {
    Program,
    Output,
    Listing,
    Command,
    Shown,
    Unconverted,
}

fn kind_of(rel: &str, block: &Block) -> Kind {
    if block.is_output() {
        return Kind::Output;
    }
    match block.lang.as_str() {
        "" => Kind::Unconverted,
        "sh" | "bash" | "console" | "toml" => Kind::Command,
        lang if Surface::of_lang(lang).is_some() => {
            if block.attr("filename").is_some() {
                Kind::Program
            } else if lang == "rust" && block.attr("listing") == Some("true") && rel.starts_with("docs/reference/") {
                Kind::Listing
            } else if lang == "powershell" && first_line(&block.body).is_some_and(|l| l.starts_with("PS> ")) {
                Kind::Command
            } else {
                Kind::Unconverted
            }
        }
        _ => Kind::Shown,
    }
}

fn first_line(body: &str) -> Option<&str> {
    body.lines().find(|l| !l.trim().is_empty())
}

/// Read every page, check every block, run every Rust and C program, and
/// print what was done; the failures, when there are any.
fn check(root: &Path, filter: Option<&str>) -> Result<(), Vec<String>> {
    let content = root.join("wiki").join("content");
    let mut failures = Vec::new();
    let pages = page::pages(&content).map_err(|e| vec![e])?;
    let workspace = Workspace::read(root).map_err(|e| vec![e])?;
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut unconverted_blocks = 0usize;
    let mut programs: Vec<Program> = Vec::new();
    let mut listings = 0usize;
    let mut claims = 0usize;
    let mut seen = 0usize;
    for (rel, path) in &pages {
        if filter.is_some_and(|f| !rel.contains(f)) {
            continue;
        }
        seen += 1;
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) => {
                failures.push(format!("{rel}: {e}"));
                continue;
            }
        };
        let blocks = match page::blocks(&text) {
            Ok(blocks) => blocks,
            Err(e) => {
                failures.push(format!("{rel}: {e}"));
                continue;
            }
        };
        match page::programs(rel, &blocks) {
            Ok(found) => programs.extend(found),
            Err(e) => failures.push(e),
        }
        let listed = UNCONVERTED.contains(&rel.as_str());
        let mut held = 0usize;
        for block in &blocks {
            let kind = kind_of(rel, block);
            *kinds.entry(format!("{kind:?}")).or_default() += 1;
            match kind {
                Kind::Unconverted if listed => held += 1,
                Kind::Unconverted => failures.push(format!(
                    "{rel}:{}: a `{}` block that is not a program, a listing or a command{}",
                    block.line,
                    block.lang,
                    if block.lang == "rust" && block.attr("listing").is_some() {
                        " (a listing belongs on a reference page)"
                    } else {
                        ""
                    }
                )),
                Kind::Listing => {
                    listings += 1;
                    failures.extend(workspace.check_listing(rel, block));
                }
                Kind::Command => {
                    let (count, bad) = workspace.check_command(rel, block);
                    claims += count;
                    failures.extend(bad);
                }
                Kind::Program | Kind::Output | Kind::Shown => {}
            }
        }
        if listed && held == 0 {
            failures.push(format!("{rel}: every block is written to the conventions; take it off UNCONVERTED"));
        }
        unconverted_blocks += held;
    }
    for listed in UNCONVERTED {
        if !pages.iter().any(|(rel, _)| rel == listed) {
            failures.push(format!("{listed}: on UNCONVERTED, and no such page"));
        }
    }
    let scope = match filter {
        Some(f) => format!(" whose path holds {f:?}"),
        None => String::new(),
    };
    println!(
        "{seen} pages{scope}; blocks by kind: {}",
        kinds.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join(", ")
    );
    println!(
        "{} pages not yet converted, holding {unconverted_blocks} blocks; {listings} listings checked; \
         {claims} package names and versions checked",
        UNCONVERTED.len()
    );
    failures.extend(run_programs(&programs));
    if failures.is_empty() { Ok(()) } else { Err(failures) }
}

/// One program's process, started.
struct Started {
    child: Child,
    began: Instant,
    stdout: JoinHandle<Result<Vec<u8>, String>>,
    stderr: JoinHandle<Result<Vec<u8>, String>>,
}

/// Run every Rust and C program, a page's programs for one surface in page
/// order from a folder of their own, and return what failed.
fn run_programs(programs: &[Program]) -> Vec<String> {
    let mut failures = Vec::new();
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return vec![format!("this test binary's own path: {e}")],
    };
    let base = std::env::temp_dir().join(format!("subetha-wiki-{}", std::process::id()));
    let mut groups: Vec<((String, Surface), Vec<&Program>)> = Vec::new();
    for program in programs.iter().filter(|p| matches!(p.surface, Surface::Rust | Surface::C)) {
        let key = (program.page.clone(), program.surface);
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(program),
            None => groups.push((key, vec![program])),
        }
    }
    let mut ran = 0usize;
    let mut passed = 0usize;
    let mut other_os: BTreeMap<String, usize> = BTreeMap::new();
    for (n, ((page, surface), list)) in groups.iter().enumerate() {
        let dir = base.join(format!("{n:03}-{}", surface.tab_name()));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            failures.push(format!("{page}: the folder {}: {e}", dir.display()));
            continue;
        }
        let mut alongside: Vec<(&Program, Started)> = Vec::new();
        for program in list {
            if let Some(os) = &program.os
                && os != std::env::consts::OS
            {
                *other_os.entry(os.clone()).or_default() += 1;
                continue;
            }
            ran += 1;
            let started = match start(&exe, program, &dir) {
                Ok(started) => started,
                Err(e) => {
                    failures.push(format!("{}:{}: {e}", program.page, program.line));
                    continue;
                }
            };
            if program.alongside {
                alongside.push((program, started));
                continue;
            }
            match judge(program, started) {
                Ok(()) => passed += 1,
                Err(e) => failures.push(e),
            }
        }
        for (program, started) in alongside {
            match judge(program, started) {
                Ok(()) => passed += 1,
                Err(e) => failures.push(e),
            }
        }
    }
    if base.exists()
        && let Err(e) = std::fs::remove_dir_all(&base)
    {
        failures.push(format!("the programs' folders under {} were not removed: {e}", base.display()));
    }
    let skipped: usize = other_os.values().sum();
    let by_os = if other_os.is_empty() {
        String::new()
    } else {
        format!(" ({})", other_os.iter().map(|(os, n)| format!("{os} {n}")).collect::<Vec<_>>().join(", "))
    };
    println!(
        "ran {ran} Rust and C programs: {passed} passed, {} failed; {skipped} for another operating system{by_os}",
        ran - passed
    );
    failures
}

/// Start `program` as a process of its own, in `dir`, with its temporary
/// directory pointed at `dir`.
fn start(exe: &Path, program: &Program, dir: &Path) -> Result<Started, String> {
    let mut command = Command::new(exe);
    command
        .env(PLAY, format!("{}:{}", program.page, program.line))
        .env("TEMP", dir)
        .env("TMP", dir)
        .env("TMPDIR", dir)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if cfg!(unix) {
        command.env("XDG_RUNTIME_DIR", dir);
    }
    let mut child = command.spawn().map_err(|e| format!("starting the program: {e}"))?;
    let stdout = drain(child.stdout.take().ok_or("the program's standard output is not piped")?);
    let stderr = drain(child.stderr.take().ok_or("the program's standard error is not piped")?);
    Ok(Started { child, began: Instant::now(), stdout, stderr })
}

/// Everything `from` yields, read on a thread of its own so a program that
/// prints more than a pipe holds is never stopped by a full pipe.
fn drain(mut from: impl Read + Send + 'static) -> JoinHandle<Result<Vec<u8>, String>> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        from.read_to_end(&mut bytes).map_err(|e| format!("reading the program's output: {e}"))?;
        Ok(bytes)
    })
}

/// Wait for `started` to end, stopping it at the deadline, and hold what
/// it did to what its page shows.
fn judge(program: &Program, mut started: Started) -> Result<(), String> {
    let at = format!("{}:{} ({})", program.page, program.line, program.filename);
    let status = loop {
        match started.child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.began.elapsed() >= DEADLINE => {
                started.child.kill().map_err(|e| format!("{at}: stopping it at the deadline: {e}"))?;
                started.child.wait().map_err(|e| format!("{at}: waiting for it after stopping it: {e}"))?;
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("{at}: waiting for it: {e}")),
        }
    };
    let stdout = joined(started.stdout, &at)?;
    let stderr = joined(started.stderr, &at)?;
    let printed = page::normalized(&String::from_utf8_lossy(&stdout));
    let report = |why: String| {
        format!(
            "{at}: {why}\n--- printed\n{printed}\n--- standard error\n{}",
            String::from_utf8_lossy(&stderr).trim_end()
        )
    };
    let Some(status) = status else {
        return Err(report(format!("still running after {} s, so stopped", DEADLINE.as_secs())));
    };
    if !status.success() {
        return Err(report(format!("exited with {status}")));
    }
    if let Some((line, shown)) = &program.output {
        let shown = page::normalized(shown);
        if printed != shown {
            return Err(report(format!("printed other than the Output block at line {line} shows:\n--- shown\n{shown}")));
        }
    }
    Ok(())
}

fn joined(handle: JoinHandle<Result<Vec<u8>, String>>, at: &str) -> Result<Vec<u8>, String> {
    match handle.join() {
        Ok(bytes) => bytes.map_err(|e| format!("{at}: {e}")),
        Err(payload) => {
            let why = match (payload.downcast_ref::<&str>(), payload.downcast_ref::<String>()) {
                (Some(s), _) => (*s).to_string(),
                (None, Some(s)) => s.clone(),
                (None, None) => "a panic carrying no message".to_string(),
            };
            Err(format!("{at}: the thread reading its output panicked: {why}"))
        }
    }
}

/// What the commands and listings are held to: the workspace's version,
/// its published crates, the Python package's and the PowerShell module's
/// names, the cargo-pwrs version the module is built by, and every line of
/// every crate's source.
struct Workspace {
    version: String,
    crates: Vec<String>,
    python: String,
    module: String,
    cargo_pwrs: String,
    source_lines: HashSet<String>,
}

impl Workspace {
    fn read(root: &Path) -> Result<Workspace, String> {
        let manifest = read(&root.join("Cargo.toml"))?;
        let version = value_in_section(&manifest, "[workspace.package]", "version")
            .ok_or("the workspace manifest names no [workspace.package] version")?;
        let mut crates = Vec::new();
        let mut source_lines = HashSet::new();
        let crates_dir = root.join("crates");
        let entries = std::fs::read_dir(&crates_dir).map_err(|e| format!("read {}: {e}", crates_dir.display()))?;
        for entry in entries {
            let dir = entry.map_err(|e| format!("an entry under {}: {e}", crates_dir.display()))?.path();
            let toml = dir.join("Cargo.toml");
            if !toml.exists() {
                continue;
            }
            let text = read(&toml)?;
            let name = value_in_section(&text, "[package]", "name")
                .ok_or_else(|| format!("{} names no package", toml.display()))?;
            if !text.lines().any(|l| l.trim() == "publish = false") {
                crates.push(name);
            }
            let src = dir.join("src");
            if src.exists() {
                for file in rust_files(&src)? {
                    for line in read(&file)?.lines() {
                        source_lines.insert(collapsed(line));
                    }
                }
            }
        }
        let pyproject = read(&root.join("crates").join("subetha-py").join("pyproject.toml"))?;
        let python = value_in_section(&pyproject, "[project]", "name").ok_or("pyproject.toml names no project")?;
        let pwrs_lib = read(&root.join("crates").join("subetha-pwrs").join("src").join("lib.rs"))?;
        let module = pwrs_lib
            .split_once("export_module!")
            .and_then(|(_, rest)| rest.split_once("name: \""))
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(name, _)| name.to_string())
            .ok_or("subetha-pwrs's export_module! names no module")?;
        let pwrs_toml = read(&root.join("crates").join("subetha-pwrs").join("Cargo.toml"))?;
        let cargo_pwrs = pwrs_toml
            .lines()
            .find(|l| l.contains("package = \"PoWerRuSt\""))
            .and_then(|l| l.split_once("version = \"="))
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(v, _)| v.to_string())
            .ok_or("subetha-pwrs pins no exact PoWerRuSt version")?;
        Ok(Workspace { version, crates, python, module, cargo_pwrs, source_lines })
    }

    /// Every line of a listing that is in no crate's source.
    fn check_listing(&self, rel: &str, block: &Block) -> Vec<String> {
        let mut bad = Vec::new();
        for (k, line) in block.body.lines().enumerate() {
            let line = collapsed(line);
            if matches!(line.as_str(), "" | "{" | "}" | "};" | "...") {
                continue;
            }
            if !self.source_lines.contains(&line) {
                bad.push(format!("{rel}:{}: a listing line no crate's source holds: {line}", block.line + 1 + k));
            }
        }
        bad
    }

    /// How many SubEtha package names and versions a command block states,
    /// and each one that is not the workspace's.
    fn check_command(&self, rel: &str, block: &Block) -> (usize, Vec<String>) {
        let mut count = 0usize;
        let mut bad = Vec::new();
        for (k, raw) in block.body.lines().enumerate() {
            let at = format!("{rel}:{}", block.line + 1 + k);
            let line = raw.trim().trim_start_matches("PS> ").trim_start_matches("$ ");
            let words: Vec<&str> = line.split_whitespace().collect();
            if block.lang == "toml" {
                if let Some((name, rest)) = line.split_once('=')
                    && name.trim().starts_with("subetha")
                {
                    count += 1;
                    let name = name.trim();
                    if !self.crates.iter().any(|c| c == name) {
                        bad.push(format!("{at}: `{name}` is not a published SubEtha crate"));
                    }
                    match requirement(rest) {
                        Some(req) if self.allows(&req) => {}
                        Some(req) => {
                            bad.push(format!("{at}: `{name}` at {req:?}, which {} does not meet", self.version))
                        }
                        None => bad.push(format!("{at}: `{name}` with no version")),
                    }
                }
                continue;
            }
            if words.len() >= 2 && words[0] == "cargo" && words[1] == "add" {
                for word in words[2..].iter().filter(|w| w.starts_with("subetha")) {
                    count += 1;
                    let (name, version) = match word.split_once('@') {
                        Some((name, version)) => (name, Some(version)),
                        None => (*word, None),
                    };
                    if !self.crates.iter().any(|c| c == name) {
                        bad.push(format!("{at}: `{name}` is not a published SubEtha crate"));
                    }
                    if let Some(version) = version
                        && !self.allows(version)
                    {
                        bad.push(format!("{at}: `{name}@{version}`, which {} does not meet", self.version));
                    }
                }
            }
            if let Some(pip) = words.iter().position(|w| *w == "pip")
                && words.get(pip + 1) == Some(&"install")
            {
                for word in words[pip + 2..].iter().filter(|w| w.starts_with("subetha")) {
                    count += 1;
                    let (spec, version) = match word.split_once("==") {
                        Some((spec, version)) => (spec, Some(version)),
                        None => (*word, None),
                    };
                    let name = match spec.split_once('[') {
                        Some((name, _extras)) => name,
                        None => spec,
                    };
                    if name != self.python {
                        bad.push(format!("{at}: `{name}` is not the Python package, {}", self.python));
                    }
                    if let Some(version) = version
                        && version != self.version
                    {
                        bad.push(format!("{at}: `{name}=={version}` where the workspace is {}", self.version));
                    }
                }
            }
            let gallery = ["Install-Module", "Install-PSResource", "Save-PSResource", "Update-Module", "Find-Module"];
            if words.first().is_some_and(|w| gallery.contains(w)) {
                for (i, word) in words.iter().enumerate().skip(1) {
                    let value = words.get(i + 1).copied();
                    let named = if word.eq_ignore_ascii_case("-Name") {
                        Some(value.unwrap_or(""))
                    } else if i == 1 && !word.starts_with('-') {
                        Some(*word)
                    } else {
                        None
                    };
                    if let Some(name) = named {
                        count += 1;
                        if name != self.module {
                            bad.push(format!("{at}: `{name}` is not the module, {}", self.module));
                        }
                    }
                    if ["-RequiredVersion", "-Version", "-MinimumVersion"].iter().any(|f| word.eq_ignore_ascii_case(f)) {
                        count += 1;
                        let version = value.unwrap_or("");
                        if version != self.version {
                            bad.push(format!("{at}: version `{version}` where the workspace is {}", self.version));
                        }
                    }
                }
            }
            if words.len() >= 3 && words[0] == "cargo" && words[1] == "install" && words[2] == "cargo-pwrs" {
                count += 1;
                match words.iter().position(|w| *w == "--version").and_then(|i| words.get(i + 1)) {
                    Some(v) if *v == self.cargo_pwrs => {}
                    Some(v) => bad.push(format!("{at}: cargo-pwrs {v} where subetha-pwrs pins {}", self.cargo_pwrs)),
                    None => bad.push(format!("{at}: cargo-pwrs with no --version, where subetha-pwrs pins {}", self.cargo_pwrs)),
                }
            }
        }
        (count, bad)
    }

    /// Whether a version requirement written `0.6`, `0.6.0`, `=0.6.0` or
    /// `^0.6` accepts the workspace's version.
    fn allows(&self, req: &str) -> bool {
        let req = req.trim_start_matches(['=', '^', '~']);
        let mut have = self.version.split('.');
        req.split('.').all(|part| have.next() == Some(part))
    }
}

/// A dependency line's version: `"0.6"` or `{ version = "0.6", ... }`.
fn requirement(rest: &str) -> Option<String> {
    let rest = rest.trim();
    let quoted = if let Some(inner) = rest.strip_prefix('{') {
        inner.split_once("version")?.1.split_once('=')?.1.trim_start()
    } else {
        rest
    };
    let quoted = quoted.strip_prefix('"')?;
    Some(quoted.split_once('"')?.0.to_string())
}

/// The value of `key = "..."` in the `section` of a TOML file.
fn value_in_section(text: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == section;
            continue;
        }
        if inside
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().trim_matches('"').to_string());
        }
    }
    None
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// Every `.rs` file under `dir`.
fn rust_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("read {}: {e}", dir.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| format!("an entry under {}: {e}", dir.display()))?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }
    Ok(found)
}

/// `line` with its runs of whitespace made one space and its ends trimmed.
fn collapsed(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}
