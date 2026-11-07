---
title: Remove-GuiItem
weight: 16
---

Removes items from a widget that lists them: a list's items, a table's rows, a tree's nodes or a canvas's items. Source: `crates/pwrs-gui/src/window.rs`, `RemoveGuiItem`, `remove_table_rows` and `remove_canvas_items`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command Remove-GuiItem -Syntax
```

```text
Remove-GuiItem [-Name] <string> [-Item] <Object[]> [-Window <ulong>] [<CommonParameters>]
```

## Description

`Remove-GuiItem` takes `-Item` out of the widget `-Name` names, in a window [Show-GuiWindow](Show-GuiWindow.md) opened from a `-Content` block. A [ListBox](New-GuiListBox.md), [ComboBox](New-GuiComboBox.md) or [RadioGroup](New-GuiRadioGroup.md) item is named by its text, without regard to case, and what the widget has chosen stays chosen among the items that stay. A [Table](New-GuiTable.md) row and a [TreeView](New-GuiTreeView.md) node are named by the object they were made from: the same hashtable or object, text that matches without regard to case, or a number of the same value. A table's selection stays on those of its selected rows that remain, and a node goes with the nodes under it. A [Canvas](New-GuiCanvas.md)'s item is named by its name, at any depth, without regard to case, and a Group goes with its items and their animations.

A log's lines and a chart's points are not taken away one by one: [Update-GuiWidget](Update-GuiWidget.md) gives a log new `Line` and a chart new `Series`. An item the widget does not have is an error naming it, and nothing is removed.

Inside a handler, `-Window` may be left out: the handler's window is the one it acts on. Anywhere else, `-Window` names the window.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The widget's name, compared without regard to case.

### -Item

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | all: position 1, mandatory |
| Pipeline input | no |
| Default | none |

The items to remove.

### -Window

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: named |
| Pipeline input | yes, by property name, as `Window` or `Id` |
| Default | inside a handler, the handler's window |

The window's number.

## Inputs

An object with a `Window` or `Id` property, such as the window `Show-GuiWindow -NoWait` writes.

## Outputs

None.

## Examples

A list that loses an item while its choice stays, which a snapshot's `State` gives by its row number, from 0, here web03's:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Servers' { ListBox 'web01', 'web02', 'web03' -Name Servers -Selected web03 }
Remove-GuiItem -Window $w.Id -Name Servers -Item 'WEB01'
(Get-GuiSnapshot -Window $w.Id).Tree.Properties.items
(Get-GuiSnapshot -Window $w.Id).State.Servers
Close-GuiWindow -Window $w.Id
```

```text
web02
web03
1
```

An item the list does not have names it:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Missing' { ListBox 'a', 'b' -Name Pick }
try { Remove-GuiItem -Window $w.Id -Name Pick -Item 'c' } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
Close-GuiWindow -Window $w.Id
```

```text
GuiNoItem,Pwrs.Modules.PWRS_GUI.RemoveGuiItemCommand
Pick has no item c; its items are a, b
```

A canvas's Group taken away with its items, and a name no item has:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Board' {
    Canvas -Name Board -Width 200 -Height 100 @(
        @{ Kind = 'Group'; Name = 'Layer'; Children = @{ Kind = 'Rect'; Name = 'Box'; X = 0; Y = 0; Width = 40; Height = 40 } }
        @{ Kind = 'Ellipse'; Name = 'Dot'; X = 100; Y = 0; Width = 20; Height = 20 }
    )
}
Remove-GuiItem -Window $w.Id -Name Board -Item LAYER
@((Get-GuiSnapshot -Window $w.Id).Tree.Properties.items).name
try { Remove-GuiItem -Window $w.Id -Name Board -Item Box } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
Close-GuiWindow -Window $w.Id
```

```text
Dot
GuiNoItem,Pwrs.Modules.PWRS_GUI.RemoveGuiItemCommand
Board: no item is named Box
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoWindow` | no window described in PowerShell has the number `-Window` gives, or `-Window` is left out outside a handler |
| `GuiNoWidget` | the window has no widget of that name |
| `GuiNoItems` | the widget lists no items, or is a log or a chart, whose lines and points are not removed one by one |
| `GuiNoItem` | the widget does not have an item named, or a canvas no item of a name given; nothing is removed |
| `GuiNoNode` | no node of the tree was made from an item named |
| `GuiTwoNodes` | two nodes match an item; the message names both |
| `GuiNoRow` | no row of the table was made from an item named |
| `GuiTwoRows` | two or more rows match an item; the message names them |

Each is described in the [Error Reference](Error-Reference.md).
