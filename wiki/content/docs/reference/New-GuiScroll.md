---
title: New-GuiScroll
weight: 27
---

Describes widgets stacked in an area that scrolls: those its block writes, in order. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiScroll`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiScroll -Syntax
```

```text
New-GuiScroll [[-Content] <scriptblock>] [-Height <double>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiScroll` writes a `PwrsGui.Widget` that contains the widgets its `-Content` block writes, which [Show-GuiWindow](Show-GuiWindow.md) draws one under another inside an area with a scroll bar, so a window can contain more than it shows. With `-Height` the area is at most that high; without it the area shares the height its column leaves. In a `-Content` block `Scroll` stands for `New-GuiScroll`.

The block runs once, as the window is described, and must write only widgets; `$null` is passed over. [Update-GuiWidget](Update-GuiWidget.md) sets `Height`.

## Parameters

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: an empty area |

The block that writes the area's widgets.

### -Height

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: a share of the height its column leaves |

The most the area is high, in points, zero or more.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the area where no widget inside it takes them, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the area.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Scroll`, its `Properties` with `Height` when it is given, and its `Children` the widgets the block wrote.

## Examples

Fifty lines in an area 120 points high, made taller while the window is open:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Lines' {
    Scroll -Height 120 -Name Lines { 1..50 | ForEach-Object { Text "line $_" } }
}
$area = (Get-GuiSnapshot -Window $w.Id).Tree
'{0} lines, {1} points high' -f $area.Children.Count, $area.Properties.height
Update-GuiWidget -Window $w.Id -Name Lines -Property @{ Height = 200 }
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.height
Close-GuiWindow -Window $w.Id
```

```text
50 lines, 120 points high
200
```

A height is zero or more points:

```powershell
try { New-GuiScroll -Height -1 { New-GuiText 'x' } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadSize,Pwrs.Modules.PWRS_GUI.NewGuiScrollCommand
a Scroll's Height is -1; it must be zero or more points
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadSize` | `-Height` is below 0 |
| `GuiNotAWidget` | the block writes something other than a widget |

Each is described in the [Error Reference](Error-Reference.md).
