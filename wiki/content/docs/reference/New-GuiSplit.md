---
title: New-GuiSplit
weight: 26
---

Describes two widgets side by side, or one above the other, with a divider between them that a person drags: the two its block writes. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiSplit`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiSplit -Syntax
```

```text
New-GuiSplit [[-Content] <scriptblock>] [-Orientation <Orientation>] [-Position <double>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiSplit` writes a `PwrsGui.Widget` that contains the two widgets its `-Content` block writes, which [Show-GuiWindow](Show-GuiWindow.md) draws side by side, the first on the left, or with `-Orientation Vertical` one above the other, the first on top. The divider starts at `-Position`, the share of the space the first widget takes, and a person drags it. A vertical split, or one with a list, table or tree in it, shares the height its column leaves. In a `-Content` block `Split` stands for `New-GuiSplit`. A list beside the details of what is chosen in it is the usual use.

The block runs once, as the window is described, and must write exactly two widgets; a [Column](New-GuiColumn.md) puts several on one side. [Update-GuiWidget](Update-GuiWidget.md) sets `Orientation` and `Position`.

## Parameters

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none |

The block that writes the two widgets, the first on the left or on top.

### -Orientation

| | |
|---|---|
| Type | `PwrsGui.Orientation` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Horizontal` |

`Horizontal`, side by side, or `Vertical`, one above the other.

### -Position

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | 0.5 |

The share of the space the first widget takes, from 0 to 1.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the split where neither widget takes them, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the split.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Split`, its `Properties` with `Orientation` and `Position` when they are given, and its `Children` the two widgets.

## Examples

A list beside its details, turned to one above the other:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Browse' {
    Split -Name Panes -Position 0.3 { ListBox 'alpha', 'beta' -Name Pick; Text 'Details' -Name Details }
}
Update-GuiWidget -Window $w.Id -Name Panes -Property @{ Orientation = 'Vertical'; Position = 0.5 }
(Get-GuiSnapshot -Window $w.Id).Tree.Properties
Close-GuiWindow -Window $w.Id
```

```text
Name                           Value
----                           -----
orientation                    vertical
position                       0.5
```

A split contains exactly two widgets:

```powershell
try { New-GuiSplit { New-GuiText 'one' } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiNotTwoWidgets,Pwrs.Modules.PWRS_GUI.NewGuiSplitCommand
a Split contains two widgets, and its block wrote 1
```

## Errors

| Error ID | When |
|---|---|
| `GuiNotTwoWidgets` | the block writes other than two widgets |
| `GuiBadNumber` | `-Position` is outside 0 to 1 |
| `GuiNotAWidget` | the block writes something other than a widget |

Each is described in the [Error Reference](Error-Reference.md).
