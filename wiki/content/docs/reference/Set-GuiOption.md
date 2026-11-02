---
title: Set-GuiOption
weight: 9
---

Sets where windows are drawn, which renderer draws widget windows, the theme windows open in, the fonts they draw text in, which GPU egui draws on, and where webviews put their data. Source: `crates/pwrs-gui/src/options.rs`, `SetGuiOption`; the adapter choice is `choose_adapter` in `crates/pwrs-gui-egui/src/lib.rs`, and the web data folders are `crates/pwrs-gui-web/src/data.rs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Where an example waits for a person, the clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows.

## Syntax

```powershell
Get-Command Set-GuiOption -Syntax
```

```text
Set-GuiOption [-Mode <Mode>] [-Renderer <Renderer>] [-Theme <Theme>] [-Font <Font>] [-Adapter <string>] [-LowPower] [-WebView <WebView>] [-WebViewDataFolder <string>] [<CommonParameters>]
```

## Description

`Set-GuiOption` changes the options the next renderer PWRS-GUI starts takes, and the renderer and theme the next window takes. A renderer already running goes on with the options it started with, and draws every window until no runspace of the process has the module imported, or the process ends; with `-Verbose` the command says when one is running. A parameter left out is left as it was. The options belong to the process, so a value set in one runspace applies in all of them, and [Get-GuiOption](Get-GuiOption.md) reads them back.

The mode comes from `-Mode` once it has been set, from the `PWRS_GUI_MODE` environment variable until then, and is `Auto` when neither says. The web data choice comes from `-WebView` once set, from `PWRS_GUI_WEBVIEW` until then, and is `Shared` when neither says.

## Parameters

### -Mode

| | |
|---|---|
| Type | `PwrsGui.Mode` |
| Parameter sets | all: named |
| Pipeline input | no |
| Values | `Auto`, `Helper`, `InProcess` |
| Default | the value set last, else `PWRS_GUI_MODE`, else `Auto` |
| Refused with | `CannotConvertArgumentNoMessage` from the binder for a value that names no mode |

Where windows are drawn. `Helper` draws them in a helper process, `pwrs-gui-egui`, which the module starts from a copy PWRS stages for the process. `InProcess` draws them on a thread of the module's own in the PowerShell process, and is Windows only: elsewhere the next command that opens a window fails with `GuiStartFailed`. `Auto` starts the helper and, on Windows, draws in-process when the helper cannot start, saying why in the Verbose stream.

### -Adapter

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the renderer chooses by device type |
| Refused with | `GuiStartFailed` from the next command that starts a renderer, when no adapter's name contains it |

Part of the name of the GPU adapter to draw with, compared without regard to case: of the adapters whose name contains it, the renderer takes the one on the first of its backends in the order below. An empty string restores the renderer's own choice: a discrete GPU, then an integrated one, then a virtual one, any other device, and a software adapter such as WARP, llvmpipe or lavapipe last. Among those, the renderer takes DirectX 12, then Vulkan, then OpenGL on Windows, Metal on macOS, and Vulkan, then OpenGL on Linux and FreeBSD. [Get-GuiGpuAdapter](Get-GuiGpuAdapter.md) lists the names.

### -LowPower

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Default | off |

Prefers the adapter that draws the least power: an integrated GPU before a discrete one, when `-Adapter` names none. `-LowPower:$false` turns it off again.

### -Renderer

| | |
|---|---|
| Type | `PwrsGui.Renderer` |
| Parameter sets | all: named |
| Pipeline input | no |
| Values | `Egui`, `Html` |
| Default | `Egui` |

Which renderer draws widget windows, those of `Read-GuiChoice`, `Out-GuiGridView` and `Measure-GuiLatency`: egui on the GPU, or a webview, which is WebView2 on Windows, WebKit on macOS and WebKitGTK on Linux and FreeBSD. When the HTML renderer cannot start, a widget window draws with egui and the Verbose stream says why. A page of your own, from [Show-GuiWindow](Show-GuiWindow.md), always draws in a webview, whatever this says.

### -Theme

| | |
|---|---|
| Type | `PwrsGui.Theme` |
| Parameter sets | all: named |
| Pipeline input | no |
| Values | `System`, `Light`, `Dark` |
| Default | `System` |
| Refused with | `CannotConvertArgumentNoMessage` from the binder for a value that names no theme |

The theme windows open in, in either renderer. `System` follows the system's light or dark setting, while a window is open too; `Light` and `Dark` draw every window that way whatever the system's setting is. The next window opened takes it, with no new renderer started, so a script sets it without changing the system's own setting.

### -Font

| | |
|---|---|
| Type | `PwrsGui.Font` |
| Parameter sets | all: named |
| Pipeline input | no |
| Values | `Bundled`, `System` |
| Default | `Bundled` |
| Refused with | `CannotConvertArgumentNoMessage` from the binder for a value that names neither |

The fonts windows draw text in. `Bundled` draws text in Inter and monospace text in JetBrains Mono NL, which come with the module, so text looks the same on every system and in both renderers. `System` draws in the system's own: its UI font, such as Segoe UI on Windows, and the monospace font its browser draws with. Either way the system's fonts draw the letters the chosen ones lack. The next renderer started takes it.

### -WebView

| | |
|---|---|
| Type | `PwrsGui.WebView` |
| Parameter sets | all: named |
| Pipeline input | no |
| Values | `Shared`, `PerProcess` |
| Default | the value set last, else `PWRS_GUI_WEBVIEW`, else `Shared` |
| Refused with | `CannotConvertArgumentNoMessage` from the binder for a value that names neither |

Whether the webviews of this process share their data with other processes'. `Shared` uses one data folder for every PowerShell process of the user, and on Windows one WebView2 browser process runs the pages of them all. A later process's first page then opens faster, and cookies and storage are shared, but a crash of that browser process ends every page at once, each of which is then loaded again. `PerProcess` gives each process a data folder of its own, and on Windows a browser process of its own; the first page of a process starts its own browser process cold. [Web Page Data And Recovery](Web-Page-Data-And-Recovery.md) explains both.

### -WebViewDataFolder

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: `<LocalAppData>\PWRS-GUI\WebView2` on Windows, the engine's own folder on Linux and FreeBSD |
| Refused with | `GuiNoFolder` for a path that names no folder |

The shared web data folder, in place of the default, used with `-WebView Shared`: processes given the same folder share their data and, on Windows, their browser process. A relative path is taken from the current location. An empty string restores the default. WebKit on macOS puts its data in a store of its own, so there the folder is not used, and the renderer says so when it starts. Setting a folder while `-WebView` is `PerProcess` writes a warning, since that mode gives each process a folder of its own.

## Inputs

None.

## Outputs

None.

## Examples

The examples from the command's help.

Draw windows in the helper process only, so that a failure to start it is an error rather than a fall back to drawing in-process:

```powershell
Set-GuiOption -Mode Helper
Get-GuiOption
```

```text
Mode              : Helper
Renderer          : Egui
Theme             : System
Font              : Bundled
Adapter           :
LowPower          : False
WebView           : Shared
WebViewDataFolder :
Running           :
Backend           :
AdapterInUse      :
WebGpu            :
```

Prefer an adapter by part of its name. The machine these pages ran on has an NVIDIA GeForce RTX 5070, which the renderer takes by default, and the Radeon graphics of its AMD Ryzen 9 7900X:

```powershell
Set-GuiOption -Adapter 'Radeon' -LowPower:$false
Get-GuiOption | Select-Object Mode, Adapter, LowPower
```

```text
  Mode Adapter LowPower
  ---- ------- --------
Helper Radeon     False
```

The next renderer started draws with it:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click -->
```powershell
Read-GuiChoice -Choice 'one', 'two'
Get-GuiOption | Select-Object Running, Backend, AdapterInUse
```

```text
Running Backend AdapterInUse
------- ------- ------------
Helper  Dx12    AMD Radeon(TM) Graphics
```

A change while a renderer runs applies to the next one:

```powershell
Set-GuiOption -Adapter '' -Verbose
```

```text
VERBOSE: a renderer is running; these options apply to the next one PWRS-GUI starts
```

Draw widget windows in a webview:

```powershell
Set-GuiOption -Renderer Html
Get-GuiOption | Select-Object Renderer, WebView, WebViewDataFolder
```

```text
Renderer WebView WebViewDataFolder
-------- ------- -----------------
    Html  Shared
```

Open every window in the light theme, whatever the system's setting:

```powershell
Set-GuiOption -Theme Light
Get-GuiOption | Select-Object Theme, Font
```

```text
Theme    Font
-----    ----
Light Bundled
```

Draw text in the system's own fonts from the next renderer started, then go back to the defaults:

```powershell
Set-GuiOption -Font System
Get-GuiOption | Select-Object Theme, Font
Set-GuiOption -Theme System -Font Bundled
```

```text
Theme   Font
-----   ----
Light System
```

Put this process's web data apart from every other process's:

```powershell
Set-GuiOption -WebView PerProcess
Get-GuiOption | Select-Object Renderer, WebView, WebViewDataFolder
```

```text
Renderer    WebView WebViewDataFolder
--------    ------- -----------------
    Html PerProcess
```

Share a data folder of your choosing. A relative path is taken from the current location, and a folder given while the data is per process is warned about; the example writes the warning with the current location shown as `.`:

```powershell
Set-GuiOption -WebViewDataFolder 'WebData' -WarningVariable said -WarningAction SilentlyContinue
$said.Message -replace [regex]::Escape($PWD.Path), '.'
Set-GuiOption -WebView Shared
(Get-GuiOption).WebViewDataFolder -eq (Join-Path $PWD.Path 'WebData')
Set-GuiOption -WebViewDataFolder '' -Renderer Egui
```

```text
the web data folder .\WebData is used with -WebView Shared; with PerProcess each process has a folder of its own
True
```

## Errors

A mode the binder cannot convert is refused before the command runs:

```powershell
try { Set-GuiOption -Mode Window } catch { $_.FullyQualifiedErrorId }
```

```text
CannotConvertArgumentNoMessage,Pwrs.Modules.PWRS_GUI.SetGuiOptionCommand
```

An `-Adapter` that matches no adapter's name is reported by the next command that starts a renderer, as `GuiStartFailed`, naming every adapter the renderer found. A `-WebView` value the binder cannot convert is refused the way a mode is:

```powershell
try { Set-GuiOption -WebView Sometimes } catch { $_.FullyQualifiedErrorId }
```

```text
CannotConvertArgumentNoMessage,Pwrs.Modules.PWRS_GUI.SetGuiOptionCommand
```

The [Error Reference](Error-Reference.md) lists every error the module raises.
