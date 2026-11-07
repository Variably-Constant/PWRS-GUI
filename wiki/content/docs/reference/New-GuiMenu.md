---
title: New-GuiMenu
weight: 52
---

Describes a menu: its title, and the items, menus and rules its block writes. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiMenu`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiMenu -Syntax
```

```text
New-GuiMenu [-Title] <string> [[-Content] <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiMenu` writes a `PwrsGui.Widget` that contains the [MenuItem](New-GuiMenuItem.md), Menu and [Separator](New-GuiSeparator.md) widgets its `-Content` block writes, which [Show-GuiWindow](Show-GuiWindow.md) draws as `-Title` in a [MenuBar](New-GuiMenuBar.md), opening to show its items when a person clicks it. A Menu inside another is a submenu, and a Menu in a [ContextMenu](New-GuiContextMenu.md) opens from it. A Menu goes only in a MenuBar, a ContextMenu or another Menu, and contains only items, menus and rules; Show-GuiWindow refuses anything else. In a `-Content` block `Menu` stands for `New-GuiMenu`.

[Update-GuiWidget](Update-GuiWidget.md) changes the title.

## Parameters

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The menu's title.

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: position 1 |
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

`PwrsGui.Widget`, its `Kind` `Menu`, its `Properties` with `Title`, and its `Children` the items.

## Examples

A File menu with a submenu of recent files, one of them chosen. With `-HandlerOutput Pipeline` the handler's output comes before the result:

<!-- person: $w = Wait-Window 'Files'; Invoke-GuiTestEvent -Window $w.Id -Name Second -Event Click; Invoke-GuiTestEvent -Window $w.Id -Name Exit -Event Click -->
```powershell
$out = Show-GuiWindow -Title 'Files' -HandlerOutput Pipeline {
    Text 'No file open'
    MenuBar {
        Menu 'File' {
            Menu 'Recent' {
                MenuItem 'first.txt' -Name First -OnClick { "opening $($_.Name)" }
                MenuItem 'second.txt' -Name Second -OnClick { "opening $($_.Name)" }
            }
            Separator
            MenuItem 'Exit' -Name Exit -Result Exit
        }
    }
}
$out[0]
$out[1].Result
```

```text
opening Second
Exit
```

A menu goes in a bar, a context menu or another menu:

```powershell
try { Show-GuiWindow { Menu 'Loose' { MenuItem 'x' } } } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiNotInMenu,Pwrs.Modules.PWRS_GUI.ShowGuiWindowCommand
a Menu sits in a MenuBar, a ContextMenu or another Menu
```

## Errors

| Error ID | When |
|---|---|
| `GuiNotInMenu` | Show-GuiWindow is given a Menu outside a MenuBar, a ContextMenu or another Menu, or a Menu containing a widget other than an item, a menu or a rule |
| `GuiNotAWidget` | the block writes something other than a widget |

Each is described in the [Error Reference](Error-Reference.md).
