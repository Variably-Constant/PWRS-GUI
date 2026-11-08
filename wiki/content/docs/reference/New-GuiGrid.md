---
title: New-GuiGrid
weight: 22
---

Describes widgets in a grid of columns, filled a row at a time from those its block writes, in order. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiGrid`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiGrid -Syntax
```

```text
New-GuiGrid [[-Content] <scriptblock>] -Columns <long> [-Align <Align>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiGrid` writes a `PwrsGui.Widget` with the widgets its `-Content` block writes, which [Show-GuiWindow](Show-GuiWindow.md) places `-Columns` to a row, left to right and then top to bottom, each column as wide as its widest widget. In a `-Content` block `Grid` stands for `New-GuiGrid`. A form of labels and fields is the usual use: a label and its field make each row of a two-column grid, and the fields line up.

`-Align` places each widget in its cell: at the left, `Start`, when it is not given; in the middle, `Center`; at the right, `End`; or with `Stretch`, as wide as its column, the last column taking the room the others leave across. A form built with `Stretch` has fields of one width that grows with the window, as [Read-GuiInput](Read-GuiInput.md)'s form does.

The block runs once, as the window is described, and must write only widgets; `$null` is passed over. [Update-GuiWidget](Update-GuiWidget.md) sets `Columns`, and the grid places its widgets again.

## Parameters

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: an empty grid |

The block that writes the grid's widgets.

### -Columns

| | |
|---|---|
| Type | `System.Int64` |
| Parameter sets | all: named, mandatory |
| Pipeline input | no |
| Default | none |

How many widgets go in each row, 1 or more.

### -Align

| | |
|---|---|
| Type | `PwrsGui.Align` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Start` |

Where each widget goes in its cell: `Start`, at the left; `Center`; `End`, at the right; or `Stretch`, as wide as its column, the last column taking the room left across.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the grid where no widget inside it takes them, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the grid.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Grid`, its `Properties` with `Columns`, and `Align` when it is given, and its `Children` the widgets the block wrote.

## Examples

A form whose fields line up, as wide as the window lets them be, its fields' values read back:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Connect' {
    Grid -Columns 2 -Align Stretch -Name Form {
        Text 'Server'; TextBox 'db01' -Name Server
        Text 'Copies'; NumberBox 2 -Name Copies
    }
}
$state = (Get-GuiSnapshot -Window $w.Id).State
'Server {0}, copies {1}' -f $state.Server, $state.Copies
Close-GuiWindow -Window $w.Id
```

```text
Server db01, copies 2
```

A grid has one column or more:

```powershell
try { New-GuiGrid -Columns 0 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadProperty,Pwrs.Modules.PWRS_GUI.NewGuiGridCommand
a Grid's -Columns is 0; it must be a whole number above 0
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadProperty` | `-Columns` is below 1 |
| `GuiNotAWidget` | the block writes something other than a widget |

Each is described in the [Error Reference](Error-Reference.md).
