---
title: New-GuiRow
weight: 21
---

Describes widgets side by side: those its block writes, in order. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiRow`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiRow -Syntax
```

```text
New-GuiRow [[-Content] <scriptblock>] [-Align <Align>] [-OnDrop <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiRow` writes a `PwrsGui.Widget` with the widgets its `-Content` block writes, which [Show-GuiWindow](Show-GuiWindow.md) draws left to right, each as wide as it needs and centered top to bottom. In a `-Content` block `Row` stands for `New-GuiRow`.

`-Align` places the widgets as a group: at the left, `Start`, when it is not given; in the middle, `Center`; at the right, `End`; or with `Stretch`, sharing the row's width equally. A [Spacer](New-GuiSpacer.md) with `-Fill` among them takes the room the others leave, so the widgets after it go to the row's far end.

A row suits a line of widgets, such as a field and the button beside it. A dialog's OK and Cancel go in a [ButtonBar](New-GuiButtonBar.md), and a [Grid](New-GuiGrid.md) lines widgets up in columns as well. A list, table or tree in a row takes the row's height, and the row then shares the height its column leaves, as such a widget does.

The block runs once, as the window is described, and must write only widgets; `$null` is passed over.

## Parameters

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: an empty row |

The block that writes the row's widgets.

### -Align

| | |
|---|---|
| Type | `PwrsGui.Align` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Start` |

Where the widgets go across: `Start`, at the left; `Center`; `End`, at the right; or `Stretch`, sharing the row's width equally.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the row where no widget inside it takes them, with the event as its argument; its `Value` is the paths dropped, or the text.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands and the handlers' events use for the row.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Row`, its `Properties` with `Align` when it is given, and its `Children` the widgets the block wrote.

## Examples

A line with its text at the left and its button at the far right, the button pressed:

<!-- person: $w = Wait-Window 'Jobs'; Invoke-GuiTestEvent -Window $w.Id -Name Refresh -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Jobs' {
    Row { Text 'Three jobs queued'; Spacer -Fill; Button 'Refresh' -Name Refresh -Result Refresh }
}
$result.Result
```

```text
Refresh
```

Two buttons sharing the window's width, one of them pressed:

<!-- person: $w = Wait-Window 'Sides'; Invoke-GuiTestEvent -Window $w.Id -Name Right -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Sides' {
    Row -Align Stretch { Button 'Left' -Name Left -Result Left; Button 'Right' -Name Right -Result Right }
}
$result.Result
```

```text
Right
```

## Errors

| Error ID | When |
|---|---|
| `GuiNotAWidget` | the block writes something other than a widget |

Described in the [Error Reference](Error-Reference.md).
