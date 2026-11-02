---
title: How To Run The Tests
weight: 2
---

The lint, the Rust tests, the Pester suites in both hosts and every mode, and the runs on Linux and FreeBSD under a virtual display. Source: the `tests` folders of `crates/pwrs-gui-client` and `crates/pwrs-gui`, `crates/pwrs-gui/tests/Common.ps1` for what every suite shares, and cargo-pwrs's `test` command.

The result lines on this page are from runs on Windows 11 Pro 10.0.26200 on an AMD Ryzen 9 7900X, and on Ubuntu 24.04 and FreeBSD 15.0 under Xvfb, of the code this page describes. Every command below runs once `patches/apply.ps1` has, as [Building From Source](Building-From-Source.md) shows.

The tests open many windows. On a desktop in use, set `PWRS_GUI_NO_FOCUS=1` before a run, so that none of them takes the focus on Windows, macOS or X11, and `PWRS_GUI_MONITOR` to a monitor of their own; [Environment Variables](Environment-Variables.md) says how each works. The Windows runs on this page set both.

## Lint

```text
cargo clippy --profile test-fast --workspace --all-targets -- -D warnings
```

It exits 0 with no warnings on Windows, Linux and FreeBSD, each built on its own machine.

## The Rust tests

The client crate's tests start real renderers and open windows from a plain Rust program, and on Windows a test of the dialogs crate opens the system's folder dialog and closes it as it starts, so they need a display and both helpers built for the same profile:

```text
cargo build --profile test-fast -p pwrs-gui --bin pwrs-gui-egui --bin pwrs-gui-web
cargo test --profile test-fast --workspace
```

On Windows 208 tests pass; one more, a measurement of the fallback fonts' cost for a quiet machine, runs only when `--ignored` is given.

The unit tests cover:

- the protocol's encoding, framing and monitor placement, its widget tree and the patches that change it, and the typeface it finds for both renderers;
- the protocol's rules both renderers follow: how charts, dates, Markdown, canvas items, SVG paths, SubEtha rings and shared memory are read, how the camera and a canvas's view move, which keys each widget keeps, and when a password field offers its peek button;
- the egui renderer's style from the shared palette and spacing, its table, tree, selection, shortcuts, pictures, fallback fonts, captures and their deadline, its canvas, chart, camera, view and pointer, its own dialogs where the system has none, and its choice of backend;
- the web renderer's layout and canvas scripts, run in boa against the protocol's rules, its stylesheet in both themes, its Markdown, and its WGSL translated for WebGL 2;
- the web helper's runtime files and data folders, its captures and their deadline, the oldest engines it opens on without a warning, how it numbers a page's sends, and which window's files a `pwrs://` request gets;
- the GPU layer's buffers, textures, samplers and compute, a GpuSurface's draws and passes, and the objects a session made, kept through a new device;
- the answers each kind of system dialog takes, and how one is listed, answered and canceled;
- the client's file watch, the wake its waits share, and its search for the add-ons;
- the add-on builder's reading of licenses, ELF files and FreeBSD's catalog, its path rewrites at the same length, and the WebView2 module it writes;
- the module's choice, grid and form windows, its options, the queue Wait-GuiEvent reads, tree items found by their keys, the GPU commands' arguments, and its timing helpers.

The client's end-to-end tests drive a window by name in each renderer, through its helper and in-process, and read it back with its pixels. They cancel a wait from another thread, take the session through the native entry, and kill a helper to check that its commands wake. They kill a page's WebView2 processes to check that the page comes back as it was, and end a helper while another process's page runs on the browser process it started. They open a window on the monitor `PWRS_GUI_MONITOR` names, and run twenty in-process egui sessions one after another. Every wait in them has a deadline that cancels it, and every test a watchdog that ends the test process if it stalls, so no test can hang.

`PWRS_GUI_TRACE=1` makes each renderer write its start, each window's build, its page's navigations, loads and messages, and each step of a test's input as it begins, is taken and is seen, to standard error, which a test with `--nocapture` shows in place.

## The Pester suites

`cargo pwrs test` builds the module, runs the Rust tests, and runs `crates/pwrs-gui/tests/*.Tests.ps1` under Pester in PowerShell 7 and, on Windows, in Windows PowerShell 5.1, with `PWRS_MODULE` naming the built module. Build the stand-in module first, which one suite takes the session through, and SubEtha's module, which the suites of widgets fed from SubEtha rings import from the folder `SUBETHA_MODULE` names, built with the cargo-pwrs SubEtha's own tree pins:

```text
target/tools/bin/cargo-pwrs pwrs build --release --manifest-dir crates/pwrs-gui-standin
cargo install cargo-pwrs --version 0.7.0 --locked --root target/tools-subetha
target/tools-subetha/bin/cargo-pwrs pwrs build --release --manifest-dir ../SubEtha/crates/subetha-pwrs
```

Then, with `SUBETHA_MODULE` set to the full path of `../SubEtha/target/pwrs/SubEtha`:

```text
target/tools/bin/cargo-pwrs pwrs test --release --manifest-dir crates/pwrs-gui
```

```text
Tests Passed: 505, Failed: 0, Skipped: 14, Inconclusive: 0, NotRun: 0
pwrs pester host=7.6.6 pester=5.7.1 passed=505 failed=0
Tests Passed: 505, Failed: 0, Skipped: 14, Inconclusive: 0, NotRun: 0
pwrs pester host=5.1.26100.9444 pester=6.2.0 passed=505 failed=0
pwrs: surface check: every cmdlet ran, every pipeline parameter took a record, every ShouldProcess asked
```

That run named the WebView2 add-on, as the next paragraph describes. The 14 skipped on Windows run elsewhere. These run on Linux and FreeBSD alone:

- the renderer's own dialogs where the system has none, four with each renderer;
- a page drawing in egui's own fonts where the system names none;
- the language a page is given under the C locale;
- the display found in the session;
- a page drawn with the WebKitGTK add-on.

Windows opened without the focus are tested on macOS, Linux and FreeBSD, once with each renderer.

Each suite runs once per mode the platform has, helper and in-process on Windows and helper elsewhere, and the widget suites once per renderer, with every widget and its events, the update commands, a window that does not wait, and on Windows the screen reader's view through UI Automation. The suites drive every command from a second runspace as [How To Test A Window](How-To-Test-A-Window.md) shows. Choices are selected, activated, canceled and stopped; grids streamed, filtered by column, picked and detached; windows listed, driven and captured with their pixels. Pages are shown, replied to, sent values and closed with a result, from text and from a folder, in a process with web data of its own, and brought back after their browser process is killed. A widget window is drawn with egui when no webview engine is found, and the suites cover the options, the adapters, latency in both forms, and the stand-in drawing through the native entry beside the module's own windows. Another runspace that imported the module closes, or removes it, with every window left open, and the module's removal from its last runspace ends the session.

Two tests run on an installed add-on, which the run names, and are skipped without it: `PWRS_GUI_TEST_WEBVIEW2_ADDON`, on Windows, names a version folder of `PWRS-GUI.WebView2.win-x64` whose `Install-GuiWebView2Runtime` has run, and `PWRS_GUI_TEST_WEBKIT_ADDON`, on Linux and FreeBSD, a version folder of `PWRS-GUI.WebKit.<rid>`. The test of windows opened without the focus runs on macOS, Linux and FreeBSD. On macOS it brings Finder to the front and reads the front application with `lsappinfo`. On Linux and FreeBSD it starts an Xvfb display of its own with Openbox, which gives every new window the focus unless the window is marked not to take it, and reads the active window with `xprop`.

No test can stall the run. A command that blocks runs in a background runspace, and is stopped, failing its test, when it has not returned within 20 seconds. Every wait for a window gives up after 30, and `Invoke-GuiTestEvent -Input` fails a step not taken and seen within 10. A watchdog process ends the PowerShell host when a test file runs longer than 600 seconds, so a renderer that stops responding fails the run instead of stalling it. Before it does, it writes the thread stacks of the host's helpers, as a hung helper's are written when a suite resets its session: with `sample` on macOS, `lldb` on FreeBSD and `gdb` on Linux, where each is installed.

The runner loads Pester from `PWRS_PESTER_PATH` when it is set, else from `target/pester/Pester`, else from the host's module path; Windows PowerShell's own Pester 3.4.0 does not run these suites.

## Linux and FreeBSD

The suites run in pwsh under Xvfb, an X server with no screen, and the renderers draw through Mesa's software Vulkan or OpenGL there. The HTML renderer needs WebKitGTK 4.1: on Ubuntu 24.04 the packages beyond a server install are `xvfb`, `libxkbcommon-x11-0` and `libwebkit2gtk-4.1-dev`, on FreeBSD `xorg-vfbserver`, `webkit2-gtk_41` and `dejavu`, and the focus test needs `openbox` and `x11-utils` on Ubuntu, `openbox` and `xprop` on FreeBSD. The clipboard tests need `xclip` on both: the suites read the clipboard back through it, and fill it before a paste, since PowerShell's own clipboard commands do nothing on FreeBSD. A page draws its text in the fonts fontconfig prefers, and where the system has none, in egui's own, which the HTML renderer gives the page; Ubuntu's fontconfig brings DejaVu with it, and FreeBSD's brings no font.

Run each command on a session bus of its own with `dbus-run-session`, so the services that bus starts have the display, as a desktop session's do. GTK looks up the desktop portal on the session bus as the HTML renderer starts. On a server, the user's own bus starts the portal without a display, where its GTK backend fails, and the portal makes every caller wait until its start times out. The web helper then sends nothing for that long. Start a display and point `DISPLAY` at it before the run:

```text
Xvfb :77 -screen 0 1600x1200x24 -nolisten tcp &
export DISPLAY=:77
dbus-run-session -- cargo build --profile test-fast -p pwrs-gui --bin pwrs-gui-egui --bin pwrs-gui-web
dbus-run-session -- cargo test --profile test-fast --workspace
dbus-run-session -- target/tools/bin/cargo-pwrs pwrs build --release --manifest-dir crates/pwrs-gui-standin
cargo install cargo-pwrs --version 0.7.0 --locked --root target/tools-subetha
target/tools-subetha/bin/cargo-pwrs pwrs build --release --manifest-dir ../SubEtha/crates/subetha-pwrs
export SUBETHA_MODULE="$(cd ../SubEtha && pwd)/target/pwrs/SubEtha"
dbus-run-session -- target/tools/bin/cargo-pwrs pwrs test --release --manifest-dir crates/pwrs-gui
```

On each, 198 Rust tests pass, and the measurement of the fallback fonts' cost again runs only when `--ignored` is given; the in-process tests and those of monitor placement, page recovery and a shared browser process run on Windows only. Pester passes 285 and skips five, the two tests of WebView2's own processes, the WebView2 add-on's, and UI Automation's in each renderer, a Windows interface:

```text
Tests Passed: 285, Failed: 0, Skipped: 5, Inconclusive: 0, NotRun: 0
pwrs pester host=7.6.6 pester=6.2.0 passed=285 failed=0
```

That is Ubuntu 24.04's run; FreeBSD 15.0's, in pwsh 7.5.5, has the same counts.

Without a display, every command that would open a window fails with `GuiNoDisplay`, naming that neither `DISPLAY` nor `WAYLAND_DISPLAY` is set.
