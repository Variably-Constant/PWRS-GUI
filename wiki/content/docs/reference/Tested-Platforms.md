---
title: Tested Platforms
weight: 63
---

The machines, systems and hosts PWRS-GUI has run on, what each run showed, and what was measured. The test runs on Windows 11, Linux, FreeBSD and macOS and the module sizes are of one revision, the one that adds a password field's peek button, built against PoWerRuSt 0.8.0 and cargo-pwrs 0.8.0 from crates.io and against SubEtha's and the Wgpu module's source trees beside the checkout, as [Building From Source](Building-From-Source.md) describes. The runs on Windows Server, the runs of the Mesa and WebView2 add-ons and the measurements under [Measured on Windows](#measured-on-windows) are of earlier revisions, built against PoWerRuSt 0.5.0 and the cargo-pwrs of PWRS's source tree, and were not repeated on this one, except the measurement of a session's second dialog under [The window coming to the front](#the-window-coming-to-the-front), which is of a later revision, built as this one is. The runs of the WebKitGTK add-ons are of a later revision too, the one that rounds a page's text rows to whole physical pixels, built as this one is. Every module was built by `cargo pwrs build --release` or `cargo pwrs test --release` with the workspace's release profile, fat LTO and one codegen unit.

## Systems

| Platform | Machine | PowerShell | Drawn with |
|---|---|---|---|
| Windows x64 | Windows 11 Pro 10.0.26200, AMD Ryzen 9 7900X, NVIDIA GeForce RTX 5070 and the processor's AMD Radeon graphics | 7.6.9 on .NET 10.0.14, and Windows PowerShell 5.1.26100.9711 | egui on DirectX 12 on each of the RTX 5070, the Radeon graphics and Microsoft Basic Render Driver (WARP); WebView2 156.0.4314.56, and the WebView2 add-on's 155.0.4283.45 for the test of it |
| Windows Server x64 | Windows Server 2019 Standard Evaluation 10.0.17763 virtual machine, AppLocker's three default executable rules enforced, signed in as a standard user | 7.6.6, and Windows PowerShell 5.1.17763.2931 | egui on DirectX 12 on Microsoft Basic Render Driver, in PowerShell's own process; no WebView2 runtime installed |
| Linux x64 | Ubuntu 24.04 virtual machine, Linux 6.8, Xvfb | 7.6.6 on .NET 10.0.12, from Microsoft's tarball | egui on Vulkan on llvmpipe (LLVM 20.1.2, 256 bits), Mesa 25.2.8; WebKitGTK 2.52.6, and the WebKitGTK add-on's 2.50.4 for the test of it |
| FreeBSD x64 | FreeBSD 15.0 virtual machine, Xvfb | 7.5.5 on .NET 9.0.14 | egui on Vulkan on llvmpipe (LLVM 19.1.7, 256 bits), Mesa 26.1.3, with OpenGL 4.6 on llvmpipe listed as well; WebKitGTK 2.46.6, the system's and the add-on's |
| macOS arm64 | GitHub's hosted runners, macOS 26.6.2 and macOS 14.8.9, through `.github/workflows/ci.yml` | 7.6.5 on .NET 10.0.11, and 7.6.4 on .NET 10.0.10 | egui: not recorded by the run; the system's WebKit |

## What each run showed

On Windows: `cargo clippy --workspace --all-targets` with warnings denied; the 208 Rust tests, with one more, a measurement meant for a quiet machine, ignored; the stand-in module built; `cargo pwrs test`, whose 505 Pester tests passed in each host, with every suite run in Helper and in in-process mode and the widget suites with each renderer, UI Automation reading the widgets of egui and HTML windows by name and role among them, and whose surface check found every command run, every pipeline parameter piped into and every ShouldProcess call made; and `cargo pwrs publish --dry-run`, which packaged the module. The 14 Pester tests skipped there are those of other systems alone: the renderer's own dialogs where the system has none, eight; a page where the system has no fonts; a page under the C locale; the WebKitGTK add-on; the display found in the session's environment; and a window opened without the focus, in each renderer. No .NET process wrote a crash dump.

On Linux and FreeBSD: the same steps, with 198 Rust tests and the same one ignored, and 285 Pester tests, every suite in Helper mode, since in-process drawing is a Windows mode; five are skipped there: the two of WebView2's own processes, the WebView2 add-on's, and UI Automation's in each renderer, a Windows interface. Without `DISPLAY` or `WAYLAND_DISPLAY`, a command fails with `GuiNoDisplay`: "no window can open: neither DISPLAY nor WAYLAND_DISPLAY is set".

On macOS, in CI, on each of the two runners: clippy with warnings denied, the 191 Rust tests and the same one ignored, the stand-in module built, `cargo pwrs test`, whose 271 Pester tests passed in pwsh in Helper mode, with 19 skipped: the three of WebView2, its two own processes and its add-on; the test that hides the HTML engine, since WebKit is part of macOS; the developer settings of a page shown with `-DevTools`, which WebKit on macOS does not read back; the twelve of Linux and FreeBSD alone, the renderer's own dialogs where the system has none, eight, a page where the system has no fonts, a page under the C locale, the WebKitGTK add-on and the display found in the session's environment; and UI Automation's in each renderer, a Windows interface; and `cargo pwrs publish --dry-run`, which packaged the module.

On Windows Server 2019 under AppLocker, in each host, signed in as the standard user, with the module imported from a folder under `C:\Users\Public` and pwsh 7.6.6 from Microsoft's zip under `C:\Program Files\PowerShell\7`: AppLocker refused the egui helper the module stages under the temporary folder, so in Helper mode `Read-GuiChoice` failed with `GuiStartFailed`, ending "pwrs-gui-egui.exe cannot start: This program is blocked by group policy. For more information, contact your system administrator. (os error 1260)". In Auto mode the same command drew its window in PowerShell's process and said why in the Verbose stream: "the GUI's Egui helper could not start, so its windows are drawn in this process". `Get-GuiWindow` listed the window, `Get-GuiOption` reported it running in process on DirectX 12 on Microsoft Basic Render Driver, `Get-GuiSnapshot -Pixels` returned its 420 by 202 pixels in 182 distinct colors, and the choice made through `Invoke-GuiTestEvent` was the command's output. No WebView2 runtime was installed. In Auto mode a dialog requested in HTML was drawn with egui and returned its choice, and the Verbose stream said the HTML renderer could not start because no WebView2 engine was found. A page of its own, from `Show-GuiWindow -Html`, failed with `GuiNoWebview`, whose message names Microsoft's WebView2 Runtime as what to install.

On Windows Server 2019 Standard Evaluation with the Desktop Experience, a virtual machine with no WebView2 installed, set to let a user sign in more than once: WebView2's Fixed Version runtime 154.0.4258.62 was unpacked into a folder and named by `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER`, and one standard user was signed in at the console and, at the same time, through a Remote Desktop connection the console made to the machine itself. In Windows PowerShell 5.1, in `Shared` mode, the console's pages drew under a browser process of the console's session on `<LocalAppData>\PWRS-GUI\WebView2`. In the second session a page failed with `GuiPageFailed` 3.8 s after it was requested, WebView2 having refused its webview with 0x800700AA, "The requested resource is in use"; with `-WebView PerProcess` the second session's page drew. The console's open page then took a message, and a new page there drew. The machine ran on QEMU's software CPU, so these times say only that nothing waited.

## Module size

The module folder as the release build writes it, in bytes:

| Runtime | Library | egui helper | HTML helper | Notices | Folder |
|---|---|---|---|---|---|
| `win-x64` | `pwrs_gui.dll` 25,644,032 | `pwrs-gui-egui.exe` 21,614,080 | `pwrs-gui-web.exe` 4,188,160 | 2,494,240 | 55,260,682 |
| `linux-x64` | `libpwrs_gui.so` 3,836,992 | `pwrs-gui-egui` 28,933,424 | `pwrs-gui-web` 4,844,304 | 3,168,728 | 42,103,618 |
| `freebsd-x64` | `libpwrs_gui.so` 3,802,136 | `pwrs-gui-egui` 28,664,080 | `pwrs-gui-web` 4,816,696 | 3,104,422 | 41,707,504 |
| `osx-arm64` | `libpwrs_gui.dylib` 3,212,112 | `pwrs-gui-egui` 17,995,344 | `pwrs-gui-web` 3,992,112 | 2,379,051 | 28,898,789 |

The macOS row is the macOS 26.6.2 runner's build; on the macOS 14.8.9 runner the egui helper was 18,012,288 bytes, the HTML helper 3,992,144 and the folder 28,915,765. On Windows the library contains both in-process renderers; on the other platforms only the helpers draw. The HTML helper is small because the engine it drives is the system's.

rfd 0.17.2, whose dialogs a FilePicker opens, added 13,312 bytes to the Windows egui helper and 154,184 bytes to the Linux one when it was measured alone, built into it with a path calling its message, open, open-many, save and folder dialogs. Its other dependencies are in the helper already: it adds rfd alone on Windows, and rfd and pollster on Linux.

## The WebKitGTK add-ons

`pwrs-gui-addons` built each add-on on the virtual machine of its platform: `PWRS-GUI.WebKit.linux-x64` 2.50.4.0 from Ubuntu 22.04's packages, containing WebKitGTK 2.50.4, and `PWRS-GUI.WebKit.freebsd-x64` 2.46.6.0 from FreeBSD 15.0's quarterly packages, containing WebKitGTK 2.46.6 and GIO's OpenSSL module, the TLS module FreeBSD's glib-networking builds, as [The WebKitGTK Add-On](The-WebKitGTK-Add-On.md) describes. Each ran installed as a module in the all-users module folder, with nothing set to point at it, so PWRS-GUI found it itself, on a machine with no WebKitGTK or GTK:

- Linux: a rootless podman container on the Ubuntu 24.04 virtual machine, made from Ubuntu 24.04's image with what the add-on leaves to the system (X11, Wayland, and Mesa's EGL, OpenGL, GBM, Vulkan and drivers), pwsh's own libraries, Xvfb and procps; pwsh 7.6.6 from Microsoft's tarball; 1 GiB of `/dev/shm`, since a suite's SubEtha rings outgrow podman's default of 64 MiB.
- FreeBSD: a jail on the FreeBSD 15.0 virtual machine, made from FreeBSD 15.0's base system with pwsh 7.5.5, Xvfb, and the X11, Wayland, Mesa, EGL, GBM, DRM, Vulkan and xkbcommon packages. The jail allows `mlock(2)`, without which .NET does not start: "Failed to create CoreCLR, HRESULT: 0x8007FF02".

In each, the runtime linker found no WebKitGTK or GTK library, and the HTML suite, `Html.Tests.ps1`, passed 23 tests and failed none. It skipped four: the two of WebView2's own processes, the WebView2 add-on's, and the one that sets the add-on against the system's WebKitGTK, which neither machine had. A window of each kind of text and of several controls, drawn by both renderers, put every named widget at the same place and height in HTML as in egui, and at the same width but for a heading, one point wider.

### When EGL cannot start

No machine here has a GPU, so the check of EGL was made to fail by misconfiguring Mesa. A raw-HTML page that closes itself at once was opened under each setting, on each machine, with the same outcome on both:

| Made to fail | Outcome |
|---|---|
| nothing | the page drew |
| Mesa's Gallium driver, `GALLIUM_DRIVER=bogus` | the check failed, the session's log said "EGL did not start on this machine's GPU (...); WebKitGTK draws with Mesa's software renderer", and the page drew |
| Mesa's driver folder, `LIBGL_DRIVERS_PATH=/nonexistent`, or its loader, `MESA_LOADER_DRIVER_OVERRIDE=bogus` | EGL started anyway, and the page drew |
| every EGL vendor, `__EGL_VENDOR_LIBRARY_DIRS=/nonexistent` | the check failed with `EGL_BAD_PARAMETER` on the GPU and again in software, the log said pages may not draw, WebKitGTK's web process ended, and the command failed with `GuiPageFailed` |

Turning WebKit's accelerated compositing off as well, with `WEBKIT_DISABLE_COMPOSITING_MODE=1`, changed no outcome. With the software renderer's variables set from the start, the check is skipped: the first three drew, and with no EGL vendor WebKitGTK's web process aborted with "Could not create default EGL display: EGL_BAD_PARAMETER" and the command failed with `GuiPageFailed`.

### Sizes

Each add-on and its source module as the builder writes them, and compressed file by file with zlib at its highest level, as a package's zip stores them. NuGet.org takes a package of up to 250 MB.

| Package | Files | Size | Compressed |
|---|---|---|---|
| `PWRS-GUI.WebKit.linux-x64` | 3,123 | 232.8 MiB | 97.0 MB |
| `PWRS-GUI.WebKit.linux-x64.Source` | 196 | 220.4 MiB | 230.0 MB |
| `PWRS-GUI.WebKit.freebsd-x64` | 2,386 | 250.8 MiB | 91.0 MB |
| `PWRS-GUI.WebKit.freebsd-x64.Source` | 122 | 181.8 MiB | 189.6 MB |

## The Mesa add-on

`pwrs-gui-addons mesa` built `PWRS-GUI.Mesa.win-x64` on Windows 11 Pro on an AMD Ryzen 9 7900X, as [The Mesa Add-On](The-Mesa-Add-On.md) describes: 56.7 MiB, with `opengl32.dll` of 136,704 bytes and `libgallium_wgl.dll` of 59,281,920. It ran on a Windows Server 2019 Server Core virtual machine with no GPU, whose System32 has no `opengl32.dll`, no `vulkan-1.dll` and no WARP:

- Without the add-on, a window's renderer failed naming the libraries the system lacks and `Install-Module PWRS-GUI.Mesa.win-x64`.
- With the add-on in a folder of `PSModulePath`, egui drew through llvmpipe, and a `Read-GuiChoice` dialog opened and was answered in both modes, in Windows PowerShell 5.1 and in PowerShell 7.6.6.

## The WebView2 add-on

`pwrs-gui-addons webview2` built `PWRS-GUI.WebView2.win-x64` 155.0.4283.45, as [The WebView2 Add-On](The-WebView2-Add-On.md) describes: four files, 7,754 bytes. On Windows 11 Pro its `Install-GuiWebView2Runtime` downloaded Microsoft's cab of 310,544,938 bytes, checked its SHA-256, and expanded the runtime into the add-on's folder, 256 files and 705,527,045 bytes, with read and execute granted to all application packages and all restricted application packages. PowerShellGet 2.2.5 refuses to install the add-on until `-AcceptLicense` accepts Microsoft's terms; Windows PowerShell 5.1's own PowerShellGet 1.0.0.1 cannot read the package until PowerShellGet is updated.

HTML windows drew on the add-on's runtime in both modes and both hosts, as the HTML suite checks. From a plain Rust program on the same machine, against the evergreen runtime 154.0.4258.62, the medians of four runs of five sessions each, with the processor 12 to 97 % busy as each run began:

| Mode | Time to | Evergreen 154.0.4258.62 | Fixed Version 155.0.4283.45 |
|---|---|---|---|
| Helper | start | 38 ms | 50 ms |
| Helper | first window | 406 ms | 464 ms |
| Helper | each further window | 167 ms | 184 ms |
| In-process | start | 51 ms | 55 ms |
| In-process | first window | 164 ms | 169 ms |
| In-process | each further window | 167 ms | 183 ms |

## Measured on Windows

On the Windows 11 machine, in each host and each mode, with egui drawing on the RTX 5070 unless a row names another adapter, and HTML in WebView2. The machine was a median 12 to 24 % busy with other work during these runs, from idle to brief spikes of 100 %; each figure is as measured under that load. Clicks were sent by `Invoke-GuiTestEvent` from a second runspace, 50 ms apart.

### A click

Medians of 50 clicks, after a first click left out of them. `Measure-GuiLatency` times each from when the renderer receives it, on a clock both processes read.

| From a click to | pwsh, Helper | pwsh, in-process | 5.1, Helper | 5.1, in-process |
|---|---|---|---|---|
| egui: the module | 23.4 µs | 15.4 µs | 24.7 µs | 14.2 µs |
| egui: a handler script block | 85.2 µs | 84.8 µs | 75.4 µs | 63.9 µs |
| egui: the pipeline's result | 114.7 µs | 113.7 µs | 98.4 µs | 87.5 µs |
| egui: an engine-event action, run inside `Wait-Event` | 441.0 ms | 406.4 ms | 353.4 ms | 385.5 ms |
| HTML: the module | 23.4 µs | 15.1 µs | 18.4 µs | 12.9 µs |
| HTML: a handler script block | 99.5 µs | 63.6 µs | 57.3 µs | 45.4 µs |
| HTML: the pipeline's result | 130.0 µs | 86.9 µs | 76.7 µs | 62.8 µs |
| HTML: an engine-event action, run inside `Wait-Event` | 440.3 ms | 579.2 ms | 617.9 ms | 580.6 ms |

The HTML clicks from pwsh in Helper mode ran while the machine was 66 to 77 % busy. The first click took 1.4 to 3.6 ms to the pipeline with egui and 1.3 to 4.1 ms with HTML. With `-NoWait`, the module received a click a median 11.8 to 22.8 µs after the renderer did. An engine-event action waits for PowerShell's engine to run it, so it takes as long with either renderer.

### Stopping and ending

| | pwsh, Helper | pwsh, in-process | 5.1, Helper | 5.1, in-process |
|---|---|---|---|---|
| egui: Ctrl+C to the command returning, median of 20 | 0.6 ms | 0.3 ms | 0.6 ms | 0.4 ms |
| egui: Ctrl+C to the window no longer listed | 52.4 ms | 54.1 ms | 28.7 ms | 29.4 ms |
| egui: the helper killed to the waiting command ending, median of 10 | 82.8 ms | | 89.4 ms | |
| egui: the next command starting a new helper | 490.9 ms | | 461.3 ms | |
| HTML: Ctrl+C to the command returning, median of 20 | 0.7 ms | 0.6 ms | 0.9 ms | 0.7 ms |
| HTML: Ctrl+C to the window no longer listed | 18.2 ms | 17.1 ms | 21.7 ms | 16.6 ms |
| HTML: the helper killed to the waiting command ending, median of 10 | 9.8 ms | | 17.8 ms | |

Each killed helper ended its command with `GuiRendererGone`.

### Reloading with a window open

Medians of 5. `Import-Module -Force` with a window `Out-GuiGridView` left open, in the same runspace:

| | pwsh, Helper | pwsh, in-process | 5.1, Helper | 5.1, in-process |
|---|---|---|---|---|
| egui: until `Import-Module` returned | 18.1 ms | 23.8 ms | 19.0 ms | 22.9 ms |
| egui: until the window, and in Helper mode its helper, were gone | 18.4 ms | 24.2 ms | 19.3 ms | 26.1 ms |
| egui: the next dialog, from the reloaded module, until listed | 494.8 ms | 568.1 ms | 476.0 ms | 494.4 ms |
| HTML: until `Import-Module` returned | 17.3 ms | 17.0 ms | 18.4 ms | 19.3 ms |
| HTML: until the window, and in Helper mode its helper, were gone | 17.5 ms | 17.2 ms | 18.9 ms | 19.8 ms |
| HTML: the next dialog, from the reloaded module, until listed | 269.0 ms | 124.8 ms | 266.2 ms | 133.7 ms |

Every next dialog took its answer and wrote it. `Remove-Module` with a dialog waiting in another runspace returned in 0.9 to 9.7 ms, medians 1.0 to 1.4 ms, and left that dialog open, since the other runspace still had the module imported. Once the module was imported again, the dialog was still listed, took its answer and wrote it, in 5 runs of 5 in each host, mode and renderer.

### The window coming to the front

The dialog opened from a classic console window and from Windows Terminal, three times each in every host, mode and renderer, while the desktop was in use. The rows count the runs in which the terminal was in front and the last key pressed had gone to it, as after typing the command, and no other window took the front; each checked the foreground window for 3 s after the dialog was listed:

| | Classic console | Windows Terminal |
|---|---|---|
| egui, Helper | in front 9 to 19 ms after it was listed, 2 of 2 | in front 10 to 19 ms after it was listed, 5 of 5 |
| egui, in-process | in front 46 ms after it was listed, 1 of 1 | in front 33 to 53 ms after it was listed, 6 of 6 |
| HTML, Helper | no run counted: the console was not in front when the dialog opened, or another window took the front | in front 8 to 25 ms after it was listed, 4 of 4 |
| HTML, in-process | in front 7 ms after it was listed, 1 of 1 | in front 8 to 23 ms after it was listed, 4 of 4 |

A dialog drawn in PowerShell's process requests the front from Windows with `SetForegroundWindow` as it opens, which Windows grants while the command runs in the terminal a person typed it into; once they have moved to another program it is refused, and the dialog opens behind it. A helper's dialog comes forward by Windows' rule for a newly started process. In the runs left out, a Visual Studio Code or Edge window took the front, or the terminal was not in front when the dialog opened.

A session's second dialog was measured on the desktop with no other program in use: each run opened a dialog and canceled it, checked whether the terminal was in front again within 10 s, then pressed a key in it and opened a second dialog of the same session, three times from each terminal in each host. No other window took the front in any run:

| | Runs | First dialog in front | Terminal in front again | Second dialog in front |
|---|---|---|---|---|
| egui, Helper | 12 | 6 to 15 ms after it was listed | 12 of 12 | 1 to 8 ms after it was listed |
| egui, in-process | 12 | 6 to 22 ms after it was listed | 12 of 12 | 1 to 22 ms after it was listed |
| HTML, Helper | 12 | 6 to 20 ms after it was listed | 12 of 12 | 1 to 7 ms after it was listed |

A helper's later dialogs come forward because the module lets the helper take the front, with `AllowSetForegroundWindow`, before it requests a window from the helper. An egui window, as it closes, gives the front back to the window that had it when it opened, as Windows does for a webview's window.

### The first window

`first_frame`, the client crate's example, starting a session and opening five windows in each of five runs; times from the start to the first window's `Opened`, and for each later window:

| | Start, first run | Start, later runs | First window | Later windows |
|---|---|---|---|---|
| egui helper, RTX 5070 | 657 ms | 432 to 452 ms | 42 to 60 ms | 63 to 87 ms |
| egui helper, Radeon | 422 ms | 433 to 465 ms | 85 to 107 ms | 107 to 125 ms |
| egui helper, WARP | 367 ms | 382 to 442 ms | 52 to 65 ms | 153 to 200 ms |
| egui in-process, RTX 5070 | 445 ms | 460 to 465 ms | 48 to 61 ms | 63 to 83 ms |
| egui in-process, Radeon | 428 ms | 436 to 465 ms | 79 to 98 ms | 104 to 128 ms |
| egui in-process, WARP | 367 ms | 478 to 611 ms | 47 to 62 ms | 153 to 235 ms |
| HTML helper | 81 ms | 25 to 29 ms | 307 to 338 ms | 117 to 167 ms |
| HTML in-process | 4 ms | 37 to 41 ms | 289 ms in the first run, 126 to 130 ms in the others | 133 ms |

The machine was 6 to 34 % busy during these runs. WebView2 picks its own adapter, so the HTML rows ran once per mode.

### A page brought back

The client crate's `web_recovery` test, run 30 times with `PWRS_GUI_TRACE` set, kills the WebView2 browser process of a widget window's page and then its renderer process. From the renderer finding the process ended to the page loaded again, which requests the tree and state the renderer stored: after the browser process, 204.5 to 253.8 ms, a median of 215.7 ms; after the renderer process, 37.8 to 52.1 ms, a median of 39.9 ms. Every page came back with what it had.

### Memory

With egui, the helper with one dialog open used 182.6 to 182.8 MB working set and 406.0 to 406.3 MB private in the two hosts, and with a 10,000-row grid 187.2 to 191.5 MB and 410.7 to 414.7 MB. By adapter, with one dialog, from pwsh: RTX 5070 248.5 MB and 470.7 MB, Radeon 355.8 MB and 579.4 MB, WARP 164.7 MB and 482.7 MB. In-process, the host's private memory grew 440.5 MB in pwsh and 456.5 MB in Windows PowerShell from the import to one dialog open; with that dialog pwsh used 324.9 MB working set and 477.9 MB private, Windows PowerShell 310.2 MB and 524.6 MB.

With HTML, the helper with one dialog open used 23.4 MB working set and 3.9 MB private, and the page's six WebView2 processes 311.7 to 313.3 MB and 142.0 to 143.2 MB; with a 10,000-row grid, the helper 40.7 to 40.8 MB and 21.7 to 21.8 MB, and the WebView2 processes 983.5 to 1,070.6 MB and 812.0 to 897.2 MB. In-process, the host's private memory grew 40.1 MB in pwsh and 82.5 MB in Windows PowerShell from the import to one dialog open; with that dialog pwsh used 168.1 MB working set and 78.8 MB private, Windows PowerShell 177.2 MB and 151.5 MB, beside the WebView2 processes' 314.1 to 317.3 MB and 141.8 to 149.2 MB.

### Idle

Over 30 s with a dialog waiting and no input: with egui, the helper used 0 to 16 ms of processor time and the host 16 to 31 ms in Helper mode, and in-process the host 47 to 62 ms; with HTML, the helper 0 to 63 ms and the host 16 to 31 ms, and in-process the host 16 to 31 ms. With a renderer running and no window, the host used 0 to 16 ms, except Windows PowerShell's with the in-process egui renderer, which used 469 ms.

### Pixels

The same dialog, 420 by 178 pixels, captured with `Get-GuiSnapshot -Pixels`, gave the same pixels in both hosts and both modes for each renderer and adapter; the first 16 hex digits of their SHA-256: egui on the RTX 5070 `1D565F1076F14AB0`, on the Radeon graphics `C5E9DCC18E36D5D8`, on WARP `063CBFD06FCB3953`, and WebView2 `52F03F0ACCADA763`.

## Not run

- Wayland: the Linux and FreeBSD runs drew on Xvfb, an X server.
- A GPU on Linux or FreeBSD: Mesa's software adapters drew there, and the check of EGL failed only where Mesa was made to fail.
- Windows, Linux or FreeBSD on Arm64.
- A session's second dialog coming to the front when HTML draws it in PowerShell's own process: those runs covered egui in both modes and HTML through its helper.
- macOS outside CI: the project owns no Mac, so macOS has run the tests and the build, and none of the measurements.
