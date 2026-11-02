---
title: Getting Started
weight: 1
---

This series builds one tool, LogLens, a viewer for log files, over eleven chapters. Each chapter adds to the same script, `LogLens.ps1`, and ends with the whole script as it then is. This chapter loads the module, writes a log to look at, picks it in the system's file dialog, shows its lines in a grid, and reports a log that is not there. Source: `crates/pwrs-gui/src/lib.rs`, which declares the commands, and the module manifest and `PWRS-GUI.psm1` that cargo-pwrs writes from `Cargo.toml`.

The examples on this page ran in order in PowerShell 7.6.9 and Windows PowerShell 5.1, in a folder of their own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Where an example waits for a person, the clicks were sent by `Invoke-GuiTestEvent` from a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows; a person's clicks give the same output.

## What it runs on

- Windows PowerShell 5.1, or PowerShell 7.4 or later.
- A desktop session that can show windows. On Windows a service's session cannot, and a command that would open a window says so.
- A GPU, or none: windows are drawn through DirectX 12, Vulkan or OpenGL, and on a machine without a GPU through Microsoft's WARP software renderer.

[Tested Platforms](Tested-Platforms.md) lists the machines and systems it has run on.

## Install and load the module

Install the module from the PowerShell Gallery, or build it as [Building From Source](Building-From-Source.md) shows:

<!-- skip: installs the module from the PowerShell Gallery -->
```powershell
Install-Module PWRS-GUI -Scope CurrentUser
```

PowerShell loads it when one of its commands is first used, or `Import-Module` loads it at once:

```powershell
Import-Module PWRS-GUI
Get-Command -Module PWRS-GUI | Where-Object Verb -ne New
```

```text
CommandType     Name                                               Version    Source
-----------     ----                                               -------    ------
Cmdlet          Add-GuiItem                                        0.1.0      PWRS-GUI
Cmdlet          Close-GuiWindow                                    0.1.0      PWRS-GUI
Cmdlet          Get-GuiCanvasView                                  0.1.0      PWRS-GUI
Cmdlet          Get-GuiDialog                                      0.1.0      PWRS-GUI
Cmdlet          Get-GuiGpuAdapter                                  0.1.0      PWRS-GUI
Cmdlet          Get-GuiGpuCamera                                   0.1.0      PWRS-GUI
Cmdlet          Get-GuiNativeEntry                                 0.1.0      PWRS-GUI
Cmdlet          Get-GuiOption                                      0.1.0      PWRS-GUI
Cmdlet          Get-GuiSnapshot                                    0.1.0      PWRS-GUI
Cmdlet          Get-GuiWindow                                      0.1.0      PWRS-GUI
Cmdlet          Import-GuiGpuImage                                 0.1.0      PWRS-GUI
Cmdlet          Invoke-GuiGpuCompute                               0.1.0      PWRS-GUI
Cmdlet          Invoke-GuiTestEvent                                0.1.0      PWRS-GUI
Cmdlet          Measure-GuiLatency                                 0.1.0      PWRS-GUI
Cmdlet          Out-GuiGridView                                    0.1.0      PWRS-GUI
Cmdlet          Read-GuiChoice                                     0.1.0      PWRS-GUI
Cmdlet          Read-GuiCredential                                 0.1.0      PWRS-GUI
Cmdlet          Read-GuiFile                                       0.1.0      PWRS-GUI
Cmdlet          Read-GuiFolder                                     0.1.0      PWRS-GUI
Cmdlet          Read-GuiGpuBuffer                                  0.1.0      PWRS-GUI
Cmdlet          Read-GuiGpuTexture                                 0.1.0      PWRS-GUI
Cmdlet          Read-GuiInput                                      0.1.0      PWRS-GUI
Cmdlet          Read-GuiSavePath                                   0.1.0      PWRS-GUI
Cmdlet          Remove-GuiCanvasView                               0.1.0      PWRS-GUI
Cmdlet          Remove-GuiGpuBuffer                                0.1.0      PWRS-GUI
Cmdlet          Remove-GuiGpuCamera                                0.1.0      PWRS-GUI
Cmdlet          Remove-GuiGpuTexture                               0.1.0      PWRS-GUI
Cmdlet          Remove-GuiItem                                     0.1.0      PWRS-GUI
Cmdlet          Reset-GuiGpuDevice                                 0.1.0      PWRS-GUI
Cmdlet          Save-GuiGpuSurface                                 0.1.0      PWRS-GUI
Cmdlet          Send-GuiMessage                                    0.1.0      PWRS-GUI
Cmdlet          Set-GuiCanvasView                                  0.1.0      PWRS-GUI
Cmdlet          Set-GuiFocus                                       0.1.0      PWRS-GUI
Cmdlet          Set-GuiGpuCamera                                   0.1.0      PWRS-GUI
Cmdlet          Set-GuiGpuSurface                                  0.1.0      PWRS-GUI
Cmdlet          Set-GuiOption                                      0.1.0      PWRS-GUI
Cmdlet          Set-GuiProgress                                    0.1.0      PWRS-GUI
Cmdlet          Show-GuiMessage                                    0.1.0      PWRS-GUI
Cmdlet          Show-GuiProgress                                   0.1.0      PWRS-GUI
Cmdlet          Show-GuiWindow                                     0.1.0      PWRS-GUI
Cmdlet          Update-GuiWidget                                   0.1.0      PWRS-GUI
Cmdlet          Wait-GuiEvent                                      0.1.0      PWRS-GUI
Cmdlet          Write-GuiGpuBuffer                                 0.1.0      PWRS-GUI
Cmdlet          Write-GuiGpuTexture                                0.1.0      PWRS-GUI
```

Beside these, a `New-Gui<Widget>` command describes each widget a window can contain, as [A Window Of Your Own](A-Window-Of-Your-Own.md) shows.

Nothing starts when the module loads. The first command that opens a window starts the renderer that draws it, and the renderer stays up for the rest of the session's windows.

## A log to look at

LogLens reads lines of four parts separated by spaces: a time, a level, the source that wrote the line, and a message. This writes such a log, `app.log`, which every chapter of the series uses:

```powershell
Set-Content -Path app.log -Value @'
2026-10-20T09:30:01 INFO web01 started
2026-10-20T09:30:02 INFO web02 started
2026-10-20T09:30:04 INFO db01 accepting connections
2026-10-20T09:30:15 INFO web01 GET /orders 200
2026-10-20T09:30:41 WARN web02 GET /orders took 1.8 s
2026-10-20T09:31:07 INFO web01 GET /orders/17 200
2026-10-20T09:31:30 ERROR db01 deadlock on table orders
2026-10-20T09:31:31 ERROR web01 GET /orders/17 500
2026-10-20T09:31:52 INFO web02 GET /health 200
2026-10-20T09:32:10 WARN db01 replica 2 behind by 4 s
2026-10-20T09:32:44 INFO web01 GET /orders 200
2026-10-20T09:33:05 INFO web02 POST /orders 201
2026-10-20T09:33:29 ERROR web02 POST /orders 503
2026-10-20T09:33:30 WARN web02 retrying POST /orders
2026-10-20T09:33:31 INFO web02 POST /orders 201
2026-10-20T09:34:12 INFO db01 checkpoint done
2026-10-20T09:34:48 WARN web01 cache miss rate 41%
2026-10-20T09:35:20 INFO web01 GET /orders 200
2026-10-20T09:35:57 ERROR db01 disk 92% full
2026-10-20T09:36:03 WARN db01 vacuum skipped
2026-10-20T09:36:40 INFO web02 GET /health 200
2026-10-20T09:37:15 ERROR web01 GET /orders 500
2026-10-20T09:37:16 ERROR web02 GET /orders 500
2026-10-20T09:37:30 WARN db01 replica 2 behind by 9 s
2026-10-20T09:38:02 INFO db01 replica 2 caught up
2026-10-20T09:38:40 INFO web01 GET /orders 200
2026-10-20T09:39:11 INFO web02 GET /orders 200
2026-10-20T09:39:55 INFO web01 stopping
'@
(Get-Content app.log).Count
```

```text
28
```

## Pick the log

`Read-GuiFile` shows the system's dialog for files to open and writes the path chosen, or nothing when the dialog is canceled. `-Filter` limits the files it shows, and `-Title` names the dialog:

<!-- person: $d = Wait-Dialog 'Open a log'; Invoke-GuiTestEvent -Dialog $d.Id -Value (Join-Path $PWD 'app.log') -->
```powershell
$path = Read-GuiFile -Filter 'Logs|*.log', 'All files|*' -Title 'Open a log'
Split-Path -Leaf $path
```

```text
app.log
```

The dialog is the system's own, the one other programs show for opening a file, so it looks as the system draws it.

## Show the lines in a grid

A line becomes an object with a property for each of its parts, which a grid can sort and filter by. `-split ' ', 4` splits a line at its first three spaces, so a message is not split at its own spaces:

```powershell
function ConvertFrom-LogLine {
    param([Parameter(ValueFromPipeline)] [string] $Line)
    process {
        $time, $level, $source, $message = $Line -split ' ', 4
        [pscustomobject]@{ Time = [datetime]$time; Level = $level; Source = $source; Message = $message }
    }
}
$entries = Get-Content -LiteralPath $path | ConvertFrom-LogLine
$entries.Count
$entries[6] | Format-List
```

```text
28

Time    : 10/20/2026 9:31:30 AM
Level   : ERROR
Source  : db01
Message : deadlock on table orders
```

`Out-GuiGridView` shows piped objects in a table that a person sorts by clicking a column's heading and filters by typing above the rows. With `-PassThru` it waits for OK and writes the rows selected, the original objects in their original order:

<!-- keep: getting-started-grid.png images/getting-started-grid.png -->
<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Grid -Event Select -Value 6, 7; Wait-Enabled $w Ok; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'getting-started-grid.png'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$picked = $entries | Out-GuiGridView -Title 'LogLens' -PassThru
$picked | Format-Table Level, Source, Message
```

```text
Level Source Message
----- ------ -------
ERROR db01   deadlock on table orders
ERROR web01  GET /orders/17 500
```

The window looked like this just before OK was pressed, as [Get-GuiSnapshot](Get-GuiSnapshot.md) `-Pixels` captured it in the run above, at 100% display scale:

![The LogLens grid window with the columns Time, Level, Source and Message, the 28 lines of app.log, the two ERROR lines at 9:31:30 and 9:31:31 selected, and OK and Cancel](/images/getting-started-grid.png)

Without `-PassThru` the command returns once its input has ended and its window has opened, and leaves the window open, as Out-GridView does.

## A log that is not there

`Show-GuiMessage` shows the system's message box, with an icon and buttons, and writes the button pressed. LogLens uses it when the log it is given does not exist:

<!-- keep: getting-started-missing.png images/getting-started-missing.png -->
<!-- person: $d = Wait-Dialog 'LogLens'; Save-DialogPng 'LogLens' (Join-Path $PWD 'getting-started-missing.png'); Invoke-GuiTestEvent -Dialog $d.Id -Value Ok -->
```powershell
$missing = Join-Path $PWD 'old.log'
if (-not (Test-Path -LiteralPath $missing)) {
    Show-GuiMessage 'old.log does not exist.' -Title 'LogLens' -Icon Error
}
```

```text
Ok
```

![A Windows message box titled LogLens with the error icon, the message "old.log does not exist." and an OK button](/images/getting-started-missing.png)

## The whole script so far

`LogLens.ps1` takes the log's path, or shows the file dialog when none is given, and shows the log's lines in the grid, writing the lines a person selects:

<!-- file: LogLens.ps1 -->
```powershell
param([string] $Path)
Import-Module PWRS-GUI

function ConvertFrom-LogLine {
    param([Parameter(ValueFromPipeline)] [string] $Line)
    process {
        $time, $level, $source, $message = $Line -split ' ', 4
        [pscustomobject]@{ Time = [datetime]$time; Level = $level; Source = $source; Message = $message }
    }
}

if (-not $Path) {
    $Path = Read-GuiFile -Filter 'Logs|*.log', 'All files|*' -Title 'Open a log'
    if (-not $Path) { return }
}
if (-not (Test-Path -LiteralPath $Path)) {
    $null = Show-GuiMessage "$(Split-Path -Leaf $Path) does not exist." -Title 'LogLens' -Icon Error
    return
}
Get-Content -LiteralPath $Path | ConvertFrom-LogLine | Out-GuiGridView -Title 'LogLens' -PassThru
```

Run it on the log, and select the line about the disk:

<!-- person: $w = Wait-Window 'LogLens'; Invoke-GuiTestEvent -Window $w.Id -Name Grid -Event Select -Value 18; Wait-Enabled $w Ok; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
.\LogLens.ps1 -Path .\app.log | Format-List
```

```text
Time    : 10/20/2026 9:35:57 AM
Level   : ERROR
Source  : db01
Message : disk 92% full
```

## See where it drew

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
WebGpu            :
```

On the machine these pages ran on, the renderer ran in a helper process, `pwrs-gui-egui`, drawing through DirectX 12 on its NVIDIA GPU. [How To Choose Where Windows Are Drawn](How-To-Choose-Where-Windows-Are-Drawn.md) changes either.

## Help at the prompt

Each command comes with its help, written from the same source as the reference pages, with examples:

```powershell
(Get-Help Read-GuiFile).Synopsis
(Get-Help Read-GuiFile).examples.example.code
```

```text
Shows the system's own dialog for files to open and writes the path of the file chosen, or with -Multiple of each file chosen; nothing when it is canceled. Ctrl+C in the console closes the dialog and writes nothing.
Read-GuiFile -Filter 'Logs|*.log', 'All files|*'
Read-GuiFile -Multiple -InitialDirectory ~/Documents | Get-Item
```

## Next

[Asking For Things](Asking-For-Things.md): a choice of the levels to show, a search, and a confirmation before LogLens opens a large log.
