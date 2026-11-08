---
title: New-GuiColumn
weight: 20
---

Describes widgets stacked top to bottom: those its block writes, in order. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiColumn`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiColumn -Syntax
```

```text
New-GuiColumn [[-Content] <scriptblock>] [-Align <Align>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiColumn` writes a `PwrsGui.Widget` with the widgets its `-Content` block writes, which [Show-GuiWindow](Show-GuiWindow.md) draws one under another, each as wide as it needs and as the column allows. In a `-Content` block `Column` stands for `New-GuiColumn`, and the block of `Show-GuiWindow -Content` is itself a column: a window's widgets stack unless a [Row](New-GuiRow.md), [Grid](New-GuiGrid.md) or other container places them otherwise.

`-Align` places each widget across: at the left, `Start`, when it is not given; in the middle, `Center`; at the right, `End`; or with `Stretch`, as wide as the column, so buttons and text fields line up with each other.

A list, a table or a tree, a log, scroll area, chart, GPU surface or canvas given no height, and a widget with one of these inside, share the height the column's other widgets leave, at least three rows of controls each. Such a widget scrolls in its place rather than push the widgets after it out of the window.

The block runs once, as the window is described, and must write only widgets; `$null` is passed over. Any PowerShell may run in it, so a loop can write a widget per item.

## Parameters

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: an empty column |

The block that writes the column's widgets.

### -Align

| | |
|---|---|
| Type | `PwrsGui.Align` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Start` |

Where each widget goes across: `Start`, at the left; `Center`; `End`, at the right; or `Stretch`, as wide as the column.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the column where no widget inside it takes them, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the column.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Column`, its `Properties` with `Align` when it is given, and its `Children` the widgets the block wrote.

## Examples

A column of a text and the buttons a loop writes:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Stacked' {
    Column -Name Steps {
        Text 'Steps'
        foreach ($step in 'fetch', 'build') { Button $step -Name $step }
    }
}
(Get-GuiSnapshot -Window $w.Id).Tree.Children | ForEach-Object { '{0} {1}' -f $_.Kind, $_.Name }
Close-GuiWindow -Window $w.Id
```

```text
Text
Button fetch
Button build
```

A block writes widgets only:

```powershell
try { New-GuiColumn { 'not a widget' } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiNotAWidget,Pwrs.Modules.PWRS_GUI.NewGuiColumnCommand
the block of a Column wrote a System.String, which is not a widget; the widgets inside come from the short names, such as Text and Button, or from New-Gui<Widget>
```

## Errors

| Error ID | When |
|---|---|
| `GuiNotAWidget` | the block writes something other than a widget |

Described in the [Error Reference](Error-Reference.md).
