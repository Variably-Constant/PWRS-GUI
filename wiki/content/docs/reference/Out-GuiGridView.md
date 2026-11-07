---
title: Out-GuiGridView
weight: 2
---

Shows the piped objects in a window as a table that sorts by any column, filters as a person types and selects rows, with Out-GridView's parameters. Source: `crates/pwrs-gui/src/grid.rs`, `OutGuiGridView`, `COLUMNS`, `Grid` and `cell`; the table is drawn by `crates/pwrs-gui-egui/src/lib.rs`, and in an HTML window by `crates/pwrs-gui-web/src/runtime/runtime.js`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks came from `Invoke-GuiTestEvent` in a second runspace of the same process, as [How To Test A Window](How-To-Test-A-Window.md) shows. They reach the window as a person's clicks do.

## Syntax

```powershell
Get-Command Out-GuiGridView -Syntax
```

```text
Out-GuiGridView [-InputObject <Object>] [-Title <string>] [-PassThru] [<CommonParameters>]

Out-GuiGridView [-InputObject <Object>] [-Title <string>] [-Wait] [<CommonParameters>]

Out-GuiGridView [-InputObject <Object>] [-Title <string>] [-OutputMode <OutputMode>] [<CommonParameters>]
```

The three lines are the parameter sets: `PassThru` (the default), `Wait` and `OutputMode`.

## Description

The window opens with the first object. Its title is the command line that ran `Out-GuiGridView`, unless `-Title` names one. Rows arrive while the pipeline runs. Objects that come quickly are converted and sent 256 at a time; an object that comes a tenth of a second or more after the last rows were sent goes at once, with any rows waiting.

The columns come from the first object:

- When `Get-FormatData` gives one of its type names a table view, the columns are that view's, with its labels and format strings. Each property or script block is evaluated as the console's table evaluates it.
- Otherwise there is one column per property of the object's default display set, or of all its properties when it has no such set.
- A string or a value type is one column, itself.

Clicking a column's heading sorts by it, which the heading shows with ↑, and clicking again reverses the order, ↓. Numbers, dates and time spans sort by value, other cells by their text, and empty cells go last. Numbers and time spans are at the right of their columns and dates and text at the left, unless the table view places them. The headings are drawn in the strong text color, framed only while the pointer is over one, above a line. Each column is as wide as its widest cell, or as its heading with room for the sort arrow, so sorting moves no column. The last column takes the room left when its cells are at the left; a last column of numbers stays at its own width, and the room after it stays empty. Dragging the line between two columns resizes the one before it, and a double click on the line returns it to that width. The Filter box above the table shows only the rows with a cell whose text contains what is typed, without regard to case. Beside it, a count says how many of the window's rows are shown. A snapshot's `State` gives the table's `selected` rows, its `rows` and `shown` counts, and its `filter` text.

How the command ends depends on its parameters:

| Parameters | The command |
|---|---|
| none, or `-OutputMode None` | returns once its input has ended and its window has opened; the window stays open, as Out-GridView's does, until it is closed |
| `-Wait` | waits for the window to be closed, and writes nothing |
| `-PassThru`, or `-OutputMode Multiple` | waits for OK, then writes the rows selected |
| `-OutputMode Single` | waits for OK or a double-click on a row, then writes the one row |

A click selects a row. Where several rows can be chosen, Ctrl+click (Cmd+click on macOS) adds a row to the selection or removes it, and Shift+click selects the rows between the last one clicked and this one. The rows written are the original objects, in input order, whatever the table's sort. Cancel, closing the window and Ctrl+C in the console write nothing.

To copy rows, as in Out-GridView, select them, which gives the rows the keyboard focus, and press Ctrl+C (Cmd+C on macOS). The rows selected are copied as text in the order shown, each cell as the grid shows it followed by a tab, and each row by the system's line ending, so they paste into any text or spreadsheet program. You cannot copy columns or parts of rows, and you cannot copy the heading row.

## Parameters

### -InputObject

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | by value |

The objects to show, one row each. A null object is passed over.

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the command line that ran `Out-GuiGridView` |

The window's title.

### -PassThru

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | `PassThru`: named |

Waits for OK and writes the rows selected, as `-OutputMode Multiple` does.

### -Wait

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | `Wait`: named |

Waits for the window to be closed, and writes nothing.

### -OutputMode

| | |
|---|---|
| Type | `PwrsGui.OutputMode` |
| Parameter sets | `OutputMode`: named |
| Values | `None`, `Single`, `Multiple` |
| Default | `None` |
| Refused with | `CannotConvertArgumentNoMessage` from the binder for a value that names no mode |

What is written: nothing, one row chosen, or any rows chosen.

## Inputs

Any object, by value to `-InputObject`.

## Outputs

With `-PassThru` or `-OutputMode Single` or `Multiple`, the objects chosen, as they were piped: `System.Object`, of whatever type each is. Otherwise nothing.

The columns a process gets from its table view, read from the window while it is open. The command returns before its window has opened, and [Get-GuiWindow](Get-GuiWindow.md) lists a window once it has, so the example waits for it:

```powershell
Get-Process -Id $PID | Out-GuiGridView -Title 'One process'
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (-not ($window = Get-GuiWindow -Title 'One process') -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
$table = (Get-GuiSnapshot -Window $window.Id).Tree.Children | Where-Object Name -eq Grid
$table.Properties.columns.label -join ' | '
```

```text
NPM(K) | PM(M) | WS(M) | CPU(s) | Id | SI | ProcessName
```

Windows PowerShell 5.1's table view for a process has columns of its own:

<!-- powershell -->
```text
Handles | NPM(K) | PM(K) | WS(K) | CPU(s) | Id | SI | ProcessName
```

## Examples

The examples from the command's help.

Show processes and return at once, leaving the window open; its title is the command line. Once the window has opened, `Get-GuiWindow` lists it beside the one before:

```powershell
Get-Process | Out-GuiGridView
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while (-not (Get-GuiWindow -Title 'Get-Process | Out-GuiGridView') -and [DateTime]::UtcNow -lt $deadline) {
    Start-Sleep -Milliseconds 50
}
Get-GuiWindow | Select-Object Id, Title
```

```text
Id Title
-- -----
 1 One process
 6 Get-Process | Out-GuiGridView
```

Choose services and pass them on; here the first and the last of three are selected:

<!-- keep: out-guigridview-services.png images/out-guigridview-services.png -->
<!-- person: $w = Wait-Window 'Pick services'; Invoke-GuiTestEvent -Window $w.Id -Name Grid -Event Select -Value 0, 2; Wait-Enabled $w Ok; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'out-guigridview-services.png'); Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
Get-Service -Name Dhcp, Dnscache, W32Time | Out-GuiGridView -Title 'Pick services' -PassThru | Restart-Service -WhatIf
```

```text
What if: Performing the operation "Restart-Service" on target "DHCP Client (Dhcp)".
What if: Performing the operation "Restart-Service" on target "Windows Time (W32Time)".
```

The window just before OK was pressed, as [Get-GuiSnapshot](Get-GuiSnapshot.md) `-Pixels` captured it in that run:

![The Pick services window: a Filter box and the count of rows shown, a table of three services with the columns Status, Name and DisplayName, the first and last rows selected, and OK and Cancel at the bottom right](/images/out-guigridview-services.png)

Choose one file, here with a double-click on the second row:

```powershell
'a', 'b', 'c' | ForEach-Object { Set-Content -Path ".\$_.txt" -Value $_ }
```

<!-- person: $w = Wait-Window 'Choose a file'; Invoke-GuiTestEvent -Window $w.Id -Name Grid -Event Activate -Value 1 -->
```powershell
(Get-ChildItem -Filter *.txt | Out-GuiGridView -OutputMode Single -Title 'Choose a file').Name
```

```text
b.txt
```

Cancel ends the wait and writes nothing:

<!-- person: $w = Wait-Window 'Nothing chosen'; Invoke-GuiTestEvent -Window $w.Id -Name Cancel -Event Click -->
```powershell
$none = 1..5 | Out-GuiGridView -Title 'Nothing chosen' -PassThru
$null -eq $none
```

```text
True
```

## Errors

An object with no properties to show is refused with the terminating `GuiNoColumns`:

```powershell
try { [pscustomobject]@{} | Out-GuiGridView } catch { '{0}: {1}' -f $_.FullyQualifiedErrorId, $_.Exception.Message }
```

```text
GuiNoColumns,Pwrs.Modules.PWRS_GUI.OutGuiGridViewCommand: the first object has no properties to show
```

The [Error Reference](Error-Reference.md) describes every error the module raises.
