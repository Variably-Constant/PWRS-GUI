//! Reference values for Wgpu's conversion of a float image from its color
//! profile to linear light in sRGB's primaries, from littleCMS 2.19, which
//! lcms2-sys 4.0.7 vendors and this tool builds, through its float
//! transforms with its optimizations off, NOOPTIMIZE, so it computes each
//! value from the profile as written. Each profile converts with the
//! perceptual, relative colorimetric, saturation and absolute colorimetric
//! intents, to a version 4 profile littleCMS builds of sRGB's white and
//! primaries, as its cmsCreate_sRGBProfile takes them, with curves of
//! gamma 1.
//!
//! The profiles are the RGB and gray ones src/profile_reference.rs holds,
//! and two littleCMS writes here: RGB of sRGB's white and primaries and
//! gray of D50, each with curves of gamma 1, as the profiles of Photoshop's
//! 32-bit documents are. Each converts from the bytes the output holds.
//! The values run from below 0 to above 1, each a sum of powers of 2, so a
//! float holds each exactly.
//!
//! It also gives the reference values for a Photoshop fill color of another
//! mode than its document's, converted into the document's profile: sRGB
//! and Lab colors, from cmsCreate_sRGBProfile and cmsCreateLab4Profile of
//! D50, converted relative colorimetrically as doubles into each profile a
//! fill writes into: the RGB and gray matrix-shapers src/profile_reference.rs
//! holds, its gray profile of a Lab PCS, and four littleCMS writes here,
//! CMYK through a lut16 table, CMYK through a lutBtoA table, RGB through a
//! BToD1 table, and RGB through tables both ways with no matrix-shaper's
//! curves. Each
//! value is held within 0 to 1, as an 8-bit or 16-bit document holds it.
//! Two colors of the last are also mixed in the XYZ of its tables, as a
//! Photoshop gradient of the linear method mixes them in a document of
//! that profile.
//!
//! It writes the Rust test constants in src/float_reference.rs and
//! src/fill_reference.rs. From the repository's folder, with a C compiler
//! for littleCMS's source:
//! cargo run --manifest-path tools/float_reference/Cargo.toml -- src/float_reference.rs src/fill_reference.rs

use std::ffi::{c_char, c_void, CStr};
use std::fmt::Write as _;
use std::ptr;

use lcms2_sys as lcms;

#[allow(dead_code)]
#[path = "../../../src/profile_reference.rs"]
mod reference;

/// The RGB values every RGB profile converts: black, white and grays, dark
/// values, primaries and mixtures, light above 1 and values below 0.
const RGB_VALUES: [[f32; 3]; 24] = [
    [0.0, 0.0, 0.0],
    [1.0, 1.0, 1.0],
    [0.5, 0.5, 0.5],
    [0.25, 0.25, 0.25],
    [0.0625, 0.0625, 0.0625],
    [0.001953125, 0.00390625, 0.0078125],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.75, 0.5, 0.25],
    [0.125, 0.375, 0.875],
    [0.9375, 0.0625, 0.03125],
    [0.0, 0.8125, 0.1875],
    [1.5, 1.5, 1.5],
    [2.0, 1.0, 0.5],
    [4.0, 4.0, 4.0],
    [8.0, 0.5, 0.25],
    [16.0, 16.0, 16.0],
    [1.25, 0.0, 0.0625],
    [3.0, 0.75, 1.5],
    [-0.25, 0.5, 0.5],
    [0.5, -0.125, 0.25],
    [-0.0625, -0.0625, -0.0625],
    [0.0009765625, 0.5, 2.5],
];

/// The gray values every gray profile converts.
const GRAY_VALUES: [f32; 12] = [0.0, 0.00390625, 0.0625, 0.25, 0.5, 0.75, 1.0, 1.5, 4.0, 16.0, -0.25, 0.0009765625];

/// The four intents, in the order the tests list them, and their names in
/// the output.
const INTENTS: [(lcms::Intent, &str); 4] = [
    (lcms::Intent::Perceptual, "PERCEPTUAL"),
    (lcms::Intent::RelativeColorimetric, "RELATIVE"),
    (lcms::Intent::Saturation, "SATURATION"),
    (lcms::Intent::AbsoluteColorimetric, "ABSOLUTE"),
];

/// sRGB's white, D65, and its primaries, as littleCMS's
/// cmsCreate_sRGBProfile takes them.
const D65: lcms::CIExyY = lcms::CIExyY { x: 0.3127, y: 0.3290, Y: 1.0 };
const PRIMARIES: lcms::CIExyYTRIPLE = lcms::CIExyYTRIPLE {
    Red: lcms::CIExyY { x: 0.64, y: 0.33, Y: 1.0 },
    Green: lcms::CIExyY { x: 0.30, y: 0.60, Y: 1.0 },
    Blue: lcms::CIExyY { x: 0.15, y: 0.06, Y: 1.0 },
};

/// littleCMS's errors, written to stderr as they come.
unsafe extern "C" fn logged(_context: lcms::Context, code: u32, text: *const c_char) {
    // SAFETY: littleCMS passes a message it ends with a 0.
    let text = unsafe { CStr::from_ptr(text) };
    eprintln!("littleCMS error {code}: {}", text.to_string_lossy());
}

/// The profile `bytes` hold, opened by littleCMS.
fn opened(name: &str, bytes: &[u8]) -> lcms::HPROFILE {
    let size = u32::try_from(bytes.len()).expect("a profile of fewer than 4 GB");
    // SAFETY: littleCMS reads `size` bytes from the pointer, all of `bytes`.
    let profile = unsafe { lcms::cmsOpenProfileFromMem(bytes.as_ptr().cast::<c_void>(), size) };
    assert!(!profile.is_null(), "littleCMS cannot open {name}");
    profile
}

/// `profile` closed, which littleCMS reports it did.
fn closed(name: &str, profile: lcms::HPROFILE) {
    // SAFETY: `profile` is a handle littleCMS gave and no one uses again.
    assert!(unsafe { lcms::cmsCloseProfile(profile) } != 0, "littleCMS cannot close {name}");
}

/// The profile `with` builds from a curve of gamma 1, the curve freed
/// after.
fn with_linear_curve(with: impl FnOnce(*const lcms::ToneCurve) -> lcms::HPROFILE) -> lcms::HPROFILE {
    // SAFETY: littleCMS builds the curve in its default context.
    let curve = unsafe { lcms::cmsBuildGamma(ptr::null_mut(), 1.0) };
    assert!(!curve.is_null(), "littleCMS cannot build a curve of gamma 1");
    let profile = with(curve);
    // SAFETY: the profile holds its own copy of the curve, and nothing else
    // holds this one.
    unsafe { lcms::cmsFreeToneCurve(curve) };
    assert!(!profile.is_null(), "littleCMS cannot build a profile of curves of gamma 1");
    profile
}

/// The creation date every profile this tool writes carries, in its
/// header's dateTimeNumber at bytes 24 to 35, where littleCMS stamps the
/// time it runs, so each run writes the same bytes. littleCMS writes no
/// profile ID the date would be part of.
const CREATED: [u8; 12] = [7, 234, 0, 10, 0, 27, 0, 14, 0, 6, 0, 29];

/// The bytes littleCMS writes `profile` as, its creation date CREATED.
fn saved(name: &str, profile: lcms::HPROFILE) -> Vec<u8> {
    let mut size = 0u32;
    // SAFETY: with no buffer littleCMS writes only the size it needs.
    assert!(unsafe { lcms::cmsSaveProfileToMem(profile, ptr::null_mut(), &mut size) } != 0, "littleCMS cannot size {name}");
    let mut bytes = vec![0u8; size as usize];
    // SAFETY: the buffer holds the `size` bytes littleCMS asked for.
    assert!(unsafe { lcms::cmsSaveProfileToMem(profile, bytes.as_mut_ptr().cast::<c_void>(), &mut size) } != 0, "littleCMS cannot write {name}");
    bytes.truncate(size as usize);
    bytes[24..36].copy_from_slice(&CREATED);
    bytes
}

/// `values`, a float for each channel of each pixel, converted from
/// `profile` to `target` with `intent` as floats, three to a pixel.
fn converted(name: &str, profile: lcms::HPROFILE, gray: bool, target: lcms::HPROFILE, intent: lcms::Intent, values: &[f32]) -> Vec<[f32; 3]> {
    let (format, channels) = if gray { (lcms::PixelFormat::GRAY_FLT, 1) } else { (lcms::PixelFormat::RGB_FLT, 3) };
    let pixels = values.len() / channels;
    // SAFETY: both handles are open profiles littleCMS gave.
    let transform = unsafe { lcms::cmsCreateTransform(profile, format, target, lcms::PixelFormat::RGB_FLT, intent, lcms::FLAGS_NOOPTIMIZE) };
    assert!(!transform.is_null(), "littleCMS cannot convert {name} with {intent:?}");
    let mut out = vec![0f32; pixels * 3];
    let count = u32::try_from(pixels).expect("fewer than 4 billion pixels");
    // SAFETY: the transform reads `pixels` pixels of `format` from `values`
    // and writes as many of three floats to `out`, which holds them.
    unsafe {
        lcms::cmsDoTransform(transform, values.as_ptr().cast::<c_void>(), out.as_mut_ptr().cast::<c_void>(), count);
        lcms::cmsDeleteTransform(transform);
    }
    out.chunks_exact(3).map(|p| [p[0], p[1], p[2]]).collect()
}

/// A float as a Rust literal that reads back as the same float.
fn literal(v: f32) -> String {
    assert!(v.is_finite(), "littleCMS gave {v}");
    format!("{v:?}")
}

fn rust_rows(name: &str, doc: &str, rows: &[[f32; 3]]) -> String {
    let mut out = format!("/// {doc}\npub(crate) static {name}: [[f32; 3]; {}] = [\n", rows.len());
    for row in rows {
        writeln!(out, "    [{}, {}, {}],", literal(row[0]), literal(row[1]), literal(row[2])).expect("a string takes every write");
    }
    out.push_str("];");
    out
}

fn rust_floats(name: &str, doc: &str, values: &[f32]) -> String {
    let listed: Vec<String> = values.iter().map(|&v| literal(v)).collect();
    format!("/// {doc}\npub(crate) static {name}: [f32; {}] = [{}];", values.len(), listed.join(", "))
}

fn rust_bytes(name: &str, doc: &str, data: &[u8]) -> String {
    let mut out = format!("/// {doc}\npub(crate) static {name}: [u8; {}] = [\n", data.len());
    for line in data.chunks(24) {
        let listed: Vec<String> = line.iter().map(u8::to_string).collect();
        writeln!(out, "    {},", listed.join(", ")).expect("a string takes every write");
    }
    out.push_str("];");
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, out_path, fill_path] = args.as_slice() else {
        eprintln!("usage: float-reference <src/float_reference.rs> <src/fill_reference.rs>");
        std::process::exit(2);
    };
    // SAFETY: the handler is a function littleCMS may call at any time.
    unsafe { lcms::cmsSetLogErrorHandler(Some(logged)) };

    // SAFETY: littleCMS copies the white, primaries and curves it is given.
    let linear_rgb = with_linear_curve(|curve| unsafe { lcms::cmsCreateRGBProfile(&D65, &PRIMARIES, [curve; 3].as_ptr()) });
    let linear_gray = with_linear_curve(|curve| unsafe { lcms::cmsCreateGrayProfile(lcms::cmsD50_xyY(), curve) });
    let linear_rgb_bytes = saved("the linear RGB profile", linear_rgb);
    let linear_gray_bytes = saved("the linear gray profile", linear_gray);
    closed("the linear RGB profile", linear_rgb);
    closed("the linear gray profile", linear_gray);
    let target = with_linear_curve(|curve| unsafe { lcms::cmsCreateRGBProfile(&D65, &PRIMARIES, [curve; 3].as_ptr()) });

    let rgb: [(&str, &[u8]); 13] = [
        ("SRGB", &reference::SRGB_PROFILE),
        ("DISPLAYP3", &reference::DISPLAYP3_PROFILE),
        ("DISPLAYP3_V2", &reference::DISPLAYP3_V2_PROFILE),
        ("ADOBE", &reference::ADOBE_PROFILE),
        ("PROPHOTO", &reference::PROPHOTO_PROFILE),
        ("REC2020", &reference::REC2020_PROFILE),
        ("MAB_RGB", &reference::MAB_RGB_PROFILE),
        ("LUT8_RGB", &reference::LUT8_RGB_PROFILE),
        ("FLOAT_RGB", &reference::FLOAT_RGB_PROFILE),
        ("LIFTED_RGB", &reference::LIFTED_RGB_PROFILE),
        ("A2B1_ONLY_RGB", &reference::A2B1_ONLY_RGB_PROFILE),
        ("WHITE_RGB", &reference::WHITE_RGB_PROFILE),
        ("LINEAR_RGB", &linear_rgb_bytes),
    ];
    let gray: [(&str, &[u8]); 5] = [
        ("SGREY", &reference::SGREY_PROFILE),
        ("MAB_GRAY", &reference::MAB_GRAY_PROFILE),
        ("FLOAT_GRAY", &reference::FLOAT_GRAY_PROFILE),
        ("LAB_GRAY", &reference::LAB_GRAY_PROFILE),
        ("LINEAR_GRAY", &linear_gray_bytes),
    ];

    let mut parts = vec![
        [
            "//! Reference values for image_decode::tests, written by",
            "//! tools/float_reference from littleCMS 2.19's float transforms: float",
            "//! values converted from each profile to linear light in sRGB's",
            "//! primaries, with littleCMS's optimizations off, as the profile is",
            "//! written. The profiles are profile_reference's RGB and gray ones and",
            "//! the two linear ones here, which littleCMS writes.",
        ]
        .join("\n"),
        rust_rows("RGB_VALUES", "The RGB values every RGB profile converts.", &RGB_VALUES),
        rust_floats("GRAY_VALUES", "The gray values every gray profile converts.", &GRAY_VALUES),
        rust_bytes("LINEAR_RGB_PROFILE", "A version 4 RGB profile of sRGB's white and primaries with curves of gamma 1, as littleCMS writes it.", &linear_rgb_bytes),
        rust_bytes("LINEAR_GRAY_PROFILE", "A version 4 gray profile of D50 with a curve of gamma 1, as littleCMS writes it.", &linear_gray_bytes),
    ];
    let flat_rgb: Vec<f32> = RGB_VALUES.iter().flatten().copied().collect();
    for (profiles, values, what) in [(&rgb[..], &flat_rgb[..], "RGB_VALUES"), (&gray[..], &GRAY_VALUES[..], "GRAY_VALUES")] {
        for &(name, bytes) in profiles {
            let profile = opened(name, bytes);
            for (intent, intent_name) in INTENTS {
                let rows = converted(name, profile, what == "GRAY_VALUES", target, intent, values);
                let doc = format!("{what} from {name}_PROFILE as linear light in sRGB's primaries, {}.", intent_name.to_lowercase());
                parts.push(rust_rows(&format!("{name}_{intent_name}"), &doc, &rows));
            }
            closed(name, profile);
            println!("converted {name}");
        }
    }
    closed("the target profile", target);
    std::fs::write(out_path, parts.join("\n\n") + "\n").expect("the output written");
    println!("WROTE {out_path}");
    std::fs::write(fill_path, fill_references().join("\n\n") + "\n").expect("the fill references written");
    println!("WROTE {fill_path}");
}

/// The sRGB colors every fill test converts: black, white, grays,
/// primaries, mixtures and a dark color.
const FILL_SRGB: [[f64; 3]; 12] = [
    [0.0, 0.0, 0.0],
    [1.0, 1.0, 1.0],
    [0.5, 0.5, 0.5],
    [0.2, 0.2, 0.2],
    [1.0, 0.0, 0.0],
    [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0],
    [0.75, 0.5, 0.25],
    [0.125, 0.375, 0.875],
    [0.9, 0.1, 0.05],
    [0.02, 0.01, 0.03],
    [1.0, 1.0, 0.0],
];

/// The Lab colors every fill test converts: black, white, a gray, colors
/// most profiles hold, and one past every one's gamut.
const FILL_LAB: [[f64; 3]; 9] = [
    [0.0, 0.0, 0.0],
    [100.0, 0.0, 0.0],
    [50.0, 0.0, 0.0],
    [60.0, 40.0, 30.0],
    [30.0, -20.0, -50.0],
    [80.0, -60.0, 70.0],
    [50.0, 90.0, -100.0],
    [20.0, 10.0, -40.0],
    [95.0, -5.0, 10.0],
];

/// XYZ under D50 to linear sRGB, sRGB's primaries adapted to D50 by
/// Bradford, for the test profiles' models.
const XYZ_TO_LINEAR_SRGB: [[f64; 3]; 3] = [[3.1338561, -1.6168667, -0.4906146], [-0.9787684, 1.9161415, 0.0334540], [0.0719453, -0.2289914, 1.4052427]];

/// `xyz` as the test model's RGB: linear sRGB clipped to 0 to 1, raised
/// to 1 over 2.2.
fn model_rgb(xyz: [f64; 3]) -> [f64; 3] {
    XYZ_TO_LINEAR_SRGB.map(|row| (row[0] * xyz[0] + row[1] * xyz[1] + row[2] * xyz[2]).clamp(0.0, 1.0).powf(1.0 / 2.2))
}

/// The CMYK test model's inks, each from 0 to 65535, for a Lab color in
/// littleCMS's 16-bit encoding: the model's RGB of it, black 1 less the
/// largest of red, green and blue, and each other ink what is left of its
/// channel after black.
unsafe extern "C" fn model_inks(input: *const u16, output: *mut u16, _cargo: *mut c_void) -> i32 {
    // SAFETY: littleCMS passes a pipeline's 3 inputs and room for its 4
    // outputs.
    let (input, output) = unsafe { (std::slice::from_raw_parts(input, 3), std::slice::from_raw_parts_mut(output, 4)) };
    let mut lab = lcms::CIELab { L: 0.0, a: 0.0, b: 0.0 };
    let mut xyz = lcms::CIEXYZ { X: 0.0, Y: 0.0, Z: 0.0 };
    // SAFETY: each pointer is to a value of the type littleCMS writes or
    // reads there.
    unsafe {
        lcms::cmsLabEncoded2Float(&mut lab, input.as_ptr());
        lcms::cmsLab2XYZ(lcms::cmsD50_XYZ(), &mut xyz, &lab);
    }
    let [r, g, b] = model_rgb([xyz.X, xyz.Y, xyz.Z]);
    let k = 1.0 - r.max(g).max(b);
    let inks = if k < 1.0 { [(1.0 - r - k) / (1.0 - k), (1.0 - g - k) / (1.0 - k), (1.0 - b - k) / (1.0 - k), k] } else { [0.0, 0.0, 0.0, 1.0] };
    for (o, v) in output.iter_mut().zip(inks) {
        *o = (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
    }
    1
}

/// The RGB test model's grid for a BToD1 table: each of three linear
/// values from 0 to 1 raised to 1 over 2.2.
unsafe extern "C" fn model_encoding(input: *const f32, output: *mut f32, _cargo: *mut c_void) -> i32 {
    // SAFETY: littleCMS passes the grid's 3 inputs and room for its 3
    // outputs.
    let (input, output) = unsafe { (std::slice::from_raw_parts(input, 3), std::slice::from_raw_parts_mut(output, 3)) };
    for (o, &v) in output.iter_mut().zip(input) {
        *o = v.clamp(0.0, 1.0).powf(1.0 / 2.2);
    }
    1
}

/// An output profile littleCMS writes: `version`, CMYK or RGB of a PCS,
/// D50's media white, and `tables` written to their tags.
fn written(name: &str, version: f64, class: lcms::ProfileClassSignature, space: lcms::ColorSpaceSignature, pcs: lcms::ColorSpaceSignature, tables: &[(lcms::TagSignature, *mut lcms::Pipeline)]) -> Vec<u8> {
    // SAFETY: littleCMS builds the profile in its default context, copies
    // what each tag is given, and the pipelines are freed after.
    unsafe {
        let profile = lcms::cmsCreateProfilePlaceholder(ptr::null_mut());
        assert!(!profile.is_null(), "littleCMS cannot start {name}");
        lcms::cmsSetProfileVersion(profile, version);
        lcms::cmsSetDeviceClass(profile, class);
        lcms::cmsSetColorSpace(profile, space);
        lcms::cmsSetPCS(profile, pcs);
        assert!(lcms::cmsWriteTag(profile, lcms::TagSignature::MediaWhitePointTag, lcms::cmsD50_XYZ() as *const lcms::CIEXYZ as *const c_void) != 0, "littleCMS cannot write {name}'s white");
        for &(tag, pipeline) in tables {
            assert!(lcms::cmsWriteTag(profile, tag, pipeline as *const c_void) != 0, "littleCMS cannot write {name}'s {tag:?}");
        }
        let bytes = saved(name, profile);
        closed(name, profile);
        bytes
    }
}

/// The CMYK test profile as littleCMS writes it at `version`: B2A0 and
/// B2A1 tables of curves of gamma 1, the CMYK model's inks on a grid of 17
/// points and curves of gamma 1 again, a lut16 in a version 2 profile and a
/// lutBtoA in a version 4 one.
fn cmyk_profile(version: f64) -> Vec<u8> {
    // SAFETY: littleCMS builds the curve, the stages and the pipeline,
    // each stage holding its own copies of the curve, and samples the model
    // into the grid; the curve is freed after.
    let pipeline = unsafe {
        let curve = lcms::cmsBuildGamma(ptr::null_mut(), 1.0);
        assert!(!curve.is_null(), "littleCMS cannot build a curve of gamma 1");
        let pipeline = lcms::cmsPipelineAlloc(ptr::null_mut(), 3, 4);
        let before = lcms::cmsStageAllocToneCurves(ptr::null_mut(), 3, [curve as *const lcms::ToneCurve; 3].as_ptr());
        let grid = lcms::cmsStageAllocCLut16bit(ptr::null_mut(), 17, 3, 4, ptr::null());
        let after = lcms::cmsStageAllocToneCurves(ptr::null_mut(), 4, [curve as *const lcms::ToneCurve; 4].as_ptr());
        lcms::cmsFreeToneCurve(curve);
        assert!(!pipeline.is_null() && !before.is_null() && !grid.is_null() && !after.is_null(), "littleCMS cannot build the CMYK table");
        assert!(lcms::cmsStageSampleCLut16bit(grid, model_inks, ptr::null_mut(), 0) != 0, "littleCMS cannot sample the CMYK model");
        for stage in [before, grid, after] {
            assert!(lcms::cmsPipelineInsertStage(pipeline, lcms::StageLoc::AT_END, stage) != 0, "littleCMS cannot add a stage of the CMYK table");
        }
        pipeline
    };
    let bytes = written("the CMYK profile", version, lcms::ProfileClassSignature::OutputClass, lcms::ColorSpaceSignature::CmykData, lcms::ColorSpaceSignature::LabData, &[(lcms::TagSignature::BToA0Tag, pipeline), (lcms::TagSignature::BToA1Tag, pipeline)]);
    // SAFETY: the profile holds its own copies; nothing else holds this one.
    unsafe { lcms::cmsPipelineFree(pipeline) };
    bytes
}

/// A curve set littleCMS builds, for the caller to free: three curves, each
/// a formula of its input raised to `gamma`, its type 6, (a x + b) to the
/// gamma plus c with a 1 and b and c 0, as littleCMS writes a curve into a
/// D2B or B2D table only from types 6 to 8.
fn power_curves(gamma: f64) -> *mut lcms::Stage {
    let params = [gamma, 1.0, 0.0, 0.0];
    // SAFETY: littleCMS builds the curve and the stage, which holds its own
    // copies of the curve; the curve is freed after.
    unsafe {
        let curve = lcms::cmsBuildParametricToneCurve(ptr::null_mut(), 6, params.as_ptr());
        assert!(!curve.is_null(), "littleCMS cannot build a curve of gamma {gamma}");
        let stage = lcms::cmsStageAllocToneCurves(ptr::null_mut(), 3, [curve as *const lcms::ToneCurve; 3].as_ptr());
        lcms::cmsFreeToneCurve(curve);
        assert!(!stage.is_null(), "littleCMS cannot build a curve set of gamma {gamma}");
        stage
    }
}

/// The inverse of the 3 by 3 matrix `m`.
fn inverse(m: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let [[a, b, c], [d, e, f], [g, h, i]] = m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    assert!(det != 0.0, "the matrix has no inverse");
    [
        [(e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det],
        [(f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det],
        [(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
    ]
}

/// A BToD1 table littleCMS builds, for the caller to free: XYZ to linear
/// sRGB by a matrix, then the RGB model's encoding on a grid of 9 points.
fn btod_pipeline() -> *mut lcms::Pipeline {
    let rows: Vec<f64> = XYZ_TO_LINEAR_SRGB.iter().flatten().copied().collect();
    let offset = [0.0f64; 3];
    // SAFETY: littleCMS builds the pipeline, copying the matrix, and
    // samples the model into its grid.
    unsafe {
        let pipeline = lcms::cmsPipelineAlloc(ptr::null_mut(), 3, 3);
        let matrix = lcms::cmsStageAllocMatrix(ptr::null_mut(), 3, 3, rows.as_ptr(), offset.as_ptr());
        let grid = lcms::cmsStageAllocCLutFloat(ptr::null_mut(), 9, 3, 3, ptr::null());
        assert!(!pipeline.is_null() && !matrix.is_null() && !grid.is_null(), "littleCMS cannot build the BToD1 table");
        assert!(lcms::cmsStageSampleCLutFloat(grid, model_encoding, ptr::null_mut(), 0) != 0, "littleCMS cannot sample the RGB model");
        assert!(lcms::cmsPipelineInsertStage(pipeline, lcms::StageLoc::AT_END, matrix) != 0, "littleCMS cannot add the BToD1 matrix");
        assert!(lcms::cmsPipelineInsertStage(pipeline, lcms::StageLoc::AT_END, grid) != 0, "littleCMS cannot add the BToD1 grid");
        pipeline
    }
}

/// The RGB test profile of a BToD1 table, version 4 of an XYZ PCS: XYZ to
/// linear sRGB by a matrix, then the RGB model's encoding on a grid of 9
/// points.
fn btod_profile() -> Vec<u8> {
    let pipeline = btod_pipeline();
    let bytes = written("the BToD1 profile", 4.3, lcms::ProfileClassSignature::DisplayClass, lcms::ColorSpaceSignature::RgbData, lcms::ColorSpaceSignature::XYZData, &[(lcms::TagSignature::BToD1Tag, pipeline)]);
    // SAFETY: the profile holds its own copy; nothing else holds this one.
    unsafe { lcms::cmsPipelineFree(pipeline) };
    bytes
}

/// The RGB test profile of tables both ways and no matrix-shaper's curves,
/// version 4 of an XYZ PCS: a DToB1 table of the RGB model's decoding, each
/// channel raised to 2.2 by a formula, then linear sRGB to XYZ by a matrix,
/// and a BToD1 table of the same back, so a color comes back as it went.
fn table_rgb_profile() -> Vec<u8> {
    let into_xyz: Vec<f64> = inverse(XYZ_TO_LINEAR_SRGB).iter().flatten().copied().collect();
    let out_of_xyz: Vec<f64> = XYZ_TO_LINEAR_SRGB.iter().flatten().copied().collect();
    let offset = [0.0f64; 3];
    // SAFETY: littleCMS builds the pipelines and the matrices, copying the
    // rows, and each pipeline holds the stages put into it.
    let (reading, writing) = unsafe {
        let reading = lcms::cmsPipelineAlloc(ptr::null_mut(), 3, 3);
        let writing = lcms::cmsPipelineAlloc(ptr::null_mut(), 3, 3);
        let into = lcms::cmsStageAllocMatrix(ptr::null_mut(), 3, 3, into_xyz.as_ptr(), offset.as_ptr());
        let out = lcms::cmsStageAllocMatrix(ptr::null_mut(), 3, 3, out_of_xyz.as_ptr(), offset.as_ptr());
        assert!(!reading.is_null() && !writing.is_null() && !into.is_null() && !out.is_null(), "littleCMS cannot build the RGB table profile's tables");
        for (pipeline, stage) in [(reading, power_curves(2.2)), (reading, into), (writing, out), (writing, power_curves(1.0 / 2.2))] {
            assert!(lcms::cmsPipelineInsertStage(pipeline, lcms::StageLoc::AT_END, stage) != 0, "littleCMS cannot add a stage of the RGB table profile");
        }
        (reading, writing)
    };
    let bytes = written("the RGB table profile", 4.3, lcms::ProfileClassSignature::DisplayClass, lcms::ColorSpaceSignature::RgbData, lcms::ColorSpaceSignature::XYZData, &[(lcms::TagSignature::DToB1Tag, reading), (lcms::TagSignature::BToD1Tag, writing)]);
    // SAFETY: the profile holds its own copies; nothing else holds these.
    unsafe {
        lcms::cmsPipelineFree(reading);
        lcms::cmsPipelineFree(writing);
    }
    bytes
}

/// The two colors of the RGB table profile the gradient light test mixes.
const MIXED_COLORS: [[f64; 3]; 2] = [[0.875, 0.25, 0.125], [0.125, 0.375, 0.75]];

/// Where between the two the test mixes them, from 0 to 1.
const MIX_PLACES: [f64; 5] = [0.0, 0.25, 0.5, 0.75, 1.0];

/// MIXED_COLORS mixed at each of MIX_PLACES in the XYZ of `profile`'s
/// tables: each color taken to XYZ relative colorimetrically with
/// littleCMS's optimizations off, the two XYZ mixed in a straight line, and
/// each mix taken back the same way, held within 0 to 1.
fn mixed_in_xyz(profile: &[u8]) -> Vec<Vec<f64>> {
    let rgb = opened("the RGB table profile", profile);
    // SAFETY: littleCMS builds the XYZ profile in its default context, and
    // both handles are open profiles it gave.
    let (xyz, to_xyz, from_xyz) = unsafe {
        let xyz = lcms::cmsCreateXYZProfile();
        assert!(!xyz.is_null(), "littleCMS cannot build an XYZ profile");
        let to = lcms::cmsCreateTransform(rgb, lcms::PixelFormat::RGB_DBL, xyz, lcms::PixelFormat::XYZ_DBL, lcms::Intent::RelativeColorimetric, lcms::FLAGS_NOOPTIMIZE);
        let from = lcms::cmsCreateTransform(xyz, lcms::PixelFormat::XYZ_DBL, rgb, lcms::PixelFormat::RGB_DBL, lcms::Intent::RelativeColorimetric, lcms::FLAGS_NOOPTIMIZE);
        assert!(!to.is_null() && !from.is_null(), "littleCMS cannot convert between the RGB table profile and XYZ");
        (xyz, to, from)
    };
    let mut ends = [[0f64; 3]; 2];
    for (color, end) in MIXED_COLORS.iter().zip(&mut ends) {
        // SAFETY: the transform reads one color of three doubles and writes
        // one XYZ of three, which `end` holds.
        unsafe { lcms::cmsDoTransform(to_xyz, color.as_ptr().cast::<c_void>(), end.as_mut_ptr().cast::<c_void>(), 1) };
    }
    let mixes = MIX_PLACES
        .iter()
        .map(|&t| {
            let mixed = [0, 1, 2].map(|k| ends[0][k] + (ends[1][k] - ends[0][k]) * t);
            let mut out = [0f64; 3];
            // SAFETY: the transform reads one XYZ of three doubles and
            // writes one color of three, which `out` holds.
            unsafe { lcms::cmsDoTransform(from_xyz, mixed.as_ptr().cast::<c_void>(), out.as_mut_ptr().cast::<c_void>(), 1) };
            out.iter().map(|v| v.clamp(0.0, 1.0)).collect()
        })
        .collect();
    // SAFETY: the transforms are littleCMS's and no one uses them again.
    unsafe {
        lcms::cmsDeleteTransform(to_xyz);
        lcms::cmsDeleteTransform(from_xyz);
    }
    closed("littleCMS's XYZ profile", xyz);
    closed("the RGB table profile", rgb);
    mixes
}

/// `colors` converted from `source`, taking `input` doubles, into `target`
/// relative colorimetrically with littleCMS's optimizations off, `channels`
/// doubles each, divided by `scale`, held within 0 to 1.
#[allow(clippy::too_many_arguments)]
fn into(name: &str, source: lcms::HPROFILE, input: lcms::PixelFormat, target: &[u8], output: lcms::PixelFormat, channels: usize, scale: f64, colors: &[[f64; 3]]) -> Vec<Vec<f64>> {
    let profile = opened(name, target);
    // SAFETY: both handles are open profiles littleCMS gave.
    let transform = unsafe { lcms::cmsCreateTransform(source, input, profile, output, lcms::Intent::RelativeColorimetric, lcms::FLAGS_NOOPTIMIZE) };
    assert!(!transform.is_null(), "littleCMS cannot convert into {name}");
    let values: Vec<f64> = colors.iter().flatten().copied().collect();
    let mut out = vec![0f64; colors.len() * channels];
    let count = u32::try_from(colors.len()).expect("a few colors");
    // SAFETY: the transform reads `count` colors of three doubles and
    // writes as many of `channels`, which `out` holds.
    unsafe {
        lcms::cmsDoTransform(transform, values.as_ptr().cast::<c_void>(), out.as_mut_ptr().cast::<c_void>(), count);
        lcms::cmsDeleteTransform(transform);
    }
    closed(name, profile);
    out.chunks_exact(channels).map(|c| c.iter().map(|v| (v / scale).clamp(0.0, 1.0)).collect()).collect()
}

fn rust_colors(name: &str, doc: &str, rows: &[Vec<f64>]) -> String {
    let width = rows.first().map_or(0, Vec::len);
    let mut out = format!("/// {doc}\npub(crate) static {name}: [[f64; {width}]; {}] = [\n", rows.len());
    for row in rows {
        let listed: Vec<String> = row.iter().map(|v| format!("{v:?}")).collect();
        writeln!(out, "    [{}],", listed.join(", ")).expect("a string takes every write");
    }
    out.push_str("];");
    out
}

fn rust_numbers(name: &str, doc: &str, values: &[f64]) -> String {
    let listed: Vec<String> = values.iter().map(|v| format!("{v:?}")).collect();
    format!("/// {doc}\npub(crate) static {name}: [f64; {}] = [{}];", values.len(), listed.join(", "))
}

/// The parts of src/fill_reference.rs.
fn fill_references() -> Vec<String> {
    let cmyk_v2 = cmyk_profile(2.1);
    let cmyk_v4 = cmyk_profile(4.3);
    let btod = btod_profile();
    let table_rgb = table_rgb_profile();
    let mut parts = vec![
        [
            "//! Reference values for image_decode::tests' fill colors, written by",
            "//! tools/float_reference from littleCMS 2.19: sRGB and Lab colors",
            "//! converted relative colorimetrically into each profile, with",
            "//! littleCMS's optimizations off, each value held within 0 to 1. The",
            "//! profiles are profile_reference's and the four littleCMS writes",
            "//! here; two colors of the last are also mixed in the XYZ of its",
            "//! tables, as a gradient of the linear method mixes them.",
        ]
        .join("\n"),
        rust_colors("FILL_SRGB", "The sRGB colors every fill test converts.", &FILL_SRGB.map(|c| c.to_vec())),
        rust_colors("FILL_LAB", "The Lab colors every fill test converts.", &FILL_LAB.map(|c| c.to_vec())),
        rust_bytes("CMYK_V2_PROFILE", "A version 2 CMYK output profile of a Lab PCS whose B2A0 and B2A1 are lut16 tables of the CMYK model, as littleCMS writes it.", &cmyk_v2),
        rust_bytes("CMYK_V4_PROFILE", "A version 4 CMYK output profile of a Lab PCS whose B2A0 and B2A1 are lutBtoA tables of the CMYK model, as littleCMS writes it.", &cmyk_v4),
        rust_bytes("BTOD_RGB_PROFILE", "A version 4 RGB profile of an XYZ PCS whose BToD1 table is a matrix to linear sRGB and a grid of the RGB model's encoding, as littleCMS writes it.", &btod),
        rust_bytes("TABLE_RGB_PROFILE", "A version 4 RGB profile of an XYZ PCS with tables both ways and no matrix-shaper's curves: a DToB1 table of the RGB model's decoding by formula curves and a matrix to XYZ, and a BToD1 table of the same back, as littleCMS writes it.", &table_rgb),
        rust_colors("MIXED_COLORS", "The two colors of TABLE_RGB_PROFILE the gradient light test mixes.", &MIXED_COLORS.map(|c| c.to_vec())),
        rust_numbers("MIX_PLACES", "Where between the two the test mixes them, from 0 to 1.", &MIX_PLACES),
        rust_colors("TABLE_RGB_MIXED", "MIXED_COLORS mixed in a straight line at each of MIX_PLACES in the XYZ of TABLE_RGB_PROFILE's tables, as littleCMS takes them there and back.", &mixed_in_xyz(&table_rgb)),
    ];
    let targets: [(&str, &[u8], lcms::PixelFormat, usize, f64); 10] = [
        ("SRGB", &reference::SRGB_PROFILE, lcms::PixelFormat::RGB_DBL, 3, 1.0),
        ("DISPLAYP3", &reference::DISPLAYP3_PROFILE, lcms::PixelFormat::RGB_DBL, 3, 1.0),
        ("ADOBE", &reference::ADOBE_PROFILE, lcms::PixelFormat::RGB_DBL, 3, 1.0),
        ("PROPHOTO", &reference::PROPHOTO_PROFILE, lcms::PixelFormat::RGB_DBL, 3, 1.0),
        ("REC2020", &reference::REC2020_PROFILE, lcms::PixelFormat::RGB_DBL, 3, 1.0),
        ("SGREY", &reference::SGREY_PROFILE, lcms::PixelFormat::GRAY_DBL, 1, 1.0),
        ("LAB_GRAY", &reference::LAB_GRAY_PROFILE, lcms::PixelFormat::GRAY_DBL, 1, 1.0),
        ("CMYK_V2", &cmyk_v2, lcms::PixelFormat::CMYK_DBL, 4, 100.0),
        ("CMYK_V4", &cmyk_v4, lcms::PixelFormat::CMYK_DBL, 4, 100.0),
        ("BTOD_RGB", &btod, lcms::PixelFormat::RGB_DBL, 3, 1.0),
    ];
    // SAFETY: littleCMS builds both in its default context.
    let (srgb, lab) = unsafe { (lcms::cmsCreate_sRGBProfile(), lcms::cmsCreateLab4Profile(ptr::null())) };
    assert!(!srgb.is_null() && !lab.is_null(), "littleCMS cannot build sRGB and Lab");
    for (name, bytes, format, channels, scale) in targets {
        let from_srgb = into(name, srgb, lcms::PixelFormat::RGB_DBL, bytes, format, channels, scale, &FILL_SRGB);
        parts.push(rust_colors(&format!("{name}_FROM_SRGB"), &format!("FILL_SRGB converted into {name}_PROFILE."), &from_srgb));
        let from_lab = into(name, lab, lcms::PixelFormat::Lab_DBL, bytes, format, channels, scale, &FILL_LAB);
        parts.push(rust_colors(&format!("{name}_FROM_LAB"), &format!("FILL_LAB converted into {name}_PROFILE."), &from_lab));
        println!("converted the fill colors into {name}");
    }
    closed("littleCMS's sRGB", srgb);
    closed("littleCMS's Lab", lab);
    parts
}
