---
title: New-GuiTextBox
weight: 40
---

Describes a text field of one line or several, or a password field. Source: `crates/pwrs-gui/src/widgets.rs`, `NewGuiTextBox`.

The examples ran in order, in PowerShell 7.6.7 and Windows PowerShell 5.1, each page in a process of its own. The output shown is PowerShell 7.6's; Windows PowerShell 5.1 printed the same apart from column widths, except where its output is shown too. The typing and clicks were sent by `Invoke-GuiTestEvent`, as a person's would be, and each example closes its window at its end.

## Syntax

```powershell
Get-Command New-GuiTextBox -Syntax
```

```text
New-GuiTextBox [[-Text] <string>] [-Multiline] [-Password] [-Placeholder <string>] [-Enabled] [-OnChange <scriptblock>] [-OnSubmit <scriptblock>] [-OnDrop <scriptblock>] [-OnKey <scriptblock>] [-Name <string>] [<CommonParameters>]
```

## Description

`New-GuiTextBox` writes a `PwrsGui.Widget` that [Show-GuiWindow](Show-GuiWindow.md) draws as a field with `-Text` in it, one line, or several with `-Multiline`, showing `-Placeholder` while it is empty. In a `-Content` block `TextBox` stands for `New-GuiTextBox`. Each edit raises `Change` with the text, which `-OnChange` acts on, and `State` has the text under the widget's name. In a field of one line, Enter raises `Submit` with the text, which `-OnSubmit` acts on; in one of several lines Enter starts a line, so `-OnSubmit` is refused there.

With `-Password` the characters are hidden, and `State` and the handlers' events give the text as a `SecureString`, as `Read-GuiInput` gives a password. A password field is one line, and whether a field is one is set when the window opens. Once a password field has the focus and a character is typed into it, it shows an eye at its right end, as Windows' password boxes do: the password shows while the eye is held down, or while Alt+F8 is. A desktop whose window manager takes Alt+F8 itself, as GNOME's does to resize a window, keeps that key from the field, and the eye is held with the pointer there. When another widget of the window takes the focus, the eye goes, and it comes back only once the field is cleared and typed into again, so a password left in a field cannot be shown by whoever comes to it later. Switching to another program and back keeps it. Text a script gives the field, with `-Text` or `Update-GuiWidget`, holds the eye back the same way until the field is cleared. A field that has the focus uses the keys it edits with itself, and a password field Alt+F8 too, and the window's shortcuts leave them to it; `-OnKey` gets the others. [Update-GuiWidget](Update-GuiWidget.md) sets `Text`, `Multiline`, `Placeholder` and `Enabled`, and leaves what a person typed as it is when it sets the others.

## Parameters

### -Text

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: position 0 |
| Pipeline input | no |
| Default | empty |

The text when the window opens.

### -Multiline

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | one line |

Takes several lines.

### -Password

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | a plain field |

Hides the characters typed; `State` and the handlers' events give the text as a `SecureString`. Once the field has the focus and is typed into, it shows an eye at its right end: the password shows while the eye is held, or Alt+F8 is. Once the focus leaves the field the eye is not offered again until the field is cleared. Taken when the window opens.

### -Placeholder

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Text shown while the field is empty.

### -Enabled

| | |
|---|---|
| Type | `System.Management.Automation.SwitchParameter` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | enabled |

Whether the text can be changed; `-Enabled:$false` makes it a field that cannot, and [Invoke-GuiTestEvent](Invoke-GuiTestEvent.md) refuses to type in it.

### -OnChange

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs after each edit, with the event as its argument and as `$_`; its `Value` is the text.

### -OnSubmit

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

In a field of one line, runs when Enter is pressed, with the event as its argument; its `Value` is the text.

### -OnDrop

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when files or text are dropped on the field, with the event as its argument; its `Value` is the paths dropped, or the text.

### -OnKey

| | |
|---|---|
| Type | `System.Management.Automation.ScriptBlock` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

Runs when a key the field does not edit with is pressed while it has the focus, with the event as its argument; its `Value` is the key written as a shortcut, as `Ctrl+K`.

### -Name

| | |
|---|---|
| Type | `System.String` |
| Parameter sets | all: named |
| Pipeline input | no |
| Default | none |

The name `State`, the update commands and the handlers' events use for the field.

## Inputs

None.

## Outputs

`PwrsGui.Widget`, its `Kind` `TextBox` and its `Properties` with `Text`, `Multiline` and `Password`, and `Placeholder` and `Enabled` when they are given.

## Examples

A field a person types into and submits with Enter. With `-HandlerOutput Pipeline` the handlers' output comes before the result:

<!-- person: $w = Wait-Window 'Connect'; Invoke-GuiTestEvent -Window $w.Id -Name Server -Event Change -Value 'web02'; Invoke-GuiTestEvent -Window $w.Id -Name Server -Event Submit -Value 'web02' -->
```powershell
$out = Show-GuiWindow -Title 'Connect' -HandlerOutput Pipeline {
    TextBox 'web01' -Name Server -Placeholder 'host name' -OnChange { "typed $($_.Value)" } -OnSubmit { "connecting to $($_.Value)"; Close-GuiWindow -Result Connect }
}
$out[0..1]
$out[2].Result
$out[2].State.Server
```

```text
typed web02
connecting to web02
Connect
web02
```

A password field, whose text comes back as a `SecureString`:

<!-- person: $w = Wait-Window 'Sign in'; Invoke-GuiTestEvent -Window $w.Id -Name Secret -Event Change -Value 'correct horse'; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Sign in' {
    TextBox -Name Secret -Password
    ButtonBar { Button 'OK' -Name Ok -Default -Result Ok }
}
$result.State.Secret.GetType().Name
$result.State.Secret.Length
```

```text
SecureString
13
```

Once a person gives the password field the focus and types, the field offers its eye. Here `Set-GuiFocus` and `Invoke-GuiTestEvent` stand in for the person, and `-Event Peek` holds the eye down, as a person pressing it does:

<!-- focus: its pictures show the password field with the keyboard focus and its eye -->
<!-- keep: new-guitextbox-peek.png images/new-guitextbox-peek.png -->
<!-- keep: new-guitextbox-peek-held.png images/new-guitextbox-peek-held.png -->
<!-- person: $w = Wait-Window 'Unlock'; Set-GuiFocus -Window $w.Id -Name Secret; Invoke-GuiTestEvent -Window $w.Id -Name Secret -Event Change -Value 'correct horse'; Wait-Property $w Secret peek $true; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'new-guitextbox-peek.png'); Invoke-GuiTestEvent -Window $w.Id -Name Secret -Event Peek -Value $true; Save-SnapshotPng (Get-GuiSnapshot -Window $w.Id -Pixels) (Join-Path $PWD 'new-guitextbox-peek-held.png'); Invoke-GuiTestEvent -Window $w.Id -Name Secret -Event Peek -Value $false; Invoke-GuiTestEvent -Window $w.Id -Name Ok -Event Click -->
```powershell
$result = Show-GuiWindow -Title 'Unlock' {
    Text 'The key to the vault' -Style Instruction
    TextBox -Name Secret -Password
    ButtonBar { Button 'Unlock' -Name Ok -Default -Result Ok }
}
$result.Result
```

```text
Ok
```

The field with the keyboard focus and a password typed, offering its eye at its right end:

![The Unlock window with the line "The key to the vault", a password field with the keyboard focus showing dots and an eye at its right end, and an Unlock button at the bottom right](/images/new-guitextbox-peek.png)

The same field while its eye is held down, the password shown:

![The Unlock window with the password field showing correct horse, its eye drawn in the selection's color, and an Unlock button at the bottom right](/images/new-guitextbox-peek-held.png)

A password field is one line:

```powershell
try { New-GuiTextBox -Password -Multiline } catch { $_.FullyQualifiedErrorId; $_.Exception.Message }
```

```text
GuiBadProperty,Pwrs.Modules.PWRS_GUI.NewGuiTextBoxCommand
a password field is one line; give -Password or -Multiline, not both
```

## Errors

| Error ID | When |
|---|---|
| `GuiBadProperty` | `-Password` is given with `-Multiline`, or `-OnSubmit` with `-Multiline` |
| `GuiOpeningOnly` | `Update-GuiWidget` sets `Password`, which is taken when the window opens |

Each is described in the [Error Reference](Error-Reference.md).
