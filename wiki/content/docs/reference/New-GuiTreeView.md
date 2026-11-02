---
title: New-GuiTreeView
weight: 50
---

Describes a tree of nodes that open to show the nodes inside them, given up front or found when a node opens. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiTreeView`; a tree's items while its window is open, `TreeItems` there and `Add-GuiItem` in `crates/pwrs-gui/src/window.rs`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The openings, selections and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiTreeView -Syntax
```

```text
New-GuiTreeView [[-Node] <Object[]>] [-Text <Object>] [-HasChildren <scriptblock>] [-Multiple] [-OnExpand <scriptblock>] [-OnSelect <scriptblock>] [-OnActivate <scriptblock>] [-OnDrop <scriptblock>] [-OnKey <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiTreeView` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a tree whose top nodes `-Node` and the pipeline give. In a `-Content` block `TreeView` stands for `New-GuiTreeView`. A node is made from what is given:

| Given | Node |
|---|---|
| a string | a leaf with that text |
| a hashtable | its `Text`, and its `Children` up front, in the same forms |
| any other object | its text from `-Text`, a property name or a script block run with the node as `$_`, or else its own text; when `-HasChildren` says so, it opens through `-OnExpand` |

`-OnExpand` runs when a person opens a node that `-HasChildren` says has children and whose children are still to come, with the node's object as the event's `Value`, and what it writes becomes the node's children, rather than output. A handler that writes nothing makes the node a leaf; one that fails has its error written as handlers' errors are, and the node closes again, so opening it once more runs `-OnExpand` again. `-HasChildren` and `-OnExpand` come together or not at all.

One node can be selected, or with `-Multiple` any number, with Ctrl and Shift. A change of selection raises `Select`, which `-OnSelect` acts on, and a double click or Enter raises `Activate`, which `-OnActivate` acts on. `State` and the events give the objects the nodes were made from, as they were given: a string's own text, one object, or with `-Multiple` a list in tree order. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) names nodes by their rows as shown, from 0: `Expand` opens a row's node, raising `Expand` only when its children are still to come, as a person's opening does.

[Add-GuiItem](Add-GuiItem.md) adds nodes, at the top or with `-Parent` under the node made from that object, and [Remove-GuiItem](Remove-GuiItem.md) removes a node and the nodes under it; a node is found by the same object, a string without regard to case, and two nodes that match are an error naming both. [Update-GuiWidget](Update-GuiWidget.md) sets `Multiple`; the nodes and how they give their text and children are taken when the window opens.

A node that opens has a chevron before its text, pointing right while the node is closed and down while it is open, and a node's children are a level in, their chevrons under its text. A node's text too long for the tree ends in an ellipsis, as a [ListBox](New-GuiListBox.md)'s item does. A window that fits its content opens wide enough for the rows the tree shows as it opens.

A click, the Tab key or [Set-GuiFocus](Set-GuiFocus.md) gives the tree the keyboard focus, as one widget. The row the keyboard is on scrolls into sight, is outlined while the window is the active one, and is the first row until a key or click moves it. Up, Down, Home, End, PageUp and PageDown move to a row and select it, as a click there would; Space selects the row, and Enter raises `Activate` when the row is selected. Right opens a node, or moves into one already open, and Left closes an open node, or moves to its parent. With `-Multiple`, Shift selects the rows from where the selection started, Ctrl moves without selecting, Space adds the row or takes it away, and Ctrl+A selects every row shown. `-OnKey` gets every key but the arrows, Home, End, the page keys, Space, Enter and Ctrl+A.

## Parameters

### -Node

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | all: position 0 |
| Pipeline input | yes, by value |
| Default | none: an empty tree |

The top nodes, strings, hashtables or other objects; piped nodes are added in the order piped.

### -Text

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | the object's own text |

How an object node gives its text: the name of one of its properties, or a script block run with the node as `$_`. A string gives itself, and a hashtable its `Text`.

### -HasChildren

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: object nodes are leaves |

A script block run with an object node as `$_` that says whether the node has children, which `-OnExpand` gives when a person opens it.

### -Multiple

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | one node |

Lets any number of nodes be selected, with Ctrl and Shift, rather than one.

### -OnExpand

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a person opens a node `-HasChildren` says has children, with the event as its argument, whose `Value` is the node's object; what it writes becomes the node's children, in the forms `-Node` takes.

### -OnSelect

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the selection changes, with the event as its argument and as `$_`; its `Value` is the object selected, or with `-Multiple` the objects.

### -OnActivate

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a node is double-clicked or chosen with Enter, with the event as its argument; its `Value` is the node's object.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the tree, with the event as its argument; its `Value` is the paths dropped, or the text.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key the tree does not use itself is pressed while it has the focus, with the event as its argument; its `Value` is the key written as a shortcut, as `Ctrl+K`.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name `State`, the update commands and the handlers' events use for the tree.

## Inputs

`System.Object`: top nodes, by value.

## Outputs

`PwrsGui.Widget`, its `Kind` `TreeView` and its `Properties` with `Node` and `Multiple`, and `Text` and `HasChildren` when they are given.

## Examples

A tree given up front, a node opened and a leaf double-clicked. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Food'; Invoke-GuiTestEvent -Window $w.Id -Name Food -Event Expand -Value 0; Invoke-GuiTestEvent -Window $w.Id -Name Food -Event Activate -Value 2; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Food' -HandlerOutput Pipeline {
    TreeView -Name Food -Node @{ Text = 'Fruit'; Children = 'Apple', 'Pear' }, @{ Text = 'Veg'; Children = @('Leek') } -OnActivate { "opened $($_.Value)" }
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$out[0]
$out[1].State.Food
```

```text
opened Pear
Pear
```

Children found when a node opens, from the object it was made from:

<!-- person: $w = Wait-Window 'Shares'; Invoke-GuiTestEvent -Window $w.Id -Name Shares -Event Expand -Value 0; while (@((Get-GuiSnapshot -Window $w.Id).State.Shares.shown).Count -lt 3) { Start-Sleep -Milliseconds 50 }; Invoke-GuiTestEvent -Window $w.Id -Name Shares -Event Select -Value 2; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$share = [pscustomobject]@{ Name = 'share'; Kids = 2 }
$result = Show-GuiWindow -Title 'Shares' {
    TreeView -Name Shares -Node $share -Text Name -HasChildren { $_.Kids -gt 0 } -OnExpand {
        1..$_.Value.Kids | ForEach-Object { [pscustomobject]@{ Name = "folder$_"; Kids = 0 } }
    }
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$result.State.Shares.Name
```

```text
folder2
```

`-HasChildren` and `-OnExpand` come together:

```powershell
try { New-GuiTreeView 'a' -HasChildren { $true } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadProperty,Pwrs.Modules.PWRS_GUI.NewGuiTreeViewCommand
-HasChildren says which nodes have children, which -OnExpand gives; give -OnExpand too
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadProperty` | `-HasChildren` is given without `-OnExpand`, or `-OnExpand` without `-HasChildren`, or `-Text` is neither a property name nor a script block |
| `GuiOpeningOnly` | `Update-GuiWidget` sets `Node`, `Text` or `HasChildren`, which are taken when the window opens |
| `GuiTwoNodes` | `Add-GuiItem -Parent` or `Remove-GuiItem` names an object two nodes match |

Each is described in the [Error Reference](Error-Reference.md).
