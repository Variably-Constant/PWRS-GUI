---
title: Get-GuiTrayIcon
weight: 57
---

Gets the tray icons the egui renderer shows, in any runspace of the process. Source: `crates/pwrs-gui/src/tray.rs`, `GetGuiTrayIcon`.

The examples ran in order in PowerShell 7.6.9 on Windows, the tray icon pages in one process, so the icons' numbers count up from page to page.

## Syntax

```powershell
Get-Command Get-GuiTrayIcon -Syntax
```

```text
Get-GuiTrayIcon [<CommonParameters>]
```

## Description

Writes each tray icon the renderer shows, in the order of their numbers. It writes nothing when the renderer shows none or is not running. A tray icon is not a window, so [Get-GuiWindow](Get-GuiWindow.md) leaves it out.

## Parameters

None but the common parameters.

## Inputs

None.

## Outputs

`PwrsGui.TrayIcon`, one for each tray icon shown, as [New-GuiTrayIcon](New-GuiTrayIcon.md) describes, with `SourceIdentifier` null.

## Examples

List two icons, then take every icon away:

```powershell
$null = New-GuiTrayIcon .\backup.png 'Backups'
$null = New-GuiTrayIcon .\busy.png 'Sync'
Get-GuiTrayIcon | Format-Table Id, Tooltip
Get-GuiTrayIcon | Remove-GuiTrayIcon
@(Get-GuiTrayIcon).Count
```

```text
Id Tooltip
-- -------
11 Backups
12 Sync

0
```

## Errors

None of its own; the session's errors in the [Error Reference](Error-Reference.md) apply.
