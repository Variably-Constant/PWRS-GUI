---
title: New-GuiProgressBar
weight: 33
---

Describes a progress bar: how far work has gone, out of a maximum, a thin bar under a line of text and the percentage done. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiProgressBar`; the items it takes from a SubEtha ring, `crates/pwrs-gui-proto/src/source.rs`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiProgressBar -Syntax
```

```text
New-GuiProgressBar [[-Value] <double>] [-Maximum <double>] [-Text <string>] [-Ring <Object>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiProgressBar` writes a `PwrsGui.Widget` describing a progress bar, which [Show-GuiWindow](Show-GuiWindow.md) draws in the window its `-Content` block describes; in that block `ProgressBar` stands for `New-GuiProgressBar`. It is drawn as Windows and macOS draw one: a thin bar, filled from the left to `-Value` out of `-Maximum` in the selection's color, under a line with `-Text` at the left and the percentage done at the right. Text too long for the line ends in an ellipsis before the percentage. It is as wide as the room it is given, 96 points at least.

The bar moves when [Update-GuiWidget](Update-GuiWidget.md) sets its `Value`, `Maximum` or `Text`, or when another process writes an item to the SubEtha ring `-Ring` names. Each item is a JSON object of any of `Value`, `Maximum` and `Text`, keys in any case, such as `{"Value": 40, "Text": "copying"}`, and moves the bar as `Update-GuiWidget` would. The renderer reads the ring on a thread of its own, woken by the ring when an item arrives, so nothing polls. `-Ring` takes the object one of these wrote:

| Ring | Its items |
|---|---|
| `New-SubEthaRing` | sent with `SendFrame`, of any length |
| `New-SubEthaLocaleRing` | sent with `Send`, at most one slot, once the writer has moved the ring to `File` or `ShmFs` with `MigrateTo`: a locale ring starts in its writer's own memory, where no other process can read it |
| `New-SubEthaBroadcastRing` | pushed with `Push`, at most 52 bytes; every window reading the ring gets every item |

An item the bar cannot take, such as a `Value` past the `Maximum`, is written as a warning naming the item and why, and the bar shows what it showed before. A ring made with order stamps, and a capacity ring, whose resizes a reader in another process does not follow, are refused. [How To Feed A Widget From A SubEtha Ring](How-To-Feed-A-Widget-From-A-SubEtha-Ring.md) shows each kind of ring.

## Parameters

### -Value

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | 0 |

How far the work has gone, from 0 to `-Maximum`.

### -Maximum

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 100 |

The value at which the work is done, above 0.

### -Text

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The text over the bar, at the left of the percentage.

### -Ring

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

A SubEtha ring another process writes, each item moving the bar, as the description says. It is taken when the window opens; `Update-GuiWidget` cannot change it.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the bar, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the bar.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, which a `-Content` block and the widgets that contain others take: its `Kind` is `ProgressBar`, and its `Properties` have the parameters given, by name.

## Examples

A bar a script moves as it works, read back from the window:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Copying' { ProgressBar -Name Copy -Text 'Copying' }
foreach ($done in 25, 50, 75) {
    Update-GuiWidget -Window $w.Id -Name Copy -Property @{ Value = $done; Text = "$done of 100" }
}
(Get-GuiSnapshot -Window $w.Id).Tree.Properties
Close-GuiWindow -Window $w.Id
```

```text
Name                           Value
----                           -----
text                           75 of 100
value                          75
```

A bar a second process moves through a SubEtha ring. The job stands for any process that writes to the ring; the loop gives up after 20 seconds:

```powershell
Import-Module SubEtha
$path = Join-Path ([IO.Path]::GetTempPath()) "progress-$PID"
$ring = New-SubEthaRing -Path $path -Capacity 64
$w = Show-GuiWindow -NoWait -Title 'Building' { ProgressBar -Name Build -Ring $ring }
$job = Start-Job -ArgumentList $path {
    param($path)
    Import-Module SubEtha
    $ring = New-SubEthaRing -Path $path -Capacity 64
    $producer = $ring.RegisterProducer()
    foreach ($step in 1..4) {
        $null = $ring.SendFrame($producer, "{""Value"": $($step * 25), ""Text"": ""step $step of 4""}")
    }
    $ring.Dispose()
}
$null = Wait-Job $job
$deadline = [DateTime]::UtcNow.AddSeconds(20)
while ((Get-GuiSnapshot -Window $w.Id).Tree.Properties.value -ne 100 -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.text
Close-GuiWindow -Window $w.Id
$ring.Dispose()
Remove-Job $job
Remove-Item -Path "$path*" -Force
```

```text
step 4 of 4
```

A value past the maximum is refused as the bar is made:

```powershell
try { New-GuiProgressBar -Value 120 -Maximum 100 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadNumber,Pwrs.Modules.PWRS_GUI.NewGuiProgressBarCommand
a ProgressBar's Value is 120; it must be from 0 to its Maximum 100
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadNumber` | `-Maximum` is not above 0, or `-Value` is outside 0 to the maximum, as the bar is made or updated |
| `GuiBadProperty` | `-Ring` names no ring a widget reads: a ring made with order stamps, a capacity ring, or anything other than a SubEtha ring |
| `GuiOpeningOnly` | `Update-GuiWidget` sets `Ring`, which is taken when the window opens |

Each is described in the [Error Reference](Error-Reference.md).
