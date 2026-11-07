---
title: Update-GuiTrayIcon
weight: 57
---

Changes a tray icon's picture, its tooltip or its menu. Source: `crates/pwrs-gui/src/tray.rs`, `UpdateGuiTrayIcon`.

The examples ran in order in PowerShell 7.6.9 on Windows, the tray icon pages in one process, so the icons' numbers count up from page to page.

## Syntax

```powershell
Get-Command Update-GuiTrayIcon -Syntax
```

```text
Update-GuiTrayIcon [-TrayIcon] <ulong> [-Icon <Object>] [-Tooltip <string>] [-Menu <scriptblock>] [<CommonParameters>]
```

## Description

Each part given replaces the icon's own, and the parts not given stay as they are. A menu given anew replaces the whole menu, and its items' `-OnClick` handlers replace the old items' handlers. [Update-GuiWidget](Update-GuiWidget.md), with the icon's number as `-Window`, changes one item's text or whether it is enabled. The handler of a click on the icon itself stays as [New-GuiTrayIcon](New-GuiTrayIcon.md) gave it.

## Parameters

### -TrayIcon

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | yes, by property name, as `TrayIcon` or `Id` |
| Default | none |

The tray icon's number, as `New-GuiTrayIcon` writes it.

### -Icon

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the picture stays |

The new picture: a path from the current location, or bytes, as `New-GuiTrayIcon -Icon` takes.

### -Tooltip

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the tooltip stays |

The new tooltip; an empty string shows none.

### -Menu

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the menu stays |

The block that writes the new menu, as `New-GuiTrayIcon -Menu` does.

## Inputs

An object with a `TrayIcon` or `Id` property, such as the tray icon `New-GuiTrayIcon` writes.

## Outputs

None.

## Examples

A new tooltip, then a new picture and menu from the pipeline:

```powershell
$tray = New-GuiTrayIcon .\backup.png 'Backups' -Menu { MenuItem 'Run now' -Name Run }
Update-GuiTrayIcon -TrayIcon $tray.Id -Tooltip 'Backups: running'
$tray | Update-GuiTrayIcon -Icon .\busy.png -Menu { MenuItem 'Stop' -Name Stop }
Get-GuiTrayIcon | Format-List Id, Tooltip, Names
Remove-GuiTrayIcon $tray.Id
```

```text
Id      : 8
Tooltip : Backups: running
Names   : {Stop}
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoTrayIcon` | no tray icon is shown with that number |
| `GuiBadIcon` | `-Icon` is neither a path nor bytes, or its picture is in no format an Image takes |
| `GuiNoFile` | `-Icon`'s path names no file, or several |
| `GuiNotAllowedHere` | `-Menu` wrote a widget other than a MenuItem, a Menu or a Separator |

Each is described in the [Error Reference](Error-Reference.md).
