//! Links the library and the egui helper, which both contain the egui
//! renderer, with `opengl32.dll` delay-loaded on Windows: Windows Server
//! Core has no such library, and a static import would stop either from
//! loading at all. The renderer leaves OpenGL out wherever the system
//! cannot load it ([`pwrs_gui_egui::backends`]), so nothing calls into it
//! there. The web helper links no renderer and takes nothing here.

fn main() {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if windows && msvc {
        for flag in ["/DELAYLOAD:opengl32.dll", "delayimp.lib"] {
            println!("cargo:rustc-link-arg-cdylib={flag}");
            println!("cargo:rustc-link-arg-bin=pwrs-gui-egui={flag}");
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
