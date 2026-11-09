---
title: Show-GuiNotification
weight: 58
---

Shows a notification and returns it once it shows, or with `-Wait` once it is answered. Source: `crates/pwrs-gui/src/notify.rs`, `ShowGuiNotification`, the egui renderer's `crates/pwrs-gui-egui/src/notify.rs`, and the toast document `crates/pwrs-gui-proto/src/notification.rs` writes.

The examples ran in order in PowerShell 7.6.9 on Windows, the notification pages in one process, so the notifications' numbers count up from page to page. The answers were sent by `Invoke-GuiTestEvent`, as a person's would be. Every toast was silent.

## Syntax

```powershell
Get-Command Show-GuiNotification -Syntax
```

```text
Show-GuiNotification [[-Title] <string>] [[-Body] <string>] [-Image <string>] [-HeroImage <string>] [-AppLogo <string>] [-Button <Object[]>] [-Input <Object[]>] [-Progress <Object>] [-Audio <Object>] [-Scenario <NotificationScenario>] [-Header <string>] [-App <string>] [-Quiet] [-Xml <string>] [-OnActivate <scriptblock>] [-OnClick <scriptblock>] [-OnDismiss <scriptblock>] [-Wait] [-HandlerOutput <HandlerOutput>] [<CommonParameters>]
```

## Description

The egui renderer shows the notification. On Windows it is a toast, and on Linux and FreeBSD it goes to the desktop's notification service, `org.freedesktop.Notifications`, over D-Bus. On macOS the notification center shows it under PWRS-GUI.app, which the renderer runs inside. A part the platform cannot show is named in a warning, and the rest shows; an input the platform has no box for is refused instead, since a script would wait on its answer.

On macOS, the first notification asks the person whether PWRS-GUI may show notifications, and the command waits for the answer with no time limit; Ctrl+C ends the wait. A refusal is `GuiNotificationDenied`, and macOS remembers the answer. With `-Quiet`, the command asks for provisional permission instead, which macOS gives without asking and which delivers quietly to Notification Center. macOS shows one text input, through the button whose `Input` names it, and no choice.

A click on a button raises `Click` under the button's name, and a click on the notification itself raises `Activate`. Each input's answer comes first, as a `Change` under its name, and every event's `State` holds the inputs' values. A dismissal raises `Dismiss`, its value `User`, `Timeout` or `Hidden`. A notification that timed out stays in the system's list and can still be answered. An answer, or a dismissal by the person, ends the notification, and `Close` comes last.

The notification's number is one of the numbers windows take. Its events wait for [Wait-GuiEvent](Wait-GuiEvent.md), with the number as `-Window`, and `-OnClick`, `-OnActivate` and `-OnDismiss` run as engine-event actions, as those of a window `Show-GuiWindow -NoWait` opened do. With `-Wait`, the command waits until the notification ends and writes the event that ended it. Handlers run once the command returns, as PowerShell runs engine-event actions between commands.

On Windows, a notification shows under an identity, an AppUserModelID registered for the user. Without `-App` it shows under PWRS-GUI's own, which the first notification registers under `HKCU\Software\Classes\AppUserModelId\PWRS-GUI`, with its icon copied to `%LOCALAPPDATA%\PWRS-GUI\Notifications\PWRS-GUI.png`. [Register-GuiNotificationApp](Register-GuiNotificationApp.md) registers an identity of your own, and [Unregister-GuiNotificationApp](Unregister-GuiNotificationApp.md) removes one.

## Parameters

### -Title

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | none |

The first line, in bold. A notification gives `-Title`, `-Body` or both, or `-Xml`.

### -Body

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 1 |
| Pipeline input | no |
| Default | none |

The text below the title.

### -Image, -HeroImage, -AppLogo

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

A picture's path, from the current location: `-Image` is shown in the notification, `-HeroImage` across its top on Windows, and `-AppLogo` in place of the app's icon. A path on a share is refused, since opening it would send your credentials to the share's host.

### -Button

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The buttons, at most five on Windows. Each is a text, which is its name as well, or a hashtable of `Text`, `Name`, `Input`, the name of the input it sends, and `OnClick`, which runs in place of `-OnClick`.

### -Input

| | |
|---|---|
| Type | `System.Object[]` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The inputs, at most five on Windows. Each is a name, which makes a text box with that placeholder, or a hashtable. A text box's hashtable takes `Name`, `Title`, `Placeholder` and `Text`, and a choice's takes `Name`, `Title`, `Choices` and `Selected`, a choice's text or its row from 0. On Linux and FreeBSD, one text box is shown where the server offers inline reply, as KDE's does.

### -Progress

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

A progress bar: a number from 0 to 1, or a hashtable of `Value`, `Title`, `Status` and `ValueText`. A null `Value` makes a bar that moves without saying how far. [Update-GuiNotification](Update-GuiNotification.md) moves it.

### -Audio

| | |
|---|---|
| Type | `System.Object` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none: the platform's own sound |

The sound: one of Windows' notification sounds, such as `Mail` or `Alarm2`, or a hashtable of `Sound`, `Silent` and `Loop`. Windows names `Default`, `IM`, `Mail`, `Reminder`, `SMS`, `Alarm` to `Alarm10` and `Call` to `Call10`.

### -Scenario

| | |
|---|---|
| Type | `PwrsGui.NotificationScenario` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | `Default` |

What the notification is for, which decides how long it stays: `Default` times out as any other does, `Reminder` stays on the screen until it is answered, `Alarm` and `IncomingCall` stay and repeat their sound, and `Urgent` breaks through Do Not Disturb where the person allows it.

### -Header

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The title of the group the notification is shown under in the system's list, on Windows.

### -App

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | PWRS-GUI's own identity |

The identity the notification is shown under: on Windows an AppUserModelID, such as one `Register-GuiNotificationApp` registers, and elsewhere the app's name.

### -Quiet

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Shows the notification without a banner: on Windows in the notification center alone, on Linux and FreeBSD at low urgency, and on macOS passively, with provisional permission when the person has not answered yet.

### -Xml

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

A toast's XML of your own, which Windows shows in place of every other part given. A click on it raises `Activate`, whose value holds the `Arguments` the click carried and the `Inputs`' answers, by their ids.

### -OnActivate, -OnClick, -OnDismiss

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Run with the event as their argument: `-OnActivate` when the notification itself is clicked, `-OnClick` when a button with no `OnClick` of its own is clicked, and `-OnDismiss` when it is dismissed.

### -Wait

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | off |

Waits until the notification ends, and writes the event that ended it: the button's `Click`, the `Activate` of a click on it, the `Dismiss` of a person's dismissal, or else its `Close`. Ctrl+C ends the wait.

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

`PwrsGui.Notification`, once the notification shows, or with `-Wait` the `PwrsGui.WidgetEvent` that ended it, as `Wait-GuiEvent` writes:

| Property | Type | What it gives |
|---|---|---|
| `Id` | `System.UInt64` | the notification's number, which `Wait-GuiEvent` takes as `-Window`, and the notification commands and `Invoke-GuiTestEvent` as `-Notification` |
| `Title` | `System.String` | the title; null for none |
| `Names` | `System.String[]` | the names of its inputs and buttons, inputs first |
| `SourceIdentifier` | `System.String` | the engine event the handlers run on, which `Unregister-Event` removes; null for a notification with no handlers |

## Examples

A title and a body:

```powershell
Show-GuiNotification 'Backup done' '12 files copied' -Audio @{ Silent = $true } | Format-List Id, Title, Names
```

```text
Id    : 16
Title : Backup done
Names : {}
```

Two buttons, waiting for the answer:

<!-- person: Invoke-GuiTestEvent -Notification (Get-GuiNotification | Where-Object Title -EQ 'Deploy?').Id -Name Deploy -Event Click -->
```powershell
Show-GuiNotification 'Deploy?' 'web01 is ready' -Button 'Deploy', 'Later' -Audio @{ Silent = $true } -Wait | Select-Object Name, Kind
```

```text
Name    Kind
----    ----
Deploy Click
```

A reply box with its send button, read with `Wait-GuiEvent`:

<!-- person: Invoke-GuiTestEvent -Notification $n.Id -Name Send -Event Click -Value @{ Reply = 'On my way' } -->
```powershell
$n = Show-GuiNotification 'New message' 'Ann: lunch?' -Input @{ Name = 'Reply'; Placeholder = 'Type a reply' } -Button @{ Text = 'Send'; Input = 'Reply' } -Audio @{ Silent = $true }
do {
    $e = Wait-GuiEvent $n.Id -Timeout 30
    if ($e) { '{0} {1} {2}' -f $e.Kind, $e.Name, $e.State.Reply }
} until (-not $e -or $e.Kind -eq 'Close')
```

```text
Change Reply On my way
Click Send On my way
Close  On my way
```

A progress bar, which [Update-GuiNotification](Update-GuiNotification.md) moves:

```powershell
$n = Show-GuiNotification 'Copying' -Progress @{ Value = 0; Status = 'Starting' } -Audio @{ Silent = $true }
$n.Title
```

```text
Copying
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadNotification` | a part the notification cannot take, such as no title, body or XML, a key a hashtable does not take, or a sound Windows does not name |
| `GuiNoFile` | a picture's path names no file, or several |
| `GuiNameTwice` | two inputs or buttons have one name |
| `GuiNotificationInput` | the platform has no box for an input |
| `GuiNotificationFailed` | the platform could not show the notification |
| `GuiNotificationDenied` | on macOS, the person did not allow PWRS-GUI's notifications |
| `GuiNotificationIdentity` | PWRS-GUI's own identity could not be registered |

Each is described in the [Error Reference](Error-Reference.md).
