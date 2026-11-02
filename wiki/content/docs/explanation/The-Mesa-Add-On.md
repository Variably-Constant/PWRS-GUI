---
title: The Mesa Add-On
weight: 7
---

What the egui renderer draws OpenGL with on a Windows machine that has no GPU and no WARP, such as Windows Server Core. Also what the add-on includes, how a session finds and loads it, how it is built, and what its licenses require. Source: `crates/pwrs-gui-addons/src/mesa.rs` builds it, `crates/pwrs-gui-client/src/mesa.rs` finds it, and `crates/pwrs-gui-egui/src/lib.rs` loads it.

## When it is used

The egui renderer draws with DirectX 12, then Vulkan, then OpenGL. On a machine without a GPU, DirectX 12 draws through WARP, its renderer for the processor, and OpenGL through the GPU vendor's driver or not at all. Windows Server Core has neither WARP's `d3d10warp.dll` nor `vulkan-1.dll` nor `opengl32.dll`, so there no backend offers an adapter. The add-on `PWRS-GUI.Mesa.win-x64` includes Mesa's llvmpipe, which draws OpenGL on the processor, as an `opengl32.dll` of its own.

1. When `PWRS_GUI_MESA_ADDON` names a folder that has the add-on's `opengl32.dll`, OpenGL draws through it on any machine, including one with a GPU; `none` leaves OpenGL to the system.
2. Otherwise, where System32 has no `opengl32.dll`, OpenGL draws through the newest version of `PWRS-GUI.Mesa.win-x64` in the folders of the session's `PSModulePath`, in order. Where System32 has one, the add-on is left alone.

With neither on a machine without OpenGL, a window's renderer fails with "no GPU adapter was found on any backend", naming each library the system lacks and `Install-Module PWRS-GUI.Mesa.win-x64` as the remedy. `Get-GuiGpuAdapter` lists the add-on's adapter as `llvmpipe (LLVM 23.1.3, 256 bits)` on the `Gl` backend, of device type `Cpu`. With `PWRS_GUI_TRACE` set, the renderer writes the `opengl32.dll` it draws OpenGL through.

## How the renderer loads it

The module's library and the egui helper delay-load `opengl32.dll`: Windows loads it when OpenGL is first called, not when the module imports. Before the renderer makes its first OpenGL call, it loads the add-on's `opengl32.dll` by its full path, with the add-on's folder searched for the libraries that one loads in turn. It leaves the library loaded until the process ends. The delay-loaded calls then go to it, since a library of that name is loaded already. In a process that has loaded another `opengl32.dll` already, as an in-process renderer that drew with the system's OpenGL before the variable was set, the renderer fails naming both, since the calls may have gone to the other: a new process takes the add-on.

## What it includes

| File | |
|---|---|
| `opengl32.dll` | Mesa's WGL front end, 136,704 bytes |
| `libgallium_wgl.dll` | llvmpipe, Mesa's OpenGL and LLVM, linked in, 59,281,920 bytes |
| `PWRS-GUI.Mesa.win-x64.psd1` | the module manifest, for Windows PowerShell 5.1 and PowerShell 7 |
| `NOTICE` | what the add-on contains, how it was built, and the licenses of its code |
| `licenses/` | the license texts |

Both libraries are built with the static C runtime, so they need no Visual C++ redistributable: they import only Windows' own `GDI32`, `KERNEL32`, `USER32`, `ADVAPI32`, `ntdll`, `ole32` and `SHELL32`, which Server Core has.

## How it is built

`pwrs-gui-addons mesa <work folder> <output folder>` builds it on Windows x64, with Visual Studio 2022's C++ build tools at MSVC 14.44, a Windows SDK of 10.0.20348 or later, and Python 3.10 or later. It fetches the source of Mesa 26.2.4 and of LLVM 23.1.3, and Win flex-bison 2.5.25, each checked against its SHA-256. It installs Mesa's build tools, at fixed versions, into a Python environment of its own. It builds LLVM static, release, for the X86 target alone, then only Mesa's llvmpipe and desktop OpenGL against it, with the settings the mesa-dist-win project uses. On an AMD Ryzen 9 7900X under Windows 11 Pro, at idle priority, LLVM took 8.5 minutes and Mesa under a minute. A work folder left from an earlier build is reused, and the build continues from what is already there.

## Licenses

The build reads the license of every file the two libraries were built from: the sources their targets compile and every header those include, 1,833 files. It stops without packaging when a file is under the GPL without Bison's exception, or names a license it has no rule for:

| Files | License | Text |
|---|---|---|
| 1,555 | MIT, Mesa's own | `Mesa-MIT.txt` |
| 212 | LLVM's, the Apache License 2.0 with LLVM's exceptions | `LLVM.txt` |
| 17 | the Apache License 2.0, Khronos' headers | `Apache-2.0.txt` |
| 5 | BLAKE3's, CC0 1.0 or the Apache License 2.0, taken under the latter | `Apache-2.0.txt` |
| 1 | the SGI Free Software License B 2.0 | `SGI-B-2.0.txt` |
| 3 | the Boost Software License 1.0 | `BSL-1.0.txt` |
| 5 | BSD licenses: xxHash and Berkeley SoftFloat | each file's notice in `BSD-*.txt` |
| 26 | zlib's | `zlib.txt` |
| 6 | Bison's parser skeleton, the GPL 3 or later with Bison's exception, which lets a work containing it be distributed under terms of its own | none needed |
| 3 | flex's scanner skeleton, which flex places under no conditions | none needed |

No license of this code requires its source to be shipped, so the add-on includes none.
