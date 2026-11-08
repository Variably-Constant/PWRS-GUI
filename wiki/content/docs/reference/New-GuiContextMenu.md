---
title: New-GuiContextMenu
weight: 54
---

Describes a menu that opens where a person right-clicks a widget. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiContextMenu`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiContextMenu -Syntax
```

```text
New-GuiContextMenu [[-Content] <scriptblock>] -For <string> [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiContextMenu` writes a `PwrsGui.Widget` that contains the [MenuItem](New-GuiMenuItem.md), [Menu](New-GuiMenu.md) and [Separator](New-GuiSeparator.md) widgets its `-Content` block writes, which [Show-GuiWindow](Show-GuiWindow.md) opens where a person right-clicks the widget `-For` names: the smallest such widget, where several are under the pointer. A right click is the right button let go within 6 points and 0.8 seconds of its press, as egui counts a click, over no menu or drop-down open. A right drag, a longer right press, the Menu key and Shift+F10 open no menu, and an HTML window shows no menu of its engine's own. It may go anywhere in the block, and takes no place among the widgets. It is drawn only while it is open, so [Get-GuiSnapshot](Get-GuiSnapshot.md) `-Name -Pixels` refuses its pixels otherwise. In a `-Content` block `ContextMenu` stands for `New-GuiContextMenu`.

Its items raise their clicks as a menu's do, with the window's `State` in the event, so an item acts on the widget's current value or selection. [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) clicks an item by its name without the menu being open.

## Parameters

### -For

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named, mandatory |
| Pipeline input | no |
| Default | none |

The name of the widget the menu belongs to.

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: an empty menu |

The block that writes the items, in order.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands use for the menu.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `ContextMenu`, its `Properties` with `For`, and its `Children` the items.

## Examples

A list's menu, whose item acts on the item selected. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Servers'; Invoke-GuiTestEvent -Window $w.Id -Name CopyServer -Event Click; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Servers' -HandlerOutput Pipeline {
    ListBox 'web01', 'web02' -Name Servers -Selected 'web02'
    ContextMenu -For Servers -Name ServerMenu { MenuItem 'Copy' -Name CopyServer -OnClick { "copying $($_.State.Servers)" } }
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$out[0]
$out[1].Result
```

```text
copying web02
Ok
```

A menu for a widget the window does not have opens no window:

```powershell
try { Show-GuiWindow { Text 'x' -Name Here; ContextMenu -For Elsewhere { MenuItem 'x' } } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiNoWidget,Pwrs.Modules.PWRS_GUI.ShowGuiWindowCommand
a ContextMenu is for the widget named Elsewhere, and the window has none; its widgets are Here
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoWidget` | `-For` names no widget of the window |
| `GuiNotInMenu` | the menu contains a widget other than an item, a menu or a rule |
| `GuiNotAWidget` | the block writes something other than a widget |

Each is described in the [Error Reference](Error-Reference.md).
