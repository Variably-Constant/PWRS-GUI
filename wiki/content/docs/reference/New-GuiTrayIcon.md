---
title: New-GuiTrayIcon
weight: 57
---

Shows an icon in the system's tray, with a tooltip and a menu, and returns it once it shows. Source: `crates/pwrs-gui/src/tray.rs`, `NewGuiTrayIcon`, and the egui renderer's `crates/pwrs-gui-egui/src/tray.rs`.

The examples ran in order in PowerShell 7.6.9 on Windows, the tray icon pages in one process, so the icons' numbers count up from page to page. The clicks were sent by `Invoke-GuiTestEvent`, as a person's would be.

## Syntax

```powershell
Get-Command New-GuiTrayIcon -Syntax
```

```text
New-GuiTrayIcon [-Icon] <Object> [[-Tooltip] <string>] [-Menu <scriptblock>] [-OnClick <scriptblock>] [-OnDoubleClick <scriptblock>] [-HandlerOutput <HandlerOutput>] [<CommonParameters>]
```

## Description

The egui renderer shows the icon: in the notification area on Windows, in the menu bar on macOS, and on Linux and FreeBSD as a StatusNotifierItem, which KDE's panel shows, and GNOME with its AppIndicator extension. On a desktop with no tray of that kind, a warning says so, and the icon shows once one starts.

`-Menu` writes the menu's MenuItem, Menu and Separator widgets, as a window's `-Content` writes its widgets, and an item's `-OnClick` runs when the item is chosen. A left click on the icon runs `-OnClick`, or without it opens the menu, as most macOS and GNOME tray icons do. A right click opens the menu, and on Windows a double click runs `-OnDoubleClick`.

The icon's number is one of the numbers windows take. Every click and choice waits for [Wait-GuiEvent](Wait-GuiEvent.md), with the icon's number as `-Window`, whether or not a handler ran for it. The handlers run as engine-event actions, as those of a window `Show-GuiWindow -NoWait` opened do, and [Update-GuiWidget](Update-GuiWidget.md) changes a menu item's text or whether it is enabled. The icon stays until [Remove-GuiTrayIcon](Remove-GuiTrayIcon.md) takes it away or the module is removed from every runspace.

## Parameters

### -Icon

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The icon's picture: a path from the current location, or bytes, in any format [New-GuiImage](New-GuiImage.md) shows. A picture larger than 256 pixels on a side is shrunk to fit, and an SVG is drawn 64 pixels on its longer side.

### -Tooltip

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 1 |
| Pipeline input | no |
| Default | none |

The text shown while the pointer rests on the icon.

### -Menu

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: no menu |

The block that writes the menu: [MenuItem](New-GuiMenuItem.md), [Menu](New-GuiMenu.md) and [Separator](New-GuiSeparator.md) widgets, each short name standing for its `New-Gui<Widget>` command.

### -OnClick

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: a left click opens the menu |

Runs when the icon is clicked with the left button, with the event as its argument.

### -OnDoubleClick

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when the icon is double-clicked with the left button, on Windows, with the event as its argument.

### -HandlerOutput

| | |
|---|---|
| Type | `PwrsGui.HandlerOutput` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Discard` |

What happens to what the handlers write: `Discard` drops it, `Pipeline` writes it into the handler's event job, and `Console` sends it to `Out-Default`.

## Inputs

None.

## Outputs

`PwrsGui.TrayIcon`, once the icon shows:

| Property | Type | What it gives |
|---|---|---|
| `Id` | `System.UInt64` | the icon's number, which `Wait-GuiEvent` and `Update-GuiWidget` take as `-Window`, and the tray icon commands and `Invoke-GuiTestEvent` as `-TrayIcon` |
| `Tooltip` | `System.String` | the tooltip; null for none |
| `Names` | `System.String[]` | the names of the menu's items and menus, in menu order |
| `SourceIdentifier` | `System.String` | the engine event the handlers run on, which `Unregister-Event` removes |

## Examples

An icon with a menu:

```powershell
$tray = New-GuiTrayIcon -Icon .\backup.png -Tooltip 'Backups' -Menu { MenuItem 'Run now' -Name Run; Separator; MenuItem 'Quit' -Name Quit }
$tray | Format-List Id, Tooltip, Names
```

```text
Id      : 1
Tooltip : Backups
Names   : {Run, Quit}
```

The menu's choices wait for `Wait-GuiEvent`:

<!-- person: Invoke-GuiTestEvent -TrayIcon $tray.Id -Name Quit -Event Click -->
```powershell
$e = Wait-GuiEvent $tray.Id -Timeout 20
$e | Select-Object Name, Kind
Remove-GuiTrayIcon $tray.Id
```

```text
Name  Kind
----  ----
Quit Click
```

A left click runs `-OnClick`:

<!-- person: Invoke-GuiTestEvent -TrayIcon (Get-GuiTrayIcon).Id -Event Click -->
```powershell
$tray = New-GuiTrayIcon .\backup.png 'Backups' -OnClick { Write-Host 'Backups are running' }
Start-Sleep -Seconds 1
Remove-GuiTrayIcon $tray.Id
```

```text
Backups are running
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadIcon` | `-Icon` is neither a path nor bytes, or its picture is in no format an Image takes |
| `GuiNoFile` | `-Icon`'s path names no file, or several |
| `GuiNotAllowedHere` | `-Menu` wrote a widget other than a MenuItem, a Menu or a Separator |
| `GuiNameTwice` | two of the menu's items or menus have one name |
| `GuiTrayFailed` | the system's tray refused the icon |

Each is described in the [Error Reference](Error-Reference.md).
