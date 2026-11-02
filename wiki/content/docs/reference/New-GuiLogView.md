---
title: New-GuiLogView
weight: 35
---

Describes a log of lines that stays scrolled to the newest. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiLogView`; the lines it takes from a SubEtha ring, `crates/pwrs-gui-proto/src/source.rs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiLogView -Syntax
```

```text
New-GuiLogView [[-Line] <string[]>] [-Width <double>] [-Height <double>] [-Ring <Object>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiLogView` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as lines of monospaced text in an area that scrolls, which stays at the newest line unless a person has scrolled up. `-Width` and `-Height` give the area's size, which stays the same however many lines it shows, so a log that starts empty opens its window around the room its lines will take. Without `-Width` the area is as wide as the window leaves it, and without `-Height` it shares the height its column leaves, at least three rows of controls. In a `-Content` block `LogView` stands for `New-GuiLogView`.

The lines are those `-Line` gives, those [Add-GuiItem](Add-GuiItem.md) adds, which reach the window without the lines it already has, and those another process writes to the SubEtha ring `-Ring` names, each item one line, its line break at the end dropped. `Update-GuiWidget -Property @{ Line = @() }` clears the log, and [Remove-GuiItem](Remove-GuiItem.md) refuses it: a log's lines are not items to take back. `-Ring` takes the object one of these wrote:

| Ring | Its items |
|---|---|
| `New-SubEthaRing` | sent with `SendFrame`, of any length |
| `New-SubEthaLocaleRing` | sent with `Send`, at most one slot, once the writer has moved the ring to `File` or `ShmFs` with `MigrateTo`: a locale ring starts in its writer's own memory, where no other process can read it |
| `New-SubEthaBroadcastRing` | pushed with `Push`, at most 52 bytes; every window reading the ring gets every item |

The renderer reads the ring on a thread of its own, woken by the ring when an item arrives, so nothing polls. A ring made with order stamps, and a capacity ring, whose resizes a reader in another process does not follow, are refused. [How To Feed A Widget From A SubEtha Ring](How-To-Feed-A-Widget-From-A-SubEtha-Ring.md) shows each kind of ring.

## Parameters

### -Line

| | |
|---|---|
| Type | `System.String[]` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: an empty log |

The lines shown when the window opens.

### -Width

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: as wide as the window leaves it |

How wide the log is, in points, zero or more. A line longer than that wraps.

### -Height

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: a share of the height its column leaves |

How high the log is, in points, zero or more.

### -Ring

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

A SubEtha ring another process writes, each item one line, as the description says. It is taken when the window opens; `Update-GuiWidget` cannot change it.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the log, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the log.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `LogView` and its `Properties` with `Line`, and `Width`, `Height` and `Ring` when they are given.

## Examples

Lines given, added, read back and cleared:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Build' { LogView 'started', 'fetching' -Name Log -Height 120 }
Add-GuiItem -Window $w.Id -Name Log -Item 'building', 'done'
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.lines
Update-GuiWidget -Window $w.Id -Name Log -Property @{ Line = @() }
@((Get-GuiSnapshot -Window $w.Id).Tree.Properties.lines).Count
Close-GuiWindow -Window $w.Id
```

```text
started
fetching
building
done
0
```

A log a second process writes through a SubEtha ring. The job stands for any process that writes to the ring; the loop gives up after 20 seconds:

```powershell
Import-Module SubEtha
$path = Join-Path ([IO.Path]::GetTempPath()) "log-$PID"
$ring = New-SubEthaRing -Path $path -Capacity 64
$w = Show-GuiWindow -NoWait -Title 'Deploy' { LogView -Name Log -Ring $ring -Height 120 }
$job = Start-Job -ArgumentList $path {
    param($path)
    Import-Module SubEtha
    $ring = New-SubEthaRing -Path $path -Capacity 64
    $producer = $ring.RegisterProducer()
    foreach ($line in 'web01 stopped', 'web01 updated', 'web01 started') {
        $null = $ring.SendFrame($producer, $line)
    }
    $ring.Dispose()
}
$null = Wait-Job $job
$deadline = [DateTime]::UtcNow.AddSeconds(20)
while (@((Get-GuiSnapshot -Window $w.Id).Tree.Properties.lines).Count -lt 3 -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.lines
Close-GuiWindow -Window $w.Id
$ring.Dispose()
Remove-Job $job
Remove-Item -Path "$path*" -Force
```

```text
web01 stopped
web01 updated
web01 started
```

A width and a height are zero or more points:

```powershell
try { New-GuiLogView -Width -1 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
try { New-GuiLogView -Height -1 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadSize,Pwrs.Modules.PWRS_GUI.NewGuiLogViewCommand
a LogView's Width is -1; it must be zero or more points
GuiBadSize,Pwrs.Modules.PWRS_GUI.NewGuiLogViewCommand
a LogView's Height is -1; it must be zero or more points
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadSize` | `-Width` or `-Height` is below 0 |
| `GuiBadProperty` | `-Ring` names no ring a widget reads: a ring made with order stamps, a capacity ring, or anything other than a SubEtha ring |
| `GuiOpeningOnly` | `Update-GuiWidget` sets `Ring`, which is taken when the window opens |
| `GuiNoItems` | `Remove-GuiItem` names a log |

Each is described in the [Error Reference](Error-Reference.md).
