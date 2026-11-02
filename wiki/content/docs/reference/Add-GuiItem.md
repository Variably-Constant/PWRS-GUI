---
title: Add-GuiItem
weight: 15
---

Adds items to a widget that lists them: a list's items, a log's lines, a chart's points, a table's rows, a tree's nodes or a canvas's items. Source: `crates/pwrs-gui/src/window.rs`, `AddGuiItem`, `add_table_rows` and `add_canvas_items`; a chart's points, `chart_points` in `crates/pwrs-gui/src/widgets.rs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. Each example closes its window at its end.

## Syntax

```powershell
Get-Command Add-GuiItem -Syntax
```

```text
Add-GuiItem [-Name] <string> [-Item] <Object[]> [-Parent <Object>] [-Window <ulong>] [<CommonParameters>]
```

## Description

`Add-GuiItem` adds `-Item` after the items the widget `-Name` names already has, in a window [Show-GuiWindow](Show-GuiWindow.md) opened from a `-Content` block:

| Widget | An item is | Notes |
|---|---|---|
| [ListBox](New-GuiListBox.md), [ComboBox](New-GuiComboBox.md), [RadioGroup](New-GuiRadioGroup.md) | its text | what the widget has chosen stays chosen |
| [LogView](New-GuiLogView.md) | a line | only the new lines travel to the window, however long the log is |
| [Chart](New-GuiChart.md) | a hashtable of the `Series` it joins, its `Y`, and its `X` | a point joins the series of its name, or starts one of the chart's type; one without `X` takes its series' count of points; the first point a chart gets settles whether its `X` are numbers or dates, and on a chart of dates each point gives its date |
| [Table](New-GuiTable.md) | an object, a row | its cells come from the columns the table opened with, or from the first row's when it opened with neither an object nor `-Property`; only the new rows travel to the window |
| [TreeView](New-GuiTreeView.md) | a node, in the forms `New-GuiTreeView`'s `-Node` takes | under the node made from `-Parent`, or at the top without it |
| [Canvas](New-GuiCanvas.md) | a hashtable of an item's `Kind`, `Name` and keys, as `New-GuiCanvas`'s `-Item` takes it | drawn above the canvas's own items, or above the own items of the Group `-Parent` names; a name the canvas already has is refused, and so is any item it cannot take, none of the items being added |

A widget that lists nothing, such as a button, is refused. Inside a handler, `-Window` may be left out: the handler's window is the one it acts on. Anywhere else, `-Window` names the window.

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

The items to add, in order; an empty list adds none.

### -Parent

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the tree's top, or the canvas's own items |

For a TreeView, the node the items go under, named by the object it was made from: the same hashtable or object, text that matches without regard to case, or a number of the same value. For a Canvas, the name of the Group the items go into, compared without regard to case.

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

A list that grows while its choice stays, which a snapshot's `State` gives by its row number, from 0:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Servers' { ListBox 'web01', 'web02' -Name Servers -Selected web02 }
Add-GuiItem -Window $w.Id -Name Servers -Item 'web03', 'web04'
(Get-GuiSnapshot -Window $w.Id).State.Servers
Close-GuiWindow -Window $w.Id
```

```text
1
```

A log's lines and a chart's points:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Work' {
    LogView -Name Log -Height 80
    Chart -Name Load -Height 120 -Series @{ Name = 'cpu'; Y = 3, 5 }
}
Add-GuiItem -Window $w.Id -Name Log -Item 'started', 'copied 12 files'
Add-GuiItem -Window $w.Id -Name Load -Item @{ Series = 'cpu'; Y = 4 }, @{ Series = 'disk'; X = 0; Y = 1 }
$tree = (Get-GuiSnapshot -Window $w.Id).Tree
$tree.Children[0].Properties.lines
@($tree.Children[1].Properties.points).Count
Close-GuiWindow -Window $w.Id
```

```text
started
copied 12 files
2
```

A tree's nodes, under a node named by the object it was made from:

```powershell
$fruit = @{ Text = 'Fruit'; Children = 'Apple', 'Pear' }
$w = Show-GuiWindow -NoWait -Title 'Food' { TreeView -Name Food -Node $fruit }
Add-GuiItem -Window $w.Id -Name Food -Item 'Kiwi' -Parent $fruit
@((Get-GuiSnapshot -Window $w.Id).Tree.Children[0].Children).ForEach({ $_.Properties.text })
Close-GuiWindow -Window $w.Id
```

```text
Apple
Pear
Kiwi
```

A canvas's items, one into a Group named by `-Parent`, and a name the canvas already has refused:

```powershell
$w = Show-GuiWindow -NoWait -Title 'Board' {
    Canvas -Name Board -Width 200 -Height 100 @{ Kind = 'Group'; Name = 'Layer'; Children = @{ Kind = 'Rect'; Name = 'Box'; X = 0; Y = 0; Width = 40; Height = 40 } }
}
Add-GuiItem -Window $w.Id -Name Board -Item @{ Kind = 'Ellipse'; Name = 'Dot'; X = 50; Y = 0; Width = 20; Height = 20 } -Parent layer
Add-GuiItem -Window $w.Id -Name Board -Item @{ Kind = 'Line'; Name = 'Rule'; X1 = 0; Y1 = 90; X2 = 200; Y2 = 90 }
$items = (Get-GuiSnapshot -Window $w.Id).Tree.Properties.items
'{0}: {1}' -f $items[0].name, ($items[0].children.name -join ', ')
$items[1].name
try { Add-GuiItem -Window $w.Id -Name Board -Item @{ Kind = 'Rect'; Name = 'box'; X = 0; Y = 0; Width = 1; Height = 1 } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
Close-GuiWindow -Window $w.Id
```

```text
Layer: Box, Dot
Rule
GuiBadItem,Pwrs.Modules.PWRS_GUI.AddGuiItemCommand
Board: Rect box: the name box is given twice, and a name finds one item
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoWindow` | no window described in PowerShell has the number `-Window` gives, or `-Window` is left out outside a handler |
| `GuiNoWidget` | the window has no widget of that name |
| `GuiNoItems` | the widget lists no items |
| `GuiBadPoint` | a chart's point lacks its `Series` or `Y`, names a key a point does not take, or gives an `X` unlike the chart's |
| `GuiNoParent` | `-Parent` is given for a widget other than a TreeView or a Canvas |
| `GuiNoNode` | no node of the tree was made from `-Parent` |
| `GuiTwoNodes` | two nodes match `-Parent`; the message names both |
| `GuiNoItem` | the canvas has no item of the name `-Parent` gives |
| `GuiBadItem` | an item the canvas cannot take, a name it already has, or a `-Parent` naming an item that is no Group; the message names the canvas and the item |

Each is described in the [Error Reference](Error-Reference.md).
