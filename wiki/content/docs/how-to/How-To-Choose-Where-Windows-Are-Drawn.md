---
title: How To Choose Where Windows Are Drawn
weight: 9
---

Draw windows in the helper process or, on Windows, in PowerShell's own process, choose the GPU and the graphics API that draw them, draw widget windows in a webview, and choose what webviews share with other processes. Source: `crates/pwrs-gui/src/options.rs`; `Session::start` in `crates/pwrs-gui-client/src/lib.rs`; `choose_adapter` and `backends` in `crates/pwrs-gui-egui/src/lib.rs`; `crates/pwrs-gui-web/src/data.rs` for web data.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own, on a machine with an NVIDIA GeForce RTX 5070 and the Radeon graphics of its AMD Ryzen 9 7900X. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths and the helper's process id. The clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows.

## Helper or in-process

By default the mode is `Auto`: the first window starts a helper process, `pwrs-gui-egui`, which draws every window of the session and ends with it, and on Windows, when the helper cannot start, the windows are drawn in PowerShell's own process instead, with the reason in the Verbose stream. PWRS starts a helper from a copy it stages in the temporary folder, and [AppLocker's default executable rules](https://learn.microsoft.com/windows/security/application-security/application-control/app-control-for-business/applocker/executable-rules-in-applocker) let a standard user run programs only from the Windows and Program Files folders, so where they are enforced the helper is refused and the windows are drawn in-process.

The helper is a child of the PowerShell process:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click -->
<!-- varies: the helper's process id -->
```powershell
Read-GuiChoice -Choice 'one', 'two'
(Get-GuiOption).Running
Get-CimInstance Win32_Process -Filter "Name = 'pwrs-gui-egui.exe' AND ParentProcessId = $PID" | Select-Object Name, ProcessId
```

```text
Helper

Name              ProcessId
----              ---------
pwrs-gui-egui.exe     93828
```

`Set-GuiOption -Mode` and the `PWRS_GUI_MODE` environment variable choose. A renderer goes on with the options it started with, so a change applies to the next one. Removing the module from every runspace that has it ends the session, closing its windows and its helper:

```powershell
Remove-Module PWRS-GUI
Import-Module PWRS-GUI
Set-GuiOption -Mode InProcess
```

The next window starts a renderer with the options set then:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click -->
```powershell
Read-GuiChoice -Choice 'one', 'two'
(Get-GuiOption).Running
@(Get-CimInstance Win32_Process -Filter "Name = 'pwrs-gui-egui.exe' AND ParentProcessId = $PID").Count
```

```text
InProcess
0
```

In-process, the renderer runs on a thread of the module's own and no helper exists. It is Windows only: elsewhere `InProcess` fails the next window with `GuiStartFailed`, and `Auto` means the helper.

`Helper` refuses to fall back: a helper that cannot start is a terminating `GuiStartFailed`, naming why.

## Choose the GPU

[Get-GuiGpuAdapter](Get-GuiGpuAdapter.md) lists the adapters, and `Set-GuiOption -Adapter` prefers one by part of its name, for the next renderer:

```powershell
Remove-Module PWRS-GUI
Import-Module PWRS-GUI
Set-GuiOption -Mode Helper -Adapter 'Radeon'
```

The next window draws on it:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click -->
```powershell
Read-GuiChoice -Choice 'one', 'two'
Get-GuiOption | Select-Object Backend, AdapterInUse
```

```text
Backend AdapterInUse
------- ------------
Dx12    AMD Radeon(TM) Graphics
```

Microsoft's WARP, the software renderer DirectX 12 offers on every Windows machine, is listed as `Microsoft Basic Render Driver`, so `-Adapter 'Basic Render'` draws without the GPU:

```powershell
Remove-Module PWRS-GUI
Import-Module PWRS-GUI
Set-GuiOption -Mode Helper -Adapter 'Basic Render'
```

The next window draws on it:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click -->
```powershell
Read-GuiChoice -Choice 'one', 'two'
Get-GuiOption | Select-Object Backend, AdapterInUse
```

```text
Backend AdapterInUse
------- ------------
Dx12    Microsoft Basic Render Driver
```

Without `-Adapter`, the renderer takes a discrete GPU first; `-LowPower` takes an integrated one first. An empty `-Adapter` restores the renderer's own choice:

```powershell
Remove-Module PWRS-GUI
Import-Module PWRS-GUI
Set-GuiOption -Mode Helper -Adapter '' -LowPower
```

The next window draws on the adapter that choice gives:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click -->
```powershell
Read-GuiChoice -Choice 'one', 'two'
Get-GuiOption | Select-Object LowPower, Backend, AdapterInUse
```

```text
LowPower Backend AdapterInUse
-------- ------- ------------
    True Dx12    AMD Radeon(TM) Graphics
```

## Choose the graphics API

The renderer tries DirectX 12, Vulkan and then OpenGL on Windows, Metal on macOS, and Vulkan and then OpenGL on Linux and FreeBSD. On Windows it tries OpenGL only where the system has `opengl32.dll`. Windows Server Core has none, nor WARP, DirectX 12's renderer for a machine without a GPU. There the renderer draws with DirectX 12 on a GPU and its driver, and on a machine without one with OpenGL on the processor through the Mesa add-on, `PWRS-GUI.Mesa.win-x64` ([The Mesa Add-On](The-Mesa-Add-On.md)). Without the add-on the start fails, naming the libraries the system lacks and the add-on. The `PWRS_GUI_BACKEND` environment variable narrows that to the APIs it names, separated by commas, read when a renderer starts; here in a new PowerShell process of the same host:

```powershell
& (Get-Process -Id $PID).Path -NoProfile -Command {
    $env:PWRS_GUI_BACKEND = 'gl'
    Get-GuiGpuAdapter | Where-Object InUse | Select-Object Name, Backend, DeviceType
}
```

```text
Name                              Backend DeviceType
----                              ------- ----------
NVIDIA GeForce RTX 5070/PCIe/SSE2 Gl      Other
```

## Draw widget windows in a webview

`Set-GuiOption -Renderer Html` draws the windows of `Read-GuiChoice`, `Out-GuiGridView` and `Measure-GuiLatency` in the system's webview: WebView2 on Windows, WebKit on macOS and WebKitGTK on Linux and FreeBSD. The HTML renderer runs in a helper of its own, `pwrs-gui-web`, or in-process on Windows, as `-Mode` says, and `Get-GuiOption` names its engine and the engine's version:

```powershell
Remove-Module PWRS-GUI
Import-Module PWRS-GUI
Set-GuiOption -Mode Helper -Renderer Html
```

The next widget window draws in the webview:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click -->
<!-- varies: the WebView2 version -->
```powershell
Read-GuiChoice -Choice 'one', 'two'
Get-GuiOption | Select-Object Renderer, Running, Backend, AdapterInUse
```

```text
Renderer Running Backend  AdapterInUse
-------- ------- -------  ------------
    Html Helper  WebView2 156.0.4314.58
```

Where the webview cannot start, as on a server without the WebView2 Runtime, a widget window draws with egui and the Verbose stream says why. A page of your own, from [Show-GuiWindow](Show-GuiWindow.md), always draws in a webview, whichever renderer `-Renderer` names.

## Give each process its own web data

Pages store cookies and local storage, and by default every PowerShell process of the user stores them in one place, and on Windows runs its pages under one WebView2 browser process. A page in a second process reads what a page in the first stored; `-WebView PerProcess` gives a process data of its own:

```powershell
(Show-GuiWindow -Html '<script>localStorage.setItem("by", "the first process"); window.pwrs.close("stored");</script>').Result
& (Get-Process -Id $PID).Path -NoProfile -Command {
    (Show-GuiWindow -Html '<script>window.pwrs.close(String(localStorage.getItem("by")));</script>').Result
}
& (Get-Process -Id $PID).Path -NoProfile -Command {
    Set-GuiOption -WebView PerProcess
    (Show-GuiWindow -Html '<script>window.pwrs.close(String(localStorage.getItem("by")));</script>').Result
}
```

```text
stored
the first process
null
```

The environment variable `PWRS_GUI_WEBVIEW` sets the same choice for processes that have not called `Set-GuiOption -WebView`, and `-WebViewDataFolder` names a shared folder of your own, so only the processes given that folder share. [Web Page Data And Recovery](Web-Page-Data-And-Recovery.md) explains what each choice shares and what it costs.
