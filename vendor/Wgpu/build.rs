//! Compiles Basis Universal from vendor/basis_universal: its encoder, its
//! transcoder and the zstd it ships, with the C APIs src/basis.rs calls,
//! as its own CMakeLists.txt builds them, with SSE4.1 left out so the
//! library runs on every CPU of its architecture. The compilers' warnings
//! about this third-party code are not shown. native/basisu_output.h is
//! forced into every basisu source, so what basisu prints reaches the
//! module through native/basisu_output.c instead of the console.
//!
//! On Linux the library is linked with `-z nodelete`, so it stays mapped
//! after a process unloads it: zig's link gives it nothing that removes
//! the destructors its C++ code registers with `__cxa_atexit`, and they
//! run at the process's exit.

use std::path::Path;

/// The C++ sources, as CMakeLists.txt lists them for the encoder library,
/// with the Android ASTC decompressor and the two C API files.
const SOURCES: &[&str] = &[
    "encoder/basisu_backend.cpp",
    "encoder/basisu_basis_file.cpp",
    "encoder/basisu_comp.cpp",
    "encoder/basisu_enc.cpp",
    "encoder/basisu_etc.cpp",
    "encoder/basisu_frontend.cpp",
    "encoder/basisu_gpu_texture.cpp",
    "encoder/basisu_pvrtc1_4.cpp",
    "encoder/basisu_resampler.cpp",
    "encoder/basisu_resample_filters.cpp",
    "encoder/basisu_ssim.cpp",
    "encoder/basisu_uastc_enc.cpp",
    "encoder/basisu_bc7e_scalar.cpp",
    "encoder/basisu_dds_export.cpp",
    "encoder/basisu_bc7enc.cpp",
    "encoder/jpgd.cpp",
    "encoder/basisu_kernels_sse.cpp",
    "encoder/basisu_bc15_spmd.cpp",
    "encoder/basisu_bc15_spmd_sse.cpp",
    "encoder/basisu_opencl.cpp",
    "encoder/pvpngreader.cpp",
    "encoder/basisu_uastc_hdr_4x4_enc.cpp",
    "encoder/basisu_astc_hdr_6x6_enc.cpp",
    "encoder/basisu_astc_hdr_common.cpp",
    "encoder/basisu_astc_ldr_common.cpp",
    "encoder/basisu_astc_ldr_encode.cpp",
    "encoder/basisu_astc_ldr_fencode.cpp",
    "encoder/basisu_xbc7_encode.cpp",
    "encoder/basisu_tinyexr.cpp",
    "encoder/3rdparty/android_astc_decomp.cpp",
    "encoder/basisu_wasm_api.cpp",
    "encoder/basisu_wasm_transcoder_api.cpp",
    "transcoder/basisu_transcoder.cpp",
];

fn main() {
    let root = Path::new("vendor/basis_universal");
    let output_header = Path::new(env!("CARGO_MANIFEST_DIR")).join("native/basisu_output.h");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=vendor/basis_universal");
    println!("cargo:rerun-if-changed=native");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").expect("cargo sets CARGO_CFG_TARGET_OS for a build script");
    if target_os == "linux" {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-z,nodelete");
    }
    // rav1d's x86-64 assembly reaches rav1d's exported tables by direct
    // relocations, which an ELF shared library takes only when the symbols
    // it defines bind inside it.
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").expect("cargo sets CARGO_CFG_TARGET_ARCH for a build script");
    if target_arch == "x86_64" && matches!(target_os.as_str(), "linux" | "freebsd") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-Bsymbolic");
    }

    let mut cpp = cc::Build::new();
    cpp.cpp(true)
        .std("c++17")
        .warnings(false)
        .flag_if_supported("-w")
        .flag_if_supported("/w")
        .flag_if_supported("-fno-strict-aliasing")
        .define("_LARGEFILE64_SOURCE", "1")
        .define("_FILE_OFFSET_BITS", "64")
        .define("BASISU_SUPPORT_SSE", "0")
        .define("BASISU_SUPPORT_OPENCL", "0")
        .define("BASISU_DISABLE_ANDROID_ASTC_DECOMP", "0")
        .define("BASISD_SUPPORT_KTX2_ZSTD", "1")
        .include(root.join("encoder"))
        .include(root.join("transcoder"))
        .include(root.join("zstd"));
    if cpp.get_compiler().is_like_msvc() {
        cpp.flag(format!("/FI{}", output_header.display()));
    } else {
        cpp.flag("-include").flag(output_header.as_os_str());
    }
    for source in SOURCES {
        cpp.file(root.join(source));
    }
    cpp.compile("basisu");

    cc::Build::new()
        .warnings(false)
        .flag_if_supported("-w")
        .flag_if_supported("/w")
        .file(root.join("zstd/zstd.c"))
        .compile("basisu_zstd");

    cc::Build::new().warnings(true).warnings_into_errors(true).file("native/basisu_output.c").compile("basisu_output");
}
