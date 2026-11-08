---
title: Remove-GuiNotification
weight: 58
---

Takes a notification away, from the screen and from the system's list, and returns once it has gone. Source: `crates/pwrs-gui/src/notify.rs`, `RemoveGuiNotification`.

The examples ran in order in PowerShell 7.6.9 on Windows, the notification pages in one process, so the notifications' numbers count up from page to page. Every toast was silent.

## Syntax

```powershell
Get-Command Remove-GuiNotification -Syntax
```

```text
Remove-GuiNotification [-Notification] <ulong> [<CommonParameters>]
```

## Description

The notification leaves the screen and, on Windows, the notification center, and its last event, `Close`, waits for [Wait-GuiEvent](Wait-GuiEvent.md). A number no notification shown has is a non-terminating error, so a list piped in goes on past it.

## Parameters

### -Notification

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | yes, by property name, as `Notification` or `Id` |
| Default | none |

The notification's number, as [Show-GuiNotification](Show-GuiNotification.md) writes it.

## Inputs

An object with a `Notification` or `Id` property, such as the notifications [Get-GuiNotification](Get-GuiNotification.md) writes.

## Outputs

None.

## Examples

The notification's `Close` comes once it has gone:

```powershell
$n = Show-GuiNotification 'Reminder' 'Stand-up in 5 minutes' -Audio @{ Silent = $true }
Remove-GuiNotification -Notification $n.Id
(Wait-GuiEvent $n.Id -Timeout 5).Kind
```

```text
Close
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoNotification` | no notification is shown with that number; non-terminating |

Each is described in the [Error Reference](Error-Reference.md).
