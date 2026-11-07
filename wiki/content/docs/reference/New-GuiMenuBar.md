---
title: New-GuiMenuBar
weight: 51
---

Describes a bar of menus along the top of the window. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiMenuBar`.

The examples ran in order, in PowerShell 7.6.6 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The clicks and keys were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiMenuBar -Syntax
```

```text
New-GuiMenuBar [[-Content] <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiMenuBar` writes a `PwrsGui.Widget` that contains the [Menu](New-GuiMenu.md) widgets its `-Content` block writes, which [Show-GuiWindow](Show-GuiWindow.md) draws as a bar of their titles along the window's top, with the window's other widgets below it, wherever in the block the bar is written. It goes among the window's own widgets, at the block's first level, and contains menus alone; Show-GuiWindow refuses anything else. In a `-Content` block `MenuBar` stands for `New-GuiMenuBar`; a [StatusBar](New-GuiStatusBar.md) is its counterpart along the bottom.

The bar raises no event of its own: its items' clicks and shortcuts are the [MenuItem](New-GuiMenuItem.md)s'.

## Parameters

### -Content

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none: an empty bar |

The block that writes the menus, in order.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name the update commands use for the bar.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `MenuBar` and its `Children` the menus.

## Examples

A bar with a File menu, its Exit chosen:

<!-- person: $w = Wait-Window 'Report'; Invoke-GuiTestEvent -Window $w.Id -Name Exit -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Report' {
    Text 'The report'
    MenuBar { Menu 'File' { MenuItem 'Exit' -Name Exit -Result Exit } }
}
$result.Result
```

```text
Exit
```

A bar inside another widget opens no window, and a bar contains menus alone:

```powershell
try { Show-GuiWindow { Row { MenuBar { Menu 'File' { MenuItem 'x' } } } } } catch { $_.FullyQualifiedErrorId }
try { Show-GuiWindow { Text 'x'; MenuBar { MenuItem 'Loose' } } } catch { $_.FullyQualifiedErrorId }
```

```text
GuiNotAtTop,Pwrs.Modules.PWRS_GUI.ShowGuiWindowCommand
GuiNotInMenu,Pwrs.Modules.PWRS_GUI.ShowGuiWindowCommand
```

## Errors

| Error ID | When |
|---|---|
| `GuiNotAtTop` | Show-GuiWindow is given a MenuBar inside another widget |
| `GuiNotInMenu` | the bar contains a widget other than a Menu |
| `GuiNotAWidget` | the block writes something other than a widget |

Each is described in the [Error Reference](Error-Reference.md).
