---
title: Building From Source
weight: 1
---

What the build needs, the source trees it reads beside the checkout, the build itself, and the module folder it writes. Source: `Cargo.toml` and `.cargo/config.toml` at the repository's root, and `crates/pwrs-gui/Cargo.toml`, whose `[package.metadata.pwrs]` names the helpers and the licenses the build allows.

The output on this page is from one build on Windows 11 Pro 10.0.26200 on an AMD Ryzen 9 7900X, of the code this page describes.

## What it needs

- Rust 1.99 or later, the `rust-version` every crate declares; on Windows the MSVC toolchain, with the Visual Studio Build Tools' C++ tools for its linker.
- PowerShell 7.4 or later on `PATH`, which cargo-pwrs runs to compile the module's C# shell.
- PWRS's source tree beside the checkout, as `../pwrs`: the workspace takes PoWerRuSt from `../pwrs/crates/pwrs` in place of crates.io's, for every crate that builds on it, the Wgpu module's among them, until PWRS's next release is published.
- SubEtha's source tree beside the checkout, as `../SubEtha`: the workspace takes `subetha-cxc` from `../SubEtha/crates/subetha-cxc`, since it needs `BlockingAdaptiveRing`, which no published SubEtha release has yet.
- The Wgpu module's source tree beside the checkout, as `../Wgpu`: the workspace takes its `wgpu-pwsh` crate, and the `wgpu-core` it vendors, from there, since neither is published.
- cargo-pwrs built from the same PWRS tree, `../pwrs/crates/cargo-pwrs`. cargo-pwrs embeds the runtime of the PoWerRuSt it is built with, so the two match.
- On Linux and FreeBSD, WebKitGTK 4.1's development files for the web renderer: `libwebkit2gtk-4.1-dev` on Ubuntu, `webkit2-gtk_41` on FreeBSD.
- git on `PATH`, or where there is none, as on a FreeBSD machine without it, `patch`, for `patches/apply.ps1`; and the first time, unless cargo has already downloaded the six crates it patches, a connection to static.crates.io.

```text
<folder>/
  PWRS-GUI/    this repository
  pwrs/        PWRS
  SubEtha/     SubEtha
  Wgpu/        the Wgpu module
```

Nothing else: the C# compiler is fetched by cargo-pwrs on its first run, the renderers' GPU and windowing libraries are loaded from the system when they run, and on Windows the web renderer draws with the system's WebView2.

## The build

First unpack and patch the crates.io crates the workspace patches:

```text
pwsh -File patches/apply.ps1
```

```text
PATCHED eframe-0.37.0 from C:\Users\<user>\.cargo\registry\cache\index.crates.io-1949cf8c6b5b557f\eframe-0.37.0.crate with eframe-0.37.0.patch
PATCHED winit-0.30.13 from C:\Users\<user>\.cargo\registry\cache\index.crates.io-1949cf8c6b5b557f\winit-0.30.13.crate with winit-0.30.13.patch
PATCHED tao-0.37.2 from C:\Users\<user>\AppData\Local\Temp\pwrs-gui-tao-0.37.2-74740.crate with tao-0.37.2.patch
PATCHED epaint-0.37.0 from C:\Users\<user>\AppData\Local\Temp\pwrs-gui-epaint-0.37.0-74740.crate with epaint-0.37.0.patch
PATCHED rfd-0.17.2 from C:\Users\<user>\.cargo\registry\cache\index.crates.io-1949cf8c6b5b557f\rfd-0.17.2.crate with rfd-0.17.2.patch
PATCHED egui-wgpu-0.37.0 from C:\Users\<user>\AppData\Local\Temp\pwrs-gui-egui-wgpu-0.37.0-74740.crate with egui-wgpu-0.37.0.patch
```

The workspace takes eframe from `patched/eframe-0.37.0`: crates.io's 0.37.0 with `patches/eframe-0.37.0.patch`, which gives every window an AccessKit adapter of its own, where eframe gives one only to its root, so screen readers reach each egui window. It takes winit 0.30.13 and tao 0.37.2 from `patched/` too, each patched so that macOS does not bring a renderer to the front at launch when it opens its windows without the focus (`PWRS_GUI_NO_FOCUS`). tao's patch also lets a window built without the focus open without it on Windows, Linux and the BSDs, brings window requests, device events and theme changes to GTK's main loop on Linux and the BSDs, and gives the BSDs Linux's progress bar and windows on every workspace.

It takes epaint 0.37.0 patched so that a canvas can draw its text from the outlines of the glyphs egui lays it out in. egui-wgpu 0.37.0 is patched so that a window whose surface setup another thread's GPU work interrupts is set up again, and so that a lost device draws nothing rather than ending the renderer. rfd 0.17.2 is patched so that a dialog the XDG portal shows can be closed, and a portal that cannot show one is told apart from a cancel.

`apply.ps1` takes each crate from cargo's registry cache, or else from static.crates.io, as it did above for the crates cargo never downloads because the workspace patches them. It refuses a crate unless its SHA-256 is the one the script names, and applies the patch with git, or with `patch` where git is not installed. It does nothing when `patched/` already has the same crate and patch, so it can run before every build, and cargo cannot load the workspace until it has run once.

Then install cargo-pwrs from PWRS's tree into the repository's own `target`, and build the module:

```text
cargo install --path ../pwrs/crates/cargo-pwrs --root target/tools --locked
target/tools/bin/cargo-pwrs pwrs build --release --manifest-dir crates/pwrs-gui
```

```text
    Finished `release` profile [optimized] target(s) in 4m 54s
pwrs: module folder C:\...\PWRS-GUI\target\pwrs\PWRS-GUI
```

The release profile links the whole program at once (`lto = "fat"`, one codegen unit) and leaves unwinding on, so a panic is caught at the module's boundary. `.cargo/config.toml` gives every target the x86-64 baseline, so the module imports on any x86-64 CPU whatever CPU built it, and links the C runtime into the Windows library and helper, so they load without the Visual C++ Redistributable. A `RUSTFLAGS` variable replaces those flags, so leave it unset.

cargo-pwrs prints a line for each crate linked into the library or a helper that has no license file of its own and has no text in the license tables of `crates/pwrs-gui/Cargo.toml`, and `cargo pwrs publish` refuses the module while any is left; the build above printed none.

## The module folder

`target/pwrs/PWRS-GUI` is the module, as the build above wrote it:

| File | Bytes | Is |
|---|---|---|
| `PWRS-GUI.psd1` | 1,579 | the manifest: both editions, PowerShell 5.1 and later, the 52 cmdlets |
| `PWRS-GUI.psm1` | 2,827 | the loader, which picks the folder for the running edition |
| `PWRS-GUI.Format.ps1xml` | 12,703 | the default views of the output types |
| `THIRD-PARTY-NOTICES.txt` | 1,753 | the notice for the assemblies PWRS compiles |
| `net10.0/`, `netstandard2.0/` | 565,986 and 583,394 | the shell, runtime and bootstrap assemblies and the help, for PowerShell 7 and Windows PowerShell 5.1 |
| `runtimes/win-x64/native/pwrs_gui.dll` | 24,871,936 | the module's library, with both in-process renderers |
| `runtimes/win-x64/native/pwrs-gui-egui.exe` | 21,355,008 | the egui helper |
| `runtimes/win-x64/native/pwrs-gui-web.exe` | 3,196,928 | the HTML helper |
| `runtimes/win-x64/THIRD-PARTY-NOTICES.txt` | 2,492,996 | the license of every crate linked into the library and the helpers |

Import it by its manifest:

<!-- skip: the path is relative to the repository's folder -->
```powershell
Import-Module ./target/pwrs/PWRS-GUI/PWRS-GUI.psd1
```

## The stand-in module

The tests also build `PwrsGuiStandin`, a module that takes PWRS-GUI's session through its native entry; its library includes no renderer:

```text
target/tools/bin/cargo-pwrs pwrs build --release --manifest-dir crates/pwrs-gui-standin
```

It lands beside the module, in `target/pwrs/PwrsGuiStandin`, where the Pester suites look for it.

## The add-ons

`pwrs-gui-addons` builds the [WebKitGTK add-on](The-WebKitGTK-Add-On.md) of the system it runs on, from that system's own packages, with no root and without touching the machine's package state:

```text
cargo run --release -p pwrs-gui-addons -- webkit <work folder> <output folder>
```

| On | It takes | It needs |
|---|---|---|
| Linux | Ubuntu 22.04's packages, through an apt state of its own in the work folder that reads jammy, jammy-updates and jammy-security | `apt-get`, `dpkg-deb`, the Ubuntu archive's keyring at `/usr/share/keyrings/ubuntu-archive-keyring.gpg`, `readelf`, GNU `tar` and `xz`, `glib-compile-schemas`, `update-mime-database` |
| FreeBSD | the release's quarterly packages, read from pkg.FreeBSD.org the way pkg reads them, the catalog checked against the keys in `/usr/share/keys/pkg` and each package against the catalog's SHA-256 | `pkg` (for the ABI), `fetch`, `sha256`, `openssl`, `readelf`, `tar`, `glib-compile-schemas`, `update-mime-database`; git comes from the catalog's `git-lite`, run from the work folder |

The work folder stores the downloads, so a second build fetches only what changed; its `stage` folder is made afresh by each build, so the build stops when one is there. The output folder receives `PWRS-GUI.WebKit.<rid>/<version>` and `PWRS-GUI.WebKit.<rid>.Source/<version>`, each a module with its manifest, and the build stops rather than write over either. The version is that of the WebKitGTK the add-on includes, without the distribution's own parts, and a fourth part counting the builds of it, as `2.50.4.0` for the Linux add-on's WebKitGTK 2.50.4.

The same tool builds the [Mesa add-on](The-Mesa-Add-On.md) on Windows, from Mesa's and LLVM's source, and the [WebView2 add-on](The-WebView2-Add-On.md) on any system; each page says how:

```text
cargo run --release -p pwrs-gui-addons -- mesa <work folder> <output folder>
cargo run --release -p pwrs-gui-addons -- webview2 <output folder>
```

## Development builds

For iteration, the `test-fast` profile has release optimization without the whole-program link, in `target/test-fast`, once `patches/apply.ps1` has run, as for the build:

```text
cargo build --profile test-fast -p pwrs-gui --bin pwrs-gui-egui --bin pwrs-gui-web
cargo test --profile test-fast --workspace
```

The client crate's tests start the helpers built for the same profile, so build them first, as the first line does. [How To Run The Tests](How-To-Run-The-Tests.md) has the full set.
