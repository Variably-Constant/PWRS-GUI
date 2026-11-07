---
title: Unregister-GuiNotificationApp
weight: 59
---

Removes a notification identity, on Windows, with everything Windows keeps of it. Source: `crates/pwrs-gui/src/notify.rs`, `UnregisterGuiNotificationApp` and its `identity` module, and `forget_app` in `crates/pwrs-gui-egui/src/notify.rs`.

The examples ran in PowerShell 7.6.9 on Windows.

## Syntax

```powershell
Get-Command Unregister-GuiNotificationApp -Syntax
```

```text
Unregister-GuiNotificationApp [-Id] <string> [-WhatIf] [-Confirm] [<CommonParameters>]
```

## Description

The identity's notifications leave the notification center first. Then the command removes its key under `HKCU\Software\Classes\AppUserModelId`, and its entry under `HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings`, which Windows makes once the identity has shown a toast and which holds the choices made for it in Settings. For PWRS-GUI's own identity, the icon copy in `%LOCALAPPDATA%\PWRS-GUI\Notifications` goes too, with its folder once empty; the next notification shown without `-App` registers it again.

The command removes the keys of whatever identity `-Id` names, so give it only an identity of your own. An identity with neither key is a non-terminating error.

## Parameters

### -Id

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The AppUserModelID, as [Register-GuiNotificationApp](Register-GuiNotificationApp.md) registered it, or `PWRS-GUI` for PWRS-GUI's own.

### -WhatIf, -Confirm

Say what the command would remove without removing it, or ask before removing.

## Inputs

None.

## Outputs

None.

## Examples

What removing an identity would do:

```powershell
Unregister-GuiNotificationApp -Id 'Contoso.Backup' -WhatIf
```

```text
What if: Performing the operation "Remove the notification identity and its notifications" on target "Contoso.Backup".
```

An identity that is not registered:

```powershell
Unregister-GuiNotificationApp -Id 'Contoso.Backup' -ErrorAction SilentlyContinue -ErrorVariable missing
$missing[0].FullyQualifiedErrorId
$missing[0].Exception.Message
```

```text
GuiNoNotificationApp,Pwrs.Modules.PWRS_GUI.UnregisterGuiNotificationAppCommand
no notification identity Contoso.Backup is registered for this user
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadAppId` | `-Id` is empty, longer than 128 characters, or holds a space, a backslash or a control character |
| `GuiNoNotificationApp` | no identity with that id is registered for the user; non-terminating |
| `GuiNotificationsKept` | the identity's notifications could not be taken from the system's list, so nothing was removed |
| `GuiNotificationIdentity` | PWRS-GUI's icon copy could not be removed |
| `GuiNotWindows` | the command ran on a system other than Windows |

Each is described in the [Error Reference](Error-Reference.md).
