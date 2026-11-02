---
title: New-GuiSpacer
weight: 28
---

Describes empty space between widgets. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiSpacer`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiSpacer -Syntax
```

```text
New-GuiSpacer [[-Size] <double>] [-Fill] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiSpacer` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as empty space `-Size` points long, along the row or column it is in. In a [Column](New-GuiColumn.md), [Group](New-GuiGroup.md), [Tab](New-GuiTab.md) or [Scroll](New-GuiScroll.md), which stack their widgets, it is that high; in a [Row](New-GuiRow.md) or [StatusBar](New-GuiStatusBar.md) it is that wide, and adds no height. In a `-Content` block `Spacer` stands for `New-GuiSpacer`. It takes no drops and raises no event; [Update-GuiWidget](Update-GuiWidget.md) sets `Size`.

With `-Fill` the space takes the room its row or column leaves, at least `-Size` points, so the widgets after it go to the far end: to the right of a row, or to the bottom of a column. Several such spaces share the room equally.

## Parameters

### -Size

| | |
|---|---|
| Type | `System.Double` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | 8 |

The space, in points, zero or more; with `-Fill`, the least the space takes.

### -Fill

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `-Size` points, no more |

Makes the space take the room its row or column leaves.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands use for the space.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `Spacer` and its `Properties` with `Size` when it is given, and `Fill` when it is set.

## Examples

Space between a heading and its text, widened while the window is open:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Spaced' {
    Text 'Report' -Style Heading
    Spacer 24 -Name Gap
    Text 'Three files copied.'
}
(Get-GuiSnapshot -Window $w.Id).Tree.Children[1].Properties.size
Update-GuiWidget -Window $w.Id -Name Gap -Property @{ Size = 48 }
(Get-GuiSnapshot -Window $w.Id).Tree.Children[1].Properties.size
Close-GuiWindow -Window $w.Id
```

```text
24
48
```

A space that takes the row's room, sending the button to the far right, the button pressed:

<!-- person: $w = Wait-Window 'Far end'; Invoke-GuiTestEvent -Window $w.Id -Name Next -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Far end' {
    Row { Text 'Step 1 of 3'; Spacer -Fill; Button 'Next' -Name Next -Result Next }
}
$result.Result
```

```text
Next
```

A size is zero or more points:

```powershell
try { New-GuiSpacer -1 } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadSize,Pwrs.Modules.PWRS_GUI.NewGuiSpacerCommand
a Spacer's Size is -1; it must be zero or more points
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadSize` | `-Size` is below 0 |

Described in the [Error Reference](Error-Reference.md).
