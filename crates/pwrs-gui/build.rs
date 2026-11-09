//! Links the library and the egui helper, which both contain the egui
//! renderer, with `opengl32.dll` delay-loaded on Windows: Windows Server
//! Core has no such library, and a static import would stop either from
//! loading at all. The renderer leaves OpenGL out wherever the system
//! cannot load it ([`pwrs_gui_egui::backends`]), so nothing calls into it
//! there. The web helper links no renderer and takes nothing here.
//!
//! On MSVC the egui helper also takes PWRS-GUI's icon as its own, which
//! Task Manager, the taskbar's settings and a helper window with no icon of
//! its own show. This script writes the icon's resource file from
//! `assets/PWRS-GUI.ico`, its images as RT_ICON resources and their
//! directory as an RT_GROUP_ICON, and the linker takes the file as it is.

use std::path::{Path, PathBuf};

/// The resource type of one of an icon's images.
const RT_ICON: u16 = 3;
/// The resource type of an icon's directory of images.
const RT_GROUP_ICON: u16 = 14;
/// U.S. English, the language rc.exe gives a resource unless told another.
const LANGUAGE: u16 = 0x0409;
/// The memory flags rc.exe writes for an icon's image: movable and
/// discardable.
const IMAGE_FLAGS: u16 = 0x1010;
/// The memory flags rc.exe writes for an icon's directory: movable, pure
/// and discardable.
const GROUP_FLAGS: u16 = 0x1030;

fn main() -> Result<(), String> {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if windows && msvc {
        for flag in ["/DELAYLOAD:opengl32.dll", "delayimp.lib"] {
            println!("cargo:rustc-link-arg-cdylib={flag}");
            println!("cargo:rustc-link-arg-bin=pwrs-gui-egui={flag}");
        }
        let res = icon_resource()?;
        println!("cargo:rustc-link-arg-bin=pwrs-gui-egui={}", res.display());
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/PWRS-GUI.ico");
    Ok(())
}

/// Writes the resource file of `assets/PWRS-GUI.ico` into the build's
/// output folder, and returns its path.
fn icon_resource() -> Result<PathBuf, String> {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").map_err(|e| format!("CARGO_MANIFEST_DIR cannot be read: {e}"))?;
    let ico_path = Path::new(&manifest).join("assets").join("PWRS-GUI.ico");
    let ico = std::fs::read(&ico_path).map_err(|e| format!("{} cannot be read: {e}", ico_path.display()))?;
    let res = resource_of_icon(&ico).map_err(|e| format!("{}: {e}", ico_path.display()))?;
    let out = std::env::var("OUT_DIR").map_err(|e| format!("OUT_DIR cannot be read: {e}"))?;
    let res_path = Path::new(&out).join("pwrs-gui-egui-icon.res");
    std::fs::write(&res_path, res).map_err(|e| format!("{} cannot be written: {e}", res_path.display()))?;
    Ok(res_path)
}

/// The resource file of the icon file `ico`: the empty resource every
/// 32-bit resource file opens with, each image as an RT_ICON numbered from
/// 1 in the icon's order, and the directory naming them as RT_GROUP_ICON 1.
fn resource_of_icon(ico: &[u8]) -> Result<Vec<u8>, String> {
    let u16_at = |at: usize| match ico.get(at..at + 2) {
        Some(b) => Ok(u16::from_le_bytes([b[0], b[1]])),
        None => Err(format!("the file ends at byte {}, inside its directory", ico.len())),
    };
    let u32_at = |at: usize| match ico.get(at..at + 4) {
        Some(b) => Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        None => Err(format!("the file ends at byte {}, inside its directory", ico.len())),
    };
    if u16_at(0)? != 0 || u16_at(2)? != 1 {
        return Err(String::from("the file's header is not an icon's"));
    }
    let count = u16_at(4)?;
    if count == 0 {
        return Err(String::from("the icon holds no image"));
    }
    let mut res = Vec::new();
    resource(&mut res, 0, 0, 0, 0, &[])?;
    let mut group = Vec::with_capacity(6 + 14 * usize::from(count));
    for word in [0u16, 1, count] {
        group.extend(word.to_le_bytes());
    }
    for index in 0..count {
        let at = 6 + 16 * usize::from(index);
        let fixed = ico.get(at..at + 8).ok_or_else(|| format!("the directory's entry {} is cut short", index + 1))?;
        let size = u32_at(at + 8)?;
        let offset = u32_at(at + 12)?;
        let start = usize::try_from(offset).map_err(|e| format!("image {} starts beyond this platform's addresses: {e}", index + 1))?;
        let length = usize::try_from(size).map_err(|e| format!("image {} is longer than this platform's addresses: {e}", index + 1))?;
        let image = start.checked_add(length).and_then(|end| ico.get(start..end)).ok_or_else(|| format!("image {} runs past the file's end", index + 1))?;
        let id = index + 1;
        resource(&mut res, RT_ICON, id, IMAGE_FLAGS, LANGUAGE, image)?;
        group.extend_from_slice(fixed);
        group.extend(size.to_le_bytes());
        group.extend(id.to_le_bytes());
    }
    resource(&mut res, RT_GROUP_ICON, 1, GROUP_FLAGS, LANGUAGE, &group)?;
    Ok(res)
}

/// Appends one resource to `res`: its 32-byte header, its type and its
/// name given by number, then its data, padded to a multiple of four
/// bytes.
fn resource(res: &mut Vec<u8>, kind: u16, name: u16, flags: u16, language: u16, data: &[u8]) -> Result<(), String> {
    let size = u32::try_from(data.len()).map_err(|e| format!("a resource of {} bytes is too large: {e}", data.len()))?;
    res.extend(size.to_le_bytes());
    res.extend(32u32.to_le_bytes());
    for word in [0xFFFF, kind, 0xFFFF, name] {
        res.extend(word.to_le_bytes());
    }
    res.extend(0u32.to_le_bytes());
    res.extend(flags.to_le_bytes());
    res.extend(language.to_le_bytes());
    res.extend(0u32.to_le_bytes());
    res.extend(0u32.to_le_bytes());
    res.extend_from_slice(data);
    while !res.len().is_multiple_of(4) {
        res.push(0);
    }
    Ok(())
}
