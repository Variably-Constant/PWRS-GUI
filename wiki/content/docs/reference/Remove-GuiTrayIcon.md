---
title: Remove-GuiTrayIcon
weight: 57
---

Takes a tray icon away, and returns once it has gone. Source: `crates/pwrs-gui/src/tray.rs`, `RemoveGuiTrayIcon`.

The examples ran in order in PowerShell 7.6.9 on Windows, the tray icon pages in one process, so the icons' numbers count up from page to page.

## Syntax

```powershell
Get-Command Remove-GuiTrayIcon -Syntax
```

```text
Remove-GuiTrayIcon [-TrayIcon] <ulong> [<CommonParameters>]
```

## Description

The icon leaves the system's tray, and its last event, `Close`, waits for [Wait-GuiEvent](Wait-GuiEvent.md), as a window's does. A number no tray icon shown has is a non-terminating error, so a list piped in goes on past it.

## Parameters

### -TrayIcon

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | yes, by property name, as `TrayIcon` or `Id` |
| Default | none |

The tray icon's number, as [New-GuiTrayIcon](New-GuiTrayIcon.md) writes it.

## Inputs

An object with a `TrayIcon` or `Id` property, such as the tray icons [Get-GuiTrayIcon](Get-GuiTrayIcon.md) writes.

## Outputs

None.

## Examples

The icon's `Close` comes once it has gone:

```powershell
$tray = New-GuiTrayIcon .\backup.png 'Backups'
Remove-GuiTrayIcon -TrayIcon $tray.Id
(Wait-GuiEvent $tray.Id -Timeout 5).Kind
```

```text
Close
```

A number no tray icon has:

```powershell
Remove-GuiTrayIcon -TrayIcon 999 -ErrorAction SilentlyContinue -ErrorVariable gone
$gone[0].FullyQualifiedErrorId
$gone[0].Exception.Message
```

```text
GuiNoTrayIcon,Pwrs.Modules.PWRS_GUI.RemoveGuiTrayIconCommand
no tray icon is shown with the number 999
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoTrayIcon` | no tray icon is shown with that number; non-terminating |

Each is described in the [Error Reference](Error-Reference.md).
