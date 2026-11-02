---
title: Building From Source
weight: 1
---

Build the module from a checkout of its repository: install what the build needs, build the module folder and check what it holds, build it for the other platforms and join them into one module, and import it in PowerShell 7 and in Windows PowerShell 5.1. Installing from the PowerShell Gallery, as [Getting Started](Getting-Started.md) does, needs none of this.

## Install what the build needs

- Rust 1.99.0, which `rust-toolchain.toml` names: rustup installs it, with clippy and rustfmt, beside the default toolchain the first time a command runs in the checkout. `Cargo.toml` declares `rust-version = "1.99"`, so Cargo refuses an older Rust.
- cargo-pwrs 0.8.0, the build tool of [PoWerRuSt](https://crates.io/crates/PoWerRuSt) 0.8.0, the crate `Cargo.toml` names and `Cargo.lock` pins: `cargo install cargo-pwrs --version 0.8.0 --locked`. The crate and cargo-pwrs must be the same version, as PWRS's README says. PWRS's [Getting Started](https://variably-constant.github.io/PWRS/docs/tutorials/getting-started/) lists what cargo-pwrs needs beside Rust.
- A C and C++ compiler, for Basis Universal: on Windows, the Visual Studio 2022 Build Tools with the C++ build tools for the target's architecture; on macOS, Xcode's command line tools; on Linux, zig, below; on FreeBSD, the base system's clang.
- For an x64 library, [nasm](https://www.nasm.us/) on `PATH`: the library reads AVIF through rav1d, the Rust port of the dav1d decoder, whose x86-64 assembly nasm assembles. On Windows, `winget install NASM.NASM`, then add the folder it installs into to `PATH`: `C:\Program Files\NASM`, or `%LOCALAPPDATA%\bin\NASM` when it installs for one user; on macOS, `brew install nasm`; on Ubuntu, `apt install nasm`; on FreeBSD, `pkg install nasm`. An Arm64 library needs none.
- On Windows x64, also the Build Tools' ATL component, `Microsoft.VisualStudio.Component.VC.ATL`: wgpu links DXC, the DirectX shader compiler, into the library there, and DXC uses ATL.
- For Linux, [zig](https://ziglang.org/) 0.16 and [cargo-zigbuild](https://github.com/rust-cross/cargo-zigbuild) on `PATH`, which build the library against glibc 2.35, so it loads on any distribution with that glibc or a later one, and compile Basis Universal with zig's C and C++ compilers. Install cargo-zigbuild from its repository at the commit that leaves out the `-O1` linker setting zig 0.16 does not take, with `cargo install --git https://github.com/rust-cross/cargo-zigbuild --rev d9f7fd32fbac13cc26d23847835f7f23a5ed3a25 --locked cargo-zigbuild`; with cargo-zigbuild 0.23.4 from crates.io, a Linux build prints zig's warning that it ignores that setting.

The module's own build compiles C and C++: `build.rs` builds Basis Universal 2.50 and the zstd it carries from `vendor/basis_universal`, and `native/basisu_output.c`, with the cc crate, which finds the compiler above. A Windows build also reaches github.com for DXC. On Windows x64 it downloads DXC prebuilt, with the `curl` Windows carries, from a pinned release of [mach-dxcompiler](https://github.com/DouglasDwyer/mach-dxcompiler), and checks the archive's SHA-256. On Windows Arm64, which links no DXC, cargo-pwrs downloads the [DirectXShaderCompiler](https://github.com/microsoft/DirectXShaderCompiler) release archive `Cargo.toml` names, once, checks it against the SHA-256 it pins, and puts the archive's `dxcompiler.dll` beside the native library, with its license texts in that library's notices; Direct3D 12 compiles shaders with it there.

## Build the module

From the repository's root, with `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS` unset, since either replaces the flags `.cargo/config.toml` gives every build:

```text
cargo pwrs build --release
```

`.cargo/config.toml` builds every x64 library for the x86-64 baseline and the macOS Arm64 one for the Apple A14, whose cores the M1 shares, so a library runs on any CPU of its architecture, and links the C runtime into the Windows library, so the module needs no Visual C++ Redistributable. The release profile in `Cargo.toml` builds with fat link-time optimization and one codegen unit.

## Check what it wrote

The module folder is `target\pwrs\Wgpu`, under `CARGO_TARGET_DIR` when that is set. The release build on the machine [Where It Has Been Run](Where-It-Has-Been-Run.md) describes wrote these files, with their sizes in bytes:

```text
THIRD-PARTY-NOTICES.txt 7513
Wgpu.Format.ps1xml 67624
Wgpu.psd1 4435
Wgpu.psm1 5390
net10.0\Pwrs.Bootstrap.dll 9216
net10.0\Pwrs.Runtime.dll 99840
net10.0\Wgpu.Shell.d2283a519dfda48a.dll 291840
net10.0\en-US\Wgpu.Shell.d2283a519dfda48a.dll-Help.xml 627855
netstandard2.0\Microsoft.Win32.Registry.dll 31376
netstandard2.0\Pwrs.Bootstrap.dll 8704
netstandard2.0\Pwrs.Runtime.dll 116736
netstandard2.0\System.IO.FileSystem.AccessControl.dll 27792
netstandard2.0\System.Security.AccessControl.dll 39048
netstandard2.0\System.Security.Principal.Windows.dll 23688
netstandard2.0\Wgpu.Shell.d2283a519dfda48a.dll 292352
netstandard2.0\en-US\Wgpu.Shell.d2283a519dfda48a.dll-Help.xml 627855
runtimes\win-x64\pwrs-build.json 152
runtimes\win-x64\THIRD-PARTY-NOTICES.txt 1660903
runtimes\win-x64\native\wgpu_pwsh.dll 50798080
```

Beside the manifest, `Wgpu.psd1`, and `Wgpu.psm1`, it holds the managed assemblies for PowerShell 7 under `net10.0` and for Windows PowerShell 5.1 under `netstandard2.0`, each with the cmdlets' help, and under `netstandard2.0` also the four assemblies that build references, `Microsoft.Win32.Registry`, `System.IO.FileSystem.AccessControl`, `System.Security.AccessControl` and `System.Security.Principal.Windows`; the formats of the module's classes; the native library under `runtimes\win-x64\native`, and beside its folder `pwrs-build.json`, cargo-pwrs's record of the build, which `cargo pwrs merge` carries with each platform's runtime; and two `THIRD-PARTY-NOTICES.txt` files, one at its root for the assemblies PWRS compiles and the four beside them, and one beside the native library's folder with the license of each crate compiled into it, of DXC on Windows x64 and of the `dxcompiler.dll` beside the library on Windows Arm64, and of the code the module's own build compiles: Basis Universal, with the zstd, tinyexr, qoi, tinydds and Android ASTC decoder it carries, block_compression's shaders, and the tone-mapping curves ported from three.js. It quotes from `licenses/` in the repository the texts the packages leave out, and, last among the module's own texts, where the source of avif-parse is, the one crate under the Mozilla Public License 2.0, which that license requires a binary that carries it to say.

`dumpbin /dependents` on that library lists only DLLs that are part of Windows: `KERNEL32.dll`, again as `kernel32.dll`, `oleaut32.dll`, `api-ms-win-core-synch-l1-2-0.dll`, `opengl32.dll`, `setupapi.dll`, `user32.dll`, `dxgi.dll`, `gdi32.dll`, `combase.dll`, `bcryptprimitives.dll`, `ole32.dll` and `ntdll.dll`. wgpu loads the Direct3D 12 and Vulkan libraries when it looks for adapters, and lists none through an API the machine lacks.

## Build for the other platforms

Build each platform's library with `--target`: `aarch64-pc-windows-msvc` for Windows Arm64, `x86_64-unknown-linux-gnu.2.35` and `aarch64-unknown-linux-gnu.2.35` for Linux, which cargo-pwrs builds through cargo-zigbuild, `aarch64-apple-darwin` and `x86_64-apple-darwin` on a Mac, and `x86_64-unknown-freebsd` on FreeBSD. Each writes a module folder whose `runtimes` holds that platform's library. Then join the folders into one that carries all seven:

```text
cargo pwrs merge target\pwrs\Wgpu <module folder of another platform>...
```

## Import the folder

```powershell
Import-Module .\target\pwrs\Wgpu\Wgpu.psd1
Get-Command -Module Wgpu | Group-Object CommandType | Select-Object Name, Count
```

```text
Name   Count
----   -----
Alias     43
Cmdlet    43
```

The same folder imports into Windows PowerShell 5.1. From here the module behaves as the installed one does, and [Getting Started](Getting-Started.md) goes on from its first command. [How To Run The Tests](How-To-Run-The-Tests.md) tests the build.
