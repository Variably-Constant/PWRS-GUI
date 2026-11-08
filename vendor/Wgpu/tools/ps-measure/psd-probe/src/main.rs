//! Reads each Photoshop file named on the command line with Wgpu's own
//! reader, the source files compiled in by path, and writes what it reads:
//! `read` mode the picture the import loads, `merge` mode the document's
//! layers merged whether or not its composite is the real merge. Each
//! picture goes to <output folder>/<name>.<mode>.f32 as little-endian
//! float32 samples from 0 to 1, interleaved, and a JSON line per file
//! gives its name, its path, its size, color, alpha, warnings and what was
//! done; the name is <folder>__<file>, and ~2, ~3 and on where a folder
//! and file of the same names came earlier in the run. `gradients`
//! mode writes no picture: a JSON line per gradient-filled layer says how
//! the merge's drawing of it from its settings compares with the pixels it
//! stores, inside its shape and clear of a stroke's band. `costs` mode
//! writes no picture either: a JSON line per file of what the merge's
//! parts cost in seconds. `bands` mode takes a band size where the others
//! take the output folder, merges each file in bands of that size, and
//! writes a JSON line of the merge's time, a digest of every bit of its
//! picture and of its warnings, and the process's peak working set, which
//! only describes the size it ran when the process merges one file.
//!
//! `band-costs` mode takes, there, a band's rows and the step between the
//! bands it samples, and writes a JSON line of what those bands cost on one
//! core, the whole merge and its parts.
//!
//!     psd-probe <read|merge|gradients|costs> <output folder> <file>...
//!     psd-probe bands <whole|cores|rows:N|pixels:P> <file>...
//!     psd-probe band-costs <rows>:<every> <file>...
//!
//! Prints `DONE <count>` at the end.

// The module's sources, compiled in by path, hold items only the module
// itself uses.
#![allow(dead_code)]

#[path = "../../../../src/psd.rs"]
mod psd;
#[path = "../../../../src/psd_composite8.rs"]
mod psd_composite8;
#[path = "../../../../src/psd_descriptor.rs"]
mod psd_descriptor;
#[path = "../../../../src/psd_fill.rs"]
mod psd_fill;
#[path = "../../../../src/psd_gradient.rs"]
mod psd_gradient;
mod psd_merge {
    include!(concat!(env!("OUT_DIR"), "/psd_merge.rs"));
}
#[path = "../../../../src/psd_raster.rs"]
mod psd_raster;
#[path = "../../../../src/psd_stroke.rs"]
mod psd_stroke;
#[path = "../../../../src/psd_vector.rs"]
mod psd_vector;

use std::io::Write as _;

fn floats(samples: &psd::Samples) -> Vec<f32> {
    match samples {
        psd::Samples::Eight(v) => v.iter().map(|&x| f32::from(x) / 255.0).collect(),
        psd::Samples::Sixteen(v) => v.iter().map(|&x| f32::from(x) / 65535.0).collect(),
        psd::Samples::Float(v) => v.clone(),
    }
}

fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn list(items: &[String]) -> String {
    format!("[{}]", items.iter().map(|s| quoted(s)).collect::<Vec<_>>().join(", "))
}

/// The probe compiles the PSD files without the module's color handling, so
/// it converts no fill color of another mode; such a layer is left out with
/// this reason, where the module would convert it.
fn unconverted(_color: psd_fill::Foreign, _header: &psd::Header, _icc: Option<&[u8]>) -> Result<Vec<f32>, String> {
    Err("the probe converts no fill color of another mode".to_string())
}

fn srgb_decode(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

fn srgb_encode(v: f64) -> f64 {
    if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.max(0.0).powf(1.0 / 2.4) - 0.055 }
}

/// The probe's light for the linear method: an RGB or gray document's
/// without a profile as sRGB's, a 32-bit one's as stored; it reads no
/// profile's curves, so a document with a profile has none here.
fn probe_light(header: &psd::Header, icc: Option<&[u8]>) -> Result<psd_fill::Light, String> {
    let channels = match header.mode {
        psd::Mode::Rgb => 3,
        psd::Mode::Gray => 1,
        mode => return Err(format!("a {} document's samples are not light the merge mixes in", mode.name())),
    };
    let each = |f: fn(f64) -> f64| -> Box<dyn Fn(&[f64], &mut [f64])> {
        Box::new(move |from, to| {
            for (out, &v) in to.iter_mut().zip(from) {
                *out = f(v);
            }
        })
    };
    if header.depth == 32 {
        return Ok(psd_fill::Light { width: channels, into: each(|v| v), back: each(|v| v) });
    }
    if icc.is_some() {
        return Err("the probe reads no profile's curves".to_string());
    }
    Ok(psd_fill::Light { width: channels, into: each(srgb_decode), back: each(srgb_encode) })
}

const COLORS: psd_fill::Colors = psd_fill::Colors { convert: unconverted, light: probe_light };

fn run(mode: &str, path: &str) -> Result<(psd::Picture, Vec<String>, Vec<String>), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    match mode {
        "read" => {
            let document = psd::read(&bytes, COLORS)?;
            Ok((document.picture, document.warnings, document.said))
        }
        _ => {
            let sections = psd::sections(&bytes)?;
            let merged = psd_merge::merged(&sections.header, &sections.layers, sections.color_data, &sections.resources, COLORS)?;
            Ok((merged.picture, merged.warnings, Vec::new()))
        }
    }
}

/// `gradients` mode for one file: a JSON line per gradient-filled layer.
fn gradients(name: &str, path: &str) {
    let checks = std::fs::read(path).map_err(|e| e.to_string()).and_then(|bytes| {
        let sections = psd::sections(&bytes)?;
        psd_merge::gradient_checks(&sections.header, &sections.layers, &sections.resources, COLORS)
    });
    match checks {
        Ok(checks) => {
            for c in checks {
                let undrawn = c.undrawn.as_deref().map_or("null".to_string(), quoted);
                let ends = c.ends.iter().map(|[v, apart, eps]| format!("[{v:?}, {apart:e}, {eps:.3}]")).collect::<Vec<_>>().join(", ");
                println!(
                    "{{\"name\": {}, \"layer\": {}, \"undrawn\": {}, \"compared\": {}, \"color\": [{:.3}, {:.3}, {:.4}], \"shape\": [{:.3}, {:.3}, {:.4}], \"ends\": [{}]}}",
                    quoted(name),
                    quoted(&c.name),
                    undrawn,
                    c.compared,
                    c.color[0],
                    c.color[1],
                    c.color[2],
                    c.shape[0],
                    c.shape[1],
                    c.shape[2],
                    ends
                );
            }
        }
        Err(e) => println!("{{\"name\": {}, \"error\": {}}}", quoted(name), quoted(&e)),
    }
}

/// `costs` mode for one file: a JSON line of what the merge's parts cost.
fn costs(name: &str, path: &str) {
    let result = std::fs::read(path).map_err(|e| e.to_string()).and_then(|bytes| {
        let sections = psd::sections(&bytes)?;
        psd_merge::merge_costs(&sections.header, &sections.layers, &sections.resources, sections.color_data, COLORS)
    });
    match result {
        Ok(parts) => {
            let listed: Vec<String> = parts.iter().map(|(what, v)| format!("{}: {v:.3}", quoted(what))).collect();
            println!("{{\"name\": {}, {}}}", quoted(name), listed.join(", "));
        }
        Err(e) => println!("{{\"name\": {}, \"error\": {}}}", quoted(name), quoted(&e)),
    }
}

/// What GetProcessMemoryInfo fills in, PROCESS_MEMORY_COUNTERS.
#[repr(C)]
#[derive(Default)]
struct MemoryCounters {
    cb: u32,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
}

unsafe extern "system" {
    fn GetCurrentProcess() -> isize;
    fn K32GetProcessMemoryInfo(process: isize, counters: *mut MemoryCounters, cb: u32) -> i32;
}

/// The process's largest working set so far, in megabytes, or why it
/// cannot be read.
fn peak_megabytes() -> Result<f64, String> {
    let mut counters = MemoryCounters { cb: std::mem::size_of::<MemoryCounters>() as u32, ..MemoryCounters::default() };
    // SAFETY: the current process's pseudo-handle, and counters of the size
    // the call is told.
    let read = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    if read == 0 {
        return Err(format!("the memory counters do not read: {}", std::io::Error::last_os_error()));
    }
    Ok(counters.peak_working_set_size as f64 / 1_048_576.0)
}

/// FNV-1a over `bytes`.
fn digest(bytes: impl Iterator<Item = u8>) -> u64 {
    bytes.fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3))
}

/// `bands` mode for one file: the merge in bands as `spec` sizes them,
/// `whole` one band of every row, `cores` as many bands as the machine has
/// cores, `rows:N` bands of N rows and `pixels:P` bands of the rows P
/// pixels fill, at least one; a JSON line of the bands, the merge's time,
/// a digest of every bit of its color and alpha and one of its warnings,
/// and the process's peak working set.
fn bands(name: &str, spec: &str, path: &str) {
    let result = std::fs::read(path).map_err(|e| e.to_string()).and_then(|bytes| {
        let sections = psd::sections(&bytes)?;
        let (width, height) = (sections.header.width, sections.header.height);
        let cores = std::thread::available_parallelism().map_err(|e| format!("the cores do not count: {e}"))?.get();
        let number = |n: &str| n.parse::<usize>().map_err(|e| format!("{spec}: {e}"));
        let rows = match spec.split_once(':') {
            None if spec == "whole" => height,
            None if spec == "cores" => height.div_ceil(cores),
            Some(("rows", n)) => number(n)?,
            Some(("pixels", n)) => (number(n)? / width.max(1)).max(1),
            _ => return Err(format!("no band size {spec}")),
        };
        let started = std::time::Instant::now();
        let (color, alpha, warnings) = psd_merge::merge_bands(&sections.header, &sections.layers, &sections.resources, COLORS, rows)?;
        let ms = started.elapsed().as_millis();
        let pictures = digest(color.iter().flatten().chain(&alpha).flat_map(|v| v.to_bits().to_le_bytes()));
        let warned = digest(warnings.iter().flat_map(|w| w.bytes().chain([0])));
        Ok(format!(
            "\"width\": {width}, \"height\": {height}, \"rows\": {rows}, \"bands\": {}, \"cores\": {cores}, \"ms\": {ms}, \"digest\": \"{pictures:016x}\", \"warnings\": {}, \"warnings digest\": \"{warned:016x}\", \"peak MB\": {:.0}",
            height.div_ceil(rows.max(1)),
            warnings.len(),
            peak_megabytes()?
        ))
    });
    match result {
        Ok(fields) => println!("{{\"name\": {}, \"spec\": {}, {fields}}}", quoted(name), quoted(spec)),
        Err(e) => println!("{{\"name\": {}, \"spec\": {}, \"error\": {}}}", quoted(name), quoted(spec), quoted(&e)),
    }
}

/// `band-costs` mode for one file: a JSON line of what every `every`-th
/// band of `rows` rows costs on one core, `spec` being `<rows>:<every>`.
fn band_costs(name: &str, spec: &str, path: &str) {
    let result = std::fs::read(path).map_err(|e| e.to_string()).and_then(|bytes| {
        let sections = psd::sections(&bytes)?;
        let (rows, every) = spec.split_once(':').ok_or(format!("no <rows>:<every> in {spec}"))?;
        let number = |n: &str| n.parse::<usize>().map_err(|e| format!("{spec}: {e}"));
        psd_merge::band_costs(&sections.header, &sections.layers, &sections.resources, COLORS, number(rows)?, number(every)?)
    });
    match result {
        Ok(parts) => {
            let listed: Vec<String> = parts.iter().map(|(what, v)| format!("{}: {v:.3}", quoted(what))).collect();
            println!("{{\"name\": {}, \"spec\": {}, {}}}", quoted(name), quoted(spec), listed.join(", "));
        }
        Err(e) => println!("{{\"name\": {}, \"spec\": {}, \"error\": {}}}", quoted(name), quoted(spec), quoted(&e)),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 || !matches!(args[1].as_str(), "read" | "merge" | "gradients" | "costs" | "bands" | "band-costs") {
        eprintln!("psd-probe <read|merge|gradients|costs> <output folder> <file>...");
        eprintln!("psd-probe bands <whole|cores|rows:N|pixels:P> <file>...");
        eprintln!("psd-probe band-costs <rows>:<every> <file>...");
        std::process::exit(2);
    }
    let (mode, out) = (args[1].as_str(), std::path::Path::new(&args[2]));
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for path in &args[3..] {
        let file = std::path::Path::new(path);
        let (Some(folder), Some(base)) = (file.parent().and_then(|p| p.file_name()), file.file_name()) else {
            println!("{{\"name\": {}, \"path\": {}, \"error\": \"the path names no folder and file\"}}", quoted(path), quoted(path));
            continue;
        };
        let short = format!("{}__{}", folder.to_string_lossy(), base.to_string_lossy());
        let count = seen.entry(short.clone()).or_insert(0);
        *count += 1;
        let name = if *count == 1 { short } else { format!("{short}~{count}") };
        if mode == "gradients" {
            gradients(&name, path);
            continue;
        }
        if mode == "costs" {
            costs(&name, path);
            continue;
        }
        if mode == "bands" {
            bands(&name, &args[2], path);
            continue;
        }
        if mode == "band-costs" {
            band_costs(&name, &args[2], path);
            continue;
        }
        let started = std::time::Instant::now();
        match run(mode, path) {
            Ok((picture, warnings, said)) => {
                let samples = floats(&picture.samples);
                let target = out.join(format!("{name}.{mode}.f32"));
                let mut bytes = Vec::with_capacity(samples.len() * 4);
                for s in &samples {
                    bytes.extend_from_slice(&s.to_le_bytes());
                }
                let written = std::fs::File::create(&target).and_then(|mut f| f.write_all(&bytes));
                if let Err(e) = written {
                    println!("{{\"name\": {}, \"path\": {}, \"error\": {}}}", quoted(&name), quoted(path), quoted(&format!("writing {}: {e}", target.display())));
                    continue;
                }
                println!(
                    "{{\"name\": {}, \"path\": {}, \"width\": {}, \"height\": {}, \"color\": \"{:?}\", \"alpha\": {}, \"ms\": {}, \"warnings\": {}, \"said\": {}}}",
                    quoted(&name),
                    quoted(path),
                    picture.width,
                    picture.height,
                    picture.color,
                    picture.alpha,
                    started.elapsed().as_millis(),
                    list(&warnings),
                    list(&said)
                );
            }
            Err(e) => println!("{{\"name\": {}, \"path\": {}, \"error\": {}}}", quoted(&name), quoted(path), quoted(&e)),
        }
    }
    println!("DONE {}", args.len() - 3);
}
