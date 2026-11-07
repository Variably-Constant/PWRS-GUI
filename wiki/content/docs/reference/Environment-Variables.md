---
title: Environment Variables
weight: 61
---

Every environment variable PWRS-GUI reads, all optional. Source: `mode_from_env` and `web_view_from_env` in `crates/pwrs-gui/src/options.rs`, `backends` in `crates/pwrs-gui-egui/src/lib.rs`, `check_display` in `crates/pwrs-gui-client/src/platform.rs`, `options_for`, `temp_dir` and `environment` in `crates/pwrs-gui/src/session.rs`, and `per_process_root` in `crates/pwrs-gui-web/src/data.rs`. The monitor and the focus are read in `crates/pwrs-gui-proto/src/place.rs`, the add-ons' folders in `crates/pwrs-gui-client/src/webkit.rs`, `crates/pwrs-gui-client/src/webview2.rs` and `crates/pwrs-gui-client/src/mesa.rs`, the check of EGL in `crates/pwrs-gui-web/src/egl.rs`, and the trace in `crates/pwrs-gui-proto/src/trace.rs`.

A variable set with `$env:` in a session takes effect for the next renderer the session starts, on every platform. The module reads its variables from the session's environment and starts each helper with that environment: on Linux, FreeBSD and macOS, PowerShell does not write what `$env:` sets into the process's own environment, which a helper would otherwise inherit. A renderer another module starts through PWRS-GUI's native entry reads the process's environment instead, since the entry has no session to read.

The examples ran in PowerShell 7.6.6 and Windows PowerShell 5.1, each in a new process of the same host so that the variable they set reaches no other example. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths.

| Variable | Read by | Effect |
|---|---|---|
| `PWRS_GUI_MODE` | the module, when it starts a renderer and in `Get-GuiOption` | `Auto`, `Helper` or `InProcess`, without regard to case: where windows are drawn until `Set-GuiOption -Mode` sets it. Any other value is `GuiModeUnknown` in `Get-GuiOption` and `GuiOptions` from a command that starts a renderer. |
| `PWRS_GUI_BACKEND` | the renderer, when it starts | the GPU backends it may use, separated by commas: `dx12`, `vulkan`, `metal`, `gl`, without regard to case. Without it, DirectX 12, Vulkan and OpenGL on Windows, Metal on macOS, and Vulkan and OpenGL on Linux and FreeBSD. A value naming none of them fails the start with `GuiStartFailed`. On Windows OpenGL is left out where the system has no `opengl32.dll` and no Mesa add-on gives it one, as Windows Server Core has none, and a value naming OpenGL alone then fails the start with `GuiStartFailed` naming the library and the add-on. |
| `PWRS_GUI_MONITOR` | each renderer, when it starts | a monitor's number from 1: every window the renderer opens is centered on that monitor. On Windows the number is the display's own, as in `\\.\DISPLAY2`, which Windows' display settings show; elsewhere the HTML renderer counts the monitors in the order the windowing system lists them, and the egui renderer reads the variable on Windows only. Without it, the system places windows. A value that is not a number from 1, or names no monitor connected when the renderer starts, is a warning from the command that started the renderer, and windows open where the system places them. |
| `PWRS_GUI_NO_FOCUS` | each renderer, when it starts | `1` opens every window the renderer opens without taking the focus, so the window in front stays in front and the windows a test run opens leave a person's work alone, on Windows, macOS and X11; on Wayland the compositor gives a new window the focus as it chooses, and the command that started the renderer warns so. `0`, empty or unset, each window takes the focus as it opens. Any other value is a warning from the command that started the renderer, and windows take the focus. |
| `DISPLAY`, `WAYLAND_DISPLAY` | the session, before it starts a helper on Linux and FreeBSD | one of them must be set, or the command fails with `GuiNoDisplay` |
| `TMP` and `TEMP` on Windows, `TMPDIR` elsewhere | the session, as its temporary folder: the session's `TMPDIR` on Linux, FreeBSD and macOS when it is set, otherwise the folder Rust's `std::env::temp_dir` returns (`/tmp` when `TMPDIR` is not set) | each helper's standard error goes to a log file `PWRS-GUI/<session>.log` in it, whose last lines a `GuiRendererGone` error quotes when the helper dies |
| `PWRS_GUI_WEBVIEW` | the module, when it starts the HTML renderer and in `Get-GuiOption` | `Shared` or `PerProcess`, without regard to case: whether webviews share their data with other processes until `Set-GuiOption -WebView` sets it. Any other value is `GuiWebViewUnknown` in `Get-GuiOption` and `GuiOptions` from a command that starts a renderer. |
| `LOCALAPPDATA` on Windows; `XDG_DATA_HOME`, else `HOME`, elsewhere | the HTML renderer | where the web data folders go: `<LocalAppData>\PWRS-GUI\WebView2` shared, `PWRS-GUI\WebView2-<version>` shared on the WebView2 add-on's Fixed Version runtime, and `PWRS-GUI\WebView2-PerProcess` or `PWRS-GUI/WebKitGTK-PerProcess` under them for each process. With `PerProcess`, a renderer that finds none of them cannot start: a widget window draws with egui and a page fails with `GuiNoWebview`, each saying why. |
| `PWRS_GUI_WEBKIT_ADDON` | the session, on Linux and FreeBSD, when it starts the HTML renderer | the folder of a WebKitGTK add-on to start the HTML renderer from, in place of the `PWRS-GUI.WebKit.<rid>` module found on `PSModulePath`, or `none` to use the system's WebKitGTK alone. A folder with no add-on in it fails the renderer's start, naming the folder. [The WebKitGTK Add-On](The-WebKitGTK-Add-On.md) says more. |
| `PWRS_GUI_WEBVIEW2_ADDON` | the session, on Windows, when it starts the HTML renderer | the folder of a `PWRS-GUI.WebView2.win-x64` add-on whose `Install-GuiWebView2Runtime` has run, whose Fixed Version runtime the HTML renderer then runs on, in place of the add-on found on `PSModulePath`; or `none` to use the evergreen runtime. A folder without `runtime\msedgewebview2.exe` fails the renderer's start, naming the folder. [The WebView2 Add-On](The-WebView2-Add-On.md) says more. |
| `PWRS_GUI_MESA_ADDON` | the session, on Windows, when it starts the egui renderer | the folder of a Mesa add-on, whose `opengl32.dll` OpenGL then draws through on any machine, in place of the `PWRS-GUI.Mesa.win-x64` module found on `PSModulePath`, which is used only where System32 has no `opengl32.dll`; or `none` to use only the system's OpenGL. A folder without an `opengl32.dll` fails the renderer's start, naming the folder. [The Mesa Add-On](The-Mesa-Add-On.md) says more. |
| `PSModulePath` | the session, when it starts a renderer | the folders searched in order for the add-on modules, the highest version in the first folder that has one being used: on Linux and FreeBSD `PWRS-GUI.WebKit.<rid>`, which the HTML renderer starts from, as every later HTML renderer of the process then does; on Windows `PWRS-GUI.WebView2.win-x64`, whose Fixed Version runtime the HTML renderer runs on once its `Install-GuiWebView2Runtime` has run, and `PWRS-GUI.Mesa.win-x64`, which OpenGL draws through where System32 has no `opengl32.dll` |
| `LIBGL_ALWAYS_SOFTWARE` | the HTML renderer, on Linux and FreeBSD, before WebKitGTK starts | `1` skips the check that EGL starts on the GPU, and Mesa draws in software. Without it, a renderer whose check fails sets it, with `GALLIUM_DRIVER=llvmpipe` and `WEBKIT_DISABLE_DMABUF_RENDERER=1`, for itself and the processes WebKitGTK starts, and the session's log says why ([The WebKitGTK Add-On](The-WebKitGTK-Add-On.md#when-the-gpu-cannot-start-egl)). |
| `PWRS_GUI_TRACE` | each helper when it starts, and the session at its first renderer start | when set to any value, a line on standard error for each step of a renderer's start and of each window's life, with the milliseconds since the process's first line and the thread's name, to find where a renderer stops. A helper's lines go to its log file. |

PWRS's own variables apply as they do to any PWRS module; the PWRS wiki lists them, and [How To Run The Tests](How-To-Run-The-Tests.md) names those the tests use.

## Examples

Draw with Vulkan alone:

```powershell
& (Get-Process -Id $PID).Path -NoProfile -Command {
    $env:PWRS_GUI_BACKEND = 'vulkan'
    Get-GuiGpuAdapter | Format-Table Name, Backend, DeviceType, InUse
}
```

```text
Name                    Backend DeviceType    InUse
----                    ------- ----------    -----
NVIDIA GeForce RTX 5070 Vulkan  DiscreteGpu    True
AMD Radeon(TM) Graphics Vulkan  IntegratedGpu False
```

A monitor this machine does not have, named in a warning when the renderer starts; the example writes the warning's text:

```powershell
& (Get-Process -Id $PID).Path -NoProfile -Command {
    $env:PWRS_GUI_MONITOR = '9'
    $null = Get-GuiGpuAdapter -WarningVariable said -WarningAction SilentlyContinue
    $said.Message
}
```

```text
the GUI's Egui renderer started, and PWRS_GUI_MONITOR names monitor 9, but this machine's monitors are numbered 1, 2, so windows open where the system places them
```

The web data choice a variable sets, until `Set-GuiOption` sets another:

```powershell
& (Get-Process -Id $PID).Path -NoProfile -Command {
    $env:PWRS_GUI_WEBVIEW = 'perprocess'
    'From PWRS_GUI_WEBVIEW: {0}' -f (Get-GuiOption).WebView
    Set-GuiOption -WebView Shared
    'After Set-GuiOption: {0}' -f (Get-GuiOption).WebView
}
```

```text
From PWRS_GUI_WEBVIEW: PerProcess
After Set-GuiOption: Shared
```

The mode a variable sets, until `Set-GuiOption` sets another:

```powershell
& (Get-Process -Id $PID).Path -NoProfile -Command {
    $env:PWRS_GUI_MODE = 'inprocess'
    'From PWRS_GUI_MODE: {0}' -f (Get-GuiOption).Mode
    Set-GuiOption -Mode Helper
    'After Set-GuiOption: {0}' -f (Get-GuiOption).Mode
}
```

```text
From PWRS_GUI_MODE: InProcess
After Set-GuiOption: Helper
```
