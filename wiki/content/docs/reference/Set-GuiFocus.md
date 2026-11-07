---
title: Set-GuiFocus
weight: 16
---

Gives a widget of an open window described in PowerShell the keyboard focus, found by its name. Source: `crates/pwrs-gui/src/window.rs`, `SetGuiFocus`; the renderers' side, `focus_node` in `crates/pwrs-gui-egui/src/lib.rs` and `focus` in `crates/pwrs-gui-web/src/runtime/runtime.js`.

The examples ran in order, in PowerShell 7.6.9 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The key presses and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be.

## Syntax

```powershell
Get-Command Set-GuiFocus -Syntax
```

```text
Set-GuiFocus [-Name] <string> [-Window <ulong>] [<CommonParameters>]
```

## Description

`Set-GuiFocus` gives the widget `-Name` names the keyboard focus, in a window [Show-GuiWindow](Show-GuiWindow.md) opened from a `-Content` block, as a person's click or Tab gives it. A text field then takes the keys a person types, a button the Space and Enter that press it, and a list the arrow keys that move its selection. It is how a shortcut such as Ctrl+F takes a person to a field, and how a window opens with the cursor in the field to fill first.

The widgets that take the focus are the inputs, the buttons, a [ListBox](New-GuiListBox.md), [Table](New-GuiTable.md) and [TreeView](New-GuiTreeView.md), a [Text](New-GuiText.md) with `-Style Link`, a [GpuSurface](New-GuiGpuSurface.md) with `-OnKey`, and a [Canvas](New-GuiCanvas.md) while it takes keys, for its `-OnKey` or for a view whose controls take keys. A Table's focus goes to its rows, not the field `-Filter` puts above them, a RadioGroup's to the option chosen, or the first while none is, and a ListBox's and a TreeView's to the list itself, where the arrows move through the rows. [Get-GuiSnapshot](Get-GuiSnapshot.md)'s `Focused` names the widget with the focus. A focused field draws its cursor only while its window has the system's keyboard focus, so a window behind another shows the focus once it comes to the front; the pictures below are of windows opened without it.

A widget whose kind takes no focus, such as a Text that is not a link, is a `GuiNotFocusable` error, which ends the command. The renderer refuses a widget that is disabled, one on a tab that is not shown, and a Canvas that takes no keys: each is a non-terminating `GuiRefused` error, so a script goes on past it. The command returns once the widget has the focus, so a snapshot taken after it names the widget; a window `Show-GuiWindow -NoWait` has just opened gives the focus as its first frame draws.

Inside a handler, `-Window` may be left out: the handler's window is the one it acts on, whose number is in `$PwrsGuiWindow`. Anywhere else, `-Window` names the window, by the number `Show-GuiWindow -NoWait` and [Get-GuiWindow](Get-GuiWindow.md) give; a window object piped in gives its `Id`.

## Parameters

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0, mandatory |
| Pipeline input | no |
| Default | none |

The widget's name, compared without regard to case.

### -Window

| | |
|---|---|
| Type | `System.UInt64` |
| Parameter sets | all: named |
| Pipeline input | yes, by property name, as `Window` or `Id` |
| Default | inside a handler, the handler's window |

The window's number.

## Inputs

An object with a `Window` or `Id` property, such as the window `Show-GuiWindow -NoWait` writes.

## Outputs

None.

## Examples

A shortcut that takes a person to the field, from a handler, which acts on its own window. Press Ctrl+F:

<!-- focus: its pictures show the field Set-GuiFocus gives the keyboard -->
<!-- keep: set-guifocus-shortcut.png images/set-guifocus-shortcut.png -->
<!-- person: $w = Wait-Window 'Find'; Invoke-GuiTestEvent -Window $w.Id -Shortcut 'Ctrl+F'; Wait-Focused $w Query; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'set-guifocus-shortcut.png'); Invoke-GuiTestEvent -Window $w.Id -Name Close -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Find' -OnShortcut @{ 'Ctrl+F' = { Set-GuiFocus Query } } {
    TextBox -Name Query -Placeholder 'text to find'
    ListBox 'web01', 'web02', 'db01' -Name Hosts
    ButtonBar { Button 'Close' -Name Close -Cancel -Result Close }
}
$result.Result
```

```text
Close
```

![The Find window: a field for the text to find above a list of web01, web02 and db01, and a Close button at the bottom right](/images/set-guifocus-shortcut.png)

A window that opens with the focus in its second field, given from outside the window, which [Get-GuiSnapshot](Get-GuiSnapshot.md) then names:

<!-- keep: set-guifocus-password.png images/set-guifocus-password.png -->
<!-- person: $w = Wait-Window 'Sign in'; Wait-Focused $w Password; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'set-guifocus-password.png'); Close-GuiWindow -Window $w.Id -->
```powershell
$w = Show-GuiWindow -NoWait -Title 'Sign in' {
    TextBox -Name User -Placeholder 'user name'
    TextBox -Name Password -Password
    ButtonBar { Button 'Sign in' -Name SignIn -Default -Result Ok }
}
Set-GuiFocus -Window $w.Id -Name Password
(Get-GuiSnapshot -Window $w.Id).Focused
```

```text
Password
```

![The Sign in window: a field for the user name, the password field under it, and a Sign in button at the bottom right](/images/set-guifocus-password.png)

A Text that is not a link takes no focus, which ends the command; a disabled button is refused by the renderer, which does not:

<!-- varies: the window's number -->
```powershell
$w = Show-GuiWindow -NoWait -Title 'Status' {
    Text 'Ready' -Name Status
    ButtonBar { Button 'OK' -Name Ok -Enabled:$false -Result Ok }
}
try { Set-GuiFocus -Window $w.Id -Name Status } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
Set-GuiFocus -Window $w.Id -Name Ok -ErrorVariable refused -ErrorAction SilentlyContinue
$refused[0].FullyQualifiedErrorId
$refused[0].Exception.Message
Close-GuiWindow -Window $w.Id
```

```text
GuiNotFocusable,Pwrs.Modules.PWRS_GUI.SetGuiFocusCommand
Status in window 20 is a Text, which takes the focus only with -Style Link
GuiRefused,Pwrs.Modules.PWRS_GUI.SetGuiFocusCommand
Ok in window 20 cannot take the focus: the widget is disabled
```

## Errors

| Error ID | When |
|---|---|
| `GuiNoWindow` | `-Window` is left out outside a handler, or no window described in PowerShell is open with the number it gives |
| `GuiNoWidget` | the window has no widget named `-Name`; the message names the widgets it has |
| `GuiNotFocusable` | the widget's kind takes no focus: a Text without `-Style Link`, a GpuSurface without `-OnKey`, or a widget that is not an input, a button, a list, a table or a tree |
| `GuiRefused` | the renderer refused the focus: the widget is disabled, it is on a tab that is not shown, or it is a Canvas that takes no keys; written as a non-terminating error |

Described in the [Error Reference](Error-Reference.md).
