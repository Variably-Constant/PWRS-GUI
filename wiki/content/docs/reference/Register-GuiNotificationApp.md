---
title: Register-GuiNotificationApp
weight: 59
---

Registers a notification identity of your own for `Show-GuiNotification -App`, on Windows. Source: `crates/pwrs-gui/src/notify.rs`, `RegisterGuiNotificationApp` and its `identity` module.

The examples ran in PowerShell 7.6.9 on Windows.

## Syntax

```powershell
Get-Command Register-GuiNotificationApp -Syntax
```

```text
Register-GuiNotificationApp [-Id] <string> [-Name] <string> [-Icon <string>] [-WhatIf] [-Confirm] [<CommonParameters>]
```

## Description

Windows shows a toast under an identity, an AppUserModelID registered for the user, whose name and icon the toast shows. The command writes the identity's key, `HKCU\Software\Classes\AppUserModelId\<Id>`, with `DisplayName` set to `-Name` and `IconUri` set to the full path of `-Icon`. The icon stays where `-Icon` names it, so moving or deleting the file takes it from the identity's toasts. Registering an identity again replaces its name and icon. It stays until [Unregister-GuiNotificationApp](Unregister-GuiNotificationApp.md) removes it.

PWRS-GUI's own identity needs no registering: the first notification shown without `-App` registers it. On Linux and FreeBSD, notifications show under the app name `-App` gives, and this command fails with `GuiNotWindows`.

## Parameters

### -Id

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The AppUserModelID, conventionally `Company.Product`: 1 to 128 characters, with no spaces, backslashes or control characters.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 1, mandatory |
| Pipeline input | no |
| Default | none |

The name the identity's toasts show.

### -Icon

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: Windows' own icon |

The icon the identity's toasts show: a picture's path, from the current location, written as its full path.

### -WhatIf, -Confirm

Say what the command would register without registering it, or ask before registering.

## Inputs

None.

## Outputs

None.

## Examples

What registering an identity would do:

```powershell
Register-GuiNotificationApp -Id 'Contoso.Backup' -Name 'Contoso Backup' -Icon .\backup.png -WhatIf
```

```text
What if: Performing the operation "Register the notification identity" on target "Contoso.Backup".
```

Without `-WhatIf`, the identity's notifications then show under its name and icon, as `Show-GuiNotification 'Backup done' -App 'Contoso.Backup'` shows one.

## Errors

| Error ID | When |
|---|---|
| `GuiBadAppId` | `-Id` is empty, longer than 128 characters, or holds a space, a backslash or a control character |
| `GuiNoFile` | `-Icon` names no file, or several |
| `GuiNotWindows` | the command ran on a system other than Windows |

Each is described in the [Error Reference](Error-Reference.md).
