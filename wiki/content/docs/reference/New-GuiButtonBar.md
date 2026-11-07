---
title: New-GuiButtonBar
weight: 56
---

Describes the window's buttons along its bottom, at the right, in the order the system's own dialogs put them. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiButtonBar`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiButtonBar -Syntax
```

```text
New-GuiButtonBar [[-Content] <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiButtonBar` writes a `PwrsGui.Widget` with the [Buttons](New-GuiButton.md) its `-Content` block writes, which [Show-GuiWindow](Show-GuiWindow.md) draws along the window's bottom, at the right, wherever in the block the bar is written. In a `-Content` block `ButtonBar` stands for `New-GuiButtonBar`.

The buttons go in the order the system's own dialogs put them, by each button's role:

| System | Order |
|---|---|
| Windows | the `-Default` button first, then the buttons without a role in the order written, then the `-Cancel` button: OK, Cancel |
| macOS, Linux and the BSDs | the buttons without a role in the order written, then the `-Cancel` button, then the `-Default` button: Cancel, OK |

Each button is at least 80 by 24 points, the buttons are 8 points apart, and the `-Default` button is drawn in the selection's colors while it is enabled. A window with a button bar draws its other widgets inside a 12-point margin, as a dialog does. On Windows the bar is a strip a shade apart from the window, with a line over it, as Windows' own dialogs draw it.

Enter pressed alone presses the `-Default` button, and Escape pressed alone presses the `-Cancel` button, unless the widget with the focus uses the key itself. A text field of several lines takes Enter for a new line, a list, table or tree takes it for its item, and a drop-down takes Escape to close its list. In a text field of one line, Enter raises the field's `Submit` and then presses the default button.

The bar is one of the window's own widgets, at the block's first level, and a window has one at most. Only Buttons go in it, and a window has one `-Default` button and one `-Cancel` button at most; Show-GuiWindow refuses any other window.

## Parameters

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: an empty bar |

The block that writes the bar's buttons.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands find the bar by.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `ButtonBar` and its `Children` the buttons the block wrote.

## Examples

A dialog to confirm a delete, its Delete button pressed:

<!-- person: $w = Wait-Window 'Old logs'; Invoke-GuiTestEvent -Window $w.Id -Name Delete -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Old logs' {
    Text 'Delete the logs older than a month?' -Style Instruction
    Text '214 files, 1.2 GB'
    ButtonBar {
        Button 'Delete' -Name Delete -Default -Result Delete
        Button 'Cancel' -Name Cancel -Cancel -Result Cancel
    }
}
$result.Result
```

```text
Delete
```

A button bar inside another widget opens no window:

```powershell
try { Show-GuiWindow { Column { ButtonBar { Button 'OK' -Result Ok } } } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiNotAtTop,Pwrs.Modules.PWRS_GUI.ShowGuiWindowCommand
a ButtonBar stands among the window's own widgets, not inside another
```

Nor does a window with two default buttons:

```powershell
try { Show-GuiWindow { ButtonBar { Button 'Save' -Default; Button 'Save All' -Default } } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiButtonRole,Pwrs.Modules.PWRS_GUI.ShowGuiWindowCommand
-Default is set on two buttons, Save and Save All; a window has one default button
```

## Errors

| Error ID | When |
|---|---|
| `GuiButtonRole` | the window has two button bars, two `-Default` buttons or two `-Cancel` buttons |
| `GuiNotAButton` | the bar has a widget in it that is not a Button |
| `GuiNotAtTop` | the bar is inside another widget |
| `GuiNotAWidget` | the block writes something other than a widget |

Each is described in the [Error Reference](Error-Reference.md).
