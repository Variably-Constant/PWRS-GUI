---
title: Update-GuiNotification
weight: 58
---

Changes a notification's title, its body or its progress bar where it shows. Source: `crates/pwrs-gui/src/notify.rs`, `UpdateGuiNotification`.

The examples ran in order in PowerShell 7.6.9 on Windows, the notification pages in one process, so the notifications' numbers count up from page to page. Every toast was silent.

## Syntax

```powershell
Get-Command Update-GuiNotification -Syntax
```

```text
Update-GuiNotification [-Notification] <ulong> [-Title <string>] [-Body <string>] [-Progress <Object>] [<CommonParameters>]
```

## Description

On Windows the toast changes in place, through the data its title, body and progress bar are bound to, so it does not pop up again. On Linux and FreeBSD the notification service replaces the notification with one of the new parts. A notification of `-Xml` keeps the parts its XML gives. The command does not wait for the renderer, so a change the renderer refuses, as there, is written to the process's standard error.

## Parameters

### -Notification

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | yes, by property name, as `Notification` or `Id` |
| Default | none |

The notification's number, as [Show-GuiNotification](Show-GuiNotification.md) writes it.

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the title stays |

The new title.

### -Body

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the body stays |

The new body.

### -Progress

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the progress bar stays |

The progress bar now: a number from 0 to 1, or a hashtable of `Value`, `Title`, `Status` and `ValueText`, as `Show-GuiNotification -Progress` takes.

## Inputs

An object with a `Notification` or `Id` property, such as the notification `Show-GuiNotification` writes.

## Outputs

None.

## Examples

Move a progress bar, then finish it under a new title, with the notification from [Show-GuiNotification](Show-GuiNotification.md)'s last example:

```powershell
Update-GuiNotification -Notification $n.Id -Progress @{ Value = 0.5; Status = 'Copying' }
Start-Sleep -Milliseconds 500
$p = (Get-GuiSnapshot -Window $n.Id).Tree.Properties.progress
'{0} {1}' -f $p.value, $p.status
$n | Update-GuiNotification -Title 'Copied' -Progress 1
Start-Sleep -Milliseconds 500
(Get-GuiSnapshot -Window $n.Id).Tree.Properties.title
Remove-GuiNotification $n.Id
```

```text
0.5 Copying
Copied
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoNotification` | no notification is shown with that number |
| `GuiBadNotification` | none of `-Title`, `-Body` and `-Progress` was given, or the progress cannot be read |

Each is described in the [Error Reference](Error-Reference.md).
