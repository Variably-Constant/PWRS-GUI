---
title: New-GuiTable
weight: 49
---

Describes a table of objects, one row each, that sorts by any column and selects rows, and with `-Filter` shows only the rows with the text a person types. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiTable`; the columns, from the objects' table view, `crates/pwrs-gui/src/grid.rs`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The selections and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiTable -Syntax
```

```text
New-GuiTable [[-InputObject] <Object[]>] [-Property <string[]>] [-Selection <Selection>] [-Filter] [-OnSelect <scriptblock>] [-OnActivate <scriptblock>] [-OnDrop <scriptblock>] [-OnKey <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiTable` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a table of the objects `-InputObject` gives and the pipeline passes, one row each, in order: the grid view's widget, which [Out-GuiGridView](Out-GuiGridView.md) shows on its own. Its columns are the properties `-Property` names, or the columns of the table view `Get-FormatData` gives the first object's type, or all its properties when it has none. A person sorts by clicking a column's heading, which then shows ↑, and clicks it again to sort the other way, ↓. With `-Filter`, a field above the rows shows only the rows with the text typed in it, without regard to case, beside how many rows it shows of how many. In a `-Content` block `Table` stands for `New-GuiTable`.

A column's cells go where the table view places them, when the columns come from one; otherwise a column whose first row is a number or a length of time goes at the right, as PowerShell's tables place numbers, and any other at the left, dates among them. The headings are drawn in the strong text color, framed only while the pointer is over one, above a line. Each column is as wide as its widest cell, or as its heading with room for the sort arrow, so sorting moves no column. The last column takes the room left when its cells are at the left; a last column of numbers stays at its own width, so its values stay beside the rest of their row, and the room after it stays empty, the row's shading reaching across it. A line is drawn between each two columns: dragging it widens or narrows the column before it, never below that width, and a double click on it returns the column to that width. Rows are striped from the first, and the row under the pointer is shaded. The table shares the height its column leaves and scrolls when its rows do not fit.

`-Selection` says how many rows can be selected: `None`, `Single` or `Multiple`. A click anywhere on a row selects it, its cells' text included. A change of selection raises `Select`, which `-OnSelect` acts on, and a double click or Enter raises `Activate` with the row's object, which `-OnActivate` acts on; with `-Selection None` neither raises anything. `State` and the events give the objects the rows were made from, as they were given: one, or with `-Selection Multiple` a list. Rows reach the window in batches while it opens. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) selects and activates rows by their numbers in the order given, from 0.

[Add-GuiItem](Add-GuiItem.md) adds rows after those there while the window is open, their cells from the columns the table opened with, or from the first row's when it opened with neither an object nor `-Property`. [Remove-GuiItem](Remove-GuiItem.md) takes away the rows made from the objects it is given, found as a tree's nodes are: the same object, or text or a number of the same value; selected rows that stay are still selected. [Update-GuiWidget](Update-GuiWidget.md) sets `Selection` and `Filter`; `Item` and `Property` are taken when the window opens.

Rows `Add-GuiItem` adds while the table shows its last row scroll it to its new last row, as a [LogView](New-GuiLogView.md) stays at its newest line, so a table fed as a log grows shows each new row. A table scrolled away from its end stays where it is, and so does a grid that [Out-GuiGridView](Out-GuiGridView.md) fills as its input arrives.

A press among the rows or below the last one, the Tab key or [Set-GuiFocus](Set-GuiFocus.md) gives the rows the keyboard focus, as one widget. A press below the last row selects no row, as in Explorer's and Out-GridView's lists, so the keys work at once. A `-Filter` field above the rows takes the focus on its own. The row the keyboard is on scrolls into sight and is outlined while the window is the active one, and the keys go through the rows in the order shown, sorted and filtered. Up, Down, Home, End, PageUp and PageDown move to a row and select it, as a click there would; Space selects the row, and Enter raises `Activate` when the row is selected. With `-Selection Multiple`, Shift selects the rows from where the selection started, Ctrl moves without selecting, Space adds the row or takes it away, and Ctrl+A selects every row shown. With `-Selection None` the keys move the outline alone, and Enter presses the window's default button. `-OnKey` gets every key but the arrows, Home, End, the page keys, Space, Enter, Ctrl+A and Ctrl+C.

To copy rows, select them and press Ctrl+C (Cmd+C on macOS) while the rows have the focus, as in Out-GridView. The rows selected are copied as text in the order shown, each cell as the table shows it followed by a tab, and each row by the system's line ending, so they paste into any text or spreadsheet program. You cannot copy columns or parts of rows, and you cannot copy the heading row. With no row selected, Ctrl+C copies nothing.

## Parameters

### -InputObject

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | all: position 0 |
| Pipeline input | yes, by value |
| Default | none: an empty table |

The objects, one row each; piped objects are added in the order piped.

### -Property

| | |
|---|---|
| Type | `System.String[]` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the first object's table view, or all its properties |

The properties shown as columns, in order.

### -Selection

| | |
|---|---|
| Type | `PwrsGui.Selection` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Single` |

How many rows can be selected: `None`, `Single` or `Multiple`.

### -Filter

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | no field |

Puts a field above the rows that shows only the rows with the text typed in it, without regard to case, beside how many rows it shows of how many.

### -OnSelect

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the selection changes, with the event as its argument and as `$_`; its `Value` is the object selected, or with `-Selection Multiple` the objects.

### -OnActivate

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a row is double-clicked or chosen with Enter, with the event as its argument; its `Value` is the row's object.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the table, with the event as its argument; its `Value` is the paths dropped, or the text.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key the table does not use itself is pressed while it has the focus, with the event as its argument; its `Value` is the key written as a shortcut, as `Ctrl+K`.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name `State`, the update commands and the handlers' events use for the table.

## Inputs

`System.Object`: the rows' objects, by value.

## Outputs

`PwrsGui.Widget`, its `Kind` `Table` and its `Properties` with `Item`, the objects, and `Selection`, and `Property` and `Filter` when they are given.

## Examples

Files a person selects two of, after double-clicking one. With `-HandlerOutput Pipeline` the handler's output comes before the result, and the selection comes back as the objects themselves:

<!-- person: $w = Wait-Window 'Files'; while ((Get-GuiSnapshot -Window $w.Id).State.Files.rows -lt 3) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Files -Event Activate -Value 2; Invoke-GuiTestEvent -Window $w.Id -Name Files -Event Select -Value 0, 2; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$rows = 1..3 | ForEach-Object { [pscustomobject]@{ Name = "report$_.txt"; Size = $_ * 10 } }
$out = Show-GuiWindow -Title 'Files' -HandlerOutput Pipeline {
    Table $rows -Selection Multiple -Name Files -OnActivate { "opened $($_.Value.Name)" }
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$out[0]
$out[1].State.Files | Format-Table -AutoSize
```

```text
opened report3.txt

Name        Size
----        ----
report1.txt   10
report3.txt   30
```

Rows added and taken away while the window is open, by the objects they were made from:

```powershell
$jobs = 1..3 | ForEach-Object { [pscustomobject]@{ Job = "job$_"; Minutes = $_ * 5 } }
$w = Show-GuiWindow -NoWait -Title 'Jobs' { Table $jobs -Name Jobs }
Add-GuiItem -Window $w.Id -Name Jobs -Item ([pscustomobject]@{ Job = 'job4'; Minutes = 20 })
Remove-GuiItem -Window $w.Id -Name Jobs -Item $jobs[0]
$deadline = [DateTime]::UtcNow.AddSeconds(20)
while ((Get-GuiSnapshot -Window $w.Id).State.Jobs.rows -ne 3 -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 50 }
(Get-GuiSnapshot -Window $w.Id).State.Jobs.rows
try { Remove-GuiItem -Window $w.Id -Name Jobs -Item $jobs[0] } catch { $_.FullyQualifiedErrorId }
Close-GuiWindow -Window $w.Id
```

```text
3
GuiNoRow,Pwrs.Modules.PWRS_GUI.RemoveGuiItemCommand
```

Columns named by `-Property`, in that order:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Columns' { Table ([pscustomobject]@{ Name = 'a'; Size = 1; Owner = 'me' }) -Property Size, Name -Name Files }
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.columns.label
Close-GuiWindow -Window $w.Id
```

```text
Size
Name
```

## Errors

| Error ID | When |
|---|---|
| `GuiOpeningOnly` | `Update-GuiWidget` sets `Item` or `Property`, which are taken when the window opens |
| `GuiNoRow` | `Remove-GuiItem` names an object no row was made from |
| `GuiTwoRows` | `Remove-GuiItem` names an object two or more rows were made from, as the same string twice |

Each is described in the [Error Reference](Error-Reference.md).
