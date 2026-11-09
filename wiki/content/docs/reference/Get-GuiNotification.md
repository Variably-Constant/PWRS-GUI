---
title: Get-GuiNotification
weight: 58
---

Gets the notifications the egui renderer shows, in any runspace of the process. Source: `crates/pwrs-gui/src/notify.rs`, `GetGuiNotification`.

The examples ran in order in PowerShell 7.6.9 on Windows, the notification pages in one process, so the notifications' numbers count up from page to page. Every toast was silent.

## Syntax

```powershell
Get-Command Get-GuiNotification -Syntax
```

```text
Get-GuiNotification [<CommonParameters>]
```

## Description

Writes each notification the renderer shows and has not yet seen end, in the order of their numbers, a timed-out one among them. It writes nothing when the renderer shows none or is not running. A notification is not a window, so [Get-GuiWindow](Get-GuiWindow.md) leaves it out.

## Parameters

None but the common parameters.

## Inputs

None.

## Outputs

`PwrsGui.Notification`, one for each notification shown, as [Show-GuiNotification](Show-GuiNotification.md) describes, with `SourceIdentifier` null.

## Examples

List two notifications, then take every one away:

```powershell
$null = Show-GuiNotification 'Build 1 passed' -Audio @{ Silent = $true }
$null = Show-GuiNotification 'Build 2 failed' -Audio @{ Silent = $true }
Get-GuiNotification | Format-Table Id, Title
Get-GuiNotification | Remove-GuiNotification
@(Get-GuiNotification).Count
```

```text
Id Title
-- -----
29 Build 1 passed
30 Build 2 failed

0
```

## Errors

None of its own; the session's errors in the [Error Reference](Error-Reference.md) apply.
