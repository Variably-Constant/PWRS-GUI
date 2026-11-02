---
title: Get-GuiOption
weight: 10
---

Gets the options PWRS-GUI starts its renderer with, what the running renderer uses, and what the running HTML renderer's pages draw GpuSurfaces with. Source: `crates/pwrs-gui/src/options.rs`, `GetGuiOption`, `GuiOption` and `current`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too.

## Syntax

```powershell
Get-Command Get-GuiOption -Syntax
```

```text
Get-GuiOption [<CommonParameters>]
```

## Description

`Get-GuiOption` writes one `PwrsGui.Option`. Its `Mode`, `Renderer`, `Theme`, `Font`, `Adapter`, `LowPower`, `WebView` and `WebViewDataFolder` are what the next renderer PWRS-GUI starts, or the next window, takes: what [Set-GuiOption](Set-GuiOption.md) set. A mode it has not set is read from the `PWRS_GUI_MODE` environment variable and a web data choice from `PWRS_GUI_WEBVIEW`, and they are `Auto` and `Shared` when those are not set either. Its `Running`, `Backend` and `AdapterInUse` describe the running session of the renderer `Renderer` names, and are null when none runs. Its `WebGpu` says what the running HTML renderer's pages draw GpuSurfaces with, whichever renderer `Renderer` names: `WebGPU` when the engine's WebGPU offers an adapter, and `WebGL2` otherwise. A page settles it the first time a GpuSurface needs it, so it is null until a page has drawn one, and while no HTML renderer runs.

The options belong to the process: every runspace reads and sets the same ones.

## Parameters

`Get-GuiOption` takes only the common parameters.

## Inputs

None.

## Outputs

`PwrsGui.Option`, one:

| Property | Type | What it gives |
|---|---|---|
| `Mode` | `PwrsGui.Mode` | `Auto`, `Helper` or `InProcess`: where the next renderer draws windows |
| `Renderer` | `PwrsGui.Renderer` | `Egui` or `Html`: the renderer that draws widget windows |
| `Theme` | `PwrsGui.Theme` | `System`, `Light` or `Dark`: the theme windows open in |
| `Font` | `PwrsGui.Font` | `Bundled` or `System`: the fonts the next renderer started draws text in |
| `Adapter` | `System.String` | part of an adapter's name to prefer; null for the default choice |
| `LowPower` | `System.Boolean` | whether the adapter that draws the least power is preferred |
| `WebView` | `PwrsGui.WebView` | `Shared` or `PerProcess`: whether webviews share their data with other processes' |
| `WebViewDataFolder` | `System.String` | the shared web data folder in place of the default, used with `Shared`; null for the default |
| `Running` | `System.String` | where the running session of that renderer draws, `Helper` or `InProcess`; null when none runs |
| `Backend` | `System.String` | that session's GPU backend, such as `Dx12` or `Vulkan`, or its webview engine, such as `WebView2`; null when none runs |
| `AdapterInUse` | `System.String` | that session's adapter, or its webview engine's version; null when none runs |
| `WebGpu` | `System.String` | `WebGPU` or `WebGL2`: what the running HTML renderer's pages draw GpuSurfaces with; null until a page has drawn one, and while no HTML renderer runs |

Before any window has opened:

```powershell
Get-GuiOption
```

```text
Mode              : Auto
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

```powershell
Get-GuiOption | Get-Member -MemberType Properties
```

```text
   TypeName: PwrsGui.Option

Name              MemberType Definition
----              ---------- ----------
Adapter           Property   string Adapter {get;set;}
AdapterInUse      Property   string AdapterInUse {get;set;}
Backend           Property   string Backend {get;set;}
Font              Property   PwrsGui.Font Font {get;set;}
LowPower          Property   bool LowPower {get;set;}
Mode              Property   PwrsGui.Mode Mode {get;set;}
Renderer          Property   PwrsGui.Renderer Renderer {get;set;}
Running           Property   string Running {get;set;}
Theme             Property   PwrsGui.Theme Theme {get;set;}
WebGpu            Property   string WebGpu {get;set;}
WebView           Property   PwrsGui.WebView WebView {get;set;}
WebViewDataFolder Property   string WebViewDataFolder {get;set;}
```

After a window has opened and closed, the renderer goes on running for the next one:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click -->
```powershell
Read-GuiChoice -Choice 'one', 'two'
Get-GuiOption
```

```text
Mode              : Auto
Renderer          : Egui
Theme             : System
Font              : Bundled
Adapter           :
LowPower          : False
WebView           : Shared
WebViewDataFolder :
Running           : Helper
Backend           : Dx12
AdapterInUse      : NVIDIA GeForce RTX 5070
WebGpu            :
```

With the HTML renderer, the running session is the webview's:

<!-- person: $w = Wait-Window 'Choose'; Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click -->
<!-- varies: the WebView2 version -->
```powershell
Set-GuiOption -Renderer Html
Read-GuiChoice -Choice 'one', 'two'
Get-GuiOption | Select-Object Renderer, Running, Backend, AdapterInUse
Set-GuiOption -Renderer Egui
```

```text
Renderer Running Backend  AdapterInUse
-------- ------- -------  ------------
    Html Helper  WebView2 156.0.4314.58
```

Once a page has drawn a GpuSurface, `WebGpu` says what the HTML renderer's pages draw with:

```powershell
Set-GuiOption -Renderer Html
$w = Show-GuiWindow -NoWait -Title 'Surface' { GpuSurface View -Width 64 -Height 64 }
Set-GuiGpuSurface View -ClearColor 0, 0, 1 -Window $w.Id
(Get-GuiOption).WebGpu
Close-GuiWindow -Window $w.Id
Set-GuiOption -Renderer Egui
```

```text
WebGPU
```

## Examples

The example from the command's help:

```powershell
Get-GuiOption
```

```text
Mode              : Auto
Renderer          : Egui
Theme             : System
Font              : Bundled
Adapter           :
LowPower          : False
WebView           : Shared
WebViewDataFolder :
Running           : Helper
Backend           : Dx12
AdapterInUse      : NVIDIA GeForce RTX 5070
WebGpu            : WebGPU
```

## Errors

`GuiModeUnknown` when `PWRS_GUI_MODE` names no mode, and `GuiModeUnreadable` when it cannot be read; `GuiWebViewUnknown` and `GuiWebViewUnreadable` likewise for `PWRS_GUI_WEBVIEW`. All are non-terminating, and described in the [Error Reference](Error-Reference.md). Reading what the running HTML renderer's pages draw with fails as any call to a renderer does, with the terminating `GuiRendererGone` when the renderer ended meanwhile.

```powershell
$env:PWRS_GUI_MODE = 'Window'
Get-GuiOption -ErrorVariable failed -ErrorAction SilentlyContinue
$failed.FullyQualifiedErrorId
$failed.Exception.Message
Remove-Item Env:\PWRS_GUI_MODE
```

```text
GuiModeUnknown,Pwrs.Modules.PWRS_GUI.GetGuiOptionCommand
PWRS_GUI_MODE is 'Window'; it must be Auto, Helper or InProcess
```
